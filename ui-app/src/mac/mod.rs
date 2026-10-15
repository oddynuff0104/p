//! macOS host: a borderless, transparent AppKit window whose content view is a
//! `CAMetalLayer` the Metal renderer draws into, paced by the display link.
//!
//! The shell is the whole window, as on Windows: its title bar moves the window
//! (`performWindowDragWithEvent`, so tiling and Spaces work; double-click follows the
//! system's "double-click a window's title bar" setting), its corner grip and a thin band
//! around its edge resize it, and the close button quits. The shell draws its own shadow,
//! rounded corners and border, so the window has none of its own.

use core::cell::RefCell;
use std::time::Instant;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{AnyThread, ClassType, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAlert, NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSColor, NSCursor,
    NSCursorFrameResizeDirections, NSCursorFrameResizePosition, NSEvent, NSMenu, NSMenuItem,
    NSResponder, NSScreen, NSTrackingArea, NSTrackingAreaOptions, NSView, NSWindow,
    NSWindowStyleMask,
};
use objc2_core_foundation::CGSize;
use objc2_core_graphics::{CGColorSpace, kCGColorSpaceSRGB};
use objc2_foundation::{
    NSObject, NSObjectProtocol, NSPoint, NSRect, NSRunLoop, NSRunLoopCommonModes, NSSize, NSString,
    NSTimer, NSUserDefaults,
};
use objc2_metal::{MTLCommandBuffer, MTLCommandQueue, MTLDevice, MTLDrawable, MTLPixelFormat};
use objc2_quartz_core::{CAMetalDrawable, CAMetalLayer};
use poly_ui::input::Collector;
use poly_ui::metal::{CommandQueue, MetalRenderer, MetalTarget, default_device};
use poly_ui::render::MenuRenderer;

use crate::hit::{Hit, hit_test};
use crate::page::Page;

mod keys;

/// An edge or corner drag in progress: where it started (screen points) and which edges
/// move.
struct Resize {
    edges: [i8; 2],
    mouse: NSPoint,
    frame: NSRect,
}

struct App {
    window: Retained<PolyWindow>,
    view: Retained<PolyView>,
    layer: Retained<CAMetalLayer>,
    queue: Retained<CommandQueue>,
    renderer: MetalRenderer,
    page: Page,
    input: Collector,
    last: Option<Instant>,
    /// Drawable size in pixels.
    size: [u32; 2],
    /// Frame before "maximise" (zoom to the visible screen area); `Some` while maximised.
    restore: Option<NSRect>,
    resize: Option<Resize>,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Runs `f` on the app unless it does not exist yet or is busy (an AppKit callback from
/// inside a handler that holds it, e.g. a resize from `setFrame`).
fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| a.try_borrow_mut().ok()?.as_mut().map(f))
}

/// Ends the process after a failure while the event loop runs.
fn fail(e: &str) -> ! {
    eprintln!("poly-ui-app: {e}");
    error_box(e);
    std::process::exit(1);
}

/// What a mouse-down on the window does, decided while the app is borrowed and carried
/// out after (window drags and zooms run AppKit loops that call back into the app).
enum Press {
    Drag,
    DoubleClickCaption,
    Nothing,
}

impl App {
    fn scale(&self) -> f32 {
        self.page.kit.theme.scale
    }

    /// Transparent shadow margin (none while maximised).
    fn margin(&self) -> f32 {
        if self.restore.is_some() {
            0.0
        } else {
            self.page.shadow_margin()
        }
    }

    /// Mouse position of `event` in surface pixels (top-left origin).
    fn pixel(&self, event: &NSEvent) -> [f32; 2] {
        let p = self
            .view
            .convertPoint_fromView(event.locationInWindow(), None);
        let s = self.scale();
        [p.x as f32 * s, p.y as f32 * s]
    }

    fn hit(&self, p: [f32; 2]) -> Hit {
        let shell = &self.page.shell;
        let areas = [shell.window(), shell.caption(), shell.grip()];
        if self.restore.is_some() {
            return if areas[1].contains(p) {
                Hit::Caption
            } else {
                Hit::Client
            };
        }
        let size = [self.size[0] as f32, self.size[1] as f32];
        hit_test(p, size, self.margin(), self.scale(), areas)
    }

    /// Size in pixels the drawable should have for the view's current bounds.
    fn view_pixels(&self) -> [u32; 2] {
        let b = self.view.bounds().size;
        let s = f64::from(self.scale());
        [
            (b.width * s).round().max(1.0) as u32,
            (b.height * s).round().max(1.0) as u32,
        ]
    }

    /// One frame: input → kit → Metal, presented on the next vblank (or with the window's
    /// own transaction while it is being resized, so content and frame move together).
    fn render(&mut self) {
        if self.window.isMiniaturized() || !self.window.isVisible() {
            return;
        }
        let size = self.view_pixels();
        if size != self.size {
            self.size = size;
            self.layer.setDrawableSize(CGSize {
                width: f64::from(size[0]),
                height: f64::from(size[1]),
            });
        }
        let now = Instant::now();
        let dt = self.last.map_or(0.0, |l| (now - l).as_secs_f32());
        self.last = Some(now);
        let mut input = self.input.take();
        input.dt = dt;
        let margin = self.margin();
        let (backdrop, list, textures, close) = self.page.frame(input, self.size, margin);
        let Some(drawable) = self.layer.nextDrawable() else {
            return;
        };
        let Some(commands) = self.queue.commandBuffer() else {
            fail("Metal: command buffer");
        };
        let texture = drawable.texture();
        let scene = MetalTarget {
            commands: &commands,
            texture: &texture,
            clear: Some([0.0; 4]),
        };
        let kit = MetalTarget {
            commands: &commands,
            texture: &texture,
            clear: None,
        };
        let painted = self
            .renderer
            .paint(scene, &textures, backdrop, self.size)
            .and_then(|()| self.renderer.paint(kit, &[], list, self.size));
        if let Err(e) = painted {
            fail(&e);
        }
        let drawable: &ProtocolObject<dyn MTLDrawable> = ProtocolObject::from_ref(&*drawable);
        if self.resize.is_some() {
            commands.commit();
            commands.waitUntilScheduled();
            drawable.present();
        } else {
            commands.presentDrawable(drawable);
            commands.commit();
        }
        if close {
            let mtm = MainThreadMarker::from(&*self.view);
            NSApplication::sharedApplication(mtm).terminate(None);
        }
    }

    fn mouse_down(&mut self, event: &NSEvent) -> Press {
        let p = self.pixel(event);
        let hit = self.hit(p);
        if let Some(edges) = hit.edges() {
            self.resize = Some(Resize {
                edges,
                mouse: NSEvent::mouseLocation(),
                frame: self.window.frame(),
            });
            self.layer.setPresentsWithTransaction(true);
            return Press::Nothing;
        }
        match hit {
            Hit::Caption if event.clickCount() == 2 => Press::DoubleClickCaption,
            Hit::Caption => Press::Drag,
            Hit::Client => {
                self.input.pointer(p);
                self.input.button(0, true);
                Press::Nothing
            }
            _ => Press::Nothing,
        }
    }

    /// Follows an edge drag: moves the dragged edges with the mouse, keeps the shell's
    /// minimum size, and draws the new size before the window shows it.
    fn resize_to_mouse(&mut self) {
        let Some(r) = &self.resize else {
            return;
        };
        let m = NSEvent::mouseLocation();
        let (dx, dy) = (m.x - r.mouse.x, m.y - r.mouse.y);
        let s = f64::from(self.scale());
        let min = self.page.min_size().map(|v| f64::from(v) / s);
        let f = r.frame;
        let (mut x, mut y, mut w, mut h) = (f.origin.x, f.origin.y, f.size.width, f.size.height);
        // Screen coordinates grow upwards: the top edge is y + h.
        match r.edges[0] {
            -1 => {
                w = (f.size.width - dx).max(min[0]);
                x = f.origin.x + f.size.width - w;
            }
            1 => w = (f.size.width + dx).max(min[0]),
            _ => {}
        }
        match r.edges[1] {
            -1 => h = (f.size.height + dy).max(min[1]),
            1 => {
                h = (f.size.height - dy).max(min[1]);
                y = f.origin.y + f.size.height - h;
            }
            _ => {}
        }
        let frame = NSRect::new(NSPoint::new(x, y), NSSize::new(w, h));
        if frame != self.window.frame() {
            self.window.setFrame_display(frame, true);
            self.render();
        }
    }

    fn mouse_up(&mut self, event: &NSEvent) {
        if self.resize.take().is_some() {
            self.layer.setPresentsWithTransaction(false);
            return;
        }
        let p = self.pixel(event);
        self.input.pointer(p);
        self.input.button(0, false);
    }

    fn mouse_moved(&mut self, event: &NSEvent) {
        let p = self.pixel(event);
        self.input.pointer(p);
        cursor(self.hit(p)).set();
    }

    fn other_button(&mut self, event: &NSEvent, down: bool) {
        let p = self.pixel(event);
        self.input.pointer(p);
        let button = match event.buttonNumber() {
            1 => 1,
            n @ 2..=4 => n as usize,
            _ => return,
        };
        self.input.button(button, down);
    }

    /// Zoom to the screen's visible area and back.
    fn toggle_maximize(&mut self) -> NSRect {
        match self.restore.take() {
            Some(frame) => frame,
            None => {
                let frame = self.window.frame();
                self.restore = Some(frame);
                self.window
                    .screen()
                    .map_or(frame, |screen| screen.visibleFrame())
            }
        }
    }

    /// Scale or screen changed: new atlas resolution, layer scale and drawable size.
    fn backing_changed(&mut self) {
        let scale = self.window.backingScaleFactor();
        self.layer.setContentsScale(scale);
        if let Err(e) = self.page.kit.set_scale(scale as f32) {
            fail(&e);
        }
        self.size = [0, 0];
    }
}

/// Resize cursors for the window edges (system frame-resize cursors on macOS 15+).
fn cursor(hit: Hit) -> Retained<NSCursor> {
    let modern = NSCursor::class().responds_to(sel!(frameResizeCursorFromPosition:inDirections:));
    let position = match hit {
        Hit::Left => NSCursorFrameResizePosition::Left,
        Hit::Right => NSCursorFrameResizePosition::Right,
        Hit::Top => NSCursorFrameResizePosition::Top,
        Hit::Bottom => NSCursorFrameResizePosition::Bottom,
        Hit::TopLeft => NSCursorFrameResizePosition::TopLeft,
        Hit::TopRight => NSCursorFrameResizePosition::TopRight,
        Hit::BottomLeft => NSCursorFrameResizePosition::BottomLeft,
        Hit::BottomRight => NSCursorFrameResizePosition::BottomRight,
        Hit::Nowhere | Hit::Client | Hit::Caption => return NSCursor::arrowCursor(),
    };
    #[allow(deprecated)] // The fallbacks are the only resize cursors before macOS 15.
    if modern {
        NSCursor::frameResizeCursorFromPosition_inDirections(
            position,
            NSCursorFrameResizeDirections::All,
        )
    } else if matches!(hit, Hit::Left | Hit::Right) {
        NSCursor::resizeLeftRightCursor()
    } else if matches!(hit, Hit::Top | Hit::Bottom) {
        NSCursor::resizeUpDownCursor()
    } else {
        NSCursor::crosshairCursor()
    }
}

define_class!(
    /// Borderless windows cannot become key by default; this one can, so it gets the
    /// keyboard.
    #[unsafe(super(NSWindow, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PolyWindow"]
    struct PolyWindow;

    unsafe impl NSObjectProtocol for PolyWindow {}

    impl PolyWindow {
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key_window(&self) -> bool {
            true
        }

        #[unsafe(method(canBecomeMainWindow))]
        fn can_become_main_window(&self) -> bool {
            true
        }

        /// Held modifiers and buttons do not survive switching apps.
        #[unsafe(method(resignKeyWindow))]
        fn resign_key_window(&self) {
            // SAFETY: Forwards to NSWindow's implementation with no arguments.
            let () = unsafe { msg_send![super(self), resignKeyWindow] };
            with_app(|a| a.input.reset());
        }
    }
);

define_class!(
    /// The content view: hosts the Metal layer and turns AppKit events into kit input.
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PolyView"]
    struct PolyView;

    unsafe impl NSObjectProtocol for PolyView {}

    impl PolyView {
        /// Top-left origin, like the kit.
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        /// The first click on an inactive window already works.
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        /// Display link (or timer) callback.
        #[unsafe(method(tick:))]
        fn tick(&self, _sender: Option<&AnyObject>) {
            with_app(App::render);
        }

        #[unsafe(method(viewDidChangeBackingProperties))]
        fn view_did_change_backing_properties(&self) {
            with_app(App::backing_changed);
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            match with_app(|a| a.mouse_down(event)) {
                Some(Press::Drag) => {
                    if let Some(window) = self.window() {
                        // Moving a maximised window makes it a normal one again.
                        with_app(|a| a.restore = None);
                        window.performWindowDragWithEvent(event);
                    }
                }
                Some(Press::DoubleClickCaption) => double_click_caption(),
                _ => {}
            }
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            with_app(|a| {
                if a.resize.is_some() {
                    a.resize_to_mouse();
                } else {
                    let p = a.pixel(event);
                    a.input.pointer(p);
                }
            });
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            with_app(|a| a.mouse_up(event));
        }

        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) {
            with_app(|a| a.mouse_moved(event));
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            with_app(|a| {
                if a.resize.is_none() {
                    a.input.leave();
                    NSCursor::arrowCursor().set();
                }
            });
        }

        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, event: &NSEvent) {
            with_app(|a| a.other_button(event, true));
        }

        #[unsafe(method(rightMouseUp:))]
        fn right_mouse_up(&self, event: &NSEvent) {
            with_app(|a| a.other_button(event, false));
        }

        #[unsafe(method(rightMouseDragged:))]
        fn right_mouse_dragged(&self, event: &NSEvent) {
            with_app(|a| {
                let p = a.pixel(event);
                a.input.pointer(p);
            });
        }

        #[unsafe(method(otherMouseDown:))]
        fn other_mouse_down(&self, event: &NSEvent) {
            with_app(|a| a.other_button(event, true));
        }

        #[unsafe(method(otherMouseUp:))]
        fn other_mouse_up(&self, event: &NSEvent) {
            with_app(|a| a.other_button(event, false));
        }

        #[unsafe(method(otherMouseDragged:))]
        fn other_mouse_dragged(&self, event: &NSEvent) {
            with_app(|a| {
                let p = a.pixel(event);
                a.input.pointer(p);
            });
        }

        /// Trackpads report points (one notch = 72 pt in the kit), wheels lines.
        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &NSEvent) {
            let dy = event.scrollingDeltaY() as f32;
            let notches = if event.hasPreciseScrollingDeltas() {
                dy / 72.0
            } else {
                dy
            };
            with_app(|a| a.input.wheel(notches));
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            with_app(|a| keys::key_down(&mut a.input, event));
        }

        #[unsafe(method(keyUp:))]
        fn key_up(&self, event: &NSEvent) {
            with_app(|a| keys::key_up(&mut a.input, event));
        }

        #[unsafe(method(flagsChanged:))]
        fn flags_changed(&self, event: &NSEvent) {
            with_app(|a| keys::flags_changed(&mut a.input, event));
        }
    }
);

impl PolyView {
    fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        // SAFETY: NSView's designated initialiser on a freshly allocated instance of this
        // subclass, which adds no instance variables.
        unsafe { msg_send![Self::alloc(mtm), initWithFrame: frame] }
    }
}

/// The system setting for double-clicking a title bar: zoom (default), minimise or nothing.
fn double_click_caption() {
    let action = NSUserDefaults::standardUserDefaults()
        .stringForKey(&NSString::from_str("AppleActionOnDoubleClick"))
        .map(|s| s.to_string());
    match action.as_deref() {
        Some("None") => {}
        Some("Minimize") => {
            if let Some(window) = with_app(|a| a.window.clone()) {
                window.miniaturize(None);
            }
        }
        _ => {
            let Some((window, frame)) = with_app(|a| (a.window.clone(), a.toggle_maximize()))
            else {
                return;
            };
            window.setFrame_display(frame, true);
        }
    }
}

/// App menu (Hide, Quit) and Window menu (Minimise), so the usual shortcuts work.
fn menu(mtm: MainThreadMarker) -> Retained<NSMenu> {
    let item = |title: &str, action, key: &str| {
        // SAFETY: Each action is a standard NSApplication / NSWindow selector that takes
        // the sender.
        unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(title),
                Some(action),
                &NSString::from_str(key),
            )
        }
    };
    let submenu = |title: &str, items: &[Retained<NSMenuItem>]| {
        let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(title));
        for i in items {
            menu.addItem(i);
        }
        let holder = NSMenuItem::new(mtm);
        holder.setSubmenu(Some(&menu));
        holder
    };
    let bar = NSMenu::new(mtm);
    bar.addItem(&submenu(
        "poly",
        &[
            item("Hide poly", sel!(hide:), "h"),
            item("Quit poly", sel!(terminate:), "q"),
        ],
    ));
    bar.addItem(&submenu(
        "Window",
        &[item("Minimize", sel!(miniaturize:), "m")],
    ));
    bar
}

pub fn run() -> Result<(), String> {
    let mtm = MainThreadMarker::new().ok_or("poly-ui-app must run on the main thread")?;
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    app.setMainMenu(Some(&menu(mtm)));

    let screen = NSScreen::mainScreen(mtm).ok_or("no screen")?;
    let scale = screen.backingScaleFactor();
    let page = Page::new(scale as f32)?;
    // Initial size: the shell's default size plus its shadow margin, in points.
    let margin = 2.0 * f64::from(page.shadow_margin()) / scale;
    let initial = page.shell.state.size;
    let work = screen.visibleFrame();
    let w = (f64::from(initial[0]) + margin).min(work.size.width);
    let h = (f64::from(initial[1]) + margin).min(work.size.height);
    let frame = NSRect::new(
        NSPoint::new(
            work.origin.x + (work.size.width - w) / 2.0,
            work.origin.y + (work.size.height - h) / 2.0,
        ),
        NSSize::new(w, h),
    );

    let style = NSWindowStyleMask::Borderless
        | NSWindowStyleMask::Resizable
        | NSWindowStyleMask::Miniaturizable;
    // SAFETY: NSWindow's designated initialiser on a fresh instance of a subclass without
    // instance variables; the arguments are plain values.
    let window: Retained<PolyWindow> = unsafe {
        msg_send![
            PolyWindow::alloc(mtm),
            initWithContentRect: frame,
            styleMask: style,
            backing: NSBackingStoreType::Buffered,
            defer: false,
        ]
    };
    // SAFETY: The window is owned by `App` for the rest of the process.
    unsafe { window.setReleasedWhenClosed(false) };
    window.setTitle(&NSString::from_str("poly"));
    window.setOpaque(false);
    window.setBackgroundColor(Some(&NSColor::clearColor()));
    window.setHasShadow(false);
    window.setAcceptsMouseMovedEvents(true);

    let device = default_device()?;
    let queue = device.newCommandQueue().ok_or("Metal: command queue")?;
    let layer = CAMetalLayer::new();
    layer.setDevice(Some(&device));
    layer.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
    // The glass blur reads the drawable.
    layer.setFramebufferOnly(false);
    layer.setOpaque(false);
    layer.setContentsScale(window.backingScaleFactor());
    // Same sRGB colours as on Windows, also on wide-gamut displays.
    // SAFETY: A constant CFString provided by CoreGraphics.
    let srgb = unsafe { kCGColorSpaceSRGB };
    layer.setColorspace(CGColorSpace::with_name(Some(srgb)).as_deref());

    let content = NSRect::new(NSPoint::new(0.0, 0.0), frame.size);
    let view = PolyView::new(mtm, content);
    // A layer-hosting view: the Metal layer is the view's own layer.
    view.setLayer(Some(&layer));
    view.setWantsLayer(true);
    // SAFETY: The tracking area's owner is the view itself, which outlives it; no user info.
    let tracking = unsafe {
        NSTrackingArea::initWithRect_options_owner_userInfo(
            NSTrackingArea::alloc(),
            content,
            NSTrackingAreaOptions::MouseEnteredAndExited
                | NSTrackingAreaOptions::MouseMoved
                | NSTrackingAreaOptions::ActiveAlways
                | NSTrackingAreaOptions::InVisibleRect,
            Some(&view),
            None,
        )
    };
    view.addTrackingArea(&tracking);
    window.setContentView(Some(&view));
    window.makeFirstResponder(Some(&view));

    let mut input = Collector::default();
    input.leave();
    let renderer = MetalRenderer::new(device)?;
    APP.with(|a| {
        *a.borrow_mut() = Some(App {
            window: window.clone(),
            view: view.clone(),
            layer,
            queue,
            renderer,
            page,
            input,
            last: None,
            size: [0, 0],
            restore: None,
            resize: None,
        })
    });

    // Frames follow the display's refresh (display link on macOS 14+, else a 120 Hz
    // timer); common modes keep them coming while menus or drags track the mouse.
    let run_loop = NSRunLoop::mainRunLoop();
    if view.respondsToSelector(sel!(displayLinkWithTarget:selector:)) {
        // SAFETY: `tick:` takes the sender and is implemented by the view, which the link
        // retains as its target.
        let link = unsafe { view.displayLinkWithTarget_selector(&view, sel!(tick:)) };
        // SAFETY: The main run loop and a standard mode.
        unsafe { link.addToRunLoop_forMode(&run_loop, NSRunLoopCommonModes) };
    } else {
        // SAFETY: As above, for a repeating timer.
        let timer = unsafe {
            NSTimer::timerWithTimeInterval_target_selector_userInfo_repeats(
                1.0 / 120.0,
                &view,
                sel!(tick:),
                None,
                true,
            )
        };
        // SAFETY: The main run loop and a standard mode.
        unsafe { run_loop.addTimer_forMode(&timer, NSRunLoopCommonModes) };
    }

    with_app(App::render);
    window.makeKeyAndOrderFront(None);
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
    app.run();
    Ok(())
}

/// Shows a fatal error in an alert.
pub fn error_box(e: &str) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let _ = NSApplication::sharedApplication(mtm);
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str("poly"));
    alert.setInformativeText(&NSString::from_str(e));
    alert.runModal();
}
