//! A one-line text prompt and a yes/no confirm, as state plus a pending
//! question. Port of prompt.svelte.ts and its tests.
//!
//! The reference built this because `window.prompt` was dead in its
//! webview; the reason is gone here but the shape stays, since commands are
//! synchronous and fire-and-forget the question. What the asker wants done
//! with the answer travels as a value of type `P` (the model's `Pending`
//! enum) rather than a closure, so the model stays a plain struct and the
//! flow is testable. Empty text counts as a cancel so that clearing the
//! field and pressing Enter cannot make a nameless group. A second ask
//! REPLACES the first: two prompts sharing one input would leave whichever
//! lost never answered, so `ask` hands the loser back as cancelled.
//!
//! `confirm` is a question with two buttons, the verb on the second
//! (`action`): Enter or the button for yes, Escape or Cancel for no. Used
//! only where a key would otherwise destroy something with nothing to undo
//! it (closing a workspace kills every shell on it). `confirm3` adds a
//! third, the macOS save sheet's shape (2026-09-26, Ekin): Save, Don't Save
//! (Cmd+D, the old Mac key, in his muscle memory), Cancel; a dirty editor's
//! close asks it, because the two-press close it replaced lost work to a
//! double press. `alert` is a message with one button, for something the
//! status bar's notice is too small for. The dialog element renders all
//! three and calls `settle`.
//!
//! The buttons are keyboard-reachable (`choice`): arrows and Tab move a
//! highlight that starts on the action, and Enter presses whatever is
//! highlighted. macOS splits that between Return (the default) and Space
//! (the focused button), a hidden rule; here one key does one thing.
#[derive(Debug, PartialEq, Eq)]
pub struct Prompt<P> {
    pub open: bool,
    pub label: String,
    pub value: String,
    /// A yes/no question rather than a text field.
    pub confirm: bool,
    /// A message with one button; settles yes on Enter, no on Escape.
    pub alert: bool,
    /// The verb on the confirming button ("Close workspace"), "OK" for an alert.
    pub action: String,
    /// A third button (`confirm3`): "Don't Save".
    pub alt_action: Option<String>,
    /// The highlighted button, an index into `buttons()`.
    pub choice: usize,
    /// A text prompt's starting selection, as a char range, when it is not
    /// the whole value (`ask_selecting`: the stem of `untitled.txt`).
    pub select: Option<(usize, usize)>,
    /// Counts the questions asked, so the ui refills its field for a new
    /// one even when it replaces another without the dialog closing (the
    /// save sheet's Save opening the save-as field in the same answer;
    /// watching `open` alone left that field empty, 2026-09-26).
    pub serial: u64,
    pending: Option<P>,
}

/// What a button does when pressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonKind {
    /// The action, the default: `Some("")`.
    Primary,
    /// The third choice (`confirm3`): `Some(ALT)`.
    Alt,
    /// `None`.
    Cancel,
}

/// The answer text a pressed third button settles with.
pub const ALT: &str = "alt";

/// What a settled prompt hands back: the question's payload and the answer,
/// trimmed text or `None` for a cancel (for a confirm, `Some` means yes).
pub type Answer<P> = (P, Option<String>);

impl<P> Default for Prompt<P> {
    fn default() -> Self {
        Prompt {
            open: false,
            label: String::new(),
            value: String::new(),
            confirm: false,
            alert: false,
            action: String::new(),
            alt_action: None,
            choice: 0,
            select: None,
            serial: 0,
            pending: None,
        }
    }
}

impl<P> Prompt<P> {
    /// Opens a text prompt. Returns the previous question, cancelled, if one
    /// was still up.
    pub fn ask(&mut self, label: &str, initial: &str, pending: P) -> Option<Answer<P>> {
        let displaced = self.pending.take().map(|p| (p, None));
        self.open = true;
        self.confirm = false;
        self.alert = false;
        self.action = String::new();
        self.alt_action = None;
        self.choice = 0;
        self.select = None;
        self.serial += 1;
        self.label = label.into();
        self.value = initial.into();
        self.pending = Some(pending);
        displaced
    }

    /// `ask` with only part of the value selected, so typing replaces that
    /// part: the name in a save-as path.
    pub fn ask_selecting(
        &mut self,
        label: &str,
        initial: &str,
        select: (usize, usize),
        pending: P,
    ) -> Option<Answer<P>> {
        let displaced = self.ask(label, initial, pending);
        self.select = Some(select);
        displaced
    }

    /// Opens a yes/no question with `action` on the yes button; the
    /// answer's text is `Some("")` for yes.
    pub fn confirm(&mut self, label: &str, action: &str, pending: P) -> Option<Answer<P>> {
        let displaced = self.pending.take().map(|p| (p, None));
        self.open = true;
        self.confirm = true;
        self.alert = false;
        self.action = action.into();
        self.alt_action = None;
        self.serial += 1;
        self.label = label.into();
        self.value.clear();
        self.pending = Some(pending);
        self.choice = self.primary_index();
        displaced
    }

    /// A confirm with a third button, `alt` ("Don't Save"), laid out the
    /// way macOS does it: the alternative on the left, then Cancel, then
    /// the action.
    pub fn confirm3(
        &mut self,
        label: &str,
        action: &str,
        alt: &str,
        pending: P,
    ) -> Option<Answer<P>> {
        let displaced = self.confirm(label, action, pending);
        self.alt_action = Some(alt.into());
        self.choice = self.primary_index();
        displaced
    }

    /// The buttons left to right, as labels and what each does.
    pub fn buttons(&self) -> Vec<(String, ButtonKind)> {
        if self.alert {
            return vec![(self.action.clone(), ButtonKind::Primary)];
        }
        if !self.confirm {
            return vec![];
        }
        let mut b = vec![];
        if let Some(alt) = &self.alt_action {
            b.push((alt.clone(), ButtonKind::Alt));
        }
        b.push(("Cancel".into(), ButtonKind::Cancel));
        b.push((self.action.clone(), ButtonKind::Primary));
        b
    }

    fn primary_index(&self) -> usize {
        self.buttons()
            .iter()
            .position(|(_, k)| *k == ButtonKind::Primary)
            .unwrap_or(0)
    }

    /// Arrows and Tab: the highlight moves by `delta`, wrapping.
    pub fn move_choice(&mut self, delta: i32) {
        let n = self.buttons().len() as i32;
        if n > 0 {
            self.choice = (self.choice as i32 + delta).rem_euclid(n) as usize;
        }
    }

    /// Enter: presses the highlighted button.
    pub fn press_choice(&mut self) -> Option<Answer<P>> {
        let kind = self.buttons().get(self.choice).map(|(_, k)| *k)?;
        self.press(kind)
    }

    /// A button, by what it does.
    pub fn press(&mut self, kind: ButtonKind) -> Option<Answer<P>> {
        match kind {
            ButtonKind::Primary => self.settle(Some("")),
            ButtonKind::Alt if self.alt_action.is_some() => self.settle(Some(ALT)),
            ButtonKind::Alt => None,
            ButtonKind::Cancel => self.settle(None),
        }
    }

    /// Chords a confirm takes before the canvas sees them: Cmd+D is Don't
    /// Save (else it would split the card behind the dialog) and Cmd+. is
    /// Cancel, both macOS's.
    pub fn chord(&self, chord: &str) -> Option<ButtonKind> {
        if !self.open || !self.confirm || self.alert {
            return None;
        }
        match chord {
            "cmd+d" if self.alt_action.is_some() => Some(ButtonKind::Alt),
            "cmd+." => Some(ButtonKind::Cancel),
            _ => None,
        }
    }

    /// Opens a message with one button. Settles like a confirm.
    pub fn alert(&mut self, label: &str, pending: P) -> Option<Answer<P>> {
        let displaced = self.confirm(label, "OK", pending);
        self.alert = true;
        displaced
    }

    /// The panel's verdict: `Some(text)` for Enter, `None` for a cancel.
    /// Returns the question to act on, or nothing when none was pending
    /// (settling twice resolves nothing stale).
    pub fn settle(&mut self, value: Option<&str>) -> Option<Answer<P>> {
        let pending = self.pending.take();
        self.open = false;
        let pending = pending?;
        if self.confirm || self.alert {
            return Some((pending, value.map(String::from))); // "" is a yes; only None is a no
        }
        Some((
            pending,
            value
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(String::from),
        ))
    }

    pub fn is_open(&self) -> bool {
        self.open
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The save sheet: Don't Save, Cancel, Save, the highlight on Save so
    // Enter keeps your work; arrows and Tab walk the buttons, wrapping;
    // Enter presses what is highlighted; Cmd+D is Don't Save.
    #[test]
    fn the_save_sheet_defaults_to_save_and_walks_by_keys() {
        let mut p = Prompt::default();
        p.confirm3("save changes to .zshrc?", "Save", "Don't Save", 1);
        let labels: Vec<String> = p.buttons().into_iter().map(|(l, _)| l).collect();
        assert_eq!(labels, ["Don't Save", "Cancel", "Save"]);
        assert_eq!(p.choice, 2, "Save is highlighted");
        p.move_choice(1);
        assert_eq!(p.choice, 0, "wraps");
        p.move_choice(-1);
        p.move_choice(-1);
        assert_eq!(p.choice, 1, "Cancel");
        assert_eq!(p.press_choice(), Some((1, None)), "Enter pressed Cancel");
        p.confirm3("q", "Save", "Don't Save", 2);
        assert_eq!(
            p.press_choice(),
            Some((2, Some(String::new()))),
            "Enter is Save"
        );
        p.confirm3("q", "Save", "Don't Save", 3);
        assert_eq!(p.chord("cmd+d"), Some(ButtonKind::Alt));
        assert_eq!(p.press(ButtonKind::Alt), Some((3, Some(ALT.into()))));
    }

    // A two-button confirm has no Cmd+D (it would split the card behind
    // it for nothing) and starts on its action.
    #[test]
    fn a_plain_confirm_starts_on_its_action_and_leaves_cmd_d_alone() {
        let mut p = Prompt::default();
        p.confirm("close workspace?", "Close workspace", 1);
        assert_eq!(p.buttons().len(), 2);
        assert_eq!(p.choice, 1);
        assert_eq!(p.chord("cmd+d"), None);
        assert_eq!(p.chord("cmd+."), Some(ButtonKind::Cancel));
    }

    // A question replacing another with the dialog still up is still a new
    // question: the serial moves.
    #[test]
    fn every_question_has_its_own_serial() {
        let mut p = Prompt::default();
        p.confirm3("save?", "Save", "Don't Save", 1);
        let first = p.serial;
        p.ask_selecting("save as", "~/untitled.txt", (2, 10), 2);
        assert!(p.open);
        assert_ne!(p.serial, first);
    }

    #[test]
    fn ask_opens_with_the_label_and_the_suggested_value() {
        let mut p = Prompt::default();
        assert!(p.ask("group name", "api", 1).is_none());
        assert!(p.open);
        assert_eq!(p.label, "group name");
        assert_eq!(p.value, "api");
        assert_eq!(p.settle(None), Some((1, None)));
    }

    #[test]
    fn settle_closes_and_resolves_with_the_trimmed_text() {
        let mut p = Prompt::default();
        p.ask("group name", "", 1);
        assert_eq!(
            p.settle(Some("  billing  ")),
            Some((1, Some("billing".into())))
        );
        assert!(!p.open);
    }

    // Empty resolves as a cancel rather than an empty string.
    #[test]
    fn blank_text_resolves_as_a_cancel() {
        let mut p = Prompt::default();
        p.ask("group name", "api", 1);
        assert_eq!(p.settle(Some("   ")), Some((1, None)));
    }

    #[test]
    fn cancelling_resolves_with_none() {
        let mut p = Prompt::default();
        p.ask("card name", "", 1);
        assert_eq!(p.settle(None), Some((1, None)));
    }

    // Two prompts sharing one input would leave whichever lost never answered.
    #[test]
    fn a_second_ask_cancels_the_first_instead_of_stacking() {
        let mut p = Prompt::default();
        p.ask("group name", "api", 1);
        assert_eq!(p.ask("card name", "web", 2), Some((1, None)));
        assert_eq!(p.label, "card name");
        assert_eq!(p.settle(Some("web")), Some((2, Some("web".into()))));
    }

    #[test]
    fn settling_twice_does_not_resolve_a_stale_answer() {
        let mut p = Prompt::default();
        p.ask("group name", "", 1);
        assert_eq!(p.settle(Some("one")), Some((1, Some("one".into()))));
        assert_eq!(p.settle(Some("two")), None);
    }

    // Native check: confirm answers yes on Enter with an empty field, no on cancel.
    #[test]
    fn confirm_is_yes_on_enter_and_no_on_cancel() {
        let mut p = Prompt::default();
        p.confirm("close the workspace?", "Close workspace", 1);
        assert!(p.confirm);
        assert_eq!(p.action, "Close workspace");
        assert_eq!(p.settle(Some("")), Some((1, Some(String::new()))));
        p.confirm("again?", "Yes", 2);
        assert_eq!(p.settle(None), Some((2, None)));
        // An alert is a confirm with one button and the same verdicts.
        p.alert("the file is gone", 3);
        assert!(p.alert && p.confirm);
        assert_eq!(p.action, "OK");
        assert_eq!(p.settle(Some("")), Some((3, Some(String::new()))));
        // A text prompt after that carries neither.
        p.ask("name", "", 4);
        assert!(!p.confirm && !p.alert);
    }
}
