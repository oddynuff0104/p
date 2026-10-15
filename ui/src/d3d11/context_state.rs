//! Eigener D3D11-Pipeline-Zustand für das Menü (`mesh.rs`), gezeichnet im Immediate Context
//! direkt vor dem `Present` des Spiels.
//!
//! - Der Pipeline-Zustand des Spiels bleibt unangetastet: Wir binden einen
//!   eigenen `ID3DDeviceContextState` (`SwapDeviceContextState`), setzen darin
//!   alles, was wir brauchen, und tauschen danach den Zustand des Spiels zurück
//!   (E-OV-11). Voraussetzung ist die D3D11.1-Laufzeit (ab Windows 8).
//! - Vor dem Zurücktauschen werden Render Target und Textur gelöst: Der eigene
//!   Zustand würde den Backbuffer sonst festhalten (E-OV-02).

use windows::Win32::Graphics::Direct3D11::{
    D3D11_1_CREATE_DEVICE_CONTEXT_STATE_SINGLETHREADED, D3D11_CREATE_DEVICE_SINGLETHREADED,
    D3D11_SDK_VERSION, ID3D11Device, ID3D11Device1, ID3D11DeviceContext1, ID3DDeviceContextState,
};
use windows::core::Interface;

use super::hr;
/// Holds the previous context state until all overlay drawing has left scope.
pub(super) struct ContextState<'a> {
    context: &'a ID3D11DeviceContext1,
    previous: Option<ID3DDeviceContextState>,
}
impl<'a> ContextState<'a> {
    pub(super) fn enter(context: &'a ID3D11DeviceContext1, state: &ID3DDeviceContextState) -> Self {
        let mut previous = None;
        // SAFETY: context/state belong to the same device, are live throughout
        // drawing, and the Present thread exclusively uses the immediate context. The previous
        // state comes back as one owned reference (or None).
        unsafe { context.SwapDeviceContextState(state, Some(&mut previous)) };
        Self { context, previous }
    }
}
impl Drop for ContextState<'_> {
    fn drop(&mut self) {
        // SAFETY: the context and saved state outlive this guard. Releasing our
        // bindings before the swap keeps the game's backbuffer available to resize.
        unsafe {
            self.context.OMSetRenderTargets(None, None);
            self.context.PSSetShaderResources(0, Some(&[None]));
            self.context
                .SwapDeviceContextState(self.previous.as_ref(), None);
        }
    }
}

/// Eigener Zustand mit demselben Feature Level und Threading-Modus wie das Device des Spiels.
pub(super) fn create_context_state(
    device: &ID3D11Device,
) -> Result<ID3DDeviceContextState, String> {
    let device1: ID3D11Device1 = device
        .cast()
        .map_err(hr("QueryInterface(ID3D11Device1), D3D11.1 fehlt"))?;
    // SAFETY: The device is live and GetFeatureLevel has no pointer arguments.
    let level = unsafe { device.GetFeatureLevel() };
    // SAFETY: The device is live and GetCreationFlags has no pointer arguments.
    let flags = if unsafe { device.GetCreationFlags() } & D3D11_CREATE_DEVICE_SINGLETHREADED.0 != 0
    {
        D3D11_1_CREATE_DEVICE_CONTEXT_STATE_SINGLETHREADED.0 as u32
    } else {
        0
    };
    let mut state = None;
    // SAFETY: The feature-level slice is live for the call and the output slot is writable; the
    // emulated interface IID matches the device interface. Success stores one owned reference.
    unsafe {
        device1.CreateDeviceContextState(
            flags,
            &[level],
            D3D11_SDK_VERSION,
            &ID3D11Device::IID,
            None,
            Some(&mut state),
        )
    }
    .map_err(hr("CreateDeviceContextState"))?;
    state.ok_or_else(|| "CreateDeviceContextState: kein Objekt".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn context_state_is_restored_when_drawing_unwinds() {
        let device = super::super::mesh::warp_device().unwrap();
        // SAFETY: The device is live; GetImmediateContext returns an owned reference.
        let context: ID3D11DeviceContext1 = unsafe { device.GetImmediateContext() }
            .unwrap()
            .cast()
            .unwrap();
        let game = create_context_state(&device).unwrap();
        let overlay = create_context_state(&device).unwrap();
        // SAFETY: both states were created on this device and this test owns the context.
        unsafe { context.SwapDeviceContextState(&game, None) };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = ContextState::enter(&context, &overlay);
            panic!("injected drawing failure");
        }));
        assert!(result.is_err());
        let mut restored = None;
        // SAFETY: retrieve one owned reference to the state restored by Drop.
        unsafe { context.SwapDeviceContextState(&overlay, Some(&mut restored)) };
        assert_eq!(restored.unwrap().as_raw(), game.as_raw());
    }
}
