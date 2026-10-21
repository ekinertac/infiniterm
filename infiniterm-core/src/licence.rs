//! The commercial licence, registered on this Mac (#106).
//!
//! Licensing is the honour system: the app never needs the key, and nothing
//! changes for anyone who does not register. `ift licence <email> <key>`
//! asks Lemon Squeezy once whether the key is ours and the buyer's, and on a
//! yes writes `licence.json` beside the save file; the About window reads it
//! back to say who the copy is licensed to. Nothing ever re-checks it.
//!
//! This file is the pure half: `accept` judges a parsed response from
//! Lemon Squeezy's public validate endpoint, `read` / `write` own the file.
//! The network call is the cli's (`infiniterm-cli/src/licence_cmd.rs`, curl,
//! the way the updater and the omnibox suggestions fetch), and the About
//! window is `about_card` in `infiniterm-ui/src/overlays.rs`.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Lemon Squeezy's licence check. Public: it needs the key, not an API key.
pub const VALIDATE_URL: &str = "https://api.lemonsqueezy.com/v1/licenses/validate";
/// The infiniterm store; a key from any other store is someone else's product.
pub const STORE_ID: u64 = 487111;
/// The product's name on the store, the same in test and live mode (the
/// product ids differ between the two, the name does not).
pub const PRODUCT_NAME: &str = "infiniterm commercial licence";
/// Where an unregistered copy is pointed.
pub const BUY_PAGE: &str = "https://infiniterm.app/#pricing";

/// What a registered Mac keeps.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Licence {
    pub email: String,
    pub key: String,
    /// The buyer's name as the store has it.
    pub name: String,
    /// When `ift licence` checked it, RFC 3339.
    pub checked_at: String,
}

/// Why a key was refused; each says which check failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rejection {
    UnknownKey,
    Disabled,
    Expired,
    OtherProduct,
    EmailMismatch,
    TestMode,
}

impl Rejection {
    pub fn message(self) -> &'static str {
        match self {
            Rejection::UnknownKey => "Lemon Squeezy does not know this key",
            Rejection::Disabled => "this key has been disabled (refunded?)",
            Rejection::Expired => "this key has expired",
            Rejection::OtherProduct => "this key is for another product",
            Rejection::EmailMismatch => "this key was bought with another email address",
            Rejection::TestMode => "this is a test-mode key; buy at infiniterm.app",
        }
    }
}

/// Judges a validate response for `email`. `allow_test` lets a test-mode key
/// through, for a development build only, so a test purchase can exercise the
/// command without registering a release build. On a yes, the buyer's name.
pub fn accept(resp: &Value, email: &str, allow_test: bool) -> Result<String, Rejection> {
    let key = &resp["license_key"];
    let meta = &resp["meta"];
    // An unknown key comes back with `valid: false` and nulls for both.
    if key.is_null() || meta.is_null() {
        return Err(Rejection::UnknownKey);
    }
    match key["status"].as_str() {
        Some("disabled") => return Err(Rejection::Disabled),
        Some("expired") => return Err(Rejection::Expired),
        _ => {}
    }
    if resp["valid"].as_bool() != Some(true) {
        return Err(Rejection::UnknownKey);
    }
    if meta["store_id"].as_u64() != Some(STORE_ID)
        || meta["product_name"].as_str() != Some(PRODUCT_NAME)
    {
        return Err(Rejection::OtherProduct);
    }
    let bought_with = meta["customer_email"].as_str().unwrap_or("");
    if !bought_with.trim().eq_ignore_ascii_case(email.trim()) {
        return Err(Rejection::EmailMismatch);
    }
    if key["test_mode"].as_bool() == Some(true) && !allow_test {
        return Err(Rejection::TestMode);
    }
    Ok(meta["customer_name"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string())
}

/// `<data>/licence.json`, beside the save file, so a scratch instance has its own.
pub fn path() -> PathBuf {
    crate::paths::app_support_dir().join("licence.json")
}

/// The registration, or `None` when there is none or it does not parse: a
/// damaged file reads as "not registered", never as an error.
pub fn read(path: &Path) -> Option<Licence> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// Writes the file readable by its owner only (0600): it holds the key.
pub fn write(path: &Path, licence: &Licence) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    // Through a link, not over it (#219).
    let path = &crate::files::resolve_link(path);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string_pretty(licence).map_err(std::io::Error::other)?;
    // Written to a temporary beside it and renamed, so a crash mid-write
    // leaves the old registration rather than half a file.
    let tmp = path.with_extension("json.tmp");
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)?;
    // `mode` applies only when the file is created; a leftover keeps its own.
    f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    f.write_all(text.as_bytes())?;
    f.write_all(b"\n")?;
    drop(f);
    std::fs::rename(&tmp, path)
}

/// The time to stamp a registration with.
pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// What `ift licence` and the About window say about a registration.
pub fn licensed_to(l: &Licence) -> String {
    if l.name.is_empty() {
        format!("Licensed to {}", l.email)
    } else {
        format!("Licensed to {} ({})", l.name, l.email)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // The shape of a real response for a live key, trimmed to what is read.
    fn good() -> Value {
        json!({
            "valid": true,
            "error": null,
            "license_key": {"status": "active", "test_mode": false},
            "meta": {
                "store_id": STORE_ID,
                "product_name": PRODUCT_NAME,
                "customer_name": "Ada Lovelace",
                "customer_email": "ada@example.com"
            }
        })
    }

    // A licence.json kept in a dotfiles repo stays a link, and its target
    // stays private (#219).
    #[test]
    fn writing_through_a_symlink_keeps_the_link_and_the_mode() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let root = std::env::temp_dir().join(format!("infiniterm-lic-link-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let real = root.join("real.json");
        let link = root.join("licence.json");
        std::fs::write(&real, "old").unwrap();
        symlink(&real, &link).unwrap();
        let l = Licence {
            email: "a@example.com".into(),
            key: "K".into(),
            name: "A".into(),
            checked_at: "2026-10-03T00:00:00Z".into(),
        };
        write(&link, &l).unwrap();
        assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());
        assert_eq!(read(&real), Some(l));
        assert_eq!(
            std::fs::metadata(&real).unwrap().permissions().mode() & 0o777,
            0o600
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_good_key_gives_the_buyers_name() {
        assert_eq!(
            accept(&good(), "ada@example.com", false),
            Ok("Ada Lovelace".into())
        );
    }

    #[test]
    fn the_email_is_compared_without_case_or_spaces() {
        assert!(accept(&good(), " ADA@Example.com ", false).is_ok());
        assert_eq!(
            accept(&good(), "bob@example.com", false),
            Err(Rejection::EmailMismatch)
        );
    }

    #[test]
    fn an_unknown_key_is_refused() {
        let r = json!({"valid": false, "error": "license_key not found", "license_key": null, "meta": null});
        assert_eq!(
            accept(&r, "ada@example.com", false),
            Err(Rejection::UnknownKey)
        );
        assert_eq!(
            accept(&json!({}), "ada@example.com", false),
            Err(Rejection::UnknownKey)
        );
    }

    #[test]
    fn disabled_and_expired_say_so() {
        let mut r = good();
        r["valid"] = json!(false);
        r["license_key"]["status"] = json!("disabled");
        assert_eq!(
            accept(&r, "ada@example.com", false),
            Err(Rejection::Disabled)
        );
        r["license_key"]["status"] = json!("expired");
        assert_eq!(
            accept(&r, "ada@example.com", false),
            Err(Rejection::Expired)
        );
    }

    #[test]
    fn another_store_or_product_is_refused() {
        let mut r = good();
        r["meta"]["store_id"] = json!(1);
        assert_eq!(
            accept(&r, "ada@example.com", false),
            Err(Rejection::OtherProduct)
        );
        let mut r = good();
        r["meta"]["product_name"] = json!("Something else");
        assert_eq!(
            accept(&r, "ada@example.com", false),
            Err(Rejection::OtherProduct)
        );
    }

    #[test]
    fn a_test_key_registers_only_a_development_build() {
        let mut r = good();
        r["license_key"]["test_mode"] = json!(true);
        assert_eq!(
            accept(&r, "ada@example.com", false),
            Err(Rejection::TestMode)
        );
        assert!(accept(&r, "ada@example.com", true).is_ok());
    }

    #[test]
    fn the_file_round_trips_owner_only_and_a_corrupt_one_is_not_registered() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("ift-licence-{}", uuid::Uuid::new_v4()));
        let p = dir.join("licence.json");
        assert_eq!(read(&p), None);
        let l = Licence {
            email: "ada@example.com".into(),
            key: "K".into(),
            name: "Ada Lovelace".into(),
            checked_at: "2026-10-03T00:00:00Z".into(),
        };
        write(&p, &l).unwrap();
        assert_eq!(read(&p), Some(l.clone()));
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        std::fs::write(&p, "{not json").unwrap();
        assert_eq!(read(&p), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn licensed_to_names_the_buyer() {
        let mut l = Licence {
            email: "ada@example.com".into(),
            key: "K".into(),
            name: "Ada Lovelace".into(),
            checked_at: String::new(),
        };
        assert_eq!(
            licensed_to(&l),
            "Licensed to Ada Lovelace (ada@example.com)"
        );
        l.name.clear();
        assert_eq!(licensed_to(&l), "Licensed to ada@example.com");
    }
}
