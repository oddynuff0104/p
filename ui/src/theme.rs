//! Design system of the menu kit (`menu::kit`): every colour, size, radius, font size,
//! shadow and animation speed in one place.
//!
//! Metrics are in points; [`Theme::px`] multiplies by the DPI `scale` and rounds to whole
//! physical pixels so edges stay crisp. Colours are straight (non-premultiplied) sRGB
//! [`Color`]s; [`Color::pm`] converts to the premultiplied `u32` of [`super::draw`].

use super::draw::rgba;

/// Straight sRGB colour with alpha.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Color(pub [u8; 4]);

impl Color {
    pub const TRANSPARENT: Color = Color([0, 0, 0, 0]);
    pub const WHITE: Color = Color([255, 255, 255, 255]);
    pub const BLACK: Color = Color([0, 0, 0, 255]);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self([r, g, b, 255])
    }
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self([r, g, b, a])
    }
    /// `0xRRGGBB`.
    pub const fn hex(v: u32) -> Self {
        Self([(v >> 16) as u8, (v >> 8) as u8, v as u8, 255])
    }
    /// Premultiplied RGBA8 for the draw list.
    pub const fn pm(self) -> u32 {
        rgba(self.0[0], self.0[1], self.0[2], self.0[3])
    }
    /// Same colour with alpha multiplied by `f` (0..1).
    pub fn alpha(self, f: f32) -> Self {
        let a = (self.0[3] as f32 * f.clamp(0.0, 1.0)).round() as u8;
        Self([self.0[0], self.0[1], self.0[2], a])
    }
    /// Linear mix in sRGB space, `t = 0` → `self`, `t = 1` → `other`.
    pub fn mix(self, other: Color, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let c = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
        Self([
            c(self.0[0], other.0[0]),
            c(self.0[1], other.0[1]),
            c(self.0[2], other.0[2]),
            c(self.0[3], other.0[3]),
        ])
    }
    /// `#RRGGBB`.
    pub fn to_hex(self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.0[0], self.0[1], self.0[2])
    }
    /// Parses `#RRGGBB`, `RRGGBB` or `#RGB`.
    pub fn parse_hex(s: &str) -> Option<Self> {
        let s = s.trim().trim_start_matches('#');
        if !s.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        match s.len() {
            6 => u32::from_str_radix(s, 16).ok().map(Self::hex),
            3 => {
                let v = u32::from_str_radix(s, 16).ok()?;
                let d = |n: u32| ((n & 0xF) * 17) as u8;
                Some(Self::rgb(d(v >> 8), d(v >> 4), d(v)))
            }
            _ => None,
        }
    }
    /// Hue (0..1), saturation, value.
    pub fn to_hsv(self) -> [f32; 3] {
        let [r, g, b] = [self.0[0], self.0[1], self.0[2]].map(|c| c as f32 / 255.0);
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let d = max - min;
        let h = if d <= f32::EPSILON {
            0.0
        } else if max == r {
            ((g - b) / d).rem_euclid(6.0) / 6.0
        } else if max == g {
            ((b - r) / d + 2.0) / 6.0
        } else {
            ((r - g) / d + 4.0) / 6.0
        };
        let s = if max <= f32::EPSILON { 0.0 } else { d / max };
        [h, s, max]
    }
    pub fn from_hsv(h: f32, s: f32, v: f32) -> Self {
        let h = h.rem_euclid(1.0) * 6.0;
        let (s, v) = (s.clamp(0.0, 1.0), v.clamp(0.0, 1.0));
        let c = v * s;
        let x = c * (1.0 - (h % 2.0 - 1.0).abs());
        let (r, g, b) = match h as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let m = v - c;
        let q = |f: f32| ((f + m) * 255.0).round().clamp(0.0, 255.0) as u8;
        Self::rgb(q(r), q(g), q(b))
    }
}

/// Selectable accent colours (Settings › Appearance); index 0 is the default.
pub const ACCENTS: [(&str, Color); 6] = [
    ("Indigo", Color::rgb(108, 124, 255)),
    ("Sky", Color::rgb(56, 170, 255)),
    ("Mint", Color::rgb(44, 200, 150)),
    ("Violet", Color::rgb(164, 108, 255)),
    ("Rose", Color::rgb(255, 92, 132)),
    ("Amber", Color::rgb(255, 170, 50)),
];

/// Corner radii in points.
#[derive(Clone, Copy, Debug)]
pub struct Radii {
    pub sm: f32,
    pub md: f32,
    pub lg: f32,
    pub xl: f32,
}

/// Font sizes in points; each gets its own natively rasterised atlas size.
#[derive(Clone, Copy, Debug)]
pub struct TypeScale {
    pub caption: f32,
    pub body: f32,
    pub section: f32,
    pub title: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextStyle {
    Caption,
    Body,
    Section,
    Title,
}

/// Soft drop shadow: offset and blur (feather width) in points.
#[derive(Clone, Copy, Debug)]
pub struct Shadow {
    pub offset: [f32; 2],
    pub blur: f32,
    pub color: Color,
}

#[derive(Clone, Debug)]
pub struct Theme {
    /// DPI factor (physical pixels per point).
    pub scale: f32,
    // Semantic colour tokens.
    pub bg: Color,
    pub surface: Color,
    pub surface_raised: Color,
    pub surface_hover: Color,
    pub border: Color,
    pub border_strong: Color,
    pub text: Color,
    pub text_muted: Color,
    pub text_faint: Color,
    pub accent: Color,
    /// Text and icons on an accent fill.
    pub on_accent: Color,
    pub danger: Color,
    pub success: Color,
    pub warning: Color,
    /// Modal backdrop.
    pub scrim: Color,
    /// Spacing scale in points: `space[i]` for i = 0..7 → 2, 4, 8, 12, 16, 24, 32.
    pub space: [f32; 7],
    pub radius: Radii,
    pub type_scale: TypeScale,
    pub shadow_card: Shadow,
    pub shadow_popup: Shadow,
    pub shadow_window: Shadow,
    /// Row height of a setting (label + control) in points.
    pub row: f32,
    /// Height of buttons, fields, dropdown headers in points.
    pub control: f32,
    /// Exponential approach rate of animations per second (higher = snappier).
    pub anim_rate: f32,
    /// Hover time before a tooltip shows, seconds.
    pub tooltip_delay: f32,
}

impl Theme {
    /// The dark default theme with `accent`.
    pub fn dark(accent: Color, scale: f32) -> Self {
        Self {
            scale: scale.max(0.5),
            bg: Color::rgba(13, 14, 19, 246),
            surface: Color::rgb(21, 23, 30),
            surface_raised: Color::rgb(29, 32, 41),
            surface_hover: Color::rgb(37, 41, 52),
            border: Color::rgb(39, 43, 54),
            border_strong: Color::rgb(56, 61, 76),
            text: Color::rgb(232, 234, 240),
            text_muted: Color::rgb(148, 154, 170),
            text_faint: Color::rgb(98, 104, 120),
            accent,
            on_accent: Color::WHITE,
            danger: Color::rgb(240, 86, 96),
            success: Color::rgb(52, 196, 120),
            warning: Color::rgb(240, 176, 60),
            scrim: Color::rgba(5, 6, 9, 150),
            space: [2.0, 4.0, 8.0, 12.0, 16.0, 24.0, 32.0],
            radius: Radii {
                sm: 4.0,
                md: 6.0,
                lg: 10.0,
                xl: 14.0,
            },
            type_scale: TypeScale {
                caption: 11.5,
                body: 13.5,
                section: 15.0,
                title: 18.0,
            },
            shadow_card: Shadow {
                offset: [0.0, 2.0],
                blur: 10.0,
                color: Color::rgba(0, 0, 0, 70),
            },
            shadow_popup: Shadow {
                offset: [0.0, 6.0],
                blur: 18.0,
                color: Color::rgba(0, 0, 0, 120),
            },
            shadow_window: Shadow {
                offset: [0.0, 10.0],
                blur: 36.0,
                color: Color::rgba(0, 0, 0, 150),
            },
            row: 32.0,
            control: 28.0,
            anim_rate: 18.0,
            tooltip_delay: 0.45,
        }
    }

    /// Points → whole physical pixels.
    pub fn px(&self, points: f32) -> f32 {
        (points * self.scale).round()
    }
    /// Points → physical pixels without rounding (strokes, radii).
    pub fn pxf(&self, points: f32) -> f32 {
        points * self.scale
    }
    /// Spacing step `i` (0..7) in pixels.
    pub fn sp(&self, i: usize) -> f32 {
        self.px(self.space[i.min(self.space.len() - 1)])
    }
    pub fn font_points(&self, style: TextStyle) -> f32 {
        let t = &self.type_scale;
        match style {
            TextStyle::Caption => t.caption,
            TextStyle::Body => t.body,
            TextStyle::Section => t.section,
            TextStyle::Title => t.title,
        }
    }
    /// Accent mixed toward the surface (tinted backgrounds of active items).
    pub fn accent_soft(&self) -> Color {
        self.surface.mix(self.accent, 0.22)
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::dark(ACCENTS[0].1, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_and_hsv_round_trip() {
        let c = Color::parse_hex("#3A7BFF").unwrap();
        assert_eq!(c, Color::rgb(0x3A, 0x7B, 0xFF));
        assert_eq!(c.to_hex(), "#3A7BFF");
        assert_eq!(Color::parse_hex("fff"), Some(Color::WHITE));
        assert_eq!(Color::parse_hex("#12345"), None);
        assert_eq!(Color::parse_hex("#GG0000"), None);
        for c in [
            c,
            Color::rgb(255, 0, 0),
            Color::rgb(10, 200, 90),
            Color::WHITE,
        ] {
            let [h, s, v] = c.to_hsv();
            let back = Color::from_hsv(h, s, v);
            for i in 0..3 {
                assert!(back.0[i].abs_diff(c.0[i]) <= 1, "{c:?} -> {back:?}");
            }
        }
    }

    #[test]
    fn px_scales_and_rounds() {
        let t = Theme::dark(ACCENTS[0].1, 1.5);
        assert_eq!(t.px(10.0), 15.0);
        assert_eq!(t.sp(2), 12.0);
        assert_eq!(Color::rgb(0, 0, 0).mix(Color::WHITE, 0.5).0[0], 128);
    }
}
