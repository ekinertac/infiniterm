//! `ift push add | remove | list`: edits the push-devices file the phone
//! companion app fills over ssh. The file logic lives in
//! `infiniterm_core::push_notify`; this is argument parsing and file I/O.

use std::path::Path;
use std::process::ExitCode;

use infiniterm_core::push_notify::{
    add_device, devices_file_text, parse_devices_file, remove_device, valid_device_id, Device,
    DEVICES_FILE,
};

const ADD_USAGE: &str = "ift: push add needs an id and --server https://…";
const USAGE: &str = "ift: push takes add <id> --server <url>, remove <id>, or list";

#[derive(Debug, PartialEq, Eq)]
enum Action<'a> {
    Add { id: &'a str, url: &'a str },
    Remove { id: &'a str },
    List,
}

/// Err carries the usage line for stderr. Exit 2 either way.
fn parse(args: &[String]) -> Result<Action<'_>, &'static str> {
    match args.first().map(String::as_str) {
        Some("add") => {
            let mut id = None;
            let mut url = None;
            let mut rest = args[1..].iter().map(String::as_str);
            while let Some(a) = rest.next() {
                if a == "--server" && url.is_none() {
                    url = Some(rest.next().ok_or(ADD_USAGE)?);
                } else if id.is_none() && !a.starts_with("--") {
                    id = Some(a);
                } else {
                    return Err(ADD_USAGE);
                }
            }
            match (id, url) {
                (Some(id), Some(url)) if valid_device_id(id) && valid_url(url) => {
                    Ok(Action::Add { id, url })
                }
                _ => Err(ADD_USAGE),
            }
        }
        Some("remove") => match args {
            [_, id] if !id.is_empty() => Ok(Action::Remove { id }),
            _ => Err(USAGE),
        },
        Some("list") if args.len() == 1 => Ok(Action::List),
        _ => Err(USAGE),
    }
}

/// An https url with a host and no whitespace: the file is split on
/// whitespace, so a space would make a line the parser rejects. The host is
/// what follows `https://` up to the first `/`, `?` or `#`; it must not be
/// empty or start with `@` (a user-info part with nothing after it).
fn valid_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    !authority.is_empty()
        && !authority.starts_with('@')
        && !authority.ends_with('@')
        && !url.chars().any(char::is_whitespace)
}

/// Enough to tell devices apart in a list, never the whole id: the id is what
/// lets anyone push to the phone.
fn short(id: &str) -> String {
    format!("{}…", id.chars().take(6).collect::<String>())
}

pub fn run(args: &[String]) -> ExitCode {
    let action = match parse(args) {
        Ok(a) => a,
        Err(usage) => {
            eprintln!("{usage}");
            return ExitCode::from(2);
        }
    };
    let path = infiniterm_core::paths::config_dir().join(DEVICES_FILE);
    let devices = match read(&path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("ift: {e}");
            return ExitCode::from(1);
        }
    };
    match action {
        Action::List => {
            for d in &devices {
                println!("{} {}", d.url, short(&d.id));
            }
            ExitCode::SUCCESS
        }
        Action::Add { id, url } => {
            let device = Device {
                url: url.to_string(),
                id: id.to_string(),
            };
            match write(&path, &add_device(&devices, device)) {
                // The phone counts this line as success: keep `push: added `.
                Ok(()) => {
                    println!("push: added {}", short(id));
                    ExitCode::SUCCESS
                }
                Err(e) => fail(&path, &e),
            }
        }
        Action::Remove { id } => match remove(&path, &devices, id) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => fail(&path, &e),
        },
    }
}

fn fail(path: &Path, e: &std::io::Error) -> ExitCode {
    eprintln!("ift: cannot write {}: {e}", path.display());
    ExitCode::from(1)
}

/// A missing file is an empty list; an unreadable or invalid one is an error.
fn read(path: &Path) -> Result<Vec<Device>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse_devices_file(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("cannot read {}: {e}", path.display())),
    }
}

fn write(path: &Path, devices: &[Device]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // The ids are capabilities to push to a phone: owner only from the first
    // byte, even when the file already existed with wider permissions.
    infiniterm_core::files::write_private_atomically(path, &devices_file_text(devices))
}

/// Writes nothing, and makes no directory or file, when the id is not there.
fn remove(path: &Path, devices: &[Device], id: &str) -> std::io::Result<()> {
    if devices.iter().all(|d| d.id != id) {
        return Ok(());
    }
    write(path, &remove_device(devices, id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn add_needs_a_valid_id_and_an_https_server() {
        let ok = a(&["add", "abc_123-XYZ", "--server", "https://x.example"]);
        assert_eq!(
            parse(&ok),
            Ok(Action::Add {
                id: "abc_123-XYZ",
                url: "https://x.example"
            })
        );
        let flag_first = a(&["add", "--server", "https://x.example", "abc"]);
        assert!(parse(&flag_first).is_ok());
        for bad in [
            a(&["add"]),
            a(&["add", "abc"]),
            a(&["add", "--server", "https://x.example"]),
            a(&["add", "abc", "--server"]),
            a(&["add", "abc", "--server", "http://x.example"]),
            a(&["add", "abc", "--server", "https://"]),
            a(&["add", "abc", "--server", "https:///x"]),
            a(&["add", "abc", "--server", "https://@host.example"]),
            a(&["add", "abc", "--server", "https://@"]),
            a(&["add", "abc", "--server", "https://?q=1"]),
            a(&[
                "add",
                "abc",
                "--server",
                "https://a.example",
                "--server",
                "https://b.example",
            ]),
            a(&["add", "bad id", "--server", "https://x.example"]),
            a(&["add", "a/b", "--server", "https://x.example"]),
            a(&["add", "abc", "def", "--server", "https://x.example"]),
        ] {
            assert_eq!(parse(&bad), Err(ADD_USAGE), "{bad:?}");
        }
    }

    #[test]
    fn remove_and_list_take_exact_arguments() {
        assert_eq!(
            parse(&a(&["remove", "abc"])),
            Ok(Action::Remove { id: "abc" })
        );
        assert_eq!(parse(&a(&["list"])), Ok(Action::List));
        for bad in [
            a(&[]),
            a(&["remove"]),
            a(&["remove", "a", "b"]),
            a(&["list", "x"]),
            a(&["nope"]),
        ] {
            assert_eq!(parse(&bad), Err(USAGE), "{bad:?}");
        }
    }

    #[test]
    fn removing_an_absent_id_writes_nothing() {
        let dir = std::env::temp_dir().join(format!("infiniterm-pushcmd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("sub").join(DEVICES_FILE);
        remove(&path, &[], "abc").unwrap();
        assert!(!dir.exists(), "no directory or file made");

        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(DEVICES_FILE);
        let d = Device {
            url: "https://x.example".into(),
            id: "abc".into(),
        };
        remove(&path, std::slice::from_ref(&d), "other").unwrap();
        assert!(!path.exists());
        remove(&path, &[d], "abc").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn short_never_shows_the_whole_id() {
        assert_eq!(short("abcdefghij"), "abcdef…");
        assert_eq!(short("abc"), "abc…");
    }
}
