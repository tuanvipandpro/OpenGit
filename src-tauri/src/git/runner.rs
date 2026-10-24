use super::error::{error, git_failure};
use crate::domain::contracts::*;
use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::io::{AsyncRead, AsyncReadExt};

#[derive(Clone, Default)]
pub struct Cancellation(Arc<AtomicBool>);
impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}
#[derive(Clone)]
pub struct GitRunner {
    executable: PathBuf,
    pub timeout: Duration,
    pub output_limit: usize,
}
pub struct GitOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub success: bool,
}
#[derive(Clone, Default)]
pub struct RunOptions {
    pub environment: Vec<(OsString, OsString)>,
    pub interactive: bool,
    pub lifetime_guard: Option<Arc<dyn Send + Sync>>,
    #[cfg(windows)]
    pub before_resume: Option<SpawnCallback>,
}
#[cfg(windows)]
pub type SpawnCallback = Arc<dyn Fn(OwnedJob) -> Result<(), AppError> + Send + Sync>;
#[cfg(windows)]
pub use owned_job::OwnedJob;
impl GitRunner {
    /// Trusted internal constructor, never exposed through IPC.
    pub fn trusted(executable: PathBuf) -> Result<Self, AppError> {
        if !executable.is_absolute() || !executable.is_file() {
            return Err(error(ErrorCode::GitUnavailable, Phase::Discovery));
        }
        Ok(Self {
            executable: std::fs::canonicalize(executable)
                .map_err(|_| error(ErrorCode::GitUnavailable, Phase::Discovery))?,
            timeout: Duration::from_secs(30),
            output_limit: 8 * 1024 * 1024,
        })
    }
    pub fn discover() -> Result<Self, AppError> {
        let mut candidates = Vec::new();
        for name in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(root) = std::env::var_os(name) {
                candidates.push(PathBuf::from(root).join("Git/cmd/git.exe"));
            }
        }
        if let Some(paths) = std::env::var_os("PATH") {
            for directory in std::env::split_paths(&paths).filter(|p| p.is_absolute()) {
                candidates.push(directory.join(if cfg!(windows) { "git.exe" } else { "git" }));
            }
        }
        candidates
            .into_iter()
            .find_map(|p| Self::trusted(p).ok())
            .ok_or_else(|| error(ErrorCode::GitUnavailable, Phase::Discovery))
    }
    pub fn executable(&self) -> &Path {
        &self.executable
    }
    pub async fn run(
        &self,
        cwd: Option<&Path>,
        args: &[OsString],
        cancel: &Cancellation,
        phase: Phase,
    ) -> Result<Vec<u8>, AppError> {
        let output = self.output(cwd, args, cancel, phase.clone()).await?;
        if output.success {
            Ok(output.stdout)
        } else {
            Err(git_failure(&output.stderr, phase))
        }
    }
    pub async fn output(
        &self,
        cwd: Option<&Path>,
        args: &[OsString],
        cancel: &Cancellation,
        phase: Phase,
    ) -> Result<GitOutput, AppError> {
        self.output_with(cwd, args, cancel, phase, RunOptions::default())
            .await
    }
    pub async fn run_with(
        &self,
        cwd: Option<&Path>,
        args: &[OsString],
        cancel: &Cancellation,
        phase: Phase,
        options: RunOptions,
    ) -> Result<Vec<u8>, AppError> {
        let output = self
            .output_with(cwd, args, cancel, phase.clone(), options)
            .await?;
        if output.success {
            Ok(output.stdout)
        } else {
            Err(git_failure(&output.stderr, phase))
        }
    }
    async fn output_with(
        &self,
        cwd: Option<&Path>,
        args: &[OsString],
        cancel: &Cancellation,
        phase: Phase,
        options: RunOptions,
    ) -> Result<GitOutput, AppError> {
        let lifetime_guard = options.lifetime_guard;
        if cancel.is_cancelled() {
            return Err(error(ErrorCode::Interrupted, phase));
        }
        if args.iter().any(|a| a.to_string_lossy().contains('\0'))
            || cwd.is_some_and(|p| !p.is_absolute())
        {
            return Err(error(ErrorCode::InvalidRequest, phase));
        }
        let mut command = tokio::process::Command::new(&self.executable);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("LC_ALL", "C")
            .env("LANG", "C")
            .env("GCM_INTERACTIVE", "never")
            .env("GIT_OPTIONAL_LOCKS", "0");
        if options.interactive {
            command
                .env("GCM_INTERACTIVE", "always")
                .env_remove("GIT_OPTIONAL_LOCKS");
        }
        command.envs(options.environment);
        if let Some(cwd) = cwd {
            command.current_dir(cwd);
        } else if let Some(neutral) = self.executable.parent() {
            command.current_dir(neutral);
        }
        for variable in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_COMMON_DIR",
            "GIT_INDEX_FILE",
            "GIT_OBJECT_DIRECTORY",
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            "GIT_NAMESPACE",
        ] {
            command.env_remove(variable);
        }
        #[cfg(windows)]
        command.creation_flags(0x00000004 | 0x08000000); // suspended, no console
        let mut child = command
            .spawn()
            .map_err(|_| error(ErrorCode::GitUnavailable, phase.clone()))?;
        #[cfg(windows)]
        let job = match owned_job::attach(&child) {
            Ok(job) => job,
            Err(()) => {
                let _ = child.kill().await;
                return Err(error(ErrorCode::Internal, phase));
            }
        };
        #[cfg(windows)]
        let _job_guard = owned_job::TerminationGuard(job.clone());
        #[cfg(windows)]
        {
            if let Some(callback) = options.before_resume {
                callback(job.clone())?;
            }
            if owned_job::resume(&child).is_err() {
                let _ = child.kill().await;
                return Err(error(ErrorCode::Internal, phase));
            }
        }
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| error(ErrorCode::Internal, phase.clone()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| error(ErrorCode::Internal, phase.clone()))?;
        let limit = self.output_limit;
        let run = async {
            let (status, out, err) = tokio::try_join!(
                async {
                    let status = child.wait().await.map_err(|_| ErrorCode::Io)?;
                    // A detached descendant may retain inherited pipe handles.
                    // Stop it on parent exit, before waiting for pipe EOF.
                    #[cfg(windows)]
                    job.terminate();
                    Ok(status)
                },
                bounded(stdout, limit),
                bounded(stderr, limit)
            )?;
            Ok::<_, ErrorCode>(GitOutput {
                stdout: out,
                stderr: err,
                success: status.success(),
            })
        };
        let cancellation = async {
            loop {
                if cancel.is_cancelled() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        };
        let result = tokio::select! {
            value = tokio::time::timeout(self.timeout, run) => match value { Ok(v) => v, Err(_) => Err(ErrorCode::Timeout) },
            _ = cancellation => Err(ErrorCode::Interrupted),
        };
        if result.is_err() {
            // Do not wait without a bound on one process before the bounded
            // whole-job quiescence check below.
            let _ = child.start_kill();
        }
        #[cfg(windows)]
        if job.stop_and_wait().await.is_err() {
            // Native clone fails closed after this error (at most two in
            // flight). Retain only pure file leases until process exit rather
            // than release path pins while an owned process may still run.
            if let Some(guard) = lifetime_guard {
                std::mem::forget(guard);
            }
            // No caller may mutate/clean staging on an unconfirmed tree stop.
            return Err(error(ErrorCode::Interrupted, Phase::Recovery));
        }
        result.map_err(|code| error(code, phase))
    }
}
async fn bounded(mut stream: impl AsyncRead + Unpin, limit: usize) -> Result<Vec<u8>, ErrorCode> {
    let mut output = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        let count = stream.read(&mut buffer).await.map_err(|_| ErrorCode::Io)?;
        if count == 0 {
            return Ok(output);
        }
        if output.len().saturating_add(count) > limit {
            return Err(ErrorCode::OutputLimit);
        }
        output.extend_from_slice(&buffer[..count]);
    }
}
pub fn argv(args: &[&str]) -> Vec<OsString> {
    args.iter()
        .map(OsStr::new)
        .map(OsStr::to_os_string)
        .collect()
}

#[cfg(windows)]
mod owned_job {
    use std::mem::{size_of, zeroed};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
        System::{
            Diagnostics::ToolHelp::*,
            JobObjects::*,
            Threading::{
                OpenProcess, OpenThread, ResumeThread, PROCESS_QUERY_LIMITED_INFORMATION,
                THREAD_SUSPEND_RESUME,
            },
        },
    };
    struct Job(HANDLE);
    // The handle is owned and only closed once; the OS synchronizes job operations.
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}
    #[derive(Clone)]
    pub struct OwnedJob(std::sync::Arc<Job>);
    impl OwnedJob {
        pub fn contains(&self, pid: u32) -> bool {
            unsafe {
                let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
                if process.is_null() {
                    return false;
                }
                let mut member = 0;
                let valid = IsProcessInJob(process, self.0 .0, &mut member) != 0 && member != 0;
                CloseHandle(process);
                valid
            }
        }
        pub fn terminate(&self) {
            unsafe {
                TerminateJobObject(self.0 .0, 1);
            }
        }
        pub(super) async fn stop_and_wait(&self) -> Result<(), ()> {
            self.terminate();
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    let mut accounting: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION =
                        unsafe { zeroed() };
                    if unsafe {
                        QueryInformationJobObject(
                            self.0 .0,
                            JobObjectBasicAccountingInformation,
                            &mut accounting as *mut _ as _,
                            size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                            std::ptr::null_mut(),
                        )
                    } == 0
                    {
                        return Err(());
                    }
                    if accounting.ActiveProcesses == 0 {
                        return Ok(());
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
            .await
            .map_err(|_| ())?
        }
    }
    pub struct TerminationGuard(pub OwnedJob);
    impl Drop for TerminationGuard {
        fn drop(&mut self) {
            self.0.terminate();
        }
    }
    impl Drop for Job {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    pub fn attach(child: &tokio::process::Child) -> Result<OwnedJob, ()> {
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return Err(());
            }
            let owned = Job(job);
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as _,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            ) == 0
                || AssignProcessToJobObject(job, child.raw_handle().ok_or(())? as HANDLE) == 0
            {
                return Err(());
            }
            Ok(OwnedJob(std::sync::Arc::new(owned)))
        }
    }
    pub fn resume(child: &tokio::process::Child) -> Result<(), ()> {
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
            if snapshot == INVALID_HANDLE_VALUE {
                return Err(());
            }
            let mut entry: THREADENTRY32 = zeroed();
            entry.dwSize = size_of::<THREADENTRY32>() as u32;
            let mut next = Thread32First(snapshot, &mut entry);
            let mut resumed = false;
            while next != 0 {
                if Some(entry.th32OwnerProcessID) == child.id() {
                    let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID);
                    if !thread.is_null() {
                        resumed = ResumeThread(thread) != u32::MAX;
                        CloseHandle(thread);
                    }
                    break;
                }
                next = Thread32Next(snapshot, &mut entry);
            }
            CloseHandle(snapshot);
            if resumed {
                Ok(())
            } else {
                Err(())
            }
        }
    }
}
