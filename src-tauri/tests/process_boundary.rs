use opengit_lib::{
    domain::contracts::*,
    git::{
        error::git_failure,
        runner::{argv, Cancellation, GitRunner},
    },
    ipc::validation,
};
use std::{path::PathBuf, time::Duration};

#[test]
fn minimum_git_policy_parses_only_real_banners_and_checks_boundary() {
    use opengit_lib::git::version::{parse, GitVersion};
    for (banner, supported) in [
        ("git version 2.20.0", false),
        ("git version 2.30.99", false),
        ("git version 2.31.0", true),
        ("git version 2.56.0.windows.1\n", true),
        ("git version 3.0.0", true),
    ] {
        assert_eq!(parse(banner.as_bytes()).unwrap().supported(), supported);
    }
    assert_eq!(parse(b"git version 2.31.0"), Some(GitVersion(2, 31, 0)));
    for malformed in [
        "helper version 9.99.0",
        "git version nope",
        "git version 2.31",
        "git version 2.31.0.CANARY",
        "git version 999999999999999999.31.0",
        "git version 2.31.0\nCANARY",
    ] {
        assert!(parse(malformed.as_bytes()).is_none());
    }
}

#[cfg(windows)]
#[tokio::test]
async fn controlled_git_versions_gate_discovery_and_inspection_before_repository_commands() {
    use opengit_lib::{backend::Backend, git::version, repositories::discovery};
    let fixture = tempfile::tempdir().unwrap();
    let source = fixture.path().join("fake_git.rs");
    let executable = fixture.path().join("fake_git.exe");
    // A real controlled executable receives exactly the production argv. No
    // runner bypass, shell interpolation, or production fault API is added.
    std::fs::write(&source,r#"fn main() {
        let root=std::env::current_exe().unwrap().parent().unwrap().to_owned();
        let args:Vec<_>=std::env::args().skip(1).collect();
        if args==["--version"] {print!("{}",std::fs::read_to_string(root.join("version.txt")).unwrap());return;}
        if args.iter().any(|arg|arg=="rev-parse"||arg=="status"||arg=="clone") {std::fs::write(root.join("repository-command-spawned"),b"unexpected").unwrap();}
        std::process::exit(17);
    }"#).unwrap();
    let rustc = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .filter(|p| p.is_absolute())
        .map(|p| p.join("rustc.exe"))
        .find(|p| p.is_file())
        .unwrap();
    let compile = std::process::Command::new(rustc)
        .arg(&source)
        .args(["--edition", "2021", "-o"])
        .arg(&executable)
        .current_dir(fixture.path())
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "controlled fixture executable did not compile"
    );
    let real = GitRunner::discover().unwrap();
    let repository = fixture.path().join("repository");
    std::fs::create_dir(&repository).unwrap();
    real.run(
        Some(&repository),
        &argv(&["init"]),
        &Cancellation::default(),
        Phase::Discovery,
    )
    .await
    .unwrap();
    let data = fixture.path().join("database");
    let backend = Backend::with_git(&data, real).unwrap();
    let workspace = backend
        .register_repository("main", &repository)
        .await
        .unwrap();
    drop(backend);
    let runner = GitRunner::trusted(executable.clone()).unwrap();
    for (banner, expected_version, supported) in [
        ("git version 2.20.0", Some("2.20.0"), false),
        ("git version malformed CANARY", None, false),
        ("git version 2.31.0", Some("2.31.0"), true),
        ("git version 2.56.0.windows.1", Some("2.56.0"), true),
    ] {
        std::fs::write(fixture.path().join("version.txt"), banner).unwrap();
        let environment = opengit_lib::git::environment::environment(Some(&runner)).await;
        assert_eq!(environment.git.available, supported);
        assert_eq!(environment.git.version.as_deref(), expected_version);
        let checked = version::ensure_supported(&runner).await;
        if supported {
            assert!(checked.is_ok());
            continue;
        }
        let failure = checked.unwrap_err();
        assert_eq!(failure.code, ErrorCode::UnsupportedGit);
        assert_eq!(failure.retry, AppErrorRetry::Retry);
        assert!(failure.message.contains("2.31.0"));
        assert!(!failure.message.contains("CANARY"));
        assert_eq!(
            discovery::discover(&runner, &repository)
                .await
                .unwrap_err()
                .code,
            ErrorCode::UnsupportedGit
        );
        let backend = std::sync::Arc::new(Backend::with_git(&data, runner.clone()).unwrap());
        backend.get_workspace("main").unwrap();
        let selection = workspace.active_selection.clone().unwrap();
        assert_eq!(
            backend
                .register_repository("main", &repository)
                .await
                .unwrap_err()
                .code,
            ErrorCode::UnsupportedGit
        );
        let request = InspectRepositoryRequest {
            repo_id: selection.repo_id,
            worktree_id: selection.worktree_id,
            generation: "1".into(),
        };
        assert_eq!(
            backend
                .inspect_repository("main", request)
                .await
                .unwrap_err()
                .code,
            ErrorCode::UnsupportedGit
        );
        let destination = backend
            .grant_clone_destination("main", fixture.path())
            .unwrap();
        assert_eq!(
            backend
                .start_clone(
                    "main",
                    StartCloneRequest {
                        source: CloneSource::Remote {
                            url: "https://example.invalid/never-contacted".into()
                        },
                        destination_handle: destination.destination_handle,
                        directory_name: "must-not-clone".into(),
                    }
                )
                .await
                .unwrap_err()
                .code,
            ErrorCode::UnsupportedGit
        );
        assert!(backend.scheduler.snapshot().unwrap().operations.is_empty());
        assert!(!fixture.path().join("must-not-clone").exists());
        assert!(
            !std::fs::read_dir(fixture.path()).unwrap().any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".opengit-stage-"))
        );
        assert!(!fixture.path().join("repository-command-spawned").exists());
    }
    std::fs::remove_file(&executable).unwrap();
    let environment = opengit_lib::git::environment::environment(Some(&runner)).await;
    assert!(!environment.git.available);
    assert!(environment.git.version.is_none());
    assert_eq!(
        version::ensure_supported(&runner).await.unwrap_err().code,
        ErrorCode::GitUnavailable
    );
}

#[tokio::test]
async fn real_git_success_literal_arguments_and_safe_errors() {
    let runner = GitRunner::discover().unwrap();
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(
        folder.path().join("git.exe"),
        b"repository executable must not be searched",
    )
    .unwrap();
    runner
        .run(
            Some(folder.path()),
            &argv(&["--version"]),
            &Cancellation::default(),
            Phase::Discovery,
        )
        .await
        .unwrap();
    assert!(GitRunner::trusted(folder.path().join("missing.exe")).is_err());
    let args = vec![
        "init".into(),
        "--bare".into(),
        folder.path().join("literal & name Ω.git").into_os_string(),
    ];
    runner
        .run(None, &args, &Cancellation::default(), Phase::Discovery)
        .await
        .unwrap();
    assert!(folder.path().join("literal & name Ω.git/HEAD").is_file());
    let failure = git_failure(
        b"fatal CANARY_SECRET https://user:CANARY_SECRET@example.invalid authentication failed",
        Phase::Transfer,
    );
    assert_eq!(failure.code, ErrorCode::AuthenticationFailed);
    assert!(!serde_json::to_string(&failure).unwrap().contains("CANARY"));
    assert!(GitRunner::trusted(PathBuf::from("git.exe")).is_err());
    assert!(validation::counter("-1").is_err());
    assert!(validation::counter("18446744073709551616").is_err());
    assert!(validation::request_size(&"x".repeat(17000)).is_err());
}

#[tokio::test]
async fn output_cap_and_pre_cancel() {
    let mut runner = GitRunner::discover().unwrap();
    runner.output_limit = 2;
    assert_eq!(
        runner
            .run(
                None,
                &argv(&["--version"]),
                &Cancellation::default(),
                Phase::Discovery
            )
            .await
            .unwrap_err()
            .code,
        ErrorCode::OutputLimit
    );
    let cancelled = Cancellation::default();
    cancelled.cancel();
    assert_eq!(
        runner
            .run(None, &argv(&["--version"]), &cancelled, Phase::Discovery)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Interrupted
    );
}
#[tokio::test]
async fn environment_reports_actual_probe_results_without_identity_values() {
    let runner = GitRunner::discover().unwrap();
    let environment = opengit_lib::git::environment::environment(Some(&runner)).await;
    assert!(environment.git.available);
    let version = environment.git.version.unwrap();
    assert!(version.bytes().all(|b| b.is_ascii_digit() || b == b'.'));
    assert!(!environment.web_view2.available);
    let unavailable = opengit_lib::git::environment::environment(None).await;
    assert!(!unavailable.git.available);
    assert!(!unavailable.identity.name_configured);
    assert!(!unavailable.identity.email_configured);
}

#[cfg(windows)]
#[tokio::test]
async fn owned_descendant_timeout_and_cancel_leave_unrelated_process_alive() {
    // Use the trusted PowerShell executable as a fake Git; no shell user input.
    let exe = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let fixture = tempfile::tempdir().unwrap();
    let marker = fixture.path().join("descendant.txt");
    let script = fixture.path().join("fixture.ps1");
    let body = format!("$child = Start-Process -WindowStyle Hidden -PassThru powershell.exe -ArgumentList '-NoProfile','-Command','Start-Sleep -Seconds 4; Set-Content -LiteralPath ''{}'' -Value escaped'; Start-Sleep -Seconds 60", marker.display());
    std::fs::write(&script, body).unwrap();
    let mut unrelated = tokio::process::Command::new(&exe)
        .args(["-NoProfile", "-Command", "Start-Sleep -Seconds 30"])
        .creation_flags(0x08000000)
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut runner = GitRunner::trusted(exe).unwrap();
    runner.timeout = Duration::from_millis(1000);
    let args = vec![
        "-NoProfile".into(),
        "-ExecutionPolicy".into(),
        "Bypass".into(),
        "-File".into(),
        script.into_os_string(),
    ];
    assert_eq!(
        runner
            .run(None, &args, &Cancellation::default(), Phase::Inspection)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Timeout
    );
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert!(!marker.exists(), "owned descendant escaped job");
    assert!(
        unrelated.try_wait().unwrap().is_none(),
        "unrelated process was terminated"
    );
    let cancel = Cancellation::default();
    let trigger = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        trigger.cancel();
    });
    assert_eq!(
        runner
            .run(None, &args, &cancel, Phase::Inspection)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Interrupted
    );
    assert!(GitRunner::discover()
        .unwrap()
        .run(
            None,
            &argv(&["--version"]),
            &Cancellation::default(),
            Phase::Discovery
        )
        .await
        .is_ok());
    assert!(unrelated.try_wait().unwrap().is_none());
    unrelated.kill().await.unwrap();
}

#[cfg(windows)]
#[tokio::test]
async fn successful_parent_exit_waits_for_detached_owned_tree_but_preserves_bystander() {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::{
        Foundation::{CloseHandle, WAIT_OBJECT_0},
        System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE},
    };
    struct Held(windows_sys::Win32::Foundation::HANDLE);
    impl Drop for Held {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    let fixture = tempfile::tempdir().unwrap();
    let source = fixture.path().join("detached.rs");
    let executable = fixture.path().join("detached fixture.exe");
    std::fs::write(&source, r#"use std::{process::{Command,Stdio},time::Duration,os::windows::fs::OpenOptionsExt};
fn wait(path:&std::path::Path){while !path.exists(){std::thread::sleep(Duration::from_millis(1));}}
fn spawn(mode:&str){Command::new(std::env::current_exe().unwrap()).arg(mode).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();}
fn main(){let root=std::env::current_exe().unwrap().parent().unwrap().to_owned();let mode=std::env::args().nth(1).unwrap();
if mode=="bystander"{std::thread::sleep(Duration::from_secs(30));return;}
if mode=="parent"{spawn("child");wait(&root.join("release-parent"));return;}
let _lease=std::fs::OpenOptions::new().create(true).write(true).truncate(true).share_mode(0).open(root.join(format!("{mode}.lock"))).unwrap();
std::fs::write(root.join(format!("{mode}.pid")),std::process::id().to_string()).unwrap();
if mode=="child"{wait(&root.join("spawn-grandchild"));spawn("grandchild");}
std::thread::sleep(Duration::from_secs(2));std::fs::write(root.join(format!("{mode}.late-write")),b"unexpected").unwrap();}"#).unwrap();
    let rustc = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .filter(|p| p.is_absolute())
        .map(|p| p.join("rustc.exe"))
        .find(|p| p.is_file())
        .unwrap();
    assert!(std::process::Command::new(rustc)
        .arg(&source)
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap()
        .status
        .success());
    let mut bystander = tokio::process::Command::new(&executable)
        .arg("bystander")
        .creation_flags(0x08000000)
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let runner = GitRunner::trusted(executable).unwrap();
    let run = tokio::spawn(async move {
        runner
            .run(
                None,
                &argv(&["parent"]),
                &Cancellation::default(),
                Phase::Transfer,
            )
            .await
    });
    let mut handles = Vec::new();
    for name in ["child", "grandchild"] {
        let pid_file = fixture.path().join(format!("{name}.pid"));
        tokio::time::timeout(Duration::from_secs(10), async {
            while !pid_file.is_file() {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
        let pid: u32 = std::fs::read_to_string(pid_file).unwrap().parse().unwrap();
        let handle = Held(unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) });
        assert!(
            !handle.0.is_null(),
            "original descendant handle must be opened before termination"
        );
        assert!(std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(fixture.path().join(format!("{name}.lock")))
            .is_err());
        handles.push(handle);
        if name == "child" {
            std::fs::write(fixture.path().join("spawn-grandchild"), b"go").unwrap();
        }
    }
    std::fs::write(fixture.path().join("release-parent"), b"go").unwrap();
    run.await.unwrap().unwrap();
    // No grace sleep/reopen-by-PID: the original handles and exclusive file
    // access must already be ready on the runner's return boundary.
    for (name, handle) in ["child", "grandchild"].into_iter().zip(&handles) {
        assert_eq!(
            unsafe { WaitForSingleObject(handle.0, 0) },
            WAIT_OBJECT_0,
            "runner returned before {name} signaled"
        );
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(fixture.path().join(format!("{name}.lock")))
            .unwrap();
        assert!(!fixture.path().join(format!("{name}.late-write")).exists());
    }
    assert!(bystander.try_wait().unwrap().is_none());
    tokio::time::sleep(Duration::from_millis(2200)).await;
    assert!(!fixture.path().join("child.late-write").exists());
    assert!(!fixture.path().join("grandchild.late-write").exists());
    assert!(bystander.try_wait().unwrap().is_none());
    bystander.kill().await.unwrap();
}
