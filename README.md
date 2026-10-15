# poly-ui

A standalone window for the poly UI kit: the same shell, widgets, theme and Liquid Glass look as the poly menu, in a borderless, per-pixel transparent window. On Windows it is Win32 + D3D11 + DirectComposition, on macOS AppKit + Metal.

```sh
cargo run -p poly-ui-app --release
```

## Layout

| Path | Contents |
| --- | --- |
| `ui/` | `poly-ui`: immediate-mode kit (widgets, window shell, gallery), theme, draw lists, anti-aliased primitives, font, D3D11 renderer (Windows), Metal renderer (macOS), input collector |
| `ui-app/` | `poly-ui-app`: test page and hit testing shared by both hosts; `win/` window creation, D3D11 + DirectComposition swapchain, message loop; `mac/` NSWindow, `CAMetalLayer`, display link, key mapping |

## The window

- Borderless Win32 window (`WS_EX_NOREDIRECTIONBITMAP`, DWM frame, shadow, corners and border off).
- D3D11 renders into a premultiplied-alpha composition swapchain shown through DirectComposition, so the shell's rounded corners and soft shadow sit on the desktop.
- The shell fills the window inside a shadow margin (`Shell::host`). Title bar moves the window (double-click maximises), the corner grip and a thin band around the edge resize it, the X closes it. Moving and resizing go through `WM_NCHITTEST`, so snapping and the system menu work.
- Per-monitor DPI, mouse capture while dragging, redraw during the modal move/resize loop, vsync-paced rendering.
- Pages: **Test inputs** (one of each input with live values, buttons, toast, confirm dialog, accent) and **Gallery** (every widget).

Known limit: clicks on the transparent shadow outside the resize band are swallowed instead of reaching the window behind.

### macOS

- Borderless, transparent `NSWindow` without its own shadow; the content view hosts a `CAMetalLayer` (sRGB colour space, `framebufferOnly` off so the glass can blur the drawable), drawn on every display-link tick.
- The Metal renderer runs the same shaders and glass blur (¼ downsample, separable Gaussian) as D3D11, so both platforms look the same.
- Title bar drags through `performWindowDragWithEvent` (tiling, Spaces); double-click follows the system's title-bar setting. Edges, corners and the grip resize the window, with the system resize cursors; the shell redraws in step with the frame.
- Keys arrive as Windows virtual-key codes (layout-independent `kVK_*` mapping), ⌥ is Alt, ⌘A / ⌃A select all, ⌘Q quits, ⌘H hides, ⌘M minimises; trackpad scrolling is smooth.

## Build and check

Windows x64 (MSVC) or macOS 11+ (Apple Silicon or Intel), Rust 1.96 or newer.

```sh
cargo build --release --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all --check
```

The font is Cantarell Regular (SIL OFL 1.1, `ui/assets/fonts/`).
