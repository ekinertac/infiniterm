//! Windows job objects: the one mechanism that makes a process tree die
//! when it should.
//!
//! Two callers, for the same reason from opposite ends. `backend::local_pty`
//! puts each pane's shell in a job, because `ChildKiller::kill` ends the one
//! process portable-pty spawned and under ConPTY that is not the tree (the
//! shell sits beside an OpenConsole of its own, and its own children are
//! untouched). `main.rs` puts the WHOLE APP in a job, because Chromium's
//! subprocesses do not follow their parent out: killing infiniterm and
//! waiting eight seconds left all eight of them running, measured
//! 2026-09-25.
//!
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` is what does the work in both. The
//! kernel closes the handle when the owning process ends, however it ends —
//! a clean quit, a panic, a taskkill, a crash — and everything left in the
//! job goes with it. That is a stronger guarantee than any shutdown code
//! could make, because it does not depend on our code running.
//!
//! Jobs NEST on Windows 8 and later, which is why the app-wide job and the
//! per-pane jobs can both exist: a pane's shell is in its own job inside the
//! app's. On an older Windows the inner assignment fails and a pane simply
//! leaks its tree the way it did before jobs existed, which is why every
//! failure here is a `None` rather than an error worth stopping for.
//!
//! Related: `backend/local_pty.rs` (the pane side), `infiniterm-ui/main.rs`
//! (the app side and the quit that relies on it).
#![cfg(windows)]

use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, TerminateProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
};

/// A job holding one process and everything it starts.
pub struct Job(OwnedHandle);

impl Job {
    /// An empty job set to kill its members when its last handle closes.
    fn empty() -> Option<Job> {
        let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if job.is_null() {
            return None;
        }
        let job = unsafe { OwnedHandle::from_raw_handle(job as _) };

        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let set = unsafe {
            SetInformationJobObject(
                job.as_raw_handle() as _,
                JobObjectExtendedLimitInformation,
                std::ptr::addr_of!(limits).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        (set != 0).then_some(Job(job))
    }

    /// A job holding the process with this pid.
    ///
    /// `None` when any step fails: a pane with no job still runs, it just
    /// leaks its tree the way every pane did before this existed. Not worth
    /// refusing to open a card over.
    pub fn holding(pid: u32) -> Option<Job> {
        let job = Job::empty()?;
        // A handle on the child, which portable-pty does not hand out; it
        // gives a pid, and the job wants the process.
        let child = unsafe { OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid) };
        if child.is_null() {
            return None;
        }
        let assigned = unsafe { AssignProcessToJobObject(job.0.as_raw_handle() as _, child) };
        unsafe { CloseHandle(child) };
        (assigned != 0).then_some(job)
    }

    /// A job holding THIS process, so every child it has not reaped dies
    /// with it.
    ///
    /// The returned handle must outlive everything: dropping it closes the
    /// job and kills us along with the tree. The caller leaks it on purpose
    /// (`adopt_this_process`).
    pub fn holding_this_process() -> Option<Job> {
        let job = Job::empty()?;
        let assigned =
            unsafe { AssignProcessToJobObject(job.0.as_raw_handle() as _, GetCurrentProcess()) };
        (assigned != 0).then_some(job)
    }

    /// Ends every process in the job, which is the shell and everything it
    /// started.
    pub fn terminate(&self) {
        unsafe { TerminateJobObject(self.0.as_raw_handle() as _, 1) };
    }
}

/// End this process immediately, running nothing on the way out.
///
/// `std::process::exit` is not enough with CEF loaded: it runs atexit
/// handlers and every DLL's `DLL_PROCESS_DETACH`, and libcef's does not
/// return. Measured 2026-09-25 — the call took the whole process tree down
/// (the job did its work, six of seven processes gone) and then sat in its
/// own teardown past a twenty-five second wait, which is the same hang as
/// `cef_shutdown` by a shorter road. `TerminateProcess` on ourselves skips
/// all of it.
///
/// Only safe because of `adopt_this_process`: nothing we own is left
/// unflushed by the callers (the canvas and the window frame are written
/// first), and the children that would otherwise be orphaned are in the job
/// the kernel is about to close.
pub fn exit_now() -> ! {
    unsafe { TerminateProcess(GetCurrentProcess(), 0) };
    // TerminateProcess is asynchronous: it returns before the process is
    // gone. Nothing may run after it, so park here rather than fall out
    // into code that assumed it never came back.
    loop {
        std::thread::park();
    }
}

/// Put this process in a job that takes its children down with it, and hold
/// the handle for the rest of the process's life.
///
/// Called once, early in `main`, BEFORE CEF starts any subprocess, or the
/// ones already running would not be in the job. Deliberately leaked: the
/// handle has to stay open until the kernel closes it at process exit, which
/// is the moment that does the killing.
///
/// Silent when it fails. A machine where this does not work is one where
/// Chromium's processes can outlive the app, which is where every Windows
/// build was until this existed; it is not a reason to refuse to start.
pub fn adopt_this_process() {
    if let Some(job) = Job::holding_this_process() {
        std::mem::forget(job);
    }
}
