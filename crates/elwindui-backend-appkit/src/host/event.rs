//! `NSEvent` -> `elwindui_core::input` translation. Pure value mapping; the dispatch itself
//! lives on `TreeHostView` in this module's parent.

use elwindui_core::input::{Key, KeyModifiers};
use objc2_app_kit::{NSEvent, NSEventModifierFlags};

/// `NSEvent.modifierFlags()` -> `elwindui_core::input::KeyModifiers`.
pub(crate) fn nsevent_modifiers(event: &NSEvent) -> KeyModifiers {
    let flags = event.modifierFlags();
    KeyModifiers {
        shift: flags.contains(NSEventModifierFlags::Shift),
        control: flags.contains(NSEventModifierFlags::Control),
        alt: flags.contains(NSEventModifierFlags::Option),
        meta: flags.contains(NSEventModifierFlags::Command),
    }
}

/// `NSEvent.keyCode()` (a fixed physical-key code, not layout-remapped) -> `elwindui_core::input::
/// Key` for the named keys `Key` distinguishes; every other key falls back to
/// `charactersIgnoringModifiers()`'s first character (`Key::Character`, layout-dependent —
/// see that variant's own doc comment). The named-key codes below are macOS's standard (and
/// long-stable) virtual keycodes for the US keyboard's physical key positions.
pub(crate) fn nsevent_key(event: &NSEvent) -> Option<Key> {
    let key = match event.keyCode() {
        36 => Some(Key::Enter),
        48 => Some(Key::Tab),
        49 => Some(Key::Space),
        51 => Some(Key::Backspace),
        53 => Some(Key::Escape),
        117 => Some(Key::Delete),
        115 => Some(Key::Home),
        119 => Some(Key::End),
        116 => Some(Key::PageUp),
        121 => Some(Key::PageDown),
        123 => Some(Key::Left),
        124 => Some(Key::Right),
        125 => Some(Key::Down),
        126 => Some(Key::Up),
        122 => Some(Key::F1),
        120 => Some(Key::F2),
        99 => Some(Key::F3),
        118 => Some(Key::F4),
        96 => Some(Key::F5),
        97 => Some(Key::F6),
        98 => Some(Key::F7),
        100 => Some(Key::F8),
        101 => Some(Key::F9),
        109 => Some(Key::F10),
        103 => Some(Key::F11),
        111 => Some(Key::F12),
        _ => None,
    };
    key.or_else(|| {
        event
            .charactersIgnoringModifiers()
            .and_then(|s| s.to_string().chars().next())
            .map(Key::Character)
    })
}
