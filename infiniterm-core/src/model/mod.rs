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
pub mod color_cmd;
pub mod context;
pub mod cover;
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
use crate::cards::default_size;
use crate::config::{default_config, Config};
use crate::grid::{Point, Rect, Size, HALF_CELL};
use crate::groups::{group_bounds, UNGROUPED};
use crate::keymap::{default_keymap, Keymap};
use crate::layout::block_slot;
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
    /// The agent that session belongs to, by its adapter's name; `None` is
    /// Claude Code. SAVED (`agentKind`) beside `agent_session`.
    pub agent_kind: Option<String>,
    /// `card.protect` (Cmd+Shift+L): the card cannot be closed, by Cmd+W,
    /// by its workspace closing, or by its shell exiting (a fresh shell
    /// takes the pane instead). SAVED (`protected`); the label wears a lock
    /// and a different ground. Not `locked`, which is the keyboard's.
    pub protected: bool,
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

impl Card {
    /// An agent has run here: this session's hooks gave a transcript, or
    /// the save file kept its session id. The transcript alone is runtime,
    /// so after a relaunch an idle Claude card lost its name to the
    /// directory until its next hook (2026-09-25); the saved id holds.
    pub fn runs_agent(&self) -> bool {
        self.transcript_path.is_some() || self.agent_session.is_some()
    }
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

/// A focus, from the moment it landed until it leaves or earns its place.
#[derive(Clone, Debug, PartialEq)]
pub struct FocusVisit {
    pub id: String,
    pub since: f64,
    pub earned: bool,
    /// Something was typed into this card while it held the focus.
    pub typed: bool,
}

/// The card switcher's state while it is up: the rows it offered when it
/// opened (frozen, so stepping cannot shuffle under the hand) and where
/// the selection is.
#[derive(Clone, Debug, PartialEq)]
pub struct Switcher {
    pub list: Vec<String>,
    pub index: usize,
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
    /// A parked card was reopened: let go of its pane (it stays running)
    /// and list its session as live, so the new body adopts it and the
    /// ring's replay redraws the screen the card missed while closed.
    ReleasePane(PaneId),
    KillAllPanes,
    /// `app.keycast`: the ui owns the overlay and its clock.
    ToggleKeycast,
    /// `app.fullscreen`: the ui sends `toggleFullScreen:` to the window,
    /// the message the green button sends, so both take one path.
    ToggleFullScreen,
    /// `app.update.check`: ask the updater thread for a check now.
    CheckForUpdate,
    /// `app.update.install`: swap in the staged update and restart.
    InstallUpdate,
    /// Re-read the snippets folder into `Model::snippets`, seeding it when
    /// missing; before the snippet picker opens.
    RefreshSnippets,
    /// Paste `text` into the card's body as Cmd+V would: bracketed when the
    /// program asked for it, inserted at the caret in an editor.
    PasteText {
        card_id: String,
        text: String,
    },
    ClearPane(PaneId),
    WritePane(PaneId, Vec<u8>),
    DraftDelete(String),
    /// These cards are about to move: their rects as they are now, so the
    /// ui can glide each from there to wherever the command put it.
    MarkSwap(Vec<(String, Rect)>),
    /// Cmd+E on a terminal: the ui reads its selection and calls
    /// `Model::find_with`.
    FindSelection(String),
    /// "Editor: open a file": the ui shows the macOS open panel (#64) and
    /// hands the pick to `Model::open_picked`, beside `from`.
    PickFile {
        from: Option<String>,
    },
    /// Cmd+Shift+C: visual mode on this terminal card; the grid is the
    /// body's, so the ui turns it on (`Grid::visual_enter`).
    Visual(String),
    /// The window's colour changed (`window.color`): the ui redraws the Dock
    /// icon (no colour is the plain icon) and, in a remote instance, for
    /// `Save` or `Forget`, writes or removes `remote.json`. A local choice is
    /// saved by the model as the setting `ui.windowColor`.
    WindowColor {
        color: Option<crate::remote_identity::Rgb>,
        save: WindowColorSave,
    },
    /// `ift read`: the ui answers request `request_id` with the card's text,
    /// since only the terminal body holds its grid (#127).
    ReadCard {
        request_id: u64,
        card_id: String,
        last: Option<usize>,
        scrollback: bool,
    },
    /// The late answer to an `ift` request (`edit`, when its cover closes).
    CliReply {
        id: u64,
        ok: bool,
        text: String,
    },
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
    /// Leave the text: the lock is dropped and the card is an ordinary card
    /// on the canvas again (`browser.leave`, #265).
    Unlock,
    /// A palette-only text transform (Batch 1, 2026-09-24): no chord of its
    /// own, reachable only through the command palette, `cards_cmd.rs`'s
    /// `editor.transform.*` entries.
    Transform(TextTransform),
}

/// A pure function in `infiniterm_editor::transforms`, picked by name here
/// since the model crate cannot depend on the ui crate that runs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextTransform {
    Upper,
    Lower,
    Title,
    Snake,
    Kebab,
    Camel,
    SortLines,
    UniqueLines,
    ReverseLines,
    TrimTrailingWhitespace,
    IndentTabsToSpaces,
    IndentSpacesToTabs,
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
    /// Closing cards with unsaved changes: the ids and `reclaim` to close
    /// with, after a save or without one.
    UnsavedClose {
        ids: Vec<String>,
        reclaim: bool,
    },
    NavigateBrowser(String),
    /// Naming an untitled buffer (`open_save_as`); `then_close` is the
    /// close the save sheet's Save is waiting to finish.
    SaveAs {
        id: String,
        then_close: Option<(Vec<String>, bool)>,
    },
    /// The typed path is a file already: replace it?
    SaveAsReplace {
        id: String,
        path: String,
        then_close: Option<(Vec<String>, bool)>,
    },
    /// The About window (`app.about`): yes is "Check for Updates".
    About,
    /// The custom hex colour of a window (`window.color`).
    WindowColor,
    /// `editor.goToLine`: the number typed lands in `card.line` for the body.
    GoToLine(String),
}

/// What a colour change asks to be saved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowColorSave {
    /// A preview while the picker is open: nothing is written.
    No,
    Save,
    /// Back to the default: the host's hashed colour, or none.
    Forget,
}

/// How long a notice stays up after the last one.
pub const NOTICE_MS: f64 = 5000.;

/// How long a done card is looked at before it counts as seen: the
/// switcher's dwell, so a card walked past is not a card read.
pub const SEEN_MS: f64 = crate::switcher::TRAIL_DWELL_MS;

/// Every card's rect on one canvas at one moment. Cards closed since are
/// skipped on the way back; cards made since are left where they are.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutSnapshot {
    pub workspace_id: String,
    pub rects: Vec<(String, Rect)>,
}

/// One step of the canvas's undo trail (Cmd+Z / Cmd+Shift+Z): a move or
/// resize, or a close. Undoing a close reopens the card in its slot (a
/// fresh shell there, the way `card.reopen` does). One trail, in the order
/// things happened, because a close among moves is the thing worth walking
/// back.
///
/// Undo never CLOSES a card. Making a card was a step from 2026-09-21 to
/// 09-26, so Cmd+Z on a card you had opened and started working in closed
/// it, shell and all (Ekin: "pretty dangerous"); an unwanted new card is a
/// Cmd+W away. For the same reason there is no redo of a close.
#[derive(Clone, Debug, PartialEq)]
pub enum UndoStep {
    Rects(LayoutSnapshot),
    /// The card as `close_card` kept it: runtime facts stripped. Boxed
    /// because a Card is ten times a snapshot and the trail is a Vec.
    Closed(Box<Card>),
}

/// How many layout changes Cmd+Z can walk back.
pub const LAYOUT_UNDO_DEPTH: usize = 100;

pub struct Model {
    pub cards: Vec<Card>,
    pub groups: Vec<Group>,
    pub workspaces: Vec<Workspace>,
    pub active_workspace: Option<String>,
    pub selection: Selection,
    /// The card last focused inside each group (`UNGROUPED` for the loose
    /// set), so stepping back into a group returns to where you were.
    pub last_focused: HashMap<String, String>,
    /// The window is the key window; the ui sets it before each tick. A
    /// done card is only SEEN while the app is in front.
    pub app_active: bool,
    /// The focused done card and since when it has been watched, with the
    /// app in front (`clear_seen_done`). Session-only.
    pub done_watch: Option<(String, f64)>,
    /// Every card focused this session, oldest first, each once. Closing
    /// the focused card goes back along it; not saved, a restart has no
    /// "before".
    pub focus_trail: Vec<String>,
    /// In-place editors (`cover.rs`): cover card id to the terminal card it
    /// lies over, where each pair was last synced, and the `ift` request
    /// each one answers when it closes. Session-only.
    pub covers: HashMap<String, String>,
    pub cover_at: HashMap<String, Rect>,
    pub edit_waiters: HashMap<String, u64>,
    /// Set by an `ift` verb that answers later (`edit`): the ui must not
    /// reply to this request now.
    pub reply_deferred: bool,
    /// A close waiting for its Save to land (`save_then_close`): the ids,
    /// `reclaim`, and when to give up.
    pub close_after_save: Option<(Vec<String>, bool, f64)>,
    /// Each terminal card's escape-sequence scanner and the command in
    /// flight (`program_state`), by card id. Session-only: a command
    /// running across a restart is simply not known about.
    pub programs: std::collections::HashMap<
        String,
        (crate::program_state::Scanner, crate::program_state::Track),
    >,
    /// The focus as it stands: which card, when it took the focus, and
    /// whether that visit has earned a place in the trail yet
    /// (`switcher::earns_trail`). A card crossed with Cmd+Alt+Arrow on the
    /// way to another never earns one, which is what makes the switcher's
    /// first row the card you actually work in.
    pub focus_visit: Option<FocusVisit>,
    /// The card switcher (Ctrl+Tab), while it is up.
    pub switcher: Option<Switcher>,
    /// Layout undo (Cmd+Z / Cmd+Shift+Z): every card's rect on the active
    /// canvas as it was before each move, swap, drop, split or resize.
    /// Session-only. A snapshot is taken by `remember_layout` at the top
    /// of each of those; undo applies one and pushes the present onto
    /// `redo`, which a new change empties.
    pub layout_undo: Vec<UndoStep>,
    pub layout_redo: Vec<UndoStep>,
    /// Set while an undo or redo runs, so the close or the create it
    /// performs is not recorded as a new step.
    pub undoing: bool,
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
    /// Set when this instance runs its cards on a host (`ift connect`, #118):
    /// what the window shows, and the reason browser cards are refused.
    pub remote: Option<crate::remote_identity::RemoteIdentity>,
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
    /// No save file existed at load: the very first launch on this Mac (or
    /// in this data dir). It is what makes the first seed the welcome canvas
    /// rather than one bare terminal (`seed_first_card`, `welcome.rs`).
    pub first_run: bool,
    pub read_only: bool,
    pub dirty_layout: bool,
    pub config: Config,
    pub keymap: Keymap,
    pub settings_error: Option<String>,
    pub theme_names: Vec<String>,
    /// the snippets folder's files, read by the ui when the picker opens
    /// (`Effect::RefreshSnippets`), the way `theme_names` is.
    pub snippets: Vec<crate::snippets::Snippet>,
    pub theme_current: Option<String>,
    pub theme_before_preview: Option<String>,
    /// The colour the window had when its colour picker opened (`Some(None)`:
    /// none), which a dismissed picker puts back (`color_cmd.rs`).
    pub window_color_before_preview: Option<Option<crate::remote_identity::Rgb>>,
    /// The local instance's colour, from `ui.windowColor` (`None`: the normal
    /// title bar). A remote instance wears its host's instead.
    pub local_color: Option<crate::remote_identity::Rgb>,
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
    /// Closed terminal cards whose program is still running and watched
    /// (lifecycle.rs, `Parked`). Runtime-only: a relaunch's orphan sweep
    /// ends whatever is left.
    pub parked: Vec<lifecycle::Parked>,
    /// The backend can keep a closed card's program for a reopen (the
    /// daemon); set by the ui at startup. False in tests unless set.
    pub can_park: bool,
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

/// The lowest card number from 1 up that `used` does not hold. Pure, for
/// `Model::take_number`, the load and a reopened card.
pub fn lowest_free_number(used: &[u32]) -> u32 {
    (1..).find(|n| !used.contains(n)).unwrap_or(1)
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
            app_active: true,
            done_watch: None,
            focus_trail: Vec::new(),
            covers: HashMap::new(),
            cover_at: HashMap::new(),
            edit_waiters: HashMap::new(),
            reply_deferred: false,
            close_after_save: None,
            programs: std::collections::HashMap::new(),
            focus_visit: None,
            switcher: None,
            layout_undo: Vec::new(),
            layout_redo: Vec::new(),
            undoing: false,
            viewport: INITIAL_VIEWPORT,
            view_size: Size { w: 0., h: 0. },
            framing: false,
            pending_scale: None,
            ui_scale: 1.,
            usage: Usage::default(),
            palette: PaletteState::default(),
            prompt: crate::prompt::Prompt::default(),
            shortcuts_open: false,
            remote: None,
            omni: omni_cmd::OmniState::default(),
            find: find_cmd::FindState::default(),
            history: crate::omni::history::History::default(),
            notice: None,
            notice_until: 0.,
            loaded: false,
            first_run: false,
            read_only: false,
            dirty_layout: false,
            config: default_config(),
            keymap: default_keymap(),
            settings_error: None,
            theme_names: vec![],
            snippets: vec![],
            theme_current: None,
            theme_before_preview: None,
            window_color_before_preview: None,
            local_color: None,
            start_dir: "/".into(),
            home: String::new(),
            dev_build: cfg!(debug_assertions),
            config_pairs: vec![],
            closed: vec![],
            parked: vec![],
            can_park: false,
            stress_zoom: None,
            now_ms: 0.,
            effects: vec![],
        }
    }

    /// The ui calls this once per frame with the wall clock in epoch ms.
    /// Notices expire here; the staleness sweep is `sweep_stale`.
    pub fn tick(&mut self, now_ms: f64) {
        self.now_ms = now_ms;
        // A card sat in earns its place without waiting for the focus to
        // leave it: the trail is read by the switcher and by a close.
        self.promote_focus();
        self.promote_programs();
        self.clear_seen_done();
        self.end_parked();
        self.sync_covers();
        self.close_when_saved();
        if self.notice.is_some() && now_ms >= self.notice_until {
            self.notice = None;
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

    /// The cards on the active canvas you can see: a terminal under an
    /// in-place editor (`cover.rs`) is left out, so navigation, tidy and
    /// fit-all treat the pair as the one card on screen.
    /// The number the status bar shows: every card, or this workspace's
    /// with `ui.workspaceIsolation` (#101).
    pub fn card_count(&self) -> usize {
        if self.config.ui.workspace_isolation {
            self.here().len()
        } else {
            self.cards.len()
        }
    }

    /// A browser card in a remote instance: its page would load on THIS Mac,
    /// not through the host, which is not what the window says it is. Off
    /// until browsing through the server is built. True when it was refused.
    pub(super) fn browser_refused(&mut self) -> bool {
        if self.remote.is_none() {
            return false;
        }
        self.notify("browser cards are off in a remote instance");
        true
    }

    pub fn here(&self) -> Vec<&Card> {
        self.cards_on(self.active_workspace.as_deref())
            .into_iter()
            .filter(|c| self.covered_by(&c.id).is_none())
            .collect()
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
            crate::cards::parse_shape(&self.config.cards.shape).unwrap_or(Some(16. / 9.)),
        )
    }

    /// The first free slot in block order (`layout::block_slot`): a square
    /// block grown from the top-left, the same place whatever card is
    /// focused, holes first. `origin` moves the grid's anchor (a card
    /// joining a group starts from its siblings). Only cards on the SAME
    /// canvas are in the way.
    fn next_slot(&self, origin: Option<Point>, avoid: &[Rect], workspace_id: &str) -> Rect {
        let size = self.default_size();
        let anchor = Point {
            x: HALF_CELL,
            y: HALF_CELL,
        };
        let mut taken: Vec<Rect> = self
            .cards
            .iter()
            .filter(|c| c.workspace_id == workspace_id)
            .map(|c| c.rect)
            .collect();
        taken.extend_from_slice(avoid);
        match origin {
            // A card joining a group starts from its siblings.
            Some(origin) => block_slot(&taken, size, origin, self.gap()),
            None => crate::layout::fill_slot(&taken, size, anchor, self.gap()),
        }
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
            .unwrap_or_else(|| self.next_slot(origin, &opts.avoid, &workspace_id));
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
            agent_kind: None,
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
            protected: false,
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

    /// The number a new card gets: the lowest one no card has. It counted
    /// up forever until 2026-09-24, so a week of cards on Ekin's canvas
    /// would have read #2332; a closed card's number goes back into use.
    /// The price, taken knowingly: `#7` from last week may be another card
    /// now.
    pub fn take_number(&self) -> u32 {
        let used: Vec<u32> = self.cards.iter().map(|c| c.number).collect();
        lowest_free_number(&used)
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
        if self.focus_visit.as_ref().is_some_and(|v| v.id == id) {
            self.focus_visit = None;
        }
        if let Some(s) = &mut self.switcher {
            s.list.retain(|t| t != id);
            s.index = s.index.min(s.list.len().saturating_sub(1));
        }
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
        self.land_focus(id, true);
    }

    /// Focus moved by WALKING the canvas (Cmd+Alt+Arrow): the card is
    /// focused like any other, but the visit only reaches the switcher's
    /// list if it is stayed in (`switcher::earns_trail`). Ekin crosses
    /// three cards on the way to a fourth; a plain most-recent list filled
    /// with those and the one press that should return him to the card he
    /// was working in landed on a card he passed.
    pub fn focus_traversing(&mut self, id: Option<&str>) {
        self.selection.extra.clear();
        self.land_focus(id, false);
    }

    /// Focus moved by EXTENDING: the extras are kept as the caller set them.
    pub fn focus_extended(&mut self, id: &str, extra: Vec<String>) {
        self.selection.extra = extra;
        self.land_focus(Some(id), true);
    }

    /// A key reached the focused card's body: the visit is earned, whatever
    /// brought the focus there. The ui calls this from `key_down`.
    pub fn note_input(&mut self) {
        if let Some(v) = &mut self.focus_visit {
            v.typed = true;
        }
        self.promote_focus();
        // Typing into a done card is reading it.
        if let Some(id) = self
            .focused()
            .filter(|c| c.agent == AgentState::Done)
            .map(|c| c.id.clone())
        {
            self.mark_seen(&id);
        }
    }

    /// Done means "finished and you have not looked": once the focused done
    /// card has been watched `SEEN_MS` with the app in front, it goes grey.
    /// Before 2026-09-27 a done card stayed green until its next turn, and
    /// seven green cards on one canvas said nothing (Ekin). The dwell is
    /// the switcher's (`TRAIL_DWELL_MS`), so walking past a card with
    /// Cmd+Alt+Arrow does not count as reading it.
    fn clear_seen_done(&mut self) {
        let watched = self
            .focused()
            .filter(|c| self.app_active && c.agent == AgentState::Done)
            .map(|c| c.id.clone());
        let Some(id) = watched else {
            self.done_watch = None;
            return;
        };
        match &self.done_watch {
            Some((w, since)) if *w == id => {
                if self.now_ms - since >= SEEN_MS {
                    self.mark_seen(&id);
                }
            }
            _ => self.done_watch = Some((id, self.now_ms)),
        }
    }

    fn mark_seen(&mut self, id: &str) {
        self.done_watch = None;
        if let Some(c) = self.card_mut(id) {
            c.agent = AgentState::None;
        }
        self.effects.push(Effect::AgentLog(format!(
            "{}  seen -> none",
            &id[..8.min(id.len())]
        )));
    }

    /// A seen done card is due to go grey: the ui asks for a frame then,
    /// since nothing else may be drawing.
    pub fn done_seen_due(&self, now_ms: f64) -> bool {
        self.done_watch
            .as_ref()
            .is_some_and(|(_, since)| now_ms - since >= SEEN_MS)
    }

    /// Put the current visit in the trail if it has earned its place. Run
    /// when the focus leaves, when a key arrives, and on every tick, so a
    /// card sat in is in the trail before you leave it: closing a card
    /// walks the same trail.
    fn promote_focus(&mut self) {
        let now = self.now_ms;
        let Some(v) = &self.focus_visit else { return };
        if v.earned {
            return;
        }
        if !crate::switcher::earns_trail(false, v.typed, now - v.since) {
            return;
        }
        let id = v.id.clone();
        self.remember_focus(&id);
        if let Some(v) = &mut self.focus_visit {
            v.earned = true;
        }
    }

    /// The trail, most recent last, each card once.
    fn remember_focus(&mut self, id: &str) {
        self.focus_trail.retain(|t| t != id);
        self.focus_trail.push(id.to_string());
        if self.focus_trail.len() > Self::FOCUS_TRAIL {
            self.focus_trail.remove(0);
        }
    }

    fn land_focus(&mut self, id: Option<&str>, deliberate: bool) {
        // The visit that is ending: it keeps its place only if it earned
        // one. Then the new visit starts, earned at once when it was
        // chosen rather than walked into.
        self.promote_focus();
        self.focus_visit = id.map(|id| FocusVisit {
            id: id.to_string(),
            since: self.now_ms,
            earned: false,
            typed: false,
        });
        if deliberate {
            if let Some(id) = id {
                self.remember_focus(id);
            }
            if let Some(v) = &mut self.focus_visit {
                v.earned = true;
            }
        }
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
    /// The space between cards (`cards.gap`).
    pub fn gap(&self) -> f64 {
        self.config.cards.gap
    }

    /// A group's frame sits half a gap outside its cards, so two frames fit
    /// in one gap and groups line up on the grid like cards.
    pub fn group_pad(&self) -> f64 {
        self.gap() / 2.
    }

    pub fn group_frame(&self, group_id: &str, workspace_id: &str) -> Option<Rect> {
        let rects: Vec<Rect> = self
            .cards
            .iter()
            .filter(|c| c.group_id.as_deref() == Some(group_id) && c.workspace_id == workspace_id)
            .map(|c| c.rect)
            .collect();
        group_bounds(&rects, self.group_pad())
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
