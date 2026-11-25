//! The window's menu bar: the titles and what each one drops down.
//!
//! The portable window paints its bar into the frame instead of borrowing a
//! platform menu, so the same code works on macOS, Linux and Windows. Choosing
//! an entry runs the same command the chord table would, so the two can never
//! drift apart.

use crate::config::Action;

/// What choosing a row does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entry {
    Action(Action),
    Quit,
    /// A spacer between groups; it is not selectable.
    Separator,
}

/// The titles, in bar order.
pub const TITLES: [&str; 4] = ["Berth", "Terminal", "View", "Window"];

const BERTH: &[Entry] = &[
    Entry::Action(Action::Settings),
    Entry::Action(Action::Palette),
    Entry::Action(Action::QuickTerminal),
    Entry::Separator,
    Entry::Quit,
];

const TERMINAL: &[Entry] = &[
    Entry::Action(Action::NewTab),
    Entry::Action(Action::SplitRight),
    Entry::Action(Action::SplitDown),
    Entry::Action(Action::CloseTab),
    Entry::Separator,
    Entry::Action(Action::NextTab),
    Entry::Action(Action::PreviousTab),
    Entry::Action(Action::NextPane),
    Entry::Action(Action::PreviousPane),
    Entry::Separator,
    Entry::Action(Action::Search),
    Entry::Action(Action::CopyMode),
    Entry::Action(Action::PasteHistory),
    Entry::Separator,
    Entry::Action(Action::ZoomIn),
    Entry::Action(Action::ZoomOut),
    Entry::Action(Action::ZoomReset),
    Entry::Separator,
    Entry::Action(Action::ToggleBroadcast),
    Entry::Action(Action::TogglePaneZoom),
    Entry::Action(Action::DuplicateTab),
    Entry::Action(Action::RestartTab),
];

const VIEW: &[Entry] = &[
    Entry::Action(Action::ToggleSidebar),
    Entry::Action(Action::SidebarFilter),
    Entry::Separator,
    Entry::Action(Action::ShowAttention),
    Entry::Action(Action::JumpAttention),
    Entry::Separator,
    Entry::Action(Action::NewWorkspace),
];

const WINDOW: &[Entry] = &[
    Entry::Action(Action::NewWindow),
    Entry::Action(Action::NextWindow),
    Entry::Separator,
    Entry::Action(Action::Settings),
    Entry::Separator,
    Entry::Quit,
];

/// The entries of the menu at `menu`, or an empty slice for an unknown index.
pub fn entries(menu: usize) -> &'static [Entry] {
    match menu {
        0 => BERTH,
        1 => TERMINAL,
        2 => VIEW,
        3 => WINDOW,
        _ => &[],
    }
}

impl Entry {
    /// The row's label; empty for a separator.
    pub fn label(self) -> &'static str {
        match self {
            Self::Action(action) => action.label(),
            Self::Quit => "Quit Berth",
            Self::Separator => "",
        }
    }

    /// The action whose chord is shown beside the label, if any.
    pub fn action(self) -> Option<Action> {
        match self {
            Self::Action(action) => Some(action),
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
}
