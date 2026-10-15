//! Multi-size glyph atlas for the kit: caption, body, section and title are each
//! rasterised natively at their pixel size (no scaling of a single atlas, so small text
//! stays sharp), packed into one texture ([`FONT_TEXTURE`]) with a 4×4 white block at
//! the origin for solid fills (covers [`super::super::draw::WHITE_UV`] at any atlas size).
//! Coverage gets a mild contrast curve (`c^0.85`): light text on dark backgrounds would
//! otherwise look thin with plain linear coverage.

use std::collections::HashMap;

use ab_glyph::{Font as _, FontRef, PxScale, ScaleFont as _};

use crate::draw::{DrawList, FONT_TEXTURE, Rect, TextureDelta};
use crate::font::FONT_TTF;
use crate::theme::{TextStyle, Theme};

#[derive(Clone, Copy, Debug)]
struct Glyph {
    uv: Rect,
    offset: [f32; 2],
    size: [f32; 2],
    advance: f32,
}

pub struct SizedFont {
    glyphs: HashMap<char, Glyph>,
    pub px: f32,
    pub line_height: f32,
    /// Height of capital letters (for optical vertical centring).
    pub cap_height: f32,
    pub ascent: f32,
}

pub struct FontSet {
    fonts: [SizedFont; 4],
    pub atlas_size: u32,
}

fn chars() -> impl Iterator<Item = char> {
    (' '..='~')
        .chain('\u{a0}'..='\u{ff}')
        .chain(['…', '·', '–', '—', '•', '›'])
}

const STYLES: [TextStyle; 4] = [
    TextStyle::Caption,
    TextStyle::Body,
    TextStyle::Section,
    TextStyle::Title,
];

fn index(style: TextStyle) -> usize {
    match style {
        TextStyle::Caption => 0,
        TextStyle::Body => 1,
        TextStyle::Section => 2,
        TextStyle::Title => 3,
    }
}

impl FontSet {
    /// Rasterises all four sizes of `theme`; the texture is to be uploaded once.
    pub fn new(theme: &Theme) -> Result<(Self, TextureDelta), String> {
        let sizes = STYLES.map(|s| (theme.font_points(s) * theme.scale).round().max(6.0));
        let mut last = String::new();
        for atlas in [512u32, 1024, 2048, 4096] {
            match Self::build(sizes, atlas) {
                Ok(v) => return Ok(v),
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    fn build(sizes: [f32; 4], atlas: u32) -> Result<(Self, TextureDelta), String> {
        let font = FontRef::try_from_slice(FONT_TTF).map_err(|e| format!("Schrift: {e}"))?;
        let size = atlas as usize;
        let mut alpha = vec![0u8; size * size];
        for y in 0..4 {
            alpha[y * size..y * size + 4].fill(255);
        }
        let (mut x, mut y, mut row) = (6usize, 0usize, 4usize);
        let mut build_one = |px: f32| -> Result<SizedFont, String> {
            let scaled = font.as_scaled(PxScale::from(px));
            let ascent = scaled.ascent();
            let mut glyphs = HashMap::new();
            for ch in chars() {
                let id = scaled.glyph_id(ch);
                let mut glyph = Glyph {
                    uv: Rect::default(),
                    offset: [0.0; 2],
                    size: [0.0; 2],
                    advance: scaled.h_advance(id),
                };
                let positioned = id.with_scale_and_position(px, ab_glyph::point(0.0, ascent));
                if let Some(outline) = font.outline_glyph(positioned) {
                    let b = outline.px_bounds();
                    let (w, h) = (b.width() as usize, b.height() as usize);
                    if x + w + 1 > size {
                        (x, y, row) = (0, y + row + 1, 0);
                    }
                    if y + h > size {
                        return Err("Schrift-Atlas zu klein".into());
                    }
                    outline.draw(|gx, gy, c| {
                        if gx < w as u32 && gy < h as u32 {
                            let v = c.clamp(0.0, 1.0).powf(0.85);
                            alpha[(y + gy as usize) * size + x + gx as usize] =
                                (v * 255.0).round() as u8;
                        }
                    });
                    let s = atlas as f32;
                    glyph.uv = Rect {
                        min: [x as f32 / s, y as f32 / s],
                        max: [(x + w) as f32 / s, (y + h) as f32 / s],
                    };
                    glyph.offset = [b.min.x, b.min.y];
                    glyph.size = [w as f32, h as f32];
                    x += w + 1;
                    row = row.max(h);
                }
                glyphs.insert(ch, glyph);
            }
            let cap_height = font
                .outline_glyph(scaled.glyph_id('H').with_scale(px))
                .map_or(px * 0.7, |o| o.px_bounds().height());
            Ok(SizedFont {
                glyphs,
                px,
                line_height: (ascent - scaled.descent() + scaled.line_gap()).ceil(),
                cap_height,
                ascent,
            })
        };
        let fonts = [
            build_one(sizes[0])?,
            build_one(sizes[1])?,
            build_one(sizes[2])?,
            build_one(sizes[3])?,
        ];
        let rgba = alpha.iter().flat_map(|&a| [a, a, a, a]).collect();
        let texture = TextureDelta::Create {
            id: FONT_TEXTURE,
            size: [atlas; 2],
            rgba,
            nearest: true,
        };
        Ok((
            Self {
                fonts,
                atlas_size: atlas,
            },
            texture,
        ))
    }

    pub fn get(&self, style: TextStyle) -> &SizedFont {
        &self.fonts[index(style)]
    }
}

impl SizedFont {
    fn glyph(&self, ch: char) -> Option<&Glyph> {
        self.glyphs.get(&ch).or_else(|| self.glyphs.get(&'?'))
    }

    pub fn width(&self, text: &str) -> f32 {
        text.chars()
            .filter_map(|c| self.glyph(c))
            .map(|g| g.advance)
            .sum()
    }

    /// `text` cut with "…" to fit `max` px.
    pub fn ellipsize<'a>(&self, text: &'a str, max: f32) -> std::borrow::Cow<'a, str> {
        if self.width(text) <= max {
            return text.into();
        }
        let budget = max - self.width("…");
        let mut w = 0.0;
        let mut out = String::new();
        for c in text.chars() {
            let a = self.glyph(c).map_or(0.0, |g| g.advance);
            if w + a > budget {
                break;
            }
            w += a;
            out.push(c);
        }
        out.push('…');
        out.into()
    }

    /// Greedy word wrap to `max_width` px; honours `\n`, splits over-long words.
    pub fn wrap(&self, text: &str, max_width: f32) -> Vec<String> {
        let mut lines = Vec::new();
        for paragraph in text.split('\n') {
            let mut line = String::new();
            for word in paragraph.split_whitespace() {
                let candidate = if line.is_empty() {
                    word.to_string()
                } else {
                    format!("{line} {word}")
                };
                if self.width(&candidate) <= max_width {
                    line = candidate;
                    continue;
                }
                if !line.is_empty() {
                    lines.push(std::mem::take(&mut line));
                }
                for ch in word.chars() {
                    line.push(ch);
                    if self.width(&line) > max_width && line.chars().count() > 1 {
                        line.pop();
                        lines.push(std::mem::take(&mut line));
                        line.push(ch);
                    }
                }
            }
            lines.push(line);
        }
        lines
    }

    /// Draws `text` with its line box top-left at `pos`; returns the advance.
    pub fn draw(&self, list: &mut DrawList, pos: [f32; 2], text: &str, color: u32) -> f32 {
        let mut pen = pos[0].round();
        let top = pos[1].round();
        for g in text.chars().filter_map(|c| self.glyph(c)) {
            if g.size[0] > 0.0 {
                let rect = Rect::new(pen + g.offset[0], top + g.offset[1], g.size[0], g.size[1]);
                list.quad_uv(rect, g.uv, color, FONT_TEXTURE);
            }
            pen += g.advance;
        }
        pen - pos[0]
    }

    /// Top of the line box so capitals are optically centred on `center_y`.
    pub fn top_for_center(&self, center_y: f32) -> f32 {
        (center_y - self.ascent + self.cap_height / 2.0).round()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_distinct_and_white_block_exists() {
        for scale in [1.0, 1.5, 2.0, 4.0] {
            let theme = Theme::dark(crate::theme::ACCENTS[0].1, scale);
            let (set, tex) = FontSet::new(&theme).unwrap();
            let TextureDelta::Create { size, rgba, .. } = tex else {
                panic!()
            };
            let w = size[0] as usize;
            for (x, y) in [(0, 0), (2, 2), (3, 3)] {
                assert_eq!(rgba[(y * w + x) * 4], 255);
            }
            let wb = set.get(TextStyle::Body).width("Settings");
            let wt = set.get(TextStyle::Title).width("Settings");
            let wc = set.get(TextStyle::Caption).width("Settings");
            assert!(wc < wb && wb < wt, "{scale}: {wc} {wb} {wt}");
        }
    }

    #[test]
    fn ellipsize_fits() {
        let (set, _) = FontSet::new(&Theme::default()).unwrap();
        let f = set.get(TextStyle::Body);
        let s = f.ellipsize("A very long label that does not fit", 80.0);
        assert!(s.ends_with('…') && f.width(&s) <= 80.0, "{s}");
        assert_eq!(f.ellipsize("Short", 200.0), "Short");
    }
}
