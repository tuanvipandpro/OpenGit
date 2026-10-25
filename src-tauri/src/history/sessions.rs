use super::{query::Fingerprint, *};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tokio::sync::Mutex as AsyncMutex;

pub(crate) fn token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}
#[derive(Clone, PartialEq)]
pub(crate) enum Cursor {
    Page {
        start: usize,
        size: usize,
    },
    Search {
        start: usize,
        size: usize,
        query: String,
        graph_fence: usize,
    },
}
pub(crate) struct Session {
    pub cancel: crate::git::runner::Cancellation,
    pub context: HistoryContext,
    pub id: String,
    pub location: RepositoryLocation,
    pub fingerprint: Fingerprint,
    pub refs: Vec<HistoryRef>,
    pub head: Head,
    pub roots: Vec<String>,
    pub order: Vec<String>,
    pub all_order: Vec<String>,
    pub positions: HashMap<String, usize>,
    pub all: std::collections::HashSet<String>,
    pub shallow: std::collections::HashSet<String>,
    pub graph_fence: usize,
    pub handles: HashMap<String, String>, // OID -> handle; raw OIDs never supplied by renderer
    pub handle_oids: HashMap<String, String>,
    pub cursors: HashMap<String, Cursor>,
    pub files: HashMap<String, super::details::FileRecord>,
}
impl Session {
    pub fn handle(&mut self, oid: &str) -> Result<String, AppError> {
        if let Some(handle) = self.handles.get(oid) {
            return Ok(handle.clone());
        }
        if self.handles.len() >= HISTORY_COMMITS_PER_SESSION {
            return Err(limit());
        }
        let handle = token();
        self.handles.insert(oid.into(), handle.clone());
        self.handle_oids.insert(handle.clone(), oid.into());
        Ok(handle)
    }
    pub fn cursor(&mut self, cursor: Cursor) -> Result<String, AppError> {
        if let Some((id, _)) = self.cursors.iter().find(|(_, value)| **value == cursor) {
            return Ok(id.clone());
        }
        if self.cursors.len() >= HISTORY_CURSORS_PER_SESSION {
            return Err(limit());
        }
        let id = token();
        self.cursors.insert(id.clone(), cursor);
        Ok(id)
    }
}
pub(crate) struct Entry {
    pub session: AsyncMutex<Session>,
    pub revoked: AtomicBool,
    created: Instant,
}
impl Entry {
    pub fn valid(&self) -> Result<(), AppError> {
        if self.revoked.load(Ordering::SeqCst)
            || self.created.elapsed() >= Duration::from_secs(HISTORY_SESSION_TTL_SECONDS as u64)
        {
            Err(invalid_handle())
        } else {
            Ok(())
        }
    }
}
struct Stored {
    window: String,
    context: HistoryContext,
    epoch: u64,
    accessed: Instant,
    entry: Arc<Entry>,
}
#[derive(Default)]
struct Window {
    enabled: bool,
    epoch: u64,
    scope: Option<HistoryScope>,
    cancel: crate::git::runner::Cancellation,
}
#[derive(Default)]
struct State {
    windows: HashMap<String, Window>,
    sessions: HashMap<String, Stored>,
}
#[derive(Clone)]
pub(crate) struct Ticket {
    window: String,
    epoch: u64,
    scope: HistoryScope,
    pub cancel: crate::git::runner::Cancellation,
}
#[derive(Default)]
pub struct Sessions {
    state: Mutex<State>,
}
impl Sessions {
    #[cfg(test)]
    pub(crate) fn expire_for_test(&self, id: &str, created: Instant) {
        let mut state = self.state.lock().unwrap();
        let item = state.sessions.get_mut(id).unwrap();
        Arc::get_mut(&mut item.entry).unwrap().created = created;
    }
    pub fn open_window(&self, window: &str) {
        if let Ok(mut state) = self.state.lock() {
            state.windows.entry(window.into()).or_default().enabled = true;
        }
    }
    pub fn revoke_window(&self, window: &str, close: bool) {
        if let Ok(mut state) = self.state.lock() {
            if let Some(value) = state.windows.get_mut(window) {
                value.cancel.cancel();
                value.cancel = Default::default();
                value.enabled = !close;
                value.epoch = value.epoch.saturating_add(1);
                value.scope = None;
            }
            state.sessions.retain(|_, item| {
                if item.window == window {
                    item.entry.revoked.store(true, Ordering::SeqCst);
                    false
                } else {
                    true
                }
            });
        }
    }
    pub fn revoke_repo(&self, repo: &str) {
        if let Ok(mut state) = self.state.lock() {
            let affected: Vec<_> = state
                .windows
                .iter()
                .filter(|(_, w)| w.scope.as_ref().is_some_and(|s| s.repo_id == repo))
                .map(|(id, _)| id.clone())
                .collect();
            for id in affected {
                if let Some(w) = state.windows.get_mut(&id) {
                    w.cancel.cancel();
                    w.cancel = Default::default();
                    w.epoch = w.epoch.saturating_add(1);
                    w.scope = None;
                }
            }
            state.sessions.retain(|_, item| {
                if item.context.scope.repo_id == repo {
                    item.entry.revoked.store(true, Ordering::SeqCst);
                    false
                } else {
                    true
                }
            });
        }
    }
    pub(crate) fn begin(&self, window: &str, scope: &HistoryScope) -> Result<Ticket, AppError> {
        self.begin_checked(window, scope, || Ok(()))
    }
    pub(crate) fn begin_checked(
        &self,
        window: &str,
        scope: &HistoryScope,
        validate: impl FnOnce() -> Result<(), AppError>,
    ) -> Result<Ticket, AppError> {
        let generation = validation::counter(&scope.generation)?;
        let mut state = self.state.lock().map_err(|_| internal())?;
        let active = state
            .windows
            .get_mut(window)
            .filter(|w| w.enabled)
            .ok_or_else(invalid_handle)?;
        if let Some(current) = &active.scope {
            let previous = validation::counter(&current.generation)?;
            if generation < previous
                || (generation == previous
                    && (scope.repo_id != current.repo_id
                        || scope.worktree_id != current.worktree_id))
            {
                return Err(invalid_handle());
            }
        }
        // Watch registration checks must succeed before changing scope or
        // revoking sessions. Keep the existing state -> database lock order.
        validate()?;
        let replacing = active.scope.as_ref() != Some(scope);
        if replacing {
            active.cancel.cancel();
            active.cancel = Default::default();
            active.epoch = active.epoch.checked_add(1).ok_or_else(internal)?;
            active.scope = Some(scope.clone());
        }
        let epoch = active.epoch;
        let cancel = active.cancel.clone();
        if replacing {
            state.sessions.retain(|_, item| {
                if item.window == window {
                    item.entry.revoked.store(true, Ordering::SeqCst);
                    false
                } else {
                    true
                }
            });
        }
        Ok(Ticket {
            window: window.into(),
            epoch,
            scope: scope.clone(),
            cancel,
        })
    }
    pub(crate) fn publish(&self, ticket: &Ticket, session: Session) -> Result<(), AppError> {
        let mut state = self.state.lock().map_err(|_| internal())?;
        let active = state
            .windows
            .get(&ticket.window)
            .filter(|w| {
                w.enabled && w.epoch == ticket.epoch && w.scope.as_ref() == Some(&ticket.scope)
            })
            .ok_or_else(invalid_handle)?;
        let _ = active;
        state.sessions.retain(|_, item| {
            if item.entry.valid().is_err() {
                item.entry.revoked.store(true, Ordering::SeqCst);
                false
            } else {
                true
            }
        });
        if state
            .sessions
            .values()
            .filter(|item| item.window == ticket.window)
            .count()
            >= HISTORY_SESSIONS_PER_WINDOW
        {
            let oldest = state
                .sessions
                .iter()
                .filter(|(_, item)| item.window == ticket.window)
                .min_by_key(|(_, item)| item.accessed)
                .map(|(id, _)| id.clone())
                .ok_or_else(internal)?;
            if let Some(item) = state.sessions.remove(&oldest) {
                item.entry.revoked.store(true, Ordering::SeqCst);
            }
        }
        let now = Instant::now();
        state.sessions.insert(
            session.id.clone(),
            Stored {
                window: ticket.window.clone(),
                context: session.context.clone(),
                epoch: ticket.epoch,
                accessed: now,
                entry: Arc::new(Entry {
                    session: AsyncMutex::new(session),
                    revoked: AtomicBool::new(false),
                    created: now,
                }),
            },
        );
        Ok(())
    }
    pub(crate) fn with_ticket<T>(
        &self,
        ticket: &Ticket,
        action: impl FnOnce() -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        let state = self.state.lock().map_err(|_| internal())?;
        state
            .windows
            .get(&ticket.window)
            .filter(|w| {
                w.enabled && w.epoch == ticket.epoch && w.scope.as_ref() == Some(&ticket.scope)
            })
            .ok_or_else(invalid_handle)?;
        action()
    }
    pub(crate) fn get(
        &self,
        window: &str,
        context: &HistoryContext,
        id: &str,
    ) -> Result<Arc<Entry>, AppError> {
        let mut state = self.state.lock().map_err(|_| internal())?;
        let epoch = state
            .windows
            .get(window)
            .filter(|w| w.enabled && w.scope.as_ref() == Some(&context.scope))
            .ok_or_else(invalid_handle)?
            .epoch;
        let item = state
            .sessions
            .get_mut(id)
            .filter(|s| s.window == window && s.context == *context && s.epoch == epoch)
            .ok_or_else(invalid_handle)?;
        item.entry.valid()?;
        item.accessed = Instant::now();
        Ok(item.entry.clone())
    }
    pub(crate) fn get_context(
        &self,
        window: &str,
        context: &HistoryContext,
    ) -> Result<Arc<Entry>, AppError> {
        let mut state = self.state.lock().map_err(|_| internal())?;
        let epoch = state
            .windows
            .get(window)
            .filter(|w| w.enabled && w.scope.as_ref() == Some(&context.scope))
            .ok_or_else(invalid_handle)?
            .epoch;
        let item = state
            .sessions
            .values_mut()
            .find(|s| s.window == window && s.context == *context && s.epoch == epoch)
            .ok_or_else(invalid_handle)?;
        item.entry.valid()?;
        item.accessed = Instant::now();
        Ok(item.entry.clone())
    }
}
