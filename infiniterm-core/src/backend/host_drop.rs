//! Sending a file, a folder or a pasted image from a joined window to the host
//! (#556), so the program in the card can read it.
//!
//! In a joined window the shell runs on the host. A path dropped or pasted
//! there is a path on THIS Mac, which an agent on the host cannot read. So the
//! viewer copies the thing over the ssh it already has into
//! `~/.infiniterm/drops/<id>/` on the host and the card is given that path.
//!
//! A file goes through `cat`, a folder through `tar` (packed to a temp file
//! here first, since `run_on_stdin` takes a file), and a clipboard image is
//! written to a temp file named `clipboard.<ext>` first. Each call is one ssh
//! and one drop folder, so two files dropped together land side by side. Drop
//! folders older than a week are removed by the next send (the same ssh, no
//! timer on the host).
//!
//! Called by the ui's `send_to_host` on a thread of its own (a transfer waits
//! on the network). Related: remote.rs (`run_on_stdin`, `RemoteHost`), drop.rs
//! (what a drop becomes in a local card).

use std::path::Path;

use super::remote::{run_on_stdin, RemoteHost};
use crate::drop::posix_quote as shell_quote;
use crate::file_source::{Code, FileError, FileSource};

// Shared with the host's file service, which does the same storing for a send
// over the file connection (`FileSource::put_drop`).
pub use crate::file_source::{host_name, DROPS_DIR, KEEP_DAYS};

/// The biggest file sent over the file connection: it goes whole in one
/// request and sits in memory at both ends. A bigger one keeps using ssh and
/// `cat`, which stream.
pub const SERVICE_MAX: u64 = 64 * 1024 * 1024;

/// Whether a failed send over the file connection should be tried over ssh
/// instead: the host's `ift` does not know the request, the connection could
/// not be made, or the file is too big for it. Any other failure is real.
fn try_ssh_instead(e: &FileError) -> bool {
    matches!(e.code, Code::Unsupported | Code::Lost | Code::TooLarge)
}

/// `send_path` over the host's file connection when it will take the file
/// (one request, no new login); over ssh when it will not, and for a folder
/// (a tar keeps its links and modes).
pub fn send_path_via(
    source: &dyn FileSource,
    host: &RemoteHost,
    local: &Path,
    id: &str,
) -> Result<String, String> {
    let small = std::fs::metadata(local).is_ok_and(|m| m.is_file() && m.len() <= SERVICE_MAX);
    if small {
        let name = host_name(
            &local
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        );
        let bytes = std::fs::read(local).map_err(|e| format!("could not read {name}: {e}"))?;
        match source.put_drop(id, &name, &bytes) {
            Ok(path) => return Ok(path),
            Err(e) if try_ssh_instead(&e) => {}
            Err(e) => return Err(format!("could not send {name} to the host: {e}")),
        }
    }
    send_path(host, local, id)
}

/// `send_image` over the file connection, with the same fallback.
pub fn send_image_via(
    source: &dyn FileSource,
    host: &RemoteHost,
    bytes: &[u8],
    ext: &str,
    id: &str,
) -> Result<String, String> {
    let name = format!("clipboard.{}", host_name(ext));
    match source.put_drop(id, &name, bytes) {
        Ok(path) => Ok(path),
        Err(e) if try_ssh_instead(&e) => send_image(host, bytes, ext, id),
        Err(e) => Err(format!("could not send the image to the host: {e}")),
    }
}

/// The remote command that makes the drop folder `id`, tidies old ones, runs
/// `body` inside it (which reads stdin) and prints the full path of `name`.
fn command(id: &str, name: &str, body: &str) -> String {
    format!(
        "root=\"$HOME/{DROPS_DIR}\"; d=\"$root/{id}\"; \
         find \"$root\" -mindepth 1 -maxdepth 1 -type d -mtime +{KEEP_DAYS} -exec rm -rf {{}} + 2>/dev/null; \
         mkdir -p \"$d\" && {body} && printf '%s' \"$d\"/{name}",
        name = shell_quote(name)
    )
}

/// The command for a plain file named `name`: stdin becomes the file.
pub fn file_command(id: &str, name: &str) -> String {
    command(id, name, &format!("cat > \"$d\"/{}", shell_quote(name)))
}

/// The command for a folder packed as a tar: stdin is unpacked inside the
/// drop folder, and the folder's own name is the path printed.
pub fn folder_command(id: &str, name: &str) -> String {
    command(id, name, "tar -xf - -C \"$d\"")
}

/// What the host printed: the path, when the command worked.
fn host_path(out: std::io::Result<std::process::Output>, what: &str) -> Result<String, String> {
    let out = out.map_err(|e| format!("could not send {what}: {e}"))?;
    let said = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if out.status.success() && !said.is_empty() {
        return Ok(said);
    }
    let why = String::from_utf8_lossy(&out.stderr).trim().to_string();
    Err(format!(
        "could not send {what} to the host: {}",
        why.lines().last().unwrap_or("it failed")
    ))
}

/// Copies a file or a folder from this Mac to the host's drop folder `id`;
/// the host's path of it.
pub fn send_path(host: &RemoteHost, local: &Path, id: &str) -> Result<String, String> {
    let name = host_name(
        &local
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
    );
    let what = name.clone();
    if local.is_dir() {
        // Packed here, unpacked there: a folder's links and modes survive and
        // it is one ssh.
        let tar = std::env::temp_dir().join(format!("ift-drop-{}-{id}.tar", std::process::id()));
        let parent = local.parent().unwrap_or(Path::new("/"));
        let packed = std::process::Command::new("tar")
            .arg("-cf")
            .arg(&tar)
            .arg("-C")
            .arg(parent)
            .arg(local.file_name().unwrap_or_default())
            .status()
            .map_err(|e| format!("could not pack {what}: {e}"))?;
        if !packed.success() {
            let _ = std::fs::remove_file(&tar);
            return Err(format!("could not pack {what}"));
        }
        let input = std::fs::File::open(&tar).map_err(|e| format!("could not read {what}: {e}"));
        let out = input.and_then(|f| {
            run_on_stdin(host, &folder_command(id, &name), f).map_err(|e| e.to_string())
        });
        let _ = std::fs::remove_file(&tar);
        return host_path(out.map_err(std::io::Error::other), &what);
    }
    let input = std::fs::File::open(local).map_err(|e| format!("could not read {what}: {e}"))?;
    host_path(run_on_stdin(host, &file_command(id, &name), input), &what)
}

/// Copies a pasted image (its bytes, and the extension its format has) to the
/// host as `clipboard.<ext>`; the host's path of it.
pub fn send_image(host: &RemoteHost, bytes: &[u8], ext: &str, id: &str) -> Result<String, String> {
    let name = format!("clipboard.{}", host_name(ext));
    let tmp = std::env::temp_dir().join(format!("ift-drop-{}-{id}.img", std::process::id()));
    std::fs::write(&tmp, bytes).map_err(|e| format!("could not stage the image: {e}"))?;
    let result = std::fs::File::open(&tmp)
        .map_err(|e| format!("could not stage the image: {e}"))
        .and_then(|f| host_path(run_on_stdin(host, &file_command(id, &name), f), "the image"));
    let _ = std::fs::remove_file(&tmp);
    result
}

/// A fresh drop folder id: the time and a counter, so two sends never meet.
pub fn new_id() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("{ms}-{}", N.fetch_add(1, Ordering::Relaxed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    use crate::file_source::{EntryKind, FileData, HostPaths, ListEntry, Stat};
    use std::sync::Mutex;

    /// A file connection whose answer to a send the test chooses; it records
    /// what was sent.
    struct Connection {
        answer: Mutex<Result<String, FileError>>,
        sent: Mutex<Vec<(String, String, usize)>>,
    }

    impl Connection {
        fn new(answer: Result<String, FileError>) -> Connection {
            Connection {
                answer: Mutex::new(answer),
                sent: Mutex::new(vec![]),
            }
        }
    }

    impl FileSource for Connection {
        fn is_local(&self) -> bool {
            false
        }
        fn put_drop(&self, id: &str, name: &str, bytes: &[u8]) -> Result<String, FileError> {
            self.sent
                .lock()
                .unwrap()
                .push((id.into(), name.into(), bytes.len()));
            self.answer.lock().unwrap().clone()
        }
        fn home(&self) -> Result<HostPaths, FileError> {
            unreachable!()
        }
        fn stat(&self, _: &str) -> Result<Stat, FileError> {
            unreachable!()
        }
        fn exists(&self, _: &[String]) -> Result<Vec<Option<EntryKind>>, FileError> {
            unreachable!()
        }
        fn list(&self, _: &str) -> Result<Vec<ListEntry>, FileError> {
            unreachable!()
        }
        fn read(&self, _: &str) -> Result<FileData, FileError> {
            unreachable!()
        }
        fn write(&self, _: &str, _: &[u8], _: Option<u64>) -> Result<Option<u64>, FileError> {
            unreachable!()
        }
    }

    /// An ssh that cannot be run: a send that reaches it is a send that did
    /// not use the file connection.
    fn no_ssh() -> RemoteHost {
        let mut host = RemoteHost::new("h");
        host.ssh = "/nonexistent/ssh".into();
        host
    }

    // A file and a pasted image go over the file connection when it takes
    // them: one request each, the host's answer is the path, and no ssh runs.
    #[test]
    fn a_file_and_an_image_go_over_the_file_connection_with_no_ssh() {
        let dir = std::env::temp_dir().join(format!("hd-{}-via", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let local = dir.join("it's a shot.png");
        std::fs::write(&local, b"12345").unwrap();
        let conn = Connection::new(Ok("/host/drops/1-0/x".into()));

        assert_eq!(
            send_path_via(&conn, &no_ssh(), &local, "1-0").unwrap(),
            "/host/drops/1-0/x"
        );
        assert_eq!(
            send_image_via(&conn, &no_ssh(), b"png", "png", "1-0").unwrap(),
            "/host/drops/1-0/x"
        );
        assert_eq!(
            *conn.sent.lock().unwrap(),
            vec![
                ("1-0".to_string(), "it's a shot.png".to_string(), 5),
                ("1-0".to_string(), "clipboard.png".to_string(), 3),
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // A host whose ift does not know the request, or no connection, or a file
    // too big for one request: ssh does it as before. A real refusal is not
    // retried over ssh, which would hide it.
    #[test]
    fn the_old_ssh_way_is_the_fallback_and_a_real_failure_is_not_hidden() {
        let (host, dir) = stand_in("via-fallback");
        let local = dir.join("a.txt");
        std::fs::write(&local, b"by ssh").unwrap();
        for code in [Code::Unsupported, Code::Lost, Code::TooLarge] {
            let conn = Connection::new(Err(FileError::new(code, "no")));
            let path = send_path_via(&conn, &host, &local, &new_id()).unwrap();
            assert_eq!(std::fs::read(&path).unwrap(), b"by ssh", "{code:?}");
            let image = send_image_via(&conn, &host, b"img", "png", &new_id()).unwrap();
            assert_eq!(std::fs::read(&image).unwrap(), b"img", "{code:?}");
        }

        let refused = Connection::new(Err(FileError::new(Code::Denied, "permission denied")));
        let err = send_path_via(&refused, &no_ssh(), &local, "1-0").unwrap_err();
        assert!(
            err.contains("a.txt") && err.contains("permission denied"),
            "{err}"
        );
        let err = send_image_via(&refused, &no_ssh(), b"x", "png", "1-0").unwrap_err();
        assert!(err.contains("permission denied"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // A folder is a tar over ssh (links and modes survive), never a request.
    #[test]
    fn a_folder_is_sent_over_ssh_not_the_file_connection() {
        let (host, dir) = stand_in("via-folder");
        let local = dir.join("project");
        std::fs::create_dir_all(&local).unwrap();
        std::fs::write(local.join("a.rs"), "fn a() {}").unwrap();
        let conn = Connection::new(Ok("/should/not/be/used".into()));
        let path = send_path_via(&conn, &host, &local, &new_id()).unwrap();
        assert!(path.ends_with("/project"), "{path}");
        assert!(conn.sent.lock().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_name_is_one_safe_path_component() {
        assert_eq!(host_name("shot 1.png"), "shot 1.png");
        assert_eq!(host_name("/a/b/c.txt"), "c.txt");
        assert_eq!(host_name("a\nb"), "a_b");
        assert_eq!(host_name(".."), "file");
        assert_eq!(host_name(""), "file");
        assert_eq!(host_name("x/"), "file");
    }

    #[test]
    fn the_remote_commands_quote_the_name_and_keep_the_folder_to_a_week() {
        let c = file_command("1-0", "it's a \"file\" $(rm -rf ~).png");
        // The name is quoted by drop::shell_quote everywhere it appears.
        assert!(c.contains("'it'\\''s a \"file\" $(rm -rf ~).png'"), "{c}");
        assert!(
            c.contains("-mtime +7") && c.contains("$HOME/.infiniterm/drops"),
            "{c}"
        );
        assert!(folder_command("1-0", "dir").contains("tar -xf -"));
    }

    /// A stand-in ssh that runs the remote command here with HOME set to a
    /// scratch folder, as the host's login shell would.
    fn stand_in(name: &str) -> (RemoteHost, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("hd-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("home")).unwrap();
        let script = dir.join("ssh");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nfor last; do :; done\nHOME='{}' exec sh -c \"$last\"\n",
                dir.join("home").display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        // The script was just written; a parallel test's fork may hold its
        // write descriptor for a moment (ETXTBSY), so wait until it runs.
        for _ in 0..200 {
            match std::process::Command::new(&script)
                .arg("true")
                .stdin(std::process::Stdio::null())
                .output()
            {
                Err(e) if e.raw_os_error() == Some(26) => {
                    std::thread::sleep(std::time::Duration::from_millis(10))
                }
                _ => break,
            }
        }
        let mut host = RemoteHost::new("h");
        host.ssh = script.display().to_string();
        (host, dir)
    }

    #[test]
    fn a_file_with_an_awkward_name_arrives_whole_and_its_host_path_is_printed() {
        let (host, dir) = stand_in("file");
        let local = dir.join("it's a \"shot\" 1.png");
        std::fs::write(&local, b"\x89PNG bytes").unwrap();
        let id = new_id();
        let path = send_path(&host, &local, &id).unwrap();
        assert_eq!(
            path,
            dir.join("home/.infiniterm/drops")
                .join(&id)
                .join("it's a \"shot\" 1.png")
                .to_string_lossy()
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"\x89PNG bytes");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_folder_arrives_with_its_files_and_a_pasted_image_as_clipboard_png() {
        let (host, dir) = stand_in("folder");
        let local = dir.join("project");
        std::fs::create_dir_all(local.join("src")).unwrap();
        std::fs::write(local.join("src/a.rs"), "fn a() {}").unwrap();
        let id = new_id();
        let path = send_path(&host, &local, &id).unwrap();
        assert!(path.ends_with(&format!("{id}/project")), "{path}");
        assert_eq!(
            std::fs::read_to_string(std::path::Path::new(&path).join("src/a.rs")).unwrap(),
            "fn a() {}"
        );
        let image = send_image(&host, b"pngdata", "png", &new_id()).unwrap();
        assert!(image.ends_with("/clipboard.png"), "{image}");
        assert_eq!(std::fs::read(&image).unwrap(), b"pngdata");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn two_sends_land_in_folders_of_their_own_and_old_folders_are_tidied() {
        let (host, dir) = stand_in("tidy");
        let drops = dir.join("home/.infiniterm/drops");
        std::fs::create_dir_all(drops.join("ancient")).unwrap();
        // A folder a fortnight old: touched back in time.
        let old = std::process::Command::new("touch")
            .args(["-t", "202001010000"])
            .arg(drops.join("ancient"))
            .status()
            .unwrap();
        assert!(old.success());
        let a = send_image(&host, b"1", "png", &new_id()).unwrap();
        let b = send_image(&host, b"2", "png", &new_id()).unwrap();
        assert_ne!(a, b);
        assert!(
            !drops.join("ancient").exists(),
            "the old folder was removed"
        );
        assert!(std::path::Path::new(&a).exists() && std::path::Path::new(&b).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_host_that_cannot_be_reached_says_what_it_could_not_send() {
        let mut host = RemoteHost::new("h");
        host.ssh = "/nonexistent/ssh".into();
        let dir = std::env::temp_dir().join(format!("hd-{}-down", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let local = dir.join("a.txt");
        std::fs::write(&local, "x").unwrap();
        let err = send_path(&host, &local, "1-0").unwrap_err();
        assert!(err.contains("a.txt"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
