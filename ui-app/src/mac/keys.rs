//! AppKit key events → kit input. Keys are reported as Windows virtual-key codes, which the
//! kit's keybind widget names; ⌘ is the Windows key (raw code only), ⌥ is Alt.

use objc2_app_kit::{NSEvent, NSEventModifierFlags};
use poly_ui::input::{Collector, vk};

/// macOS virtual key code (`kVK_*`, layout independent) → Windows virtual-key code.
pub fn virtual_key(code: u16) -> Option<u32> {
    const LETTERS: [(u16, u8); 26] = [
        (0x00, b'A'),
        (0x0B, b'B'),
        (0x08, b'C'),
        (0x02, b'D'),
        (0x0E, b'E'),
        (0x03, b'F'),
        (0x05, b'G'),
        (0x04, b'H'),
        (0x22, b'I'),
        (0x26, b'J'),
        (0x28, b'K'),
        (0x25, b'L'),
        (0x2E, b'M'),
        (0x2D, b'N'),
        (0x1F, b'O'),
        (0x23, b'P'),
        (0x0C, b'Q'),
        (0x0F, b'R'),
        (0x01, b'S'),
        (0x11, b'T'),
        (0x20, b'U'),
        (0x09, b'V'),
        (0x0D, b'W'),
        (0x07, b'X'),
        (0x10, b'Y'),
        (0x06, b'Z'),
    ];
    const DIGITS: [u16; 10] = [0x1D, 0x12, 0x13, 0x14, 0x15, 0x17, 0x16, 0x1A, 0x1C, 0x19];
    const KEYPAD: [u16; 10] = [0x52, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5B, 0x5C];
    const FUNCTION: [u16; 20] = [
        0x7A, 0x78, 0x63, 0x76, 0x60, 0x61, 0x62, 0x64, 0x65, 0x6D, 0x67, 0x6F, 0x69, 0x6B, 0x71,
        0x6A, 0x40, 0x4F, 0x50, 0x5A,
    ];
    if let Some(&(_, c)) = LETTERS.iter().find(|(k, _)| *k == code) {
        return Some(u32::from(c));
    }
    if let Some(i) = DIGITS.iter().position(|&k| k == code) {
        return Some(0x30 + i as u32);
    }
    if let Some(i) = KEYPAD.iter().position(|&k| k == code) {
        return Some(vk::NUMPAD0 + i as u32);
    }
    if let Some(i) = FUNCTION.iter().position(|&k| k == code) {
        return Some(vk::F1 + i as u32);
    }
    Some(match code {
        0x24 | 0x4C => vk::RETURN,
        0x30 => vk::TAB,
        0x31 => vk::SPACE,
        0x33 => vk::BACK,
        0x35 => vk::ESCAPE,
        0x75 => vk::DELETE,
        0x72 => vk::INSERT,
        0x73 => vk::HOME,
        0x77 => vk::END,
        0x74 => vk::PRIOR,
        0x79 => vk::NEXT,
        0x7B => vk::LEFT,
        0x7C => vk::RIGHT,
        0x7D => vk::DOWN,
        0x7E => vk::UP,
        0x38 => vk::LSHIFT,
        0x3C => vk::RSHIFT,
        0x3B => vk::LCONTROL,
        0x3E => vk::RCONTROL,
        0x3A => vk::LMENU,
        0x3D => vk::RMENU,
        0x37 => LWIN,
        0x36 => RWIN,
        0x39 => vk::CAPITAL,
        0x32 => vk::OEM_3,
        0x29 => 0xBA, // ;
        0x18 => 0xBB, // =
        0x2B => 0xBC, // ,
        0x1B => 0xBD, // -
        0x2F => 0xBE, // .
        0x2C => 0xBF, // /
        0x21 => 0xDB, // [
        0x2A => 0xDC, // \
        0x1E => 0xDD, // ]
        0x27 => 0xDE, // '
        _ => return None,
    })
}

const LWIN: u32 = 0x5B;
const RWIN: u32 = 0x5C;

/// Device-dependent modifier bits of `NSEvent.modifierFlags` (`NX_DEVICE*KEYMASK`): which
/// side of a modifier is held.
const DEVICE_BITS: [(u32, usize); 8] = [
    (vk::LCONTROL, 0x0001),
    (vk::LSHIFT, 0x0002),
    (vk::RSHIFT, 0x0004),
    (LWIN, 0x0008),
    (RWIN, 0x0010),
    (vk::LMENU, 0x0020),
    (vk::RMENU, 0x0040),
    (vk::RCONTROL, 0x2000),
];

pub fn key_down(input: &mut Collector, event: &NSEvent) {
    let Some(code) = virtual_key(event.keyCode()) else {
        return;
    };
    let flags = event.modifierFlags();
    let command = flags.contains(NSEventModifierFlags::Command);
    let control = flags.contains(NSEventModifierFlags::Control);
    // ⌘A and ⌃A select all, as Ctrl+A on Windows.
    if code == u32::from(b'A') && (command || control) {
        input.select_all();
        return;
    }
    input.key(code, true);
    if command || control {
        return;
    }
    if let Some(text) = event.characters() {
        // Arrow and function keys arrive as private-use characters.
        let text: String = text
            .to_string()
            .chars()
            .filter(|c| !('\u{F700}'..='\u{F8FF}').contains(c))
            .collect();
        input.text(&text);
    }
}

pub fn key_up(input: &mut Collector, event: &NSEvent) {
    if let Some(code) = virtual_key(event.keyCode()) {
        input.key(code, false);
    }
}

/// A modifier went down or up: which one from the key code, which way from its side's
/// device bit.
pub fn flags_changed(input: &mut Collector, event: &NSEvent) {
    let Some(code) = virtual_key(event.keyCode()) else {
        return;
    };
    let flags = event.modifierFlags().0;
    if let Some(&(_, bit)) = DEVICE_BITS.iter().find(|(c, _)| *c == code) {
        input.key(code, flags & bit != 0);
    } else if code == vk::CAPITAL {
        input.key(code, flags & NSEventModifierFlags::CapsLock.0 != 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_independent_codes_map_to_virtual_keys() {
        assert_eq!(virtual_key(0x00), Some(u32::from(b'A')));
        assert_eq!(virtual_key(0x06), Some(u32::from(b'Z')));
        assert_eq!(virtual_key(0x1D), Some(0x30), "0");
        assert_eq!(virtual_key(0x19), Some(0x39), "9");
        assert_eq!(virtual_key(0x7A), Some(vk::F1));
        assert_eq!(virtual_key(0x6F), Some(vk::F1 + 11), "F12");
        assert_eq!(virtual_key(0x53), Some(vk::NUMPAD0 + 1));
        assert_eq!(virtual_key(0x33), Some(vk::BACK));
        assert_eq!(virtual_key(0x75), Some(vk::DELETE));
        assert_eq!(virtual_key(0x7B), Some(vk::LEFT));
        assert_eq!(virtual_key(0x3A), Some(vk::LMENU), "⌥ is Alt");
        assert_eq!(virtual_key(0x3F), None, "fn");
    }
}
