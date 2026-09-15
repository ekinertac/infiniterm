//! A one-line text prompt and a yes/no confirm, as state plus a pending
//! answer. Port of prompt.svelte.ts and its tests.
//!
//! The reference built this because `window.prompt` was dead in its
//! webview; the reason is gone here but the shape stays, since commands are
//! synchronous and fire-and-forget the question. The answer arrives through
//! a callback the asker hands over. Empty text counts as a cancel so that
//! clearing the field and pressing Enter cannot make a nameless group with
//! no way to identify it again. A second ask REPLACES the first: two
//! prompts sharing one input would leave whichever lost never answered.
//!
//! `confirm` is the same panel without the field: Enter for yes, Escape for
//! no. Used only where a key would otherwise destroy something with nothing
//! to undo it (closing a workspace kills every shell on it). A dirty editor
//! deliberately does NOT get one; a dialog on every close is a dialog
//! nobody reads. The name-prompt element renders this and calls `settle`.
type Answer = Box<dyn FnOnce(Option<String>)>;

#[derive(Default)]
pub struct Prompt {
    pub open: bool,
    pub label: String,
    pub value: String,
    /// A yes/no question rather than a text field.
    pub confirm: bool,
    pending: Option<Answer>,
}

impl Prompt {
    /// `answer` gets the trimmed text, or `None` if cancelled or left empty.
    pub fn ask(
        &mut self,
        label: &str,
        initial: &str,
        answer: impl FnOnce(Option<String>) + 'static,
    ) {
        if let Some(previous) = self.pending.take() {
            previous(None);
        }
        self.open = true;
        self.confirm = false;
        self.label = label.into();
        self.value = initial.into();
        self.pending = Some(Box::new(answer));
    }

    /// `answer` gets true on Enter, false on Escape or a click outside.
    pub fn confirm(&mut self, label: &str, answer: impl FnOnce(bool) + 'static) {
        if let Some(previous) = self.pending.take() {
            previous(None);
        }
        self.open = true;
        self.confirm = true;
        self.label = label.into();
        self.value.clear();
        self.pending = Some(Box::new(move |value| answer(value.is_some())));
    }

    /// The panel's verdict: `Some(text)` for Enter, `None` for a cancel.
    pub fn settle(&mut self, value: Option<&str>) {
        let pending = self.pending.take();
        self.open = false;
        let Some(resolve) = pending else { return };
        if self.confirm {
            resolve(value.map(String::from)); // "" is a yes here; only None is a no
            return;
        }
        resolve(
            value
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(String::from),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, rc::Rc};

    type Slot = Rc<RefCell<Option<Option<String>>>>;

    fn ask(p: &mut Prompt, label: &str, initial: &str) -> Slot {
        let slot: Slot = Rc::default();
        let s = slot.clone();
        p.ask(label, initial, move |v| *s.borrow_mut() = Some(v));
        slot
    }

    #[test]
    fn ask_opens_with_the_label_and_the_suggested_value() {
        let mut p = Prompt::default();
        let answer = ask(&mut p, "group name", "api");
        assert!(p.open);
        assert_eq!(p.label, "group name");
        assert_eq!(p.value, "api");
        p.settle(None);
        assert_eq!(*answer.borrow(), Some(None));
    }

    #[test]
    fn settle_closes_and_resolves_with_the_trimmed_text() {
        let mut p = Prompt::default();
        let answer = ask(&mut p, "group name", "");
        p.settle(Some("  billing  "));
        assert_eq!(*answer.borrow(), Some(Some("billing".into())));
        assert!(!p.open);
    }

    // Empty resolves as a cancel rather than an empty string.
    #[test]
    fn blank_text_resolves_as_a_cancel() {
        let mut p = Prompt::default();
        let answer = ask(&mut p, "group name", "api");
        p.settle(Some("   "));
        assert_eq!(*answer.borrow(), Some(None));
    }

    #[test]
    fn cancelling_resolves_with_none() {
        let mut p = Prompt::default();
        let answer = ask(&mut p, "card name", "");
        p.settle(None);
        assert_eq!(*answer.borrow(), Some(None));
    }

    // Two prompts sharing one input would leave whichever lost never answered.
    #[test]
    fn a_second_ask_cancels_the_first_instead_of_stacking() {
        let mut p = Prompt::default();
        let first = ask(&mut p, "group name", "api");
        let second = ask(&mut p, "card name", "web");
        assert_eq!(*first.borrow(), Some(None));
        assert_eq!(p.label, "card name");
        p.settle(Some("web"));
        assert_eq!(*second.borrow(), Some(Some("web".into())));
    }

    #[test]
    fn settling_twice_does_not_resolve_a_stale_answer() {
        let mut p = Prompt::default();
        let answer = ask(&mut p, "group name", "");
        p.settle(Some("one"));
        p.settle(Some("two"));
        assert_eq!(*answer.borrow(), Some(Some("one".into())));
    }

    // Native check: confirm answers yes on Enter with an empty field, no on cancel.
    #[test]
    fn confirm_is_yes_on_enter_and_no_on_cancel() {
        let mut p = Prompt::default();
        let got: Rc<RefCell<Vec<bool>>> = Rc::default();
        let g = got.clone();
        p.confirm("close the workspace?", move |yes| g.borrow_mut().push(yes));
        assert!(p.confirm);
        p.settle(Some(""));
        let g = got.clone();
        p.confirm("again?", move |yes| g.borrow_mut().push(yes));
        p.settle(None);
        assert_eq!(*got.borrow(), [true, false]);
    }
}
