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
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

pub const INPUT_LIMIT_BYTES: usize = 8 * 1024 * 1024;

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
    pub exit_code: Option<i32>,
}
struct OutputPolicy<'a> {
    input: Option<&'a [u8]>,
    suppress_grafts: bool,
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
        self.output_with(cwd, args, cancel, phase, RunOptions::default(), None)
            .await
    }
    /// Trusted internal binary input, never a renderer-provided command/path.
    /// Feed concurrently with draining both output pipes to avoid deadlock.
    pub async fn output_with_input(
        &self,
        cwd: Option<&Path>,
        args: &[OsString],
        cancel: &Cancellation,
        phase: Phase,
        input: &[u8],
    ) -> Result<GitOutput, AppError> {
        self.output_with(cwd, args, cancel, phase, RunOptions::default(), Some(input))
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
            .output_with(cwd, args, cancel, phase.clone(), options, None)
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
        input: Option<&[u8]>,
    ) -> Result<GitOutput, AppError> {
        self.output_internal(
            cwd,
            args,
            cancel,
            phase,
            options,
            OutputPolicy {
                input,
                suppress_grafts: false,
            },
        )
        .await
    }
    pub(crate) async fn output_history(
        &self,
        cwd: Option<&Path>,
        args: &[OsString],
        cancel: &Cancellation,
        phase: Phase,
        options: RunOptions,
        input: Option<&[u8]>,
    ) -> Result<GitOutput, AppError> {
        self.output_internal(
            cwd,
            args,
            cancel,
            phase,
            options,
            OutputPolicy {
                input,
                suppress_grafts: true,
            },
        )
        .await
    }
    async fn output_internal(
        &self,
        cwd: Option<&Path>,
        args: &[OsString],
        cancel: &Cancellation,
        phase: Phase,
        options: RunOptions,
        policy: OutputPolicy<'_>,
    ) -> Result<GitOutput, AppError> {
        let OutputPolicy {
            input,
            suppress_grafts,
        } = policy;
        let lifetime_guard = options.lifetime_guard;
        if cancel.is_cancelled() {
            return Err(error(ErrorCode::Interrupted, phase));
        }
        if input.is_some_and(|bytes| bytes.len() > INPUT_LIMIT_BYTES) {
            return Err(error(ErrorCode::InvalidRequest, phase));
        }
        if args.iter().any(|a| a.to_string_lossy().contains('\0'))
            || cwd.is_some_and(|p| !p.is_absolute())
        {
            return Err(error(ErrorCode::InvalidRequest, phase));
        }
        let mut command = tokio::process::Command::new(&self.executable);
        command
            .args(args)
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
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
            "GIT_GRAFT_FILE",
            "GIT_SHALLOW_FILE",
            "GIT_REPLACE_REF_BASE",
        ] {
            command.env_remove(variable);
        }
        if suppress_grafts {
            command.env(
                "GIT_GRAFT_FILE",
                if cfg!(windows) { "NUL" } else { "/dev/null" },
            );
        }
        #[cfg(windows)]
        command.creation_flags(0x00000004 | 0x08000000); // suspended, no console
        let mut child = command
            .spawn()
            .map_err(|_| error(ErrorCode::GitUnavailable, phase.clone()))?;
        #[cfg(windows)]
        let job = match owned_job::attach(&child) {
            Ok(job) => job,
            Err(failure) => {
                let _ = child.kill().await;
                return Err(match failure {
                    owned_job::AttachError::Capacity => {
                        error(ErrorCode::Interrupted, Phase::Recovery)
                    }
                    owned_job::AttachError::Setup => error(ErrorCode::Internal, phase),
                });
            }
        };
        #[cfg(windows)]
        let mut job_guard = owned_job::TerminationGuard::new(job.clone(), lifetime_guard);
        #[cfg(windows)]
        {
            let resumed = options
                .before_resume
                .map_or(Ok(()), |callback| callback(job.clone()))
                .and_then(|()| {
                    owned_job::resume(&child)
                        .map_err(|()| error(ErrorCode::Internal, phase.clone()))
                });
            if let Err(failure) = resumed {
                if job.stop_and_wait().await.is_err() {
                    return Err(error(ErrorCode::Interrupted, Phase::Recovery));
                }
                job_guard.confirmed();
                return Err(failure);
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
        let mut stdin = child.stdin.take();
        let run = async {
            let (status, out, err, ()) = tokio::try_join!(
                async {
                    let status = child.wait().await.map_err(|_| ErrorCode::Io)?;
                    // A detached descendant may retain inherited pipe handles.
                    // Stop it on parent exit, before waiting for pipe EOF.
                    #[cfg(windows)]
                    job.terminate();
                    Ok(status)
                },
                bounded(stdout, limit),
                bounded(stderr, limit),
                async move {
                    if let Some(bytes) = input {
                        let mut writer = stdin.take().ok_or(ErrorCode::Io)?;
                        writer.write_all(bytes).await.map_err(|_| ErrorCode::Io)?;
                        // Tokio Windows buffers a write on a blocking worker;
                        // shutdown alone is immediately ready. Observe the
                        // actual OS write result before reporting stdin sent.
                        writer.flush().await.map_err(|_| ErrorCode::Io)?;
                        writer.shutdown().await.map_err(|_| ErrorCode::Io)?;
                    }
                    Ok(())
                }
            )?;
            Ok::<_, ErrorCode>(GitOutput {
                stdout: out,
                stderr: err,
                success: status.success(),
                exit_code: status.code(),
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
            // No caller may mutate/clean staging on an unconfirmed tree stop.
            return Err(error(ErrorCode::Interrupted, Phase::Recovery));
        }
        #[cfg(windows)]
        job_guard.confirmed();
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
mod owned_job;
