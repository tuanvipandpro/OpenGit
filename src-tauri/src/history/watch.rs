//! Path-free change hints. Workers retain only repository metadata and pure
//! callback resources, never Backend, database, scheduler or credential state.
#[cfg(test)]
mod tests;
use crate::{
    domain::contracts::*,
    git::error::error,
    repositories::{discovery, registry::RepositoryLocation},
};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    collections::{HashMap, VecDeque},
    io::Read,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
        Arc, Condvar, Mutex, Weak,
    },
    time::{Duration, Instant},
};
use tokio::sync::{broadcast, oneshot};

const CHANGED: u8 = 1;
const OVERFLOW: u8 = 2;
const UNAVAILABLE: u8 = 4;
const WORKERS: usize = 64;
const EVENTS: usize = 256;
const TOMBSTONES: usize = 256;
const DEBOUNCE: Duration = Duration::from_millis(150);
const BURST: Duration = Duration::from_secs(1);
const TICK: Duration = Duration::from_millis(50);
const RETRY: Duration = Duration::from_secs(1);
const STARTUP: Duration = Duration::from_secs(2);
const TOKEN_TTL: Duration = Duration::from_secs(900);

#[cfg(test)]
#[derive(Default)]
pub(crate) struct TestPause {
    pub entered: tokio::sync::Notify,
    pub release: tokio::sync::Notify,
}
#[cfg(test)]
#[derive(Default)]
pub(crate) struct TestHooks {
    pub before_begin: Mutex<Option<Arc<TestPause>>>,
    pub activation_wait: Mutex<Option<Arc<TestPause>>>,
}
#[cfg(test)]
impl TestHooks {
    pub async fn pause(&self, before_begin: bool) {
        let pause = {
            let slot = if before_begin {
                &self.before_begin
            } else {
                &self.activation_wait
            };
            slot.lock().unwrap().take()
        };
        if let Some(pause) = pause {
            pause.entered.notify_one();
            pause.release.notified().await;
        }
    }
}

#[derive(Clone)]
pub struct RoutedChange {
    pub window: String,
    pub event: RepositoryChangedEvent,
}
#[derive(Default)]
struct Signal {
    flags: AtomicU8,
    stop: AtomicBool,
    late: AtomicBool,
    mutex: Mutex<()>,
    wake: Condvar,
}
impl Signal {
    fn mark(&self, flags: u8) {
        self.flags.fetch_or(flags, Ordering::SeqCst);
        self.wake.notify_one();
    }
    fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        self.wake.notify_one();
    }
    fn wait(&self) {
        if let Ok(lock) = self.mutex.lock() {
            let _ = self.wake.wait_timeout(lock, TICK);
        }
    }
}
struct Resource {
    count: Arc<AtomicUsize>,
}
impl Drop for Resource {
    fn drop(&mut self) {
        self.count.fetch_sub(1, Ordering::SeqCst);
    }
}
struct Active {
    token: String,
    scope: HistoryScope,
    sequence: u64,
    status: WatchStatus,
    signal: Arc<Signal>,
}
impl Drop for Active {
    fn drop(&mut self) {
        self.signal.stop();
    }
}
#[derive(Default)]
struct Window {
    active: Option<Active>,
    stopped: VecDeque<(String, Instant)>,
}
#[derive(Default)]
struct State {
    windows: HashMap<String, Window>,
}
struct Inner {
    state: Mutex<State>,
    events: broadcast::Sender<RoutedChange>,
    count: Arc<AtomicUsize>,
    limit: usize,
}
pub struct Watches {
    inner: Arc<Inner>,
}
impl Drop for Watches {
    fn drop(&mut self) {
        if let Ok(mut state) = self.inner.state.lock() {
            state.windows.clear();
        }
    }
}
impl Default for Watches {
    fn default() -> Self {
        let (events, _) = broadcast::channel(EVENTS);
        Self {
            inner: Arc::new(Inner {
                state: Mutex::new(State::default()),
                events,
                count: Arc::new(AtomicUsize::new(0)),
                limit: WORKERS,
            }),
        }
    }
}
fn invalid() -> AppError {
    error(ErrorCode::InvalidHandle, Phase::Validation)
}
fn internal() -> AppError {
    error(ErrorCode::Internal, Phase::Inspection)
}
fn remember(window: &mut Window, token: String) {
    let now = Instant::now();
    window
        .stopped
        .retain(|(_, time)| now.duration_since(*time) < TOKEN_TTL);
    if window.stopped.len() >= TOMBSTONES {
        window.stopped.pop_front();
    }
    window.stopped.push_back((token, now));
}
impl Watches {
    pub fn subscribe(&self) -> broadcast::Receiver<RoutedChange> {
        self.inner.events.subscribe()
    }
    pub fn delivery_gap(&self) {
        if let Ok(state) = self.inner.state.lock() {
            for window in state.windows.values() {
                if let Some(active) = &window.active {
                    active.signal.mark(OVERFLOW);
                }
            }
        }
    }
    pub fn forward_current(&self, change: &RoutedChange, send: impl FnOnce() -> bool) {
        if let Ok(state) = self.inner.state.lock() {
            if let Some(active) = state
                .windows
                .get(&change.window)
                .and_then(|w| w.active.as_ref())
                .filter(|a| {
                    a.token == change.event.subscription_id && a.scope == change.event.scope
                })
            {
                if !send() {
                    active.signal.mark(OVERFLOW);
                }
            }
        }
    }
    pub fn revoke_window(&self, window: &str, close: bool) {
        if let Ok(mut state) = self.inner.state.lock() {
            if close {
                state.windows.remove(window);
            } else if let Some(window) = state.windows.get_mut(window) {
                if let Some(active) = window.active.take() {
                    remember(window, active.token.clone());
                }
            }
        }
    }
    pub fn revoke_repo(&self, repo: &str) {
        if let Ok(mut state) = self.inner.state.lock() {
            for window in state.windows.values_mut() {
                if window
                    .active
                    .as_ref()
                    .is_some_and(|a| a.scope.repo_id == repo)
                {
                    if let Some(active) = window.active.take() {
                        remember(window, active.token.clone());
                    }
                }
            }
        }
    }
    pub fn unwatch(&self, window: &str, token: &str) -> Result<(), AppError> {
        let mut state = self.inner.state.lock().map_err(|_| internal())?;
        let window = state.windows.get_mut(window).ok_or_else(invalid)?;
        window
            .stopped
            .retain(|(_, time)| time.elapsed() < TOKEN_TTL);
        if window.active.as_ref().is_some_and(|a| a.token == token) {
            let active = window.active.take().ok_or_else(invalid)?;
            remember(window, active.token.clone());
            Ok(())
        } else if window.stopped.iter().any(|(id, _)| id == token) {
            Ok(())
        } else {
            Err(invalid())
        }
    }
    pub(crate) fn start(
        &self,
        window: &str,
        scope: HistoryScope,
        location: RepositoryLocation,
    ) -> Result<(String, oneshot::Receiver<bool>), AppError> {
        self.inner.count.try_update(Ordering::SeqCst,Ordering::SeqCst,|n|(n<self.inner.limit).then_some(n+1)).map_err(|_|AppError {code:ErrorCode::OutputLimit,phase:Phase::Inspection,retry:AppErrorRetry::None,message:"Repository watching reached its resource limit. Periodic refresh remains available.".into()})?;
        let resource = Arc::new(Resource {
            count: self.inner.count.clone(),
        });
        let token = super::sessions::token();
        let signal = Arc::new(Signal::default());
        let (ready, receiver) = oneshot::channel();
        {
            let mut state = self.inner.state.lock().map_err(|_| internal())?;
            if !state.windows.contains_key(window) && state.windows.len() >= WORKERS {
                return Err(error(ErrorCode::OutputLimit, Phase::Inspection));
            }
            let entry = state.windows.entry(window.into()).or_default();
            if let Some(old) = entry.active.take() {
                remember(entry, old.token.clone());
            }
            entry.active = Some(Active {
                token: token.clone(),
                scope,
                sequence: 0,
                status: WatchStatus::Fallback,
                signal: signal.clone(),
            });
        }
        let weak = Arc::downgrade(&self.inner);
        let name = window.to_owned();
        let id = token.clone();
        if std::thread::Builder::new()
            .name("opengit-watch".into())
            .spawn(move || worker(weak, name, id, location, signal, resource, ready))
            .is_err()
        {
            current(
                &self.inner,
                window,
                &token,
                Some(WatchStatus::Fallback),
                Some(RepositoryChangeReason::Unavailable),
            );
        }
        Ok((token, receiver))
    }
    pub(crate) async fn finish(
        &self,
        window: &str,
        token: String,
        ready: oneshot::Receiver<bool>,
    ) -> Result<RepositoryWatch, AppError> {
        let late = tokio::time::timeout(STARTUP, ready).await.is_err();
        let state = self.inner.state.lock().map_err(|_| internal())?;
        let active = state
            .windows
            .get(window)
            .and_then(|w| w.active.as_ref())
            .filter(|a| a.token == token)
            .ok_or_else(invalid)?;
        if late {
            active.signal.late.store(true, Ordering::SeqCst);
        }
        Ok(RepositoryWatch {
            scope: active.scope.clone(),
            subscription_id: token,
            sequence: "0".into(),
            status: active.status.clone(),
        })
    }
}
struct Roots {
    location: RepositoryLocation,
    git_identity: Vec<u8>,
    paths: Vec<PathBuf>,
}
fn metadata(path: &Path) -> Option<String> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(65_537)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > 65_536 {
        return None;
    }
    String::from_utf8(bytes)
        .ok()
        .map(|s| s.trim_end_matches(['\r', '\n']).to_owned())
}
impl Roots {
    fn new(location: RepositoryLocation) -> Option<Self> {
        let git_identity = discovery::identity(&location.git_dir).ok()?;
        let mut candidates = vec![
            location.root.clone(),
            location.git_dir.clone(),
            location.common.clone(),
        ];
        candidates.sort_by_key(|path| path.components().count());
        let mut paths: Vec<PathBuf> = Vec::new();
        for path in candidates {
            // A recursive ancestor already observes this directory. On Windows
            // a separate open child watch prevents renaming its ancestor even
            // with FILE_SHARE_DELETE, so retain only the minimal cover.
            if !paths.iter().any(|parent| path.starts_with(parent)) {
                paths.push(path);
            }
        }
        let result = Self {
            location,
            git_identity,
            paths,
        };
        result.valid().then_some(result)
    }
    fn valid(&self) -> bool {
        let l = &self.location;
        if discovery::identity(&l.common).as_ref() != Ok(&l.common_identity)
            || discovery::identity(&l.git_dir).as_ref() != Ok(&self.git_identity)
            || l.root_identity
                .as_ref()
                .is_some_and(|id| discovery::identity(&l.root).as_ref() != Ok(id))
        {
            return false;
        }
        if !l.bare {
            let dot = l.root.join(".git");
            let git = if dot.is_dir() {
                Some(dot)
            } else {
                metadata(&dot).and_then(|s| s.strip_prefix("gitdir: ").map(|s| l.root.join(s)))
            };
            if git.and_then(|p| std::fs::canonicalize(p).ok()).as_ref() != Some(&l.git_dir) {
                return false;
            }
        }
        let common_file = l.git_dir.join("commondir");
        let common = if common_file.exists() {
            metadata(&common_file).and_then(|s| std::fs::canonicalize(l.git_dir.join(s)).ok())
        } else {
            Some(l.git_dir.clone())
        };
        common.as_ref() == Some(&l.common)
    }
}
fn current(
    inner: &Inner,
    window: &str,
    token: &str,
    status: Option<WatchStatus>,
    reason: Option<RepositoryChangeReason>,
) -> bool {
    let Ok(mut state) = inner.state.lock() else {
        return false;
    };
    let Some(active) = state
        .windows
        .get_mut(window)
        .and_then(|w| w.active.as_mut())
        .filter(|a| a.token == token)
    else {
        return false;
    };
    if let Some(status) = status {
        active.status = status;
    }
    if let Some(mut reason) = reason {
        let Some(next) = active.sequence.checked_add(1) else {
            active.signal.stop();
            return false;
        };
        active.sequence = next;
        if next == u64::MAX {
            reason = RepositoryChangeReason::Unavailable;
            active.status = WatchStatus::Fallback;
            active.signal.stop();
        }
        let _ = inner.events.send(RoutedChange {
            window: window.into(),
            event: RepositoryChangedEvent {
                scope: active.scope.clone(),
                subscription_id: token.into(),
                sequence: next.to_string(),
                reason,
            },
        });
    }
    true
}
fn native(
    roots: &Roots,
    signal: &Arc<Signal>,
    resource: &Arc<Resource>,
) -> Result<(RecommendedWatcher, Weak<()>), (notify::Error, Weak<()>)> {
    let marker = Arc::new(());
    let weak = Arc::downgrade(&marker);
    let signal = signal.clone();
    let resource = resource.clone();
    let mut watcher = RecommendedWatcher::new(
        move |result: notify::Result<notify::Event>| {
            let _keep = (&marker, &resource);
            match result {
                Ok(event) if event.need_rescan() => signal.mark(OVERFLOW),
                Ok(event) if !matches!(event.kind, notify::EventKind::Access(_)) => {
                    signal.mark(CHANGED)
                }
                Ok(_) => (),
                Err(_) => signal.mark(UNAVAILABLE),
            }
        },
        notify::Config::default().with_follow_symlinks(false),
    )
    .map_err(|e| (e, weak.clone()))?;
    for path in &roots.paths {
        if let Err(error) = watcher.watch(path, RecursiveMode::Recursive) {
            drop(watcher);
            return Err((error, weak));
        }
    }
    Ok((watcher, weak))
}
fn worker(
    inner: Weak<Inner>,
    window: String,
    token: String,
    location: RepositoryLocation,
    signal: Arc<Signal>,
    resource: Arc<Resource>,
    ready: oneshot::Sender<bool>,
) {
    let mut roots = Roots::new(location.clone());
    let mut watcher = None;
    let mut retiring: Option<Weak<()>> = None;
    let mut ready = Some(ready);
    let mut first = true;
    let mut retry = Instant::now();
    let mut pending = 0u8;
    let mut first_event = None;
    let mut last_event = Instant::now();
    let mut unavailable = false;
    let mut force_rearm = false;
    while !signal.stop.load(Ordering::SeqCst) {
        let Some(manager) = inner.upgrade() else {
            break;
        };
        if !current(&manager, &window, &token, None, None) {
            break;
        }
        let now = Instant::now();
        let flags = signal.flags.swap(0, Ordering::SeqCst);
        if flags != 0 {
            pending |= flags;
            first_event.get_or_insert(now);
            last_event = now;
        }
        force_rearm |= flags & (OVERFLOW | UNAVAILABLE) != 0;
        if now >= retry || force_rearm {
            retry = now + RETRY;
            if roots.is_none() {
                roots = Roots::new(location.clone());
            }
            let valid = roots.as_ref().is_some_and(Roots::valid);
            if !valid || force_rearm {
                // Publish known loss before any native re-registration: its
                // upstream ACK may block. The independent Git fallback must
                // not wait for rearm to learn that a full refresh is needed.
                if !unavailable || force_rearm {
                    let reason = if flags & OVERFLOW != 0 {
                        RepositoryChangeReason::Overflow
                    } else {
                        RepositoryChangeReason::Unavailable
                    };
                    if !current(
                        &manager,
                        &window,
                        &token,
                        Some(WatchStatus::Fallback),
                        Some(reason),
                    ) {
                        break;
                    }
                    pending = 0;
                    first_event = None;
                    unavailable = true;
                }
                if let Some((old, marker)) = watcher.take() {
                    drop(old);
                    retiring = Some(marker);
                }
                current(&manager, &window, &token, Some(WatchStatus::Fallback), None);
            }
            force_rearm = false;
            if watcher.is_none()
                && retiring
                    .as_ref()
                    .is_none_or(|weak| weak.upgrade().is_none())
            {
                retiring = None;
                // Recheck the same captured identities; never follow a newly
                // substituted directory under the registered path.
                if valid {
                    if let Some(root) = &roots {
                        match native(root, &signal, &resource) {
                            Ok(value) => {
                                watcher = Some(value);
                                current(
                                    &manager,
                                    &window,
                                    &token,
                                    Some(WatchStatus::Watching),
                                    None,
                                );
                                if !first || signal.late.load(Ordering::SeqCst) {
                                    pending |= CHANGED;
                                    first_event.get_or_insert(now);
                                    last_event = now;
                                }
                                unavailable = false;
                            }
                            Err((_, marker)) => {
                                retiring = Some(marker);
                                if !unavailable {
                                    pending |= UNAVAILABLE;
                                    first_event.get_or_insert(now);
                                    last_event = now;
                                    unavailable = true;
                                }
                            }
                        }
                    }
                }
            }
            if let Some(ready) = ready.take() {
                let _ = ready.send(watcher.is_some());
            }
            first = false;
        }
        if let Some(start) = first_event {
            if now.duration_since(last_event) >= DEBOUNCE || now.duration_since(start) >= BURST {
                let reason = if pending & OVERFLOW != 0 {
                    RepositoryChangeReason::Overflow
                } else if pending & UNAVAILABLE != 0 {
                    RepositoryChangeReason::Unavailable
                } else {
                    RepositoryChangeReason::Changed
                };
                if !current(&manager, &window, &token, None, Some(reason)) {
                    break;
                }
                pending = 0;
                first_event = None;
            }
        }
        drop(manager);
        signal.wait();
    }
    drop(watcher);
    drop(retiring);
    drop(roots.take());
}
