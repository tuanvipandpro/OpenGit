#![cfg(windows)]
use opengit_lib::{
    auth::{prompts, AuthBroker},
    domain::contracts::*,
    git::runner::{argv, Cancellation, GitRunner},
    operations::scheduler::{OperationSpec, Scheduler},
    persistence::workspace::Database,
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
const SECRET: &str = "AUTH_SYNTHETIC_CANARY_NOT_A_CREDENTIAL";

fn helper(fixture: &Path) -> PathBuf {
    let source = fixture.join("helper.rs");
    let executable = fixture.join("askpass helper Ω quote's.exe");
    let real_git = GitRunner::discover().unwrap();
    let code=r#"#![windows_subsystem = "windows"]
    fn main() {
        if std::env::args().nth(1).as_deref()==Some("--credential-git") {
            use std::io::Write;
            let mut child=std::process::Command::new(OPENGIT_REAL_GIT_EXECUTABLE).args(["-c","credential.helper=","credential","fill"]).env("GIT_CONFIG_GLOBAL","NUL").env("GIT_CONFIG_SYSTEM","NUL").stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap();
            child.stdin.take().unwrap().write_all(b"protocol=https\nhost=example.invalid\n\n").unwrap();
            let mut output=child.wait_with_output().unwrap();let accepted=output.status.success()&&output.stdout.windows(b"password=AUTH_SYNTHETIC_CANARY_NOT_A_CREDENTIAL".len()).any(|part|part==b"password=AUTH_SYNTHETIC_CANARY_NOT_A_CREDENTIAL");output.stdout.fill(0);
            let error=String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();let flags=format!("cannot_run={} no_file={} permission={} username={} password={}",error.contains("cannot run"),error.contains("no such file"),error.contains("permission"),error.contains("could not read username"),error.contains("could not read password"));std::fs::write(std::env::current_exe().unwrap().parent().unwrap().join("safe-credential-diagnostic"),flags).unwrap();
            if accepted{println!("accepted");return;}std::process::exit(17);
        }
        if std::env::args().nth(1).as_deref()==Some("--fake-git") {
            let child=std::process::Command::new(std::env::current_exe().unwrap()).arg("Password for 'https://example.invalid': ").output().unwrap();
            if child.status.success() && child.stdout==b"AUTH_SYNTHETIC_CANARY_NOT_A_CREDENTIAL\n" {println!("accepted");return;}
            std::process::exit(17);
        }
        std::process::exit(opengit_lib::auth::helper_entry().unwrap_or(1));
    }"#.replace("OPENGIT_REAL_GIT_EXECUTABLE",&format!("{:?}",real_git.executable()));
    std::fs::write(&source, code).unwrap();
    let deps = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let library = std::fs::read_dir(&deps)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("libopengit_lib")
                && path.extension().is_some_and(|ext| ext == "rlib")
        })
        .max_by_key(|path| std::fs::metadata(path).unwrap().modified().unwrap())
        .unwrap();
    let rustc = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .filter(|path| path.is_absolute())
        .map(|path| path.join("rustc.exe"))
        .find(|path| path.is_file())
        .unwrap();
    // This fixture invokes rustc directly, so it must forward Cargo's native
    // import-library search path as well as the Rust dependency directory.
    // notify's pinned Windows backend now brings in windows-targets 0.53.
    let mut native_paths = std::collections::BTreeSet::new();
    for entry in std::fs::read_dir(deps.parent().unwrap().join("build"))
        .unwrap()
        .filter_map(Result::ok)
    {
        if !entry
            .file_name()
            .to_string_lossy()
            .starts_with("windows_x86_64_msvc-")
        {
            continue;
        }
        let Ok(output) = std::fs::read_to_string(entry.path().join("output")) else {
            continue;
        };
        for line in output.lines() {
            if let Some(path) = line.strip_prefix("cargo:rustc-link-search=native=") {
                let path = PathBuf::from(path);
                if path.is_absolute() && path.join("windows.0.53.0.lib").is_file() {
                    native_paths.insert(path);
                }
            }
        }
    }
    assert!(
        !native_paths.is_empty(),
        "Cargo Windows native import-library output missing"
    );
    let mut compiler = std::process::Command::new(rustc);
    compiler
        .arg(source)
        .args(["--edition", "2021", "--extern"])
        .arg(format!("opengit_lib={}", library.display()))
        .arg("-L")
        .arg(format!("dependency={}", deps.display()))
        .arg("-o")
        .arg(&executable);
    for path in native_paths {
        compiler.arg("-L").arg(format!("native={}", path.display()));
    }
    let compiled = compiler.output().unwrap();
    assert!(
        compiled.status.success(),
        "fixture helper compile failed: {}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    executable
}
#[tokio::test]
async fn actual_git_invokes_gui_helper_at_space_unicode_apostrophe_path() {
    let fixture = tempfile::tempdir().unwrap();
    let helper = helper(fixture.path());
    let scheduler = scheduler(&fixture.path().join("data"));
    let broker = AuthBroker::new(scheduler.clone());
    let operation = operation(&scheduler);
    let cancel = Cancellation::default();
    let session = broker
        .session("main", &operation.id, cancel.clone())
        .unwrap();
    let options = session.options(&helper).unwrap();
    let runner = GitRunner::trusted(helper).unwrap();
    let task = tokio::spawn(async move {
        runner
            .run_with(
                None,
                &argv(&["--credential-git"]),
                &cancel,
                Phase::Authentication,
                options,
            )
            .await
    });
    let mut answered = std::collections::HashSet::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(12);
    while !task.is_finished() && tokio::time::Instant::now() < deadline {
        let snapshot = scheduler.snapshot().unwrap();
        if let Some(prompt) = snapshot
            .operations
            .iter()
            .find(|entry| entry.id == operation.id)
            .and_then(|entry| entry.auth_prompt.clone())
        {
            if answered.insert(prompt.prompt_handle.clone()) {
                let value = if prompt.kind == AuthPromptKind::Username {
                    "fixture"
                } else {
                    SECRET
                };
                broker
                    .respond(
                        "main",
                        RespondAuthPromptRequest {
                            operation_id: operation.id.clone(),
                            prompt_handle: prompt.prompt_handle,
                            value: value.into(),
                        },
                    )
                    .unwrap();
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    if !task.is_finished() {
        session.close();
        panic!("actual Git helper timed out");
    }
    let result = task.await.unwrap();
    let diagnostic = std::fs::read_to_string(fixture.path().join("safe-credential-diagnostic"))
        .unwrap_or_default();
    assert_eq!(
        result.unwrap_or_else(|error| panic!(
            "actual Git helper failed {:?}; {diagnostic}",
            error.code
        )),
        b"accepted\n"
    );
    assert_eq!(answered.len(), 2);
    session.close();
}
#[tokio::test]
async fn actual_windows_openssh_decrypts_key_using_unicode_helper_ascii_alias() {
    let fixture = tempfile::tempdir().unwrap();
    let helper = helper(fixture.path());
    let keygen = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
        .join("System32/OpenSSH/ssh-keygen.exe");
    assert!(
        keygen.is_file(),
        "Windows OpenSSH fixture executable required"
    );
    let key = fixture.path().join("owned encrypted key");
    let generated = std::process::Command::new(&keygen)
        .args(["-q", "-t", "ed25519", "-N", SECRET, "-f"])
        .arg(&key)
        .output()
        .unwrap();
    assert!(
        generated.status.success(),
        "owned encrypted key generation failed"
    );
    let scheduler = scheduler(&fixture.path().join("data"));
    let operation = operation(&scheduler);
    let broker = AuthBroker::new(scheduler.clone());
    let cancel = Cancellation::default();
    let session = broker
        .session("main", &operation.id, cancel.clone())
        .unwrap();
    let options = session.options(&helper).unwrap();
    assert!(session.ssh_prompt_available());
    let ssh = options
        .environment
        .iter()
        .find(|(key, _)| key == "SSH_ASKPASS")
        .unwrap()
        .1
        .clone();
    assert!(ssh.to_str().unwrap().is_ascii());
    assert_eq!(
        opengit_lib::repositories::discovery::identity(Path::new(&ssh)).unwrap(),
        opengit_lib::repositories::discovery::identity(&helper).unwrap()
    );
    assert_eq!(
        options
            .environment
            .iter()
            .find(|(key, _)| key == "GIT_ASKPASS")
            .unwrap()
            .1,
        helper.as_os_str()
    );
    let runner = GitRunner::trusted(keygen).unwrap();
    let args = vec!["-y".into(), "-f".into(), key.into_os_string()];
    let task = tokio::spawn(async move {
        runner
            .run_with(None, &args, &cancel, Phase::Authentication, options)
            .await
    });
    let prompt = wait_prompt(&scheduler, &operation.id).await;
    assert_eq!(prompt.kind, AuthPromptKind::Passphrase);
    broker
        .respond(
            "main",
            RespondAuthPromptRequest {
                operation_id: operation.id,
                prompt_handle: prompt.prompt_handle,
                value: SECRET.into(),
            },
        )
        .unwrap();
    let output = tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(output.starts_with(b"ssh-ed25519 "));
    assert!(std::fs::rename(&helper, fixture.path().join("moved helper.exe")).is_err());
    session.close();
    std::fs::rename(&helper, fixture.path().join("moved helper.exe")).unwrap();
    assert!(!serde_json::to_string(&scheduler.snapshot().unwrap())
        .unwrap()
        .contains(SECRET));
}
fn scheduler(path: &Path) -> Arc<Scheduler> {
    Arc::new(Scheduler::new(Arc::new(Database::open(path).unwrap())).unwrap())
}
fn operation(scheduler: &Scheduler) -> OperationSnapshot {
    let op = scheduler
        .enqueue(OperationSpec {
            source_display: "https://example.invalid/repo".into(),
            destination_display: "Selected destination".into(),
            reservation: None,
        })
        .unwrap();
    scheduler
        .transition(&op.id, OperationState::Running, Phase::Authentication, None)
        .unwrap()
}
async fn wait_prompt(scheduler: &Scheduler, id: &str) -> AuthPrompt {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let snapshot = scheduler.snapshot().unwrap();
            if let Some(prompt) = snapshot
                .operations
                .iter()
                .find(|op| op.id == id)
                .and_then(|op| op.auth_prompt.clone())
            {
                return prompt;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}
#[test]
fn prompt_templates_and_actual_host_fingerprint_are_safe() {
    let fixture = tempfile::tempdir().unwrap();
    let backend = opengit_lib::backend::Backend::new(fixture.path()).unwrap();
    backend.get_workspace("main").unwrap();
    assert_eq!(
        backend
            .respond_auth_prompt(
                "main",
                RespondAuthPromptRequest {
                    operation_id: "x".repeat(17000),
                    prompt_handle: "unknown".into(),
                    value: SECRET.into()
                }
            )
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    for (raw, kind) in [
        (
            "Username for 'https://CANARY@example.invalid':",
            AuthPromptKind::Username,
        ),
        ("Password for 'CANARY':", AuthPromptKind::Password),
        (
            "Enter passphrase for key CANARY:",
            AuthPromptKind::Passphrase,
        ),
    ] {
        let (parsed, message) = prompts::classify(raw).unwrap();
        assert_eq!(parsed, kind);
        assert!(!message.contains("CANARY"));
    }
    let raw="The authenticity of host 'example.invalid (127.0.0.1)' can't be established.\nED25519 key fingerprint is SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA.\nAre you sure you want to continue connecting (yes/no/[fingerprint])?";
    let (kind, message) = prompts::classify(raw).unwrap();
    assert_eq!(kind, AuthPromptKind::Confirmation);
    assert!(message.contains("example.invalid"));
    assert!(message.contains("SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"));
    assert!(
        prompts::classify("Are you sure you want to continue connecting? no fingerprint").is_err()
    );
    assert!(prompts::classify("unknown CANARY").is_err());
    assert!(prompts::validate_value(&kind, "yes").is_ok());
    assert!(prompts::validate_value(&kind, "accept").is_err());
    assert!(prompts::validate_value(&AuthPromptKind::Password, "secret\n").is_err());
}
#[tokio::test]
async fn pipe_rejects_unowned_pid_then_accepts_owned_helper_scope_once_and_redacts_journal() {
    let fixture = tempfile::tempdir().unwrap();
    let helper = helper(fixture.path());
    let scheduler = scheduler(&fixture.path().join("db"));
    let broker = AuthBroker::new(scheduler.clone());
    let op = operation(&scheduler);
    let cancel = Cancellation::default();
    let session = broker.session("main", &op.id, cancel.clone()).unwrap();
    let options = session.options(&helper).unwrap();
    // Knowing the nonce is insufficient: the OS reports this test process as
    // outside the session's job, so it cannot create a prompt.
    let pipe = options
        .environment
        .iter()
        .find(|(key, _)| key == "OPENGIT_ASKPASS_PIPE")
        .unwrap()
        .1
        .to_string_lossy()
        .to_string();
    let mut stranger = tokio::net::windows::named_pipe::ClientOptions::new()
        .open(&pipe)
        .unwrap();
    let _ = stranger.write_all(b"untrusted").await;
    let mut byte = [0];
    assert!(
        tokio::time::timeout(Duration::from_secs(2), stranger.read(&mut byte))
            .await
            .unwrap()
            .is_ok_and(|count| count == 0)
            || stranger.read(&mut byte).await.is_err()
    );
    drop(stranger);
    let runner = GitRunner::trusted(helper.clone()).unwrap();
    let task = tokio::spawn(async move {
        runner
            .run_with(
                None,
                &argv(&["--fake-git"]),
                &cancel,
                Phase::Authentication,
                options,
            )
            .await
    });
    let prompt = wait_prompt(&scheduler, &op.id).await;
    let response = |window: &str, operation: &str, value: &str| {
        broker.respond(
            window,
            RespondAuthPromptRequest {
                operation_id: operation.into(),
                prompt_handle: prompt.prompt_handle.clone(),
                value: value.into(),
            },
        )
    };
    assert_eq!(
        response("other-window", &op.id, SECRET).unwrap_err().code,
        ErrorCode::InvalidHandle
    );
    assert!(response("main", "wrong-operation", SECRET).is_err());
    response("main", &op.id, SECRET).unwrap();
    assert!(response("main", &op.id, SECRET).is_err());
    assert_eq!(task.await.unwrap().unwrap(), b"accepted\n");
    session.close();
    let serialized = serde_json::to_string(&scheduler.snapshot().unwrap()).unwrap();
    assert!(!serialized.contains(SECRET));
    let bytes = std::fs::read(fixture.path().join("db/workspace.sqlite3")).unwrap();
    assert!(!bytes
        .windows(SECRET.len())
        .any(|bytes| bytes == SECRET.as_bytes()));
    let op = operation(&scheduler);
    let cancel = Cancellation::default();
    let session = broker.session("main", &op.id, cancel.clone()).unwrap();
    let options = session.options(&helper).unwrap();
    let runner = GitRunner::trusted(helper).unwrap();
    let running = cancel.clone();
    let task = tokio::spawn(async move {
        runner
            .run_with(
                None,
                &argv(&["--fake-git"]),
                &running,
                Phase::Authentication,
                options,
            )
            .await
    });
    let prompt = wait_prompt(&scheduler, &op.id).await;
    scheduler.cancel(&op.id).unwrap();
    cancel.cancel();
    broker.invalidate(&op.id);
    assert!(broker
        .respond(
            "main",
            RespondAuthPromptRequest {
                operation_id: op.id,
                prompt_handle: prompt.prompt_handle,
                value: SECRET.into()
            }
        )
        .is_err());
    assert_eq!(
        task.await.unwrap().unwrap_err().code,
        ErrorCode::Interrupted
    );
}
