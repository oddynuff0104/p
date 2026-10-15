//! Platform input → [`KitInput`] ([`Collector`]): the host feeds pointer, button, wheel,
//! key and text events between two frames, the frame takes one [`KitInput`]. Key codes
//! are Windows virtual-key codes on every platform (the kit's keybind names use them);
//! [`Collector::feed`] translates Win32 window messages directly.

#[cfg(windows)]
use windows_sys::Win32::Foundation::WPARAM;
#[cfg(windows)]
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::kit::{Key, KitInput};

/// Virtual-key codes the collector itself interprets (same values as `VK_*`).
pub mod vk {
    pub const LBUTTON: u32 = 0x01;
    pub const RBUTTON: u32 = 0x02;
    pub const MBUTTON: u32 = 0x04;
    pub const XBUTTON1: u32 = 0x05;
    pub const XBUTTON2: u32 = 0x06;
    pub const BACK: u32 = 0x08;
    pub const TAB: u32 = 0x09;
    pub const RETURN: u32 = 0x0D;
    pub const SHIFT: u32 = 0x10;
    pub const CONTROL: u32 = 0x11;
    pub const MENU: u32 = 0x12;
    pub const CAPITAL: u32 = 0x14;
    pub const ESCAPE: u32 = 0x1B;
    pub const SPACE: u32 = 0x20;
    pub const PRIOR: u32 = 0x21;
    pub const NEXT: u32 = 0x22;
    pub const END: u32 = 0x23;
    pub const HOME: u32 = 0x24;
    pub const LEFT: u32 = 0x25;
    pub const UP: u32 = 0x26;
    pub const RIGHT: u32 = 0x27;
    pub const DOWN: u32 = 0x28;
    pub const INSERT: u32 = 0x2D;
    pub const DELETE: u32 = 0x2E;
    pub const NUMPAD0: u32 = 0x60;
    pub const F1: u32 = 0x70;
    pub const LSHIFT: u32 = 0xA0;
    pub const RSHIFT: u32 = 0xA1;
    pub const LCONTROL: u32 = 0xA2;
    pub const RCONTROL: u32 = 0xA3;
    pub const LMENU: u32 = 0xA4;
    pub const RMENU: u32 = 0xA5;
    pub const OEM_3: u32 = 0xC0;
}

/// Input between two menu frames: the window procedure feeds messages, the frame takes
/// one [`KitInput`].
#[derive(Default)]
pub struct Collector {
    mouse: [f32; 2],
    down: [bool; 2],
    /// Pressed since the last take: a click within one frame is not lost.
    pressed: [bool; 2],
    wheel: f32,
    keys: Vec<Key>,
    text: String,
    /// Shift, Ctrl, Alt held (from key down/up, deterministic without polling).
    modifiers: [bool; 3],
    /// Raw virtual-key codes since the last take (keyboard and mouse buttons, for keybinds).
    pressed_vk: Vec<u32>,
}

/// At most this many raw keys per frame (guards against key repeat without a take).
const MAX_PRESSED: usize = 32;

impl Collector {
    pub fn take(&mut self) -> KitInput {
        let input = KitInput {
            mouse: self.mouse,
            down: [
                self.down[0] || self.pressed[0],
                self.down[1] || self.pressed[1],
            ],
            wheel: std::mem::take(&mut self.wheel),
            keys: std::mem::take(&mut self.keys),
            text: std::mem::take(&mut self.text),
            shift: self.modifiers[0],
            ctrl: self.modifiers[1],
            alt: self.modifiers[2],
            pressed_vk: std::mem::take(&mut self.pressed_vk),
            dt: 0.0,
        };
        self.pressed = [false; 2];
        input
    }
    /// The pointer left the surface: nothing is hovered any more.
    pub fn leave(&mut self) {
        self.mouse = [-1.0, -1.0];
    }
    /// Pointer position in surface pixels.
    pub fn pointer(&mut self, pos: [f32; 2]) {
        self.mouse = pos;
    }
    /// Mouse button 0 (left) to 4 (X2) pressed or released; 0 and 1 drive the kit, every
    /// press also records its raw code.
    pub fn button(&mut self, button: usize, down: bool) {
        if let Some(b) = self.down.get_mut(button) {
            *b = down;
            if down {
                self.pressed[button] = true;
            }
        }
        if down {
            let codes = [
                vk::LBUTTON,
                vk::RBUTTON,
                vk::MBUTTON,
                vk::XBUTTON1,
                vk::XBUTTON2,
            ];
            if let Some(&code) = codes.get(button) {
                self.press_vk(code);
            }
        }
    }
    /// Wheel in notches (positive scrolls up).
    pub fn wheel(&mut self, notches: f32) {
        self.wheel += notches;
    }
    /// A key (virtual-key code) went down or up; modifiers are tracked from these.
    pub fn key(&mut self, code: u32, down: bool) {
        let modifier = match code {
            vk::SHIFT | vk::LSHIFT | vk::RSHIFT => Some(0),
            vk::CONTROL | vk::LCONTROL | vk::RCONTROL => Some(1),
            vk::MENU | vk::LMENU | vk::RMENU => Some(2),
            _ => None,
        };
        if let Some(m) = modifier {
            self.modifiers[m] = down;
        }
        if !down {
            return;
        }
        self.press_vk(code);
        let key = match code {
            vk::TAB => Some(Key::Tab),
            vk::BACK => Some(Key::Backspace),
            vk::RETURN => Some(Key::Enter),
            vk::SPACE => Some(Key::Space),
            vk::ESCAPE => Some(Key::Escape),
            vk::DELETE => Some(Key::Delete),
            vk::LEFT => Some(Key::Left),
            vk::RIGHT => Some(Key::Right),
            vk::UP => Some(Key::Up),
            vk::DOWN => Some(Key::Down),
            vk::HOME => Some(Key::Home),
            vk::END => Some(Key::End),
            _ => None,
        };
        if let Some(k) = key {
            self.push_key(k);
        }
    }
    /// Select-all shortcut (Ctrl+A, ⌘A).
    pub fn select_all(&mut self) {
        self.push_key(Key::SelectAll);
    }
    /// Typed text; control characters are dropped.
    pub fn text(&mut self, text: &str) {
        for c in text.chars().filter(|c| !c.is_control()) {
            if self.text.len() >= 64 {
                break;
            }
            self.text.push(c);
        }
    }
    /// Focus lost: held buttons and modifiers are released, nothing is hovered.
    pub fn reset(&mut self) {
        *self = Self::default();
        self.leave();
    }
    fn push_key(&mut self, key: Key) {
        if self.keys.len() < MAX_PRESSED {
            self.keys.push(key);
        }
    }
    fn press_vk(&mut self, vk: u32) {
        if self.pressed_vk.len() < MAX_PRESSED && !self.pressed_vk.contains(&vk) {
            self.pressed_vk.push(vk);
        }
    }
    /// Translates a window message (mouse position already in backbuffer pixels).
    #[cfg(windows)]
    pub fn feed(&mut self, msg: u32, wparam: WPARAM, pixel: Option<(u32, u32)>) {
        if let Some((x, y)) = pixel {
            self.pointer([x as f32, y as f32]);
        }
        match msg {
            WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => self.button(0, true),
            WM_RBUTTONDOWN | WM_RBUTTONDBLCLK => self.button(1, true),
            WM_LBUTTONUP => self.button(0, false),
            WM_RBUTTONUP => self.button(1, false),
            WM_MBUTTONDOWN | WM_MBUTTONDBLCLK => self.button(2, true),
            // HIWORD: XBUTTON1 = 1 (mouse 4), XBUTTON2 = 2 (mouse 5).
            WM_XBUTTONDOWN | WM_XBUTTONDBLCLK => match (wparam >> 16) as u16 {
                1 => self.button(3, true),
                2 => self.button(4, true),
                _ => {}
            },
            WM_MOUSEWHEEL => self.wheel((wparam >> 16) as u16 as i16 as f32 / 120.0),
            WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP => self.key(
                u32::from(wparam as u16),
                matches!(msg, WM_KEYDOWN | WM_SYSKEYDOWN),
            ),
            // Ctrl+A arrives as the control character 0x01 after TranslateMessage.
            WM_CHAR if wparam == 0x01 => self.select_all(),
            WM_CHAR => {
                if let Some(c) = char::from_u32(wparam as u32) {
                    self.text(c.encode_utf8(&mut [0; 4]));
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_neutral_events() {
        let mut c = Collector::default();
        c.pointer([10.0, 20.0]);
        c.button(0, true);
        c.button(0, false);
        c.button(3, true);
        c.key(vk::LSHIFT, true);
        c.key(vk::LEFT, true);
        c.key(0x41, true);
        c.select_all();
        c.text("a\u{8}b");
        c.wheel(-0.5);
        let first = c.take();
        assert_eq!(first.mouse, [10.0, 20.0]);
        assert!(first.down[0] && first.shift && !first.ctrl);
        assert_eq!(first.keys, vec![Key::Left, Key::SelectAll]);
        assert_eq!(first.text, "ab");
        assert_eq!(first.wheel, -0.5);
        assert_eq!(first.pressed_vk, vec![0x01, 0x05, 0xA0, 0x25, 0x41]);
        c.reset();
        let second = c.take();
        assert!(!second.down[0] && !second.shift && second.mouse == [-1.0, -1.0]);
    }
}

#[cfg(all(test, windows))]
mod win_tests {
    use super::*;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;

    #[test]
    fn collector_keeps_clicks_within_one_frame_and_text() {
        let mut c = Collector::default();
        c.feed(WM_MOUSEMOVE, 0, Some((40, 50)));
        c.feed(WM_LBUTTONDOWN, 0, Some((41, 51)));
        c.feed(WM_LBUTTONUP, 0, Some((41, 51)));
        c.feed(WM_MOUSEWHEEL, (120usize) << 16, None);
        c.feed(WM_CHAR, 'a' as usize, None);
        c.feed(WM_CHAR, 0x08, None);
        c.feed(WM_KEYDOWN, VK_BACK as usize, None);
        c.feed(WM_KEYDOWN, VK_LEFT as usize, None);
        c.feed(WM_KEYDOWN, VK_DELETE as usize, None);
        c.feed(WM_CHAR, 0x01, None);
        let first = c.take();
        assert_eq!(first.mouse, [41.0, 51.0]);
        assert!(
            first.down[0],
            "press and release in one frame still presses"
        );
        assert_eq!(first.wheel, 1.0);
        assert_eq!(first.text, "a");
        assert_eq!(
            first.keys,
            vec![Key::Backspace, Key::Left, Key::Delete, Key::SelectAll]
        );
        let second = c.take();
        assert!(!second.down[0], "released in the next frame");
        assert!(second.text.is_empty() && second.keys.is_empty() && second.wheel == 0.0);
    }

    #[test]
    fn collector_navigation_keys_modifiers_and_raw_codes() {
        let mut c = Collector::default();
        c.feed(WM_KEYDOWN, VK_SHIFT as usize, None);
        c.feed(WM_KEYDOWN, VK_TAB as usize, None);
        for vk in [VK_UP, VK_DOWN, VK_LEFT, VK_RIGHT, VK_HOME, VK_END] {
            c.feed(WM_KEYDOWN, vk as usize, None);
        }
        c.feed(WM_KEYDOWN, VK_RETURN as usize, None);
        c.feed(WM_KEYDOWN, VK_SPACE as usize, None);
        c.feed(WM_KEYDOWN, VK_ESCAPE as usize, None);
        c.feed(WM_KEYDOWN, VK_CONTROL as usize, None);
        c.feed(WM_SYSKEYDOWN, VK_MENU as usize, None);
        c.feed(WM_KEYDOWN, 0x46, None); // F: no menu key, only the raw code
        let first = c.take();
        assert_eq!(
            first.keys,
            vec![
                Key::Tab,
                Key::Up,
                Key::Down,
                Key::Left,
                Key::Right,
                Key::Home,
                Key::End,
                Key::Enter,
                Key::Space,
                Key::Escape
            ]
        );
        assert!(first.shift && first.ctrl && first.alt);
        assert!(first.pressed_vk.contains(&0x46) && first.pressed_vk.contains(&0x09));
        assert_eq!(first.dt, 0.0, "the overlay sets dt");
        // Modifiers stay held across frames until released; raw codes do not repeat.
        c.feed(WM_KEYUP, VK_SHIFT as usize, None);
        c.feed(WM_SYSKEYUP, VK_MENU as usize, None);
        let second = c.take();
        assert!(!second.shift && second.ctrl && !second.alt);
        assert!(second.pressed_vk.is_empty() && second.keys.is_empty());
        // Mouse buttons 1–5 as raw codes (keybind capture); X buttons from HIWORD.
        c.feed(WM_LBUTTONDOWN, 0, Some((1, 1)));
        c.feed(WM_RBUTTONDOWN, 0, Some((1, 1)));
        c.feed(WM_MBUTTONDOWN, 0, Some((1, 1)));
        c.feed(WM_XBUTTONDOWN, 1 << 16, Some((1, 1)));
        c.feed(WM_XBUTTONDOWN, 2 << 16, Some((1, 1)));
        assert_eq!(c.take().pressed_vk, vec![0x01, 0x02, 0x04, 0x05, 0x06]);
        // Held key repeat is bounded.
        for _ in 0..100 {
            c.feed(WM_KEYDOWN, VK_DOWN as usize, None);
        }
        assert_eq!(c.take().keys.len(), MAX_PRESSED);
    }
}
