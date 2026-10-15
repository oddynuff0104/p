//! Design system of the menu kit (`menu::kit`): every colour, size, radius, font size,
//! shadow and animation speed in one place. The look is dark "liquid glass" with an
//! accent [`Gradient`] (docs/UI.md, "Aussehen: Liquid Glass").
//!
//! Metrics are in points; [`Theme::px`] multiplies by the DPI `scale` and rounds to whole
//! physical pixels so edges stay crisp. Colours are straight (non-premultiplied) sRGB
//! [`Color`]s; [`Color::pm`] converts to the premultiplied `u32` of [`super::draw`].

use super::draw::rgba;

/// Straight sRGB colour with alpha.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Color(pub [u8; 4]);

impl Color {
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

/// Single accent colours: colour-picker presets and the migration of old window files
/// (`accent=<index>` before gradients); index 0 was the default.
pub const ACCENTS: [(&str, Color); 6] = [
    ("Indigo", Color::rgb(108, 124, 255)),
    ("Sky", Color::rgb(56, 170, 255)),
    ("Mint", Color::rgb(44, 200, 150)),
    ("Violet", Color::rgb(164, 108, 255)),
    ("Rose", Color::rgb(255, 92, 132)),
    ("Amber", Color::rgb(255, 170, 50)),
];

/// Gradient presets of Settings › Appearance (index 0 is the default). "Single colour"
/// is not listed here: it is a gradient with one stop.
pub const GRADIENTS: [(&str, &[Color]); 5] = [
    (
        "Rainbow",
        &[
            Color::rgb(255, 77, 109),
            Color::rgb(255, 159, 67),
            Color::rgb(255, 210, 63),
            Color::rgb(61, 220, 151),
            Color::rgb(58, 176, 255),
            Color::rgb(142, 107, 255),
        ],
    ),
    (
        "Sunset",
        &[
            Color::rgb(255, 79, 129),
            Color::rgb(255, 128, 82),
            Color::rgb(255, 196, 102),
        ],
    ),
    (
        "Ocean",
        &[
            Color::rgb(0, 210, 255),
            Color::rgb(0, 122, 255),
            Color::rgb(108, 92, 255),
        ],
    ),
    (
        "Aurora",
        &[
            Color::rgb(61, 220, 151),
            Color::rgb(34, 211, 238),
            Color::rgb(167, 139, 250),
            Color::rgb(244, 114, 182),
        ],
    ),
    (
        "Neon",
        &[
            Color::rgb(247, 37, 133),
            Color::rgb(144, 60, 230),
            Color::rgb(67, 97, 238),
            Color::rgb(76, 201, 240),
        ],
    ),
];

/// The accent: 1–6 colour stops (1 = a plain single colour), an angle and an optional
/// slow hue rotation. Drawn through a 256-texel lookup row ([`Gradient::lut`]), so any
/// shape can be filled with it without extra draw calls.
#[derive(Clone, Copy, Debug)]
pub struct Gradient {
    pub stops: [Color; Gradient::MAX_STOPS],
    /// Used stops, 1..=MAX_STOPS.
    pub count: usize,
    /// CSS-style degrees: 0 = left → right, 90 = top → bottom.
    pub angle: f32,
    /// Hue rotation in degrees per second; 0 = static.
    pub hue_speed: f32,
}

impl Default for Gradient {
    fn default() -> Self {
        Self::preset(0)
    }
}

/// Equal when the used stops, angle and speed are (unused stop slots do not count).
impl PartialEq for Gradient {
    fn eq(&self, o: &Self) -> bool {
        self.stops() == o.stops() && self.angle == o.angle && self.hue_speed == o.hue_speed
    }
}

impl Gradient {
    pub const MAX_STOPS: usize = 6;
    pub const DEFAULT_ANGLE: f32 = 20.0;
    pub const MAX_HUE_SPEED: f32 = 120.0;

    /// Preset `i` of [`GRADIENTS`] (clamped).
    pub fn preset(i: usize) -> Self {
        let (_, stops) = GRADIENTS[i.min(GRADIENTS.len() - 1)];
        let mut g = Self::single(stops[0]);
        g.set_stops(stops);
        g
    }

    pub fn single(color: Color) -> Self {
        Self {
            stops: [color; Self::MAX_STOPS],
            count: 1,
            angle: Self::DEFAULT_ANGLE,
            hue_speed: 0.0,
        }
    }

    pub fn stops(&self) -> &[Color] {
        &self.stops[..self.count.clamp(1, Self::MAX_STOPS)]
    }

    /// Uses `stops` (at most [`Self::MAX_STOPS`], at least one).
    pub fn set_stops(&mut self, stops: &[Color]) {
        if stops.is_empty() {
            return;
        }
        let n = stops.len().min(Self::MAX_STOPS);
        self.stops[..n].copy_from_slice(&stops[..n]);
        self.count = n;
    }

    /// Changes the number of stops; new stops continue the hue walk of the last ones.
    pub fn set_count(&mut self, count: usize) {
        let count = count.clamp(1, Self::MAX_STOPS);
        for i in self.count..count {
            let last = self.stops[i.saturating_sub(1)];
            let [h, s, v] = last.to_hsv();
            self.stops[i] = Color::from_hsv(h + 0.13, s.max(0.55), v.max(0.85));
        }
        self.count = count;
    }

    /// Index into [`GRADIENTS`] whose stops equal ours, if any.
    pub fn preset_index(&self) -> Option<usize> {
        GRADIENTS.iter().position(|(_, s)| *s == self.stops())
    }

    /// Hue rotation (0..1) at kit time `time` seconds.
    pub fn phase(&self, time: f32) -> f32 {
        (time * self.hue_speed / 360.0).rem_euclid(1.0)
    }

    /// Colour at `t` (0..1) with the hue rotated by `phase` turns; stops are blended in
    /// linear light so mid tones stay bright.
    pub fn sample(&self, t: f32, phase: f32) -> Color {
        let stops = self.stops();
        let c = if stops.len() == 1 {
            stops[0]
        } else {
            let x = t.clamp(0.0, 1.0) * (stops.len() - 1) as f32;
            let i = (x.floor() as usize).min(stops.len() - 2);
            mix_linear(stops[i], stops[i + 1], x - i as f32)
        };
        if phase == 0.0 {
            return c;
        }
        let [h, s, v] = c.to_hsv();
        Color::from_hsv(h + phase, s, v)
    }

    /// Mean colour (icons, text and other places that cannot show a gradient).
    pub fn average(&self, phase: f32) -> Color {
        let n = 8;
        let sum = (0..n).fold([0.0f32; 3], |acc, i| {
            let c = self.sample(i as f32 / (n - 1) as f32, phase);
            [
                acc[0] + to_linear(c.0[0]),
                acc[1] + to_linear(c.0[1]),
                acc[2] + to_linear(c.0[2]),
            ]
        });
        let q = |v: f32| to_srgb(v / n as f32);
        Color::rgb(q(sum[0]), q(sum[1]), q(sum[2]))
    }

    /// The 256×1 premultiplied lookup row (`draw::GRADIENT_TEXTURE`).
    pub fn lut(&self, phase: f32) -> Vec<u8> {
        let w = super::draw::GRADIENT_WIDTH as usize;
        let mut out = Vec::with_capacity(w * 4);
        for i in 0..w {
            let c = self.sample(i as f32 / (w - 1) as f32, phase);
            out.extend_from_slice(&[c.0[0], c.0[1], c.0[2], 255]);
        }
        out
    }

    /// `#RRGGBB,#RRGGBB,…` (window file).
    pub fn stops_text(&self) -> String {
        self.stops()
            .iter()
            .map(|c| c.to_hex())
            .collect::<Vec<_>>()
            .join(",")
    }

    /// Parses [`Self::stops_text`]; `None` when no stop parses.
    pub fn parse_stops(text: &str) -> Option<Vec<Color>> {
        let stops: Vec<Color> = text
            .split(',')
            .filter_map(Color::parse_hex)
            .take(Self::MAX_STOPS)
            .collect();
        (!stops.is_empty()).then_some(stops)
    }
}

fn to_linear(c: u8) -> f32 {
    let c = c as f32 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn to_srgb(v: f32) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let s = if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    };
    (s * 255.0).round() as u8
}

fn mix_linear(a: Color, b: Color, t: f32) -> Color {
    let m = |x: u8, y: u8| to_srgb(to_linear(x) + (to_linear(y) - to_linear(x)) * t);
    Color::rgb(m(a.0[0], b.0[0]), m(a.0[1], b.0[1]), m(a.0[2], b.0[2]))
}

/// How a glass surface is drawn (`Kit::glass`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlassKind {
    /// The window: blurred backdrop, dark tint, gradient rim and glow.
    Window,
    /// Popups, dropdowns, dialogs, tooltips, toasts: denser tint, deeper shadow.
    Popup,
    /// Cards and the sidebar inside the window: a light platter with a lit rim.
    Platter,
}

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
    // Semantic colour tokens (surfaces are glass: see `glass_*`, `platter`).
    pub border: Color,
    pub border_strong: Color,
    pub text: Color,
    pub text_muted: Color,
    pub text_faint: Color,
    /// Mean colour of the accent gradient (text, icons; kept current by the kit).
    pub accent: Color,
    /// The accent gradient (fills, rims, glow).
    pub gradient: Gradient,
    /// Hue rotation of the gradient this frame (0..1, from `gradient.hue_speed`).
    pub phase: f32,
    /// Real backdrop blur behind glass (else layered translucency only).
    pub blur: bool,
    /// Glass tint over the blurred backdrop (window / popups) with blur on and off.
    pub glass_tint: Color,
    pub glass_alpha: [f32; 2],
    pub popup_alpha: [f32; 2],
    /// Light platter of cards and the sidebar inside the window.
    pub platter: Color,
    /// Specular rim: the lit (top/left) and the shaded (bottom/right) edge.
    pub rim_light: Color,
    pub rim_shade: Color,
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
    /// Spring of hover/press/toggle motion: stiffness (1/s²) and damping ratio (< 1
    /// overshoots slightly; settles in about a quarter second).
    pub spring: [f32; 2],
    /// Hover time before a tooltip shows, seconds.
    pub tooltip_delay: f32,
}

impl Theme {
    /// The dark glass theme with a single-colour `accent`.
    #[cfg(test)]
    pub fn dark(accent: Color, scale: f32) -> Self {
        Self::glass(Gradient::single(accent), scale)
    }

    /// The dark glass theme with the accent `gradient`.
    pub fn glass(gradient: Gradient, scale: f32) -> Self {
        Self {
            scale: scale.max(0.5),
            border: Color::rgba(255, 255, 255, 22),
            border_strong: Color::rgba(255, 255, 255, 40),
            text: Color::rgb(242, 244, 250),
            text_muted: Color::rgb(176, 182, 198),
            text_faint: Color::rgb(128, 134, 152),
            accent: gradient.average(0.0),
            gradient,
            phase: 0.0,
            blur: true,
            glass_tint: Color::rgb(11, 13, 22),
            glass_alpha: [0.56, 0.90],
            popup_alpha: [0.70, 0.94],
            platter: Color::rgba(255, 255, 255, 12),
            rim_light: Color::rgba(255, 255, 255, 92),
            rim_shade: Color::rgba(255, 255, 255, 16),
            on_accent: Color::WHITE,
            danger: Color::rgb(255, 92, 102),
            success: Color::rgb(60, 214, 132),
            warning: Color::rgb(255, 186, 70),
            scrim: Color::rgba(5, 6, 9, 140),
            space: [2.0, 4.0, 8.0, 12.0, 16.0, 24.0, 32.0],
            radius: Radii {
                sm: 6.0,
                md: 10.0,
                lg: 16.0,
                xl: 24.0,
            },
            type_scale: TypeScale {
                caption: 11.5,
                body: 13.5,
                section: 15.0,
                title: 18.0,
            },
            shadow_card: Shadow {
                offset: [0.0, 3.0],
                blur: 14.0,
                color: Color::rgba(0, 0, 0, 46),
            },
            shadow_popup: Shadow {
                offset: [0.0, 10.0],
                blur: 30.0,
                color: Color::rgba(0, 0, 0, 130),
            },
            shadow_window: Shadow {
                offset: [0.0, 18.0],
                blur: 56.0,
                color: Color::rgba(0, 0, 0, 140),
            },
            row: 32.0,
            control: 28.0,
            anim_rate: 18.0,
            spring: [420.0, 0.72],
            tooltip_delay: 0.45,
        }
    }

    /// Glass tint alpha of `kind` for the current blur setting.
    pub fn tint_alpha(&self, kind: GlassKind) -> f32 {
        let i = usize::from(!self.blur);
        match kind {
            GlassKind::Window => self.glass_alpha[i],
            GlassKind::Popup => self.popup_alpha[i],
            GlassKind::Platter => self.platter.0[3] as f32 / 255.0,
        }
    }

    /// Accent colour for text and icons: the mean, lifted for contrast on dark glass.
    pub fn accent_text(&self) -> Color {
        self.accent.mix(Color::WHITE, 0.35)
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
}

impl Default for Theme {
    fn default() -> Self {
        Self::glass(Gradient::default(), 1.0)
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
    fn gradient_samples_stops_and_round_trips() {
        let g = Gradient::preset(1);
        assert_eq!(g.sample(0.0, 0.0), GRADIENTS[1].1[0]);
        assert_eq!(g.sample(1.0, 0.0), *GRADIENTS[1].1.last().unwrap());
        assert_eq!(g.preset_index(), Some(1));
        let parsed = Gradient::parse_stops(&g.stops_text()).unwrap();
        assert_eq!(parsed, g.stops());
        assert_eq!(Gradient::parse_stops("junk,#zz"), None);
        let single = Gradient::single(Color::rgb(10, 200, 90));
        assert_eq!(single.sample(0.7, 0.0), Color::rgb(10, 200, 90));
        assert_eq!(single.lut(0.0).len(), 256 * 4);
        // Hue rotation by half a turn moves red to cyan.
        let red = Gradient::single(Color::rgb(255, 0, 0)).sample(0.5, 0.5);
        assert_eq!(red, Color::rgb(0, 255, 255));
        let mut g = Gradient::single(Color::rgb(255, 0, 0));
        g.set_count(4);
        assert_eq!(g.stops().len(), 4);
        assert_ne!(g.stops[1], g.stops[0], "new stops walk the hue");
        g.set_count(99);
        assert_eq!(g.count, Gradient::MAX_STOPS);
    }

    #[test]
    fn px_scales_and_rounds() {
        let t = Theme::dark(ACCENTS[0].1, 1.5);
        assert_eq!(t.px(10.0), 15.0);
        assert_eq!(t.sp(2), 12.0);
        assert_eq!(Color::rgb(0, 0, 0).mix(Color::WHITE, 0.5).0[0], 128);
    }
}
