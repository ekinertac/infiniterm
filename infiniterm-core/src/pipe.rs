//! A Windows named pipe that a reader and a writer can use AT THE SAME TIME,
//! with an owner-only access list: the transport `ift join` needs on a
//! Windows host (Windows only).
//!
//! Why not `transport.rs`: its pipe is a plain `File`, which is a synchronous
//! handle, and Windows serialises synchronous I/O on one handle. A thread
//! blocked in `ReadFile` (waiting for the viewer's keys) would hold up the
//! `WriteFile` on another thread (sending the terminal's output), so a
//! duplex connection deadlocks. That is harmless for the hook and `ift`
//! channel, which is one request and one reply, and fatal for a viewer that
//! types while the host prints. Here every handle is opened with
//! `FILE_FLAG_OVERLAPPED` and every call waits on its own event, so reads and
//! writes on a handle run side by side, and `shutdown` can cancel what is
//! waiting.
//!
//! The shape follows a unix stream so `backend/join.rs` and `ift proxy` read
//! the same on both: `PipeStream` is `Read + Write`, `try_clone` gives a
//! second handle on the same connection, `shutdown` wakes whatever is blocked
//! on it (a connection ends only when every copy is dropped, and a writer
//! parked on a queue holds one), and `connect` is the client half.
//!
//! Security: the join pipe gives the control of every card, like `ift send`,
//! and a pipe's default access list lets other local accounts read it. The
//! listener's list names the current user's SID and nobody else (an elevated
//! ssh session of the same user has the same SID). The first instance is
//! created with `FILE_FLAG_FIRST_PIPE_INSTANCE`, so a process that created
//! the name first makes `bind` fail instead of being served our clients, and
//! a client opens with anonymous impersonation so a server it did not mean to
//! reach cannot act as the user. Remote (SMB) clients are refused.
//!
//! Related: transport.rs (the hook and `ift` channel, unchanged),
//! paths.rs (`join_socket`, the name), backend/join.rs (the server) and
//! infiniterm-cli/src/proxy.rs (the client), docs/windows-handoff.md
//! ("Join on Windows").

use std::io::{self, Read, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use windows_sys::Win32::Foundation::{
    CloseHandle, LocalFree, ERROR_BROKEN_PIPE, ERROR_FILE_NOT_FOUND, ERROR_IO_PENDING,
    ERROR_NO_DATA, ERROR_OPERATION_ABORTED, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED,
    ERROR_PIPE_NOT_CONNECTED, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
    WAIT_OBJECT_0,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, TokenUser, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED,
    OPEN_EXISTING, PIPE_ACCESS_DUPLEX, SECURITY_ANONYMOUS, SECURITY_SQOS_PRESENT,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, WaitNamedPipeW, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows_sys::Win32::System::Threading::{
    CreateEventW, GetCurrentProcess, OpenProcessToken, WaitForSingleObject, INFINITE,
};
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};

/// A pipe buffer each way: a frame of terminal output is up to 64 KiB, so a
/// frame is one write without waiting for the reader mid-frame.
const BUF: u32 = 64 * 1024;
/// How long a client waits for a busy instance to free up, per attempt.
const BUSY_WAIT_MS: u32 = 2000;
/// Attempts at an instance that is busy or not yet created.
const CONNECT_TRIES: u32 = 5;

/// The connection's handle and whether `shutdown` was called, shared by every
/// clone: the handle closes when the last clone is dropped.
struct Inner {
    handle: OwnedHandle,
    closed: AtomicBool,
}

/// One end of a connection. Clones share it.
#[derive(Clone)]
pub struct PipeStream(Arc<Inner>);

impl PipeStream {
    fn new(handle: OwnedHandle) -> Self {
        PipeStream(Arc::new(Inner {
            handle,
            closed: AtomicBool::new(false),
        }))
    }

    /// A second handle on the same connection (a clone of the `Arc`: there is
    /// one overlapped handle, and overlapped I/O is what lets two threads use
    /// it at once).
    pub fn try_clone(&self) -> io::Result<PipeStream> {
        Ok(self.clone())
    }

    /// Ends the connection for everyone using it: marks it closed and cancels
    /// every read and write waiting on it, which then report end of file or a
    /// broken pipe. The peer sees the hang-up when the last clone is dropped
    /// (the handle closes then). `how` is only there so a call reads like a
    /// unix socket's.
    pub fn shutdown(&self, _how: std::net::Shutdown) -> io::Result<()> {
        self.0.closed.store(true, Ordering::SeqCst);
        // Cancels this process's pending I/O on the handle; fails with "not
        // found" when nothing is pending, which is fine.
        unsafe { CancelIoEx(raw(&self.0.handle), std::ptr::null()) };
        Ok(())
    }

    /// Whether `shutdown` has been called on this connection.
    pub fn is_shut_down(&self) -> bool {
        self.0.closed.load(Ordering::SeqCst)
    }

    /// One read or write, waited for. `op` starts it with the OVERLAPPED it is
    /// given; this waits for completion on that call's own event, so the
    /// buffer and the OVERLAPPED outlive the I/O.
    fn io(&self, op: impl FnOnce(*mut OVERLAPPED) -> i32) -> Result<u32, u32> {
        let handle = raw(&self.0.handle);
        let event = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
        if event.is_null() {
            return Err(last_error());
        }
        let _event = Event(event);
        let mut ov: OVERLAPPED = unsafe { std::mem::zeroed() };
        ov.hEvent = event;
        let started = op(&mut ov);
        if started == 0 {
            let err = last_error();
            if err != ERROR_IO_PENDING {
                return Err(err);
            }
            // `shutdown` between this call starting the I/O and now would
            // have missed it: it sets the flag before it cancels, so the flag
            // is seen here and this call cancels itself.
            if self.0.closed.load(Ordering::SeqCst) {
                unsafe { CancelIoEx(handle, &ov) };
            }
            if unsafe { WaitForSingleObject(event, INFINITE) } != WAIT_OBJECT_0 {
                // Never leave with I/O in flight: it writes into `ov`.
                unsafe { CancelIoEx(handle, &ov) };
                let mut n = 0;
                unsafe { GetOverlappedResult(handle, &ov, &mut n, 1) };
                return Err(ERROR_OPERATION_ABORTED);
            }
        }
        let mut n = 0u32;
        if unsafe { GetOverlappedResult(handle, &ov, &mut n, 0) } == 0 {
            return Err(last_error());
        }
        Ok(n)
    }
}

impl std::fmt::Debug for PipeStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PipeStream(shut down: {})", self.is_shut_down())
    }
}

fn last_error() -> u32 {
    io::Error::last_os_error().raw_os_error().unwrap_or(0) as u32
}

fn raw(h: &OwnedHandle) -> HANDLE {
    h.as_raw_handle() as HANDLE
}

struct Event(HANDLE);
impl Drop for Event {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

/// What a read or write ending in `err` means to a caller: the peer hanging
/// up, or the connection being shut down here, is the end, not a failure.
fn is_hangup(err: u32) -> bool {
    matches!(
        err,
        ERROR_BROKEN_PIPE | ERROR_PIPE_NOT_CONNECTED | ERROR_NO_DATA | ERROR_OPERATION_ABORTED
    )
}

impl Read for PipeStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        (&*self).read(buf)
    }
}

impl Read for &PipeStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() || self.is_shut_down() {
            return Ok(0);
        }
        let handle = raw(&self.0.handle);
        let len = buf.len().min(u32::MAX as usize) as u32;
        let ptr = buf.as_mut_ptr();
        match self.io(|ov| unsafe { ReadFile(handle, ptr.cast(), len, std::ptr::null_mut(), ov) }) {
            Ok(n) => Ok(n as usize),
            Err(e) if is_hangup(e) => Ok(0),
            Err(e) => Err(io::Error::from_raw_os_error(e as i32)),
        }
    }
}

impl Write for PipeStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        (&*self).write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Write for &PipeStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.is_shut_down() {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        let handle = raw(&self.0.handle);
        let len = buf.len().min(u32::MAX as usize) as u32;
        let ptr = buf.as_ptr();
        match self.io(|ov| unsafe { WriteFile(handle, ptr.cast(), len, std::ptr::null_mut(), ov) })
        {
            Ok(n) => Ok(n as usize),
            Err(e) if is_hangup(e) => Err(io::ErrorKind::BrokenPipe.into()),
            Err(e) => Err(io::Error::from_raw_os_error(e as i32)),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

fn pipe_name(path: &Path) -> io::Result<Vec<u16>> {
    let text = path.to_string_lossy();
    const PREFIX: &str = r"\\.\pipe\";
    if !text.starts_with(PREFIX) || text.len() <= PREFIX.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{text} is not a named pipe path"),
        ));
    }
    Ok(wide(&text))
}

/// The security descriptor text that names one user and nobody else.
pub fn owner_only_sddl(user_sid: &str) -> String {
    // D: a DACL, P: it does not inherit; one ACE: allow, all access, to the SID.
    format!("D:P(A;;GA;;;{user_sid})")
}

/// The current user's SID as text (`S-1-5-21-...`).
fn current_user_sid() -> io::Result<String> {
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(io::Error::last_os_error());
        }
        let _token = Event(token);
        let mut needed = 0u32;
        GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut needed);
        let mut buf = vec![0u8; needed as usize];
        if GetTokenInformation(
            token,
            TokenUser,
            buf.as_mut_ptr().cast(),
            needed,
            &mut needed,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut text: *mut u16 = std::ptr::null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut text) == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut len = 0;
        while *text.add(len) != 0 {
            len += 1;
        }
        let sid = String::from_utf16_lossy(std::slice::from_raw_parts(text, len));
        LocalFree(text.cast());
        Ok(sid)
    }
}

/// A security descriptor built from SDDL, freed on drop.
struct Descriptor {
    raw: *mut core::ffi::c_void,
}

impl Descriptor {
    fn from_sddl(sddl: &str) -> io::Result<Descriptor> {
        let text = wide(sddl);
        let mut raw = std::ptr::null_mut();
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                text.as_ptr(),
                SDDL_REVISION_1,
                &mut raw,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Descriptor { raw })
    }
}

impl Drop for Descriptor {
    fn drop(&mut self) {
        unsafe { LocalFree(self.raw) };
    }
}

// The descriptor is plain memory, read only after it is built.
unsafe impl Send for Descriptor {}

/// A bound pipe name, accepting connections.
pub struct PipeListener {
    name: Vec<u16>,
    descriptor: Descriptor,
    /// The instance already waiting, so the name exists from `bind` on.
    pending: Option<OwnedHandle>,
}

impl PipeListener {
    /// Binds `path` (`\\.\pipe\<name>`), owner-only. Fails if the name already
    /// exists, whoever made it.
    pub fn bind(path: &Path) -> io::Result<PipeListener> {
        let name = pipe_name(path)?;
        let descriptor = Descriptor::from_sddl(&owner_only_sddl(&current_user_sid()?))?;
        let first = create_instance(&name, &descriptor, true)?;
        Ok(PipeListener {
            name,
            descriptor,
            pending: Some(first),
        })
    }

    /// Waits for one client. The next instance is created before this one is
    /// handed over, so a client never finds the name with nobody listening.
    pub fn accept(&mut self) -> io::Result<PipeStream> {
        let handle = match self.pending.take() {
            Some(h) => h,
            None => create_instance(&self.name, &self.descriptor, false)?,
        };
        let stream = PipeStream::new(handle);
        let result = stream.io(|ov| unsafe { ConnectNamedPipe(raw(&stream.0.handle), ov) });
        match result {
            Ok(_) => {}
            // The client connected between the create and this call.
            Err(e) if e == ERROR_PIPE_CONNECTED => {}
            Err(e) => return Err(io::Error::from_raw_os_error(e as i32)),
        }
        self.pending = create_instance(&self.name, &self.descriptor, false).ok();
        Ok(stream)
    }

    /// Connections as they arrive; an error is yielded and the loop goes on.
    pub fn incoming(mut self) -> impl Iterator<Item = io::Result<PipeStream>> {
        std::iter::from_fn(move || Some(self.accept()))
    }
}

fn create_instance(name: &[u16], descriptor: &Descriptor, first: bool) -> io::Result<OwnedHandle> {
    let attrs = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.raw,
        bInheritHandle: 0,
    };
    let mut open_mode = PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED;
    if first {
        open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
    }
    let h = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            open_mode,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            BUF,
            BUF,
            0,
            &attrs,
        )
    };
    if h == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { OwnedHandle::from_raw_handle(h as _) })
}

/// Opens a connection to a listening pipe. A busy instance is waited for,
/// and a name with no instance yet is retried briefly (the gap between one
/// accept handing a client over and the next instance being created).
pub fn connect(path: &Path) -> io::Result<PipeStream> {
    let name = pipe_name(path)?;
    let mut last = io::Error::from(io::ErrorKind::NotFound);
    for attempt in 0..CONNECT_TRIES {
        let h = unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_ANONYMOUS,
                std::ptr::null_mut(),
            )
        };
        if h != INVALID_HANDLE_VALUE {
            return Ok(PipeStream::new(unsafe {
                OwnedHandle::from_raw_handle(h as _)
            }));
        }
        let err = last_error();
        last = io::Error::from_raw_os_error(err as i32);
        if err == ERROR_PIPE_BUSY {
            unsafe { WaitNamedPipeW(name.as_ptr(), BUSY_WAIT_MS) };
        } else if err == ERROR_FILE_NOT_FOUND && attempt + 1 < CONNECT_TRIES {
            // Not there yet, or between two instances; a name that never
            // appears is reported after the last try.
            std::thread::sleep(std::time::Duration::from_millis(30));
        } else {
            break;
        }
    }
    Err(last)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::endpoint;
    use std::sync::mpsc;
    use std::time::Duration;

    fn pair(tag: &str) -> (PipeStream, PipeStream) {
        let path = endpoint(tag);
        let mut listener = PipeListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || listener.accept().unwrap());
        let client = connect(&path).unwrap();
        (server.join().unwrap(), client)
    }

    #[test]
    fn the_access_list_names_one_user_and_does_not_inherit() {
        assert_eq!(
            owner_only_sddl("S-1-5-21-1-2-3-1001"),
            "D:P(A;;GA;;;S-1-5-21-1-2-3-1001)"
        );
        let sid = current_user_sid().unwrap();
        assert!(sid.starts_with("S-1-5-"), "{sid}");
    }

    // The reason this module exists: a read blocked on a handle must not stop
    // a write on the same handle from another thread.
    #[test]
    fn a_blocked_read_does_not_hold_up_a_write_on_the_same_connection() {
        let (server, client) = pair("duplex");
        let (tx, rx) = mpsc::channel();
        // The server's reader blocks: the client sends nothing yet.
        let mut reader = server.try_clone().unwrap();
        std::thread::spawn(move || {
            let mut b = [0u8; 16];
            let n = reader.read(&mut b).unwrap();
            tx.send(b[..n].to_vec()).unwrap();
        });
        std::thread::sleep(Duration::from_millis(200));
        // Meanwhile the server writes 3 MB; the client must get all of it.
        let payload: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
        let want = payload.clone();
        let mut writer = server.try_clone().unwrap();
        let w = std::thread::spawn(move || writer.write_all(&payload).unwrap());
        let mut client_read = client.try_clone().unwrap();
        let mut got = vec![0u8; want.len()];
        client_read.read_exact(&mut got).unwrap();
        w.join().unwrap();
        assert!(got == want, "3 MB arrived intact while a read was blocked");
        // Now the client speaks, and the blocked read wakes with it.
        (&client).write_all(b"keys").unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), b"keys");
    }

    #[test]
    fn shutdown_wakes_a_blocked_read_and_a_dropped_peer_is_end_of_file() {
        let (server, client) = pair("shutdown");
        let (tx, rx) = mpsc::channel();
        let mut reader = server.try_clone().unwrap();
        std::thread::spawn(move || {
            let mut b = [0u8; 4];
            tx.send(reader.read(&mut b).unwrap()).unwrap();
        });
        std::thread::sleep(Duration::from_millis(200));
        server.shutdown(std::net::Shutdown::Both).unwrap();
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            0,
            "the blocked read ended"
        );
        assert!(
            (&server).write_all(b"x").is_err(),
            "a shut down connection takes no more writes"
        );
        // The peer: dropping every copy of the server end is end of file.
        drop(server);
        let mut b = [0u8; 4];
        assert_eq!((&client).read(&mut b).unwrap(), 0);
    }

    /// The access list of a REAL bound pipe, read back from the kernel object:
    /// one entry, for the current user, and no one else (no Everyone, no
    /// Administrators group, no SYSTEM).
    #[test]
    fn a_bound_pipe_lets_in_the_current_user_and_nobody_else() {
        use windows_sys::Win32::Security::Authorization::{
            ConvertSecurityDescriptorToStringSecurityDescriptorW, GetSecurityInfo, SE_KERNEL_OBJECT,
        };
        use windows_sys::Win32::Security::DACL_SECURITY_INFORMATION;
        let listener = PipeListener::bind(&endpoint("acl")).unwrap();
        let handle = raw(listener.pending.as_ref().unwrap());
        let sddl = unsafe {
            let mut sd = std::ptr::null_mut();
            let rc = GetSecurityInfo(
                handle,
                SE_KERNEL_OBJECT,
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut sd,
            );
            assert_eq!(rc, 0, "GetSecurityInfo");
            let mut text: *mut u16 = std::ptr::null_mut();
            assert_ne!(
                ConvertSecurityDescriptorToStringSecurityDescriptorW(
                    sd,
                    SDDL_REVISION_1,
                    DACL_SECURITY_INFORMATION,
                    &mut text,
                    std::ptr::null_mut(),
                ),
                0
            );
            let mut len = 0;
            while *text.add(len) != 0 {
                len += 1;
            }
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(text, len));
            LocalFree(text.cast());
            LocalFree(sd);
            s
        };
        let me = current_user_sid().unwrap();
        assert_eq!(sddl.matches("(A;").count(), 1, "one allow entry: {sddl}");
        assert!(!sddl.contains("(D;"), "no deny entries: {sddl}");
        assert!(sddl.contains(&me), "the entry is for this user: {sddl}");
        assert!(sddl.starts_with("D:P"), "protected, not inherited: {sddl}");
        // The well-known broad groups are not in it.
        for broad in [";WD)", ";BA)", ";SY)", ";AU)", ";BU)"] {
            assert!(!sddl.contains(broad), "{broad} is not allowed: {sddl}");
        }
    }

    #[test]
    fn a_second_bind_of_the_same_name_fails_and_a_missing_name_is_not_found() {
        let path = endpoint("twice");
        let _first = PipeListener::bind(&path).unwrap();
        assert!(PipeListener::bind(&path).is_err());
        let missing = endpoint("never-bound");
        let err = connect(&missing).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound, "{err}");
        assert!(PipeListener::bind(Path::new(r"C:\not\a\pipe")).is_err());
    }
}
