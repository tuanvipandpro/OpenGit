pub mod faults;
pub mod history;
pub mod repos;

use opengit_lib::repositories::discovery::identity;
use std::{
    fs, io,
    path::{Component, Path, PathBuf},
};

pub const SYNTHETIC_SECRET: &str = "OPENGIT_SYNTHETIC_CANARY_08_NOT_A_CREDENTIAL";
const MARKER: &str = ".opengit-fixture-owner";

#[derive(Debug)]
pub enum FixtureError {
    Io(io::Error),
    Refused(&'static str),
}
impl From<io::Error> for FixtureError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}
impl std::fmt::Display for FixtureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "Fixture I/O failed ({:?})", error.kind()),
            Self::Refused(reason) => write!(f, "Fixture guard refused: {reason}"),
        }
    }
}
impl std::error::Error for FixtureError {}

/// Every recursive removal uses this in-memory capability, a random disk marker
/// and the original OS directory identity. A failed guard leaves the fixture.
pub struct OwnedFixture {
    root: PathBuf,
    identity: Vec<u8>,
    nonce: String,
    cleaned: bool,
}
impl OwnedFixture {
    pub fn new() -> Result<Self, FixtureError> {
        let root = tempfile::Builder::new()
            .prefix("opengit-fixture-")
            .tempdir()?
            .keep();
        let root = fs::canonicalize(root)?;
        let nonce = uuid::Uuid::new_v4().to_string();
        fs::write(root.join(MARKER), nonce.as_bytes())?;
        let identity =
            identity(&root).map_err(|_| FixtureError::Refused("directory identity unavailable"))?;
        Ok(Self {
            root,
            identity,
            nonce,
            cleaned: false,
        })
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn child(&self, component: &str) -> Result<PathBuf, FixtureError> {
        let mut components = Path::new(component).components();
        if !matches!(components.next(), Some(Component::Normal(_)))
            || components.next().is_some()
            || component.contains(['/', '\\', ':'])
            || component.chars().any(char::is_control)
        {
            return Err(FixtureError::Refused("one relative component required"));
        }
        Ok(self.root.join(component))
    }
    pub fn sentinel(&self, name: &str, bytes: &[u8]) -> Result<Sentinel, FixtureError> {
        let path = self.child(name)?;
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        file.write_all(bytes)?;
        Ok(Sentinel {
            path,
            bytes: bytes.to_vec(),
        })
    }
    pub fn marker_path(&self) -> PathBuf {
        self.root.join(MARKER)
    }
    pub fn require_owned_file(&self, path: &Path) -> Result<(), FixtureError> {
        self.guard(&self.root)?;
        let canonical = fs::canonicalize(path)?;
        if !canonical.starts_with(&self.root) || canonical == self.root || !canonical.is_file() {
            return Err(FixtureError::Refused("file is outside fixture"));
        }
        reject_reparse(path)?;
        Ok(())
    }
    pub fn guard(&self, target: &Path) -> Result<(), FixtureError> {
        // A caller cannot turn a display path, copied marker or a parent into
        // cleanup authority. Reject the actual source checkout explicitly.
        let developer = fs::canonicalize(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .ok_or(FixtureError::Refused("source root unavailable"))?,
        )?;
        reject_reparse(target)?;
        let canonical = fs::canonicalize(target)?;
        if canonical == developer
            || canonical.starts_with(&developer)
            || canonical != self.root
            || target != self.root
            || canonical.parent().is_none()
        {
            return Err(FixtureError::Refused(
                "target is not the owned fixture root",
            ));
        }
        if identity(target).map_err(|_| FixtureError::Refused("directory identity unavailable"))?
            != self.identity
        {
            return Err(FixtureError::Refused("directory identity changed"));
        }
        if fs::read(target.join(MARKER))? != self.nonce.as_bytes() {
            return Err(FixtureError::Refused("ownership marker changed"));
        }
        let mut remaining = 100_000usize;
        verify_tree(target, 0, &mut remaining)?;
        Ok(())
    }
    pub fn cleanup_target(&mut self, target: &Path) -> Result<(), FixtureError> {
        self.guard(target)?;
        fs::remove_dir_all(target)?;
        self.cleaned = true;
        Ok(())
    }
    pub fn cleanup(&mut self) -> Result<(), FixtureError> {
        if self.cleaned {
            return Ok(());
        }
        let root = self.root.clone();
        self.cleanup_target(&root)
    }
}
impl Drop for OwnedFixture {
    fn drop(&mut self) {
        if !self.cleaned {
            let _ = self.cleanup();
        }
    }
}

fn reject_reparse(path: &Path) -> Result<fs::Metadata, FixtureError> {
    let metadata = fs::symlink_metadata(path)?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(FixtureError::Refused(
                "reparse point is not a cleanup target",
            ));
        }
    }
    if metadata.file_type().is_symlink() {
        return Err(FixtureError::Refused("symlink is not a cleanup target"));
    }
    Ok(metadata)
}
fn verify_tree(path: &Path, depth: usize, remaining: &mut usize) -> Result<(), FixtureError> {
    if depth > 64 || *remaining == 0 {
        return Err(FixtureError::Refused(
            "fixture exceeds cleanup inspection limit",
        ));
    }
    *remaining -= 1;
    let metadata = reject_reparse(path)?;
    if metadata.is_dir() {
        for entry in fs::read_dir(path)? {
            verify_tree(&entry?.path(), depth + 1, remaining)?;
        }
    }
    Ok(())
}
pub struct Sentinel {
    path: PathBuf,
    bytes: Vec<u8>,
}
impl Sentinel {
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn unchanged(&self) -> Result<bool, FixtureError> {
        Ok(fs::read(&self.path)? == self.bytes)
    }
}
