//! What the window shows: the poly shell with a "Test inputs" page (one of each input,
//! their live values next to them, the appearance editor) and the full widget gallery.
//!
//! The glass blurs whatever is in the back buffer under it. In the game that is the game
//! picture; here it is [`wallpaper`], a plain dark fill painted inside the shell before the
//! kit draws.

use poly_ui::draw::{DrawList, Rect, TextureDelta};
use poly_ui::kit::gallery::{self, GalleryState, WEAPONS};
use poly_ui::kit::shell::{NavItem, Shell, WindowState};
use poly_ui::kit::widgets::{ButtonKind, CardHead};
use poly_ui::kit::{Kit, KitInput, ToastKind};
use poly_ui::primitives::{Icon, Shapes};
use poly_ui::theme::{Color, GRADIENTS, Gradient, Theme};

const PAGES: [(&str, Icon); 2] = [("Test inputs", Icon::Keyboard), ("Gallery", Icon::Grid)];

#[derive(Clone)]
struct Inputs {
    enabled: bool,
    toggle: bool,
    checkbox: bool,
    slider: f32,
    stepper: i32,
    segment: usize,
    choice: usize,
    text: String,
    color: Color,
    key: u32,
    clicks: u32,
}

impl Default for Inputs {
    fn default() -> Self {
        Self {
            enabled: true,
            toggle: true,
            checkbox: false,
            slider: 42.0,
            stepper: 3,
            segment: 1,
            choice: 2,
            text: "Hello".into(),
            color: Color::rgb(255, 92, 132),
            key: 0,
            clicks: 0,
        }
    }
}

pub struct Page {
    pub kit: Kit,
    pub shell: Shell,
    inputs: Inputs,
    gallery: GalleryState,
    search: String,
    fps: f32,
    backdrop: DrawList,
}

impl Page {
    pub fn new(scale: f32) -> Result<Self, String> {
        let window = WindowState::default();
        let mut theme = Theme::glass(window.gradient, scale);
        theme.blur = window.blur;
        Ok(Self {
            kit: Kit::new(theme)?,
            shell: Shell::new(window),
            inputs: Inputs::default(),
            gallery: GalleryState::default(),
            search: String::new(),
            fps: 0.0,
            backdrop: DrawList::default(),
        })
    }

    /// Transparent margin around the shell that holds its shadow, in pixels: the shadow
    /// feathers half its blur beyond the offset rectangle.
    pub fn shadow_margin(&self) -> f32 {
        let t = &self.kit.theme;
        let sh = t.shadow_window;
        let offset = sh.offset[0].abs().max(sh.offset[1].abs());
        (t.pxf(sh.blur) * 0.5 + t.pxf(offset)).ceil()
    }

    /// Smallest window in pixels (shell minimum plus the margin).
    pub fn min_size(&self) -> [f32; 2] {
        let m = 2.0 * self.shadow_margin();
        let s = self.kit.theme.scale;
        [WindowState::MIN[0] * s + m, WindowState::MIN[1] * s + m]
    }

    /// One frame; `margin` is 0 while maximised. Returns the backdrop (painted first, the
    /// glass blurs it), the kit's draw list, texture changes and whether the close button
    /// was pressed.
    pub fn frame(
        &mut self,
        input: KitInput,
        size: [u32; 2],
        margin: f32,
    ) -> (&DrawList, &DrawList, Vec<TextureDelta>, bool) {
        if input.dt > 0.0 {
            let fps = 1.0 / input.dt;
            self.fps += (fps - self.fps) * if self.fps == 0.0 { 1.0 } else { 0.05 };
        }
        self.shell.host = Some(margin);
        self.kit.begin(input, size);
        let close = self.build(size);
        let radius = self.kit.theme.pxf(self.kit.theme.radius.xl);
        let (list, textures) = self.kit.end();
        self.backdrop.clear();
        wallpaper(&mut self.backdrop, self.shell.window(), radius);
        (&self.backdrop, list, textures, close)
    }

    fn build(&mut self, size: [u32; 2]) -> bool {
        let nav: Vec<NavItem> = PAGES
            .iter()
            .map(|&(label, icon)| NavItem {
                label,
                icon,
                badge: None,
            })
            .collect();
        let page = self.shell.state.page.min(PAGES.len() - 1);
        let mut search = std::mem::take(&mut self.search);
        let frame = self
            .shell
            .begin(&mut self.kit, PAGES[page].0, &nav, &mut search, "");
        self.search = search;
        if frame.page_changed {
            self.kit.scroll_to_top("content");
        }
        self.kit.begin_scroll("content", frame.content);
        let [x0, x1] = self.kit.region_x();
        let pad = self.kit.theme.sp(2);
        let y = self.kit.cursor_y() + pad;
        let scrollbar = self.kit.theme.px(8.0);
        self.kit.push_region(x0 + pad, x1 - pad - scrollbar, y, 0.0);
        match page {
            0 => self.test_inputs(),
            _ => gallery::draw(&mut self.kit, &mut self.gallery),
        }
        let y = self.kit.pop_region();
        self.kit.region().y = y + pad;
        self.kit.end_scroll();
        let status = format!("Ready · {}×{} · {:.0} FPS", size[0], size[1], self.fps);
        let right = format!("poly-ui {}", env!("CARGO_PKG_VERSION"));
        self.shell.end(&mut self.kit, &status, true, &right);
        frame.close
    }

    fn test_inputs(&mut self) {
        let kit = &mut self.kit;
        let s = &mut self.inputs;
        kit.begin_grid(kit.theme.px(320.0), 3);

        let mut on = s.enabled;
        kit.card(
            CardHead {
                title: "Inputs",
                icon: Some(Icon::Keyboard),
                hint: "One of each input; the values show up in the next card.",
                ..Default::default()
            },
            Some(&mut on),
        );
        s.enabled = on;
        kit.toggle("Toggle", &mut s.toggle);
        kit.checkbox("Checkbox", &mut s.checkbox);
        kit.slider_f32("Slider", &mut s.slider, (0.0, 100.0), 0.5, "%");
        kit.hint("Drag, Shift for fine control, arrows step.");
        kit.slider_i32("Stepper", &mut s.stepper, (1, 10), 1, "");
        kit.segmented("Segments", &mut s.segment, &["One", "Two", "Three"]);
        kit.dropdown("Dropdown", &mut s.choice, &WEAPONS);
        kit.text_field("Text", &mut s.text, 32);
        kit.color_edit("Colour", &mut s.color, &gallery::presets());
        kit.keybind("Key", &mut s.key);
        kit.end_card();

        kit.card(
            CardHead {
                title: "Live values",
                icon: Some(Icon::Eye),
                ..Default::default()
            },
            None,
        );
        let [r, g, b, _] = s.color.0;
        let key = if s.key == 0 {
            "none".to_string()
        } else {
            poly_ui::kit::widgets::vk_name(s.key)
        };
        for line in [
            format!("Card switch: {}", on_off(s.enabled)),
            format!("Toggle: {}", on_off(s.toggle)),
            format!("Checkbox: {}", on_off(s.checkbox)),
            format!("Slider: {:.1} %", s.slider),
            format!("Stepper: {}", s.stepper),
            format!("Segments: {}", ["One", "Two", "Three"][s.segment.min(2)]),
            format!("Dropdown: {}", WEAPONS[s.choice.min(WEAPONS.len() - 1)]),
            format!("Text: \"{}\"", s.text),
            format!("Colour: #{r:02x}{g:02x}{b:02x}"),
            format!("Key: {key}"),
            format!("Button clicks: {}", s.clicks),
        ] {
            kit.label(&line);
        }
        kit.end_card();

        kit.card(
            CardHead {
                title: "Actions",
                icon: Some(Icon::Run),
                ..Default::default()
            },
            None,
        );
        let row = kit.alloc(kit.theme.px(kit.theme.control));
        let w = (row.width() - kit.theme.sp(2)) / 2.0;
        let a = Rect::new(row.min[0], row.min[1], w, row.height());
        let b = Rect::new(
            row.min[0] + w + kit.theme.sp(2),
            row.min[1],
            w,
            row.height(),
        );
        if kit.button_at("Click me", a, ButtonKind::Primary) {
            s.clicks += 1;
            kit.toast(format!("Clicked {} times", s.clicks), ToastKind::Success);
        }
        if kit.button_at("Show error", b, ButtonKind::Secondary) {
            kit.toast("Something went wrong (on purpose)", ToastKind::Error);
        }
        if kit.button("Reset values…", ButtonKind::Danger) {
            kit.open_confirm("reset");
        }
        if let Some(yes) = kit.confirm(
            "reset",
            "Reset all test values?",
            "Every input returns to its starting value.",
            "Reset",
            true,
        ) && yes
        {
            *s = Inputs::default();
            kit.toast("Values reset", ToastKind::Info);
        }
        kit.end_card();

        self.appearance();
        self.kit.end_grid();
    }

    /// The accent gradient editor (preset, 1–6 stops, angle, hue shift), glass blur and
    /// the compact sidebar, as in poly's Settings › Appearance.
    fn appearance(&mut self) {
        let kit = &mut self.kit;
        kit.card(
            CardHead {
                title: "Appearance",
                icon: Some(Icon::Palette),
                hint: "Accent gradient, glass and sidebar.",
                ..Default::default()
            },
            None,
        );
        let before = self.shell.state.gradient;
        let mut g = before;
        gradient_preview(kit);
        let mut names: Vec<&str> = GRADIENTS.iter().map(|p| p.0).collect();
        names.extend(["Single colour", "Custom"]);
        let single = GRADIENTS.len();
        let current = match g.preset_index() {
            Some(i) => i,
            None if g.count == 1 => single,
            None => single + 1,
        };
        let mut pick = current;
        if kit.dropdown("Accent", &mut pick, &names) && pick != current {
            if pick < GRADIENTS.len() {
                g = Gradient {
                    angle: g.angle,
                    hue_speed: g.hue_speed,
                    ..Gradient::preset(pick)
                };
            } else if pick == single {
                g.count = 1;
            }
        }
        let mut count = g.count as i32;
        if kit.slider_i32(
            "Colour stops",
            &mut count,
            (1, Gradient::MAX_STOPS as i32),
            1,
            "",
        ) {
            g.set_count(count as usize);
        }
        let count = g.count;
        kit.color_row("Colours", &mut g.stops[..count], &gallery::presets());
        kit.hint("Click a swatch to edit that colour stop.");
        if g.count > 1 {
            kit.slider_f32("Gradient angle", &mut g.angle, (0.0, 360.0), 5.0, "°");
        }
        kit.slider_f32(
            "Hue shift",
            &mut g.hue_speed,
            (0.0, Gradient::MAX_HUE_SPEED),
            1.0,
            "°/s",
        );
        if g != before {
            self.shell.state.gradient = g;
            kit.set_gradient(g);
            self.shell.mark_changed();
        }
        let mut blur = self.shell.state.blur;
        if kit.toggle("Background blur", &mut blur) {
            self.shell.state.blur = blur;
            kit.theme.blur = blur;
            self.shell.mark_changed();
        }
        kit.hint("Frosted glass over the wallpaper; off = tinted glass without blur.");
        let mut collapsed = self.shell.state.sidebar_collapsed;
        if kit.toggle("Compact sidebar", &mut collapsed) {
            self.shell.state.sidebar_collapsed = collapsed;
            self.shell.mark_changed();
        }
        kit.end_card();
    }
}

/// A wide capsule showing the current gradient as the controls use it.
fn gradient_preview(kit: &mut Kit) {
    let t = kit.theme.clone();
    let row = kit.alloc(t.px(30.0));
    let h = t.px(18.0);
    let bar = Rect::new(row.min[0], (row.min[1] + t.px(5.0)).round(), row.width(), h);
    kit.grad_glow(bar, h / 2.0, t.pxf(6.0), 0.3);
    kit.grad_fill(bar, h / 2.0, 1.0, bar);
    kit.dl()
        .gradient_rounded(bar, h / 2.0, Color::WHITE.alpha(0.22).pm(), 0);
    kit.lit_rim(bar, h / 2.0, 1.0);
}

/// What the glass of `window` blurs: a plain deep blue-violet base inside the shell's
/// rounded corners.
fn wallpaper(list: &mut DrawList, window: Rect, radius: f32) {
    if window.width() <= 2.0 * radius || window.height() <= 2.0 * radius {
        return;
    }
    list.fill_rounded(window, radius, Color::rgb(14, 16, 34).pm());
}

fn on_off(v: bool) -> &'static str {
    if v { "on" } else { "off" }
}

#[cfg(test)]
mod tests {
    use super::*;
    use poly_ui::kit::Key;

    const SIZE: [u32; 2] = [1280, 800];

    fn at(mouse: [f32; 2], down: bool) -> KitInput {
        KitInput {
            mouse,
            down: [down, false],
            dt: 1.0 / 60.0,
            ..Default::default()
        }
    }

    #[test]
    fn hosted_shell_fills_the_window_and_only_the_button_closes() {
        let mut page = Page::new(1.0).unwrap();
        let margin = page.shadow_margin();
        assert!(margin > 0.0);
        for p in 0..PAGES.len() {
            page.shell.state.page = p;
            let (_, list, _, close) = page.frame(at([-1.0, -1.0], false), SIZE, margin);
            assert!(!list.commands.is_empty() && !close);
        }
        let window = page.shell.window();
        assert_eq!(window.min, [margin, margin]);
        assert_eq!(
            window.max,
            [SIZE[0] as f32 - margin, SIZE[1] as f32 - margin]
        );
        let caption = page.shell.caption();
        assert!(caption.width() > 100.0 && caption.min == window.min);

        let esc = KitInput {
            keys: vec![Key::Escape],
            ..at([-1.0, -1.0], false)
        };
        assert!(
            !page.frame(esc, SIZE, margin).3,
            "Esc keeps the window open"
        );

        let close = [caption.max[0] + 24.0, caption.min[1] + 24.0];
        let mut closed = false;
        for down in [false, true, false] {
            closed |= page.frame(at(close, down), SIZE, margin).3;
        }
        assert!(closed, "close button");
    }

    /// The whole page on the offscreen GPU target (WARP on Windows, Metal on macOS).
    #[test]
    fn test_page_renders_on_gpu() {
        #[cfg(windows)]
        use poly_ui::d3d11::Offscreen;
        #[cfg(target_os = "macos")]
        use poly_ui::metal::Offscreen;
        let mut page = Page::new(1.0).unwrap();
        let mut target = Offscreen::new(SIZE[0], SIZE[1]).unwrap();
        let margin = page.shadow_margin();
        let mut textures = Vec::new();
        for _ in 0..3 {
            textures.extend(page.frame(at([-1.0, -1.0], false), SIZE, margin).2);
        }
        let (backdrop, list, t, _) = page.frame(at([-1.0, -1.0], false), SIZE, margin);
        textures.extend(t);
        target.clear([0.0; 4]);
        target.paint(&textures, backdrop).unwrap();
        target.paint(&[], list).unwrap();
        let px = target.pixels().unwrap();
        let alpha = |x: u32, y: u32| px[((y * SIZE[0] + x) * 4 + 3) as usize];
        assert_eq!(alpha(1, 1), 0, "transparent outside the shadow");
        assert_eq!(alpha(640, 400), 255, "opaque shell");
        let shadow = alpha(640, SIZE[1] - margin as u32 + 4);
        assert!(shadow > 0 && shadow < 255, "soft shadow below: {shadow}");
    }
}
