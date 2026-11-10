// NSEvent <-> ghostty_input_key_s translation.
//
// A trimmed port of Ghostty's own macOS mapping (macos/Sources/Ghostty/
// Ghostty.Input.swift + NSEvent+Extension.swift), keeping the heuristics that
// decide what text is attached to a key event. Ghostty encodes control keys
// itself from (keycode, mods, text), so we must not pre-encode C-c into 0x03.
//
// Not here yet: IME/dead keys (needs NSTextInputClient + interpretKeyEvents),
// mouse, and key tables. See docs/05-build.md for the M0 gap list.

import AppKit
import GhosttyKit

enum BerthInput {
    /// NSEvent modifiers -> Ghostty modifier mask.
    static func mods(_ flags: NSEvent.ModifierFlags) -> ghostty_input_mods_e {
        var mods = GHOSTTY_MODS_NONE.rawValue
        if flags.contains(.shift) { mods |= GHOSTTY_MODS_SHIFT.rawValue }
        if flags.contains(.control) { mods |= GHOSTTY_MODS_CTRL.rawValue }
        if flags.contains(.option) { mods |= GHOSTTY_MODS_ALT.rawValue }
        if flags.contains(.command) { mods |= GHOSTTY_MODS_SUPER.rawValue }
        if flags.contains(.capsLock) { mods |= GHOSTTY_MODS_CAPS.rawValue }
        return ghostty_input_mods_e(mods)
    }

    /// Ghostty modifier mask -> NSEvent modifiers (ignoring sided bits).
    static func flags(_ mods: ghostty_input_mods_e) -> NSEvent.ModifierFlags {
        var flags: NSEvent.ModifierFlags = []
        if mods.rawValue & GHOSTTY_MODS_SHIFT.rawValue != 0 { flags.insert(.shift) }
        if mods.rawValue & GHOSTTY_MODS_CTRL.rawValue != 0 { flags.insert(.control) }
        if mods.rawValue & GHOSTTY_MODS_ALT.rawValue != 0 { flags.insert(.option) }
        if mods.rawValue & GHOSTTY_MODS_SUPER.rawValue != 0 { flags.insert(.command) }
        return flags
    }

    /// The modifiers that participate in character translation, after Ghostty
    /// applies config such as `macos-option-as-alt`. The raw event flags carry
    /// hidden bits that dead keys depend on, so we only adjust the four we know.
    static func translationFlags(_ event: NSEvent, surface: ghostty_surface_t) -> NSEvent.ModifierFlags {
        let translated = flags(
            ghostty_surface_key_translation_mods(surface, mods(event.modifierFlags)))
        var result = event.modifierFlags
        for flag in [NSEvent.ModifierFlags.shift, .control, .option, .command] {
            if translated.contains(flag) { result.insert(flag) } else { result.remove(flag) }
        }
        return result
    }
}

/// One keyboard event in the shape Ghostty wants.
struct GhosttyKeyEvent {
    var action: ghostty_input_action_e
    var mods: ghostty_input_mods_e
    var consumedMods: ghostty_input_mods_e
    var keycode: UInt32
    var text: String?
    var unshiftedCodepoint: UInt32 = 0
    var composing: Bool = false

    /// Run `body` with the C struct. The text pointer is only valid for the
    /// duration of the closure, which is all Ghostty needs (it copies).
    @discardableResult
    func withCValue<T>(_ body: (ghostty_input_key_s) -> T) -> T {
        var cEvent = ghostty_input_key_s()
        cEvent.action = action
        cEvent.keycode = keycode
        cEvent.mods = mods
        cEvent.consumed_mods = consumedMods
        cEvent.unshifted_codepoint = unshiftedCodepoint
        cEvent.composing = composing

        guard let text = text?.keyEventText else {
            cEvent.text = nil
            return body(cEvent)
        }
        return text.withCString { ptr in
            cEvent.text = ptr
            return body(cEvent)
        }
    }

    /// Hand the event to Ghostty. Returns true when Ghostty consumed it
    /// (encoded it or performed a binding).
    @discardableResult
    func send(to surface: ghostty_surface_t) -> Bool {
        withCValue { ghostty_surface_key(surface, $0) }
    }

    /// Would this chord trigger a Ghostty binding right now?
    ///
    /// Needed in `performKeyEquivalent`: AppKit offers Command chords to the
    /// menu bar first, so we must claim the ones Ghostty binds before the menu
    /// steals them (⌘C copy, ⌘K clear, custom binds…).
    func isBinding(on surface: ghostty_surface_t) -> Bool {
        withCValue { ghostty_surface_key_is_binding(surface, $0, nil) }
    }
}

extension String {
    /// The text to attach to a key event, or nil when it is empty or starts
    /// with an ASCII control character. Control characters are encoded by
    /// Ghostty from the physical key so protocols like Kitty keep working.
    var keyEventText: String? {
        guard let first = unicodeScalars.first else { return nil }
        guard first.value >= 0x20, first.value != 0x7F else { return nil }
        return self
    }
}

extension NSEvent {
    /// Build a Ghostty key event from this NSEvent.
    ///
    /// `translationFlags` should be the modifiers used for character
    /// translation (see `BerthInput.translationFlags`); they decide what text
    /// is attached and which modifiers count as "consumed" by producing it.
    func berthKeyEvent(
        _ action: ghostty_input_action_e,
        translationFlags: NSEvent.ModifierFlags? = nil
    ) -> GhosttyKeyEvent {
        let translation = translationFlags ?? modifierFlags

        var event = GhosttyKeyEvent(
            action: action,
            // Control and Command never contribute to text translation on
            // macOS; everything else is assumed to have been consumed.
            mods: BerthInput.mods(modifierFlags),
            consumedMods: BerthInput.mods(translation.subtracting([.control, .command])),
            keycode: UInt32(keyCode),
            text: nil
        )

        // Only physical key events carry characters. A release usually has
        // none, and Ghostty does not need text for it.
        if type == .keyDown || type == .keyUp {
            event.text = berthCharacters(translationFlags: translation)

            // Codepoint with no modifiers at all. `charactersIgnoringModifiers`
            // changes behaviour when Control is held, so ask for the bare
            // layout result instead.
            if let scalar = characters(byApplyingModifiers: [])?.unicodeScalars.first {
                event.unshiftedCodepoint = scalar.value
            }
        }

        return event
    }

    /// The characters to attach to a key event, matching Ghostty's heuristics.
    private func berthCharacters(translationFlags: NSEvent.ModifierFlags) -> String? {
        guard let characters else { return nil }

        if characters.count == 1, let scalar = characters.unicodeScalars.first {
            // A single control character means the layout produced a C0 byte;
            // strip Control so Ghostty can encode the chord from mods + key.
            if scalar.value < 0x20 {
                return self.characters(byApplyingModifiers: translationFlags.subtracting(.control))
            }

            // Private Use Area values are how AppKit reports function keys.
            if scalar.value >= 0xF700, scalar.value <= 0xF8FF {
                return nil
            }
        }

        return characters
    }
}
