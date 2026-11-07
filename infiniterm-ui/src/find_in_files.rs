//! The files side of Find in Files and Replace in Files (#602): list the
//! project, read each file through the card's `FileSource`, run the pattern
//! (`infiniterm_editor::grep`), and either hand the hits to the palette
//! (`Model::open_grep`) or count and then rewrite the files.
//!
//! Called by `editors.rs` when the model's prompts finish
//! (`EditorAction::FindInFiles`, `ReplaceInFilesCount`, `ReplaceInFilesApply`).
//! Related: `infiniterm-core/src/quick_files.rs` (the file list, shared with
//! Go to File), `model/palette_state.rs` (`Source::Grep`, the pick).
//!
//! It runs on the UI thread, once per command: a repository of a few
//! thousand source files is a fraction of a second in a release build. A
//! file that is not UTF-8, is binary or is over `MAX_FILE_BYTES` is skipped
//! by both the search and the replace, so a replace never rewrites a file it
//! did not search. A write goes through `FileSource::write` with the mtime
//! the read had, so a file changed since is refused rather than overwritten,
//! and an open buffer with unsaved edits keeps them (its own save is refused
//! the same way, which is the editor's existing rule).

use crate::AppView;
use infiniterm_core::file_source::FileSource;
use infiniterm_core::model::{Card, GrepHit, Pending};
use infiniterm_editor::grep::{self, MAX_FILE_BYTES};
use infiniterm_editor::search::{self, Options};
use std::sync::Arc;

/// A file's text and mtime, or `None` for one the search leaves alone.
fn read_text(source: &Arc<dyn FileSource>, path: &str) -> Option<(String, Option<u64>)> {
    let data = source.read(path).ok()?;
    if data.bytes.len() > MAX_FILE_BYTES || grep::is_binary(&data.bytes) {
        return None;
    }
    let mtime = data.mtime_ms;
    String::from_utf8(data.bytes).ok().map(|t| (t, mtime))
}

impl AppView {
    /// The folder a card's project search covers and the files under it:
    /// the tree's root, else the open file's folder, else the card's
    /// directory; on this machine then the top of the git repository it is
    /// in, so the whole project is covered whichever file you are on. A repo
    /// lists from `git ls-files`, any other folder or a host's from a capped
    /// walk.
    pub fn project_files(&mut self, card: &Card) -> (String, Vec<String>) {
        use infiniterm_core::quick_files as qf;
        let source = self.file_source();
        let base = card
            .root
            .clone()
            .or_else(|| {
                card.path
                    .as_deref()
                    .map(infiniterm_core::paths::parent_dir)
                    .filter(|d| !d.is_empty())
            })
            .unwrap_or_else(|| card.cwd.clone());
        let local = source.is_local();
        let root = if local {
            qf::repo_root(&base).unwrap_or(base)
        } else {
            base
        };
        let files = local
            .then(|| qf::list_local_git(&root, qf::CAP))
            .flatten()
            .unwrap_or_else(|| {
                let cap = if local { qf::CAP } else { qf::REMOTE_CAP };
                qf::walk(&root, cap, &mut |dir| {
                    source
                        .list(dir)
                        .map(|es| es.into_iter().map(|e| (e.name, e.is_dir)).collect())
                        .unwrap_or_default()
                })
            });
        (root, files)
    }

    fn notice(&mut self, text: &str) {
        self.model.prompt.alert(text, Pending::Notice);
    }

    /// `EditorAction::FindInFiles`: search the project and open the hits.
    pub fn find_in_files(&mut self, card_id: &str) {
        let Some(job) = self.model.files_job.clone() else {
            return;
        };
        let Some(card) = self.model.card(card_id).cloned() else {
            return;
        };
        let o = Options {
            regex: job.regex,
            ..Default::default()
        };
        let re = match search::build(&job.find, o) {
            Ok(re) => re,
            Err(e) => return self.notice(&format!("Not a valid pattern: {e}")),
        };
        let (root, files) = self.project_files(&card);
        let source = self.file_source();
        // At `MAX_HITS` the search stops: the list is the first of many, in
        // file order, and a pattern that wide wants narrowing anyway.
        let mut hits = vec![];
        for rel in &files {
            let Some((text, _)) = read_text(&source, &format!("{root}/{rel}")) else {
                continue;
            };
            if grep::search_text(&re, rel, &text, &mut hits) {
                break;
            }
        }
        if hits.is_empty() {
            return self.notice(&format!("No matches for \"{}\"", job.find));
        }
        let hits = hits
            .into_iter()
            .map(|h| GrepHit {
                path: h.path,
                line: h.line,
                text: h.text,
            })
            .collect();
        self.model.open_grep(card_id, &root, hits);
    }

    /// `EditorAction::ReplaceInFilesCount`: how many matches in how many
    /// files, asked before anything is written.
    pub fn replace_in_files_count(&mut self, card_id: &str) {
        let Some(job) = self.model.files_job.clone() else {
            return;
        };
        let Some(card) = self.model.card(card_id).cloned() else {
            return;
        };
        let o = Options {
            regex: job.regex,
            ..Default::default()
        };
        if let Err(e) = search::build(&job.find, o) {
            return self.notice(&format!("Not a valid pattern: {e}"));
        }
        let (root, files) = self.project_files(&card);
        let source = self.file_source();
        let (mut matches, mut touched) = (0, 0);
        for rel in &files {
            let Some((text, _)) = read_text(&source, &format!("{root}/{rel}")) else {
                continue;
            };
            if let Ok(Some((_, n))) = grep::replace_text(&text, &job.find, &job.replace, o) {
                matches += n;
                touched += 1;
            }
        }
        if matches == 0 {
            return self.notice(&format!("No matches for \"{}\"", job.find));
        }
        let label = format!(
            "Replace {matches} match{} in {touched} file{} with \"{}\"? \
             This rewrites the files on disk.",
            if matches == 1 { "" } else { "es" },
            if touched == 1 { "" } else { "s" },
            job.replace
        );
        self.model
            .prompt
            .confirm(&label, "Replace", Pending::ReplaceInFilesYes);
    }

    /// `EditorAction::ReplaceInFilesApply`, after the yes: write the files.
    pub fn replace_in_files_apply(&mut self, card_id: &str) {
        let Some(job) = self.model.files_job.take() else {
            return;
        };
        let Some(card) = self.model.card(card_id).cloned() else {
            return;
        };
        let o = Options {
            regex: job.regex,
            ..Default::default()
        };
        let (root, files) = self.project_files(&card);
        let source = self.file_source();
        let (mut matches, mut written, mut refused) = (0, 0, 0);
        for rel in &files {
            let path = format!("{root}/{rel}");
            let Some((text, mtime)) = read_text(&source, &path) else {
                continue;
            };
            let Ok(Some((new, n))) = grep::replace_text(&text, &job.find, &job.replace, o) else {
                continue;
            };
            match source.write(&path, new.as_bytes(), mtime) {
                Ok(_) => {
                    matches += n;
                    written += 1;
                }
                Err(_) => refused += 1,
            }
        }
        let mut text = format!(
            "Replaced {matches} match{} in {written} file{}.",
            if matches == 1 { "" } else { "es" },
            if written == 1 { "" } else { "s" },
        );
        if refused > 0 {
            text.push_str(&format!(
                " {refused} file{} changed on disk meanwhile or could not be written and \
                 {} left as it was.",
                if refused == 1 { "" } else { "s" },
                if refused == 1 { "was" } else { "were" },
            ));
        }
        self.notice(&text);
    }
}
