//! Widget gallery: every kit component on one page (preview images, manual checks).

use super::Kit;
use super::ToastKind;
use super::widgets::{ButtonKind, CardHead};
use crate::draw::Rect;
use crate::primitives::Icon;
use crate::theme::{ACCENTS, Color, TextStyle};

pub const WEAPONS: [&str; 34] = [
    "AK-47",
    "AUG",
    "AWP",
    "CZ75-Auto",
    "Desert Eagle",
    "Dual Berettas",
    "FAMAS",
    "Five-SeveN",
    "G3SG1",
    "Galil AR",
    "Glock-18",
    "M249",
    "M4A1-S",
    "M4A4",
    "MAC-10",
    "MAG-7",
    "MP5-SD",
    "MP7",
    "MP9",
    "Negev",
    "Nova",
    "P2000",
    "P250",
    "P90",
    "PP-Bizon",
    "R8 Revolver",
    "Sawed-Off",
    "SCAR-20",
    "SG 553",
    "SSG 08",
    "Tec-9",
    "UMP-45",
    "USP-S",
    "XM1014",
];

pub fn presets() -> Vec<Color> {
    let mut v: Vec<Color> = ACCENTS.iter().map(|a| a.1).collect();
    v.extend([
        Color::rgb(255, 80, 80),
        Color::rgb(255, 220, 60),
        Color::rgb(120, 220, 90),
        Color::rgb(255, 255, 255),
        Color::rgb(150, 150, 150),
        Color::rgb(20, 20, 20),
        Color::rgb(80, 200, 255),
        Color::rgb(230, 100, 230),
        Color::rgb(255, 150, 60),
        Color::rgb(60, 200, 160),
    ]);
    v
}

pub struct GalleryState {
    pub enabled: bool,
    pub check: bool,
    pub check2: bool,
    pub fov: f32,
    pub smooth: i32,
    pub hitchance: f32,
    pub mode: usize,
    pub bone: usize,
    pub weapon: usize,
    pub color: Color,
    pub color2: Color,
    pub key: u32,
    pub key2: u32,
    pub tab: usize,
    pub name: String,
}

impl Default for GalleryState {
    fn default() -> Self {
        Self {
            enabled: true,
            check: true,
            check2: false,
            fov: 12.5,
            smooth: 4,
            hitchance: 65.0,
            mode: 1,
            bone: 0,
            weapon: 2,
            color: Color::rgb(255, 92, 132),
            color2: Color::rgb(56, 170, 255),
            key: 0x05,
            key2: 0,
            tab: 0,
            name: "poly".into(),
        }
    }
}

/// Draws the gallery cards into the current region (inside the shell content).
pub fn draw(kit: &mut Kit, s: &mut GalleryState) {
    let presets = presets();
    kit.begin_grid(kit.theme.px(300.0), 3);

    let mut on = s.enabled;
    kit.card(
        CardHead {
            title: "Toggles",
            icon: Some(Icon::Shield),
            badge: Some("beta"),
            hint: "Card header with icon, badge and enable switch.",
        },
        Some(&mut on),
    );
    s.enabled = on;
    kit.toggle("Silent aim", &mut s.check);
    kit.hint("Animated switch: the knob slides, the track fades to the accent.");
    kit.checkbox("Auto fire", &mut s.check2);
    kit.checkbox("Auto scope", &mut s.check);
    kit.end_card();

    kit.card(
        CardHead {
            title: "Sliders",
            icon: Some(Icon::Crosshair),
            ..Default::default()
        },
        None,
    );
    kit.slider_f32("Field of view", &mut s.fov, (0.0, 180.0), 0.5, "°");
    kit.hint("Drag, Shift for fine control, arrows step.");
    kit.slider_i32("Smoothing", &mut s.smooth, (1, 20), 1, "");
    kit.slider_f32("Hit chance", &mut s.hitchance, (0.0, 100.0), 1.0, "%");
    kit.end_card();

    kit.card(
        CardHead {
            title: "Choices",
            icon: Some(Icon::Grid),
            ..Default::default()
        },
        None,
    );
    kit.segmented("Mode", &mut s.mode, &["Off", "Legit", "Rage"]);
    kit.dropdown(
        "Target bone",
        &mut s.bone,
        &["Head", "Neck", "Chest", "Stomach"],
    );
    kit.dropdown("Weapon", &mut s.weapon, &WEAPONS);
    kit.hint("Long lists get a search field.");
    kit.end_card();

    kit.card(
        CardHead {
            title: "Colours & keys",
            icon: Some(Icon::Palette),
            ..Default::default()
        },
        None,
    );
    kit.color_edit("Enemy colour", &mut s.color, &presets);
    kit.color_edit("Team colour", &mut s.color2, &presets);
    kit.keybind("Aim key", &mut s.key);
    kit.keybind("Panic key", &mut s.key2);
    kit.text_field("Config name", &mut s.name, 24);
    kit.end_card();

    kit.card(
        CardHead {
            title: "Actions",
            icon: Some(Icon::Run),
            ..Default::default()
        },
        None,
    );
    let row = kit.alloc(kit.theme.px(kit.theme.control));
    let w = (row.width() - kit.theme.sp(2)) / 2.0;
    let a = Rect::new(row.min[0], row.min[1], w, row.height());
    let b = Rect::new(
        row.min[0] + w + kit.theme.sp(2),
        row.min[1],
        w,
        row.height(),
    );
    if kit.button_at("Show success", a, ButtonKind::Primary) {
        kit.toast("Preset applied", ToastKind::Success);
    }
    if kit.button_at("Show error", b, ButtonKind::Secondary) {
        kit.toast("Gate refused: not -insecure", ToastKind::Error);
    }
    if kit.button("Reset everything…", ButtonKind::Danger) {
        kit.open_confirm("reset");
    }
    if let Some(yes) = kit.confirm(
        "reset",
        "Reset all settings?",
        "Every feature returns to its default value. This cannot be undone.",
        "Reset",
        true,
    ) && yes
    {
        kit.toast("All settings reset", ToastKind::Info);
    }
    kit.tabs("tabs", &mut s.tab, &["Players", "Local", "Feedback"]);
    if kit.collapsible("Advanced", true) {
        kit.paragraph("Collapsible sections animate their height and remember their state.");
        kit.end_collapsible();
    }
    kit.end_card();

    kit.card(
        CardHead {
            title: "Icons & type",
            icon: Some(Icon::Eye),
            ..Default::default()
        },
        None,
    );
    let t = kit.theme.clone();
    let per = 10usize;
    let cell = t.px(28.0);
    let rows = Icon::ALL.len().div_ceil(per);
    let r = kit.alloc(cell * rows as f32);
    for (i, icon) in Icon::ALL.iter().enumerate() {
        let c = [
            r.min[0] + cell * ((i % per) as f32 + 0.5),
            r.min[1] + cell * ((i / per) as f32 + 0.5),
        ];
        kit.icon(*icon, c, 18.0, t.text_muted);
    }
    for (style, name) in [
        (TextStyle::Title, "Title 18"),
        (TextStyle::Section, "Section 15"),
        (TextStyle::Body, "Body 13.5 – the quick brown fox"),
        (TextStyle::Caption, "Caption 11.5 – hints, values, status"),
    ] {
        let h = kit.font(style).line_height + t.px(4.0);
        let rr = kit.alloc(h);
        kit.text_at(style, rr.min, name, t.text);
    }
    let br = kit.alloc(t.px(20.0));
    let cy = br.min[1] + br.height() / 2.0;
    let mut x = br.min[0];
    x += kit.badge_at(x, cy, "beta", t.warning) + t.sp(2);
    x += kit.badge_at(x, cy, "new", t.accent) + t.sp(2);
    x += kit.badge_at(x, cy, "server", t.success) + t.sp(2);
    kit.badge_at(x, cy, "risky", t.danger);
    kit.end_card();

    kit.end_grid();
}
