//! Repository commands require Git 2.31.0: discovery uses `rev-parse
//! --path-format=absolute`, introduced in that release. One policy serves both
//! diagnostics and the repository-command gate; a numeric banner alone is not
//! evidence that the required commands are supported.
use super::{
    error::error,
    runner::{argv, Cancellation, GitRunner},
};
use crate::domain::contracts::{AppError, ErrorCode, Phase};

pub const MINIMUM: GitVersion = GitVersion(2, 31, 0);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct GitVersion(pub u32, pub u32, pub u32);
impl std::fmt::Display for GitVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}
impl GitVersion {
    pub fn supported(self) -> bool {
        self >= MINIMUM
    }
}
pub fn parse(output: &[u8]) -> Option<GitVersion> {
    let text = std::str::from_utf8(output)
        .ok()?
        .trim_end_matches(['\r', '\n']);
    if text.chars().any(char::is_control) {
        return None;
    }
    let token = text
        .strip_prefix("git version ")?
        .split_ascii_whitespace()
        .next()?;
    let parts: Vec<_> = token.split('.').collect();
    if parts.len() != 3
        && !(parts.len() == 5
            && parts[3] == "windows"
            && !parts[4].is_empty()
            && parts[4].bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    let number = |value: &str| {
        if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
            None
        } else {
            value.parse::<u32>().ok()
        }
    };
    Some(GitVersion(
        number(parts[0])?,
        number(parts[1])?,
        number(parts[2])?,
    ))
}
pub async fn probe(runner: &GitRunner) -> Result<GitVersion, AppError> {
    let mut probe = runner.clone();
    probe.timeout = std::time::Duration::from_secs(3);
    probe.output_limit = 4096;
    let bytes = probe
        .run(
            None,
            &argv(&["--version"]),
            &Cancellation::default(),
            Phase::Discovery,
        )
        .await?;
    parse(&bytes).ok_or_else(|| error(ErrorCode::UnsupportedGit, Phase::Discovery))
}
pub async fn ensure_supported(runner: &GitRunner) -> Result<GitVersion, AppError> {
    let version = probe(runner).await?;
    if version.supported() {
        Ok(version)
    } else {
        Err(error(ErrorCode::UnsupportedGit, Phase::Discovery))
    }
}
