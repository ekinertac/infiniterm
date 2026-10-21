//! An Ed25519 signature over what an update is, for the platform that has no
//! code signing of its own.
//!
//! Responsibility: make, check and name the signature that goes in a release
//! manifest (`latest-windows.json`), and nothing else. The macOS updater does
//! not use it: a Mac download must be signed by Ekin's Developer ID and
//! notarized, which is a stronger check. Windows has no Authenticode
//! certificate (a cost Ekin declined, 2026-09-25), so until now the manifest's
//! SHA-256 was the only check, and whoever could write the releases repo
//! decided what ran. This adds a second factor that costs no money and needs
//! one private key kept off the repo.
//!
//! Where it fits: `update.rs` holds the public key constant and the rule that
//! applies it (`check_manifest_signature`); `infiniterm-ui/src/updater.rs`
//! calls that before it downloads anything; `ift update-key new` and
//! `ift update-sign` (infiniterm-cli) call `generate` and `sign`, and
//! `tools/dist.ps1` runs the second.
//!
//! What is signed is the TAG, the BUILD and the SHA-256, not the file: a
//! signature over only the hash could be replayed on an older release's hash
//! and file (a downgrade), and a signature over the tag and build could not
//! be replayed on another file. `signed_message` is the one spelling of it.
//!
//! Keys and signatures are hex: a hand-written hex codec is ten lines and
//! needs no base64 crate. The private key is a PKCS#8 document, as `ring`
//! reads it, written in hex to a file the person keeps; the public key is the
//! 32 bytes that go in `update::UPDATE_PUBLIC_KEY`.
//!
//! Constraint: `ring` is already in the dependency tree (the http stack uses
//! it), so this adds an edge, not a new crate.

use ring::rand::SystemRandom;
use ring::signature::{Ed25519KeyPair, KeyPair, UnparsedPublicKey, ED25519};

/// The text a release signature covers. A version prefix first, so a future
/// change of what is signed cannot be confused with this one.
pub fn signed_message(tag: &str, build: u64, sha256: &str) -> Vec<u8> {
    // Lowercase: `certutil` and `shasum` spell a digest differently, and a
    // manifest field is written by whichever tool made it.
    format!(
        "infiniterm-update-v1\n{tag}\n{build}\n{}",
        sha256.to_ascii_lowercase()
    )
    .into_bytes()
}

/// A new key pair: the private key as hex of its PKCS#8 document (to keep),
/// and the public key as hex of its 32 bytes (to commit).
pub fn generate() -> Result<(String, String), String> {
    let rng = SystemRandom::new();
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng).map_err(|e| e.to_string())?;
    let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).map_err(|e| e.to_string())?;
    Ok((to_hex(pkcs8.as_ref()), to_hex(pair.public_key().as_ref())))
}

/// The public key for a private key, so a key file can be checked against
/// the constant in `update.rs` without signing anything.
pub fn public_key_of(private_hex: &str) -> Result<String, String> {
    let pair = key_pair(private_hex)?;
    Ok(to_hex(pair.public_key().as_ref()))
}

/// The signature of `message` under a private key, as hex.
pub fn sign(private_hex: &str, message: &[u8]) -> Result<String, String> {
    let pair = key_pair(private_hex)?;
    Ok(to_hex(pair.sign(message).as_ref()))
}

/// Whether `signature_hex` is `public_hex`'s signature of `message`.
pub fn verify(public_hex: &str, message: &[u8], signature_hex: &str) -> Result<(), String> {
    let public = from_hex(public_hex).map_err(|e| format!("the update key: {e}"))?;
    let signature = from_hex(signature_hex).map_err(|e| format!("the signature: {e}"))?;
    UnparsedPublicKey::new(&ED25519, public)
        .verify(message, &signature)
        .map_err(|_| "the signature does not match".to_string())
}

fn key_pair(private_hex: &str) -> Result<Ed25519KeyPair, String> {
    let bytes = from_hex(private_hex.trim()).map_err(|e| format!("the private key: {e}"))?;
    Ed25519KeyPair::from_pkcs8(&bytes).map_err(|_| "the private key is not an Ed25519 key".into())
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn from_hex(text: &str) -> Result<Vec<u8>, String> {
    let text = text.trim();
    if !text.len().is_multiple_of(2) {
        return Err("hex has an odd number of digits".into());
    }
    (0..text.len())
        .step_by(2)
        .map(|i| {
            text.get(i..i + 2)
                .and_then(|pair| u8::from_str_radix(pair, 16).ok())
                .ok_or_else(|| "not hex".to_string())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff";

    fn signed(tag: &str, build: u64, sha: &str) -> (String, String) {
        let (private, public) = generate().unwrap();
        let sig = sign(&private, &signed_message(tag, build, sha)).unwrap();
        (public, sig)
    }

    #[test]
    fn a_key_pair_signs_and_its_public_half_verifies() {
        let (private, public) = generate().unwrap();
        assert_eq!(public.len(), 64, "32 bytes in hex");
        assert_eq!(public_key_of(&private).unwrap(), public);
        let m = signed_message("v0.7.0-1100", 1100, SHA);
        let sig = sign(&private, &m).unwrap();
        assert_eq!(sig.len(), 128, "64 bytes in hex");
        assert!(verify(&public, &m, &sig).is_ok());
    }

    // The point of signing the tag and build with the hash: a signature for
    // one release is no good for another.
    #[test]
    fn a_signature_does_not_carry_to_another_tag_build_or_hash() {
        let (public, sig) = signed("v0.7.0-1100", 1100, SHA);
        let other_hash = SHA.replace("00ff", "ff00");
        for (tag, build, sha) in [
            ("v0.6.9-1000", 1100, SHA.to_string()),
            ("v0.7.0-1100", 1000, SHA.to_string()),
            ("v0.7.0-1100", 1100, other_hash),
        ] {
            let m = signed_message(tag, build, &sha);
            assert!(verify(&public, &m, &sig).is_err(), "{tag} {build} {sha}");
        }
    }

    #[test]
    fn another_key_does_not_verify() {
        let (_, sig) = signed("v1", 1, SHA);
        let (_, other_public) = generate().unwrap();
        assert!(verify(&other_public, &signed_message("v1", 1, SHA), &sig).is_err());
    }

    // Written by `certutil` in lowercase and by some tools in capitals: the
    // same digest must verify either way.
    #[test]
    fn a_digest_in_capitals_is_the_same_digest() {
        let (public, sig) = signed("v1", 1, SHA);
        let m = signed_message("v1", 1, &SHA.to_ascii_uppercase());
        assert!(verify(&public, &m, &sig).is_ok());
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        let m = signed_message("v1", 1, SHA);
        let (public, sig) = signed("v1", 1, SHA);
        assert!(verify("zz", &m, &sig).is_err());
        assert!(verify(&public, &m, "abc").is_err());
        assert!(verify(&public, &m, "").is_err());
        assert!(verify("", &m, &sig).is_err());
        assert!(sign("nothex", &m).is_err());
        assert!(sign("abcd", &m).is_err(), "hex, but not a key");
        assert!(public_key_of("").is_err());
    }

    #[test]
    fn the_message_has_the_version_and_the_four_parts_in_order() {
        assert_eq!(
            String::from_utf8(signed_message("t", 7, "AB")).unwrap(),
            "infiniterm-update-v1\nt\n7\nab"
        );
    }
}
