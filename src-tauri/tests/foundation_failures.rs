mod support;
use opengit_lib::{
    domain::contracts::*,
    git::runner::{argv, Cancellation},
    operations::scheduler::{OperationSpec, Scheduler},
    persistence::workspace::Database,
    repositories::{discovery, registry::Registry},
};
use std::{fs, path::Path, sync::Arc};
use support::{faults, repos::GitFixtures, OwnedFixture, SYNTHETIC_SECRET};

#[test]
fn cleanup_requires_original_root_identity_and_marker_and_rejects_source_checkout() {
    let mut fixture = OwnedFixture::new().unwrap();
    let root = fixture.root().to_path_buf();
    let source = fixture
        .sentinel("source.sentinel", b"source must survive a rejected cleanup")
        .unwrap();
    let mut outside = OwnedFixture::new().unwrap();
    let destination = outside
        .sentinel("destination.sentinel", b"outside data")
        .unwrap();
    assert!(fixture.cleanup_target(outside.root()).is_err());
    assert!(fixture.cleanup_target(root.parent().unwrap()).is_err());
    let checkout = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    assert!(fixture.cleanup_target(checkout).is_err());
    assert!(source.unchanged().unwrap());
    assert!(destination.unchanged().unwrap());
    let marker = fs::read(fixture.marker_path()).unwrap();
    fs::write(fixture.marker_path(), b"copied or altered marker").unwrap();
    assert!(fixture.cleanup().is_err());
    assert!(source.unchanged().unwrap());
    fs::write(fixture.marker_path(), marker).unwrap();
    assert!(fixture.child("../outside").is_err());
    assert!(fixture.child("C:\\outside").is_err());
    fixture.cleanup().unwrap();
    assert!(!root.exists());
    assert!(destination.unchanged().unwrap());
    outside.cleanup().unwrap();
}

#[tokio::test]
async fn disposable_git_repositories_and_database_faults_preserve_sentinels() {
    let mut fixture = OwnedFixture::new().unwrap();
    let source = fixture
        .sentinel("source.sentinel", b"source baseline")
        .unwrap();
    let destination = fixture
        .sentinel("destination.sentinel", b"target baseline")
        .unwrap();
    let git = GitFixtures::create(&fixture).await.unwrap();
    assert!(git.working.join(".git/HEAD").is_file());
    assert!(git.bare.join("HEAD").is_file());
    let data = fixture.child("data").unwrap();
    let registry = Registry {
        database: Arc::new(Database::open(&data).unwrap()),
    };
    let discovered = discovery::discover(&git.runner, &git.working)
        .await
        .unwrap();
    let workspace = registry.register(&discovered).unwrap();
    assert_eq!(workspace.repositories.len(), 1);
    let scheduler = Scheduler::new(registry.database.clone()).unwrap();
    let operation = scheduler
        .enqueue(OperationSpec {
            source_display: format!(
                "https://user:{SYNTHETIC_SECRET}@example.invalid/repo?token={SYNTHETIC_SECRET}"
            ),
            destination_display: "Selected fixture destination".into(),
            reservation: Some(vec![1]),
        })
        .unwrap();
    let mut events = scheduler.subscribe();
    let before = scheduler.snapshot().unwrap();
    faults::refuse_journal_updates(&fixture, &data.join("workspace.sqlite3")).unwrap();
    assert_eq!(
        scheduler
            .transition(
                &operation.id,
                OperationState::Running,
                Phase::Transfer,
                None
            )
            .unwrap_err()
            .code,
        ErrorCode::Database
    );
    assert_eq!(before, scheduler.snapshot().unwrap());
    assert!(matches!(
        events.try_recv(),
        Err(tokio::sync::broadcast::error::TryRecvError::Empty)
    ));
    drop(scheduler);
    drop(registry);
    assert!(!fs::read(data.join("workspace.sqlite3"))
        .unwrap()
        .windows(SYNTHETIC_SECRET.len())
        .any(|window| window == SYNTHETIC_SECRET.as_bytes()));
    let readonly_dir = fixture.child("readonly-db").unwrap();
    fs::create_dir(&readonly_dir).unwrap();
    let readonly_db = readonly_dir.join("workspace.sqlite3");
    let connection = rusqlite::Connection::open(&readonly_db).unwrap();
    connection
        .execute("CREATE TABLE sentinel(value TEXT)", [])
        .unwrap();
    drop(connection);
    let original = fs::read(&readonly_db).unwrap();
    {
        let _readonly = faults::ReadOnlyFile::install(&fixture, &readonly_db).unwrap();
        assert_eq!(
            Database::open(&readonly_dir).err().unwrap().code,
            ErrorCode::Database
        );
        assert_eq!(original, fs::read(&readonly_db).unwrap());
    }
    assert!(source.unchanged().unwrap());
    assert!(destination.unchanged().unwrap());
    fixture.cleanup().unwrap();
}

#[cfg(windows)]
#[tokio::test]
async fn owned_fake_process_injects_real_errors_timeout_and_output_limits() {
    let mut fixture = OwnedFixture::new().unwrap();
    let source = fixture.sentinel("source.sentinel", b"source").unwrap();
    let target = fixture.sentinel("target.sentinel", b"target").unwrap();
    for (fault, expected) in [
        (
            faults::ProcessFault::Failure,
            ErrorCode::AuthenticationFailed,
        ),
        (faults::ProcessFault::Timeout, ErrorCode::Timeout),
        (faults::ProcessFault::OutputLimit, ErrorCode::OutputLimit),
    ] {
        let (runner, args) = faults::process_fault(&fixture, fault).unwrap();
        let failure = runner
            .run(
                Some(fixture.root()),
                &args,
                &Cancellation::default(),
                Phase::Transfer,
            )
            .await
            .unwrap_err();
        assert_eq!(failure.code, expected);
        let serialized = serde_json::to_string(&failure).unwrap();
        assert!(!serialized.contains(SYNTHETIC_SECRET));
    }
    assert!(source.unchanged().unwrap());
    assert!(target.unchanged().unwrap());
    fixture.cleanup().unwrap();
}

#[cfg(windows)]
#[test]
fn locked_file_cleanup_failure_keeps_outside_data_and_can_be_retried() {
    use std::os::windows::fs::OpenOptionsExt;
    let mut fixture = OwnedFixture::new().unwrap();
    let mut outside = OwnedFixture::new().unwrap();
    let sentinel = outside
        .sentinel("outside.sentinel", b"outside remains intact")
        .unwrap();
    let locked = fixture
        .sentinel("locked.sentinel", b"locked fixture data")
        .unwrap();
    let marker = fs::read(fixture.marker_path()).unwrap();
    let file = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(locked.path())
        .unwrap();
    assert!(fixture.cleanup().is_err());
    assert!(fixture.root().exists());
    assert!(sentinel.unchanged().unwrap());
    drop(file);
    // A failed owned-root removal may remove earlier children, including the
    // marker. Restore only the saved marker of this still-owned fixture.
    fs::write(fixture.marker_path(), marker).unwrap();
    fixture.cleanup().unwrap();
    assert!(sentinel.unchanged().unwrap());
    outside.cleanup().unwrap();
}

#[cfg(windows)]
#[test]
fn reparse_child_cannot_redirect_cleanup_outside_fixture() {
    let mut fixture = OwnedFixture::new().unwrap();
    let mut outside = OwnedFixture::new().unwrap();
    let sentinel = outside.sentinel("outside.sentinel", b"preserved").unwrap();
    let alias = faults::junction(&fixture, "alias", &outside).unwrap();
    assert!(fixture.cleanup().is_err());
    assert!(sentinel.unchanged().unwrap());
    fs::remove_dir(&alias).unwrap();
    fixture.cleanup().unwrap();
    assert!(sentinel.unchanged().unwrap());
    outside.cleanup().unwrap();
}

#[tokio::test]
async fn real_git_disposable_fixture_runner_is_not_a_network_auth_claim() {
    let mut fixture = OwnedFixture::new().unwrap();
    let git = GitFixtures::create(&fixture).await.unwrap();
    let output = git
        .runner
        .run(
            Some(&git.working),
            &argv(&["status", "--porcelain=v2", "-z"]),
            &Cancellation::default(),
            Phase::Inspection,
        )
        .await
        .unwrap();
    assert!(output.is_empty());
    fixture.cleanup().unwrap();
}
