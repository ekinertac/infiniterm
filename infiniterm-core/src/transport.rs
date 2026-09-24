//! The one local IPC channel: hook reports in, and `ift` requests in and
//! out. Two implementations behind one API, because the app, the hook
//! binary and `ift` must all agree on where to knock.
//!
//! On unix that channel is a unix socket at `paths::socket_path()` (see
//! `hooks.rs` for why one socket serves both message shapes). Windows has
//! no unix sockets in std, so it is a named pipe at
//! `\\.\pipe\infiniterm[-<hash of the data dir>]` instead; `paths.rs`
//! decides the name, this file only speaks it. A named pipe suits the job
//! better than a file socket in one way: a crashed instance leaves nothing
//! behind, so there is no stale-file dance on the Windows side.
//!
//! Callers: `hooks::listen` (the server), `app.rs` (its tests),
//! `infiniterm-ui/src/runtime.rs` (the "is it up" probe) and
//! `infiniterm-cli/src/socket.rs` (`ift`). The hook binary keeps its own
//! zero-dependency copy of the CLIENT half, which needs nothing from here.
//!
//! The existence of a listener IS the answer to "is infiniterm running",
//! which is why `is_live` is a connect and not a handshake.

use std::io::{self, Read, Write};
use std::path::Path;

/// One accepted connection: read the request lines, write the replies.
pub struct Stream(inner::Stream);

/// A bound endpoint. Dropping it stops accepting.
pub struct Listener(inner::Listener);

impl Stream {
    /// A second handle on the same connection, so a reader and a writer can
    /// live on one thread each without sharing a lock.
    pub fn try_clone(&self) -> io::Result<Stream> {
        self.0.try_clone().map(Stream)
    }
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf)
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl Listener {
    /// Blocks for each connection in turn. Errors are yielded rather than
    /// ending the iterator: one refused client must not take the listener
    /// down with it.
    pub fn incoming(self) -> impl Iterator<Item = io::Result<Stream>> {
        self.0.incoming().map(|r| r.map(Stream))
    }
}

/// Binds `path` and returns the listener. Any leftover from a previous run
/// is cleared first (unix only; a named pipe has nothing to leave behind).
pub fn bind(path: &Path) -> io::Result<Listener> {
    inner::bind(path).map(Listener)
}

/// Opens a connection to a running instance.
pub fn connect(path: &Path) -> io::Result<Stream> {
    inner::connect(path).map(Stream)
}

/// Something is listening: another instance is running.
///
/// A connect, and deliberately nothing cleverer. On Windows a force-killed
/// process's pipe goes on answering for a few seconds while the kernel tears
/// its handles down, so a relaunch inside that window is told the endpoint is
/// held and exits. That was measured and then made WORSE by trying to fix it:
/// asking the pipe which process serves it (`GetNamedPipeServerProcessId`)
/// and whether that process is alive looks exact, but while a dying
/// instance's handle lingers beside a live one's, a client can land on
/// either, and landing on the dead one let a SECOND instance start on the
/// same save file. Refusing to launch for three seconds after a crash is the
/// cheaper failure.
pub fn is_live(path: &Path) -> bool {
    connect(path).is_ok()
}

#[cfg(unix)]
mod inner {
    use std::io;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::Path;

    pub type Stream = UnixStream;

    pub struct Listener(UnixListener);

    impl Listener {
        pub fn incoming(self) -> impl Iterator<Item = io::Result<Stream>> {
            // `UnixListener::incoming` borrows; this owns the listener for
            // the iterator's life instead, which is what callers want.
            std::iter::from_fn(move || Some(self.0.accept().map(|(s, _)| s)))
        }
    }

    pub fn bind(path: &Path) -> io::Result<Listener> {
        // A stale socket file from a crash blocks the bind; the live-socket
        // check that keeps a second instance out is `is_live`, run first.
        let _ = std::fs::remove_file(path);
        UnixListener::bind(path).map(Listener)
    }

    pub fn connect(path: &Path) -> io::Result<Stream> {
        UnixStream::connect(path)
    }
}

#[cfg(windows)]
mod inner {
    use std::io::{self, Read, Write};
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::path::Path;
    use std::time::Duration;
    use windows_sys::Win32::Foundation::{
        ERROR_NO_DATA, ERROR_PIPE_CONNECTED, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
    use windows_sys::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE,
        PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    };

    /// Generous enough that a hook report (a few hundred bytes) or an `ift`
    /// reply (a few kilobytes) never blocks mid-write.
    const BUF: u32 = 64 * 1024;

    /// The one moment a client can find no listening instance is between
    /// `ConnectNamedPipe` returning and the accept loop creating the next
    /// instance: one syscall wide. Retrying a handful of times covers it
    /// without an overlapped-IO accept loop.
    /// shortcut: a fixed retry rather than a pool of pre-created instances,
    /// fine for a single-user desktop app; grow the pool if this ever has to
    /// serve many clients at once.
    const CONNECT_TRIES: u32 = 4;
    const CONNECT_WAIT: Duration = Duration::from_millis(20);

    /// A pipe handle. Reads and writes go through `File`, which is
    /// `ReadFile`/`WriteFile` and correct for a byte-mode pipe; the one
    /// difference from a socket is that the far end hanging up surfaces as
    /// `BrokenPipe` rather than end of file, which `read` translates so
    /// `BufReader::lines()` ends the way it does on unix.
    pub struct Stream(std::fs::File);

    impl Stream {
        pub fn try_clone(&self) -> io::Result<Stream> {
            self.0.try_clone().map(Stream)
        }
    }

    impl Read for Stream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            match self.0.read(buf) {
                Err(e) if e.kind() == io::ErrorKind::BrokenPipe => Ok(0),
                other => other,
            }
        }
    }

    impl Write for Stream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.write(buf)
        }
        fn flush(&mut self) -> io::Result<()> {
            self.0.flush()
        }
    }

    pub struct Listener {
        name: Vec<u16>,
        /// The instance already listening, so the name exists from the
        /// moment `bind` returns rather than from the first `incoming`.
        pending: Option<OwnedHandle>,
    }

    impl Listener {
        pub fn incoming(mut self) -> impl Iterator<Item = io::Result<Stream>> {
            std::iter::from_fn(move || Some(self.accept()))
        }

        fn accept(&mut self) -> io::Result<Stream> {
            let handle = match self.pending.take() {
                Some(h) => h,
                None => create_instance(&self.name)?,
            };
            let ok = unsafe { ConnectNamedPipe(handle.as_raw_handle() as _, std::ptr::null_mut()) };
            if ok == 0 {
                let e = io::Error::last_os_error();
                // PIPE_CONNECTED: the client got in between the create and
                // this call. NO_DATA: it connected, wrote and hung up before
                // we got here, which is exactly what the hook binary does —
                // its bytes are still in the buffer, so this is a connection
                // with an early EOF, not a failure.
                let code = e.raw_os_error();
                if code != Some(ERROR_PIPE_CONNECTED as i32) && code != Some(ERROR_NO_DATA as i32) {
                    return Err(e);
                }
            }
            // Listen again before handing this one off, so the window with
            // no instance waiting is as short as it can be.
            self.pending = create_instance(&self.name).ok();
            Ok(Stream(handle.into()))
        }
    }

    fn wide(path: &Path) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        path.as_os_str().encode_wide().chain([0]).collect()
    }

    fn create_instance(name: &[u16]) -> io::Result<OwnedHandle> {
        let h = unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                PIPE_UNLIMITED_INSTANCES,
                BUF,
                BUF,
                0,
                std::ptr::null(),
            )
        };
        if h == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        Ok(unsafe { OwnedHandle::from_raw_handle(h as _) })
    }

    pub fn bind(path: &Path) -> io::Result<Listener> {
        // CreateNamedPipeW only accepts this shape, and a caller that hands
        // over a plain file path is asking for a confusing failure later.
        let text = path.to_string_lossy();
        const PREFIX: &str = r"\\.\pipe\";
        if !text.starts_with(PREFIX) || text.len() <= PREFIX.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{text} is not a named pipe path"),
            ));
        }
        let name = wide(path);
        let pending = create_instance(&name)?;
        Ok(Listener {
            name,
            pending: Some(pending),
        })
    }

    pub fn connect(path: &Path) -> io::Result<Stream> {
        let mut last = None;
        for attempt in 0..CONNECT_TRIES {
            match std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
            {
                Ok(f) => return Ok(Stream(f)),
                Err(e) => last = Some(e),
            }
            if attempt + 1 < CONNECT_TRIES {
                std::thread::sleep(CONNECT_WAIT);
            }
        }
        Err(last.unwrap_or_else(|| io::Error::from(io::ErrorKind::NotFound)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::endpoint;
    use std::io::{BufRead, BufReader};

    // The round trip both callers need: a client writes a line, the server
    // answers one.
    #[test]
    fn a_client_line_reaches_the_server_and_the_reply_comes_back() {
        let path = endpoint("roundtrip");
        let listener = bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            let stream = listener.incoming().next().unwrap().unwrap();
            let mut out = stream.try_clone().unwrap();
            let mut line = String::new();
            BufReader::new(stream).read_line(&mut line).unwrap();
            writeln!(out, "saw {}", line.trim()).unwrap();
            out.flush().unwrap();
        });

        let mut client = connect(&path).unwrap();
        writeln!(client, "hello").unwrap();
        client.flush().unwrap();
        let mut reply = String::new();
        BufReader::new(client.try_clone().unwrap())
            .read_line(&mut reply)
            .unwrap();
        assert_eq!(reply.trim(), "saw hello");
        server.join().unwrap();
        let _ = std::fs::remove_file(&path);
    }

    // The single-instance lock: nothing bound means nothing running.
    #[test]
    fn is_live_is_false_until_something_binds() {
        let path = endpoint("islive");
        assert!(!is_live(&path));
        let listener = bind(&path).unwrap();
        // Accepting must happen off this thread: on Windows a connect only
        // completes once the server accepts it.
        let server = std::thread::spawn(move || {
            let _ = listener.incoming().next();
        });
        assert!(is_live(&path));
        server.join().unwrap();
        let _ = std::fs::remove_file(&path);
    }

    // A reader must see end of stream when the writer drops, or the server's
    // per-connection thread never ends.
    #[test]
    fn a_dropped_client_ends_the_servers_read() {
        let path = endpoint("eof");
        let listener = bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            let stream = listener.incoming().next().unwrap().unwrap();
            BufReader::new(stream).lines().map_while(Result::ok).count()
        });
        let mut client = connect(&path).unwrap();
        writeln!(client, "one").unwrap();
        writeln!(client, "two").unwrap();
        client.flush().unwrap();
        drop(client);
        assert_eq!(server.join().unwrap(), 2);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn bind_fails_on_a_path_it_cannot_serve() {
        #[cfg(windows)]
        let bad = std::path::PathBuf::from(r"C:\nonexistent-dir\infiniterm.sock");
        #[cfg(unix)]
        let bad = std::path::PathBuf::from("/nonexistent-dir/infiniterm.sock");
        assert!(bind(&bad).is_err());
    }
}
