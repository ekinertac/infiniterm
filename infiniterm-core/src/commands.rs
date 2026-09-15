//! Every action in the app is a named command. Port of commands.ts and its
//! tests.
//!
//! Nothing reads raw key events except the keymap, so actions stay
//! bindable, palette-discoverable and testable without a window. Labels
//! read `Domain: what it does` (`Terminal: clear`, `Canvas: zoom in`); the
//! palette and the generated keybindings file are read as a list and the
//! prefix is what makes sixty entries scannable.
//!
//! Generic over the context a command runs against (the app model in the
//! ui crate), so this crate never learns what a card body is. Every run is
//! logged: "I pressed some keys and ended up like this" is only answerable
//! with a trace, and the cost is one line. `commands/*.rs` register by
//! domain through this.
pub struct Command<C> {
    pub id: String,
    pub label: String,
    pub run: Box<dyn Fn(&mut C)>,
}

pub struct CommandRegistry<C> {
    commands: Vec<Command<C>>,
    log: Box<dyn Fn(&str)>,
    /// Something the trace can say about the moment a command ran: the
    /// active card, typically. Installed by the app, which knows.
    describe_context: Box<dyn Fn(&C) -> String>,
}

impl<C> CommandRegistry<C> {
    pub fn new(log: impl Fn(&str) + 'static) -> Self {
        CommandRegistry {
            commands: vec![],
            log: Box::new(log),
            describe_context: Box::new(|_| String::new()),
        }
    }

    pub fn set_context(&mut self, describe: impl Fn(&C) -> String + 'static) {
        self.describe_context = Box::new(describe);
    }

    /// Re-registering an id replaces the command in place.
    pub fn register(&mut self, id: &str, label: &str, run: impl Fn(&mut C) + 'static) {
        let command = Command {
            id: id.into(),
            label: label.into(),
            run: Box::new(run),
        };
        match self.commands.iter_mut().find(|c| c.id == id) {
            Some(existing) => *existing = command,
            None => self.commands.push(command),
        }
    }

    /// Runs `id`; false when no such command. A keymap naming a command that
    /// does not exist is a config typo, not a crash: it is logged and the
    /// rest of the bindings still work.
    pub fn run(&self, id: &str, ctx: &mut C) -> bool {
        let Some(cmd) = self.commands.iter().find(|c| c.id == id) else {
            (self.log)(&format!("unknown command: {id}"));
            return false;
        };
        (self.log)(&format!("cmd {id} {}", (self.describe_context)(ctx)));
        (cmd.run)(ctx);
        true
    }

    /// In registration order, for the palette.
    pub fn all(&self) -> &[Command<C>] {
        &self.commands
    }

    pub fn get(&self, id: &str) -> Option<&Command<C>> {
        self.commands.iter().find(|c| c.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, rc::Rc};

    fn registry() -> (CommandRegistry<u32>, Rc<RefCell<Vec<String>>>) {
        let log = Rc::new(RefCell::new(vec![]));
        let sink = log.clone();
        (
            CommandRegistry::new(move |line| sink.borrow_mut().push(line.to_string())),
            log,
        )
    }

    #[test]
    fn runs_a_registered_command() {
        let (mut r, _) = registry();
        r.register("test.do", "Do", |n| *n += 1);
        let mut ctx = 0;
        assert!(r.run("test.do", &mut ctx));
        assert_eq!(ctx, 1);
    }

    // A keymap typo must not take the app down with it.
    #[test]
    fn running_an_unknown_command_warns_instead_of_failing() {
        let (r, log) = registry();
        assert!(!r.run("nope.missing", &mut 0));
        assert_eq!(log.borrow().as_slice(), ["unknown command: nope.missing"]);
    }

    #[test]
    fn lists_commands_in_registration_order_for_the_palette() {
        let (mut r, _) = registry();
        r.register("a.one", "Alpha", |_| {});
        r.register("b.two", "Beta", |_| {});
        assert_eq!(
            r.all().iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
            ["a.one", "b.two"]
        );
    }

    #[test]
    fn re_registering_an_id_replaces_it_rather_than_duplicating() {
        let (mut r, _) = registry();
        r.register("x", "First", |_| {});
        r.register("x", "Second", |_| {});
        assert_eq!(r.all().len(), 1);
        assert_eq!(r.all()[0].label, "Second");
    }

    // Native check: every run leaves a trace line with the context.
    #[test]
    fn every_run_is_logged_with_the_context() {
        let (mut r, log) = registry();
        r.set_context(|n| format!("n={n}"));
        r.register("x", "X", |_| {});
        r.run("x", &mut 7);
        assert_eq!(log.borrow().as_slice(), ["cmd x n=7"]);
    }
}
