//! D3D11 backend of [`crate::render::MenuRenderer`] ([`mesh::MeshRenderer`]) and a WARP
//! render target with readback ([`Offscreen`]) for previews and tests.

mod context_state;
pub mod mesh;

use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_FLAG, D3D11_BIND_RENDER_TARGET, D3D11_CPU_ACCESS_FLAG, D3D11_CPU_ACCESS_READ,
    D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING,
    ID3D11Device, ID3D11DeviceContext, ID3D11RenderTargetView, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_R8G8B8A8_UNORM;

use crate::draw::{DrawList, TextureDelta};
use crate::render::MenuRenderer;
use mesh::{MeshRenderer, create_texture, render_target_view, texture_desc, warp_device};

fn hr<E: std::fmt::Display>(what: &'static str) -> impl FnOnce(E) -> String {
    move |e| format!("{what}: {e}")
}

/// WARP render target with readback.
pub struct Offscreen {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    texture: ID3D11Texture2D,
    rtv: ID3D11RenderTargetView,
    renderer: MeshRenderer,
    pub size: [u32; 2],
}

impl Offscreen {
    pub fn new(width: u32, height: u32) -> Result<Self, String> {
        let device = warp_device().map_err(|e| format!("WARP: {e}"))?;
        // SAFETY: The device is live; one owned immediate-context reference is returned.
        let context =
            unsafe { device.GetImmediateContext() }.map_err(|e| format!("context: {e}"))?;
        let desc = texture_desc(
            DXGI_FORMAT_R8G8B8A8_UNORM,
            [width, height],
            D3D11_USAGE_DEFAULT,
            D3D11_BIND_RENDER_TARGET,
            D3D11_CPU_ACCESS_FLAG(0),
        );
        // SAFETY: no initial-data pointers are supplied.
        let texture =
            unsafe { create_texture(&device, &desc, None) }.map_err(|e| format!("texture: {e}"))?;
        let rtv = render_target_view(&device, &texture).map_err(|e| format!("rtv: {e}"))?;
        let renderer = MeshRenderer::new(device.clone())?;
        Ok(Self {
            device,
            context,
            texture,
            rtv,
            renderer,
            size: [width, height],
        })
    }

    pub fn clear(&self, rgba: [f32; 4]) {
        // SAFETY: the RTV is owned and live; the colour array outlives the call.
        unsafe { self.context.ClearRenderTargetView(&self.rtv, &rgba) };
    }

    pub fn paint(&mut self, textures: &[TextureDelta], list: &DrawList) -> Result<(), String> {
        self.renderer
            .paint(&self.texture, textures, list, self.size)
    }

    pub fn pixels(&self) -> Result<Vec<u8>, String> {
        let [width, height] = self.size;
        let desc = texture_desc(
            DXGI_FORMAT_R8G8B8A8_UNORM,
            self.size,
            D3D11_USAGE_STAGING,
            D3D11_BIND_FLAG(0),
            D3D11_CPU_ACCESS_READ,
        );
        // SAFETY: no initial-data pointers are supplied.
        let staging = unsafe { create_texture(&self.device, &desc, None) }
            .map_err(|e| format!("staging: {e}"))?;
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        let mut out = Vec::with_capacity((width * height * 4) as usize);
        // SAFETY: matching textures on the same context; Map blocks until the copy is done
        // and every read stays within RowPitch * height; copied before Unmap.
        unsafe {
            self.context.CopyResource(&staging, &self.texture);
            self.context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .map_err(|e| format!("map: {e}"))?;
            for y in 0..height {
                let row = mapped
                    .pData
                    .cast::<u8>()
                    .add((y * mapped.RowPitch) as usize);
                out.extend_from_slice(core::slice::from_raw_parts(row, (width * 4) as usize));
            }
            self.context.Unmap(&staging, 0);
        }
        Ok(out)
    }
}
