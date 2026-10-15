//! Window shell v2: title bar (logo, page title, close), collapsible icon sidebar with an
//! animated active indicator, sticky page header with the global settings search,
//! content area, footer status line, drag to move, corner grip to resize.
//!
//! Position and size are kept in points in [`WindowState`] (DPI independent).
//! Persistence hook: [`Shell::take_changed`] yields the state once after a move, resize,
//! page or sidebar change; the owner stores [`WindowState::to_text`] wherever it keeps
//! settings and passes it back through [`WindowState::from_text`] on start.
//!
//! Hosted ([`Shell::host`]): the shell is the whole OS window. It fills the frame inset by
//! a transparent shadow margin, leaves moving and resizing to the OS (the host answers
//! `WM_NCHITTEST` with [`Shell::caption`] and [`Shell::grip`]) and Esc does not close it.

use super::{Id, Kit, hash};
use crate::draw::Rect;
use crate::primitives::{Icon, Shapes};
use crate::theme::{ACCENTS, Color, GlassKind, Gradient, TextStyle};

#[derive(Clone, Debug, PartialEq)]
pub struct WindowState {
    /// Top-left in points; `None` = centred.
    pub pos: Option<[f32; 2]>,
    /// Size in points.
    pub size: [f32; 2],
    pub sidebar_collapsed: bool,
    pub page: usize,
    /// Accent gradient (Settings › Appearance).
    pub gradient: Gradient,
    /// Real backdrop blur behind the glass.
    pub blur: bool,
}

impl Default for WindowState {
    fn default() -> Self {
        Self {
            pos: None,
            size: [1020.0, 700.0],
            sidebar_collapsed: false,
            page: 0,
            gradient: Gradient::default(),
            blur: true,
        }
    }
}

impl WindowState {
    pub const MIN: [f32; 2] = [640.0, 420.0];

    /// `key=value` lines (stable, human readable).
    pub fn to_text(&self) -> String {
        let mut s = String::new();
        if let Some([x, y]) = self.pos {
            s += &format!("x={x:.0}\ny={y:.0}\n");
        }
        let g = &self.gradient;
        s += &format!(
            "w={:.0}\nh={:.0}\nsidebar_collapsed={}\npage={}\ngradient={}\n\
             gradient_angle={:.0}\nhue_shift={:.0}\nblur={}\n",
            self.size[0],
            self.size[1],
            self.sidebar_collapsed,
            self.page,
            g.stops_text(),
            g.angle,
            g.hue_speed,
            self.blur
        );
        s
    }

    /// Parses [`Self::to_text`]; unknown keys and bad values fall back to defaults.
    /// Files from before gradients (`accent=<index>`) keep their chosen colour as a
    /// single-colour accent; the old default (index 0) becomes the default gradient.
    pub fn from_text(text: &str) -> Self {
        let mut st = Self::default();
        let (mut x, mut y) = (None, None);
        let (mut stops, mut legacy) = (None, None);
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let f = v.trim().parse::<f32>().ok().filter(|f| f.is_finite());
            let g = &mut st.gradient;
            match k.trim() {
                "x" => x = f,
                "y" => y = f,
                "w" => st.size[0] = f.unwrap_or(st.size[0]),
                "h" => st.size[1] = f.unwrap_or(st.size[1]),
                "sidebar_collapsed" => st.sidebar_collapsed = v.trim() == "true",
                "page" => st.page = v.trim().parse().unwrap_or(0),
                "accent" => legacy = v.trim().parse::<usize>().ok(),
                "gradient" => stops = Gradient::parse_stops(v),
                "gradient_angle" => g.angle = f.map_or(g.angle, |a| a.rem_euclid(360.0)),
                "hue_shift" => {
                    g.hue_speed = f.map_or(g.hue_speed, |s| s.clamp(0.0, Gradient::MAX_HUE_SPEED))
                }
                "blur" => st.blur = v.trim() != "false",
                _ => {}
            }
        }
        match (stops, legacy) {
            (Some(stops), _) => st.gradient.set_stops(&stops),
            (None, Some(i)) if i > 0 && i < ACCENTS.len() => {
                let g = Gradient::single(ACCENTS[i].1);
                st.gradient = Gradient {
                    angle: st.gradient.angle,
                    hue_speed: st.gradient.hue_speed,
                    ..g
                };
            }
            _ => {}
        }
        st.pos = x.zip(y).map(|(x, y)| [x, y]);
        st.size = [st.size[0].max(Self::MIN[0]), st.size[1].max(Self::MIN[1])];
        st
    }
}

pub struct NavItem<'a> {
    pub label: &'a str,
    pub icon: Icon,
    pub badge: Option<&'a str>,
}

/// Rectangles of one shell frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct ShellFrame {
    pub window: Rect,
    /// Scrollable content area (below the sticky header).
    pub content: Rect,
    /// Close button clicked (or Esc with nothing else to close).
    pub close: bool,
    /// Sidebar selection changed this frame.
    pub page_changed: bool,
}

#[derive(Default)]
pub struct Shell {
    pub state: WindowState,
    /// Hosted in its own OS window: transparent margin around the shell in pixels.
    pub host: Option<f32>,
    changed: bool,
    drag: Option<[f32; 2]>,
    resize: Option<[f32; 2]>,
    window: Rect,
    caption: Rect,
    grip: Rect,
    footer: Rect,
}

impl Shell {
    pub fn new(state: WindowState) -> Self {
        Self {
            state,
            ..Default::default()
        }
    }

    /// State edited by the owner (accent, sidebar from the settings page).
    pub fn mark_changed(&mut self) {
        self.changed = true;
    }

    /// Window state once after it changed (persistence hook).
    pub fn take_changed(&mut self) -> Option<WindowState> {
        std::mem::take(&mut self.changed).then(|| self.state.clone())
    }

    fn place(&mut self, kit: &Kit) -> Rect {
        let s = kit.theme.scale;
        let screen = kit.size;
        if let Some(m) = self.host {
            let m = m.min(screen[0] / 2.0).min(screen[1] / 2.0);
            return Rect::new(m, m, screen[0] - 2.0 * m, screen[1] - 2.0 * m);
        }
        let w = (self.state.size[0] * s).min(screen[0]).round();
        let h = (self.state.size[1] * s).min(screen[1]).round();
        let [x, y] = match self.state.pos {
            Some([x, y]) => [x * s, y * s],
            None => [(screen[0] - w) / 2.0, (screen[1] - h) / 2.0],
        };
        // Keep at least the title bar reachable.
        let x = x.clamp(0.0, (screen[0] - w).max(0.0)).round();
        let y = y.clamp(0.0, (screen[1] - h).max(0.0)).round();
        Rect::new(x, y, w, h)
    }

    /// Draws the frame; content goes between this and [`Self::end`].
    pub fn begin(
        &mut self,
        kit: &mut Kit,
        page_title: &str,
        nav: &[NavItem],
        search: &mut String,
        search_info: &str,
    ) -> ShellFrame {
        let t = kit.theme.clone();
        let s = t.scale;
        let mut win = self.place(kit);
        let m = kit.mouse();
        // Drag (title bar) and resize (corner grip) from last frame's interaction.
        if let Some(off) = self.drag {
            if kit.mouse_down() {
                let p = [(m[0] - off[0]) / s, (m[1] - off[1]) / s];
                if self.state.pos != Some(p) {
                    self.state.pos = Some(p);
                }
                win = self.place(kit);
            } else {
                self.drag = None;
                self.changed = true;
            }
        }
        if let Some(start) = self.resize {
            if kit.mouse_down() {
                let size = [
                    ((m[0] - win.min[0] + start[0]) / s).max(WindowState::MIN[0]),
                    ((m[1] - win.min[1] + start[1]) / s).max(WindowState::MIN[1]),
                ];
                if self.state.pos.is_none() {
                    self.state.pos = Some([win.min[0] / s, win.min[1] / s]);
                }
                self.state.size = size;
                win = self.place(kit);
            } else {
                self.resize = None;
                self.changed = true;
            }
        }
        self.window = win;
        kit.modal_area = Some(win);
        let radius = t.pxf(t.radius.xl);
        kit.glass(win, radius, GlassKind::Window, 1.0);
        let wid = kit.id("##window");

        // ---------------------------------------------------------- title bar
        let title_h = t.px(48.0);
        let bar = Rect::new(win.min[0], win.min[1], win.width(), title_h);
        let close_r = Rect::new(
            bar.max[0] - t.px(44.0),
            bar.min[1] + t.px(8.0),
            t.px(32.0),
            t.px(32.0),
        );
        let drag_r = Rect {
            min: bar.min,
            max: [close_r.min[0] - t.sp(2), bar.max[1]],
        };
        self.caption = drag_r;
        let drag = kit.interact(hash(wid, "drag"), drag_r, false);
        if drag.pressed && self.host.is_none() {
            self.drag = Some([m[0] - win.min[0], m[1] - win.min[1]]);
        }
        let cy = bar.min[1] + title_h / 2.0 + t.px(2.0);
        // Logo: gradient tile with a glass sheen and a dot, then "poly".
        let lx = bar.min[0] + t.sp(5);
        let logo = Rect::new(lx, (cy - t.px(11.0)).round(), t.px(22.0), t.px(22.0));
        let lr = t.pxf(7.0);
        kit.grad_glow(logo, lr, t.pxf(6.0), 0.35);
        kit.grad_fill(logo, lr, 1.0, logo);
        let sheen = Color::WHITE.alpha(0.28).pm();
        kit.dl().gradient_rounded(logo, lr, sheen, 0);
        kit.lit_rim(logo, lr, 1.0);
        kit.dl().circle(
            [logo.min[0] + logo.width() / 2.0, cy],
            t.px(4.0),
            Color::WHITE.pm(),
        );
        let mut x = logo.max[0] + t.sp(2);
        x += kit.text_mid(TextStyle::Section, x, cy, "poly", t.text);
        x += t.sp(3);
        kit.text_mid(TextStyle::Body, x, cy, "/", t.text_faint);
        x += t.sp(3);
        let max = drag_r.max[0] - x - t.sp(3);
        kit.text_fit(TextStyle::Body, x, cy, page_title, max, t.text_muted);
        let close = kit.interact(hash(wid, "close"), close_r, true);
        let hv = kit.anim_bool(hash(wid, "close_h"), close.hovered);
        let press = kit.spring_bool(hash(wid, "close_p"), close.held);
        let cc = [close_r.min[0] + close_r.width() / 2.0, cy];
        let cr = t.px(13.0) * (1.0 - 0.08 * press);
        kit.dl()
            .circle(cc, cr, Color::WHITE.alpha(0.07 * (1.0 - hv)).pm());
        kit.dl().circle(cc, cr, t.danger.alpha(0.9 * hv).pm());
        let ic = t.text_muted.mix(Color::WHITE, hv);
        kit.icon(Icon::Close, cc, 13.0, ic);
        kit.focus_ring(hash(wid, "close"), close_r, close_r.width() / 2.0);

        // ---------------------------------------------------------- sidebar
        let collapsed = self.state.sidebar_collapsed;
        let k = kit.spring_bool(hash(wid, "collapse"), collapsed);
        let side_w = (t.px(188.0) + (t.px(56.0) - t.px(188.0)) * k).round();
        let footer_h = t.px(30.0);
        let inset = t.sp(2);
        // A floating glass platter inset from the window edges.
        let side = Rect {
            min: [win.min[0] + inset, bar.max[1]],
            max: [win.min[0] + inset + side_w, win.max[1] - footer_h],
        };
        kit.glass(side, t.pxf(t.radius.lg), GlassKind::Platter, 1.0);
        let item_h = t.px(38.0);
        let pad = t.sp(1) + t.px(2.0);
        let mut page_changed = false;
        let mut y = side.min[1] + t.sp(2);
        let active_y = side.min[1] + t.sp(2) + item_h * self.state.page as f32;
        let ind_y = kit.spring(hash(wid, "ind"), active_y);
        // Active indicator: a gradient pill with a lit rim and a soft glow, sliding
        // (springy) between items.
        let ind = Rect::new(
            side.min[0] + pad,
            ind_y,
            side_w - 2.0 * pad,
            item_h - t.px(4.0),
        );
        if !nav.is_empty() {
            let r = t.pxf(t.radius.md);
            kit.grad_glow(ind, r, t.pxf(8.0), 0.22);
            kit.grad_fill(ind, r, 0.42, ind);
            kit.grad_stroke(ind, r, 1.0, 0.75, ind);
            kit.lit_rim(ind.shrink(1.0), r - 1.0, 0.6);
        }
        for (i, item) in nav.iter().enumerate() {
            let r = Rect::new(side.min[0] + pad, y, side_w - 2.0 * pad, item_h - t.px(4.0));
            let id = hash(wid, item.label);
            let resp = kit.interact(id, r, true);
            if resp.activated && self.state.page != i {
                self.state.page = i;
                self.changed = true;
                page_changed = true;
            }
            let active = self.state.page == i;
            let hv = kit.anim_bool(hash(id, "h"), resp.hovered && !active);
            kit.dl()
                .fill_rounded(r, t.pxf(t.radius.md), Color::WHITE.alpha(hv * 0.07).pm());
            let icy = r.min[1] + r.height() / 2.0;
            let ix = r.min[0] + t.px(20.0);
            let c = if active {
                Color::WHITE
            } else {
                t.text_muted.mix(t.text, hv)
            };
            kit.icon(item.icon, [ix, icy], 18.0, c);
            if k < 0.6 {
                let alpha = 1.0 - k / 0.6;
                let tx = ix + t.px(18.0);
                let max = r.max[0] - tx - t.sp(2);
                let tc = if active {
                    t.text
                } else {
                    t.text_muted.mix(t.text, hv)
                };
                kit.text_fit(TextStyle::Body, tx, icy, item.label, max, tc.alpha(alpha));
                if let Some(b) = item.badge {
                    let bw = kit.text_width(TextStyle::Caption, b) + t.px(12.0);
                    kit.badge_at(r.max[0] - bw - t.sp(1), icy, b, t.warning.alpha(alpha));
                }
            } else {
                kit.hint(item.label);
            }
            kit.focus_ring(id, r, t.pxf(t.radius.md));
            y += item_h;
        }
        // Collapse toggle at the bottom of the sidebar.
        let tr = Rect::new(
            side.min[0] + pad,
            side.max[1] - t.px(40.0),
            side_w - 2.0 * pad,
            t.px(32.0),
        );
        let toggle = kit.interact(hash(wid, "collapse_btn"), tr, true);
        if toggle.activated {
            self.state.sidebar_collapsed = !self.state.sidebar_collapsed;
            self.changed = true;
        }
        let hv = kit.anim_bool(hash(wid, "collapse_h"), toggle.hovered);
        kit.dl()
            .fill_rounded(tr, t.pxf(t.radius.md), Color::WHITE.alpha(hv * 0.07).pm());
        let icon = if collapsed {
            Icon::ChevronRight
        } else {
            Icon::ChevronLeft
        };
        let tcy = tr.min[1] + tr.height() / 2.0;
        kit.icon(icon, [tr.min[0] + t.px(20.0), tcy], 14.0, t.text_muted);
        if k < 0.6 {
            let a = (1.0 - k / 0.6).clamp(0.0, 1.0);
            kit.text_mid(
                TextStyle::Caption,
                tr.min[0] + t.px(38.0),
                tcy,
                "Collapse",
                t.text_faint.alpha(a),
            );
        }
        kit.focus_ring(hash(wid, "collapse_btn"), tr, t.pxf(t.radius.md));

        // ---------------------------------------------------------- sticky header
        let cx0 = side.max[0] + t.sp(5);
        let cx1 = win.max[0] - t.sp(5);
        let header_h = t.px(56.0);
        let hy = bar.max[1];
        let hcy = hy + header_h / 2.0 + t.px(2.0);
        let search_w = t.px(280.0).min((cx1 - cx0) * 0.5);
        let sr = Rect::new(
            cx1 - search_w,
            (hcy - t.px(16.0)).round(),
            search_w,
            t.px(32.0),
        );
        let title = if search.is_empty() {
            page_title
        } else {
            "Search"
        };
        let tmax = sr.min[0] - cx0 - t.sp(4);
        kit.text_fit(TextStyle::Title, cx0, hcy, title, tmax, t.text);
        if !search_info.is_empty() {
            let tw = kit.text_width(TextStyle::Title, title).min(tmax);
            let iw = tmax - tw - t.sp(3);
            if iw > t.px(40.0) {
                kit.text_fit(
                    TextStyle::Caption,
                    cx0 + tw + t.sp(3),
                    hcy + t.px(1.0),
                    search_info,
                    iw,
                    t.text_faint,
                );
            }
        }
        let sid = kit.id("##search");
        kit.search_field_at(sid, sr, search, "Search settings…");
        let content = Rect {
            min: [cx0 - t.sp(2), hy + header_h],
            max: [win.max[0] - t.sp(2), win.max[1] - footer_h - 1.0],
        };
        self.footer = Rect::new(win.min[0], win.max[1] - footer_h, win.width(), footer_h);
        // Esc with nothing open and no focused field hides the menu.
        let close_now = close.activated
            || (self.host.is_none()
                && kit.open == 0
                && kit.focus == 0
                && kit.take_key(super::Key::Escape));
        ShellFrame {
            window: win,
            content,
            close: close_now,
            page_changed,
        }
    }

    /// Footer status line and resize grip.
    pub fn end(&mut self, kit: &mut Kit, status: &str, ok: bool, right: &str) {
        let t = kit.theme.clone();
        let f = self.footer;
        let cy = f.min[1] + f.height() / 2.0;
        let dot = if ok { t.success } else { t.danger };
        let dx = f.min[0] + t.sp(5);
        kit.dl().circle([dx, cy], t.px(5.0), dot.alpha(0.18).pm());
        kit.dl().circle([dx, cy], t.px(2.5), dot.pm());
        let rw = kit.text_width(TextStyle::Caption, right);
        let max = f.max[0] - rw - t.px(48.0) - dx;
        kit.text_fit(
            TextStyle::Caption,
            dx + t.sp(3),
            cy,
            status,
            max,
            t.text_muted,
        );
        kit.text_mid(
            TextStyle::Caption,
            f.max[0] - t.px(32.0) - rw,
            cy,
            right,
            t.text_faint,
        );
        // Resize grip.
        let g = t.px(16.0);
        let grip = Rect::new(
            self.window.max[0] - g - t.px(4.0),
            self.window.max[1] - g - t.px(4.0),
            g,
            g,
        );
        self.grip = grip;
        let gid: Id = kit.id("##grip");
        let resp = kit.interact(gid, grip, false);
        if resp.pressed && self.host.is_none() {
            let m = kit.mouse();
            self.resize = Some([self.window.max[0] - m[0], self.window.max[1] - m[1]]);
        }
        // A press on the bare window background is swallowed (nothing beneath reacts),
        // after all widgets had their chance.
        if kit.mouse_pressed && kit.active == 0 && self.window.contains(kit.mouse()) {
            kit.active = kit.id("##window");
        }
        let c = if resp.hovered || self.resize.is_some() {
            t.text_muted
        } else {
            t.text_faint.alpha(0.7)
        };
        // Two short arcs following the window's large corner radius.
        let r = t.pxf(t.radius.xl);
        let corner = [self.window.max[0] - r, self.window.max[1] - r];
        for (i, inset) in [t.pxf(5.0), t.pxf(9.0)].into_iter().enumerate() {
            let rr = r - inset;
            let pts: Vec<[f32; 2]> = (0..=6)
                .map(|k| {
                    let a = (0.12 + 0.26 * k as f32 / 6.0) * std::f32::consts::PI;
                    [corner[0] + rr * a.cos(), corner[1] + rr * a.sin()]
                })
                .collect();
            let w = t.pxf(1.6 - 0.3 * i as f32);
            kit.dl().polyline(&pts, false, w, c.pm());
        }
    }

    pub fn window(&self) -> Rect {
        self.window
    }

    /// Title bar area without its buttons (drag to move) of the last frame.
    pub fn caption(&self) -> Rect {
        self.caption
    }

    /// Resize grip of the last frame.
    pub fn grip(&self) -> Rect {
        self.grip
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_state_round_trips() {
        let mut gradient = Gradient::preset(3);
        gradient.angle = 45.0;
        gradient.hue_speed = 30.0;
        let st = WindowState {
            pos: Some([120.0, 80.0]),
            size: [900.0, 640.0],
            sidebar_collapsed: true,
            page: 3,
            gradient,
            blur: false,
        };
        assert_eq!(WindowState::from_text(&st.to_text()), st);
        let mut single = st.clone();
        single.gradient = Gradient::single(Color::rgb(1, 2, 3));
        assert_eq!(WindowState::from_text(&single.to_text()), single);
        let bad = WindowState::from_text(
            "w=10\nh=nan\npage=x\njunk\ngradient=nope\ngradient_angle=x\nhue_shift=9999",
        );
        assert_eq!(bad.size, [WindowState::MIN[0], 700.0]);
        assert_eq!(bad.pos, None);
        assert_eq!(bad.gradient.stops(), Gradient::default().stops());
        assert_eq!(bad.gradient.angle, Gradient::DEFAULT_ANGLE);
        assert_eq!(bad.gradient.hue_speed, Gradient::MAX_HUE_SPEED);
        assert!(bad.blur);
    }

    /// Files written before gradients: the chosen accent survives as a single colour,
    /// the old default becomes the default gradient.
    #[test]
    fn old_window_files_still_load() {
        let old = "x=100\ny=50\nw=1100\nh=720\nsidebar_collapsed=false\npage=2\naccent=4\n";
        let st = WindowState::from_text(old);
        assert_eq!(
            (st.pos, st.size, st.page),
            (Some([100.0, 50.0]), [1100.0, 720.0], 2)
        );
        assert_eq!(st.gradient.stops(), &[ACCENTS[4].1]);
        assert!(st.blur);
        let default = WindowState::from_text("w=1020\nh=700\npage=0\naccent=0\n");
        assert_eq!(default.gradient, Gradient::default());
    }
}
