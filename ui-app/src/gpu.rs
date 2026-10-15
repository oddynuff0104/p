//! D3D11 device, a premultiplied-alpha composition swapchain and the DirectComposition
//! tree that puts it on the window. Transparent pixels show the desktop, so the shell's
//! rounded corners and soft shadow show the desktop behind them.

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice, IDCompositionDevice, IDCompositionTarget, IDCompositionVisual,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_UNKNOWN,
    DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    DXGI_PRESENT, DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FLAG,
    DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL, DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGIDevice, IDXGIFactory2,
    IDXGISwapChain1,
};
use windows::core::Interface;

use poly_ui::d3d11::mesh::{MeshRenderer, create_device, render_target_view};
use poly_ui::draw::{DrawList, TextureDelta};
use poly_ui::render::MenuRenderer;

pub struct Gpu {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    swapchain: IDXGISwapChain1,
    renderer: MeshRenderer,
    size: [u32; 2],
    // The composition tree lives as long as the window shows the swapchain.
    _composition: (
        IDCompositionDevice,
        IDCompositionTarget,
        IDCompositionVisual,
    ),
}

fn err(what: &'static str) -> impl FnOnce(windows::core::Error) -> String {
    move |e| format!("{what}: {e}")
}

impl Gpu {
    pub fn new(hwnd: HWND, size: [u32; 2]) -> Result<Self, String> {
        let flags = D3D11_CREATE_DEVICE_BGRA_SUPPORT;
        let device = create_device(None, D3D_DRIVER_TYPE_HARDWARE, flags)
            .or_else(|_| create_device(None, D3D_DRIVER_TYPE_WARP, flags))?;
        // SAFETY: The device is live; one owned immediate-context reference is returned.
        let context = unsafe { device.GetImmediateContext() }.map_err(err("context"))?;
        let dxgi: IDXGIDevice = device.cast().map_err(err("IDXGIDevice"))?;
        // SAFETY: The DXGI device is live; adapter and factory come back as owned references.
        let factory: IDXGIFactory2 = unsafe { dxgi.GetAdapter().and_then(|a| a.GetParent()) }
            .map_err(err("IDXGIFactory2"))?;
        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: size[0].max(1),
            Height: size[1].max(1),
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
            AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
            ..Default::default()
        };
        // SAFETY: The device and the descriptor are live for the call; no output restriction.
        let swapchain = unsafe { factory.CreateSwapChainForComposition(&device, &desc, None) }
            .map_err(err("CreateSwapChainForComposition"))?;
        // SAFETY: The DXGI device and the window are live; every call returns owned references
        // that this struct keeps until the window is gone.
        let composition = unsafe {
            let dcomp: IDCompositionDevice =
                DCompositionCreateDevice(&dxgi).map_err(err("DCompositionCreateDevice"))?;
            let target = dcomp
                .CreateTargetForHwnd(hwnd, true)
                .map_err(err("CreateTargetForHwnd"))?;
            let visual = dcomp.CreateVisual().map_err(err("CreateVisual"))?;
            visual.SetContent(&swapchain).map_err(err("SetContent"))?;
            target.SetRoot(&visual).map_err(err("SetRoot"))?;
            dcomp.Commit().map_err(err("Commit"))?;
            (dcomp, target, visual)
        };
        let renderer = MeshRenderer::new(device.clone())?;
        Ok(Self {
            device,
            context,
            swapchain,
            renderer,
            size: [desc.Width, desc.Height],
            _composition: composition,
        })
    }

    pub fn resize(&mut self, size: [u32; 2]) -> Result<(), String> {
        let size = [size[0].max(1), size[1].max(1)];
        if size == self.size {
            return Ok(());
        }
        // SAFETY: No back buffer reference is held between frames.
        unsafe {
            self.swapchain.ResizeBuffers(
                0,
                size[0],
                size[1],
                DXGI_FORMAT_UNKNOWN,
                DXGI_SWAP_CHAIN_FLAG(0),
            )
        }
        .map_err(err("ResizeBuffers"))?;
        self.size = size;
        Ok(())
    }

    /// Clears to transparent, draws one frame and presents it on the next vblank.
    pub fn frame(&mut self, textures: &[TextureDelta], list: &DrawList) -> Result<(), String> {
        // SAFETY: Buffer 0 is the current back buffer of a flip-model swapchain.
        let buffer: ID3D11Texture2D =
            unsafe { self.swapchain.GetBuffer(0) }.map_err(err("GetBuffer"))?;
        let rtv = render_target_view(&self.device, &buffer).map_err(err("rtv"))?;
        // SAFETY: The view is owned and live; the colour array outlives the call.
        unsafe { self.context.ClearRenderTargetView(&rtv, &[0.0; 4]) };
        drop(rtv);
        self.renderer.paint(&buffer, textures, list, self.size)?;
        drop(buffer);
        // SAFETY: The swapchain is live; presenting needs no other arguments.
        unsafe { self.swapchain.Present(1, DXGI_PRESENT(0)) }
            .ok()
            .map_err(err("Present"))
    }
}
