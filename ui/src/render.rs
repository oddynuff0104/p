//! Backend-agnostic renderer contract: D3D11 on Windows (`crate::d3d11`), Metal on macOS
//! (`crate::metal`).
//! A backend only turns [`DrawList`] + [`TextureDelta`]s into draw calls; the
//! conventions are documented in [`super::draw`].

use super::draw::{DrawList, TextureDelta};

pub trait MenuRenderer {
    /// Where to draw: D3D11 the backbuffer texture, Metal a texture plus command buffer.
    type Target<'t>;

    /// Creates, patches or frees textures. Apply creates/updates before [`Self::draw`];
    /// a `Free` from the same frame may be applied afterwards.
    fn apply(&mut self, delta: &TextureDelta) -> Result<(), String>;

    /// Draws all commands in order into a target of `size_px` physical pixels.
    fn draw(
        &mut self,
        target: Self::Target<'_>,
        list: &DrawList,
        size_px: [u32; 2],
    ) -> Result<(), String>;

    /// One frame: texture changes (frees last), then the draw list.
    fn paint(
        &mut self,
        target: Self::Target<'_>,
        textures: &[TextureDelta],
        list: &DrawList,
        size_px: [u32; 2],
    ) -> Result<(), String> {
        let is_free = |d: &&TextureDelta| matches!(d, TextureDelta::Free(_));
        for delta in textures.iter().filter(|d| !is_free(d)) {
            self.apply(delta)?;
        }
        let drawn = self.draw(target, list, size_px);
        for delta in textures.iter().filter(is_free) {
            self.apply(delta)?;
        }
        drawn
    }
}
