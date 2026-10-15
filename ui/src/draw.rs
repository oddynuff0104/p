//! Backend-neutral output of the menu: one indexed triangle list plus draw commands
//! (clip rectangle + texture per index range) and texture changes.
//!
//! Conventions every renderer backend follows:
//! - positions in physical pixels, origin top left;
//! - `color` is premultiplied RGBA8 (`r | g << 8 | b << 16 | a << 24`), gamma-encoded
//!   sRGB; texels too. Blend `ONE, ONE_MINUS_SRC_ALPHA`, sample as UNORM, multiply in
//!   gamma space; only on an `_SRGB` target convert the result to linear before writing;
//! - one scissor rectangle per [`DrawCmd`], no depth, no culling;
//! - negative `uv.y` selects a built-in source instead of the command's texture (so glass
//!   and gradients batch with text in the same draw calls):
//!   - `uv.y` in `[-1.5, -0.5)` ([`BACKDROP_UV`]): the blurred back buffer behind the
//!     pixel (screen position, linear filter); the backend fills it once per frame when
//!     [`DrawList::backdrop`] is set, else a dark opaque texel;
//!   - `uv.y < -1.5` ([`gradient_uv`]): the accent gradient [`GRADIENT_TEXTURE`] (a
//!     256×1 lookup row) at `u = uv.x` (clamped to 0..1).

use core::ops::Range;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub min: [f32; 2],
    pub max: [f32; 2],
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self {
            min: [x, y],
            max: [x + w, y + h],
        }
    }
    pub fn width(&self) -> f32 {
        self.max[0] - self.min[0]
    }
    pub fn height(&self) -> f32 {
        self.max[1] - self.min[1]
    }
    pub fn contains(&self, p: [f32; 2]) -> bool {
        p[0] >= self.min[0] && p[0] < self.max[0] && p[1] >= self.min[1] && p[1] < self.max[1]
    }
    pub fn intersect(&self, o: &Rect) -> Rect {
        Rect {
            min: [self.min[0].max(o.min[0]), self.min[1].max(o.min[1])],
            max: [self.max[0].min(o.max[0]), self.max[1].min(o.max[1])],
        }
    }
    pub fn shrink(&self, by: f32) -> Rect {
        Rect {
            min: [self.min[0] + by, self.min[1] + by],
            max: [self.max[0] - by, self.max[1] - by],
        }
    }
    /// Smallest rectangle containing both.
    pub fn union(&self, o: &Rect) -> Rect {
        Rect {
            min: [self.min[0].min(o.min[0]), self.min[1].min(o.min[1])],
            max: [self.max[0].max(o.max[0]), self.max[1].max(o.max[1])],
        }
    }
    /// Pixel scissor `[left, top, right, bottom]` clamped to `size`; `None` when empty.
    pub fn scissor(&self, size: [u32; 2]) -> Option<[u32; 4]> {
        let c = |v: f32, max: u32| v.round().clamp(0.0, max as f32) as u32;
        let r = [
            c(self.min[0], size[0]),
            c(self.min[1], size[1]),
            c(self.max[0], size[0]),
            c(self.max[1], size[1]),
        ];
        (r[0] < r[2] && r[1] < r[3]).then_some(r)
    }
}

/// Premultiplied RGBA8 from straight (non-premultiplied) components.
pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> u32 {
    let a32 = a as u32;
    let r = (r as u32 * a32 + 127) / 255;
    let g = (g as u32 * a32 + 127) / 255;
    let b = (b as u32 * a32 + 127) / 255;
    r | (g << 8) | (b << 16) | (a32 << 24)
}

/// Same layout as the D3D11 input layout: `R32G32_FLOAT`, `R32G32_FLOAT`, `R8G8B8A8_UNORM`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vertex {
    pub pos: [f32; 2],
    pub uv: [f32; 2],
    pub color: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TextureId(pub u32);

/// The font atlas (texel [0, 0] is opaque white for solid fills).
pub const FONT_TEXTURE: TextureId = TextureId(0);
/// The accent gradient: 256×1 premultiplied RGBA8, sampled through [`gradient_uv`].
pub const GRADIENT_TEXTURE: TextureId = TextureId(1);
/// Width of [`GRADIENT_TEXTURE`].
pub const GRADIENT_WIDTH: u32 = 256;

/// UV that samples the blurred back buffer at the pixel's own screen position.
pub const BACKDROP_UV: [f32; 2] = [-1.0, -1.0];

/// UV that samples the accent gradient at `t` (0 = first stop, 1 = last).
pub const fn gradient_uv(t: f32) -> [f32; 2] {
    [t, -2.0]
}

#[derive(Clone, Debug, PartialEq)]
pub struct DrawCmd {
    pub clip: Rect,
    pub texture: TextureId,
    pub indices: Range<u32>,
}

/// Premultiplied RGBA8 texels, tightly packed rows.
/// `Update` and `Free` are part of the renderer contract (the D3D11 renderer and its
/// tests handle them); the kit itself only creates atlases.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum TextureDelta {
    Create {
        id: TextureId,
        size: [u32; 2],
        rgba: Vec<u8>,
        nearest: bool,
    },
    Update {
        id: TextureId,
        pos: [u32; 2],
        size: [u32; 2],
        rgba: Vec<u8>,
    },
    Free(TextureId),
}

#[derive(Clone, Debug, Default)]
pub struct DrawList {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub commands: Vec<DrawCmd>,
    /// Union of every shape that samples [`BACKDROP_UV`] (pixels); `None` = the backend
    /// skips the back-buffer copy and blur entirely.
    pub backdrop: Option<Rect>,
    clip: Vec<Rect>,
}

/// UV of the white texel in [`FONT_TEXTURE`].
pub const WHITE_UV: [f32; 2] = [1.0 / 512.0, 1.0 / 512.0];

impl DrawList {
    /// Appends `other` (drawn on top), keeping its commands and clips.
    pub fn append(&mut self, other: &DrawList) {
        let (base_v, base_i) = (self.vertices.len() as u32, self.indices.len() as u32);
        self.vertices.extend_from_slice(&other.vertices);
        self.indices
            .extend(other.indices.iter().map(|i| i + base_v));
        self.commands.extend(other.commands.iter().map(|c| DrawCmd {
            clip: c.clip,
            texture: c.texture,
            indices: c.indices.start + base_i..c.indices.end + base_i,
        }));
        if let Some(r) = other.backdrop {
            self.add_backdrop(r);
        }
    }

    pub fn clear(&mut self) {
        self.vertices.clear();
        self.indices.clear();
        self.commands.clear();
        self.clip.clear();
        self.backdrop = None;
    }

    /// Marks `rect` as sampling the blurred back buffer.
    pub fn add_backdrop(&mut self, rect: Rect) {
        self.backdrop = Some(match self.backdrop {
            Some(r) => r.union(&rect),
            None => rect,
        });
    }

    pub fn push_clip(&mut self, rect: Rect) {
        let rect = match self.clip.last() {
            Some(top) => top.intersect(&rect),
            None => rect,
        };
        self.clip.push(rect);
    }

    pub fn pop_clip(&mut self) {
        self.clip.pop();
    }

    pub fn current_clip(&self) -> Rect {
        self.clip
            .last()
            .copied()
            .unwrap_or(Rect::new(0.0, 0.0, f32::MAX / 2.0, f32::MAX / 2.0))
    }

    /// Opens or extends the command for the current clip and `texture`.
    fn command(&mut self, texture: TextureId, add: u32) {
        let clip = self.current_clip();
        let end = self.indices.len() as u32;
        match self.commands.last_mut() {
            Some(c) if c.clip == clip && c.texture == texture && c.indices.end == end => {
                c.indices.end += add
            }
            _ => self.commands.push(DrawCmd {
                clip,
                texture,
                indices: end..end + add,
            }),
        }
    }

    /// Reserves `add` indices in the command for the current clip and `texture`; callers
    /// then push their vertices and exactly `add` indices (custom meshes, `primitives`).
    pub fn open_command(&mut self, texture: TextureId, add: u32) {
        self.command(texture, add);
    }

    pub fn quad_uv(&mut self, rect: Rect, uv: Rect, color: u32, texture: TextureId) {
        if color >> 24 == 0 {
            return;
        }
        self.command(texture, 6);
        let base = self.vertices.len() as u32;
        let v = |pos, uv| Vertex { pos, uv, color };
        self.vertices.extend([
            v(rect.min, uv.min),
            v([rect.max[0], rect.min[1]], [uv.max[0], uv.min[1]]),
            v(rect.max, uv.max),
            v([rect.min[0], rect.max[1]], [uv.min[0], uv.max[1]]),
        ]);
        self.indices
            .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    pub fn rect(&mut self, rect: Rect, color: u32) {
        let uv = Rect {
            min: WHITE_UV,
            max: WHITE_UV,
        };
        self.quad_uv(rect, uv, color, FONT_TEXTURE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_merge_per_clip_and_texture() {
        let mut list = DrawList::default();
        list.rect(Rect::new(0.0, 0.0, 10.0, 10.0), rgba(255, 0, 0, 255));
        list.rect(Rect::new(10.0, 0.0, 10.0, 10.0), rgba(0, 255, 0, 255));
        list.push_clip(Rect::new(0.0, 0.0, 5.0, 5.0));
        list.rect(Rect::new(0.0, 0.0, 10.0, 10.0), rgba(0, 0, 255, 128));
        list.pop_clip();
        assert_eq!(list.commands.len(), 2);
        assert_eq!(list.commands[0].indices, 0..12);
        assert_eq!(list.commands[1].indices, 12..18);
        assert_eq!(list.commands[1].clip, Rect::new(0.0, 0.0, 5.0, 5.0));
        assert_eq!(list.vertices[8].color, 0x80_80_00_00);
    }

    #[test]
    fn scissor_clamps() {
        let r = Rect::new(-5.0, 10.0, 60.0, 400.0);
        assert_eq!(r.scissor([64, 300]), Some([0, 10, 55, 300]));
        assert_eq!(Rect::new(100.0, 0.0, 5.0, 5.0).scissor([64, 64]), None);
    }
}
