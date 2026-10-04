//! `ift connect <host> --install`: put `ift`, `iftd` and the hook on a host
//! over ssh (#159, stage A of #118).
//!
//! The Mac does the work: it finds the host's CPU (`uname -sm`), takes the
//! package that matches THIS version from the GitHub release
//! (`infiniterm-server-v<version>-linux-<arch>.tar.gz`, built by
//! `.github/workflows/linux.yml`, with its `.sha256`), checks the checksum
//! here, and streams the package into `tar` on the host through ssh. The host
//! needs nothing but `sh`, `tar` and a way in: no internet, no `curl`, no Rust.
//! A host that is a Mac has the binaries inside infiniterm.app already, so the
//! app there is used and nothing is copied.
//!
//! Where the files go: `/usr/local/bin` for root (on the PATH of a
//! non-interactive ssh), `~/.local/bin` for anyone else (often NOT on that
//! PATH, so the caller connects by the full path this returns).
//!
//! Called by `connect.rs`. Related: `infiniterm-core/src/backend/remote.rs`
//! (`run_on`, `run_on_stdin`), `.github/workflows/linux.yml` (the packages).
//!
//! Non-obvious constraints:
//! - The package is checked BEFORE anything is sent; a mismatch installs
//!   nothing.
//! - `tar` runs with `--no-same-owner`: as root it would otherwise give the
//!   files the id of whoever packed them (a CI runner's), seen on the first
//!   real install.
//! - `INFINITERM_RELEASE_BASE` replaces the download address (a folder, or a
//!   `file://` one) for tests; nobody else sets it.
//! - Releases carry these packages from the first one built after #147, so
//!   `--from <package>` installs from a file before that and offline.

use std::path::{Path, PathBuf};
use std::process::Command;

use infiniterm_core::backend::remote::{self, RemoteHost};
use infiniterm_core::drop::shell_quote;

/// Where the packages live: the release for a tag.
const RELEASE_BASE: &str = "https://github.com/ekinertac/infiniterm/releases/download";
/// A Mac host keeps the binaries here when infiniterm is installed.
pub const MAC_IFT: &str = "/Applications/infiniterm.app/Contents/MacOS/ift";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Linux,
    Mac,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Platform {
    pub os: Os,
    /// `x86_64` or `aarch64`, the names the package uses.
    pub arch: &'static str,
}

fn arch_of(word: &str) -> Option<&'static str> {
    match word {
        "x86_64" | "amd64" => Some("x86_64"),
        "aarch64" | "arm64" => Some("aarch64"),
        _ => None,
    }
}

/// What `uname -sm` printed: `Linux x86_64`, `Darwin arm64`, ...
pub fn parse_uname(out: &str) -> Option<Platform> {
    let mut words = out.split_whitespace();
    let os = match words.next()? {
        "Linux" => Os::Linux,
        "Darwin" => Os::Mac,
        _ => return None,
    };
    Some(Platform {
        os,
        arch: arch_of(words.next()?)?,
    })
}

/// `--platform linux-x86_64` or `linux-aarch64` (a host `uname` cannot be
/// trusted on, or a test).
pub fn parse_platform_flag(s: &str) -> Option<Platform> {
    let arch = s.strip_prefix("linux-")?;
    Some(Platform {
        os: Os::Linux,
        arch: arch_of(arch)?,
    })
}

pub fn asset_name(tag: &str, arch: &str) -> String {
    format!("infiniterm-server-{tag}-linux-{arch}.tar.gz")
}

/// The folder the package and its checksum are fetched from.
pub fn release_base(tag: &str) -> String {
    std::env::var("INFINITERM_RELEASE_BASE").unwrap_or_else(|_| format!("{RELEASE_BASE}/{tag}"))
}

/// The first word of a `.sha256` file (`<hex>  <name>`).
pub fn parse_sha256_file(text: &str) -> Option<String> {
    let w = text.split_whitespace().next()?.to_ascii_lowercase();
    (w.len() == 64 && w.chars().all(|c| c.is_ascii_hexdigit())).then_some(w)
}

/// SHA-256 of a file, with the `shasum` every Mac has.
pub fn sha256_of(path: &Path) -> std::io::Result<String> {
    let out = Command::new("shasum")
        .args(["-a", "256"])
        .arg(path)
        .output()?;
    if !out.status.success() {
        return Err(std::io::Error::other("shasum failed"));
    }
    parse_sha256_file(&String::from_utf8_lossy(&out.stdout))
        .ok_or_else(|| std::io::Error::other("shasum printed no checksum"))
}

/// The script the host runs with the package on its stdin. It prints the
/// folder it used on its last line.
pub fn install_script() -> String {
    concat!(
        "if [ \"$(id -u)\" = 0 ]; then d=/usr/local/bin; else d=\"$HOME/.local/bin\"; fi; ",
        "mkdir -p \"$d\" && tar xzf - -C \"$d\" --no-same-owner && ",
        "chmod +x \"$d/ift\" \"$d/iftd\" \"$d/infiniterm-hook\" && echo \"$d\""
    )
    .to_string()
}

/// `ift` and `iftd` on the host, or why not. Returns the full path of the
/// host's `ift`, which the caller connects by.
pub fn install(
    host: &RemoteHost,
    platform_flag: Option<&str>,
    from: Option<&Path>,
    version: &str,
) -> Result<String, String> {
    let platform = match platform_flag {
        Some(f) => parse_platform_flag(f)
            .ok_or_else(|| format!("not a platform: {f} (linux-x86_64 or linux-aarch64)"))?,
        None => {
            let out = remote::run_on(host, "uname -sm")
                .map_err(|e| format!("could not run {}: {e}", host.ssh))?;
            if !out.status.success() {
                return Err(format!(
                    "{}: could not ask the host what it is: {}",
                    host.target,
                    String::from_utf8_lossy(&out.stderr).trim()
                ));
            }
            let said = String::from_utf8_lossy(&out.stdout).trim().to_string();
            parse_uname(&said).ok_or_else(|| {
                format!("{}: infiniterm has no server build for \"{said}\" (Linux on x86_64 or aarch64 only)", host.target)
            })?
        }
    };
    if platform.os == Os::Mac {
        // The app carries its own copies; there is nothing to send.
        let probe = remote::run_on(host, &format!("[ -x {} ]", shell_quote(MAC_IFT)))
            .map_err(|e| format!("could not run {}: {e}", host.ssh))?;
        return if probe.status.success() {
            Ok(MAC_IFT.to_string())
        } else {
            Err(format!(
                "{}: this host is a Mac without infiniterm; install the app there",
                host.target
            ))
        };
    }

    let tag = format!("v{version}");
    let name = asset_name(&tag, platform.arch);
    let scratch = std::env::temp_dir().join(format!("ift-install-{}", std::process::id()));
    let result = (|| -> Result<String, String> {
        let package: PathBuf = match from {
            Some(p) => p.to_path_buf(),
            None => download(&release_base(&tag), &name, &scratch)?,
        };
        let input = std::fs::File::open(&package)
            .map_err(|e| format!("could not read {}: {e}", package.display()))?;
        let out = remote::run_on_stdin(host, &format!("sh -c {}", shell_quote(&install_script())), input)
            .map_err(|e| format!("could not run {}: {e}", host.ssh))?;
        if !out.status.success() {
            return Err(format!(
                "{}: the install failed: {}",
                host.target,
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        let dir = String::from_utf8_lossy(&out.stdout)
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .map(|l| l.trim().to_string())
            .ok_or_else(|| format!("{}: the install said nothing", host.target))?;
        Ok(format!("{dir}/ift"))
    })();
    let _ = std::fs::remove_dir_all(&scratch);
    result
}

/// Fetches `<base>/<name>` and `<name>.sha256` into `dir` and checks the first
/// against the second. Nothing is sent anywhere unless they agree.
fn download(base: &str, name: &str, dir: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let fetch = |file: &str, to: &Path| -> Result<(), String> {
        let url = format!("{base}/{file}");
        let status = Command::new("curl")
            .args(["-fsSL", "-o"])
            .arg(to)
            .arg(&url)
            .status()
            .map_err(|e| format!("could not run curl: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!(
                "could not download {url}; releases carry the server packages from the first one built after the Linux workflow landed, and `--from <package>` installs from a file"
            ))
        }
    };
    let package = dir.join(name);
    let sum = dir.join(format!("{name}.sha256"));
    fetch(name, &package)?;
    fetch(&format!("{name}.sha256"), &sum)?;
    let expected = parse_sha256_file(&std::fs::read_to_string(&sum).unwrap_or_default())
        .ok_or_else(|| format!("{name}.sha256 holds no checksum"))?;
    let actual = sha256_of(&package).map_err(|e| format!("could not check {name}: {e}"))?;
    if actual != expected {
        return Err(format!(
            "{name} does not match its checksum (expected {expected}, got {actual}); nothing was installed"
        ));
    }
    Ok(package)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hosts_cpu_is_read_from_uname() {
        let p = parse_uname("Linux x86_64\n").unwrap();
        assert_eq!((p.os, p.arch), (Os::Linux, "x86_64"));
        assert_eq!(parse_uname("Linux aarch64").unwrap().arch, "aarch64");
        assert_eq!(parse_uname("Linux arm64").unwrap().arch, "aarch64");
        assert_eq!(parse_uname("Linux amd64").unwrap().arch, "x86_64");
        let m = parse_uname("Darwin arm64").unwrap();
        assert_eq!((m.os, m.arch), (Os::Mac, "aarch64"));
        assert!(parse_uname("FreeBSD amd64").is_none());
        assert!(parse_uname("Linux riscv64").is_none());
        assert!(parse_uname("").is_none());
    }

    #[test]
    fn a_platform_flag_names_a_linux_arch() {
        assert_eq!(parse_platform_flag("linux-x86_64").unwrap().arch, "x86_64");
        assert_eq!(parse_platform_flag("linux-aarch64").unwrap().arch, "aarch64");
        assert!(parse_platform_flag("darwin-arm64").is_none());
        assert!(parse_platform_flag("linux-mips").is_none());
    }

    #[test]
    fn the_package_name_is_the_one_the_workflow_makes() {
        assert_eq!(
            asset_name("v0.5.3", "x86_64"),
            "infiniterm-server-v0.5.3-linux-x86_64.tar.gz"
        );
    }

    #[test]
    fn a_checksum_file_gives_its_first_word_if_it_is_a_checksum() {
        let sum = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert_eq!(parse_sha256_file(&format!("{sum}  pkg.tar.gz\n")).as_deref(), Some(sum));
        assert_eq!(
            parse_sha256_file(&format!("{}  x", sum.to_uppercase())).as_deref(),
            Some(sum),
            "case does not matter"
        );
        assert_eq!(parse_sha256_file("nothing here"), None);
        assert_eq!(parse_sha256_file(""), None);
    }

    #[test]
    fn a_file_is_checked_with_shasum() {
        let p = std::env::temp_dir().join(format!("ift-sha-{}", std::process::id()));
        std::fs::write(&p, b"abc").unwrap();
        assert_eq!(
            sha256_of(&p).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn the_host_script_picks_a_folder_by_user_and_does_not_keep_the_packers_owner() {
        let s = install_script();
        assert!(s.contains("/usr/local/bin") && s.contains("$HOME/.local/bin"));
        assert!(s.contains("--no-same-owner"));
        assert!(s.contains("tar xzf -"));
        assert!(s.ends_with("echo \"$d\""), "the folder is the last thing it says");
    }
}
