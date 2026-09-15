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
//! `confirm` is the same panel without the field: Enter for yes, Escape for
//! no. Used only where a key would otherwise destroy something with nothing
//! to undo it (closing a workspace kills every shell on it). A dirty editor
//! deliberately does NOT get one; a dialog on every close is a dialog
//! nobody reads. The name-prompt element renders this and calls `settle`.
#[derive(Debug, PartialEq, Eq)]
pub struct Prompt<P> {
    pub open: bool,
    pub label: String,
    pub value: String,
    /// A yes/no question rather than a text field.
    pub confirm: bool,
    pending: Option<P>,
}

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
        self.label = label.into();
        self.value = initial.into();
        self.pending = Some(pending);
        displaced
    }

    /// Opens a yes/no question; the answer's text is `Some("")` for yes.
    pub fn confirm(&mut self, label: &str, pending: P) -> Option<Answer<P>> {
        let displaced = self.pending.take().map(|p| (p, None));
        self.open = true;
        self.confirm = true;
        self.label = label.into();
        self.value.clear();
        self.pending = Some(pending);
        displaced
    }

    /// The panel's verdict: `Some(text)` for Enter, `None` for a cancel.
    /// Returns the question to act on, or nothing when none was pending
    /// (settling twice resolves nothing stale).
    pub fn settle(&mut self, value: Option<&str>) -> Option<Answer<P>> {
        let pending = self.pending.take();
        self.open = false;
        let pending = pending?;
        if self.confirm {
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
        p.confirm("close the workspace?", 1);
        assert!(p.confirm);
        assert_eq!(p.settle(Some("")), Some((1, Some(String::new()))));
        p.confirm("again?", 2);
        assert_eq!(p.settle(None), Some((2, None)));
    }
}
