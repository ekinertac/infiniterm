//! macOS's spell checker for the editor (#850): `NSSpellChecker`, so the
//! system's dictionaries and languages and the words the user taught any
//! Mac app are ours too.
//!
//! Called by `editor_body.rs`: its idle step asks `misspelled` one prose line
//! at a time (`infiniterm_editor::spell` decides what is prose and keeps the
//! answers), the right-click menu and the palette ask `guesses`, and Learn
//! and Ignore go through `learn` and `ignore`. Related: `not_macos.rs` has
//! the same names answering nothing, so spell checking is off elsewhere.
//!
//! Constraints: AppKit's checker belongs to the main thread, which is where
//! the editor's idle step and its key handling run. Ignore is for this run
//! of the app, every editor at once (one document tag for the whole app);
//! Learn writes the user's own dictionary, the one every Mac app shares.
//! Either changes what is misspelled everywhere, so `generation` moves and
//! every editor drops what it knew.

use objc::runtime::{Object, NO};
use objc::{class, msg_send, sel, sel_impl};
use std::ffi::{CStr, CString};
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

type Id = *mut Object;

/// Spell checking exists on this platform.
pub const AVAILABLE: bool = true;

/// `NSNotFound`: `NSIntegerMax`.
const NS_NOT_FOUND: usize = isize::MAX as usize;

#[repr(C)]
#[derive(Clone, Copy)]
struct NsRange {
    location: usize,
    length: usize,
}

static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Moves on every Learn and Ignore; see the header.
pub fn generation() -> u64 {
    GENERATION.load(Ordering::SeqCst)
}

fn checker() -> Id {
    unsafe { msg_send![class!(NSSpellChecker), sharedSpellChecker] }
}

/// The app's one spell document: what Ignore is remembered against.
fn tag() -> isize {
    static TAG: OnceLock<isize> = OnceLock::new();
    *TAG.get_or_init(|| unsafe { msg_send![class!(NSSpellChecker), uniqueSpellDocumentTag] })
}

fn ns_string(s: &str) -> Id {
    let c = CString::new(s.replace('\0', "")).unwrap_or_default();
    unsafe { msg_send![class!(NSString), stringWithUTF8String: c.as_ptr()] }
}

fn rust_string(s: Id) -> Option<String> {
    if s.is_null() {
        return None;
    }
    let p: *const std::os::raw::c_char = unsafe { msg_send![s, UTF8String] };
    (!p.is_null()).then(|| unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
}

/// The language to check `text` in: its dominant language as the user's
/// own variant of it (`en` read as `en_GB` when that is in the list), or
/// `None` to let the checker pick. Asked once per text, not per line: a
/// language guessed from one short line is a coin toss, and with the
/// checker's automatic mode a word passes when ANY of the user's languages
/// knows it, so a Mac set up with many lets most typos through.
pub fn language_of(text: &str) -> Option<String> {
    let found: Id = unsafe {
        msg_send![class!(NSLinguisticTagger), dominantLanguageForString: ns_string(text)]
    };
    let code = rust_string(found).filter(|c| c != "und")?;
    let fits = |l: &str| l == code || l.starts_with(&format!("{code}_"));
    let checker = checker();
    for list in [
        unsafe { msg_send![checker, userPreferredLanguages] },
        unsafe { msg_send![checker, availableLanguages] },
    ] {
        let list: Id = list;
        if list.is_null() {
            continue;
        }
        let n: usize = unsafe { msg_send![list, count] };
        let pick = (0..n)
            .filter_map(|i| rust_string(unsafe { msg_send![list, objectAtIndex: i] }))
            .find(|l| fits(l));
        if pick.is_some() {
            return pick;
        }
    }
    None
}

/// The misspelled words of `text` in `language` (`None`: the checker's
/// choice), as char ranges of it.
pub fn misspelled(text: &str, language: Option<&str>) -> Vec<Range<usize>> {
    let units = infiniterm_editor::spell::char_to_utf16(text, text.chars().count());
    let s = ns_string(text);
    let checker = checker();
    let lang: Id = language.map_or(std::ptr::null_mut(), ns_string);
    let mut out = vec![];
    let mut start = 0usize;
    while start < units {
        let r: NsRange = unsafe {
            msg_send![checker, checkSpellingOfString: s
                startingAt: start as isize
                language: lang
                wrap: NO
                inSpellDocumentWithTag: tag()
                wordCount: std::ptr::null_mut::<isize>()]
        };
        if r.location == NS_NOT_FOUND || r.length == 0 || r.location < start {
            break;
        }
        let a = infiniterm_editor::spell::utf16_to_char(text, r.location);
        let b = infiniterm_editor::spell::utf16_to_char(text, r.location + r.length);
        out.push(a..b);
        start = r.location + r.length;
    }
    out
}

/// What the checker suggests for `word` in `language`, best first.
pub fn guesses(word: &str, language: Option<&str>) -> Vec<String> {
    let s = ns_string(word);
    let nil: Id = language.map_or(std::ptr::null_mut(), ns_string);
    let range = NsRange {
        location: 0,
        length: infiniterm_editor::spell::char_to_utf16(word, word.chars().count()),
    };
    let list: Id = unsafe {
        msg_send![checker(), guessesForWordRange: range
            inString: s
            language: nil
            inSpellDocumentWithTag: tag()]
    };
    if list.is_null() {
        return vec![];
    }
    let n: usize = unsafe { msg_send![list, count] };
    (0..n)
        .filter_map(|i| rust_string(unsafe { msg_send![list, objectAtIndex: i] }))
        .collect()
}

/// Adds `word` to the user's dictionary.
pub fn learn(word: &str) {
    let _: () = unsafe { msg_send![checker(), learnWord: ns_string(word)] };
    GENERATION.fetch_add(1, Ordering::SeqCst);
}

/// Stops flagging `word` until the app quits.
pub fn ignore(word: &str) {
    let _: () =
        unsafe { msg_send![checker(), ignoreWord: ns_string(word) inSpellDocumentWithTag: tag()] };
    GENERATION.fetch_add(1, Ordering::SeqCst);
}
