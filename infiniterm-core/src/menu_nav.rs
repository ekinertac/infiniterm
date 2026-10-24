//! Keyboard and mouse navigation of a right-click menu the app draws itself
//! (#464): where macOS has no `NSMenu` to hand the rows to, which is Linux
//! and Windows. Pure: the rows are `context_menu::Row`s, a selection moves
//! over them, and a choice is a command id.
//!
//! Called by `infiniterm-ui/src/menus.rs` (opens one into `Model::menu`),
//! `input.rs` (the keys) and `menu_view.rs` (draws it and feeds the mouse).
//! Related: `context_menu.rs` (the rows), `native_menu.rs` (the Mac's popup,
//! which needs none of this).
//!
//! The model: a stack of OPEN submenus (`open`, each the index of the
//! submenu row in the level above) and one SELECTED row in the deepest open
//! level. A separator and a disabled item are never selected; moving wraps,
//! the way a desktop menu does. Hovering a submenu opens it without selecting
//! inside it, so Enter there chooses nothing until a row is picked.

use crate::context_menu::Row;

/// A menu on screen: its rows, the selection, and where it was opened in
/// window pixels (the ui clamps it into the window).
#[derive(Clone, Debug, PartialEq)]
pub struct OpenMenu {
    pub nav: MenuNav,
    pub at: (f32, f32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct MenuNav {
    rows: Vec<Row>,
    open: Vec<usize>,
    selected: Option<usize>,
}

/// What a key or a click did.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// Nothing to run; the menu stays.
    Stay,
    /// Run this command; the menu closes.
    Chosen(String),
}

/// Where the top menu goes for a click at `at`, sized `size`, in a window
/// `window` big: at the pointer, opening up instead of down when it would
/// leave the bottom and to the left when it would leave the right, the way a
/// desktop menu does; never past the top or left edge.
pub fn place(at: (f32, f32), size: (f32, f32), window: (f32, f32)) -> (f32, f32) {
    let x = if at.0 + size.0 > window.0 {
        at.0 - size.0
    } else {
        at.0
    };
    let y = if at.1 + size.1 > window.1 {
        at.1 - size.1
    } else {
        at.1
    };
    (x.max(0.0), y.max(0.0))
}

/// Where a submenu goes: right of its parent menu with its first row level
/// with the row that opened it, else left of the parent; moved up to stay
/// inside the window.
pub fn place_submenu(
    parent_x: f32,
    parent_w: f32,
    row_y: f32,
    size: (f32, f32),
    window: (f32, f32),
) -> (f32, f32) {
    let right = parent_x + parent_w;
    let x = if right + size.0 > window.0 {
        parent_x - size.0
    } else {
        right
    };
    let y = if row_y + size.1 > window.1 {
        window.1 - size.1
    } else {
        row_y
    };
    (x.max(0.0), y.max(0.0))
}

fn selectable(row: &Row) -> bool {
    match row {
        Row::Item { enabled, .. } => *enabled,
        Row::Submenu { .. } => true,
        Row::Separator => false,
    }
}

impl MenuNav {
    pub fn new(rows: Vec<Row>) -> Self {
        Self {
            rows,
            open: vec![],
            selected: None,
        }
    }

    /// The rows of each open level, top first, with the row to highlight:
    /// the open submenu's row in a level above, the selection in the deepest.
    pub fn levels(&self) -> Vec<(&[Row], Option<usize>)> {
        let mut out = vec![];
        let mut rows: &[Row] = &self.rows;
        for &i in &self.open {
            out.push((rows, Some(i)));
            match rows.get(i) {
                Some(Row::Submenu { rows: inner, .. }) => rows = inner,
                _ => break,
            }
        }
        out.push((rows, self.selected));
        out
    }

    fn deepest(&self) -> &[Row] {
        self.levels().last().map(|(rows, _)| *rows).unwrap_or(&[])
    }

    /// The next selectable row after (or before) the selection, wrapping.
    fn step(&self, forward: bool) -> Option<usize> {
        let rows = self.deepest();
        let n = rows.len();
        if n == 0 {
            return None;
        }
        let start = match (self.selected, forward) {
            (Some(i), true) => i + 1,
            (Some(i), false) => i + n - 1,
            (None, true) => 0,
            (None, false) => n - 1,
        };
        (0..n)
            .map(|k| {
                if forward {
                    (start + k) % n
                } else {
                    (start + n - k) % n
                }
            })
            .find(|&i| selectable(&rows[i]))
    }

    pub fn down(&mut self) {
        self.selected = self.step(true);
    }

    pub fn up(&mut self) {
        self.selected = self.step(false);
    }

    /// Into the selected submenu, its first row selected.
    pub fn right(&mut self) {
        let Some(i) = self.selected else { return };
        if matches!(self.deepest().get(i), Some(Row::Submenu { .. })) {
            self.open.push(i);
            self.selected = None;
            self.down();
        }
    }

    /// Out of the deepest submenu, its row selected again.
    pub fn left(&mut self) {
        if let Some(i) = self.open.pop() {
            self.selected = Some(i);
        }
    }

    pub fn enter(&mut self) -> Outcome {
        match self.selected.and_then(|i| self.deepest().get(i)) {
            Some(Row::Item {
                command,
                enabled: true,
                ..
            }) => Outcome::Chosen(command.clone()),
            Some(Row::Submenu { .. }) => {
                self.right();
                Outcome::Stay
            }
            _ => Outcome::Stay,
        }
    }

    /// The pointer is over row `index` of level `level` (0 is the top menu):
    /// deeper levels close, the row is selected, a submenu opens.
    pub fn hover(&mut self, level: usize, index: usize) {
        if level > self.open.len() {
            return;
        }
        self.open.truncate(level);
        let row = self.deepest().get(index).cloned();
        match row {
            Some(Row::Submenu { .. }) => {
                self.open.push(index);
                self.selected = None;
            }
            Some(r) if selectable(&r) => self.selected = Some(index),
            _ => self.selected = None,
        }
    }

    /// A click on row `index` of level `level`: an enabled item is chosen, a
    /// submenu opens, anything else does nothing.
    pub fn click(&mut self, level: usize, index: usize) -> Outcome {
        self.hover(level, index);
        if self.levels().len() == level + 1 {
            self.enter()
        } else {
            Outcome::Stay
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_menu::{rows, Area, Source};
    use crate::when::Context;

    fn item(command: &str, enabled: bool) -> Row {
        Row::Item {
            command: command.into(),
            title: command.into(),
            chord: None,
            enabled,
        }
    }

    fn sub(title: &str, rows: Vec<Row>) -> Row {
        Row::Submenu {
            title: title.into(),
            rows,
        }
    }

    #[test]
    fn a_menu_opens_with_nothing_selected_and_down_skips_what_cannot_be_chosen() {
        let mut nav = MenuNav::new(vec![
            Row::Separator,
            item("off", false),
            item("a", true),
            Row::Separator,
            item("b", true),
        ]);
        assert_eq!(
            nav.enter(),
            Outcome::Stay,
            "nothing selected chooses nothing"
        );
        nav.down();
        assert_eq!(nav.enter(), Outcome::Chosen("a".into()));
        nav.down();
        assert_eq!(nav.enter(), Outcome::Chosen("b".into()));
        // Wraps past the separator and the disabled row back to the first.
        nav.down();
        assert_eq!(nav.enter(), Outcome::Chosen("a".into()));
        nav.up();
        assert_eq!(nav.enter(), Outcome::Chosen("b".into()));
    }

    #[test]
    fn a_menu_opens_at_the_pointer_and_flips_near_an_edge() {
        let window = (1000.0, 800.0);
        assert_eq!(
            place((100.0, 100.0), (200.0, 300.0), window),
            (100.0, 100.0)
        );
        // Near the bottom it opens upward, near the right it opens leftward.
        assert_eq!(
            place((100.0, 700.0), (200.0, 300.0), window),
            (100.0, 400.0)
        );
        assert_eq!(
            place((900.0, 100.0), (200.0, 300.0), window),
            (700.0, 100.0)
        );
        // Taller than the room above too: pinned to the top.
        assert_eq!(place((100.0, 200.0), (200.0, 900.0), window), (100.0, 0.0));
    }

    #[test]
    fn a_submenu_opens_beside_its_row_and_stays_in_the_window() {
        let window = (1000.0, 800.0);
        assert_eq!(
            place_submenu(100.0, 200.0, 150.0, (180.0, 100.0), window),
            (300.0, 150.0)
        );
        // No room on the right: left of the parent.
        assert_eq!(
            place_submenu(700.0, 200.0, 150.0, (180.0, 100.0), window),
            (520.0, 150.0)
        );
        // A row near the bottom: moved up so the last row shows.
        assert_eq!(
            place_submenu(100.0, 200.0, 750.0, (180.0, 100.0), window),
            (300.0, 700.0)
        );
    }

    #[test]
    fn up_from_nothing_selected_is_the_last_row() {
        let mut nav = MenuNav::new(vec![item("a", true), item("b", true)]);
        nav.up();
        assert_eq!(nav.enter(), Outcome::Chosen("b".into()));
    }

    #[test]
    fn right_opens_a_submenu_and_left_comes_back_to_its_row() {
        let mut nav = MenuNav::new(vec![
            item("top", true),
            sub(
                "More",
                vec![item("one", true), Row::Separator, item("two", true)],
            ),
        ]);
        nav.down();
        nav.down();
        nav.right();
        assert_eq!(nav.levels().len(), 2);
        nav.down();
        assert_eq!(nav.enter(), Outcome::Chosen("two".into()));
        nav.left();
        assert_eq!(nav.levels().len(), 1);
        // Enter on the submenu's row opens it again, never chooses it.
        assert_eq!(nav.enter(), Outcome::Stay);
        assert_eq!(nav.levels().len(), 2);
        assert_eq!(nav.enter(), Outcome::Chosen("one".into()));
    }

    #[test]
    fn hovering_a_submenu_opens_it_without_choosing_inside() {
        let mut nav = MenuNav::new(vec![
            item("top", true),
            sub("More", vec![item("one", true)]),
        ]);
        nav.hover(0, 1);
        assert_eq!(nav.levels().len(), 2);
        assert_eq!(nav.levels()[0].1, Some(1), "the submenu's row stays lit");
        assert_eq!(nav.enter(), Outcome::Stay);
        // Back over a top row closes the submenu.
        nav.hover(0, 0);
        assert_eq!(nav.levels().len(), 1);
        assert_eq!(nav.click(0, 0), Outcome::Chosen("top".into()));
    }

    #[test]
    fn a_click_chooses_an_enabled_item_only() {
        let mut nav = MenuNav::new(vec![
            item("off", false),
            Row::Separator,
            sub("More", vec![item("one", true)]),
        ]);
        assert_eq!(nav.click(0, 0), Outcome::Stay);
        assert_eq!(nav.click(0, 1), Outcome::Stay);
        assert_eq!(nav.click(0, 2), Outcome::Stay, "a submenu opens");
        assert_eq!(nav.click(1, 0), Outcome::Chosen("one".into()));
        // A level that is not open is ignored.
        let mut fresh = MenuNav::new(vec![item("a", true)]);
        assert_eq!(fresh.click(3, 0), Outcome::Stay);
    }

    #[test]
    fn the_real_terminal_menu_reaches_a_command_inside_its_canvas_submenu() {
        let ctx = Context::terminal_focused();
        let built = rows(
            Area::Terminal,
            &Source {
                context: &ctx,
                label: &|id| Some(format!("Card: {id}")),
                chord: &|_| None,
                shown: &|_| None,
            },
        );
        let canvas = built
            .iter()
            .position(|r| matches!(r, Row::Submenu { title, .. } if title == "Canvas"))
            .expect("the terminal menu has a Canvas submenu");
        let Row::Submenu { rows: inner, .. } = &built[canvas] else {
            unreachable!()
        };
        let third = inner
            .iter()
            .filter_map(|r| match r {
                Row::Item {
                    command,
                    enabled: true,
                    ..
                } => Some(command.clone()),
                _ => None,
            })
            .nth(2)
            .expect("three enabled rows in Canvas");
        let mut nav = MenuNav::new(built.clone());
        while nav.levels()[0].1 != Some(canvas) {
            nav.down();
        }
        nav.right();
        nav.down();
        nav.down();
        assert_eq!(nav.enter(), Outcome::Chosen(third));
    }
}
