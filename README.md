# poly-ui

A standalone Windows window for the poly UI kit: the same shell, widgets, theme and D3D11 renderer as the poly menu, in a borderless, per-pixel transparent window.

```powershell
cargo run -p poly-ui-app --release
```

## Layout

| Path | Contents |
| --- | --- |
| `ui/` | `poly-ui`: immediate-mode kit (widgets, window shell, gallery), theme, draw lists, anti-aliased primitives, font, D3D11 renderer, Win32 input collector |
| `ui-app/` | `poly-ui-app.exe`: window creation, D3D11 + DirectComposition swapchain, message loop, test page |

## The window

- Borderless Win32 window (`WS_EX_NOREDIRECTIONBITMAP`, DWM frame, shadow, corners and border off).
- D3D11 renders into a premultiplied-alpha composition swapchain shown through DirectComposition, so the shell's rounded corners and soft shadow sit on the desktop.
- The shell fills the window inside a shadow margin (`Shell::host`). Title bar moves the window (double-click maximises), the corner grip and a thin band around the edge resize it, the X closes it. Moving and resizing go through `WM_NCHITTEST`, so snapping and the system menu work.
- Per-monitor DPI, mouse capture while dragging, redraw during the modal move/resize loop, vsync-paced rendering.
- Pages: **Test inputs** (one of each input with live values, buttons, toast, confirm dialog, accent) and **Gallery** (every widget).

Known limit: clicks on the transparent shadow outside the resize band are swallowed instead of reaching the window behind.

## Build and check

Windows x64 (MSVC), Rust 1.96 or newer.

```powershell
cargo build --release --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all --check
```

The font is Cantarell Regular (SIL OFL 1.1, `ui/assets/fonts/`).
