//! `ift update-key` and `ift update-sign`: the two ends of the Windows
//! release signature (#470), in `ift` so `tools/dist.ps1` needs no other tool.
//!
//! `ift update-key new <file>` makes a key pair. The PRIVATE key (hex of a
//! PKCS#8 document) goes to the file, which must not exist yet, with owner-only
//! permissions on unix; the PUBLIC key (64 hex digits) is printed on stdout,
//! for `UPDATE_PUBLIC_KEY` in `infiniterm-core/src/update.rs`. `ift
//! update-key public <file>` prints the public key of a key file, so a file
//! can be checked against the constant without signing anything.
//!
//! `ift update-sign <manifest.json> --key <file>` signs a manifest's tag,
//! build and sha256 (`update_sign::signed_message`) and writes the result
//! into the manifest as `signature`, replacing an old one. The key file can
//! also be named by `INFINITERM_UPDATE_KEY`, which is how CI or a release
//! shell would hand it over without it appearing in a process list.
//!
//! This tool never keeps a key: the private half lives only in the file the
//! person chose. Exit codes: 0 ok, 1 failed (said on stderr), 2 bad usage.
//!
//! Related: infiniterm-core/src/update_sign.rs (the cryptography),
//! update.rs (the constant and the rule), tools/dist.ps1 (calls update-sign),
//! infiniterm-ui/src/updater.rs (checks it).
use infiniterm_core::update_sign;
use std::path::Path;
use std::process::ExitCode;

pub const KEY_ENV: &str = "INFINITERM_UPDATE_KEY";

pub fn key(args: &[String]) -> ExitCode {
    match args {
        [cmd, file] if cmd == "new" => report(new_key(Path::new(file))),
        [cmd, file] if cmd == "public" => report(public_of(Path::new(file))),
        _ => usage("ift update-key new <file> | ift update-key public <file>"),
    }
}

pub fn sign(args: &[String]) -> ExitCode {
    let mut manifest = None;
    let mut key_file = std::env::var(KEY_ENV).ok();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--key" => match it.next() {
                Some(f) => key_file = Some(f.clone()),
                None => return usage("--key takes a file"),
            },
            f if manifest.is_none() && !f.starts_with("--") => manifest = Some(f.to_string()),
            _ => return usage("ift update-sign <manifest.json> --key <file>"),
        }
    }
    let (Some(manifest), Some(key_file)) = (manifest, key_file) else {
        return usage(&format!(
            "ift update-sign <manifest.json> --key <file>  (or set {KEY_ENV})"
        ));
    };
    report(sign_manifest(Path::new(&manifest), Path::new(&key_file)))
}

fn usage(text: &str) -> ExitCode {
    eprintln!("usage: {text}");
    ExitCode::from(2)
}

fn report(result: Result<String, String>) -> ExitCode {
    match result {
        Ok(line) => {
            println!("{line}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("ift: {e}");
            ExitCode::from(1)
        }
    }
}

/// Writes a new private key to `file` and returns the public key.
fn new_key(file: &Path) -> Result<String, String> {
    let (private, public) = update_sign::generate()?;
    write_private(file, &private)?;
    eprintln!(
        "ift: private key written to {}. Keep it OFF the repo, and back it up: \
         a release cannot be signed without it. Put this public key in \
         UPDATE_PUBLIC_KEY (infiniterm-core/src/update.rs):",
        file.display()
    );
    Ok(public)
}

fn public_of(file: &Path) -> Result<String, String> {
    update_sign::public_key_of(&read_key(file)?)
}

/// Owner-only, and never over an existing file: a key replaced by accident
/// is every earlier release's signature lost.
fn write_private(file: &Path, private: &str) -> Result<(), String> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut f = options.open(file).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            format!("{} exists; a key is never overwritten", file.display())
        } else {
            format!("cannot create {}: {e}", file.display())
        }
    })?;
    writeln!(f, "{private}").map_err(|e| format!("cannot write {}: {e}", file.display()))
}

fn read_key(file: &Path) -> Result<String, String> {
    std::fs::read_to_string(file)
        .map(|t| t.trim().to_string())
        .map_err(|e| format!("cannot read the key {}: {e}", file.display()))
}

/// Signs the manifest in place; returns a line saying what was signed.
fn sign_manifest(manifest: &Path, key_file: &Path) -> Result<String, String> {
    let text = std::fs::read_to_string(manifest)
        .map_err(|e| format!("cannot read {}: {e}", manifest.display()))?;
    let mut value: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("{} is not JSON: {e}", manifest.display()))?;
    let field = |k: &str| value.get(k).and_then(|x| x.as_str()).map(String::from);
    let tag = field("tag").ok_or("the manifest has no tag")?;
    let sha256 = field("sha256").ok_or("the manifest has no sha256")?;
    let build = value
        .get("build")
        .and_then(|b| b.as_u64())
        .ok_or("the manifest has no build number")?;
    let signature = update_sign::sign(
        &read_key(key_file)?,
        &update_sign::signed_message(&tag, build, &sha256),
    )?;
    value["signature"] = serde_json::Value::String(signature);
    let out = serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?;
    infiniterm_core::files::write_atomically(manifest, &format!("{out}\n"))
        .map_err(|e| format!("cannot write {}: {e}", manifest.display()))?;
    Ok(format!("signed {tag} (build {build})"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("ift-usign-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const MANIFEST: &str = r#"{"version":"0.7.0","build":1100,"tag":"v0.7.0-1100","url":"https://github.com/ekinertac/infiniterm/releases/download/v0.7.0-1100/x.zip","sha256":"00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff"}"#;

    #[test]
    fn a_new_key_is_written_once_and_its_public_half_is_printed() {
        let dir = scratch("new");
        let file = dir.join("update.key");
        let public = new_key(&file).unwrap();
        assert_eq!(public.len(), 64);
        assert_eq!(public_of(&file).unwrap(), public);
        // Never over an existing key.
        let err = new_key(&file).unwrap_err();
        assert!(err.contains("never overwritten"), "{err}");
        assert_eq!(public_of(&file).unwrap(), public, "the first key survived");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    // The whole loop: a key made here signs a manifest, and the app's own rule
    // (`check_manifest_signature`) accepts it with that key and refuses it
    // with another.
    #[test]
    fn a_signed_manifest_passes_the_apps_check_and_only_with_its_key() {
        let dir = scratch("sign");
        let key = dir.join("update.key");
        let public = new_key(&key).unwrap();
        let manifest = dir.join("latest-windows.json");
        std::fs::write(&manifest, MANIFEST).unwrap();
        let said = sign_manifest(&manifest, &key).unwrap();
        assert!(said.contains("v0.7.0-1100"), "{said}");

        let text = std::fs::read_to_string(&manifest).unwrap();
        let parsed = infiniterm_core::update::parse_manifest(&text).unwrap();
        assert!(parsed.signature.is_some());
        assert!(infiniterm_core::update::check_manifest_signature(&public, &parsed).is_ok());
        let other = dir.join("other.key");
        let other_public = new_key(&other).unwrap();
        assert!(infiniterm_core::update::check_manifest_signature(&other_public, &parsed).is_err());
        // The other fields are as they were.
        assert_eq!(parsed.build, 1100);
        assert_eq!(parsed.tag, "v0.7.0-1100");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn signing_again_replaces_the_signature_and_a_changed_manifest_no_longer_verifies() {
        let dir = scratch("again");
        let key = dir.join("update.key");
        let public = new_key(&key).unwrap();
        let manifest = dir.join("m.json");
        std::fs::write(&manifest, MANIFEST).unwrap();
        sign_manifest(&manifest, &key).unwrap();
        let first = std::fs::read_to_string(&manifest).unwrap();
        sign_manifest(&manifest, &key).unwrap();
        assert_eq!(std::fs::read_to_string(&manifest).unwrap(), first, "ed25519 is deterministic");
        // Somebody edits the hash after signing.
        let edited = first.replace("00ff", "ff00");
        let parsed = infiniterm_core::update::parse_manifest(&edited).unwrap();
        assert!(infiniterm_core::update::check_manifest_signature(&public, &parsed).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_manifest_missing_what_is_signed_is_refused() {
        let dir = scratch("bad");
        let key = dir.join("update.key");
        new_key(&key).unwrap();
        for (name, body) in [
            ("no-tag", r#"{"build":1,"sha256":"ab"}"#),
            ("no-sha", r#"{"build":1,"tag":"t"}"#),
            ("no-build", r#"{"tag":"t","sha256":"ab"}"#),
            ("not-json", "nope"),
        ] {
            let m = dir.join(format!("{name}.json"));
            std::fs::write(&m, body).unwrap();
            assert!(sign_manifest(&m, &key).is_err(), "{name}");
            assert_eq!(std::fs::read_to_string(&m).unwrap(), body, "{name} untouched");
        }
        assert!(sign_manifest(&dir.join("missing.json"), &key).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_or_broken_key_file_is_an_error() {
        let dir = scratch("nokey");
        let m = dir.join("m.json");
        std::fs::write(&m, MANIFEST).unwrap();
        assert!(sign_manifest(&m, &dir.join("absent.key")).is_err());
        let junk = dir.join("junk.key");
        std::fs::write(&junk, "not a key").unwrap();
        assert!(sign_manifest(&m, &junk).is_err());
        assert!(public_of(&junk).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
