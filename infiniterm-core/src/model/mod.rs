//! The app model: every store of the reference (`cards.svelte.ts`,
//! `selection.svelte.ts`, `workspaces.svelte.ts`, `groups.svelte.ts`,
//! `viewport.svelte.ts`, `paletteState.svelte.ts`, `notice.svelte.ts`,
//! `uiScale.svelte.ts`, `persistence.svelte.ts`) as one plain struct, and
//! every command of `src/lib/commands/*` as a method on it.
//!
//! Plain and synchronous on purpose. Anything the reference did as a Svelte
//! effect is done at the mutation site here (`set_focus` is where "a plain
//! focus change empties the extras" and "a phantom is exclusive with focus"
//! live); anything it did to the outside world (animate the viewport, kill a
//! PTY, notify, log, open a URL) is pushed as an `Effect` for the ui crate
//! to perform after the command returns; anything it awaited (a prompt) is
//! a `Pending` value the prompt hands back with the answer. Time comes in
//! through `now_ms`, set by the ui once per frame, so nothing here reads a
//! clock and every rule is testable.
//!
//! The model knows rects, ids and kinds, never what a card body draws: that
//! is the rule that keeps a Wayland surface possible later. Split by domain
//! like the reference's commands directory; the registry is `commands.rs`
//! and `register_commands` fills it.
pub mod canvas_cmd;
pub mod cards_cmd;
pub mod context;
pub mod dev_cmd;
pub mod find_cmd;
pub mod focus_cmd;
pub mod groups_cmd;
pub mod hooks_in;
pub mod ift_in;
pub mod lifecycle;
pub mod omni_cmd;
pub mod palette_state;
pub mod persist;
pub mod register;
pub mod settings_in;
pub mod tabs_cmd;
pub mod workspaces_cmd;

use crate::agent_state::AgentState;
use crate::backend::PaneId;
use crate::cards::{default_size, GUTTER};
use crate::config::{default_config, Config};
use crate::grid::{Point, Rect, Size, HALF_CELL};
use crate::groups::{group_bounds, GROUP_PAD, UNGROUPED};
use crate::keymap::{default_keymap, Keymap};
use crate::layout::nearest_free_slot;
use crate::palette_usage::Usage;
use crate::saved_layout::CardKind;
use crate::slots::Slot;
use crate::viewport::{bounding_rect, Viewport};
use crate::workspaces::{next_name, Workspace, INITIAL_VIEWPORT};
use serde_json::Value;
use std::collections::HashMap;

pub use palette_state::PaletteState;

/// A card. Fields marked runtime-only are not saved; a restored card starts
/// with no backend and no agent state and gets both when its shell spawns.
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub id: String,
    pub kind: CardKind,
    /// An editor's file, a transcript's session file; `None` for an untitled
    /// buffer or a terminal.
    pub path: Option<String>,
    /// The directory an editor's explorer is rooted at.
    pub root: Option<String>,
    pub explorer: bool,
    /// The sidebar's width in card pixels, or `None` for the default.
    pub sidebar: Option<f64>,
    pub sidebar_top: bool,
    /// A browser card's page.
    pub url: Option<String>,
    /// A browser card's page zoom, or `None` for the config default.
    pub zoom: Option<f64>,
    /// Every tab's url, active one included. Empty for a single-tab card:
    /// `url` alone still says where it is, the way it always has. SAVED
    /// only when there are more than one (`saved_layout::SavedCard::tabs`).
    pub tabs: Vec<String>,
    /// Index into `tabs`. Meaningless while `tabs` is empty. SAVED
    /// alongside `tabs`.
    pub active_tab: usize,
    /// Urls closed since the card opened, most recent last, for
    /// `browser.tab.reopenClosed`. Runtime-only: closed tabs do not
    /// survive a restart.
    pub closed_tabs: Vec<String>,
    /// Whether this browser card's keyboard is locked to the page (real
    /// Chrome shortcuts) rather than the app's. Mirrored from
    /// `BrowserBody`'s own focus state every frame (`browsers.rs`); never
    /// written by core itself except through that mirror, and never
    /// saved, the way `osc_title` and `dirty` are not.
    pub locked: bool,
    /// A program for a terminal card to run INSTEAD of the shell (through
    /// `$SHELL -lc`); the card closes when it exits. Runtime-only: a restored
    /// card is a plain shell, because relaunching an agent nobody asked for
    /// on every start is worse than an empty prompt where it was.
    pub command: Option<String>,
    /// The session file of the agent in this card, from its hook events.
    /// Runtime-only; what `card.transcript` opens.
    pub transcript_path: Option<String>,
    /// The last Claude Code session this card ran, from its hook events.
    /// SAVED (`agentSession`): after a reboot the card's shell is new and
    /// the ring is all that is left, so the lost-session notice can offer
    /// `claude --resume <id>` ready to paste. Never cleared by the app;
    /// the next hook event overwrites it.
    pub agent_session: Option<String>,
    /// A short handle for talking about the card ("close #7"), shown ahead
    /// of the label. Given once, at creation, from a counter that only
    /// grows within a session, so two cards never share one while you look
    /// at them; SAVED, so it is the same after a restart. Zero in a file
    /// from before the field, and assigned on load.
    pub number: u32,
    /// This card's shell, as an opaque handle in whichever backend holds it
    /// (a tmux window id like `@7`, or a daemon session id). SAVED, unlike
    /// every other runtime fact about a shell: it is the only thing that
    /// lets a restored card find the session it had rather than start a
    /// new one.
    pub session: Option<String>,
    /// Whether the program in this pane speaks the kitty keyboard protocol.
    /// Mirrored from the body every frame and SAVED, because a program
    /// announces itself only at startup and an adopted card's startup is
    /// long out of the ring. See `saved_layout::SavedCard::kitty_keys`.
    pub kitty_keys: bool,
    /// The title the program in this card set through the terminal (OSC 0
    /// or 2). Runtime-only and NEVER `title`: that is a name somebody chose
    /// and a program must not overwrite it, which is what LAYOUT_VERSION 2
    /// was about. Used for the label only while an agent is in the card,
    /// where the title is the session's name and a plain shell's is noise.
    pub osc_title: Option<String>,
    /// The buffer differs from the file on disk. Runtime-only.
    pub dirty: bool,
    /// Where an editor puts the caret when it opens. Runtime-only, consumed once.
    pub line: Option<u64>,
    /// The grammar an editor loaded, for the badge. Runtime-only.
    pub language: Option<String>,
    /// An editor that refuses edits (the generated config files). Runtime-only.
    pub read_only: bool,
    /// A decoy is drawn over this card (`card.mask`): somebody is reading
    /// your screen. Runtime-only; a restart shows the card.
    pub masked: bool,
    /// `ift attach` took this card's session from under the app. The shell
    /// runs on; the card shows so and takes no keys, and `terminals.rs`
    /// takes the session back when that client lets go. Runtime-only.
    pub displaced: bool,
    pub rect: Rect,
    pub z: f64,
    /// A name somebody CHOSE. Never seeded from the directory or the OSC
    /// title; the label is derived (`card_label.rs`).
    pub title: String,
    pub cwd: String,
    pub pane_id: Option<PaneId>,
    /// Epoch ms of the last output chunk; drives the activity heuristic.
    pub last_output_at: f64,
    /// Authoritative when hooks report; `None` falls back to the heuristic.
    pub agent: AgentState,
    pub last_event_at: f64,
    /// Epoch ms of the last Notification; changing it retriggers the blink.
    pub notified_at: f64,
    /// Set when the shell could not start, so the card can show why.
    pub error: Option<String>,
    /// The remote session this shell sits in, from the process table.
    pub remote: Option<String>,
    /// The foreground process, from the process table.
    pub proc: Option<String>,
    /// Which canvas this card is on. Empty only between creation and
    /// `ensure_workspace`.
    pub workspace_id: String,
    /// Membership lives on the CARD so the group's frame is derived.
    pub group_id: Option<String>,
    /// The soft group: a token shared with the cards this one was split from
    /// or into. Its one use is `reclaim` on close.
    pub soft_group_id: Option<String>,
    /// The card this one was split from: the one hint `reclaim` takes.
    pub split_from: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Phantom {
    pub rect: Rect,
    pub group_id: Option<String>,
}

/// Which card is active, and the modes that sit on top of that.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Selection {
    pub focused_id: Option<String>,
    /// Maximise shows whichever card is FOCUSED rather than remembering its
    /// own id, so moving focus while maximised flips through cards at full
    /// size, the way tmux zoom plus pane switching behaves.
    pub maximized: bool,
    /// An empty slot the arrows landed on: a hollow card, filled by Enter.
    /// Exclusive with a focused card, which is what lets Enter mean "make
    /// the card" rather than reach a shell.
    pub phantom: Option<Phantom>,
    /// Cards selected BESIDES the focused one, in the order added.
    pub extra: Vec<String>,
    /// Empty slots selected besides the current phantom.
    pub phantom_extra: Vec<Phantom>,
    /// Slot-picking mode: every empty slot around the cards, lettered.
    pub slot_picks: Vec<Slot>,
    /// Hint mode: a letter on every card on this canvas, by card id.
    pub hints: HashMap<String, char>,
}

/// One-shot work for the ui crate, pushed by commands and drained after each.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    AnimatePan {
        x: f64,
        y: f64,
    },
    AnimateZoom {
        scale: f64,
        anchor_world: Point,
        anchor_screen: Point,
    },
    AnimateFit(Viewport),
    /// Direct manipulation or a fit began; any in-flight animation stops.
    CancelAnimation,
    KillPane(PaneId),
    KillAllPanes,
    ClearPane(PaneId),
    WritePane(PaneId, Vec<u8>),
    DraftDelete(String),
    /// These cards are about to move: their rects as they are now, so the
    /// ui can glide each from there to wherever the command put it.
    MarkSwap(Vec<(String, Rect)>),
    OpenUrl(String),
    /// Written into settings.json through `patch_json_text`.
    SaveSetting {
        path: String,
        value: Value,
    },
    LoadTheme(String),
    RefreshThemes,
    /// Onto the system clipboard. The model never touches it directly:
    /// only the ui has an App to write through.
    Copy(String),
    /// The omnibox asked for completions. Answered off the UI thread; a
    /// response for a superseded query_id is dropped by the model.
    FetchSuggestions {
        query_id: u64,
        query: String,
    },
    /// An action the editor body owns; the card frame forwards it.
    Editor {
        card_id: String,
        action: EditorAction,
    },
    /// An action the page owns, for the same reason.
    Browser {
        card_id: String,
        action: BrowserAction,
    },
    /// Find in page. A None request ends the search and clears every
    /// highlight, which must happen whenever the bar closes.
    Find {
        card_id: String,
        request: Option<FindRequest>,
    },
    Log(String),
    Warn(String),
    /// A line for `agent.log`. The staleness sweep writes here rather than
    /// to stderr: an installed app has no terminal, so a card going
    /// colourless left no trace anywhere, which is exactly the event
    /// somebody needs to see.
    AgentLog(String),
    Reload,
    /// Quit and come back: the ui saves, arranges for something outside the
    /// process to reopen the bundle once it is gone, and exits. The window
    /// frame is already in `window.json`, so size and position return on
    /// their own.
    Restart,
    /// Open macOS's Emoji & Symbols panel. What it inserts comes back
    /// through the ui's input handler, not as a key.
    ShowCharacterPalette,
    /// A palette entry or an ift verb that runs a registered command: the
    /// registry lives outside the model, so the ui runs it.
    RunCommand(String),
    /// `dev.stress.zoom` step `n` began: the ui logs its frame rate.
    LogFps(u32),
    /// `dev.stress.dims`: the ui logs every terminal's grid and cell size.
    LogDims,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorAction {
    Save,
    Find,
    GoToLine,
    ToggleBlame,
    ToggleExplorer,
}

/// What a browser card's page is asked to do. History is the page's own,
/// not the model's: only the surface knows where it has been.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrowserAction {
    Back,
    Forward,
    Reload,
}

/// Find in page. `next` false is a new search, true steps through the one
/// already running; `forward` is the direction of that step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FindRequest {
    pub text: String,
    pub forward: bool,
    pub next: bool,
}

/// What a prompt's answer is for. Handed back with the answer by `settle`.
#[derive(Clone, Debug, PartialEq)]
pub enum Pending {
    RenameCard(String),
    NameGroup(Vec<String>),
    RenameGroup(String),
    RenameWorkspace(String),
    CloseWorkspace(String),
    /// A single browser card with more than one tab, asked before `Cmd+W`
    /// takes them all at once; `reclaim` carries `card.close`'s own meaning
    /// through the round trip (`card.close.leave` says no).
    CloseCard {
        id: String,
        reclaim: bool,
    },
    OpenFile {
        from: Option<String>,
    },
    /// A url for a new browser card beside `from`, or for the phantom.
    NewBrowserUrl {
        from: Option<String>,
        fill_phantom: bool,
    },
    NavigateBrowser(String),
    /// An untitled editor's first save: the path typed becomes the card's,
    /// then the save runs.
    SaveAs(String),
    /// `editor.goToLine`: the number typed lands in `card.line` for the body.
    GoToLine(String),
}

/// How long a notice stays up after the last one.
pub const NOTICE_MS: f64 = 5000.;

pub struct Model {
    pub cards: Vec<Card>,
    pub groups: Vec<Group>,
    pub workspaces: Vec<Workspace>,
    pub active_workspace: Option<String>,
    pub selection: Selection,
    /// The card last focused inside each group (`UNGROUPED` for the loose
    /// set), so stepping back into a group returns to where you were.
    pub last_focused: HashMap<String, String>,
    /// Every card focused this session, oldest first, each once. Closing
    /// the focused card goes back along it; not saved, a restart has no
    /// "before".
    pub focus_trail: Vec<String>,
    /// The next card's `number`; set past the highest loaded one.
    pub next_number: u32,
    pub viewport: Viewport,
    /// The CONTENT area, not the window.
    pub view_size: Size,
    /// Whether the view is FRAMING one card (`canvas.zoom.fitCard`): while
    /// it holds, moving focus re-fits the view instead of nudging.
    pub framing: bool,
    /// The scale an in-flight zoom is heading for, set by the animator, so a
    /// held key ramps from the target rather than the passing scale.
    pub pending_scale: Option<f64>,
    pub ui_scale: f64,
    pub usage: Usage,
    pub palette: PaletteState,
    pub prompt: crate::prompt::Prompt<Pending>,
    pub shortcuts_open: bool,
    pub omni: omni_cmd::OmniState,
    pub find: find_cmd::FindState,
    /// Where browser cards have been. Loaded once at startup and written
    /// back debounced; the model only records into it.
    pub history: crate::omni::history::History,
    pub notice: Option<String>,
    notice_until: f64,
    /// Saved-layout state: nothing is saved until `loaded`; nothing is ever
    /// saved while `read_only` (the file was written by a newer build).
    pub loaded: bool,
    pub read_only: bool,
    pub dirty_layout: bool,
    pub config: Config,
    pub keymap: Keymap,
    pub settings_error: Option<String>,
    pub theme_names: Vec<String>,
    pub theme_current: Option<String>,
    pub theme_before_preview: Option<String>,
    /// Where the FIRST card starts; every card after inherits from the one it
    /// was opened next to. The home directory, or `startingDir`.
    pub start_dir: String,
    pub home: String,
    pub dev_build: bool,
    /// Config pairs that are open: both ids and the card to return to.
    pub config_pairs: Vec<ConfigPair>,
    /// The last few cards closed, newest last, so a close can be undone.
    /// Runtime-only: a card nobody reopened before a quit is gone, the same
    /// as every other runtime fact. Cheap closing is what keeps a canvas
    /// from silting up, and it is only cheap if it is undoable.
    pub closed: Vec<Card>,
    /// Closing a dirty editor takes two presses; the first says why.
    discard_armed: Option<(String, f64)>,
    /// `dev.stress.zoom` in progress: the step taken so far and when the
    /// next is due. Steps run from `tick`, which is the model's only clock.
    pub stress_zoom: Option<(u32, f64)>,
    pub now_ms: f64,
    pub effects: Vec<Effect>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigPair {
    pub ids: Vec<String>,
    pub return_to: Option<String>,
}

pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

impl Default for Model {
    fn default() -> Self {
        Model::new()
    }
}

impl Model {
    pub fn new() -> Model {
        Model {
            cards: vec![],
            groups: vec![],
            workspaces: vec![],
            active_workspace: None,
            selection: Selection::default(),
            last_focused: HashMap::new(),
            focus_trail: Vec::new(),
            next_number: 1,
            viewport: INITIAL_VIEWPORT,
            view_size: Size { w: 0., h: 0. },
            framing: false,
            pending_scale: None,
            ui_scale: 1.,
            usage: Usage::default(),
            palette: PaletteState::default(),
            prompt: crate::prompt::Prompt::default(),
            shortcuts_open: false,
            omni: omni_cmd::OmniState::default(),
            find: find_cmd::FindState::default(),
            history: crate::omni::history::History::default(),
            notice: None,
            notice_until: 0.,
            loaded: false,
            read_only: false,
            dirty_layout: false,
            config: default_config(),
            keymap: default_keymap(),
            settings_error: None,
            theme_names: vec![],
            theme_current: None,
            theme_before_preview: None,
            start_dir: "/".into(),
            home: String::new(),
            dev_build: cfg!(debug_assertions),
            config_pairs: vec![],
            closed: vec![],
            discard_armed: None,
            stress_zoom: None,
            now_ms: 0.,
            effects: vec![],
        }
    }

    /// The ui calls this once per frame with the wall clock in epoch ms.
    /// Notices expire here; the staleness sweep is `sweep_stale`.
    pub fn tick(&mut self, now_ms: f64) {
        self.now_ms = now_ms;
        if self.notice.is_some() && now_ms >= self.notice_until {
            self.notice = None;
        }
        if self
            .discard_armed
            .as_ref()
            .is_some_and(|(_, until)| now_ms >= *until)
        {
            self.discard_armed = None;
        }
        if let Some((n, due)) = self.stress_zoom {
            if now_ms >= due {
                self.stress_zoom_step(n, now_ms);
            }
        }
    }

    /// The notice's time is up: the next tick clears it, so a frame is due.
    /// A notice on screen needs no other frame; it does not move.
    pub fn notice_expired(&self, now_ms: f64) -> bool {
        self.notice.is_some() && now_ms >= self.notice_until
    }

    pub fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }

    /// A line of transient text in the status bar. Anything that refuses to
    /// do something says why here: a command that silently does nothing is
    /// indistinguishable from an unbound key. The timer restarts, so holding
    /// a key leaves the notice up throughout.
    pub fn notify(&mut self, text: impl Into<String>) {
        self.notice = Some(text.into());
        self.notice_until = self.now_ms + NOTICE_MS;
    }

    /// Arms a move animation for `ids`, BEFORE their rects change.
    pub fn mark_swap(&mut self, ids: &[String]) {
        let rects: Vec<(String, Rect)> = ids
            .iter()
            .filter_map(|id| self.card(id).map(|c| (id.clone(), c.rect)))
            .collect();
        self.effects.push(Effect::MarkSwap(rects));
    }

    pub fn log(&mut self, line: impl Into<String>) {
        self.effects.push(Effect::Log(line.into()));
    }

    // ----- cards -----------------------------------------------------------

    pub fn card(&self, id: &str) -> Option<&Card> {
        self.cards.iter().find(|c| c.id == id)
    }

    pub fn card_mut(&mut self, id: &str) -> Option<&mut Card> {
        self.cards.iter_mut().find(|c| c.id == id)
    }

    pub fn focused(&self) -> Option<&Card> {
        self.selection
            .focused_id
            .as_deref()
            .and_then(|id| self.card(id))
    }

    /// The cards on one workspace, in their stored order.
    pub fn cards_on(&self, workspace_id: Option<&str>) -> Vec<&Card> {
        match workspace_id {
            Some(ws) => self.cards.iter().filter(|c| c.workspace_id == ws).collect(),
            None => vec![],
        }
    }

    pub fn here(&self) -> Vec<&Card> {
        self.cards_on(self.active_workspace.as_deref())
    }

    pub fn placed(&self, cards: &[&Card]) -> Vec<crate::cards::PlacedCard> {
        cards
            .iter()
            .map(|c| crate::cards::PlacedCard {
                id: c.id.clone(),
                rect: c.rect,
                group_id: c.group_id.clone(),
            })
            .collect()
    }

    /// The fixed default card, in world units at scale 1: a new card is the
    /// same size whether you were zoomed out or not.
    pub fn default_size(&self) -> Size {
        default_size(
            Size {
                w: self.config.cards.width,
                h: self.config.cards.height,
            },
            self.view_size,
        )
    }

    /// The free slot nearest the active card (`after`), so a new card opens
    /// beside the one it came from wherever that is; the window's shape
    /// decides when a row ends (`layout::nearest_free_slot`). `origin`
    /// moves the grid's anchor (a card joining a group starts from its
    /// siblings). Only cards on the SAME canvas are in the way.
    fn next_slot(
        &self,
        origin: Option<Point>,
        avoid: &[Rect],
        workspace_id: &str,
        after: Option<Rect>,
    ) -> Rect {
        let size = self.default_size();
        let origin = origin.unwrap_or(Point {
            x: HALF_CELL,
            y: HALF_CELL,
        });
        let mut taken: Vec<Rect> = self
            .cards
            .iter()
            .filter(|c| c.workspace_id == workspace_id)
            .map(|c| c.rect)
            .collect();
        taken.extend_from_slice(avoid);
        let aspect = if self.view_size.h > 0. {
            self.view_size.w / self.view_size.h
        } else {
            1.8
        };
        nearest_free_slot(&taken, size, origin, GUTTER, after, aspect)
    }

    pub fn add_card(&mut self, cwd: &str, opts: NewCard) -> String {
        let workspace_id = opts.workspace_id.unwrap_or_default();
        // Start the slot scan at the group's existing cards, so a new member
        // lands beside its siblings.
        let siblings: Vec<Rect> = match &opts.group_id {
            Some(g) => self
                .cards
                .iter()
                .filter(|c| c.group_id.as_ref() == Some(g))
                .map(|c| c.rect)
                .collect(),
            None => vec![],
        };
        let origin = bounding_rect(&siblings).map(|b| Point { x: b.x, y: b.y });
        let rect = opts
            .rect
            .unwrap_or_else(|| self.next_slot(origin, &opts.avoid, &workspace_id, opts.after));
        let card = Card {
            id: opts.id.unwrap_or_else(new_id),
            kind: opts.kind,
            path: opts.path,
            root: opts.root,
            explorer: opts.explorer,
            sidebar: opts.sidebar,
            sidebar_top: opts.sidebar_top,
            url: opts.url,
            zoom: opts.zoom,
            tabs: vec![],
            active_tab: 0,
            closed_tabs: vec![],
            locked: false,
            command: opts.command,
            transcript_path: None,
            agent_session: None,
            number: self.take_number(),
            session: None,
            kitty_keys: false,
            osc_title: None,
            dirty: false,
            line: opts.line,
            language: None,
            read_only: false,
            masked: false,
            displaced: false,
            rect,
            z: self.cards.len() as f64,
            title: String::new(),
            cwd: cwd.into(),
            pane_id: None,
            last_output_at: 0.,
            agent: AgentState::None,
            last_event_at: 0.,
            notified_at: 0.,
            error: None,
            remote: None,
            proc: None,
            workspace_id,
            group_id: opts.group_id,
            soft_group_id: opts.soft_group_id,
            split_from: opts.split_from,
        };
        let id = card.id.clone();
        self.cards.push(card);
        self.dirty_layout = true;
        id
    }

    /// The next card number, never handed out twice in a session.
    pub fn take_number(&mut self) -> u32 {
        let n = self.next_number;
        self.next_number += 1;
        n
    }

    /// The one door for a mask. The ui reconciles its decoy panes from
    /// the flag each frame (`decoys.rs`), the way terminal bodies follow
    /// the cards, so there is no effect to keep in step with it.
    pub fn set_mask(&mut self, id: &str, on: bool) {
        if let Some(card) = self.card_mut(id) {
            card.masked = on;
        }
    }

    pub fn remove_card(&mut self, id: &str) {
        self.cards.retain(|c| c.id != id);
        self.focus_trail.retain(|t| t != id);
        self.dirty_layout = true;
    }

    // ----- selection -------------------------------------------------------

    /// How far back a close can walk. The trail is pruned as cards close,
    /// so this bounds memory, not the reach.
    const FOCUS_TRAIL: usize = 64;

    /// The one door for a plain focus change. A plain move of focus empties
    /// the extras (the text-field rule: Shift+Arrow grows a selection, a
    /// plain arrow collapses it); a focused card dismisses any phantom and
    /// slot picking; the card is remembered as its group's last focus.
    pub fn set_focus(&mut self, id: Option<&str>) {
        self.selection.extra.clear();
        self.land_focus(id);
    }

    /// Focus moved by EXTENDING: the extras are kept as the caller set them.
    pub fn focus_extended(&mut self, id: &str, extra: Vec<String>) {
        self.selection.extra = extra;
        self.land_focus(Some(id));
    }

    fn land_focus(&mut self, id: Option<&str>) {
        // A find bar belongs to the card it was opened on, the way a
        // browser's belongs to its tab: looking at something else closes it
        // and clears the highlights behind it.
        if self.find.open && self.find.card_id.as_deref() != id {
            self.close_find();
        }
        self.selection.focused_id = id.map(String::from);
        if let Some(id) = id {
            self.selection.phantom = None;
            self.selection.phantom_extra.clear();
            self.selection.slot_picks.clear();
            if let Some(card) = self.card(id) {
                let key = card
                    .group_id
                    .clone()
                    .unwrap_or_else(|| UNGROUPED.to_string());
                self.last_focused.insert(key, id.to_string());
                self.focus_trail.retain(|t| t != id);
                self.focus_trail.push(id.to_string());
                if self.focus_trail.len() > Self::FOCUS_TRAIL {
                    self.focus_trail.remove(0);
                }
            }
        }
        self.dirty_layout = true;
    }

    /// The selection: the focused card and everything extending added.
    pub fn selected(&self) -> Vec<&Card> {
        let here = self.here();
        let placed = self.placed(&here);
        crate::multi_select::selected_cards(
            &placed,
            self.selection.focused_id.as_deref(),
            &self.selection.extra,
        )
        .into_iter()
        .filter_map(|p| self.card(&p.id))
        .collect()
    }

    pub fn selected_ids(&self) -> Vec<String> {
        self.selected().into_iter().map(|c| c.id.clone()).collect()
    }

    // ----- groups ----------------------------------------------------------

    pub fn group(&self, id: &str) -> Option<&Group> {
        self.groups.iter().find(|g| g.id == id)
    }

    pub fn add_group(&mut self, name: &str) -> String {
        let id = new_id();
        self.groups.push(Group {
            id: id.clone(),
            name: name.into(),
        });
        self.dirty_layout = true;
        id
    }

    /// A group's frame, derived from its members on `workspace_id`.
    pub fn group_frame(&self, group_id: &str, workspace_id: &str) -> Option<Rect> {
        let rects: Vec<Rect> = self
            .cards
            .iter()
            .filter(|c| c.group_id.as_deref() == Some(group_id) && c.workspace_id == workspace_id)
            .map(|c| c.rect)
            .collect();
        group_bounds(&rects, GROUP_PAD)
    }

    /// Every group's frame except one, on one canvas: what placement treats
    /// as occupied so a growing group does not walk through its neighbours.
    pub fn other_frames(&self, except: Option<&str>, workspace_id: &str) -> Vec<Rect> {
        self.groups
            .iter()
            .filter(|g| Some(g.id.as_str()) != except)
            .filter_map(|g| self.group_frame(&g.id, workspace_id))
            .collect()
    }

    /// Deletes a group and releases its cards where they stand: positions
    /// are permanent, and a group labels cards rather than owning them.
    pub fn remove_group(&mut self, id: &str) {
        for card in &mut self.cards {
            if card.group_id.as_deref() == Some(id) {
                card.group_id = None;
            }
        }
        self.groups.retain(|g| g.id != id);
        self.dirty_layout = true;
    }

    /// Drops groups nobody belongs to: no frame to draw, no way to select.
    /// Called after the operations that can empty a group.
    pub fn prune_empty_groups(&mut self) {
        let live: Vec<String> = self
            .cards
            .iter()
            .filter_map(|c| c.group_id.clone())
            .collect();
        self.groups.retain(|g| live.contains(&g.id));
    }

    // ----- workspaces ------------------------------------------------------

    pub fn active_ws(&self) -> Option<&Workspace> {
        self.workspaces
            .iter()
            .find(|w| Some(&w.id) == self.active_workspace.as_ref())
    }

    pub fn add_workspace(&mut self, name: Option<&str>) -> String {
        let existing: Vec<String> = self.workspaces.iter().map(|w| w.name.clone()).collect();
        let name = match name.map(str::trim).filter(|n| !n.is_empty()) {
            Some(n) => n.to_string(),
            None => next_name(&existing, "workspace"),
        };
        let id = new_id();
        self.workspaces.push(Workspace {
            id: id.clone(),
            name,
            viewport: INITIAL_VIEWPORT,
            focused: None,
        });
        self.dirty_layout = true;
        id
    }

    /// Shows a workspace, saving where you were looking in the one you leave.
    /// The viewport is one thing every workspace takes a turn owning. Focus
    /// belongs to a workspace too: a focused card on a canvas you cannot see
    /// is a keystroke going somewhere invisible.
    pub fn show_workspace(&mut self, id: &str) {
        if self.active_workspace.as_deref() == Some(id) {
            return;
        }
        let vp = self.viewport;
        // What the workspace being left remembers: where the view was, and
        // which card had the focus, if it was one of its own.
        let leaving_focus = self
            .focused()
            .filter(|c| Some(c.workspace_id.as_str()) == self.active_workspace.as_deref())
            .map(|c| c.id.clone());
        if let Some(leaving) = self
            .active_workspace
            .clone()
            .and_then(|a| self.workspaces.iter_mut().find(|w| w.id == a))
        {
            leaving.viewport = vp;
            leaving.focused = leaving_focus;
        }
        let Some(entering) = self.workspaces.iter().find(|w| w.id == id) else {
            return;
        };
        self.viewport = entering.viewport;
        let remembered = entering.focused.clone();
        self.active_workspace = Some(id.to_string());
        self.framing = false;
        self.effects.push(Effect::CancelAnimation);
        let here = self.focused().is_some_and(|c| c.workspace_id == id);
        if !here {
            // The card that was focused here last time, if it still is
            // here; else the first one, as before.
            let back = remembered
                .filter(|f| {
                    self.cards
                        .iter()
                        .any(|c| &c.id == f && c.workspace_id == id)
                })
                .or_else(|| {
                    self.cards
                        .iter()
                        .find(|c| c.workspace_id == id)
                        .map(|c| c.id.clone())
                });
            self.set_focus(back.as_deref());
        }
        self.selection.maximized = false;
        self.dirty_layout = true;
    }

    /// Ensures there is somewhere to put cards, and that every card is
    /// somewhere. A file from before workspaces has cards with no workspace;
    /// a hand-edited one can point a card at a workspace that is gone.
    pub fn ensure_workspace(&mut self) {
        if self.workspaces.is_empty() {
            self.add_workspace(Some("workspace 1"));
        }
        let ids: Vec<String> = self.workspaces.iter().map(|w| w.id.clone()).collect();
        if !self
            .active_workspace
            .as_ref()
            .is_some_and(|a| ids.contains(a))
        {
            self.active_workspace = Some(ids[0].clone());
            self.viewport = self.workspaces[0].viewport;
        }
        for card in &mut self.cards {
            if card.workspace_id.is_empty() || !ids.contains(&card.workspace_id) {
                card.workspace_id = ids[0].clone();
            }
        }
    }

    /// Removes a workspace, returning the cards to close: killing a shell is
    /// the caller's job, and a card on no workspace is invisible and running.
    pub fn remove_workspace(&mut self, id: &str) -> Vec<String> {
        let doomed: Vec<String> = self
            .cards
            .iter()
            .filter(|c| c.workspace_id == id)
            .map(|c| c.id.clone())
            .collect();
        self.workspaces.retain(|w| w.id != id);
        self.dirty_layout = true;
        doomed
    }

    /// The live viewport IS the active workspace's; written back before a save.
    pub fn sync_workspace_viewport(&mut self) {
        let vp = self.viewport;
        if let Some(ws) = self
            .active_workspace
            .clone()
            .and_then(|a| self.workspaces.iter_mut().find(|w| w.id == a))
        {
            ws.viewport = vp;
        }
    }
}

/// Options for `Model::add_card`; `Default` is a terminal wherever the scan
/// puts it.
#[derive(Clone, Debug, Default)]
pub struct NewCard {
    pub rect: Option<Rect>,
    pub group_id: Option<String>,
    pub workspace_id: Option<String>,
    /// A restored card's saved id, set at creation: the id is what agent
    /// hooks address through INFINITERM_CARD_ID.
    pub id: Option<String>,
    /// Occupied space beyond the cards: the frames of the OTHER groups.
    pub avoid: Vec<Rect>,
    pub soft_group_id: Option<String>,
    pub split_from: Option<String>,
    pub kind: CardKind,
    pub path: Option<String>,
    pub root: Option<String>,
    pub explorer: bool,
    pub sidebar: Option<f64>,
    pub sidebar_top: bool,
    pub url: Option<String>,
    pub zoom: Option<f64>,
    pub command: Option<String>,
    pub line: Option<u64>,
    /// The card this one is opened from: placement starts just past it.
    pub after: Option<Rect>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_card_starts_with_no_tabs_and_unlocked() {
        let mut m = Model::new();
        m.home = "/h".into();
        m.start_dir = "/h".into();
        let id = m.add_card("/h", NewCard::default());
        let card = m.card(&id).unwrap();
        assert!(card.tabs.is_empty());
        assert_eq!(card.active_tab, 0);
        assert!(card.closed_tabs.is_empty());
        assert!(!card.locked);
    }
}
