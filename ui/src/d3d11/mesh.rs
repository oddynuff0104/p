//! D3D11 backend of [`MenuRenderer`]: draws draw lists into any RGBA8/BGRA8 render target.
//!
//! Own `ID3DDeviceContextState` (`context_state.rs`) swapped in for drawing and back
//! afterwards (game state untouched), premultiplied
//! alpha, RTV created per frame. All meshes of a frame share one dynamic vertex and
//! index buffer (grown on demand); one scissor rectangle per primitive. Textures are
//! RGBA8 UNORM, sampled point or linear per texture; on an `_SRGB` backbuffer the
//! pixel shader converts the gamma-space result to linear before writing.
//! COM bindings: the `windows` crate (owned references drop with their values).

use core::ffi::{CStr, c_void};
use std::collections::HashMap;

use windows::Win32::Foundation::{E_POINTER, HMODULE, RECT};
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_WARP, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
    D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST, ID3DBlob,
};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM_SRGB, DXGI_FORMAT_R8G8B8A8_UNORM,
    DXGI_FORMAT_R8G8B8A8_UNORM_SRGB, DXGI_FORMAT_R32_UINT, DXGI_FORMAT_R32G32_FLOAT,
    DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::IDXGIAdapter;
use windows::core::{Interface, PCSTR};

use super::context_state::{ContextState, create_context_state};
use super::hr;
use crate::draw::{DrawList, TextureDelta, TextureId, Vertex};
use crate::render::MenuRenderer;

const SHADER: &str = r#"
cbuffer Screen : register(b0) { float2 screen_points; float srgb_target; float pad; };
Texture2D tex : register(t0);
SamplerState samp : register(s0);

struct VsIn { float2 pos : POSITION; float2 uv : TEXCOORD; float4 color : COLOR; };
struct PsIn { float4 pos : SV_POSITION; float2 uv : TEXCOORD; float4 color : COLOR; };

PsIn vs_main(VsIn i) {
    PsIn o;
    o.pos = float4(i.pos.x / screen_points.x * 2.0 - 1.0, 1.0 - i.pos.y / screen_points.y * 2.0, 0.0, 1.0);
    o.uv = i.uv;
    o.color = i.color;
    return o;
}

float3 to_linear(float3 c) {
    return lerp(pow((c + 0.055) / 1.055, 2.4), c / 12.92, step(c, 0.04045));
}

float4 ps_main(PsIn i) : SV_Target {
    float4 c = i.color * tex.Sample(samp, i.uv);
    if (srgb_target > 0.5 && c.a > 0.0) {
        c.rgb = to_linear(c.rgb / c.a) * c.a;
    }
    return c;
}
"#;

const VERTEX_SIZE: usize = size_of::<Vertex>();
const _: () = assert!(
    VERTEX_SIZE == 20,
    "menu Vertex: pos, uv (f32x2), color (RGBA8)"
);

/// `D3D11CreateDevice` (feature level 11.1, else 11.0). With `adapter`, `driver` must be
/// `D3D_DRIVER_TYPE_UNKNOWN`.
pub fn create_device(
    adapter: Option<&IDXGIAdapter>,
    driver: D3D_DRIVER_TYPE,
    flags: D3D11_CREATE_DEVICE_FLAG,
) -> Result<ID3D11Device, String> {
    let create = |levels: &[_]| {
        let mut device = None;
        // SAFETY: The optional adapter is borrowed, levels is a live slice and the output slot is
        // writable; success stores one owned device reference.
        unsafe {
            D3D11CreateDevice(
                adapter,
                driver,
                HMODULE::default(),
                flags,
                Some(levels),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                None,
            )
        }
        .and_then(|()| device.ok_or_else(|| windows::core::Error::from_hresult(E_POINTER)))
    };
    // Runtimes without D3D11.1 reject a list containing 11_1 (E_INVALIDARG).
    create(&[D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0])
        .or_else(|_| create(&[D3D_FEATURE_LEVEL_11_0]))
        .map_err(|e| format!("D3D11CreateDevice: {e}"))
}

/// A WARP device (tests, offscreen preview).
pub fn warp_device() -> Result<ID3D11Device, String> {
    create_device(None, D3D_DRIVER_TYPE_WARP, D3D11_CREATE_DEVICE_FLAG(0))
}

/// A single-mip 2D texture descriptor.
pub fn texture_desc(
    format: DXGI_FORMAT,
    size: [u32; 2],
    usage: D3D11_USAGE,
    bind: D3D11_BIND_FLAG,
    cpu: D3D11_CPU_ACCESS_FLAG,
) -> D3D11_TEXTURE2D_DESC {
    D3D11_TEXTURE2D_DESC {
        Width: size[0],
        Height: size[1],
        MipLevels: 1,
        ArraySize: 1,
        Format: format,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: usage,
        BindFlags: bind.0 as u32,
        CPUAccessFlags: cpu.0 as u32,
        MiscFlags: 0,
    }
}

/// Creates a texture from a descriptor and optional initial data.
///
/// # Safety
///
/// When `initial` is present, it must describe readable texels for every subresource of
/// `desc` (format, dimensions, pitches) until this call returns.
pub(crate) unsafe fn create_texture(
    device: &ID3D11Device,
    desc: &D3D11_TEXTURE2D_DESC,
    initial: Option<&D3D11_SUBRESOURCE_DATA>,
) -> windows::core::Result<ID3D11Texture2D> {
    let mut texture = None;
    // SAFETY: Forwarded to the caller for the initial data; desc and the output slot are live.
    unsafe { device.CreateTexture2D(desc, initial.map(|d| d as *const _), Some(&mut texture)) }?;
    texture.ok_or_else(|| windows::core::Error::from_hresult(E_POINTER))
}

/// A render target view of the whole resource.
pub fn render_target_view(
    device: &ID3D11Device,
    resource: &ID3D11Texture2D,
) -> windows::core::Result<ID3D11RenderTargetView> {
    let mut view = None;
    // SAFETY: Both objects are live; the output slot receives one owned reference.
    unsafe { device.CreateRenderTargetView(resource, None, Some(&mut view)) }?;
    view.ok_or_else(|| windows::core::Error::from_hresult(E_POINTER))
}

/// HLSL zur Laufzeit übersetzen (`D3DCompile` aus `d3dcompiler_47.dll`, ab Windows 10 im System).
fn compile_shader(source: &str, entry: &CStr, target: &CStr) -> Result<ID3DBlob, String> {
    const D3DCOMPILE_OPTIMIZATION_LEVEL3: u32 = 1 << 15;
    let mut blob = None;
    let mut errors = None;
    // SAFETY: source, entry and target remain readable for the synchronous compile. Optional
    // name/include/macros are null; both output slots receive owned blobs (or stay None).
    let result = unsafe {
        D3DCompile(
            source.as_ptr().cast(),
            source.len(),
            PCSTR::null(),
            None,
            None,
            PCSTR(entry.as_ptr().cast()),
            PCSTR(target.as_ptr().cast()),
            D3DCOMPILE_OPTIMIZATION_LEVEL3,
            0,
            &mut blob,
            Some(&mut errors),
        )
    };
    let entry = entry.to_string_lossy();
    match (result, blob) {
        (Ok(()), Some(blob)) => Ok(blob),
        (result, _) => {
            let e = result.err().map_or("kein Blob".into(), |e| e.to_string());
            Err(match errors {
                Some(errors) => format!("D3DCompile({entry}): {e}: {}", blob_text(&errors)),
                None => format!("D3DCompile({entry}): {e}"),
            })
        }
    }
}

/// The bytes of a blob, borrowed for its lifetime.
fn blob_bytes(blob: &ID3DBlob) -> &[u8] {
    // SAFETY: The blob owns GetBufferSize readable bytes at GetBufferPointer for its lifetime,
    // which bounds the returned borrow.
    unsafe {
        core::slice::from_raw_parts(blob.GetBufferPointer().cast::<u8>(), blob.GetBufferSize())
    }
}

fn blob_text(blob: &ID3DBlob) -> String {
    String::from_utf8_lossy(blob_bytes(blob))
        .trim_end_matches('\0')
        .trim()
        .to_string()
}

pub struct MeshRenderer {
    device: ID3D11Device,
    context: ID3D11DeviceContext1,
    state: ID3DDeviceContextState,
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
    layout: ID3D11InputLayout,
    blend: ID3D11BlendState,
    rasterizer: ID3D11RasterizerState,
    depth: ID3D11DepthStencilState,
    /// [linear, point]
    samplers: [ID3D11SamplerState; 2],
    constants: ID3D11Buffer,
    vertices: Option<Dynamic>,
    indices: Option<Dynamic>,
    textures: HashMap<TextureId, Texture>,
}

struct Dynamic {
    buffer: ID3D11Buffer,
    /// Capacity in elements.
    capacity: usize,
}

struct Texture {
    texture: ID3D11Texture2D,
    view: ID3D11ShaderResourceView,
    size: [u32; 2],
    nearest: bool,
}

/// Turns the `Option` output slot of a successful `Create*` call into the object.
fn created<T>(
    what: &'static str,
    result: windows::core::Result<()>,
    out: Option<T>,
) -> Result<T, String> {
    result.map_err(hr(what))?;
    out.ok_or_else(|| format!("{what}: kein Objekt"))
}

fn dynamic_buffer(
    device: &ID3D11Device,
    bytes: u32,
    bind: D3D11_BIND_FLAG,
) -> Result<ID3D11Buffer, String> {
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: bytes,
        Usage: D3D11_USAGE_DYNAMIC,
        BindFlags: bind.0 as u32,
        CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
        MiscFlags: 0,
        StructureByteStride: 0,
    };
    let mut buffer = None;
    // SAFETY: The descriptor is live during the call; no initial data; one owned reference.
    let result = unsafe { device.CreateBuffer(&desc, None, Some(&mut buffer)) };
    created("CreateBuffer(Menü)", result, buffer)
}

impl MeshRenderer {
    pub fn new(device: ID3D11Device) -> Result<Self, String> {
        // SAFETY: The device is live; GetImmediateContext returns one owned reference.
        let context: ID3D11DeviceContext1 = unsafe { device.GetImmediateContext() }
            .and_then(|c| c.cast())
            .map_err(hr("QueryInterface(ID3D11DeviceContext1), D3D11.1 fehlt"))?;
        let state = create_context_state(&device)?;

        let vs_blob = compile_shader(SHADER, c"vs_main", c"vs_4_0")?;
        let ps_blob = compile_shader(SHADER, c"ps_main", c"ps_4_0")?;
        let (vs_bytes, ps_bytes) = (blob_bytes(&vs_blob), blob_bytes(&ps_blob));
        let mut vs = None;
        // SAFETY: The compiled blob owns the shader bytes for the call; the device stays live and
        // the output slot receives one owned reference.
        let result = unsafe { device.CreateVertexShader(vs_bytes, None, Some(&mut vs)) };
        let vs = created("CreateVertexShader(Menü)", result, vs)?;
        let mut ps = None;
        // SAFETY: as above, for the pixel shader blob.
        let result = unsafe { device.CreatePixelShader(ps_bytes, None, Some(&mut ps)) };
        let ps = created("CreatePixelShader(Menü)", result, ps)?;

        let element = |name: &'static CStr, format, offset| D3D11_INPUT_ELEMENT_DESC {
            SemanticName: PCSTR(name.as_ptr().cast()),
            SemanticIndex: 0,
            Format: format,
            InputSlot: 0,
            AlignedByteOffset: offset,
            InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
            InstanceDataStepRate: 0,
        };
        let elements = [
            element(c"POSITION", DXGI_FORMAT_R32G32_FLOAT, 0),
            element(c"TEXCOORD", DXGI_FORMAT_R32G32_FLOAT, 8),
            element(c"COLOR", DXGI_FORMAT_R8G8B8A8_UNORM, 16),
        ];
        let mut layout = None;
        // SAFETY: elements (with 'static semantic names) and the vertex shader bytecode stay live
        // for the call. Success stores one owned layout reference.
        let result = unsafe { device.CreateInputLayout(&elements, vs_bytes, Some(&mut layout)) };
        let layout = created("CreateInputLayout(Menü)", result, layout)?;

        // Premultiplied alpha (menu convention); all eight entries valid (E-DX-09).
        let target_blend = D3D11_RENDER_TARGET_BLEND_DESC {
            BlendEnable: true.into(),
            SrcBlend: D3D11_BLEND_ONE,
            DestBlend: D3D11_BLEND_INV_SRC_ALPHA,
            BlendOp: D3D11_BLEND_OP_ADD,
            SrcBlendAlpha: D3D11_BLEND_INV_DEST_ALPHA,
            DestBlendAlpha: D3D11_BLEND_ONE,
            BlendOpAlpha: D3D11_BLEND_OP_ADD,
            RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8,
        };
        let blend_desc = D3D11_BLEND_DESC {
            RenderTarget: [target_blend; 8],
            ..Default::default()
        };
        let mut blend = None;
        // SAFETY: The descriptor is live during the call; one owned reference is returned.
        let result = unsafe { device.CreateBlendState(&blend_desc, Some(&mut blend)) };
        let blend = created("CreateBlendState(Menü)", result, blend)?;

        let rasterizer_desc = D3D11_RASTERIZER_DESC {
            FillMode: D3D11_FILL_SOLID,
            CullMode: D3D11_CULL_NONE,
            DepthClipEnable: true.into(),
            ScissorEnable: true.into(),
            ..Default::default()
        };
        let mut rasterizer = None;
        // SAFETY: The descriptor is live during the call; one owned reference is returned.
        let result =
            unsafe { device.CreateRasterizerState(&rasterizer_desc, Some(&mut rasterizer)) };
        let rasterizer = created("CreateRasterizerState(Menü)", result, rasterizer)?;

        let keep = D3D11_DEPTH_STENCILOP_DESC {
            StencilFailOp: D3D11_STENCIL_OP_KEEP,
            StencilDepthFailOp: D3D11_STENCIL_OP_KEEP,
            StencilPassOp: D3D11_STENCIL_OP_KEEP,
            StencilFunc: D3D11_COMPARISON_ALWAYS,
        };
        let depth_desc = D3D11_DEPTH_STENCIL_DESC {
            DepthEnable: false.into(),
            DepthWriteMask: D3D11_DEPTH_WRITE_MASK_ZERO,
            DepthFunc: D3D11_COMPARISON_ALWAYS,
            StencilEnable: false.into(),
            StencilReadMask: 0xff,
            StencilWriteMask: 0xff,
            FrontFace: keep,
            BackFace: keep,
        };
        let mut depth = None;
        // SAFETY: The descriptor is live during the call; one owned reference is returned.
        let result = unsafe { device.CreateDepthStencilState(&depth_desc, Some(&mut depth)) };
        let depth = created("CreateDepthStencilState(Menü)", result, depth)?;

        let sampler = |filter| {
            let desc = D3D11_SAMPLER_DESC {
                Filter: filter,
                AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
                MaxAnisotropy: 1,
                ComparisonFunc: D3D11_COMPARISON_ALWAYS,
                MaxLOD: f32::MAX,
                ..Default::default()
            };
            let mut sampler = None;
            // SAFETY: The descriptor is live during the call; one owned reference is returned.
            let result = unsafe { device.CreateSamplerState(&desc, Some(&mut sampler)) };
            created("CreateSamplerState(Menü)", result, sampler)
        };
        let samplers = [
            sampler(D3D11_FILTER_MIN_MAG_MIP_LINEAR)?,
            sampler(D3D11_FILTER_MIN_MAG_MIP_POINT)?,
        ];

        let constants = dynamic_buffer(&device, 16, D3D11_BIND_CONSTANT_BUFFER)
            .map_err(|e| e.replace("(Menü)", "(Menü-Konstanten)"))?;

        Ok(Self {
            device,
            context,
            state,
            vs,
            ps,
            layout,
            blend,
            rasterizer,
            depth,
            samplers,
            constants,
            vertices: None,
            indices: None,
            textures: HashMap::new(),
        })
    }

    #[cfg(test)]
    pub fn has_texture(&self, id: TextureId) -> bool {
        self.textures.contains_key(&id)
    }

    /// Grows `slot` to at least `count` elements of `stride` bytes.
    fn ensure(
        device: &ID3D11Device,
        slot: &mut Option<Dynamic>,
        count: usize,
        stride: usize,
        bind: D3D11_BIND_FLAG,
    ) -> Result<(), String> {
        if slot.as_ref().is_some_and(|b| b.capacity >= count) {
            return Ok(());
        }
        let capacity = count.next_power_of_two().max(1024);
        let bytes =
            u32::try_from(capacity * stride).map_err(|_| "Menü-Puffer zu groß".to_string())?;
        let buffer = dynamic_buffer(device, bytes, bind)?;
        *slot = Some(Dynamic { buffer, capacity });
        Ok(())
    }

    /// Maps `buffer` with WRITE_DISCARD, lets `fill` write at most its capacity, unmaps.
    fn write(&self, buffer: &ID3D11Buffer, fill: impl FnOnce(*mut u8)) -> Result<(), String> {
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        // SAFETY: The dynamic buffer belongs to this device; Map writes the mapping to the live
        // output struct and the immediate context is used only on this (Present) thread.
        unsafe {
            self.context
                .Map(buffer, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped))
        }
        .map_err(hr("Map(Menü)"))?;
        fill(mapped.pData.cast());
        // SAFETY: Unmaps the subresource mapped above on the same context.
        unsafe { self.context.Unmap(buffer, 0) };
        Ok(())
    }
}

fn is_srgb(format: DXGI_FORMAT) -> bool {
    matches!(
        format,
        DXGI_FORMAT_R8G8B8A8_UNORM_SRGB | DXGI_FORMAT_B8G8R8A8_UNORM_SRGB
    )
}

impl MenuRenderer for MeshRenderer {
    type Target<'t> = &'t ID3D11Texture2D;

    fn apply(&mut self, delta: &TextureDelta) -> Result<(), String> {
        match delta {
            TextureDelta::Create {
                id,
                size,
                rgba,
                nearest,
            } => {
                let [width, height] = *size;
                if width == 0 || height == 0 || rgba.len() != (width * height * 4) as usize {
                    return Err(format!("Menü: ungültige Textur {id:?}"));
                }
                let desc = texture_desc(
                    DXGI_FORMAT_R8G8B8A8_UNORM,
                    *size,
                    D3D11_USAGE_DEFAULT,
                    D3D11_BIND_SHADER_RESOURCE,
                    D3D11_CPU_ACCESS_FLAG(0),
                );
                let initial = D3D11_SUBRESOURCE_DATA {
                    pSysMem: rgba.as_ptr().cast(),
                    SysMemPitch: width * 4,
                    SysMemSlicePitch: 0,
                };
                // SAFETY: one mip/array slice; `rgba` holds width*height tightly packed RGBA8
                // texels (checked above) and stays alive for the synchronous creation.
                let texture = unsafe { create_texture(&self.device, &desc, Some(&initial)) }
                    .map_err(hr("CreateTexture2D(Menü)"))?;
                let mut view = None;
                // SAFETY: The texture is live; the output slot receives one owned reference.
                let result = unsafe {
                    self.device
                        .CreateShaderResourceView(&texture, None, Some(&mut view))
                };
                let view = created("CreateShaderResourceView(Menü)", result, view)?;
                self.textures.insert(
                    *id,
                    Texture {
                        texture,
                        view,
                        size: *size,
                        nearest: *nearest,
                    },
                );
            }
            TextureDelta::Update {
                id,
                pos: [x, y],
                size: [width, height],
                rgba,
            } => {
                let texture = self
                    .textures
                    .get(id)
                    .ok_or_else(|| format!("Menü: Teil-Update für unbekannte Textur {id:?}"))?;
                if x + width > texture.size[0]
                    || y + height > texture.size[1]
                    || rgba.len() != (width * height * 4) as usize
                {
                    return Err(format!("Menü: ungültiges Teil-Update {id:?}"));
                }
                let region = D3D11_BOX {
                    left: *x,
                    top: *y,
                    front: 0,
                    right: x + width,
                    bottom: y + height,
                    back: 1,
                };
                // SAFETY: The box lies inside the owned texture and `rgba` covers it with this
                // pitch (checked above); UpdateSubresource copies synchronously.
                unsafe {
                    self.context.UpdateSubresource(
                        &texture.texture,
                        0,
                        Some(&region),
                        rgba.as_ptr().cast::<c_void>(),
                        width * 4,
                        0,
                    )
                };
            }
            TextureDelta::Free(id) => {
                self.textures.remove(id);
            }
        }
        Ok(())
    }

    fn draw(
        &mut self,
        target: &ID3D11Texture2D,
        list: &DrawList,
        size_px: [u32; 2],
    ) -> Result<(), String> {
        let [width, height] = size_px;
        if width == 0 || height == 0 || list.indices.is_empty() {
            return Ok(());
        }
        if list
            .indices
            .iter()
            .any(|&i| i as usize >= list.vertices.len())
        {
            return Err("Menü: Index außerhalb der Vertices".into());
        }
        Self::ensure(
            &self.device,
            &mut self.vertices,
            list.vertices.len(),
            VERTEX_SIZE,
            D3D11_BIND_VERTEX_BUFFER,
        )?;
        Self::ensure(
            &self.device,
            &mut self.indices,
            list.indices.len(),
            4,
            D3D11_BIND_INDEX_BUFFER,
        )?;
        let vb = self.vertices.as_ref().unwrap().buffer.clone();
        let ib = self.indices.as_ref().unwrap().buffer.clone();
        self.write(&vb, |dst| {
            // SAFETY: `ensure` sized the buffer for all vertices; Vertex is repr(C), 20 bytes,
            // matching the input layout. Source and mapped memory do not overlap.
            unsafe {
                core::ptr::copy_nonoverlapping(
                    list.vertices.as_ptr().cast::<u8>(),
                    dst,
                    list.vertices.len() * VERTEX_SIZE,
                )
            };
        })?;
        self.write(&ib, |dst| {
            // SAFETY: `ensure` sized the buffer for all u32 indices; no overlap.
            unsafe {
                core::ptr::copy_nonoverlapping(
                    list.indices.as_ptr().cast::<u8>(),
                    dst,
                    list.indices.len() * 4,
                )
            };
        })?;
        let mut target_desc = D3D11_TEXTURE2D_DESC::default();
        // SAFETY: The texture is live and the descriptor is a writable output.
        unsafe { target.GetDesc(&mut target_desc) };
        let srgb = is_srgb(target_desc.Format);
        let screen = [
            width as f32,
            height as f32,
            if srgb { 1.0 } else { 0.0 },
            0.0,
        ];
        self.write(&self.constants, |dst| {
            // SAFETY: the constant buffer holds exactly 16 bytes = four f32.
            unsafe { core::ptr::copy_nonoverlapping(screen.as_ptr().cast::<u8>(), dst, 16) };
        })?;

        // Per frame: holding the backbuffer beyond the frame makes ResizeBuffers fail.
        let rtv =
            render_target_view(&self.device, target).map_err(hr("CreateRenderTargetView(Menü)"))?;
        let context = &self.context;
        let _restore = ContextState::enter(context, &self.state);
        let stride = VERTEX_SIZE as u32;
        let offset = 0u32;
        let vertex_buffers = [Some(vb)];
        let constants = [Some(self.constants.clone())];
        // SAFETY: All bound objects are owned by this renderer and live; arrays have the declared
        // counts. The immediate context is used on the Present thread while ContextState
        // guarantees the game's state is swapped back afterwards.
        unsafe {
            context.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            context.IASetInputLayout(&self.layout);
            context.IASetVertexBuffers(
                0,
                1,
                Some(vertex_buffers.as_ptr()),
                Some(&stride),
                Some(&offset),
            );
            context.IASetIndexBuffer(&ib, DXGI_FORMAT_R32_UINT, 0);
            context.VSSetShader(&self.vs, None);
            context.VSSetConstantBuffers(0, Some(&constants));
            context.PSSetShader(&self.ps, None);
            context.PSSetConstantBuffers(0, Some(&constants));
            context.OMSetBlendState(&self.blend, Some(&[0.0; 4]), u32::MAX);
            context.OMSetDepthStencilState(&self.depth, 0);
            context.RSSetState(&self.rasterizer);
            let viewport = D3D11_VIEWPORT {
                Width: width as f32,
                Height: height as f32,
                MaxDepth: 1.0,
                ..Default::default()
            };
            context.RSSetViewports(Some(&[viewport]));
            context.OMSetRenderTargets(Some(&[Some(rtv)]), None);
        }
        for cmd in &list.commands {
            let (Some([left, top, right, bottom]), Some(texture)) =
                (cmd.clip.scissor(size_px), self.textures.get(&cmd.texture))
            else {
                continue;
            };
            let end = cmd.indices.end.min(list.indices.len() as u32);
            if cmd.indices.start >= end {
                continue;
            }
            let rect = RECT {
                left: left as i32,
                top: top as i32,
                right: right as i32,
                bottom: bottom as i32,
            };
            let sampler = &self.samplers[usize::from(texture.nearest)];
            // SAFETY: as above; the rect, sampler and view are live for each call, and the
            // index range lies within the uploaded index buffer.
            unsafe {
                context.RSSetScissorRects(Some(&[rect]));
                context.PSSetSamplers(0, Some(&[Some(sampler.clone())]));
                context.PSSetShaderResources(0, Some(&[Some(texture.view.clone())]));
                context.DrawIndexed(end - cmd.indices.start, cmd.indices.start, 0);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::{FONT_TEXTURE, Rect, rgba};

    struct Target {
        device: ID3D11Device,
        context: ID3D11DeviceContext,
        texture: ID3D11Texture2D,
        rtv: ID3D11RenderTargetView,
        size: [u32; 2],
        format: DXGI_FORMAT,
    }

    impl Target {
        fn new(width: u32, height: u32, format: DXGI_FORMAT) -> Self {
            let device = warp_device().unwrap();
            // SAFETY: The device is live; one owned context reference is returned.
            let context = unsafe { device.GetImmediateContext() }.unwrap();
            let desc = texture_desc(
                format,
                [width, height],
                D3D11_USAGE_DEFAULT,
                D3D11_BIND_RENDER_TARGET,
                D3D11_CPU_ACCESS_FLAG(0),
            );
            // SAFETY: no initial-data pointers are supplied.
            let texture = unsafe { create_texture(&device, &desc, None) }.unwrap();
            let rtv = render_target_view(&device, &texture).unwrap();
            // SAFETY: The test owns context and RTV; the color array is live for the clear, and
            // the one-element RTV array is valid ("game" state that must survive drawing).
            unsafe {
                context.ClearRenderTargetView(&rtv, &[0.0, 0.0, 1.0, 1.0]);
                context.OMSetRenderTargets(Some(&[Some(rtv.clone())]), None);
            }
            Self {
                device,
                context,
                texture,
                rtv,
                size: [width, height],
                format,
            }
        }

        fn pixels(&self) -> Vec<[u8; 4]> {
            let [width, height] = self.size;
            let desc = texture_desc(
                self.format,
                self.size,
                D3D11_USAGE_STAGING,
                D3D11_BIND_FLAG(0),
                D3D11_CPU_ACCESS_READ,
            );
            // SAFETY: no initial-data pointers are supplied.
            let staging = unsafe { create_texture(&self.device, &desc, None) }.unwrap();
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            let mut out = Vec::with_capacity((width * height) as usize);
            // SAFETY: Matching textures on the same context; Map blocks until the copy is done and
            // every read stays within RowPitch * height. Pixels are copied before Unmap.
            unsafe {
                self.context.CopyResource(&staging, &self.texture);
                self.context
                    .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                    .unwrap();
                for y in 0..height {
                    for x in 0..width {
                        let p = mapped
                            .pData
                            .cast::<u8>()
                            .add((y * mapped.RowPitch + x * 4) as usize)
                            .cast::<[u8; 4]>();
                        out.push(*p);
                    }
                }
                self.context.Unmap(&staging, 0);
            }
            out
        }

        fn bound_rtv(&self) -> Option<*mut c_void> {
            let mut bound = [None];
            // SAFETY: writes one owned RTV reference (or None) into the live array.
            unsafe { self.context.OMGetRenderTargets(Some(&mut bound), None) };
            let [bound] = bound;
            bound.map(|v| v.as_raw())
        }
    }

    fn white() -> TextureDelta {
        TextureDelta::Create {
            id: FONT_TEXTURE,
            size: [1, 1],
            rgba: vec![255; 4],
            nearest: true,
        }
    }

    fn solid(list: &mut DrawList, rect: Rect, color: u32) {
        let uv = Rect::new(0.0, 0.0, 1.0, 1.0);
        list.quad_uv(rect, uv, color, FONT_TEXTURE);
    }

    fn near(a: [u8; 4], b: [u8; 4]) -> bool {
        a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 2)
    }

    /// Filled rects, premultiplied blending, scissor per command, texture free and the
    /// game's render target restored afterwards.
    #[test]
    fn draws_list_with_scissor_and_restores_state() {
        let target = Target::new(64, 64, DXGI_FORMAT_R8G8B8A8_UNORM);
        let mut renderer = MeshRenderer::new(target.device.clone()).unwrap();
        let mut list = DrawList::default();
        solid(
            &mut list,
            Rect::new(0.0, 0.0, 32.0, 32.0),
            rgba(255, 0, 0, 255),
        );
        // Half transparent red, blended over blue.
        solid(
            &mut list,
            Rect::new(32.0, 0.0, 32.0, 32.0),
            rgba(255, 0, 0, 128),
        );
        // Green bottom row, clipped to x < 16.
        list.push_clip(Rect::new(0.0, 0.0, 16.0, 64.0));
        solid(
            &mut list,
            Rect::new(0.0, 40.0, 64.0, 24.0),
            rgba(0, 255, 0, 255),
        );
        list.pop_clip();
        renderer
            .paint(&target.texture, &[white()], &list, target.size)
            .unwrap();

        assert_eq!(
            target.bound_rtv(),
            Some(target.rtv.as_raw()),
            "game RTV restored"
        );
        let pixels = target.pixels();
        let at = |x: u32, y: u32| pixels[(y * 64 + x) as usize];
        assert_eq!(at(8, 8), [255, 0, 0, 255], "opaque rect");
        assert!(
            near(at(48, 8), [128, 0, 127, 255]),
            "blend: {:?}",
            at(48, 8)
        );
        assert_eq!(at(8, 50), [0, 255, 0, 255], "inside scissor");
        assert_eq!(at(40, 50), [0, 0, 255, 255], "scissored away");
        assert_eq!(at(40, 36), [0, 0, 255, 255], "untouched");

        renderer
            .paint(
                &target.texture,
                &[TextureDelta::Free(FONT_TEXTURE)],
                &DrawList::default(),
                target.size,
            )
            .unwrap();
        assert!(!renderer.has_texture(FONT_TEXTURE));
    }

    /// A real headless kit frame (font atlas, text, shapes): a toast card.
    #[test]
    fn draws_headless_kit_frame() {
        let target = Target::new(640, 480, DXGI_FORMAT_R8G8B8A8_UNORM);
        let mut renderer = MeshRenderer::new(target.device.clone()).unwrap();
        let mut kit = crate::kit::Kit::new(crate::theme::Theme::default()).unwrap();
        kit.toast("Saved", crate::kit::ToastKind::Success);
        let mut textures = Vec::new();
        for frame in 0..3 {
            let input = crate::kit::KitInput {
                dt: 0.1,
                ..Default::default()
            };
            kit.begin(input, target.size);
            let (list, t) = kit.end();
            textures.extend(t);
            if frame == 2 {
                renderer
                    .paint(&target.texture, &textures, list, target.size)
                    .unwrap();
            }
        }
        assert!(renderer.has_texture(FONT_TEXTURE), "font atlas");
        let pixels = target.pixels();
        assert_ne!(
            pixels[410 * 640 + 500],
            [0, 0, 255, 255],
            "toast card covers"
        );
        assert_eq!(pixels[10 * 640 + 10], [0, 0, 255, 255], "outside the toast");
        let distinct: std::collections::HashSet<_> = pixels.iter().collect();
        assert!(distinct.len() > 8, "text and widgets drawn");
        assert_eq!(target.bound_rtv(), Some(target.rtv.as_raw()));
    }

    /// On an `_SRGB` backbuffer the stored bytes equal the gamma-space colour (the shader
    /// linearises, the hardware encodes back), so the menu looks the same as on UNORM.
    #[test]
    fn srgb_target_keeps_gamma_colors() {
        let target = Target::new(16, 16, DXGI_FORMAT_R8G8B8A8_UNORM_SRGB);
        let mut renderer = MeshRenderer::new(target.device.clone()).unwrap();
        let mut list = DrawList::default();
        solid(
            &mut list,
            Rect::new(0.0, 0.0, 16.0, 16.0),
            rgba(128, 64, 200, 255),
        );
        renderer
            .paint(&target.texture, &[white()], &list, target.size)
            .unwrap();
        let pixel = target.pixels()[8 * 16 + 8];
        assert!(near(pixel, [128, 64, 200, 255]), "sRGB: {pixel:?}");
    }

    #[test]
    fn shader_errors_carry_the_compiler_message() {
        let error = compile_shader("float4 broken(", c"broken", c"ps_4_0").unwrap_err();
        assert!(error.starts_with("D3DCompile(broken)"), "{error}");
    }
}
