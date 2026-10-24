//! Editor cards and their bodies: one `EditorTabs` per editor card (a
//! tab strip over one `EditorBody` per tab), kept in step with the card
//! (tabs, path, root, sidebar, lock), the settings (font, wrap, line
//! wash, cursor blink) and the theme (colours, syntax rules) each frame,
//! the way `terminals.rs` does for shells. The active tab's body reports
//! back what only it knows: dirty, the language, read-only, a file the
//! tree asked to open; the card carries those for the badges and the
//! save file. `Card.tabs` is the authority, the browser's rule.
//!
//! The editor actions the model queues (`Effect::Editor`) land here too:
//! save, find, go to line, the tree's three states.
use crate::editor_body::{EditorBody, EditorEvent};
use crate::editor_tabs::EditorTabs;
use crate::tab_strip::StripStyle;

use crate::page_body::{PageBody, PageColors};
use crate::transcript_body::{TranscriptBody, TranscriptColors};
use crate::AppView;
use gpui::Window;
use infiniterm_core::config::Wrap;
use infiniterm_core::editor_theme::{editor_colors, syntax_rules, Chrome as EditorChrome};
use infiniterm_core::grid::Size;
use infiniterm_core::model::EditorAction;
use infiniterm_core::saved_layout::CardKind;
use infiniterm_core::sidebar::{clamp_sidebar, sidebar_extent, sidebar_width};
use infiniterm_editor::language::Language;

/// The generated config files are rewritten on every launch, so an edit
/// to one is lost by design; read-only says so at the keystroke.
fn is_generated(path: &str) -> bool {
    path.ends_with("/settings.default.json")
        || path.ends_with("/keybindings.default.json")
        || std::path::Path::new(path) == infiniterm_core::welcome::welcome_path()
        || std::path::Path::new(path).starts_with(infiniterm_core::help_docs::docs_dir())
}

impl AppView {
    /// The ACTIVE tab's body of an editor card.
    pub fn editor_for(&mut self, id: &str) -> Option<&mut EditorBody> {
        self.editor_tabs_for(id).and_then(|t| t.active_body())
    }

    pub fn editor_tabs_for(&mut self, id: &str) -> Option<&mut EditorTabs> {
        self.bodies
            .get_mut(id)
            .and_then(|b| b.as_any_mut().downcast_mut::<EditorTabs>())
    }

    pub fn reconcile_editors(&mut self, window: &Window) {
        self.reconcile_transcripts(window);
        self.reconcile_pages(window);
        self.reconcile_whats_new(window);
        let metrics = self.metrics(window);
        let cfg = self.model.config.clone();
        let chrome = EditorChrome {
            card_bg: format!("#{:06x}", rgb_u32(self.chrome.card_bg)),
            text: format!("#{:06x}", rgb_u32(self.chrome.text)),
            text_faint: format!("#{:06x}", rgb_u32(self.chrome.text_faint)),
        };
        let colors = editor_colors(
            self.chrome.theme.as_ref(),
            &chrome,
            &cfg.editor.selection_color,
            &cfg.editor.selection_text_color,
            &cfg.editor.cursor_color,
        );
        let rules = syntax_rules(self.chrome.theme.as_ref());
        let style = StripStyle {
            bg: self.chrome.bar_bg,
            border: self.chrome.bar_border,
            active_bg: self.chrome.row_selected,
            text_bright: self.chrome.text_bright,
            text_muted: self.chrome.text_muted,
            regular: self.chrome.typography.regular.clone(),
            bold: self.chrome.typography.bold.clone(),
            font_px: self.chrome.typography.font_size,
        };
        let ui_scale = self.model.ui_scale as f32;
        let now = crate::now_ms();
        let cards: Vec<_> = self
            .model
            .cards
            .iter()
            .filter(|c| c.kind.has_editor_body())
            .cloned()
            .collect();
        let mut locks: Vec<(String, bool)> = vec![];
        for card in cards {
            let world = Size {
                w: card.rect.w,
                h: card.rect.h,
            };
            let is_diff = card.kind == CardKind::Diff;
            let fresh = self.editor_tabs_for(&card.id).is_none();
            if fresh {
                let mut tabs = if is_diff {
                    // The diff's scope: the root it was opened on, else its
                    // file, else the directory it was opened from.
                    let root = card
                        .root
                        .clone()
                        .or_else(|| card.path.clone())
                        .unwrap_or_else(|| card.cwd.clone());
                    EditorTabs::new_diff(&card.id, &root, &metrics, world, style.clone())
                } else {
                    EditorTabs::new(&card.id, &metrics, world, style.clone())
                };
                // A card born locked (an in-place editor, cover.rs) takes
                // keys at once. The lock is the body's from here on, and the
                // mirror below copies it back onto the card.
                tabs.locked = card.locked;
                self.bodies.insert(card.id.clone(), Box::new(tabs));
            }
            let Some(tabs) = self.editor_tabs_for(&card.id) else {
                continue;
            };
            tabs.ui_scale = ui_scale;
            if tabs.card_number != card.number {
                tabs.card_number = card.number;
                tabs.mark_dirty();
            }
            if tabs.protected != card.protected {
                tabs.protected = card.protected;
                tabs.mark_dirty();
            }
            if tabs.style != style {
                tabs.style = style.clone();
                tabs.mark_dirty();
            }
            // The card's tabs, or its one path before it had any: the
            // bodies follow. A fresh card restores drafts; a tab opened
            // later starts from the file.
            if is_diff {
                // One body, no tabs: opened on a file it shows that file's
                // diff (with the list as the card says), else it opens on
                // the list of changed files.
                if fresh {
                    if let Some(body) = tabs.active_body() {
                        body.diff_start(card.path.as_deref(), card.explorer);
                    }
                }
            } else {
                let handles: Vec<String> = if card.tabs.is_empty() {
                    vec![card.path.clone().unwrap_or_default()]
                } else {
                    card.tabs.clone()
                };
                tabs.rebuild_tabs(&handles, card.active_tab, &card.cwd, now, fresh);
            }
            // The lock, mirrored the way browsers.rs does: the body owns
            // it (a click into the text, Enter), the card shows it.
            let locked = tabs.locked;
            if card.locked != locked {
                locks.push((card.id.clone(), locked));
            }
            let Some(body) = self.editor_for(&card.id) else {
                continue;
            };
            if fresh {
                body.pending_line = card.line;
                if let Some(line) = card.line {
                    body.go_to_line(line as usize);
                }
            }
            if body.metrics != metrics {
                body.metrics = metrics.clone();
                body.mark_dirty();
            }
            // The bar lays out in cells, so it takes the text's own font and
            // cell: a proportional UI font in M-wide cells spread the path.
            let status_font = metrics.font();
            if body.status_font != status_font
                || body.status_font_px != metrics.font_px
                || body.status_cell_w != metrics.cell_w
            {
                body.status_font = status_font;
                body.status_font_px = metrics.font_px;
                body.status_cell_w = metrics.cell_w;
                body.mark_dirty();
            }
            if body.colors != colors || body.rules != rules {
                body.colors = colors.clone();
                body.rules = rules.clone();
                body.mark_dirty();
            }
            body.blink = cfg.terminal.cursor_blink;
            body.highlight_line = cfg.editor.highlight_line;
            body.complete_words = cfg.editor.complete_words;
            let path = body.path.clone().unwrap_or_default();
            body.wrap = match cfg.editor.wrap {
                Wrap::Always => true,
                Wrap::Never => false,
                Wrap::Prose => Language::is_prose(&path),
            };
            body.read_only = is_diff || is_generated(&path) || Language::is_image(&path);
            body.sidebar_top = card.sidebar_top;
            body.sidebar_w = sidebar_width(card.sidebar, sidebar_extent(world, card.sidebar_top));
            // A diff's list is its own (changed files, not the folder).
            if card.explorer && !is_diff {
                if let Some(root) = card.root.clone() {
                    body.show_tree(&root);
                }
            }
            // What the body knows and the card shows.
            let dirty = body.is_dirty();
            let language = body.language.map(|l| l.badge().to_string());
            let read_only = body.read_only;
            let list_shown = body.tree.is_some();
            let events = self
                .editor_tabs_for(&card.id)
                .map(|t| t.take_events())
                .unwrap_or_default();
            if let Some(c) = self.model.card_mut(&card.id) {
                c.dirty = dirty;
                c.language = language;
                c.read_only = read_only;
                // The list of a diff is the card's explorer: Cmd+K flips it,
                // and a relaunch brings it back.
                if is_diff && c.explorer != list_shown {
                    c.explorer = list_shown;
                    self.model.dirty_layout = true;
                }
            }
            for event in events {
                match event {
                    EditorEvent::None => {}
                    EditorEvent::Notice(text) => self.model.notify(text),
                    // The active tab's file changed under it (a save-as, a
                    // picture the tree landed on): the card's path and, when
                    // it has tabs, the active tab's handle follow.
                    EditorEvent::PathChanged { path, cwd } => {
                        if let Some(c) = self.model.card_mut(&card.id) {
                            c.path = Some(path.clone());
                            c.cwd = cwd;
                            if let Some(h) = c.tabs.get_mut(c.active_tab) {
                                *h = path;
                            }
                            self.model.dirty_layout = true;
                        }
                    }
                    // The tree's divider dragged: the width the card keeps,
                    // clamped so the text keeps its minimum (#258).
                    EditorEvent::Sidebar(w) => {
                        if let Some(c) = self.model.card_mut(&card.id) {
                            let extent = sidebar_extent(
                                infiniterm_core::grid::Size {
                                    w: c.rect.w,
                                    h: c.rect.h,
                                },
                                c.sidebar_top,
                            );
                            c.sidebar = Some(clamp_sidebar(w, extent));
                            self.model.dirty_layout = true;
                        }
                    }
                    // Enter on a file in the tree: the tab that shows it,
                    // else a new one. Through the model, the authority.
                    EditorEvent::OpenTab(path) => {
                        let already = self
                            .model
                            .card(&card.id)
                            .and_then(|c| c.tabs.iter().position(|h| *h == path));
                        match already {
                            Some(i) => self.model.browser_tab_jump(&card.id, i),
                            None => self.model.browser_tab_open(&card.id, Some(&path)),
                        }
                    }
                }
            }
        }
        for (id, locked) in locks {
            if let Some(c) = self.model.card_mut(&id) {
                c.locked = locked;
            }
            self.redraw = true;
        }
    }

    /// One `PageBody` per Page card (#42). The chrome's text on the card's
    /// ground, the theme's blue for headings, green for code and cyan for
    /// links.
    /// Where this window's cards read their files: the host's, in a joined
    /// window (one connection, made at the first request, #560), else this
    /// machine's own disk.
    #[cfg(unix)]
    pub fn file_source(&mut self) -> std::sync::Arc<dyn infiniterm_core::file_source::FileSource> {
        use infiniterm_core::file_source::{FileSource, LocalSource};
        if self.viewer {
            if let Some(s) = &self.host_files {
                return s.clone();
            }
            if let Some(host) = infiniterm_core::backend::remote::join_from_env() {
                let source: std::sync::Arc<dyn FileSource> =
                    infiniterm_core::backend::file_client::HostSource::new(host);
                self.host_files = Some(source.clone());
                return source;
            }
        }
        std::sync::Arc::new(LocalSource)
    }

    /// Joining is unix-only, so there is no host to read from elsewhere.
    #[cfg(windows)]
    pub fn file_source(&mut self) -> std::sync::Arc<dyn infiniterm_core::file_source::FileSource> {
        std::sync::Arc::new(infiniterm_core::file_source::LocalSource)
    }

    pub fn reconcile_pages(&mut self, window: &Window) {
        let metrics = self.metrics(window);
        let theme_hex = |k: &str| {
            self.chrome
                .theme
                .as_ref()
                .and_then(|t| t.get(k))
                .and_then(|s| crate::chrome::hex(s))
        };
        let colors = PageColors {
            background: self.chrome.card_bg,
            text: self.chrome.card_fg,
            faint: self.chrome.text_faint,
            heading: theme_hex("blue").unwrap_or(self.chrome.text),
            code: theme_hex("green").unwrap_or(self.chrome.text_mid),
            link: theme_hex("cyan").unwrap_or(self.chrome.focus_ring),
        };
        let inactive_dim = self.model.config.ui.inactive_dim;
        let cards: Vec<_> = self
            .model
            .cards
            .iter()
            .filter(|c| c.kind == CardKind::Page)
            .cloned()
            .collect();
        for card in cards {
            let Some(path) = card.path.clone() else {
                continue;
            };
            let world = Size {
                w: card.rect.w,
                h: card.rect.h,
            };
            // Replaced, not merely inserted: `paint` puts a `Blank` in every
            // card's slot before the bodies are built, and keeping it drew
            // an empty page.
            let built = self
                .bodies
                .get_mut(&card.id)
                .and_then(|b| b.as_any_mut().downcast_mut::<PageBody>())
                .is_some();
            if !built {
                let source = self.file_source();
                self.bodies.insert(
                    card.id.clone(),
                    Box::new(PageBody::new(
                        path.clone(),
                        &metrics,
                        colors.clone(),
                        world,
                        source,
                    )),
                );
            }
            let Some(body) = self.bodies.get_mut(&card.id) else {
                continue;
            };
            let Some(body) = body.as_any_mut().downcast_mut::<PageBody>() else {
                continue;
            };
            if body.metrics != metrics {
                body.metrics = metrics.clone();
                body.mark_dirty();
            }
            if body.colors != colors {
                body.colors = colors.clone();
                body.mark_dirty();
            }
            body.inactive_dim = inactive_dim;
        }
    }

    /// One `TranscriptBody` per transcript card. Its colours are the
    /// chrome's, with the theme's yellow and blue for who spoke.
    fn reconcile_transcripts(&mut self, window: &Window) {
        let metrics = self.metrics(window);
        let theme_hex = |k: &str| {
            self.chrome
                .theme
                .as_ref()
                .and_then(|t| t.get(k))
                .and_then(|s| crate::chrome::hex(s))
        };
        let colors = TranscriptColors {
            background: self.chrome.card_bg,
            foreground: self.chrome.card_fg,
            faint: self.chrome.text_faint,
            user: theme_hex("yellow").unwrap_or(self.chrome.text_mid),
            assistant: theme_hex("blue").unwrap_or(self.chrome.text_mid),
            // Another session's message: neither you nor this agent.
            peer: theme_hex("magenta").unwrap_or(self.chrome.text_mid),
            sel_bg: self.chrome.sel_bg,
            sel_fg: self.chrome.sel_fg,
        };
        let inactive_dim = self.model.config.ui.inactive_dim;
        let cards: Vec<_> = self
            .model
            .cards
            .iter()
            .filter(|c| c.kind == CardKind::Transcript)
            .cloned()
            .collect();
        for card in cards {
            let world = Size {
                w: card.rect.w,
                h: card.rect.h,
            };
            let exists = self
                .bodies
                .get_mut(&card.id)
                .and_then(|b| b.as_any_mut().downcast_mut::<TranscriptBody>())
                .is_some();
            if !exists {
                let mut body = TranscriptBody::new(card.path.clone(), &metrics, world);
                body.idle(crate::now_ms());
                self.bodies.insert(card.id.clone(), Box::new(body));
            }
            let Some(body) = self
                .bodies
                .get_mut(&card.id)
                .and_then(|b| b.as_any_mut().downcast_mut::<TranscriptBody>())
            else {
                continue;
            };
            if body.metrics != metrics {
                body.metrics = metrics.clone();
                body.mark_dirty();
            }
            if body.colors != colors {
                body.colors = colors.clone();
                body.mark_dirty();
            }
            body.inactive_dim = inactive_dim;
            body.sidebar_top = card.sidebar_top;
            body.sidebar_w = sidebar_width(card.sidebar, sidebar_extent(world, card.sidebar_top));
            if body.path != card.path {
                body.path = card.path.clone();
                body.idle(crate::now_ms() + 1e9);
            }
        }
    }

    /// `Effect::Editor`: the model asked a card's editor for something.
    pub fn editor_effect(&mut self, card_id: &str, action: EditorAction) {
        let now = crate::now_ms();
        let card = self.model.card(card_id).cloned();
        let Some(card) = card else { return };
        // A diff card is the editor in a read-only diff mode: its list and
        // blame are its own, find and unlock are the editor's, and nothing
        // that writes (save, transforms) or names a file's folder (the
        // tree's menu) applies.
        if card.kind == CardKind::Diff {
            match action {
                EditorAction::ToggleExplorer => {
                    if let Some(body) = self.editor_for(card_id) {
                        let shown = body.diff_toggle_list();
                        if let Some(c) = self.model.card_mut(card_id) {
                            c.explorer = shown;
                            self.model.dirty_layout = true;
                        }
                    }
                }
                EditorAction::ToggleBlame => {
                    if let Some(body) = self.editor_for(card_id) {
                        body.diff_toggle_blame();
                    }
                }
                EditorAction::Find => {
                    if let Some(tabs) = self.editor_tabs_for(card_id) {
                        tabs.locked = true;
                    }
                    if let Some(body) = self.editor_for(card_id) {
                        body.open_search(false);
                    }
                }
                EditorAction::Unlock => {
                    if let Some(t) = self.editor_tabs_for(card_id) {
                        t.locked = false;
                        t.mark_dirty();
                    }
                    if let Some(c) = self.model.card_mut(card_id) {
                        c.locked = false;
                    }
                }
                _ => {}
            }
            return;
        }
        match action {
            EditorAction::Save => {
                if let Some(path) = card.path.clone() {
                    if let Some(body) = self.editor_for(card_id) {
                        body.save(&path, now);
                    }
                }
            }
            EditorAction::Find => {
                // Cmd+F on an editor you only arrowed to: its search, and
                // the lock that lets you type into it.
                if let Some(tabs) = self.editor_tabs_for(card_id) {
                    tabs.locked = true;
                }
                if let Some(body) = self.editor_for(card_id) {
                    body.open_search(false);
                }
            }
            EditorAction::GoToLine => {
                if let Some(line) = card.line {
                    if let Some(body) = self.editor_for(card_id) {
                        body.go_to_line(line as usize);
                    }
                    if let Some(c) = self.model.card_mut(card_id) {
                        c.line = None;
                    }
                }
            }
            EditorAction::Transform(t) => {
                if let Some(body) = self.editor_for(card_id) {
                    body.apply_transform(t, now);
                }
            }
            EditorAction::ToggleExplorer => {
                // A file opened on its own has no root; its directory becomes one.
                let root = card.root.clone().unwrap_or_else(|| card.cwd.clone());
                let shown = match self.editor_for(card_id) {
                    Some(body) => body.toggle_tree(&root),
                    None => return,
                };
                if let Some(c) = self.model.card_mut(card_id) {
                    c.root = Some(root);
                    c.explorer = shown;
                    self.model.dirty_layout = true;
                }
            }
            EditorAction::ToggleBlame => {} // the diff card, Phase 7
            // The file tree's menu: the row it was opened on, else the cursor's.
            EditorAction::Tree(act) => {
                use infiniterm_core::model::TreeAct;
                let Some(body) = self.editor_for(card_id) else {
                    return;
                };
                let entry = body.menu_entry();
                let root = body.tree_root().map(String::from);
                if act == TreeAct::Open {
                    body.tree_open_entry();
                    return;
                }
                let Some(entry) = entry else { return };
                match act {
                    TreeAct::RevealInFinder => {
                        let _ = std::process::Command::new("open")
                            .args(["-R", &entry.path])
                            .spawn();
                    }
                    TreeAct::CopyPath => self.clipboard_out = Some(entry.path),
                    TreeAct::CopyRelativePath => {
                        let relative = root
                            .as_deref()
                            .and_then(|r| entry.path.strip_prefix(r))
                            .map(|p| p.trim_start_matches('/').to_string())
                            .filter(|p| !p.is_empty())
                            .unwrap_or_else(|| entry.path.clone());
                        self.clipboard_out = Some(relative);
                    }
                    TreeAct::Open => {}
                }
            }
            // The lock is the body's: the card's flag is written back from it
            // every frame, so dropping it on the card alone would be undone.
            EditorAction::Unlock => {
                if let Some(t) = self.editor_tabs_for(card_id) {
                    t.locked = false;
                    t.mark_dirty();
                }
                if let Some(c) = self.model.card_mut(card_id) {
                    c.locked = false;
                }
            }
        }
    }

    /// Between frames: drafts and the disk poll of every editor and diff.
    pub fn idle_editors(&mut self, now: f64) {
        for body in self.bodies.values_mut() {
            // Every tab, not only the active one: a file changing on disk
            // under a background tab is picked up before it is looked at.
            if let Some(t) = body.as_any_mut().downcast_mut::<EditorTabs>() {
                t.idle(now);
            } else if let Some(t) = body.as_any_mut().downcast_mut::<TranscriptBody>() {
                t.idle(now);
            } else if let Some(p) = body.as_any_mut().downcast_mut::<PageBody>() {
                p.idle(now);
            }
        }
    }
}

fn rgb_u32(c: gpui::Hsla) -> u32 {
    let rgba: gpui::Rgba = c.into();
    ((rgba.r * 255.).round() as u32) << 16
        | ((rgba.g * 255.).round() as u32) << 8
        | (rgba.b * 255.).round() as u32
}
