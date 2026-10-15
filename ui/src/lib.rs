//! Immediate-mode GUI, headless (input in, draw list out), plus its D3D11 renderer.
//! The window that hosts it lives in `poly-ui-app`.
//!
//! - [`kit`]: widgets, window shell, gallery; [`theme`]: colours, spacing, text styles.
//! - [`draw`], [`primitives`]: draw lists and anti-aliased shapes; [`render`]: the backend
//!   contract; [`d3d11`]: the D3D11 backend and an offscreen WARP target.
//! - [`input`]: Win32 messages → [`kit::KitInput`].

pub mod d3d11;
pub mod draw;
pub mod font;
pub mod input;
// A complete widget library: the gallery, previews and tests use every widget, a host only
// what it needs.
#[allow(dead_code)]
pub mod kit;
pub mod primitives;
pub mod render;
pub mod theme;
