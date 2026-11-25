//! The window's menu bar: the titles and what each one drops down.
//!
//! The portable window paints its bar into the frame instead of borrowing a
//! platform menu, so the same code works on macOS, Linux and Windows. Choosing
//! an entry runs the same command the chord table would, so the two can never
//! drift apart.
//!
//! Rows are either a configurable [`Action`] (its chord comes from the shortcut
//! table and is shown beside the label) or a fixed [`Command`] that has no
//! settings entry (copy, window management, help links).

use crate::config::{Action, Shortcuts};
use crate::keys::Command;

/// What choosing a row does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entry {
    Action(Action),
    /// A command outside the shortcut table: its chord, if any, is fixed.
    Command(Command),
    Quit,
    /// A spacer between groups; it is not selectable.
    Separator,
}

/// The titles, in bar order.
pub const TITLES: [&str; 7] = [
    "Berth", "File", "Edit", "Terminal", "View", "Window", "Help",
];

const BERTH: &[Entry] = &[
    Entry::Action(Action::Settings),
    Entry::Separator,
    Entry::Quit,
];

const FILE: &[Entry] = &[
    Entry::Action(Action::NewWindow),
    Entry::Action(Action::NewWorkspace),
    Entry::Separator,
    Entry::Action(Action::CloseTab),
    Entry::Command(Command::CloseWorkspace),
    Entry::Separator,
    Entry::Action(Action::Palette),
];

const EDIT: &[Entry] = &[
    Entry::Command(Command::Copy),
    Entry::Command(Command::Paste),
    Entry::Separator,
    Entry::Action(Action::Search),
    Entry::Action(Action::PasteHistory),
    Entry::Separator,
    Entry::Action(Action::CopyMode),
];

const TERMINAL: &[Entry] = &[
    Entry::Action(Action::NewTab),
    Entry::Action(Action::SplitRight),
    Entry::Action(Action::SplitDown),
    Entry::Separator,
    Entry::Action(Action::NextTab),
    Entry::Action(Action::PreviousTab),
    Entry::Action(Action::NextPane),
    Entry::Action(Action::PreviousPane),
    Entry::Separator,
    Entry::Action(Action::TogglePaneZoom),
    Entry::Action(Action::ToggleBroadcast),
    Entry::Action(Action::DuplicateTab),
    Entry::Action(Action::RestartTab),
    Entry::Separator,
    Entry::Action(Action::QuickTerminal),
];

const VIEW: &[Entry] = &[
    Entry::Action(Action::ToggleSidebar),
    Entry::Action(Action::SidebarFilter),
    Entry::Separator,
    Entry::Action(Action::FocusLeft),
    Entry::Action(Action::FocusRight),
    Entry::Action(Action::FocusUp),
    Entry::Action(Action::FocusDown),
    Entry::Separator,
    Entry::Action(Action::ZoomIn),
    Entry::Action(Action::ZoomOut),
    Entry::Action(Action::ZoomReset),
    Entry::Separator,
    Entry::Action(Action::ShowAttention),
    Entry::Action(Action::JumpAttention),
];

const WINDOW: &[Entry] = &[
    Entry::Action(Action::NextWindow),
    Entry::Separator,
    Entry::Command(Command::Minimize),
    Entry::Command(Command::ToggleFullscreen),
    Entry::Separator,
    Entry::Command(Command::CloseWindow),
];

const HELP: &[Entry] = &[
    Entry::Command(Command::OpenDocs),
    Entry::Command(Command::OpenIssues),
];

/// The entries of the menu at `menu`, or an empty slice for an unknown index.
pub fn entries(menu: usize) -> &'static [Entry] {
    match menu {
        0 => BERTH,
        1 => FILE,
        2 => EDIT,
        3 => TERMINAL,
        4 => VIEW,
        5 => WINDOW,
        6 => HELP,
        _ => &[],
    }
}

/// Encode a menu row as an `NSMenuItem` tag: the menu index and row in one
/// number. Called from the macOS menu builder.
#[cfg(target_os = "macos")]
pub fn tag(menu: usize, row: usize) -> isize {
    (menu * 100 + row) as isize
}

/// The entry a tag refers to, or `None` for a tag this window never created.
#[cfg(target_os = "macos")]
pub fn entry_for_tag(tag: isize) -> Option<Entry> {
    let tag = usize::try_from(tag).ok()?;
    entries(tag / 100).get(tag % 100).copied()
}

impl Entry {
    /// The row's label; empty for a separator.
    pub fn label(self) -> &'static str {
        match self {
            Self::Action(action) => action.label(),
            Self::Command(command) => command.label(),
            Self::Quit => "Quit Berth",
            Self::Separator => "",
        }
    }

    /// The chord to show beside the label, in config spelling. Actions read the
    /// user's shortcut table; fixed commands carry their own.
    pub fn chord(self, shortcuts: &Shortcuts) -> Option<String> {
        match self {
            Self::Action(action) => {
                let chord = shortcuts.label(action);
                (!chord.is_empty() && chord != "unbound").then_some(chord)
            }
            Self::Command(command) => command.chord().map(str::to_string),
            _ => None,
        }
    }

    pub fn selectable(self) -> bool {
        !matches!(self, Self::Separator)
    }
}

/// The next selectable entry index from `from`, stepping by `delta` and
/// wrapping. Returns `from` when nothing is selectable.
pub fn step(entries: &[Entry], from: usize, delta: i32) -> usize {
    let len = entries.len();
    if len == 0 {
        return 0;
    }
    let mut index = from as i32;
    for _ in 0..len {
        index = (index + delta).rem_euclid(len as i32);
        if entries[index as usize].selectable() {
            return index as usize;
        }
    }
    from.min(len - 1)
}

/// The first selectable entry, for when a menu opens.
pub fn first_selectable(entries: &[Entry]) -> usize {
    entries
        .iter()
        .position(|entry| entry.selectable())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shortcuts() -> Shortcuts {
        Shortcuts::build(&std::collections::HashMap::new())
    }

    #[test]
    fn given_every_title_then_it_has_at_least_one_selectable_entry() {
        for (index, title) in TITLES.iter().enumerate() {
            let entries = entries(index);
            assert!(!entries.is_empty(), "{title} has no entries");
            assert!(
                entries.iter().any(|entry| entry.selectable()),
                "{title} has nothing to choose"
            );
        }
    }

    #[test]
    fn given_an_action_row_then_its_chord_comes_from_the_shortcut_table() {
        let shortcuts = shortcuts();
        let entry = Entry::Action(Action::SplitRight);
        assert_eq!(
            entry.chord(&shortcuts).as_deref(),
            Some(shortcuts.label(Action::SplitRight).as_str())
        );
        assert_eq!(Entry::Separator.chord(&shortcuts), None);
    }

    #[test]
    fn given_a_fixed_command_then_it_can_still_show_a_chord() {
        assert_eq!(
            Entry::Command(Command::Copy).chord(&shortcuts()).as_deref(),
            Command::Copy.chord()
        );
    }

    #[test]
    fn given_a_separator_when_stepping_then_it_is_skipped() {
        let entries = &[
            Entry::Action(Action::NewTab),
            Entry::Separator,
            Entry::Action(Action::CloseTab),
        ];
        assert_eq!(step(entries, 0, 1), 2);
        assert_eq!(step(entries, 2, 1), 0);
        assert_eq!(step(entries, 0, -1), 2);
    }

    #[test]
    fn given_a_menu_that_starts_with_a_separator_then_the_first_choice_skips_it() {
        let entries = &[Entry::Separator, Entry::Action(Action::NewTab)];
        assert_eq!(first_selectable(entries), 1);
    }

    #[test]
    fn given_the_quit_entry_then_it_is_selectable_and_has_a_label() {
        assert_eq!(Entry::Quit.label(), "Quit Berth");
        assert!(Entry::Quit.selectable());
        assert!(!Entry::Separator.selectable());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn given_a_menu_row_then_its_tag_resolves_back_to_the_entry() {
        for menu in 0..TITLES.len() {
            for (row, entry) in entries(menu).iter().enumerate() {
                assert_eq!(entry_for_tag(tag(menu, row)), Some(*entry));
            }
        }
        assert_eq!(entry_for_tag(-1), None);
        assert_eq!(entry_for_tag(tag(9, 0)), None);
    }
}
