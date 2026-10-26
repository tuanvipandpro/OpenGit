#![allow(dead_code)]
use opengit_lib::{backend::Backend, domain::contracts::*};
use std::{path::PathBuf, sync::Arc};
pub struct Fixture {
    pub backend: Arc<Backend>,
    pub history_scope: HistoryScope,
    pub scope: ActionScope,
    pub remote: PathBuf,
    pub git: super::history::Fixture,
}
impl Fixture {
    pub async fn new(format: &str, count: usize, ref_format: &str) -> Self {
        let git = super::history::Fixture::with_ref_format(format, count, ref_format);
        let remote = git.owner.child("daily remote Ω = ' .git").unwrap();
        git.run(
            &[
                "clone",
                "--bare",
                "--",
                &git.git_path(&git.path),
                &git.git_path(&remote),
            ],
            None,
        );
        git.run(&["remote", "add", "origin", &git.git_path(&remote)], None);
        git.run(&["branch", "--set-upstream-to=origin/main", "main"], None);
        let (backend, history_scope) = git.backend().await;
        let scope = ActionScope {
            repo_id: history_scope.repo_id.clone(),
            worktree_id: history_scope.worktree_id.clone(),
            generation: history_scope.generation.clone(),
        };
        Self {
            backend: Arc::new(backend),
            history_scope,
            scope,
            remote,
            git,
        }
    }
    pub async fn catalog(&self) -> RepositoryActionCatalog {
        self.backend
            .get_repository_actions(
                "main",
                GetRepositoryActionsRequest {
                    scope: self.scope.clone(),
                },
            )
            .await
            .unwrap()
    }
    pub async fn rebind(
        &mut self,
        runner: opengit_lib::git::runner::GitRunner,
        timeout: Option<std::time::Duration>,
    ) {
        let database = self
            .git
            .owner
            .child(&format!("daily-rebind-{}", uuid::Uuid::new_v4().simple()))
            .unwrap();
        let mut backend = Backend::with_git(&database, runner).unwrap();
        if let Some(timeout) = timeout {
            backend.actions = opengit_lib::actions::Actions::with_fetch_timeout(timeout).unwrap();
        }
        let selection = backend
            .register_repository("main", &self.git.path)
            .await
            .unwrap()
            .active_selection
            .unwrap();
        self.scope = ActionScope {
            repo_id: selection.repo_id.clone(),
            worktree_id: selection.worktree_id.clone(),
            generation: "1".into(),
        };
        self.history_scope = HistoryScope {
            repo_id: selection.repo_id,
            worktree_id: selection.worktree_id,
            generation: "1".into(),
        };
        self.backend = Arc::new(backend);
    }
    pub fn advance(&self, reference: &str) -> String {
        let old = String::from_utf8(
            self.git
                .run_at(&self.remote, &["rev-parse", reference], None),
        )
        .unwrap()
        .trim()
        .to_owned();
        let tree = String::from_utf8(self.git.run_at(
            &self.remote,
            &["rev-parse", &format!("{reference}^{{tree}}")],
            None,
        ))
        .unwrap()
        .trim()
        .to_owned();
        let new = String::from_utf8(self.git.run_at(
            &self.remote,
            &["commit-tree", &tree, "-p", &old],
            Some(b"Remote fixture next\n"),
        ))
        .unwrap()
        .trim()
        .to_owned();
        self.git
            .run_at(&self.remote, &["update-ref", reference, &new, &old], None);
        new
    }
    pub async fn prepare_fetch(&self, names: &[&str], prune: bool) -> PreparedAction {
        let catalog = self.catalog().await;
        let handles = names
            .iter()
            .map(|name| {
                catalog
                    .remotes
                    .iter()
                    .find(|r| r.name == *name)
                    .unwrap()
                    .handle
                    .clone()
                    .unwrap()
            })
            .collect();
        self.backend
            .prepare_repository_action(
                "main",
                PrepareRepositoryActionRequest {
                    scope: self.scope.clone(),
                    intent: ActionIntent::Fetch {
                        remote_handles: handles,
                        prune,
                    },
                },
            )
            .await
            .unwrap()
    }
    pub async fn fetch(
        &self,
        names: &[&str],
        prune: bool,
    ) -> (PreparedAction, ActionOperationResult) {
        let plan = self.prepare_fetch(names, prune).await;
        let operation = self
            .backend
            .execute_repository_action(
                "main",
                ExecuteRepositoryActionRequest {
                    plan_id: plan.plan_id.clone(),
                },
            )
            .unwrap();
        (plan, self.wait(&operation.id).await)
    }
    pub async fn wait(&self, id: &str) -> ActionOperationResult {
        tokio::time::timeout(std::time::Duration::from_secs(60), async {
            loop {
                let result = self
                    .backend
                    .get_action_outcome(
                        "main",
                        GetActionOutcomeRequest {
                            operation_id: id.into(),
                        },
                    )
                    .unwrap();
                if opengit_lib::operations::state::terminal(&result.operation.state) {
                    return result;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("owned fetch deadline")
    }
    pub fn worktree_state(&self) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        (
            self.git.run(&["status", "--porcelain=v2", "-z"], None),
            std::fs::read(self.git.path.join(".git/index")).unwrap(),
            std::fs::read(self.git.path.join(".git/HEAD")).unwrap(),
        )
    }
}
pub struct Proxy {
    pub runner: opengit_lib::git::runner::GitRunner,
    pub mode: PathBuf,
    pub ready: PathBuf,
    pub release: PathBuf,
    pub fetched: PathBuf,
}
impl Proxy {
    pub fn new(f: &super::history::Fixture) -> Self {
        let mode = f.owner.child("fetch-mode").unwrap();
        let ready = f.owner.child("fetch-ready").unwrap();
        let release = f.owner.child("fetch-release").unwrap();
        let fetched = f.owner.child("fetch-completed").unwrap();
        std::fs::write(&mode, "normal").unwrap();
        let source = f.owner.child("fetch-proxy.rs").unwrap();
        let binary = f.owner.child("fetch proxy Ω = ' test.exe").unwrap();
        let program = format!(
            r#"
use std::io::Write;
fn main(){{let args=std::env::args_os().skip(1).collect::<Vec<_>>();let mode=std::fs::read_to_string({mode:?}).unwrap();
let fetch=args.iter().any(|v|v=="fetch");let dry=args.iter().any(|v|v=="--dry-run");
let read=args.iter().any(|v|v=="ls-remote");
if mode=="amend-shallow-hold"&&args.iter().any(|v|v=="--is-shallow-repository"){{
let output=std::process::Command::new({git:?}).args(&args).output().unwrap();
std::fs::write({ready:?},b"shallow read completed").unwrap();while !std::path::Path::new({release:?}).exists(){{std::thread::sleep(std::time::Duration::from_millis(5));}}
std::io::stdout().write_all(&output.stdout).unwrap();std::io::stderr().write_all(&output.stderr).unwrap();std::process::exit(output.status.code().unwrap_or(1));
}}
if args.iter().any(|v|v=="push")&&(mode=="push-lost-ack"||mode=="push-applied-hold"){{
let output=std::process::Command::new({git:?}).args(&args).output().unwrap();
if output.status.success(){{let mut marker=std::fs::OpenOptions::new().create(true).append(true).open({fetched:?}).unwrap();marker.write_all(b"pushed\n").unwrap();}}
if mode=="push-applied-hold"{{std::fs::write({ready:?},b"ready").unwrap();while !std::path::Path::new({release:?}).exists(){{std::thread::sleep(std::time::Duration::from_millis(5));}}}}
std::process::exit(1);
}}
if read{{match mode.as_str(){{
"remote-auth"=>{{eprintln!("authentication failed CANARY");std::process::exit(128)}},
"remote-malformed"=>{{print!("bad CANARY");return}},
"remote-output"=>{{let mut out=std::io::stdout().lock();for _ in 0..140{{let _=out.write_all(&[b'x';65536]);}}return}},
"remote-hold"=>{{std::fs::write({ready:?},b"ready").unwrap();while !std::path::Path::new({release:?}).exists(){{std::thread::sleep(std::time::Duration::from_millis(5));}}}},_=>()}}}}

if fetch&&dry{{match mode.as_str(){{"old"=>{{eprintln!("unknown porcelain CANARY");std::process::exit(129)}},"malformed"=>{{print!("bad CANARY");return}},"permission"=>{{eprintln!("permission denied CANARY");std::process::exit(128)}},_=>()}}}}
if mode=="cleanupfail"&&args.iter().any(|v|v=="--remove-section"){{eprintln!("configuration write refused CANARY");std::process::exit(1)}}
if mode=="localhold"&&(args.iter().any(|v|v=="branch")||args.iter().any(|v|v=="tag")||args.iter().any(|v|v=="update-ref")){{std::fs::write({ready:?},b"ready").unwrap();while !std::path::Path::new({release:?}).exists(){{std::thread::sleep(std::time::Duration::from_millis(5));}}}}
if mode=="canary"{{std::fs::write({ready:?},b"unexpected lazy fetch").unwrap();std::process::exit(1)}}
if mode=="cataloghold"&&args.iter().any(|v|v=="config")&&args.iter().any(|v|v=="--list")&&!args.iter().any(|v|v=="--show-origin"){{std::fs::write({ready:?},b"ready").unwrap();while !std::path::Path::new({release:?}).exists(){{std::thread::sleep(std::time::Duration::from_millis(5));}}}}
if mode=="stash-prewrite-hold"&&args.iter().any(|v|v=="hash-object"){{std::fs::write({ready:?},b"ready").unwrap();while !std::path::Path::new({release:?}).exists(){{std::thread::sleep(std::time::Duration::from_millis(5));}}}}
if mode=="stash-after-pin-hold"&&args.iter().any(|v|v=="update-ref")&&args.iter().any(|v|v.to_string_lossy().starts_with("refs/opengit/recovery/")){{let output=std::process::Command::new({git:?}).args(&args).output().unwrap();std::io::stdout().write_all(&output.stdout).unwrap();std::io::stderr().write_all(&output.stderr).unwrap();if output.status.success()&&!std::path::Path::new({fetched:?}).exists(){{std::fs::write({fetched:?},b"pin created").unwrap();}}std::process::exit(output.status.code().unwrap_or(1));}}
if mode=="stash-after-pin-hold"&&std::path::Path::new({fetched:?}).exists()&&args.iter().any(|v|v=="ls-files")&&args.iter().any(|v|v=="--ignored"){{let output=std::process::Command::new({git:?}).args(&args).output().unwrap();std::fs::write({ready:?},b"collision list read").unwrap();while !std::path::Path::new({release:?}).exists(){{std::thread::sleep(std::time::Duration::from_millis(5));}}std::io::stdout().write_all(&output.stdout).unwrap();std::io::stderr().write_all(&output.stderr).unwrap();std::process::exit(output.status.code().unwrap_or(1));}}
if mode=="stash-after-pin-hold"&&args.iter().any(|v|v=="stash")&&args.iter().any(|v|v=="apply"){{let mut marker=std::fs::OpenOptions::new().append(true).open({fetched:?}).unwrap();marker.write_all(b"\napply resumed").unwrap();}}
if fetch&&!dry{{match mode.as_str(){{
"auth"=>{{eprintln!("authentication failed CANARY");std::process::exit(128)}},
"output"=>{{let mut out=std::io::stdout().lock();let block=[b'x';65536];for _ in 0..140{{let _=out.write_all(&block);}}return}},
"hold"=>{{std::fs::write({ready:?},b"ready").unwrap();while !std::path::Path::new({release:?}).exists(){{std::thread::sleep(std::time::Duration::from_millis(5));}}}},_=>()}}
}}
if mode=="verifyhold"&&std::path::Path::new({fetched:?}).exists()&&args.iter().any(|v|v=="worktree"){{std::fs::write({ready:?},b"ready").unwrap();while !std::path::Path::new({release:?}).exists(){{std::thread::sleep(std::time::Duration::from_millis(5));}}}}
let mut command=std::process::Command::new({git:?});command.args(&args);
if mode=="amend-noop"&&args.iter().any(|v|v=="--amend"){{let at=args.iter().position(|v|v=="commit").unwrap();let out=std::process::Command::new({git:?}).args(&args[..at]).args(["show","-s","--format=%cI","HEAD"]).output().unwrap();assert!(out.status.success());command.env("GIT_COMMITTER_DATE",String::from_utf8(out.stdout).unwrap().trim());}}
let status=command.status().unwrap();
if fetch&&!dry&&status.success(){{let mut file=std::fs::OpenOptions::new().create(true).append(true).open({fetched:?}).unwrap();file.write_all(b"applied\n").unwrap();}}
std::process::exit(status.code().unwrap_or(1));}}
"#,
            mode = mode.to_str().unwrap(),
            ready = ready.to_str().unwrap(),
            release = release.to_str().unwrap(),
            fetched = fetched.to_str().unwrap(),
            git = f.git.executable().to_str().unwrap()
        );
        std::fs::write(&source, program).unwrap();
        let rustc = std::env::split_paths(&std::env::var_os("PATH").unwrap())
            .filter(|p| p.is_absolute())
            .map(|p| p.join("rustc.exe"))
            .find(|p| p.is_file())
            .unwrap();
        assert!(std::process::Command::new(rustc)
            .arg(&source)
            .args(["--edition=2021", "-o"])
            .arg(&binary)
            .status()
            .unwrap()
            .success());
        Self {
            runner: opengit_lib::git::runner::GitRunner::trusted(binary).unwrap(),
            mode,
            ready,
            release,
            fetched,
        }
    }
    pub fn set(&self, mode: &str) {
        std::fs::write(&self.mode, mode).unwrap();
    }
    pub async fn wait_ready(&self) {
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            while !self.ready.exists() {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }
    pub fn release(&self) {
        std::fs::write(&self.release, b"release").unwrap();
    }
}
pub fn metadata_tree(
    root: &std::path::Path,
) -> std::collections::BTreeMap<PathBuf, (bool, u64, std::time::SystemTime, Vec<u8>)> {
    fn walk(
        root: &std::path::Path,
        path: &std::path::Path,
        result: &mut std::collections::BTreeMap<
            PathBuf,
            (bool, u64, std::time::SystemTime, Vec<u8>),
        >,
    ) {
        for item in std::fs::read_dir(path).unwrap() {
            let path = item.unwrap().path();
            let meta = std::fs::symlink_metadata(&path).unwrap();
            assert!(!meta.file_type().is_symlink());
            result.insert(
                path.strip_prefix(root).unwrap().into(),
                (
                    meta.is_dir(),
                    if meta.is_file() { meta.len() } else { 0 },
                    meta.modified().unwrap(),
                    if meta.is_file() {
                        std::fs::read(&path).unwrap()
                    } else {
                        vec![]
                    },
                ),
            );
            if meta.is_dir() {
                walk(root, &path, result);
            }
        }
    }
    let mut result = Default::default();
    walk(root, root, &mut result);
    result
}
