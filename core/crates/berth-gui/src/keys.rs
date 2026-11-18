//! Keyboard resolution: config chords first, then the fixed bindings that
//! every terminal window needs.
//!
//! The portable window claims a chord only when it matches exactly, so plain
//! Ctrl+C still reaches the shell as SIGINT and `cmd` (Super) stays available
//! for the desktop unless a config binding asks for it.

use winit::keyboard::{Key, ModifiersState, NamedKey};

use crate::config::{Action, Binding, Shortcuts};

/// Lines per wheel notch and per Shift+Page key.
pub const SCROLL_STEP: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    SplitRight,
    SplitDown,
    NewTab,
    CloseTab,
    NextTab,
    PreviousTab,
    NextPane,
    PreviousPane,
    NewWorkspace,
    Palette,
    JumpAttention,
    ShowAttention,
    ToggleSidebar,
    Settings,
    /// Select the nth workspace, one-based.
    Workspace(usize),
    Copy,
    Paste,
    ScrollUp(usize),
    ScrollDown(usize),
    ScrollTop,
    ScrollBottom,
}

impl Command {
    pub fn for_action(action: Action) -> Self {
        match action {
            Action::SplitRight => Command::SplitRight,
            Action::SplitDown => Command::SplitDown,
            Action::NewTab => Command::NewTab,
            Action::CloseTab => Command::CloseTab,
            Action::NextTab => Command::NextTab,
            Action::PreviousTab => Command::PreviousTab,
            Action::NextPane => Command::NextPane,
            Action::PreviousPane => Command::PreviousPane,
            Action::NewWorkspace => Command::NewWorkspace,
            Action::Palette => Command::Palette,
            Action::JumpAttention => Command::JumpAttention,
            Action::ShowAttention => Command::ShowAttention,
            Action::ToggleSidebar => Command::ToggleSidebar,
            Action::Settings => Command::Settings,
        }
    }
}

/// The chord key as the config spells it: lowercase letters and named keys.
pub fn chord_key(key: &Key) -> Option<String> {
    Some(match key {
        Key::Character(text) => text.to_lowercase(),
        Key::Named(NamedKey::PageUp) => "pageup".into(),
        Key::Named(NamedKey::PageDown) => "pagedown".into(),
        Key::Named(NamedKey::Home) => "home".into(),
        Key::Named(NamedKey::End) => "end".into(),
        Key::Named(NamedKey::Enter) => "enter".into(),
        Key::Named(NamedKey::Escape) => "escape".into(),
        Key::Named(NamedKey::Tab) => "tab".into(),
        Key::Named(NamedKey::Space) => "space".into(),
        Key::Named(NamedKey::ArrowUp) => "up".into(),
        Key::Named(NamedKey::ArrowDown) => "down".into(),
        Key::Named(NamedKey::ArrowLeft) => "left".into(),
        Key::Named(NamedKey::ArrowRight) => "right".into(),
        Key::Named(NamedKey::F1) => "f1".into(),
        Key::Named(NamedKey::F2) => "f2".into(),
        Key::Named(NamedKey::F3) => "f3".into(),
        Key::Named(NamedKey::F4) => "f4".into(),
        Key::Named(NamedKey::F5) => "f5".into(),
        Key::Named(NamedKey::F6) => "f6".into(),
        Key::Named(NamedKey::F7) => "f7".into(),
        Key::Named(NamedKey::F8) => "f8".into(),
        Key::Named(NamedKey::F9) => "f9".into(),
        Key::Named(NamedKey::F10) => "f10".into(),
        Key::Named(NamedKey::F11) => "f11".into(),
        Key::Named(NamedKey::F12) => "f12".into(),
        _ => return None,
    })
}

/// Does this chord match the key and modifier state exactly?
pub fn matches(binding: &Binding, key: &Key, modifiers: ModifiersState) -> bool {
    let Some(chord) = chord_key(key) else {
        return false;
    };
    binding.key == chord
        && binding.cmd == modifiers.super_key()
        && binding.ctrl == modifiers.control_key()
        && binding.alt == modifiers.alt_key()
        && binding.shift == modifiers.shift_key()
}

/// Resolve a key press into a window command, or `None` when it belongs to the
/// shell. `repeat` is ignored for app commands, so holding a chord does not
/// open dozens of tabs.
pub fn resolve(
    key: &Key,
    modifiers: ModifiersState,
    repeat: bool,
    shortcuts: &Shortcuts,
) -> Option<Command> {
    for action in Action::ALL {
        if let Some(binding) = shortcuts.binding(action)
            && !repeat
            && matches(binding, key, modifiers)
        {
            return Some(Command::for_action(action));
        }
    }

    // Fixed bindings, the way macOS keeps ⌘1…⌘9 and ⌘Q out of the settings
    // table. Super on every platform; Ctrl works on Linux and Windows where
    // Super belongs to the desktop.
    let primary = modifiers.super_key() || (!cfg!(target_os = "macos") && modifiers.control_key());
    if primary
        && !modifiers.alt_key()
        && let Key::Character(text) = key
        && !modifiers.shift_key()
        && text.len() == 1
        && let Some(digit) = text.chars().next().and_then(|c| c.to_digit(10))
        && digit >= 1
    {
        return Some(Command::Workspace(digit as usize));
    }

    match key {
        Key::Character(text) => {
            let text = text.to_lowercase();
            let copy = (modifiers.super_key() && !modifiers.control_key())
                || (modifiers.control_key() && modifiers.shift_key());
            if copy && text == "c" && !modifiers.alt_key() {
                return Some(Command::Copy);
            }
            if copy && text == "v" && !modifiers.alt_key() {
                return Some(Command::Paste);
            }
        }
        Key::Named(NamedKey::PageUp) if modifiers.shift_key() && !modifiers.control_key() => {
            return Some(Command::ScrollUp(SCROLL_STEP * 4));
        }
        Key::Named(NamedKey::PageDown) if modifiers.shift_key() && !modifiers.control_key() => {
            return Some(Command::ScrollDown(SCROLL_STEP * 4));
        }
        Key::Named(NamedKey::Home) if modifiers.shift_key() => {
            return Some(Command::ScrollTop);
        }
        Key::Named(NamedKey::End) if modifiers.shift_key() => {
            return Some(Command::ScrollBottom);
        }
        _ => {}
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn shortcuts() -> Shortcuts {
        Shortcuts::build(&HashMap::new())
    }

    #[test]
    fn given_the_mac_split_chord_when_pressed_then_it_splits_right() {
        let shortcuts = Shortcuts::build(&HashMap::new());
        let binding = shortcuts.binding(Action::SplitRight).unwrap().clone();
        let mut modifiers = ModifiersState::empty();
        if binding.cmd {
            modifiers |= ModifiersState::SUPER;
        }
        if binding.ctrl {
            modifiers |= ModifiersState::CONTROL;
        }
        if binding.alt {
            modifiers |= ModifiersState::ALT;
        }
        if binding.shift {
            modifiers |= ModifiersState::SHIFT;
        }
        let command = resolve(&Key::Character("d".into()), modifiers, false, &shortcuts);
        assert_eq!(command, Some(Command::SplitRight));
    }

    #[test]
    fn given_plain_ctrl_c_when_pressed_then_the_shell_keeps_it() {
        let command = resolve(
            &Key::Character("c".into()),
            ModifiersState::CONTROL,
            false,
            &shortcuts(),
        );
        assert_eq!(command, None);
    }

    #[test]
    fn given_cmd_c_when_pressed_then_it_is_a_copy() {
        let command = resolve(
            &Key::Character("c".into()),
            ModifiersState::SUPER,
            false,
            &shortcuts(),
        );
        assert_eq!(command, Some(Command::Copy));
    }

    #[test]
    fn given_shift_page_up_when_pressed_then_the_view_scrolls() {
        let command = resolve(
            &Key::Named(NamedKey::PageUp),
            ModifiersState::SHIFT,
            false,
            &shortcuts(),
        );
        assert_eq!(command, Some(Command::ScrollUp(SCROLL_STEP * 4)));
    }

    #[test]
    fn given_a_held_chord_when_repeated_then_nothing_is_returned() {
        let shortcuts = Shortcuts::build(&HashMap::new());
        let binding = shortcuts.binding(Action::NewTab).unwrap().clone();
        let mut modifiers = ModifiersState::empty();
        if binding.cmd {
            modifiers |= ModifiersState::SUPER;
        }
        if binding.ctrl {
            modifiers |= ModifiersState::CONTROL;
        }
        if binding.shift {
            modifiers |= ModifiersState::SHIFT;
        }
        assert_eq!(
            resolve(&Key::Character("t".into()), modifiers, true, &shortcuts),
            None
        );
    }

    #[test]
    fn given_a_digit_when_primary_is_held_then_it_selects_a_workspace() {
        let command = resolve(
            &Key::Character("3".into()),
            ModifiersState::SUPER,
            false,
            &shortcuts(),
        );
        assert_eq!(command, Some(Command::Workspace(3)));
    }

    #[test]
    fn given_a_function_key_when_no_chord_matches_then_it_belongs_to_the_shell() {
        let command = resolve(
            &Key::Named(NamedKey::F5),
            ModifiersState::empty(),
            false,
            &shortcuts(),
        );
        assert_eq!(command, None);
    }
}
