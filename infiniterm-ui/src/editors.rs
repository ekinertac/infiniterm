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
use crate::diff_body::DiffBody;
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
use infiniterm_core::sidebar::{sidebar_extent, sidebar_width};
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

    pub fn diff_for(&mut self, id: &str) -> Option<&mut DiffBody> {
        self.bodies
            .get_mut(id)
            .and_then(|b| b.as_any_mut().downcast_mut::<DiffBody>())
    }

    pub fn reconcile_editors(&mut self, window: &Window) {
        self.reconcile_diffs(window);
        self.reconcile_transcripts(window);
        self.reconcile_pages(window);
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
        );
        let rules = syntax_rules(self.chrome.theme.as_ref());
        let style = StripStyle {
            bg: self.chrome.bar_bg,
            border: self.chrome.bar_border,
            active_bg: self.chrome.row_selected,
            text_bright: self.chrome.text_bright,
            text_muted: self.chrome.text_muted,
            font_family: crate::terminals::family_of(&cfg.terminal.font_family),
            font_px: cfg.terminal.font_size,
        };
        let ui_scale = self.model.ui_scale as f32;
        let now = crate::now_ms();
        let cards: Vec<_> = self
            .model
            .cards
            .iter()
            .filter(|c| c.kind == CardKind::Editor)
            .cloned()
            .collect();
        let mut locks: Vec<(String, bool)> = vec![];
        for card in cards {
            let world = Size {
                w: card.rect.w,
                h: card.rect.h,
            };
            let fresh = self.editor_tabs_for(&card.id).is_none();
            if fresh {
                let mut tabs = EditorTabs::new(&card.id, &metrics, world, style.clone());
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
            let handles: Vec<String> = if card.tabs.is_empty() {
                vec![card.path.clone().unwrap_or_default()]
            } else {
                card.tabs.clone()
            };
            tabs.rebuild_tabs(&handles, card.active_tab, &card.cwd, now, fresh);
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
            if body.colors != colors || body.rules != rules {
                body.colors = colors.clone();
                body.rules = rules.clone();
                body.mark_dirty();
            }
            body.blink = cfg.terminal.cursor_blink;
            body.highlight_line = cfg.editor.highlight_line;
            let path = body.path.clone().unwrap_or_default();
            body.wrap = match cfg.editor.wrap {
                Wrap::Always => true,
                Wrap::Never => false,
                Wrap::Prose => Language::is_prose(&path),
            };
            body.read_only = is_generated(&path) || Language::is_image(&path);
            body.sidebar_top = card.sidebar_top;
            body.sidebar_w = sidebar_width(card.sidebar, sidebar_extent(world, card.sidebar_top));
            if card.explorer {
                if let Some(root) = card.root.clone() {
                    body.show_tree(&root);
                }
            }
            // What the body knows and the card shows.
            let dirty = body.is_dirty();
            let language = body.language.map(|l| l.badge().to_string());
            let read_only = body.read_only;
            let events = self
                .editor_tabs_for(&card.id)
                .map(|t| t.take_events())
                .unwrap_or_default();
            if let Some(c) = self.model.card_mut(&card.id) {
                c.dirty = dirty;
                c.language = language;
                c.read_only = read_only;
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
    fn reconcile_pages(&mut self, window: &Window) {
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
            let body = self.bodies.entry(card.id.clone()).or_insert_with(|| {
                Box::new(PageBody::new(path.clone(), &metrics, colors.clone(), world))
            });
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

    /// One `DiffBody` per diff card, kept in step like the editors.
    fn reconcile_diffs(&mut self, window: &Window) {
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
        );
        let rules = syntax_rules(self.chrome.theme.as_ref());
        let cards: Vec<_> = self
            .model
            .cards
            .iter()
            .filter(|c| c.kind == CardKind::Diff)
            .cloned()
            .collect();
        for card in cards {
            let world = Size {
                w: card.rect.w,
                h: card.rect.h,
            };
            if self.diff_for(&card.id).is_none() {
                let root = card
                    .root
                    .clone()
                    .or_else(|| card.path.clone())
                    .unwrap_or_else(|| card.cwd.clone());
                let mut body = DiffBody::new(&card.id, &root, &metrics, world);
                body.tree_shown = card.explorer;
                body.refresh();
                // Opened on a file: show it at once. On a directory: the list.
                let repo = body.repo().to_string();
                let target = card
                    .path
                    .clone()
                    .or_else(|| (card.root.is_some() && !card.explorer).then(|| root.clone()));
                match target {
                    Some(t)
                        if !repo.is_empty()
                            && t.starts_with(&repo)
                            && std::path::Path::new(&t).is_file() =>
                    {
                        let rel = t[repo.len()..].trim_start_matches('/').to_string();
                        body.show(&rel);
                    }
                    _ => {
                        body.tree_shown = true;
                        body.tree_focused = true;
                    }
                }
                self.bodies.insert(card.id.clone(), Box::new(body));
            }
            let Some(body) = self.diff_for(&card.id) else {
                continue;
            };
            if body.metrics != metrics {
                body.metrics = metrics.clone();
                body.mark_dirty();
            }
            if body.colors != colors || body.rules != rules {
                body.colors = colors.clone();
                body.rules = rules.clone();
                body.mark_dirty();
            }
            body.inactive_dim = cfg.ui.inactive_dim;
            body.sidebar_top = card.sidebar_top;
            body.sidebar_w = sidebar_width(card.sidebar, sidebar_extent(world, card.sidebar_top));
            let shown = body.tree_shown;
            let events = body.take_events();
            if let Some(c) = self.model.card_mut(&card.id) {
                if c.explorer != shown {
                    c.explorer = shown;
                    self.model.dirty_layout = true;
                }
            }
            for event in events {
                match event {
                    EditorEvent::None => {}
                    EditorEvent::Notice(text) => self.model.notify(text),
                    EditorEvent::OpenTab(_) => {}
                    EditorEvent::PathChanged { path, cwd } => {
                        if let Some(c) = self.model.card_mut(&card.id) {
                            c.path = Some(path);
                            c.cwd = cwd;
                            self.model.dirty_layout = true;
                        }
                    }
                }
            }
        }
    }

    /// `Effect::Editor`: the model asked a card's editor for something.
    pub fn editor_effect(&mut self, card_id: &str, action: EditorAction) {
        let now = crate::now_ms();
        let card = self.model.card(card_id).cloned();
        let Some(card) = card else { return };
        if card.kind == CardKind::Diff {
            match action {
                EditorAction::ToggleExplorer => {
                    if let Some(body) = self.diff_for(card_id) {
                        body.toggle_tree();
                    }
                }
                EditorAction::ToggleBlame => {
                    if let Some(body) = self.diff_for(card_id) {
                        body.toggle_blame();
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
        }
    }

    /// Between frames: drafts and the disk poll of every editor and diff.
    pub fn idle_editors(&mut self, now: f64) {
        for body in self.bodies.values_mut() {
            // Every tab, not only the active one: a file changing on disk
            // under a background tab is picked up before it is looked at.
            if let Some(t) = body.as_any_mut().downcast_mut::<EditorTabs>() {
                t.idle(now);
            } else if let Some(d) = body.as_any_mut().downcast_mut::<DiffBody>() {
                d.idle(now);
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
