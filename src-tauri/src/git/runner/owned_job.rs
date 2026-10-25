//! A zero active-process count is not an exit/handle-release fence.
//! Proof requires lifetime accounting completeness AND every retained identity
//! signaled. IOCP is a discovery hint, never the completeness oracle.
use std::{
    collections::BTreeMap,
    mem::{size_of, zeroed},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc, Arc, Mutex, OnceLock,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, DuplicateHandle, GetLastError, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
        WAIT_TIMEOUT,
    },
    System::{
        Diagnostics::ToolHelp::*,
        JobObjects::*,
        Threading::{
            GetCurrentProcess, GetProcessId, OpenProcess, OpenThread, ResumeThread,
            WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
            THREAD_SUSPEND_RESUME,
        },
        IO::{CreateIoCompletionPort, GetQueuedCompletionStatus, PostQueuedCompletionStatus},
    },
};

const MAX_JOBS: usize = 64;
const MAX_IDENTITIES: usize = 1024;
const STOP_TIMEOUT: Duration = Duration::from_secs(5);
// winnt.h JOB_OBJECT_MSG_NEW_PROCESS; private IOCP has a single job/key.
const NEW_PROCESS: u32 = 6;
const JOB_KEY: usize = 1;
const STOP_KEY: usize = 2;
static ADMISSION: OnceLock<Arc<AtomicUsize>> = OnceLock::new();
#[derive(Debug)]
pub(super) enum AttachError {
    Setup,
    Capacity,
}
impl From<()> for AttachError {
    fn from(_: ()) -> Self {
        Self::Setup
    }
}

struct Handle(HANDLE);
// Owned OS handles may be queried concurrently; close happens after last Arc.
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
#[derive(Default)]
struct Tracker {
    // A held process handle pins its identity/PID even after exit. Thus a
    // duplicate PID cannot silently represent another lifetime here.
    processes: BTreeMap<u32, Handle>,
    failed: bool,
    last_total: u32,
}
struct Resources {
    admission: Arc<AtomicUsize>,
    job: Handle,
    port: Handle,
    tracker: Mutex<Tracker>,
    stop: AtomicBool,
    finished: AtomicBool,
    termination_failed: AtomicBool,
    #[cfg(test)]
    faults: Faults,
    #[cfg(test)]
    barrier_ack: AtomicBool,
    #[cfg(test)]
    root_pid: u32,
}
impl Drop for Resources {
    fn drop(&mut self) {
        self.admission.fetch_sub(1, Ordering::SeqCst);
    }
}
struct Job {
    resources: Arc<Resources>,
    collector: Mutex<Option<JoinHandle<()>>>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.resources.request_stop();
        // Never join a live collector in Drop (including task abort). Its only
        // ownership is Resources, with no reference back to Job or caller.
        if let Ok(thread) = self.collector.get_mut() {
            if thread.as_ref().is_some_and(JoinHandle::is_finished) {
                let _ = thread.take().unwrap().join();
            }
        }
    }
}
#[derive(Clone)]
pub struct OwnedJob(Arc<Job>);
impl OwnedJob {
    pub fn contains(&self, pid: u32) -> bool {
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if process.is_null() {
            return false;
        }
        self.0.resources.member(&Handle(process))
    }
    pub fn terminate(&self) {
        if unsafe { TerminateJobObject(self.0.resources.job.0, 1) } == 0 {
            self.0
                .resources
                .termination_failed
                .store(true, Ordering::SeqCst);
        }
    }
    pub(super) async fn stop_and_wait(&self) -> Result<(), ()> {
        self.stop_until(Instant::now() + STOP_TIMEOUT).await
    }
    async fn stop_until(&self, deadline: Instant) -> Result<(), ()> {
        self.terminate();
        let resources = &self.0.resources;
        if resources.termination_failed.load(Ordering::SeqCst) {
            return Err(());
        }
        loop {
            if Instant::now() >= deadline {
                return Err(());
            }
            let accounting = resources.accounting()?;
            if resources.complete(&accounting)? {
                // All lifetime identities are now signaled and no process is
                // active, so none can create an uncounted late descendant.
                resources.request_stop();
                while !resources.finished.load(Ordering::SeqCst) {
                    if Instant::now() >= deadline {
                        return Err(());
                    }
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
                let final_accounting = resources.accounting()?;
                if final_accounting.TotalProcesses != accounting.TotalProcesses
                    || !resources.complete(&final_accounting)?
                {
                    return Err(());
                }
                if let Ok(mut collector) = self.0.collector.lock() {
                    if collector.as_ref().is_some_and(JoinHandle::is_finished) {
                        let _ = collector.take().unwrap().join();
                    }
                }
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }
    fn quarantine(self, lease: Option<Arc<dyn Send + Sync>>) {
        self.terminate();
        self.0.resources.request_stop();
        // Intentionally retain bounded pure OS resources/leases until process
        // exit. No session/callback/DB/scheduler/credential ownership is kept.
        // The collector observes stop and exits; retaining Job is not a cycle.
        std::mem::forget((self, lease));
    }
}
pub(super) struct TerminationGuard {
    job: Option<OwnedJob>,
    lease: Option<Arc<dyn Send + Sync>>,
}
impl TerminationGuard {
    pub(super) fn new(job: OwnedJob, lease: Option<Arc<dyn Send + Sync>>) -> Self {
        Self {
            job: Some(job),
            lease,
        }
    }
    pub(super) fn confirmed(&mut self) {
        self.job.take();
    }
}
impl Drop for TerminationGuard {
    fn drop(&mut self) {
        if let Some(job) = self.job.take() {
            job.quarantine(self.lease.take());
        }
    }
}
impl Resources {
    fn request_stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        // GQCS also has a finite timeout if posting fails.
        unsafe {
            PostQueuedCompletionStatus(self.port.0, 0, STOP_KEY, std::ptr::null());
        }
    }
    fn member(&self, process: &Handle) -> bool {
        let mut member = 0;
        unsafe { IsProcessInJob(process.0, self.job.0, &mut member) != 0 && member != 0 }
    }
    fn fail(&self) {
        if let Ok(mut tracker) = self.tracker.lock() {
            tracker.failed = true;
        }
    }
    fn capture(&self, pid: u32) {
        let Ok(mut tracker) = self.tracker.lock() else {
            return;
        };
        if tracker.processes.contains_key(&pid) {
            return;
        }
        #[cfg(test)]
        let cap = self.faults.cap;
        #[cfg(not(test))]
        let cap = MAX_IDENTITIES;
        if pid == 0 || tracker.processes.len() >= cap {
            tracker.failed = true;
            return;
        }
        #[cfg(test)]
        if self.faults.deny_open {
            tracker.failed = true;
            return;
        }
        let process = unsafe {
            OpenProcess(
                PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
                0,
                pid,
            )
        };
        if process.is_null() {
            tracker.failed = true;
            return;
        }
        let process = Handle(process);
        if !self.member(&process) {
            tracker.failed = true;
            return;
        }
        tracker.processes.insert(pid, process);
    }
    fn accounting(&self) -> Result<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, ()> {
        #[cfg(test)]
        if self.faults.accounting_fail {
            return Err(());
        }
        let mut accounting = unsafe { zeroed() };
        if unsafe {
            QueryInformationJobObject(
                self.job.0,
                JobObjectBasicAccountingInformation,
                &mut accounting as *mut _ as _,
                size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(());
        }
        Ok(accounting)
    }
    fn complete(&self, accounting: &JOBOBJECT_BASIC_ACCOUNTING_INFORMATION) -> Result<bool, ()> {
        let mut tracker = self.tracker.lock().map_err(|_| ())?;
        if accounting.TotalProcesses < tracker.last_total
            || accounting.TotalProcesses as usize > MAX_IDENTITIES
        {
            tracker.failed = true;
        }
        tracker.last_total = accounting.TotalProcesses;
        if tracker.failed {
            return Err(());
        }
        if accounting.ActiveProcesses != 0
            || accounting.TotalProcesses as usize != tracker.processes.len()
        {
            return Ok(false);
        }
        #[cfg(test)]
        if self.faults.wait_timeout {
            return Ok(false);
        }
        for process in tracker.processes.values() {
            match unsafe { WaitForSingleObject(process.0, 0) } {
                WAIT_OBJECT_0 => (),
                WAIT_TIMEOUT => return Ok(false),
                _ => return Err(()),
            }
        }
        Ok(true)
    }
}
fn collect(resources: &Resources) {
    #[cfg(test)]
    if resources.faults.iocp_fail {
        resources.fail();
        return;
    }
    while !resources.stop.load(Ordering::SeqCst) {
        let (mut message, mut key, mut overlapped) = (0, 0, std::ptr::null_mut());
        let ok = unsafe {
            GetQueuedCompletionStatus(
                resources.port.0,
                &mut message,
                &mut key,
                &mut overlapped,
                50,
            )
        };
        if ok == 0 {
            if unsafe { GetLastError() } != WAIT_TIMEOUT {
                resources.fail();
                return;
            }
            continue;
        }
        #[cfg(test)]
        if key == 3 {
            resources.barrier_ack.store(true, Ordering::SeqCst);
            continue;
        }
        if key == STOP_KEY {
            return;
        }
        if key != JOB_KEY {
            resources.fail();
            return;
        }
        if message == NEW_PROCESS {
            #[cfg(test)]
            if resources.faults.drop_new {
                continue;
            }
            #[cfg(test)]
            if !resources.faults.delay_new.is_zero() {
                // Test-only delay is interruptible, so abort cannot strand a thread.
                let until = Instant::now() + resources.faults.delay_new;
                while Instant::now() < until && !resources.stop.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(1));
                }
                if resources.stop.load(Ordering::SeqCst) {
                    return;
                }
            }
            let pid = overlapped as usize;
            if pid > u32::MAX as usize {
                resources.fail();
                return;
            }
            #[cfg(test)]
            if resources.faults.drop_root_only && pid as u32 == resources.root_pid {
                continue;
            }
            resources.capture(pid as u32);
            #[cfg(test)]
            if resources.faults.duplicate_new {
                resources.capture(pid as u32);
            }
        }
    }
}
pub(super) fn attach(child: &tokio::process::Child) -> Result<OwnedJob, AttachError> {
    attach_inner(
        child,
        ADMISSION
            .get_or_init(|| Arc::new(AtomicUsize::new(0)))
            .clone(),
        #[cfg(test)]
        Faults::default(),
    )
}
fn attach_inner(
    child: &tokio::process::Child,
    admission: Arc<AtomicUsize>,
    #[cfg(test)] faults: Faults,
) -> Result<OwnedJob, AttachError> {
    // Reserve before creating resources. A quarantined job keeps its slot;
    // repeated uncertainty can never accumulate unbounded collectors/handles.
    admission
        .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
            (n < MAX_JOBS).then_some(n + 1)
        })
        .map_err(|_| AttachError::Capacity)?;
    let result = unsafe {
        let raw_job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if raw_job.is_null() {
            admission.fetch_sub(1, Ordering::SeqCst);
            return Err(AttachError::Setup);
        }
        let job = Handle(raw_job);
        let raw_port = CreateIoCompletionPort(INVALID_HANDLE_VALUE, std::ptr::null_mut(), 0, 1);
        if raw_port.is_null() {
            admission.fetch_sub(1, Ordering::SeqCst);
            return Err(AttachError::Setup);
        }
        let resources = Arc::new(Resources {
            admission,
            job,
            port: Handle(raw_port),
            tracker: Mutex::new(Tracker::default()),
            stop: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            termination_failed: AtomicBool::new(false),
            #[cfg(test)]
            faults,
            #[cfg(test)]
            barrier_ack: AtomicBool::new(false),
            #[cfg(test)]
            root_pid: child.id().unwrap_or(0),
        });
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let association = JOBOBJECT_ASSOCIATE_COMPLETION_PORT {
            CompletionKey: JOB_KEY as *mut _,
            CompletionPort: resources.port.0,
        };
        if SetInformationJobObject(
            resources.job.0,
            JobObjectExtendedLimitInformation,
            &limits as *const _ as _,
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        ) == 0
            || SetInformationJobObject(
                resources.job.0,
                JobObjectAssociateCompletionPortInformation,
                &association as *const _ as _,
                size_of::<JOBOBJECT_ASSOCIATE_COMPLETION_PORT>() as u32,
            ) == 0
        {
            return Err(AttachError::Setup);
        }
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let thread_resources = resources.clone();
        let collector = std::thread::Builder::new()
            .name("opengit-job-identities".into())
            .spawn(move || {
                let _ = ready_tx.send(());
                if std::panic::catch_unwind(|| collect(&thread_resources)).is_err() {
                    thread_resources.fail();
                }
                thread_resources.finished.store(true, Ordering::SeqCst);
            })
            .map_err(|_| ())?;
        let owned = OwnedJob(Arc::new(Job {
            resources: resources.clone(),
            collector: Mutex::new(Some(collector)),
        }));
        if ready_rx.recv_timeout(Duration::from_secs(1)).is_err() {
            resources.request_stop();
            return Err(AttachError::Setup);
        }
        // Duplicate the original handle, not a PID lookup, before first resume.
        let mut root = std::ptr::null_mut();
        if DuplicateHandle(
            GetCurrentProcess(),
            child.raw_handle().ok_or(())? as HANDLE,
            GetCurrentProcess(),
            &mut root,
            PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
            0,
            0,
        ) == 0
        {
            return Err(AttachError::Setup);
        }
        let root = Handle(root);
        // Hold the map lock across assignment and insertion: a duplicate root
        // notification cannot race the independently verified direct handle.
        let mut tracker = resources.tracker.lock().map_err(|_| ())?;
        if AssignProcessToJobObject(resources.job.0, child.raw_handle().ok_or(())? as HANDLE) == 0
            || !resources.member(&root)
        {
            return Err(AttachError::Setup);
        }
        let pid = GetProcessId(root.0);
        if pid == 0 || Some(pid) != child.id() {
            return Err(AttachError::Setup);
        }
        tracker.processes.insert(pid, root);
        drop(tracker);
        Ok(owned)
    };
    result
}
pub(super) fn resume(child: &tokio::process::Child) -> Result<(), ()> {
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(());
        }
        let snapshot = Handle(snapshot);
        let mut entry: THREADENTRY32 = zeroed();
        entry.dwSize = size_of::<THREADENTRY32>() as u32;
        let mut next = Thread32First(snapshot.0, &mut entry);
        while next != 0 {
            if Some(entry.th32OwnerProcessID) == child.id() {
                let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID);
                if thread.is_null() {
                    return Err(());
                }
                let thread = Handle(thread);
                return if ResumeThread(thread.0) != u32::MAX {
                    Ok(())
                } else {
                    Err(())
                };
            }
            next = Thread32Next(snapshot.0, &mut entry);
        }
        Err(())
    }
}

#[cfg(test)]
#[derive(Clone)]
struct Faults {
    drop_new: bool,
    drop_root_only: bool,
    delay_new: Duration,
    duplicate_new: bool,
    deny_open: bool,
    accounting_fail: bool,
    wait_timeout: bool,
    iocp_fail: bool,
    cap: usize,
}
#[cfg(test)]
impl Default for Faults {
    fn default() -> Self {
        Self {
            drop_new: false,
            drop_root_only: false,
            delay_new: Duration::ZERO,
            duplicate_new: false,
            deny_open: false,
            accounting_fail: false,
            wait_timeout: false,
            iocp_fail: false,
            cap: MAX_IDENTITIES,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        domain::contracts::*,
        git::{
            error::{error, sanitized_error},
            runner::{Cancellation, GitRunner, RunOptions},
        },
    };
    use std::{
        os::windows::fs::OpenOptionsExt,
        path::{Path, PathBuf},
        process::Stdio,
    };

    fn binary() -> PathBuf {
        static FIXTURE: OnceLock<tempfile::TempDir> = OnceLock::new();
        let root = FIXTURE.get_or_init(|| {
            let root = tempfile::tempdir().unwrap();
            let source = root.path().join("job-fixture.rs");
            std::fs::write(&source, r#"use std::{process::{Command,Stdio},time::Duration,os::windows::fs::OpenOptionsExt};
fn main(){let args:Vec<_>=std::env::args_os().collect();let root=std::path::Path::new(&args[2]);
if args[1]=="tree"{Command::new(&args[0]).arg("child").arg(root).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();}
let mode=if args[1]=="child"{"child"}else{"root"};
let _lock=std::fs::OpenOptions::new().create(true).write(true).truncate(true).share_mode(0).open(root.join(format!("{mode}.lock"))).unwrap();
std::fs::write(root.join(format!("{mode}.ready")),b"ready").unwrap();std::thread::sleep(Duration::from_secs(30));}"#).unwrap();
            let rustc = std::env::split_paths(&std::env::var_os("PATH").unwrap()).filter(|p| p.is_absolute()).map(|p| p.join("rustc.exe")).find(|p| p.is_file()).unwrap();
            let compile = std::process::Command::new(rustc).arg(&source).arg("-o").arg(root.path().join("fixture.exe")).output().unwrap();
            assert!(compile.status.success(), "controlled job fixture compile failed");
            root
        });
        root.path().join("fixture.exe")
    }
    async fn ready(path: &Path) {
        tokio::time::timeout(Duration::from_secs(10), async {
            while !path.exists() {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
    }
    fn suspended(
        faults: Faults,
        tree: bool,
    ) -> (
        tempfile::TempDir,
        tokio::process::Child,
        OwnedJob,
        Arc<AtomicUsize>,
    ) {
        let root = tempfile::tempdir().unwrap();
        let mut child = tokio::process::Command::new(binary());
        child
            .arg(if tree { "tree" } else { "direct" })
            .arg(root.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(0x00000004 | 0x08000000)
            .kill_on_drop(true);
        let child = child.spawn().unwrap();
        // Each fault case has its own gate; never reset/poison production state.
        let admission = Arc::new(AtomicUsize::new(0));
        let job = attach_inner(&child, admission.clone(), faults).unwrap();
        (root, child, job, admission)
    }
    async fn launch(
        faults: Faults,
        tree: bool,
    ) -> (
        tempfile::TempDir,
        tokio::process::Child,
        OwnedJob,
        Arc<AtomicUsize>,
    ) {
        let (root, child, job, admission) = suspended(faults, tree);
        resume(&child).unwrap();
        ready(
            &root
                .path()
                .join(if tree { "child.ready" } else { "root.ready" }),
        )
        .await;
        (root, child, job, admission)
    }
    fn lease(path: &Path) -> Arc<std::fs::File> {
        Arc::new(
            std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .share_mode(3)
                .open(path)
                .unwrap(),
        )
    }
    #[tokio::test]
    async fn missing_delayed_unopenable_capped_or_failed_tracking_never_claims_quiescence() {
        let cases = [
            Faults {
                drop_new: true,
                ..Default::default()
            },
            Faults {
                delay_new: Duration::from_millis(500),
                ..Default::default()
            },
            Faults {
                deny_open: true,
                ..Default::default()
            },
            Faults {
                cap: 1,
                ..Default::default()
            },
            Faults {
                accounting_fail: true,
                ..Default::default()
            },
            Faults {
                wait_timeout: true,
                ..Default::default()
            },
            Faults {
                iocp_fail: true,
                ..Default::default()
            },
        ];
        for (index, faults) in cases.into_iter().enumerate() {
            let (root, mut child, job, admission) = launch(faults, true).await;
            let file = root.path().join("pure-lease");
            let held = lease(&file);
            let weak_lease = Arc::downgrade(&held);
            let weak_job = Arc::downgrade(&job.0);
            let resources = job.0.resources.clone();
            let guard = TerminationGuard::new(job.clone(), Some(held));
            let started = Instant::now();
            assert!(
                job.stop_until(Instant::now() + Duration::from_millis(80))
                    .await
                    .is_err(),
                "case {index} must fail closed"
            );
            assert!(started.elapsed() < Duration::from_secs(2));
            drop(guard);
            drop(job);
            ready_finished(&resources).await;
            child.wait().await.unwrap();
            assert!(
                weak_job.upgrade().is_some(),
                "uncertain Job must remain pinned"
            );
            assert!(
                weak_lease.upgrade().is_some(),
                "pure file lease must survive failure"
            );
            assert!(std::fs::rename(&file, root.path().join("moved")).is_err());
            assert_eq!(admission.load(Ordering::SeqCst), 1);
            let recovery = sanitized_error(&error(ErrorCode::Interrupted, Phase::Recovery));
            assert_eq!(recovery.retry, AppErrorRetry::None);
            assert!(recovery.message.contains("Restart OpenGit"));
        }
    }
    async fn ready_finished(resources: &Resources) {
        tokio::time::timeout(Duration::from_secs(1), async {
            while !resources.finished.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
    }
    #[tokio::test]
    async fn duplicate_notifications_deduplicate_and_lost_root_notification_uses_original_handle() {
        for (faults, tree) in [
            (
                Faults {
                    duplicate_new: true,
                    ..Default::default()
                },
                true,
            ),
            (
                Faults {
                    drop_root_only: true,
                    ..Default::default()
                },
                false,
            ),
        ] {
            let (root, mut child, job, admission) = suspended(faults, tree);
            if tree {
                // No descendant can exist while the direct root is suspended.
                // FIFO barrier acknowledges both deliberately duplicated root
                // notifications, rather than assuming a total tree size.
                assert_eq!(job.0.resources.tracker.lock().unwrap().processes.len(), 1);
                for _ in 0..2 {
                    assert_ne!(
                        unsafe {
                            PostQueuedCompletionStatus(
                                job.0.resources.port.0,
                                NEW_PROCESS,
                                JOB_KEY,
                                child.id().unwrap() as usize as *const _,
                            )
                        },
                        0
                    );
                }
                assert_ne!(
                    unsafe {
                        PostQueuedCompletionStatus(job.0.resources.port.0, 0, 3, std::ptr::null())
                    },
                    0
                );
                tokio::time::timeout(Duration::from_secs(1), async {
                    while !job.0.resources.barrier_ack.load(Ordering::SeqCst) {
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                })
                .await
                .unwrap();
                let tracker = job.0.resources.tracker.lock().unwrap();
                assert!(!tracker.failed);
                assert_eq!(tracker.processes.len(), 1);
                drop(tracker);
            }
            resume(&child).unwrap();
            ready(
                &root
                    .path()
                    .join(if tree { "child.ready" } else { "root.ready" }),
            )
            .await;
            // The fixture creates no more processes after publishing ready.
            // Windows may add other legitimate members; account for all of them.
            let expected = job.0.resources.accounting().unwrap().TotalProcesses as usize;
            assert!(expected >= if tree { 2 } else { 1 });
            tokio::time::timeout(Duration::from_secs(1), async {
                loop {
                    let count = {
                        let tracker = job.0.resources.tracker.lock().unwrap();
                        assert!(!tracker.failed);
                        tracker.processes.len()
                    };
                    if count == expected {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            })
            .await
            .unwrap();
            let resources = Arc::downgrade(&job.0.resources);
            job.stop_and_wait().await.unwrap();
            assert_eq!(
                job.0.resources.tracker.lock().unwrap().processes.len(),
                expected
            );
            child.wait().await.unwrap();
            drop(job);
            tokio::time::timeout(Duration::from_secs(1), async {
                while resources.upgrade().is_some() {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            })
            .await
            .unwrap();
            assert_eq!(admission.load(Ordering::SeqCst), 0);
        }
    }
    #[tokio::test]
    async fn repeated_success_releases_job_port_process_handles_and_collector_without_cycles() {
        for _ in 0..20 {
            let (_root, mut child, job, admission) = launch(Faults::default(), false).await;
            let weak_job = Arc::downgrade(&job.0);
            let weak_resources = Arc::downgrade(&job.0.resources);
            job.stop_and_wait().await.unwrap();
            child.wait().await.unwrap();
            drop(job);
            tokio::time::timeout(Duration::from_secs(1), async {
                while weak_resources.upgrade().is_some() {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            })
            .await
            .unwrap();
            assert!(weak_job.upgrade().is_none());
            assert_eq!(admission.load(Ordering::SeqCst), 0);
        }
    }
    #[tokio::test]
    async fn task_abort_quarantines_only_job_resources_and_pure_lease_then_stops_collector() {
        let (root, child, job, admission) = launch(Faults::default(), true).await;
        let lease_path = root.path().join("abort-lease");
        let held = lease(&lease_path);
        let weak = Arc::downgrade(&held);
        let resources = job.0.resources.clone();
        let guard = TerminationGuard::new(job.clone(), Some(held));
        let task = tokio::spawn(async move {
            let _child = child;
            let _guard = guard;
            std::future::pending::<()>().await;
        });
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        drop(job);
        ready_finished(&resources).await;
        assert!(weak.upgrade().is_some());
        assert!(std::fs::rename(&lease_path, root.path().join("moved")).is_err());
        assert_eq!(admission.load(Ordering::SeqCst), 1);
    }
    #[tokio::test]
    async fn callback_error_stops_suspended_process_and_releases_guard_after_proof() {
        let root = tempfile::tempdir().unwrap();
        let held = lease(&root.path().join("lease"));
        let weak = Arc::downgrade(&held);
        let runner = GitRunner::trusted(binary()).unwrap();
        let result = runner
            .run_with(
                None,
                &["direct".into(), root.path().as_os_str().into()],
                &Cancellation::default(),
                Phase::Inspection,
                RunOptions {
                    lifetime_guard: Some(held),
                    before_resume: Some(Arc::new(|_| {
                        Err(error(ErrorCode::InvalidRequest, Phase::Inspection))
                    })),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert_eq!(result.code, ErrorCode::InvalidRequest);
        assert!(weak.upgrade().is_none());
        assert!(!root.path().join("root.ready").exists());
        std::fs::rename(root.path().join("lease"), root.path().join("released")).unwrap();
        // Normal cancellation keeps its original safe template/phase.
        assert_eq!(
            error(ErrorCode::Interrupted, Phase::Transfer).message,
            "Operation was interrupted."
        );
    }
    #[tokio::test]
    async fn full_resource_gate_rejects_without_assignment_and_does_not_leak_a_slot() {
        let root = tempfile::tempdir().unwrap();
        let mut child = tokio::process::Command::new(binary())
            .arg("direct")
            .arg(root.path())
            .creation_flags(0x00000004 | 0x08000000)
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let admission = Arc::new(AtomicUsize::new(MAX_JOBS));
        assert!(matches!(
            attach_inner(&child, admission.clone(), Faults::default()),
            Err(AttachError::Capacity)
        ));
        assert_eq!(admission.load(Ordering::SeqCst), MAX_JOBS);
        child.kill().await.unwrap();
        assert!(!root.path().join("root.ready").exists());
    }
}
