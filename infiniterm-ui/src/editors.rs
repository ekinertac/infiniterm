//! Editor cards and their bodies: one `EditorBody` per editor card, kept
//! in step with the card (path, root, sidebar), the settings (font, wrap,
//! line wash, cursor blink) and the theme (colours, syntax rules) each
//! frame, the way `terminals.rs` does for shells. The body reports back
//! what only it knows: dirty, the language, read-only, a file picked from
//! the tree; the card carries those for the badges and the save file.
//!
//! The editor actions the model queues (`Effect::Editor`) land here too:
//! save, find, go to line, the tree's three states.
use crate::diff_body::DiffBody;
use crate::editor_body::{EditorBody, EditorEvent};
use crate::terminals::family_of;
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
    path.ends_with("/settings.default.json") || path.ends_with("/keybindings.default.json")
}

impl AppView {
    pub fn editor_for(&mut self, id: &str) -> Option<&mut EditorBody> {
        self.bodies
            .get_mut(id)
            .and_then(|b| b.as_any_mut().downcast_mut::<EditorBody>())
    }

    pub fn diff_for(&mut self, id: &str) -> Option<&mut DiffBody> {
        self.bodies
            .get_mut(id)
            .and_then(|b| b.as_any_mut().downcast_mut::<DiffBody>())
    }

    pub fn reconcile_editors(&mut self, window: &Window) {
        self.reconcile_diffs(window);
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
        let now = crate::now_ms();
        let cards: Vec<_> = self
            .model
            .cards
            .iter()
            .filter(|c| c.kind == CardKind::Editor)
            .cloned()
            .collect();
        for card in cards {
            let world = Size {
                w: card.rect.w,
                h: card.rect.h,
            };
            let is_editor = self.editor_for(&card.id).is_some();
            if !is_editor {
                let mut body = EditorBody::new(
                    &card.id,
                    card.path.clone(),
                    card.cwd.clone(),
                    &metrics,
                    world,
                );
                body.pending_line = card.line;
                match card.path.clone() {
                    Some(path) => body.load(&path, true, now),
                    None => body.load_untitled(),
                }
                self.bodies.insert(card.id.clone(), Box::new(body));
            }
            let Some(body) = self.editor_for(&card.id) else {
                continue;
            };
            if body.metrics.family != family_of(&cfg.terminal.font_family)
                || body.metrics.font_px != metrics.font_px
                || body.metrics.line_height != metrics.line_height
                || body.metrics.cell_w != metrics.cell_w
            {
                body.metrics = crate::terminal_body::Metrics {
                    family: metrics.family.clone(),
                    font_px: metrics.font_px,
                    line_height: metrics.line_height,
                    cell_w: metrics.cell_w,
                };
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
            body.read_only = is_generated(&path);
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
            let events = body.take_events();
            if let Some(c) = self.model.card_mut(&card.id) {
                c.dirty = dirty;
                c.language = language;
                c.read_only = read_only;
            }
            for event in events {
                match event {
                    EditorEvent::None => {}
                    EditorEvent::Notice(text) => self.model.notify(text),
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
            if body.metrics.font_px != metrics.font_px
                || body.metrics.cell_w != metrics.cell_w
                || body.metrics.line_height != metrics.line_height
                || body.metrics.family != metrics.family
            {
                body.metrics = crate::terminal_body::Metrics {
                    family: metrics.family.clone(),
                    font_px: metrics.font_px,
                    line_height: metrics.line_height,
                    cell_w: metrics.cell_w,
                };
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
            if let Some(e) = body.as_any_mut().downcast_mut::<EditorBody>() {
                e.idle(now);
            } else if let Some(d) = body.as_any_mut().downcast_mut::<DiffBody>() {
                d.idle(now);
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
