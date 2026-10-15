//! Metal backend of [`MenuRenderer`] ([`MetalRenderer`]) and an offscreen render target
//! with readback ([`Offscreen`]) for previews and tests.
//!
//! Same conventions and shaders as the D3D11 backend: premultiplied alpha (`ONE,
//! ONE_MINUS_SRC_ALPHA`, alpha `ONE_MINUS_DST_ALPHA, ONE`), RGBA8 UNORM textures sampled
//! linear or nearest, one scissor rectangle per command, and on an `_sRGB` target the
//! fragment shader converts the gamma-space result to linear before writing. Shaders are
//! compiled from source at startup. Vertices and indices go into fresh shared buffers each
//! frame, which the command buffer keeps alive until the GPU is done with them.
//!
//! Glass: when the list has a `backdrop` region, the target under it is downsampled to ¼
//! and Gaussian-blurred in three small passes before the menu draws (the target must be
//! shader-readable: a `CAMetalLayer` with `framebufferOnly` off, or [`Offscreen`]); shapes
//! with `BACKDROP_UV` sample the result, shapes with `gradient_uv` the accent gradient row
//! (see [`crate::draw`]).

use core::ffi::c_void;
use core::ptr::NonNull;
use std::collections::HashMap;

use objc2::Message;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_foundation::NSString;
use objc2_metal::{
    MTLBlendFactor, MTLBlitCommandEncoder, MTLBuffer, MTLClearColor, MTLCommandBuffer,
    MTLCommandEncoder, MTLCommandQueue, MTLCreateSystemDefaultDevice, MTLDevice, MTLFunction,
    MTLIndexType, MTLLibrary, MTLLoadAction, MTLOrigin, MTLPixelFormat, MTLPrimitiveType,
    MTLRegion, MTLRenderCommandEncoder, MTLRenderPassDescriptor, MTLRenderPipelineDescriptor,
    MTLRenderPipelineState, MTLResourceOptions, MTLSamplerAddressMode, MTLSamplerDescriptor,
    MTLSamplerMinMagFilter, MTLSamplerState, MTLScissorRect, MTLSize, MTLStorageMode,
    MTLStoreAction, MTLTexture, MTLTextureDescriptor, MTLTextureUsage,
};

use crate::draw::{DrawList, GRADIENT_TEXTURE, Rect, TextureDelta, TextureId, Vertex};
use crate::render::MenuRenderer;

pub type Device = ProtocolObject<dyn MTLDevice>;
pub type Texture = ProtocolObject<dyn MTLTexture>;
pub type CommandBuffer = ProtocolObject<dyn MTLCommandBuffer>;
pub type CommandQueue = ProtocolObject<dyn MTLCommandQueue>;

const SHADER: &str = r#"
#include <metal_stdlib>
using namespace metal;

struct Screen { float2 screen_points; float srgb_target; float pad; float2 backdrop_scale; float2 pad2; };
struct VsIn { packed_float2 pos; packed_float2 uv; uint color; };
struct PsIn { float4 pos [[position]]; float2 uv; float4 color; };

vertex PsIn vs_main(const device VsIn* vertices [[buffer(0)]],
                    constant Screen& screen [[buffer(1)]],
                    uint id [[vertex_id]]) {
    VsIn v = vertices[id];
    PsIn o;
    o.pos = float4(v.pos.x / screen.screen_points.x * 2.0 - 1.0,
                   1.0 - v.pos.y / screen.screen_points.y * 2.0, 0.0, 1.0);
    o.uv = v.uv;
    o.color = unpack_unorm4x8_to_float(v.color);
    return o;
}

float3 to_linear(float3 c) {
    return mix(pow((c + 0.055) / 1.055, 2.4), c / 12.92, step(c, float3(0.04045)));
}

// uv.y < -1.5: accent gradient at u; -1.5 <= uv.y < -0.5: blurred backdrop at the pixel.
fragment float4 ps_main(PsIn i [[stage_in]],
                        constant Screen& screen [[buffer(0)]],
                        texture2d<float> tex [[texture(0)]],
                        texture2d<float> backdrop [[texture(1)]],
                        texture2d<float> lut [[texture(2)]],
                        sampler samp [[sampler(0)]],
                        sampler lin [[sampler(1)]]) {
    float4 a = tex.sample(samp, i.uv);
    float4 b = backdrop.sample(lin, i.pos.xy * screen.backdrop_scale, level(0));
    float u = saturate(i.uv.x) * (255.0 / 256.0) + (0.5 / 256.0);
    float4 g = lut.sample(lin, float2(u, 0.5), level(0));
    float4 t = i.uv.y < -1.5 ? g : (i.uv.y < -0.5 ? b : a);
    float4 c = i.color * t;
    if (screen.srgb_target > 0.5 && c.a > 0.0) {
        c.rgb = to_linear(c.rgb / c.a) * c.a;
    }
    return c;
}
"#;

/// Backdrop blur: ¼ downsample with vibrancy → separable 13-tap Gaussian (7 bilinear
/// fetches per direction) at ¼ resolution, all scissored to the glass region. The same
/// passes as the D3D11 backend.
const BLUR_SHADER: &str = r#"
#include <metal_stdlib>
using namespace metal;

struct Blur { float2 src_texel; float2 dir; float2 uv_scale; float2 pad; };
struct V { float4 pos [[position]]; float2 uv; };

vertex V vs_full(uint id [[vertex_id]]) {
    V o;
    float2 p = float2((id << 1) & 2, id & 2);
    o.pos = float4(p.x * 2.0 - 1.0, 1.0 - p.y * 2.0, 0.0, 1.0);
    o.uv = p;
    return o;
}

// 4 bilinear taps = the 4x4 block under the quarter pixel. Then a little more colour
// and a soft luminance cap, so glass stays dark enough for white text on bright maps.
fragment float4 ps_down(V i [[stage_in]], constant Blur& b [[buffer(0)]],
                        texture2d<float> src [[texture(0)]], sampler lin [[sampler(0)]]) {
    float2 uv = i.uv * b.uv_scale;
    float3 c = src.sample(lin, uv + b.src_texel * float2(-1.0, -1.0), level(0)).rgb
             + src.sample(lin, uv + b.src_texel * float2( 1.0, -1.0), level(0)).rgb
             + src.sample(lin, uv + b.src_texel * float2(-1.0,  1.0), level(0)).rgb
             + src.sample(lin, uv + b.src_texel * float2( 1.0,  1.0), level(0)).rgb;
    c *= 0.25;
    float l = dot(c, float3(0.2126, 0.7152, 0.0722));
    c = max(mix(float3(l), c, 1.2), 0.0);
    c *= 1.0 / (1.0 + 1.5 * l);
    return float4(c, 1.0);
}

constant float W[4] = { 0.19648255, 0.29690696, 0.09447040, 0.01038136 };
constant float O[4] = { 0.0, 1.41176471, 3.29411765, 5.17647059 };

fragment float4 ps_blur(V i [[stage_in]], constant Blur& b [[buffer(0)]],
                        texture2d<float> src [[texture(0)]], sampler lin [[sampler(0)]]) {
    float3 c = src.sample(lin, i.uv, level(0)).rgb * W[0];
    for (int k = 1; k < 4; k++) {
        float2 o = b.dir * O[k];
        c += src.sample(lin, i.uv + o, level(0)).rgb * W[k];
        c += src.sample(lin, i.uv - o, level(0)).rgb * W[k];
    }
    return float4(c, 1.0);
}
"#;

/// Blur radius multiplier of the Gaussian offsets (quarter-resolution texels).
const BLUR_SPREAD: f32 = 1.3;
/// Extra pixels read around the glass region so the blur never reads stale texels.
const BLUR_MARGIN: f32 = 40.0;
/// What glass shows when the target cannot be blurred (not shader-readable, float formats).
const BACKDROP_FALLBACK: [u8; 4] = [20, 22, 30, 255];

const VERTEX_SIZE: usize = size_of::<Vertex>();
const _: () = assert!(
    VERTEX_SIZE == 20,
    "menu Vertex: pos, uv (f32x2), color (RGBA8) = the shader's packed VsIn"
);

/// The system default Metal device.
pub fn default_device() -> Result<Retained<Device>, String> {
    MTLCreateSystemDefaultDevice().ok_or_else(|| "Metal: no device".into())
}

fn is_srgb(format: MTLPixelFormat) -> bool {
    format == MTLPixelFormat::RGBA8Unorm_sRGB || format == MTLPixelFormat::BGRA8Unorm_sRGB
}

/// Where [`MetalRenderer`] draws: a colour texture (a drawable's or an offscreen one) and the
/// command buffer that receives the render pass.
pub struct MetalTarget<'t> {
    pub commands: &'t CommandBuffer,
    pub texture: &'t Texture,
    /// Premultiplied RGBA to clear to first; `None` keeps the texture's contents.
    pub clear: Option<[f64; 4]>,
}

struct Atlas {
    texture: Retained<Texture>,
    size: [u32; 2],
    nearest: bool,
}

type Pipeline = Retained<ProtocolObject<dyn MTLRenderPipelineState>>;
type Sampler = Retained<ProtocolObject<dyn MTLSamplerState>>;
/// The blurred quarter-resolution backdrop and the UV scale that maps pixels onto it.
type Blurred = (Retained<Texture>, [f32; 2]);

struct BlurPipeline {
    down: Pipeline,
    blur: Pipeline,
}

/// Quarter-resolution ping-pong targets for one target size.
struct Backdrop {
    size: [u32; 2],
    qsize: [u32; 2],
    quarter: [Retained<Texture>; 2],
}

pub struct MetalRenderer {
    device: Retained<Device>,
    library: Retained<ProtocolObject<dyn MTLLibrary>>,
    /// One pipeline per target pixel format.
    pipelines: HashMap<usize, Pipeline>,
    /// [linear, nearest]
    samplers: [Sampler; 2],
    textures: HashMap<TextureId, Atlas>,
    blur: BlurPipeline,
    backdrop: Option<Backdrop>,
    /// 1×1 stand-ins: [backdrop when it cannot be blurred, white gradient row].
    fallback: [Retained<Texture>; 2],
    /// The last frame's command buffer: texture updates wait for it, since the GPU may
    /// still read the texels they replace.
    in_flight: Option<Retained<CommandBuffer>>,
}

fn function(
    library: &ProtocolObject<dyn MTLLibrary>,
    name: &str,
) -> Result<Retained<ProtocolObject<dyn MTLFunction>>, String> {
    library
        .newFunctionWithName(&NSString::from_str(name))
        .ok_or_else(|| format!("Metal: shader function {name}"))
}

/// A pipeline drawing into `format`, premultiplied-alpha blending or none (`blend` off:
/// the pass overwrites its target).
fn pipeline(
    device: &Device,
    library: &ProtocolObject<dyn MTLLibrary>,
    [vs, fs]: [&str; 2],
    format: MTLPixelFormat,
    blend: bool,
) -> Result<Pipeline, String> {
    let desc = MTLRenderPipelineDescriptor::new();
    desc.setVertexFunction(Some(&*function(library, vs)?));
    desc.setFragmentFunction(Some(&*function(library, fs)?));
    // SAFETY: Index 0 is within the descriptor's colour attachment array.
    let color = unsafe { desc.colorAttachments().objectAtIndexedSubscript(0) };
    color.setPixelFormat(format);
    color.setBlendingEnabled(blend);
    color.setSourceRGBBlendFactor(MTLBlendFactor::One);
    color.setDestinationRGBBlendFactor(MTLBlendFactor::OneMinusSourceAlpha);
    color.setSourceAlphaBlendFactor(MTLBlendFactor::OneMinusDestinationAlpha);
    color.setDestinationAlphaBlendFactor(MTLBlendFactor::One);
    device
        .newRenderPipelineStateWithDescriptor_error(&desc)
        .map_err(|e| format!("Metal pipeline {fs}: {}", e.localizedDescription()))
}

fn library(
    device: &Device,
    source: &str,
) -> Result<Retained<ProtocolObject<dyn MTLLibrary>>, String> {
    device
        .newLibraryWithSource_options_error(&NSString::from_str(source), None)
        .map_err(|e| format!("Metal shader: {}", e.localizedDescription()))
}

fn texture_2d(
    device: &Device,
    format: MTLPixelFormat,
    size: [u32; 2],
    usage: MTLTextureUsage,
    storage: MTLStorageMode,
) -> Result<Retained<Texture>, String> {
    // SAFETY: A plain single-mip 2D descriptor with a valid format and non-zero size.
    let desc = unsafe {
        MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
            format,
            size[0].max(1) as usize,
            size[1].max(1) as usize,
            false,
        )
    };
    desc.setUsage(usage);
    desc.setStorageMode(storage);
    device
        .newTextureWithDescriptor(&desc)
        .ok_or_else(|| format!("Metal: {}×{} texture", size[0], size[1]))
}

/// An RGBA8 texture for shader reads filled with `rgba`.
fn texels(device: &Device, size: [u32; 2], rgba: &[u8]) -> Result<Retained<Texture>, String> {
    let texture = texture_2d(
        device,
        MTLPixelFormat::RGBA8Unorm,
        size,
        MTLTextureUsage::ShaderRead,
        MTLStorageMode::Shared,
    )?;
    write_texels(&texture, [0, 0], size, rgba);
    Ok(texture)
}

fn write_texels(texture: &Texture, pos: [u32; 2], size: [u32; 2], rgba: &[u8]) {
    debug_assert!(rgba.len() >= (size[0] * size[1] * 4) as usize);
    let region = MTLRegion {
        origin: MTLOrigin {
            x: pos[0] as usize,
            y: pos[1] as usize,
            z: 0,
        },
        size: MTLSize {
            width: size[0] as usize,
            height: size[1] as usize,
            depth: 1,
        },
    };
    let Some(bytes) = NonNull::new(rgba.as_ptr().cast_mut().cast::<c_void>()) else {
        return;
    };
    // SAFETY: The callers checked that the region lies inside the texture and that `rgba`
    // holds width * height tightly packed RGBA8 texels; the copy is synchronous.
    unsafe {
        texture.replaceRegion_mipmapLevel_withBytes_bytesPerRow(
            region,
            0,
            bytes,
            size[0] as usize * 4,
        )
    };
}

/// `bytes` as a constant-data pointer for `set*Bytes`.
fn bytes_ptr<T>(bytes: &T) -> NonNull<c_void> {
    NonNull::from(bytes).cast()
}

impl MetalRenderer {
    pub fn new(device: Retained<Device>) -> Result<Self, String> {
        let library = library(&device, SHADER)?;
        let blur_library = self::library(&device, BLUR_SHADER)?;
        let blur_format = MTLPixelFormat::RGBA8Unorm;
        let blur = BlurPipeline {
            down: pipeline(
                &device,
                &blur_library,
                ["vs_full", "ps_down"],
                blur_format,
                false,
            )?,
            blur: pipeline(
                &device,
                &blur_library,
                ["vs_full", "ps_blur"],
                blur_format,
                false,
            )?,
        };
        let sampler = |filter| {
            let desc = MTLSamplerDescriptor::new();
            desc.setMinFilter(filter);
            desc.setMagFilter(filter);
            desc.setSAddressMode(MTLSamplerAddressMode::ClampToEdge);
            desc.setTAddressMode(MTLSamplerAddressMode::ClampToEdge);
            device
                .newSamplerStateWithDescriptor(&desc)
                .ok_or_else(|| "Metal: sampler".to_string())
        };
        let samplers = [
            sampler(MTLSamplerMinMagFilter::Linear)?,
            sampler(MTLSamplerMinMagFilter::Nearest)?,
        ];
        let fallback = [
            texels(&device, [1, 1], &BACKDROP_FALLBACK)?,
            texels(&device, [1, 1], &[255; 4])?,
        ];
        Ok(Self {
            device,
            library,
            pipelines: HashMap::new(),
            samplers,
            textures: HashMap::new(),
            blur,
            backdrop: None,
            fallback,
            in_flight: None,
        })
    }

    pub fn device(&self) -> &Device {
        &self.device
    }

    #[cfg(test)]
    pub fn has_texture(&self, id: TextureId) -> bool {
        self.textures.contains_key(&id)
    }

    fn pipeline(&mut self, format: MTLPixelFormat) -> Result<Pipeline, String> {
        if let Some(p) = self.pipelines.get(&format.0) {
            return Ok(p.clone());
        }
        let p = pipeline(
            &self.device,
            &self.library,
            ["vs_main", "ps_main"],
            format,
            true,
        )?;
        self.pipelines.insert(format.0, p.clone());
        Ok(p)
    }

    /// Blocks until the last submitted frame has finished on the GPU.
    pub fn wait_idle(&mut self) {
        if let Some(commands) = self.in_flight.take() {
            commands.waitUntilCompleted();
        }
    }

    fn buffer<T>(&self, data: &[T]) -> Result<Retained<ProtocolObject<dyn MTLBuffer>>, String> {
        let bytes = size_of_val(data);
        let ptr = NonNull::new(data.as_ptr().cast_mut().cast::<c_void>())
            .ok_or_else(|| "Metal: null buffer data".to_string())?;
        // SAFETY: `ptr` points to `bytes` readable bytes of `data`, copied synchronously into
        // a new shared buffer.
        unsafe {
            self.device.newBufferWithBytes_length_options(
                ptr,
                bytes,
                MTLResourceOptions::StorageModeShared,
            )
        }
        .ok_or_else(|| format!("Metal: buffer of {bytes} bytes"))
    }

    /// The target as the blur reads it: itself, or a non-sRGB view of it so the blur stays
    /// in gamma space like the rest of the menu; `None` when it cannot be sampled.
    fn readable(target: &Texture) -> Option<Retained<Texture>> {
        if !target.usage().contains(MTLTextureUsage::ShaderRead) {
            return None;
        }
        let plain = match target.pixelFormat() {
            MTLPixelFormat::RGBA8Unorm_sRGB => MTLPixelFormat::RGBA8Unorm,
            MTLPixelFormat::BGRA8Unorm_sRGB => MTLPixelFormat::BGRA8Unorm,
            MTLPixelFormat::RGBA8Unorm
            | MTLPixelFormat::BGRA8Unorm
            | MTLPixelFormat::RGB10A2Unorm
            | MTLPixelFormat::BGR10A2Unorm => return Some(target.retain()),
            _ => return None,
        };
        target.newTextureViewWithPixelFormat(plain)
    }

    /// Blurs the target under `region` (pixels) into the quarter-resolution target and
    /// returns it plus the UV scale for the main shader. Encodes its own passes into
    /// `commands`; never waits on the GPU.
    fn blur_backdrop(
        &mut self,
        commands: &CommandBuffer,
        target: &Texture,
        region: Rect,
    ) -> Result<Option<Blurred>, String> {
        let Some(source) = Self::readable(target) else {
            return Ok(None);
        };
        let size = [target.width() as u32, target.height() as u32];
        if self.backdrop.as_ref().is_none_or(|b| b.size != size) {
            // Old targets go first: at most one set alive.
            self.backdrop.take();
            let qsize = [size[0].div_ceil(4).max(1), size[1].div_ceil(4).max(1)];
            let quarter = || {
                texture_2d(
                    &self.device,
                    MTLPixelFormat::RGBA8Unorm,
                    qsize,
                    MTLTextureUsage::RenderTarget | MTLTextureUsage::ShaderRead,
                    MTLStorageMode::Private,
                )
            };
            self.backdrop = Some(Backdrop {
                size,
                qsize,
                quarter: [quarter()?, quarter()?],
            });
        }
        let Some(b) = self.backdrop.as_ref() else {
            return Ok(None);
        };
        // Read box: the glass region plus a margin, on the 4-pixel grid.
        let clamp = |v: f32, max: u32| (v.max(0.0) as u32).min(max);
        let x0 = clamp(region.min[0] - BLUR_MARGIN, size[0]) / 4 * 4;
        let y0 = clamp(region.min[1] - BLUR_MARGIN, size[1]) / 4 * 4;
        let x1 = clamp(region.max[0] + BLUR_MARGIN, size[0]).div_ceil(4) * 4;
        let y1 = clamp(region.max[1] + BLUR_MARGIN, size[1]).div_ceil(4) * 4;
        let (x1, y1) = (x1.min(size[0]), y1.min(size[1]));
        if x0 >= x1 || y0 >= y1 {
            return Ok(None);
        }
        let q = b.qsize;
        let scissor = MTLScissorRect {
            x: (x0 / 4) as usize,
            y: (y0 / 4) as usize,
            width: (x1.div_ceil(4).min(q[0]) - x0 / 4) as usize,
            height: (y1.div_ceil(4).min(q[1]) - y0 / 4) as usize,
        };
        let full = [(q[0] * 4) as f32, (q[1] * 4) as f32];
        let down: [f32; 8] = [
            1.0 / size[0] as f32,
            1.0 / size[1] as f32,
            0.0,
            0.0,
            full[0] / size[0] as f32,
            full[1] / size[1] as f32,
            0.0,
            0.0,
        ];
        let h: [f32; 8] = [0.0, 0.0, BLUR_SPREAD / q[0] as f32, 0.0, 1.0, 1.0, 0.0, 0.0];
        let v: [f32; 8] = [0.0, 0.0, 0.0, BLUR_SPREAD / q[1] as f32, 1.0, 1.0, 0.0, 0.0];
        let [qa, qb] = &b.quarter;
        let pass = |dst: &Texture,
                    src: &Texture,
                    pipeline: &Pipeline,
                    values: &[f32; 8]|
         -> Result<(), String> {
            let pass = MTLRenderPassDescriptor::new();
            // SAFETY: Index 0 is within the pass's colour attachment array.
            let color = unsafe { pass.colorAttachments().objectAtIndexedSubscript(0) };
            color.setTexture(Some(dst));
            // Outside the scissor the target keeps what it had, as in D3D11.
            color.setLoadAction(MTLLoadAction::Load);
            color.setStoreAction(MTLStoreAction::Store);
            let encoder = commands
                .renderCommandEncoderWithDescriptor(&pass)
                .ok_or_else(|| "Metal: blur encoder".to_string())?;
            encoder.setRenderPipelineState(pipeline);
            encoder.setScissorRect(scissor);
            // SAFETY: Source texture and sampler are live and retained by the encoder; the
            // 32 constant bytes are copied when set; the full-screen triangle needs no
            // buffers.
            unsafe {
                encoder.setFragmentBytes_length_atIndex(bytes_ptr(values), 32, 0);
                encoder.setFragmentTexture_atIndex(Some(src), 0);
                encoder.setFragmentSamplerState_atIndex(Some(&self.samplers[0]), 0);
                encoder.drawPrimitives_vertexStart_vertexCount(MTLPrimitiveType::Triangle, 0, 3);
            }
            encoder.endEncoding();
            Ok(())
        };
        pass(qa, &source, &self.blur.down, &down)?;
        pass(qb, qa, &self.blur.blur, &h)?;
        pass(qa, qb, &self.blur.blur, &v)?;
        Ok(Some((qa.clone(), [1.0 / full[0], 1.0 / full[1]])))
    }
}

impl MenuRenderer for MetalRenderer {
    type Target<'t> = MetalTarget<'t>;

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
                    return Err(format!("menu: invalid texture {id:?}"));
                }
                let texture = texels(&self.device, *size, rgba)?;
                self.textures.insert(
                    *id,
                    Atlas {
                        texture,
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
                let size = self
                    .textures
                    .get(id)
                    .ok_or_else(|| format!("menu: partial update of unknown texture {id:?}"))?
                    .size;
                if x + width > size[0]
                    || y + height > size[1]
                    || rgba.len() != (width * height * 4) as usize
                {
                    return Err(format!("menu: invalid partial update {id:?}"));
                }
                self.wait_idle();
                write_texels(
                    &self.textures[id].texture,
                    [*x, *y],
                    [*width, *height],
                    rgba,
                );
            }
            // The command buffers that still use it keep the texture alive.
            TextureDelta::Free(id) => {
                self.textures.remove(id);
            }
        }
        Ok(())
    }

    fn draw(
        &mut self,
        target: MetalTarget<'_>,
        list: &DrawList,
        size_px: [u32; 2],
    ) -> Result<(), String> {
        let [width, height] = size_px;
        let empty = width == 0 || height == 0 || list.indices.is_empty();
        if empty && target.clear.is_none() {
            return Ok(());
        }
        if list
            .indices
            .iter()
            .any(|&i| i as usize >= list.vertices.len())
        {
            return Err("menu: index beyond the vertices".into());
        }
        let format = target.texture.pixelFormat();
        let pipeline = self.pipeline(format)?;
        // Backdrop blur first (own passes), then the menu in one pipeline. A clear in the
        // same frame would blur the cleared target, so it only blurs what is already there.
        let blurred = match list.backdrop {
            Some(region) if !empty && target.clear.is_none() => {
                self.blur_backdrop(target.commands, target.texture, region)?
            }
            _ => None,
        };
        let (backdrop, backdrop_scale) = match blurred {
            Some((texture, scale)) => (texture, scale),
            None => (self.fallback[0].clone(), [0.0, 0.0]),
        };
        let lut = self
            .textures
            .get(&GRADIENT_TEXTURE)
            .map_or_else(|| self.fallback[1].clone(), |t| t.texture.clone());

        let pass = MTLRenderPassDescriptor::new();
        // SAFETY: Index 0 is within the pass's colour attachment array.
        let color = unsafe { pass.colorAttachments().objectAtIndexedSubscript(0) };
        color.setTexture(Some(target.texture));
        color.setStoreAction(MTLStoreAction::Store);
        match target.clear {
            Some([red, green, blue, alpha]) => {
                color.setLoadAction(MTLLoadAction::Clear);
                color.setClearColor(MTLClearColor {
                    red,
                    green,
                    blue,
                    alpha,
                });
            }
            None => color.setLoadAction(MTLLoadAction::Load),
        }
        let encoder = target
            .commands
            .renderCommandEncoderWithDescriptor(&pass)
            .ok_or_else(|| "Metal: render encoder".to_string())?;
        if !empty {
            let vertices = self.buffer(&list.vertices)?;
            let indices = self.buffer(&list.indices)?;
            let screen: [f32; 8] = [
                width as f32,
                height as f32,
                if is_srgb(format) { 1.0 } else { 0.0 },
                0.0,
                backdrop_scale[0],
                backdrop_scale[1],
                0.0,
                0.0,
            ];
            encoder.setRenderPipelineState(&pipeline);
            // SAFETY: The buffers live in the command buffer until it completes; the shader
            // reads one packed 20-byte VsIn per index, all indices were checked above. The
            // 32 constant bytes are copied when set; textures and samplers are retained by
            // the encoder.
            unsafe {
                encoder.setVertexBuffer_offset_atIndex(Some(&vertices), 0, 0);
                encoder.setVertexBytes_length_atIndex(bytes_ptr(&screen), 32, 1);
                encoder.setFragmentBytes_length_atIndex(bytes_ptr(&screen), 32, 0);
                encoder.setFragmentTexture_atIndex(Some(&backdrop), 1);
                encoder.setFragmentTexture_atIndex(Some(&lut), 2);
                encoder.setFragmentSamplerState_atIndex(Some(&self.samplers[0]), 1);
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
                let sampler = &self.samplers[usize::from(texture.nearest)];
                encoder.setScissorRect(MTLScissorRect {
                    x: left as usize,
                    y: top as usize,
                    width: (right - left) as usize,
                    height: (bottom - top) as usize,
                });
                // SAFETY: Texture and sampler are live and retained by the encoder; the index
                // range lies within the index buffer of 32-bit indices.
                unsafe {
                    encoder.setFragmentTexture_atIndex(Some(&texture.texture), 0);
                    encoder.setFragmentSamplerState_atIndex(Some(sampler), 0);
                    encoder
                        .drawIndexedPrimitives_indexCount_indexType_indexBuffer_indexBufferOffset(
                            MTLPrimitiveType::Triangle,
                            (end - cmd.indices.start) as usize,
                            MTLIndexType::UInt32,
                            &indices,
                            cmd.indices.start as usize * 4,
                        );
                }
            }
        }
        encoder.endEncoding();
        self.in_flight = Some(target.commands.retain());
        Ok(())
    }
}

/// Offscreen RGBA8 render target with readback.
pub struct Offscreen {
    queue: Retained<CommandQueue>,
    texture: Retained<Texture>,
    renderer: MetalRenderer,
    clear: Option<[f64; 4]>,
    pub size: [u32; 2],
}

impl Offscreen {
    pub fn new(width: u32, height: u32) -> Result<Self, String> {
        Self::with_format(width, height, MTLPixelFormat::RGBA8Unorm)
    }

    /// A target of another 8-bit RGBA format (e.g. `RGBA8Unorm_sRGB`).
    pub fn with_format(width: u32, height: u32, format: MTLPixelFormat) -> Result<Self, String> {
        let device = default_device()?;
        let queue = device
            .newCommandQueue()
            .ok_or_else(|| "Metal: command queue".to_string())?;
        let texture = texture_2d(
            &device,
            format,
            [width, height],
            MTLTextureUsage::RenderTarget
                | MTLTextureUsage::ShaderRead
                | MTLTextureUsage::PixelFormatView,
            MTLStorageMode::Managed,
        )?;
        let renderer = MetalRenderer::new(device)?;
        Ok(Self {
            queue,
            texture,
            renderer,
            clear: None,
            size: [width, height],
        })
    }

    /// Clears to `rgba` (premultiplied) at the start of the next [`Self::paint`].
    pub fn clear(&mut self, rgba: [f32; 4]) {
        self.clear = Some(rgba.map(f64::from));
    }

    pub fn paint(&mut self, textures: &[TextureDelta], list: &DrawList) -> Result<(), String> {
        let commands = self
            .queue
            .commandBuffer()
            .ok_or_else(|| "Metal: command buffer".to_string())?;
        let target = MetalTarget {
            commands: &commands,
            texture: &self.texture,
            clear: self.clear.take(),
        };
        self.renderer.paint(target, textures, list, self.size)?;
        commands.commit();
        commands.waitUntilCompleted();
        Ok(())
    }

    pub fn pixels(&self) -> Result<Vec<u8>, String> {
        let [width, height] = self.size;
        let commands = self
            .queue
            .commandBuffer()
            .ok_or_else(|| "Metal: command buffer".to_string())?;
        let blit = commands
            .blitCommandEncoder()
            .ok_or_else(|| "Metal: blit encoder".to_string())?;
        blit.synchronizeResource(ProtocolObject::from_ref(&*self.texture));
        blit.endEncoding();
        commands.commit();
        commands.waitUntilCompleted();
        let mut out = vec![0u8; (width * height * 4) as usize];
        let region = MTLRegion {
            origin: MTLOrigin { x: 0, y: 0, z: 0 },
            size: MTLSize {
                width: width as usize,
                height: height as usize,
                depth: 1,
            },
        };
        let dst = NonNull::new(out.as_mut_ptr().cast::<c_void>())
            .ok_or_else(|| "Metal: empty readback".to_string())?;
        // SAFETY: `out` holds width * height * 4 writable bytes, the region covers the whole
        // texture and the GPU finished writing it (synchronised above).
        unsafe {
            self.texture.getBytes_bytesPerRow_fromRegion_mipmapLevel(
                dst,
                width as usize * 4,
                region,
                0,
            )
        };
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::{FONT_TEXTURE, Rect, rgba};

    fn white_atlas() -> TextureDelta {
        TextureDelta::Create {
            id: FONT_TEXTURE,
            size: [4, 4],
            rgba: vec![255; 64],
            nearest: false,
        }
    }

    #[test]
    fn draws_premultiplied_quads_with_scissor() {
        let mut target = Offscreen::new(64, 64).unwrap();
        let mut list = DrawList::default();
        list.rect(Rect::new(0.0, 0.0, 32.0, 64.0), rgba(255, 0, 0, 255));
        list.push_clip(Rect::new(0.0, 0.0, 64.0, 16.0));
        list.rect(Rect::new(0.0, 0.0, 64.0, 64.0), rgba(0, 0, 255, 128));
        list.pop_clip();
        target.clear([0.0; 4]);
        target.paint(&[white_atlas()], &list).unwrap();
        let px = target.pixels().unwrap();
        let at = |x: usize, y: usize| &px[(y * 64 + x) * 4..(y * 64 + x) * 4 + 4];
        assert_eq!(at(8, 40), [255, 0, 0, 255], "opaque red");
        assert_eq!(at(40, 40), [0, 0, 0, 0], "untouched");
        let blended = at(8, 8);
        assert!(blended[0].abs_diff(127) <= 1 && blended[2].abs_diff(128) <= 1);
        assert_eq!(blended[3], 255);
        let over_clear = at(40, 8);
        assert_eq!(over_clear[3], 128, "half-transparent blue over nothing");
    }

    fn solid(list: &mut DrawList, rect: Rect, color: u32) {
        list.rect(rect, color);
    }

    fn near(a: &[u8], b: [u8; 4]) -> bool {
        a.iter().zip(b).all(|(&x, y)| x.abs_diff(y) <= 3)
    }

    /// `gradient_uv` samples the 256×1 gradient row.
    #[test]
    fn gradient_uv_samples_the_lookup_row() {
        use crate::draw::{GRADIENT_WIDTH, gradient_uv};
        let mut target = Offscreen::new(32, 8).unwrap();
        let mut list = DrawList::default();
        for (x, t) in [(0.0, 0.0), (24.0, 1.0)] {
            let uv = Rect {
                min: gradient_uv(t),
                max: gradient_uv(t),
            };
            let white = rgba(255, 255, 255, 255);
            list.quad_uv(Rect::new(x, 0.0, 8.0, 8.0), uv, white, FONT_TEXTURE);
        }
        // Red → blue row.
        let row: Vec<u8> = (0..GRADIENT_WIDTH)
            .flat_map(|i| {
                let t = i as f32 / (GRADIENT_WIDTH - 1) as f32;
                [((1.0 - t) * 255.0) as u8, 0, (t * 255.0) as u8, 255]
            })
            .collect();
        let lut = TextureDelta::Create {
            id: GRADIENT_TEXTURE,
            size: [GRADIENT_WIDTH, 1],
            rgba: row,
            nearest: false,
        };
        target.clear([0.0; 4]);
        target.paint(&[white_atlas(), lut], &list).unwrap();
        let px = target.pixels().unwrap();
        let at = |x: usize, y: usize| &px[(y * 32 + x) * 4..(y * 32 + x) * 4 + 4];
        assert!(near(at(4, 4), [255, 0, 0, 255]), "{:?}", at(4, 4));
        assert!(near(at(28, 4), [0, 0, 255, 255]), "{:?}", at(28, 4));
    }

    /// A shape with `BACKDROP_UV` shows the blurred target under it (tone-mapped, same
    /// hue) and sharp edges get soft. Without `backdrop` the dark stand-in texel is used.
    #[test]
    fn backdrop_uv_shows_the_blurred_target() {
        use crate::draw::BACKDROP_UV;
        let mut target = Offscreen::new(128, 64).unwrap();
        let mut scene = DrawList::default();
        solid(
            &mut scene,
            Rect::new(0.0, 0.0, 64.0, 64.0),
            rgba(255, 0, 0, 255),
        );
        solid(
            &mut scene,
            Rect::new(64.0, 0.0, 64.0, 64.0),
            rgba(0, 255, 0, 255),
        );
        target.clear([0.0; 4]);
        target.paint(&[white_atlas()], &scene).unwrap();
        let glass = Rect::new(16.0, 16.0, 96.0, 32.0);
        let mut list = DrawList::default();
        let uv = Rect {
            min: BACKDROP_UV,
            max: BACKDROP_UV,
        };
        list.quad_uv(glass, uv, rgba(255, 255, 255, 255), FONT_TEXTURE);
        list.add_backdrop(glass);
        target.paint(&[], &list).unwrap();
        let px = target.pixels().unwrap();
        let at = |x: usize, y: usize| px[(y * 128 + x) * 4..(y * 128 + x) * 4 + 4].to_vec();
        let left = at(24, 32);
        assert!(left[0] > 60 && left[1] < 30, "red side stays red: {left:?}");
        assert!(left[0] < 230, "bright backdrop is toned down: {left:?}");
        let right = at(104, 32);
        assert!(right[1] > 60 && right[0] < 30, "green side: {right:?}");
        let edge = at(64, 32);
        assert!(
            edge[0] > 20 && edge[1] > 20,
            "blurred across the edge: {edge:?}"
        );
        assert_eq!(at(4, 4), [255, 0, 0, 255], "outside the glass untouched");

        list.backdrop = None;
        target.paint(&[], &list).unwrap();
        let px = target.pixels().unwrap();
        let f = &px[(32 * 128 + 24) * 4..(32 * 128 + 24) * 4 + 4];
        assert!(near(f, [20, 22, 30, 255]), "fallback: {f:?}");
    }

    /// sRGB targets blur through a plain view and stay in gamma space: a flat mid grey
    /// comes back as a darker grey (the soft luminance cap), never the fallback.
    #[test]
    fn backdrop_on_srgb_target() {
        use crate::draw::BACKDROP_UV;
        let mut target = Offscreen::with_format(64, 64, MTLPixelFormat::RGBA8Unorm_sRGB).unwrap();
        let mut scene = DrawList::default();
        solid(
            &mut scene,
            Rect::new(0.0, 0.0, 64.0, 64.0),
            rgba(128, 128, 128, 255),
        );
        target.clear([0.0; 4]);
        target.paint(&[white_atlas()], &scene).unwrap();
        let mut list = DrawList::default();
        let uv = Rect {
            min: BACKDROP_UV,
            max: BACKDROP_UV,
        };
        let glass = Rect::new(8.0, 8.0, 48.0, 48.0);
        list.quad_uv(glass, uv, rgba(255, 255, 255, 255), FONT_TEXTURE);
        list.add_backdrop(glass);
        target.paint(&[], &list).unwrap();
        let px = target.pixels().unwrap();
        let p = &px[(32 * 64 + 32) * 4..(32 * 64 + 32) * 4 + 4];
        // 128/255 = 0.5 → 0.5 / (1 + 1.5 * 0.5) ≈ 0.29 → ~73 in gamma space.
        assert!((60..=90).contains(&p[0]) && p[0] == p[2], "{p:?}");
    }

    #[test]
    fn texture_updates_and_frees() {
        let mut target = Offscreen::new(8, 8).unwrap();
        let update = TextureDelta::Update {
            id: FONT_TEXTURE,
            pos: [1, 1],
            size: [2, 2],
            rgba: vec![0; 16],
        };
        target
            .paint(&[white_atlas(), update], &DrawList::default())
            .unwrap();
        assert!(target.renderer.has_texture(FONT_TEXTURE));
        let bad = TextureDelta::Update {
            id: FONT_TEXTURE,
            pos: [3, 3],
            size: [2, 2],
            rgba: vec![0; 16],
        };
        assert!(target.renderer.apply(&bad).is_err());
        target
            .paint(&[TextureDelta::Free(FONT_TEXTURE)], &DrawList::default())
            .unwrap();
        assert!(!target.renderer.has_texture(FONT_TEXTURE));
    }
}
