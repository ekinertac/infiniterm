//! Writes the documentation site's generated reference pages (keys,
//! commands, settings) into the directory given as the only argument.
//!
//! Run by site/package.json before `astro dev` and `astro build`, so the
//! pages come from the code on every build; the rendering itself is
//! `infiniterm_core::site_reference`, which is where the tests are.
use std::{env, fs, path::PathBuf, process::ExitCode};

fn main() -> ExitCode {
    let Some(dir) = env::args().nth(1).map(PathBuf::from) else {
        eprintln!("usage: site_reference <output dir>");
        return ExitCode::from(2);
    };
    if let Err(e) = fs::create_dir_all(&dir) {
        eprintln!("site_reference: {}: {e}", dir.display());
        return ExitCode::FAILURE;
    }
    for page in infiniterm_core::site_reference::pages() {
        let path = dir.join(page.file);
        if let Err(e) = fs::write(&path, page.text) {
            eprintln!("site_reference: {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}
