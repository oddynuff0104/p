//! Win32 window messages → [`KitInput`] ([`Collector`]).

use windows_sys::Win32::Foundation::WPARAM;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::kit::{Key, KitInput};

/// Input between two menu frames: the window procedure feeds messages, the frame takes
/// one [`KitInput`].
#[derive(Default)]
pub struct Collector {
    mouse: [f32; 2],
    down: [bool; 2],
    /// Seit dem letzten Abholen gedrückt: ein Klick innerhalb eines Frames geht nicht verloren.
    pressed: [bool; 2],
    wheel: f32,
    keys: Vec<Key>,
    text: String,
    /// Shift, Strg, Alt gehalten (aus Taste runter/hoch, deterministisch ohne Abfrage).
    modifiers: [bool; 3],
    /// Roh-VK-Codes seit dem letzten Abholen (Tastatur und Maustasten, für Tastenbelegung).
    pressed_vk: Vec<u32>,
}

/// Höchstens so viele Roh-Tasten je Frame (Schutz gegen Tastenwiederholung ohne Abholen).
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
    fn press_vk(&mut self, vk: u32) {
        if self.pressed_vk.len() < MAX_PRESSED && !self.pressed_vk.contains(&vk) {
            self.pressed_vk.push(vk);
        }
    }
    /// Übersetzt eine Fensternachricht (Mausposition schon in Backbuffer-Pixeln).
    pub fn feed(&mut self, msg: u32, wparam: WPARAM, pixel: Option<(u32, u32)>) {
        if let Some((x, y)) = pixel {
            self.mouse = [x as f32, y as f32];
        }
        let button = match msg {
            WM_LBUTTONDOWN | WM_LBUTTONDBLCLK | WM_LBUTTONUP => Some(0),
            WM_RBUTTONDOWN | WM_RBUTTONDBLCLK | WM_RBUTTONUP => Some(1),
            _ => None,
        };
        match msg {
            WM_LBUTTONDOWN | WM_LBUTTONDBLCLK | WM_RBUTTONDOWN | WM_RBUTTONDBLCLK => {
                let b = button.unwrap_or(0);
                self.down[b] = true;
                self.pressed[b] = true;
                self.press_vk(if b == 0 { 0x01 } else { 0x02 });
            }
            WM_LBUTTONUP | WM_RBUTTONUP => self.down[button.unwrap_or(0)] = false,
            WM_MBUTTONDOWN | WM_MBUTTONDBLCLK => self.press_vk(u32::from(VK_MBUTTON)),
            // HIWORD: XBUTTON1 = 1 (Maus 4), XBUTTON2 = 2 (Maus 5).
            WM_XBUTTONDOWN | WM_XBUTTONDBLCLK => match (wparam >> 16) as u16 {
                1 => self.press_vk(u32::from(VK_XBUTTON1)),
                2 => self.press_vk(u32::from(VK_XBUTTON2)),
                _ => {}
            },
            WM_MOUSEWHEEL => self.wheel += (wparam >> 16) as u16 as i16 as f32 / 120.0,
            WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP => {
                let vk = wparam as u16;
                let down = matches!(msg, WM_KEYDOWN | WM_SYSKEYDOWN);
                let modifier = match vk {
                    VK_SHIFT | VK_LSHIFT | VK_RSHIFT => Some(0),
                    VK_CONTROL | VK_LCONTROL | VK_RCONTROL => Some(1),
                    VK_MENU | VK_LMENU | VK_RMENU => Some(2),
                    _ => None,
                };
                if let Some(m) = modifier {
                    self.modifiers[m] = down;
                }
                if !down {
                    return;
                }
                self.press_vk(u32::from(vk));
                let key = match vk {
                    VK_TAB => Some(Key::Tab),
                    VK_BACK => Some(Key::Backspace),
                    VK_RETURN => Some(Key::Enter),
                    VK_SPACE => Some(Key::Space),
                    VK_ESCAPE => Some(Key::Escape),
                    VK_DELETE => Some(Key::Delete),
                    VK_LEFT => Some(Key::Left),
                    VK_RIGHT => Some(Key::Right),
                    VK_UP => Some(Key::Up),
                    VK_DOWN => Some(Key::Down),
                    VK_HOME => Some(Key::Home),
                    VK_END => Some(Key::End),
                    _ => None,
                };
                if let Some(k) = key
                    && self.keys.len() < MAX_PRESSED
                {
                    self.keys.push(k);
                }
            }
            // Ctrl+A arrives as the control character 0x01 after TranslateMessage.
            WM_CHAR if wparam == 0x01 => self.keys.push(Key::SelectAll),
            WM_CHAR => {
                if let Some(c) = char::from_u32(wparam as u32).filter(|c| !c.is_control())
                    && self.text.len() < 64
                {
                    self.text.push(c);
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
