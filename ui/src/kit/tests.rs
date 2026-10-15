//! Headless interaction tests of the kit widgets and the AA primitives (WARP).
use super::*;
use crate::theme::Color;

const SIZE: [u32; 2] = [800, 600];

struct H {
    kit: Kit,
}

fn input(mouse: [f32; 2], down: bool) -> KitInput {
    KitInput {
        mouse,
        down: [down, false],
        dt: 1.0 / 60.0,
        ..Default::default()
    }
}

fn keys(k: &[Key]) -> KitInput {
    KitInput {
        mouse: [-50.0, -50.0],
        keys: k.to_vec(),
        dt: 1.0 / 60.0,
        ..Default::default()
    }
}

fn center(r: Rect) -> [f32; 2] {
    [(r.min[0] + r.max[0]) / 2.0, (r.min[1] + r.max[1]) / 2.0]
}

impl H {
    fn new() -> Self {
        let mut kit = Kit::new(Theme::default()).unwrap();
        kit.debug_capture = true;
        Self { kit }
    }
    fn frame(&mut self, input: KitInput, f: &mut dyn FnMut(&mut Kit)) {
        self.kit.begin(input, SIZE);
        self.kit.push_region(20.0, 420.0, 20.0, 4.0);
        f(&mut self.kit);
        let (list, _) = self.kit.end();
        assert!(
            list.indices
                .iter()
                .all(|&i| (i as usize) < list.vertices.len())
        );
    }
    fn rect(&self, label: &str) -> Rect {
        self.kit
            .debug_rects
            .iter()
            .find(|(l, _)| l == label)
            .map(|(_, r)| *r)
            .unwrap_or_else(|| panic!("no rect for {label}: {:?}", self.kit.debug_rects))
    }
    fn click(&mut self, at: [f32; 2], f: &mut dyn FnMut(&mut Kit)) {
        self.frame(input(at, false), f);
        self.frame(input(at, true), f);
        self.frame(input(at, false), f);
    }
}

#[test]
fn toggle_click_and_keyboard() {
    let mut h = H::new();
    let mut on = false;
    let mut ui = |k: &mut Kit| {
        k.toggle("Enabled", &mut on);
    };
    h.frame(input([0.0; 2], false), &mut ui);
    let r = h.rect("Enabled");
    h.click(center(r), &mut ui);
    assert!(on, "click toggles");
    let mut ui = |k: &mut Kit| {
        k.toggle("Enabled", &mut on);
    };
    // Tab focuses it, Space toggles back.
    h.frame(keys(&[Key::Tab]), &mut ui);
    h.frame(keys(&[Key::Space]), &mut ui);
    assert!(!on, "space toggles the focused switch");
}

#[test]
fn slider_drag_fine_drag_and_keys() {
    let mut h = H::new();
    let mut v = 50.0f32;
    let mut ui = |k: &mut Kit| {
        k.slider_f32("FOV", &mut v, (0.0, 100.0), 0.5, "°");
    };
    h.frame(input([0.0; 2], false), &mut ui);
    let r = h.rect("FOV");
    let y = center(r)[1];
    let x0 = r.min[0] + 7.0;
    let w = r.width() - 14.0;
    // Press at the left end jumps there, drag 100 px moves 100/w of the range.
    h.frame(input([x0, y], false), &mut ui);
    h.frame(input([x0, y], true), &mut ui);
    h.frame(input([x0 + 100.0, y], true), &mut ui);
    h.frame(input([x0 + 100.0, y], false), &mut ui);
    let expect = 100.0 / w * 100.0;
    assert!((v - expect).abs() <= 0.5, "{v} vs {expect}");
    assert_eq!(v % 0.5, 0.0, "snapped to step");
    // Shift: no jump on press, 10x finer.
    let start = v;
    let mut ui = |k: &mut Kit| {
        k.slider_f32("FOV", &mut v, (0.0, 100.0), 0.5, "°");
    };
    let shift = |m: [f32; 2], down: bool| KitInput {
        shift: true,
        ..input(m, down)
    };
    let px = r.max[0] - 40.0;
    h.frame(shift([px, y], false), &mut ui);
    h.frame(shift([px, y], true), &mut ui);
    h.frame(shift([px - 200.0, y], true), &mut ui);
    h.frame(shift([px - 200.0, y], false), &mut ui);
    let fine = start - 200.0 / w * 100.0 * 0.1;
    assert!((v - fine).abs() <= 0.5, "fine drag {v} vs {fine}");
    // Focused after the press: arrows step, Shift+arrow 10 steps, End to max.
    let before = v;
    let mut ui = |k: &mut Kit| {
        k.slider_f32("FOV", &mut v, (0.0, 100.0), 0.5, "°");
    };
    h.frame(keys(&[Key::Right]), &mut ui);
    h.frame(
        KitInput {
            shift: true,
            ..keys(&[Key::Left])
        },
        &mut ui,
    );
    assert!((v - (before + 0.5 - 5.0)).abs() < 1e-3, "{before} -> {v}");
    let mut ui = |k: &mut Kit| {
        k.slider_f32("FOV", &mut v, (0.0, 100.0), 0.5, "°");
    };
    h.frame(keys(&[Key::End]), &mut ui);
    assert_eq!(v, 100.0);
}

#[test]
fn dropdown_search_and_select() {
    let mut h = H::new();
    let mut index = 0usize;
    let opts = gallery::WEAPONS;
    let mut ui = |k: &mut Kit| {
        k.dropdown("Weapon", &mut index, &opts);
    };
    h.frame(input([0.0; 2], false), &mut ui);
    let field = h.rect("Weapon");
    h.click(center(field), &mut ui);
    assert!(h.kit.open != 0, "popup open");
    let typed = KitInput {
        text: "m4a4".into(),
        ..keys(&[])
    };
    h.frame(typed, &mut ui);
    assert!(h.kit.debug_rects.iter().any(|(l, _)| l == "item:M4A4"));
    assert!(
        !h.kit.debug_rects.iter().any(|(l, _)| l == "item:AK-47"),
        "filtered"
    );
    h.frame(keys(&[Key::Enter]), &mut ui);
    assert_eq!(opts[index], "M4A4");
    // Mouse: open, click an item; Esc closes without change.
    let short = ["Head", "Neck", "Chest"];
    let mut bone = 0usize;
    let mut ui = |k: &mut Kit| {
        k.dropdown("Bone", &mut bone, &short);
    };
    h.frame(input([0.0; 2], false), &mut ui);
    let field = h.rect("Bone");
    h.click(center(field), &mut ui);
    let item = h.rect("item:Chest");
    h.click(center(item), &mut ui);
    assert_eq!(h.kit.open, 0, "closed after pick");
    h.click(center(field), &mut ui);
    assert!(h.kit.open != 0);
    h.frame(keys(&[Key::Escape]), &mut ui);
    assert_eq!(h.kit.open, 0, "Esc closes");
    assert_eq!(bone, 2);
}

#[test]
fn color_picker_hex_input() {
    let mut h = H::new();
    let mut c = Color::rgb(255, 0, 0);
    let presets = gallery::presets();
    let mut ui = |k: &mut Kit| {
        k.color_edit("Color", &mut c, &presets);
    };
    h.frame(input([0.0; 2], false), &mut ui);
    let sw = h.rect("Color");
    h.click(center(sw), &mut ui);
    let hex = h.rect("hex");
    h.click(center(hex), &mut ui);
    h.frame(keys(&[Key::SelectAll]), &mut ui);
    let typed = KitInput {
        text: "#00FF80".into(),
        ..keys(&[])
    };
    h.frame(typed, &mut ui);
    assert!(h.kit.open != 0, "picker stays open while typing");
    assert_eq!(c, Color::rgb(0, 255, 128));
}

#[test]
fn keybind_capture() {
    let mut h = H::new();
    let mut vk = 0u32;
    let mut ui = |k: &mut Kit| {
        k.keybind("Aim key", &mut vk);
    };
    h.frame(input([0.0; 2], false), &mut ui);
    let r = h.rect("Aim key");
    h.click(center(r), &mut ui);
    assert!(h.kit.capturing != 0);
    let press = KitInput {
        pressed_vk: vec![0x46],
        ..keys(&[])
    };
    h.frame(press, &mut ui);
    assert_eq!(h.kit.capturing, 0);
    // Esc cancels, Backspace clears.
    h.click(center(r), &mut ui);
    h.frame(
        KitInput {
            pressed_vk: vec![0x1B],
            ..keys(&[Key::Escape])
        },
        &mut ui,
    );
    assert_eq!(vk, 0x46, "Esc keeps the key");
    assert_eq!(widgets::vk_name(vk), "F");
    let mut ui = |k: &mut Kit| {
        k.keybind("Aim key", &mut vk);
    };
    h.click(center(r), &mut ui);
    h.frame(keys(&[Key::Backspace]), &mut ui);
    assert_eq!(vk, 0);
}

#[test]
fn tab_focus_order_and_activation() {
    let mut h = H::new();
    let (mut a, mut b, mut pressed) = (false, false, 0);
    let mut ui = |k: &mut Kit| {
        k.toggle("A", &mut a);
        k.checkbox("B", &mut b);
        if k.button("C", widgets::ButtonKind::Primary) {
            pressed += 1;
        }
    };
    h.frame(input([0.0; 2], false), &mut ui);
    let ids: Vec<Id> = h.kit.focus_order.clone();
    assert_eq!(ids.len(), 3);
    h.frame(keys(&[Key::Tab]), &mut ui);
    assert_eq!(h.kit.focus, ids[0]);
    h.frame(keys(&[Key::Tab]), &mut ui);
    assert_eq!(h.kit.focus, ids[1]);
    h.frame(keys(&[Key::Tab]), &mut ui);
    assert_eq!(h.kit.focus, ids[2]);
    h.frame(keys(&[Key::Tab]), &mut ui);
    assert_eq!(h.kit.focus, ids[0], "wraps");
    h.frame(
        KitInput {
            shift: true,
            ..keys(&[Key::Tab])
        },
        &mut ui,
    );
    assert_eq!(h.kit.focus, ids[2], "shift+tab goes back");
    h.frame(keys(&[Key::Enter]), &mut ui);
    h.frame(
        KitInput {
            shift: true,
            ..keys(&[Key::Tab])
        },
        &mut ui,
    );
    h.frame(keys(&[Key::Space]), &mut ui);
    assert_eq!(pressed, 1, "Enter on the button");
    assert!(b && !a, "Space on the checkbox only");
}

#[test]
fn segmented_and_tabs_arrows() {
    let mut h = H::new();
    let (mut seg, mut tab) = (0usize, 0usize);
    let mut ui = |k: &mut Kit| {
        k.segmented("Mode", &mut seg, &["Off", "Low", "High"]);
        k.tabs("tabs", &mut tab, &["One", "Two", "Three"]);
    };
    h.frame(input([0.0; 2], false), &mut ui);
    let r = h.rect("Mode");
    h.click([r.max[0] - 5.0, center(r)[1]], &mut ui);
    h.frame(keys(&[Key::Left]), &mut ui);
    let t = h.rect("tabs");
    h.click([t.min[0] + 2.0, center(t)[1]], &mut ui);
    for _ in 0..3 {
        h.frame(keys(&[Key::Right]), &mut ui);
    }
    assert_eq!(seg, 1, "clicked High, Left to Low");
    assert_eq!(tab, 2, "clamped at the last tab");
}

#[test]
fn toast_and_confirm_dialog() {
    let mut h = H::new();
    let mut result = None;
    let mut ui = |k: &mut Kit| {
        if k.button("Reset", widgets::ButtonKind::Danger) {
            k.open_confirm("reset");
        }
        if let Some(r) = k.confirm("reset", "Reset?", "Sure?", "Reset", true) {
            result = Some(r);
        }
    };
    h.frame(input([0.0; 2], false), &mut ui);
    let r = h.rect("Reset");
    h.click(center(r), &mut ui);
    assert!(h.kit.open != 0, "dialog open");
    // Modal: the button underneath does not react.
    h.click(center(r), &mut ui);
    h.frame(keys(&[Key::Escape]), &mut ui);
    assert_eq!(result, Some(false));
    h.kit.toast("done", ToastKind::Success);
    let mut ui = |_: &mut Kit| {};
    h.frame(input([0.0; 2], false), &mut ui);
    assert_eq!(h.kit.toast_count(), 1);
    for _ in 0..30 {
        let mut i = input([0.0; 2], false);
        i.dt = 0.2;
        h.frame(i, &mut ui);
    }
    assert_eq!(h.kit.toast_count(), 0, "expires");
}

/// Rounded rect on WARP: interior opaque, a half-covered edge pixel about half, the
/// corner curve has intermediate coverage, outside untouched.
#[test]
fn rounded_rect_edge_coverage_on_warp() {
    use crate::draw::{DrawList, FONT_TEXTURE, TextureDelta};
    use crate::primitives::Shapes;
    let mut target = crate::d3d11::Offscreen::new(64, 64).unwrap();
    target.clear([0.0, 0.0, 0.0, 1.0]);
    let mut list = DrawList::default();
    list.fill_rounded(Rect::new(10.5, 10.0, 40.0, 40.0), 12.0, Color::WHITE.pm());
    let white = TextureDelta::Create {
        id: FONT_TEXTURE,
        size: [1, 1],
        rgba: vec![255; 4],
        nearest: true,
    };
    target.paint(&[white], &list).unwrap();
    let px = target.pixels().unwrap();
    let at = |x: usize, y: usize| px[(y * 64 + x) * 4];
    assert_eq!(at(30, 30), 255, "interior");
    let edge = at(10, 30);
    assert!(
        (100..=156).contains(&edge),
        "half-covered edge pixel: {edge}"
    );
    assert_eq!(at(9, 30), 0, "outside");
    assert_eq!(at(11, 11), 0, "cut corner");
    let partial = (8..24)
        .flat_map(|y| (8..24).map(move |x| (x, y)))
        .filter(|&(x, y)| (20..=235).contains(&at(x, y)))
        .count();
    assert!(
        partial >= 8,
        "anti-aliased corner: {partial} partial pixels"
    );
}
