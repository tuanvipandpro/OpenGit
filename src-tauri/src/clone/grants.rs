use super::path::StableDirectory;
use crate::{domain::contracts::*, git::error::error, repositories::discovery::identity};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc, Mutex},
    time::{Duration, Instant},
};
pub struct Source {
    pub path: PathBuf,
    pub identity: Vec<u8>,
    pub common: PathBuf,
    pub common_identity: Vec<u8>,
    pub directory: Arc<StableDirectory>,
    pub common_directory: Arc<StableDirectory>,
}
struct Grant<T> {
    window: String,
    expires: Instant,
    value: Arc<T>,
}
type Grants<T> = Arc<Mutex<HashMap<String, Grant<T>>>>;
pub struct Control {
    pub window: String,
    pub parent_identity: Vec<u8>,
    pub leaf: String,
    pub gate: Mutex<()>,
    pub published: AtomicBool,
}
pub struct CloneState {
    sources: Grants<Source>,
    destinations: Grants<StableDirectory>,
    pub controls: Mutex<HashMap<String, Arc<Control>>>,
    pub recovery_error: Mutex<Option<AppError>>,
}
impl Default for CloneState {
    fn default() -> Self {
        let sources: Grants<Source> = Arc::new(Mutex::new(HashMap::new()));
        let destinations: Grants<StableDirectory> = Arc::new(Mutex::new(HashMap::new()));
        let weak_sources = Arc::downgrade(&sources);
        let weak_destinations = Arc::downgrade(&destinations);
        std::thread::spawn(move || loop {
            {
                let (Some(sources), Some(destinations)) =
                    (weak_sources.upgrade(), weak_destinations.upgrade())
                else {
                    break;
                };
                let now = Instant::now();
                sources
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .retain(|_, grant| grant.expires > now);
                destinations
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .retain(|_, grant| grant.expires > now);
            }
            // No strong map/Backend/Database reference survives the wait.
            std::thread::sleep(Duration::from_millis(250));
        });
        Self {
            sources,
            destinations,
            controls: Mutex::new(HashMap::new()),
            recovery_error: Mutex::new(None),
        }
    }
}
impl CloneState {
    pub fn source(
        &self,
        window: &str,
        path: PathBuf,
        common: PathBuf,
        common_identity: Vec<u8>,
    ) -> Result<LocalCloneSource, AppError> {
        let directory = Arc::new(StableDirectory::open(&path)?);
        let common_directory = Arc::new(StableDirectory::open(&common)?);
        let identity = directory.identity.clone();
        let handle = capability();
        let display = path.to_string_lossy().into();
        let mut map = self.sources.lock().map_err(|_| invalid())?;
        map.retain(|_, grant| grant.expires > Instant::now());
        map.insert(
            handle.clone(),
            Grant {
                window: window.into(),
                expires: Instant::now() + Duration::from_secs(900),
                value: Arc::new(Source {
                    path,
                    identity,
                    common,
                    common_identity,
                    directory,
                    common_directory,
                }),
            },
        );
        Ok(LocalCloneSource {
            source_handle: handle,
            display,
        })
    }
    pub fn destination(&self, window: &str, path: &Path) -> Result<CloneDestination, AppError> {
        let parent = Arc::new(StableDirectory::open(path)?);
        let handle = capability();
        let display = parent.path.to_string_lossy().into();
        let mut map = self.destinations.lock().map_err(|_| invalid())?;
        map.retain(|_, grant| grant.expires > Instant::now());
        map.insert(
            handle.clone(),
            Grant {
                window: window.into(),
                expires: Instant::now() + Duration::from_secs(900),
                value: parent,
            },
        );
        Ok(CloneDestination {
            destination_handle: handle,
            parent_display: display,
        })
    }
    pub fn get_source(&self, window: &str, handle: &str) -> Result<Arc<Source>, AppError> {
        let map = self.sources.lock().map_err(|_| invalid())?;
        let grant = map
            .get(handle)
            .filter(|grant| grant.window == window && grant.expires > Instant::now())
            .ok_or_else(invalid)?;
        grant.value.directory.recheck()?;
        grant.value.common_directory.recheck()?;
        if identity(&grant.value.path)? != grant.value.identity
            || identity(&grant.value.common)? != grant.value.common_identity
        {
            return Err(error(ErrorCode::ExternalChange, Phase::Validation));
        }
        Ok(grant.value.clone())
    }
    pub fn get_destination(
        &self,
        window: &str,
        handle: &str,
    ) -> Result<Arc<StableDirectory>, AppError> {
        let map = self.destinations.lock().map_err(|_| invalid())?;
        let grant = map
            .get(handle)
            .filter(|grant| grant.window == window && grant.expires > Instant::now())
            .ok_or_else(invalid)?;
        grant.value.recheck()?;
        Ok(grant.value.clone())
    }
    pub fn close_window(&self, window: &str) {
        if let Ok(mut map) = self.sources.lock() {
            map.retain(|_, grant| grant.window != window);
        }
        if let Ok(mut map) = self.destinations.lock() {
            map.retain(|_, grant| grant.window != window);
        }
    }
    pub fn check_ready(&self) -> Result<(), AppError> {
        if let Some(error) = self.recovery_error.lock().map_err(|_| invalid())?.as_ref() {
            Err(error.clone())
        } else {
            Ok(())
        }
    }
    pub fn fail_closed(&self) {
        if let Ok(mut value) = self.recovery_error.lock() {
            *value = Some(error(ErrorCode::Database, Phase::Recovery));
        }
    }
}
pub fn invalid() -> AppError {
    error(ErrorCode::InvalidHandle, Phase::Validation)
}
fn capability() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}
pub fn same_leaf(a: &str, b: &str) -> bool {
    let a: Vec<u16> = a.encode_utf16().collect();
    let b: Vec<u16> = b.encode_utf16().collect();
    unsafe {
        windows_sys::Win32::Globalization::CompareStringOrdinal(
            a.as_ptr(),
            a.len() as i32,
            b.as_ptr(),
            b.len() as i32,
            1,
        ) == 2
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn idle_capability_expiry_releases_handles_but_active_operation_guards_remain() {
        for active in [false, true] {
            let fixture = tempfile::tempdir().unwrap();
            let path = fixture.path().join("selected parent");
            std::fs::create_dir(&path).unwrap();
            let moved = fixture.path().join("moved");
            let state = CloneState::default();
            let grant = state.destination("main", &path).unwrap();
            let operation_guard = if active {
                Some(
                    state
                        .get_destination("main", &grant.destination_handle)
                        .unwrap(),
                )
            } else {
                None
            };
            state
                .destinations
                .lock()
                .unwrap()
                .get_mut(&grant.destination_handle)
                .unwrap()
                .expires = Instant::now() + Duration::from_millis(40);
            assert!(std::fs::rename(&path, &moved).is_err());
            let deadline = Instant::now() + Duration::from_secs(3);
            while !state.destinations.lock().unwrap().is_empty() {
                assert!(Instant::now() < deadline, "idle grant cleanup did not run");
                std::thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(
                state
                    .get_destination("main", &grant.destination_handle)
                    .err()
                    .unwrap()
                    .code,
                ErrorCode::InvalidHandle
            );
            if active {
                assert!(std::fs::rename(&path, &moved).is_err());
                drop(operation_guard);
            }
            std::fs::rename(&path, &moved).unwrap();
        }
        let fixture = tempfile::tempdir().unwrap();
        let source = fixture.path().join("source");
        let common = source.join(".git");
        std::fs::create_dir_all(&common).unwrap();
        let state = CloneState::default();
        let grant = state
            .source(
                "main",
                source.clone(),
                common.clone(),
                identity(&common).unwrap(),
            )
            .unwrap();
        state
            .sources
            .lock()
            .unwrap()
            .get_mut(&grant.source_handle)
            .unwrap()
            .expires = Instant::now() + Duration::from_millis(40);
        let moved = fixture.path().join("moved source");
        assert!(std::fs::rename(&source, &moved).is_err());
        let deadline = Instant::now() + Duration::from_secs(3);
        while !state.sources.lock().unwrap().is_empty() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            state
                .get_source("main", &grant.source_handle)
                .err()
                .unwrap()
                .code,
            ErrorCode::InvalidHandle
        );
        std::fs::rename(&source, &moved).unwrap();
    }
}
