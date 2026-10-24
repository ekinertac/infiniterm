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

/// Where on the host the drops go, relative to its `$HOME`.
pub const DROPS_DIR: &str = ".infiniterm/drops";

/// How long a drop folder is kept, in days.
pub const KEEP_DAYS: u32 = 7;

/// A name safe to put in one path component: the last part of the path, with
/// a slash or a control character turned into `_`, never empty or a dot name.
pub fn host_name(name: &str) -> String {
    let base = name.rsplit('/').next().unwrap_or(name);
    let cleaned: String = base
        .chars()
        .map(|c| if c == '/' || c.is_control() { '_' } else { c })
        .collect();
    match cleaned.as_str() {
        "" | "." | ".." => "file".to_string(),
        _ => cleaned,
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
