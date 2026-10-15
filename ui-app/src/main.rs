//! Standalone window for the poly UI kit: a borderless, per-pixel transparent window
//! that draws the kit's shell and widgets, on Windows (Win32 + D3D11 + DirectComposition,
//! [`win`]) and macOS (AppKit + Metal, `mac`).
//!
//! ```text
//! cargo run -p poly-ui-app --release
//! ```

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod hit;
#[cfg(target_os = "macos")]
mod mac;
mod page;
#[cfg(windows)]
mod win;

#[cfg(windows)]
use win as platform;

#[cfg(target_os = "macos")]
use mac as platform;

fn main() {
    if let Err(e) = platform::run() {
        eprintln!("poly-ui-app: {e}");
        platform::error_box(&e);
        std::process::exit(1);
    }
}
