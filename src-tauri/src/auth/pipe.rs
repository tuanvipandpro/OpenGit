use super::prompts;
use crate::{
    domain::contracts::*,
    git::{
        error::error,
        runner::{Cancellation, OwnedJob, RunOptions},
    },
    operations::scheduler::Scheduler,
};
use std::os::windows::io::AsRawHandle;
use std::{
    collections::HashMap,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::windows::named_pipe::{ClientOptions, NamedPipeServer, ServerOptions},
    sync::oneshot,
};
use zeroize::Zeroizing;

const TTL: Duration = Duration::from_secs(120);
struct Pending {
    window: String,
    operation: String,
    kind: AuthPromptKind,
    expires: Instant,
    cancel: Cancellation,
    response: oneshot::Sender<Zeroizing<Vec<u8>>>,
}
pub struct AuthBroker {
    scheduler: Arc<Scheduler>,
    pending: Mutex<HashMap<String, Pending>>,
}
struct Session {
    broker: Arc<AuthBroker>,
    window: String,
    operation: String,
    pipe: String,
    nonce: String,
    job: Mutex<Option<OwnedJob>>,
    active: AtomicBool,
    cancel: Cancellation,
    helper_leases: Mutex<Vec<Arc<super::helper_path::HelperPaths>>>,
    #[cfg(test)]
    before_release: Mutex<Option<Arc<ReleaseBarrier>>>,
}
#[cfg(test)]
struct ReleaseBarrier {
    received: std::sync::Barrier,
    resume: std::sync::Barrier,
}
pub struct AuthSession(Arc<Session>);
impl AuthBroker {
    pub fn new(scheduler: Arc<Scheduler>) -> Arc<Self> {
        Arc::new(Self {
            scheduler,
            pending: Mutex::new(HashMap::new()),
        })
    }
    pub fn session(
        self: &Arc<Self>,
        window: &str,
        operation: &str,
        cancel: Cancellation,
    ) -> Result<AuthSession, AppError> {
        let session = Arc::new(Session {
            broker: self.clone(),
            window: window.into(),
            operation: operation.into(),
            pipe: format!(r"\\.\pipe\opengit-askpass-{}", uuid::Uuid::new_v4()),
            nonce: format!(
                "{}{}",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            ),
            job: Mutex::new(None),
            active: AtomicBool::new(true),
            cancel,
            helper_leases: Mutex::new(Vec::new()),
            #[cfg(test)]
            before_release: Mutex::new(None),
        });
        let server = create_pipe(&session.pipe, true)?;
        let task = session.clone();
        tokio::spawn(async move {
            serve(task, server).await;
        });
        Ok(AuthSession(session))
    }
    pub fn respond(&self, window: &str, request: RespondAuthPromptRequest) -> Result<(), AppError> {
        let value = Zeroizing::new(request.value.into_bytes());
        let mut pending = self.pending.lock().map_err(|_| prompts::invalid())?;
        let item = pending
            .get(&request.prompt_handle)
            .ok_or_else(prompts::invalid)?;
        if item.window != window
            || item.operation != request.operation_id
            || item.expires <= Instant::now()
            || item.cancel.is_cancelled()
        {
            return Err(prompts::invalid());
        }
        prompts::validate_value(
            &item.kind,
            std::str::from_utf8(&value).map_err(|_| prompts::invalid())?,
        )?;
        let item = pending
            .remove(&request.prompt_handle)
            .ok_or_else(prompts::invalid)?;
        item.response.send(value).map_err(|_| prompts::invalid())
    }
    pub fn invalidate(&self, operation: &str) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.retain(|_, item| item.operation != operation);
        }
    }
    pub fn close_window(&self, window: &str) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.retain(|_, item| item.window != window);
        }
    }
}
impl AuthSession {
    pub fn ssh_prompt_available(&self) -> bool {
        self.0
            .helper_leases
            .lock()
            .ok()
            .and_then(|leases| leases.last().map(|paths| paths.ssh.is_some()))
            .unwrap_or(false)
    }
    pub fn options(&self, helper: &Path) -> Result<RunOptions, AppError> {
        if !helper.is_absolute() || !helper.is_file() {
            return Err(prompts::invalid());
        }
        let paths = Arc::new(super::helper_path::select(helper)?);
        self.options_for_paths(paths)
    }
    pub(super) fn options_for_paths(
        &self,
        paths: Arc<super::helper_path::HelperPaths>,
    ) -> Result<RunOptions, AppError> {
        let ssh = paths.ssh.clone();
        self.0
            .helper_leases
            .lock()
            .map_err(|_| prompts::invalid())?
            .push(paths.clone());
        // Git's do_askpass executes this as one argv[0], without use_shell.
        // Embedded quoting would become part of the literal executable name.
        let session = self.0.clone();
        Ok(RunOptions {
            interactive: true,
            lifetime_guard: Some(paths.clone()),
            environment: vec![
                ("GIT_ASKPASS".into(), paths.git.clone()),
                ("SSH_ASKPASS".into(), ssh.clone().unwrap_or_default()),
                (
                    "SSH_ASKPASS_REQUIRE".into(),
                    if ssh.is_some() { "force" } else { "never" }.into(),
                ),
                ("DISPLAY".into(), "opengit-askpass".into()),
                ("OPENGIT_INTERNAL_ASKPASS".into(), "1".into()),
                ("OPENGIT_ASKPASS_PIPE".into(), session.pipe.clone().into()),
                ("OPENGIT_ASKPASS_NONCE".into(), session.nonce.clone().into()),
            ],
            before_resume: Some(Arc::new(move |job| {
                *session.job.lock().map_err(|_| prompts::invalid())? = Some(job);
                Ok(())
            })),
        })
    }
    pub fn close(&self) {
        self.0.active.store(false, Ordering::SeqCst);
        self.0.broker.invalidate(&self.0.operation);
        if let Ok(mut job) = self.0.job.lock() {
            job.take();
        }
        if let Ok(mut leases) = self.0.helper_leases.lock() {
            leases.clear();
        }
    }
}
impl Drop for AuthSession {
    fn drop(&mut self) {
        self.close();
    }
}
async fn cancelled(session: &Session) {
    while session.active.load(Ordering::SeqCst) && !session.cancel.is_cancelled() {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
async fn serve(session: Arc<Session>, mut server: NamedPipeServer) {
    loop {
        tokio::select! {result=server.connect()=>if result.is_err(){break;},_=cancelled(&session)=>break}
        let next = match create_pipe(&session.pipe, false) {
            Ok(next) => next,
            Err(_) => break,
        };
        let result = tokio::select! {result=exchange(&session,&mut server)=>result,_=cancelled(&session)=>Err(prompts::invalid())};
        let _ = result;
        drop(server);
        server = next;
    }
    session.broker.invalidate(&session.operation);
}
async fn exchange(session: &Session, server: &mut NamedPipeServer) -> Result<(), AppError> {
    use windows_sys::Win32::System::Pipes::GetNamedPipeClientProcessId;
    let mut pid = 0;
    if unsafe { GetNamedPipeClientProcessId(server.as_raw_handle() as _, &mut pid) } == 0 {
        return Err(prompts::invalid());
    }
    if !session
        .job
        .lock()
        .map_err(|_| prompts::invalid())?
        .as_ref()
        .is_some_and(|job| job.contains(pid))
    {
        return Err(prompts::invalid());
    }
    let handshake = async {
        let length = server.read_u32_le().await.map_err(|_| prompts::invalid())? as usize;
        if length > 16384 {
            return Err(prompts::invalid());
        }
        let mut bytes = Zeroizing::new(vec![0; length]);
        server
            .read_exact(&mut bytes)
            .await
            .map_err(|_| prompts::invalid())?;
        let message: Hello = serde_json::from_slice(&bytes).map_err(|_| prompts::invalid())?;
        if message.nonce != session.nonce {
            return Err(prompts::invalid());
        }
        let (kind, text) = prompts::classify(&message.prompt)?;
        let handle = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let (tx, rx) = oneshot::channel();
        session
            .broker
            .pending
            .lock()
            .map_err(|_| prompts::invalid())?
            .insert(
                handle.clone(),
                Pending {
                    window: session.window.clone(),
                    operation: session.operation.clone(),
                    kind: kind.clone(),
                    expires: Instant::now() + TTL,
                    cancel: session.cancel.clone(),
                    response: tx,
                },
            );
        let prompt = AuthPrompt {
            prompt_handle: handle.clone(),
            kind,
            message: text,
            expires_at: (chrono::Utc::now() + chrono::Duration::seconds(120)).to_rfc3339(),
        };
        if session
            .broker
            .scheduler
            .auth_prompt(&session.operation, prompt)
            .is_err()
        {
            session.broker.invalidate(&session.operation);
            return Err(prompts::invalid());
        }
        let response = tokio::time::timeout(TTL, rx)
            .await
            .map_err(|_| prompts::invalid())?
            .map_err(|_| prompts::invalid())?;
        #[cfg(test)]
        if let Some(barrier) = session
            .before_release
            .lock()
            .map_err(|_| prompts::invalid())?
            .clone()
        {
            barrier.received.wait();
            barrier.resume.wait();
        }
        session
            .broker
            .pending
            .lock()
            .map_err(|_| prompts::invalid())?
            .remove(&handle);
        if !session
            .broker
            .scheduler
            .resume_auth(&session.operation, || {
                session.active.load(Ordering::SeqCst) && !session.cancel.is_cancelled()
            })?
        {
            return Err(prompts::invalid());
        }
        server
            .write_u32_le(response.len() as u32)
            .await
            .map_err(|_| prompts::invalid())?;
        server
            .write_all(&response)
            .await
            .map_err(|_| prompts::invalid())?;
        server.flush().await.map_err(|_| prompts::invalid())?;
        Ok(())
    };
    tokio::time::timeout(TTL + Duration::from_secs(5), handshake)
        .await
        .map_err(|_| prompts::invalid())?
}
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Hello {
    nonce: String,
    prompt: String,
}

fn create_pipe(name: &str, first: bool) -> Result<NamedPipeServer, AppError> {
    use std::mem::size_of;
    use windows_sys::Win32::{
        Foundation::{CloseHandle, LocalFree},
        Security::{
            Authorization::{
                ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
                SDDL_REVISION_1,
            },
            GetTokenInformation, TokenUser, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
        },
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };
    unsafe {
        let mut token = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(prompts::invalid());
        }
        let mut required = 0;
        GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut required);
        let mut buffer = vec![0usize; (required as usize).div_ceil(size_of::<usize>())];
        let ok = GetTokenInformation(
            token,
            TokenUser,
            buffer.as_mut_ptr() as _,
            required,
            &mut required,
        );
        CloseHandle(token);
        if ok == 0 {
            return Err(prompts::invalid());
        }
        let user = &*(buffer.as_ptr() as *const TOKEN_USER);
        let mut sid = std::ptr::null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut sid) == 0 {
            return Err(prompts::invalid());
        }
        let mut length = 0;
        while *sid.add(length) != 0 {
            length += 1;
        }
        let sid_text = String::from_utf16_lossy(std::slice::from_raw_parts(sid, length));
        LocalFree(sid as _);
        let sddl: Vec<u16> = format!("D:P(A;;GA;;;{sid_text})")
            .encode_utf16()
            .chain([0])
            .collect();
        let mut descriptor = std::ptr::null_mut();
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            std::ptr::null_mut(),
        ) == 0
        {
            return Err(prompts::invalid());
        }
        let mut attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        let result = ServerOptions::new()
            .first_pipe_instance(first)
            .reject_remote_clients(true)
            .create_with_security_attributes_raw(name, &mut attributes as *mut _ as _);
        LocalFree(descriptor as _);
        result.map_err(|_| error(ErrorCode::Io, Phase::Authentication))
    }
}
pub fn helper_main() -> i32 {
    if std::env::var("OPENGIT_INTERNAL_ASKPASS").ok().as_deref() != Some("1") {
        return 1;
    }
    let pipe = match std::env::var("OPENGIT_ASKPASS_PIPE") {
        Ok(pipe) if pipe.starts_with(r"\\.\pipe\opengit-askpass-") && pipe.len() < 128 => pipe,
        _ => return 1,
    };
    let nonce = match std::env::var("OPENGIT_ASKPASS_NONCE") {
        Ok(nonce) if nonce.len() == 64 && nonce.bytes().all(|b| b.is_ascii_hexdigit()) => nonce,
        _ => return 1,
    };
    let prompt = match std::env::args().nth(1) {
        Some(prompt) if prompt.len() <= 8192 => prompt,
        _ => return 1,
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => return 1,
    };
    runtime.block_on(async move {
        let transfer = async {
            let mut client = ClientOptions::new().open(pipe)?;
            let bytes =
                serde_json::to_vec(&Hello { nonce, prompt }).map_err(std::io::Error::other)?;
            client.write_u32_le(bytes.len() as u32).await?;
            client.write_all(&bytes).await?;
            let length = client.read_u32_le().await? as usize;
            if length > 8192 {
                return Err(std::io::Error::other("Invalid helper response"));
            }
            let mut response = Zeroizing::new(vec![0; length]);
            client.read_exact(&mut response).await?;
            use std::io::Write;
            let mut stdout = std::io::stdout().lock();
            stdout.write_all(&response)?;
            stdout.write_all(b"\n")?;
            stdout.flush()?;
            Ok::<_, std::io::Error>(())
        };
        if tokio::time::timeout(TTL + Duration::from_secs(10), transfer)
            .await
            .is_ok_and(|result| result.is_ok())
        {
            0
        } else {
            1
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn received_response_losing_cancel_before_resume_is_never_written_to_pipe() {
        use crate::{
            git::runner::{argv, GitRunner},
            operations::scheduler::OperationSpec,
            persistence::workspace::Database,
        };
        let fixture = tempfile::tempdir().unwrap();
        let source = fixture.path().join("receiver.rs");
        let executable = fixture.path().join("owned pipe receiver.exe");
        std::fs::write(&source,r##"use std::io::{Read,Write};fn main(){let root=std::env::current_exe().unwrap().parent().unwrap().to_owned();let mut pipe=std::fs::OpenOptions::new().read(true).write(true).open(std::env::var("OPENGIT_ASKPASS_PIPE").unwrap()).unwrap();let hello=format!(r#"{{"nonce":"{}","prompt":"Password for fixture:"}}"#,std::env::var("OPENGIT_ASKPASS_NONCE").unwrap());pipe.write_all(&(hello.len() as u32).to_le_bytes()).unwrap();pipe.write_all(hello.as_bytes()).unwrap();let mut length=[0u8;4];if pipe.read_exact(&mut length).is_ok(){std::fs::write(root.join("secret-was-released"),b"unexpected response bytes").unwrap();}else{std::fs::write(root.join("no-secret"),b"EOF").unwrap();}}"##).unwrap();
        let rustc = std::env::split_paths(&std::env::var_os("PATH").unwrap())
            .filter(|path| path.is_absolute())
            .map(|path| path.join("rustc.exe"))
            .find(|path| path.is_file())
            .unwrap();
        assert!(std::process::Command::new(rustc)
            .arg(source)
            .args(["--edition", "2021", "-o"])
            .arg(&executable)
            .output()
            .unwrap()
            .status
            .success());
        let scheduler = Arc::new(
            Scheduler::new(Arc::new(
                Database::open(&fixture.path().join("data")).unwrap(),
            ))
            .unwrap(),
        );
        let operation = scheduler
            .enqueue(OperationSpec {
                source_display: "https://example.invalid/repo".into(),
                destination_display: "Selected destination".into(),
                reservation: None,
            })
            .unwrap();
        scheduler
            .transition(
                &operation.id,
                OperationState::Running,
                Phase::Authentication,
                None,
            )
            .unwrap();
        let broker = AuthBroker::new(scheduler.clone());
        let cancel = Cancellation::default();
        let session = broker
            .session("main", &operation.id, cancel.clone())
            .unwrap();
        let barrier = Arc::new(ReleaseBarrier {
            received: std::sync::Barrier::new(2),
            resume: std::sync::Barrier::new(2),
        });
        *session.0.before_release.lock().unwrap() = Some(barrier.clone());
        let options = session.options(&executable).unwrap();
        let runner = GitRunner::trusted(executable).unwrap();
        let process = tokio::spawn(async move {
            runner
                .run_with(None, &argv(&[]), &cancel, Phase::Authentication, options)
                .await
        });
        let prompt = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(prompt) = scheduler.snapshot().unwrap().operations[0]
                    .auth_prompt
                    .clone()
                {
                    return prompt;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        broker
            .respond(
                "main",
                RespondAuthPromptRequest {
                    operation_id: operation.id.clone(),
                    prompt_handle: prompt.prompt_handle,
                    value: "UNIT_SYNTHETIC_SECRET".into(),
                },
            )
            .unwrap();
        barrier.received.wait();
        assert_eq!(
            scheduler.cancel(&operation.id).unwrap().state,
            OperationState::Cancelling
        );
        barrier.resume.wait();
        process.await.unwrap().unwrap();
        assert!(fixture.path().join("no-secret").is_file());
        assert!(!fixture.path().join("secret-was-released").exists());
        assert_ne!(
            scheduler.snapshot().unwrap().operations[0].state,
            OperationState::Running
        );
        session.close();
    }
    #[tokio::test]
    async fn actual_pipe_dacl_has_one_protected_current_user_ace() {
        use windows_sys::Win32::{
            Foundation::LocalFree,
            Security::{
                Authorization::{
                    ConvertSecurityDescriptorToStringSecurityDescriptorW, GetSecurityInfo,
                    SDDL_REVISION_1, SE_KERNEL_OBJECT,
                },
                DACL_SECURITY_INFORMATION,
            },
        };
        let name = format!(r"\\.\pipe\opengit-askpass-{}", uuid::Uuid::new_v4());
        let server = create_pipe(&name, true).unwrap();
        unsafe {
            let mut descriptor = std::ptr::null_mut();
            assert_eq!(
                GetSecurityInfo(
                    server.as_raw_handle() as _,
                    SE_KERNEL_OBJECT,
                    DACL_SECURITY_INFORMATION,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    &mut descriptor
                ),
                0
            );
            let mut text = std::ptr::null_mut();
            assert_ne!(
                ConvertSecurityDescriptorToStringSecurityDescriptorW(
                    descriptor,
                    SDDL_REVISION_1,
                    DACL_SECURITY_INFORMATION,
                    &mut text,
                    std::ptr::null_mut()
                ),
                0
            );
            let mut length = 0;
            while *text.add(length) != 0 {
                length += 1;
            }
            let sddl = String::from_utf16_lossy(std::slice::from_raw_parts(text, length));
            LocalFree(text as _);
            LocalFree(descriptor as _);
            assert!(sddl.starts_with("D:P"));
            assert_eq!(sddl.matches("(A;").count(), 1);
            assert!(!sddl.contains(";;;WD"));
            assert!(!sddl.contains(";;;AU"));
            assert!(sddl.contains(";;;S-1-5-21-"));
        }
    }
    #[test]
    fn expired_cancelled_and_closed_window_prompts_cannot_release_a_secret() {
        let fixture = tempfile::tempdir().unwrap();
        let scheduler = Arc::new(
            Scheduler::new(Arc::new(
                crate::persistence::workspace::Database::open(fixture.path()).unwrap(),
            ))
            .unwrap(),
        );
        let broker = AuthBroker::new(scheduler);
        for expired in [true, false] {
            let (tx, mut rx) = oneshot::channel();
            let cancel = Cancellation::default();
            if !expired {
                cancel.cancel();
            }
            broker.pending.lock().unwrap().insert(
                "handle".into(),
                Pending {
                    window: "main".into(),
                    operation: "operation".into(),
                    kind: AuthPromptKind::Password,
                    expires: if expired {
                        Instant::now() - Duration::from_secs(1)
                    } else {
                        Instant::now() + TTL
                    },
                    cancel,
                    response: tx,
                },
            );
            assert!(broker
                .respond(
                    "main",
                    RespondAuthPromptRequest {
                        operation_id: "operation".into(),
                        prompt_handle: "handle".into(),
                        value: "SYNTHETIC_SECRET".into()
                    }
                )
                .is_err());
            assert!(matches!(
                rx.try_recv(),
                Err(oneshot::error::TryRecvError::Empty)
            ));
            broker.close_window("main");
            assert!(matches!(
                rx.try_recv(),
                Err(oneshot::error::TryRecvError::Closed)
            ));
        }
    }
}
