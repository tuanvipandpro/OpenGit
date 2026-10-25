#![allow(dead_code)]
use opengit_lib::{backend::Backend, domain::contracts::*, git::runner::GitRunner};
use std::{
    collections::BTreeMap,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

pub struct Fixture {
    pub owner: super::OwnedFixture,
    pub path: PathBuf,
    pub git: GitRunner,
    pub count: usize,
}
impl Fixture {
    pub fn git_path(&self, path: &Path) -> String {
        assert!(path.starts_with(self.owner.root()));
        #[cfg(windows)]
        {
            let path = if path.exists() {
                PathBuf::from(opengit_lib::clone::path::git_path(path).unwrap())
            } else {
                PathBuf::from(opengit_lib::clone::path::git_path(path.parent().unwrap()).unwrap())
                    .join(path.file_name().unwrap())
            };
            path.to_str().unwrap().into()
        }
        #[cfg(not(windows))]
        {
            path.to_str().unwrap().into()
        }
    }
    pub fn new(format: &str, count: usize) -> Self {
        Self::with_ref_format(format, count, "files")
    }
    pub fn with_ref_format(format: &str, count: usize, ref_format: &str) -> Self {
        Self::build(format, count, ref_format, 3, 1)
    }
    pub fn shallow_native_shape(format: &str, count: usize) -> Self {
        Self::build(format, count, "files", 113, 3)
    }
    fn build(
        format: &str,
        count: usize,
        ref_format: &str,
        date_modulus: usize,
        beta_distance: usize,
    ) -> Self {
        assert!(count >= 6);
        let owner = super::OwnedFixture::new().unwrap();
        let path = owner.child("history Ω").unwrap();
        std::fs::create_dir(&path).unwrap();
        let git = GitRunner::discover().unwrap();
        let fixture = Self {
            owner,
            path,
            git,
            count,
        };
        fixture.run(
            &[
                "init",
                "-b",
                "main",
                &format!("--object-format={format}"),
                &format!("--ref-format={ref_format}"),
            ],
            None,
        );
        let mut stream = Vec::new();
        let emit = |stream: &mut Vec<u8>,
                    reference: &str,
                    mark: usize,
                    parents: &[usize],
                    message: &str,
                    author: &str| {
            let time = 1_700_000_000 + (mark * 37) % date_modulus;
            write!(stream, "commit {reference}\nmark :{mark}\nauthor {author} <fixture@example.invalid> {time} +0000\ncommitter Committer <committer@example.invalid> {time} +0000\ndata {}\n{message}\n", message.len()).unwrap();
            for (i, parent) in parents.iter().enumerate() {
                writeln!(
                    stream,
                    "{} :{parent}",
                    if i == 0 { "from" } else { "merge" }
                )
                .unwrap();
            }
            stream.extend_from_slice(b"\n");
        };
        for i in 1..=count {
            emit(
                &mut stream,
                "refs/heads/main",
                i,
                &if i == 1 { vec![] } else { vec![i - 1] },
                if i == 1 {
                    "DeepNeedle\n\nFull message with literal [.*]"
                } else {
                    "History commit"
                },
                if i == 2 { "AuthorNeedle" } else { "Fixture" },
            );
        }
        emit(
            &mut stream,
            "refs/heads/alpha",
            count + 1,
            &[count / 2],
            "Alpha",
            "Fixture",
        );
        emit(
            &mut stream,
            "refs/heads/beta",
            count + 2,
            &[count / 2 - beta_distance],
            "Beta",
            "Fixture",
        );
        emit(
            &mut stream,
            "refs/heads/main",
            count + 3,
            &[count, count + 1, count + 2],
            "Octopus",
            "Fixture",
        );
        emit(
            &mut stream,
            "refs/heads/unmerged",
            count + 4,
            &[count / 2],
            "UnmergedNeedle",
            "Fixture",
        );
        emit(
            &mut stream,
            "refs/heads/orphan",
            count + 5,
            &[],
            "Unrelated root",
            "Fixture",
        );
        stream.extend_from_slice(b"done\n");
        fixture.run(&["fast-import", "--quiet"], Some(&stream));
        fixture.run(&["checkout", "-f", "main"], None);
        fixture.run(&["update-ref", "refs/remotes/origin/main", "main"], None);
        fixture.run(
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/main",
            ],
            None,
        );
        fixture.run(
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "tag",
                "-a",
                "v1",
                "-m",
                "tag",
                "main~2",
            ],
            None,
        );
        let blob =
            String::from_utf8(fixture.run(&["hash-object", "-w", "--stdin"], Some(b"blob tag")))
                .unwrap();
        fixture.run(&["update-ref", "refs/tags/blob", blob.trim()], None);
        fixture
    }
    pub fn command(&self, cwd: &Path) -> Command {
        assert!(cwd.starts_with(self.owner.root()));
        let mut command = Command::new(self.git.executable());
        command
            .arg("-C")
            .arg(cwd)
            .args(["-c", "commit.gpgSign=false"])
            .env(
                "GIT_CONFIG_GLOBAL",
                if cfg!(windows) { "NUL" } else { "/dev/null" },
            )
            .env(
                "GIT_CONFIG_SYSTEM",
                if cfg!(windows) { "NUL" } else { "/dev/null" },
            )
            .env("GIT_CONFIG_COUNT", "0")
            .env_remove("GIT_CONFIG_PARAMETERS")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_AUTHOR_NAME", "Fixture")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
            .env("GIT_COMMITTER_NAME", "Fixture")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid");
        for key in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_COMMON_DIR",
            "GIT_INDEX_FILE",
            "GIT_OBJECT_DIRECTORY",
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            "GIT_NAMESPACE",
        ] {
            command.env_remove(key);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        command
    }
    pub fn run_at(&self, cwd: &Path, args: &[&str], input: Option<&[u8]>) -> Vec<u8> {
        let mut command = self.command(cwd);
        command
            .args(args)
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().unwrap();
        if let Some(input) = input {
            child.stdin.take().unwrap().write_all(input).unwrap();
        }
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "owned fixture command failed: {}",
            args[0]
        );
        output.stdout
    }
    pub fn run(&self, args: &[&str], input: Option<&[u8]>) -> Vec<u8> {
        self.run_at(&self.path, args, input)
    }
    pub fn oid(&self, revision: &str) -> String {
        String::from_utf8(self.run(&["rev-parse", revision], None))
            .unwrap()
            .trim()
            .into()
    }
    pub fn oracle(&self, roots: &[&str]) -> BTreeMap<String, Vec<String>> {
        let mut args = vec!["rev-list", "--topo-order", "--parents"];
        args.extend_from_slice(roots);
        String::from_utf8(self.run(&args, None))
            .unwrap()
            .lines()
            .map(|line| {
                let mut parts = line.split(' ');
                (
                    parts.next().unwrap().into(),
                    parts.map(str::to_owned).collect(),
                )
            })
            .collect()
    }
    pub fn snapshot(&self) -> (Vec<u8>, Option<Vec<u8>>, Vec<u8>) {
        self.snapshot_at(&self.path)
    }
    pub fn snapshot_at(&self, path: &Path) -> (Vec<u8>, Option<Vec<u8>>, Vec<u8>) {
        (
            self.run_at(
                path,
                &["for-each-ref", "--format=%(refname) %(objectname)"],
                None,
            ),
            std::fs::read(path.join(".git/index")).ok(),
            self.run_at(
                path,
                &["--no-optional-locks", "status", "--porcelain=v2", "-z"],
                None,
            ),
        )
    }
    pub fn order_at(&self, path: &Path, roots: &[String]) -> Vec<String> {
        let input = roots.iter().map(|id| format!("{id}\n")).collect::<String>();
        String::from_utf8(self.run_at(
            path,
            &["rev-list", "--topo-order", "--stdin"],
            Some(input.as_bytes()),
        ))
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
    }
    pub fn raw_parent_oracle_at(
        &self,
        path: &Path,
        children: &[String],
    ) -> BTreeMap<String, Vec<String>> {
        children
            .iter()
            .map(|id| {
                let raw = self.run_at(path, &["cat-file", "commit", id], None);
                let header = raw.windows(2).position(|w| w == b"\n\n").unwrap();
                let parents = std::str::from_utf8(&raw[..header])
                    .unwrap()
                    .lines()
                    .filter_map(|line| line.strip_prefix("parent ").map(str::to_owned))
                    .collect();
                (id.clone(), parents)
            })
            .collect()
    }
    pub async fn backend(&self) -> (Backend, HistoryScope) {
        self.backend_at(&self.path, "main").await
    }
    pub async fn backend_at(&self, path: &Path, window: &str) -> (Backend, HistoryScope) {
        let database = self
            .owner
            .child(&format!("database-{}", uuid::Uuid::new_v4().simple()))
            .unwrap();
        let backend = Backend::with_git(&database, self.git.clone()).unwrap();
        let workspace = backend.register_repository(window, path).await.unwrap();
        let selection = workspace.active_selection.unwrap();
        (
            backend,
            HistoryScope {
                repo_id: selection.repo_id,
                worktree_id: selection.worktree_id,
                generation: "1".into(),
            },
        )
    }
}
pub async fn open(
    backend: &Backend,
    scope: &HistoryScope,
    size: u32,
    filter: HistoryFilter,
) -> HistorySession {
    backend
        .open_history(
            "main",
            OpenHistoryRequest {
                scope: scope.clone(),
                filter,
                page_size: Some(size),
            },
        )
        .await
        .unwrap()
}
pub fn filter() -> HistoryFilter {
    HistoryFilter {
        hidden_refs: Vec::new(),
        solo_refs: Vec::new(),
    }
}
pub async fn collect(backend: &Backend, session: &HistorySession, size: u32) -> Vec<CommitSummary> {
    let mut commits = session.page.commits.clone();
    let mut cursor = session.page.next_cursor.clone();
    while let Some(next) = cursor {
        let page = backend
            .load_history_page(
                "main",
                LoadHistoryPageRequest {
                    context: session.context.clone(),
                    session_id: session.session_id.clone(),
                    cursor: next,
                    page_size: Some(size),
                },
            )
            .await
            .unwrap();
        cursor = page.next_cursor;
        commits.extend(page.commits);
    }
    commits
}
pub fn assert_dag(commits: &[CommitSummary], oracle: &BTreeMap<String, Vec<String>>) {
    let actual: BTreeMap<_, _> = commits
        .iter()
        .map(|commit| {
            (
                commit.oid.clone(),
                commit
                    .parents
                    .iter()
                    .map(|p| p.oid.clone())
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    assert_eq!(
        commits.len(),
        actual.len(),
        "no duplicate commit across pages"
    );
    assert_eq!(&actual, oracle);
    let positions: BTreeMap<_, _> = commits
        .iter()
        .enumerate()
        .map(|(i, c)| (&c.oid, i))
        .collect();
    for commit in commits {
        for parent in &commit.parents {
            if let Some(parent_position) = positions.get(&parent.oid) {
                assert!(
                    *parent_position > positions[&commit.oid],
                    "parent must follow child, independent of timestamps"
                );
            }
        }
    }
}
