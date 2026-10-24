use super::{FixtureError, OwnedFixture};
use opengit_lib::{
    domain::contracts::Phase,
    git::runner::{argv, Cancellation, GitRunner},
};
use std::{
    fs,
    path::{Path, PathBuf},
};
pub struct GitFixtures {
    pub runner: GitRunner,
    pub working: PathBuf,
    pub bare: PathBuf,
}
impl GitFixtures {
    pub async fn create(fixture: &OwnedFixture) -> Result<Self, FixtureError> {
        let runner = GitRunner::discover().map_err(|_| FixtureError::Refused("Git unavailable"))?;
        let working = fixture.child("working Ω repository")?;
        fs::create_dir(&working)?;
        run(&runner, &working, &["init", "-b", "main"]).await?;
        // Config is command-local; no developer/global identity is modified.
        run(
            &runner,
            &working,
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "--allow-empty",
                "-m",
                "fixture",
            ],
        )
        .await?;
        let bare = fixture.child("bare.git")?;
        fs::create_dir(&bare)?;
        run(&runner, &bare, &["init", "--bare"]).await?;
        Ok(Self {
            runner,
            working,
            bare,
        })
    }
}
pub async fn run(runner: &GitRunner, cwd: &Path, args: &[&str]) -> Result<(), FixtureError> {
    runner
        .run(
            Some(cwd),
            &argv(args),
            &Cancellation::default(),
            Phase::Inspection,
        )
        .await
        .map(|_| ())
        .map_err(|_| FixtureError::Refused("fixture Git command failed"))
}
