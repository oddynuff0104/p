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
use crate::primitives::{Corners, Icon, Shapes};
use crate::theme::{Color, TextStyle};

#[derive(Clone, Debug, PartialEq)]
pub struct WindowState {
    /// Top-left in points; `None` = centred.
    pub pos: Option<[f32; 2]>,
    /// Size in points.
    pub size: [f32; 2],
    pub sidebar_collapsed: bool,
    pub page: usize,
    /// Index into `theme::ACCENTS`.
    pub accent: usize,
}

impl Default for WindowState {
    fn default() -> Self {
        Self {
            pos: None,
            size: [1020.0, 700.0],
            sidebar_collapsed: false,
            page: 0,
            accent: 0,
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
        s += &format!(
            "w={:.0}\nh={:.0}\nsidebar_collapsed={}\npage={}\naccent={}\n",
            self.size[0], self.size[1], self.sidebar_collapsed, self.page, self.accent
        );
        s
    }

    /// Parses [`Self::to_text`]; unknown keys and bad values fall back to defaults.
    pub fn from_text(text: &str) -> Self {
        let mut st = Self::default();
        let (mut x, mut y) = (None, None);
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let f = v.trim().parse::<f32>().ok().filter(|f| f.is_finite());
            match k.trim() {
                "x" => x = f,
                "y" => y = f,
                "w" => st.size[0] = f.unwrap_or(st.size[0]),
                "h" => st.size[1] = f.unwrap_or(st.size[1]),
                "sidebar_collapsed" => st.sidebar_collapsed = v.trim() == "true",
                "page" => st.page = v.trim().parse().unwrap_or(0),
                "accent" => st.accent = v.trim().parse().unwrap_or(0),
                _ => {}
            }
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
        let sh = t.shadow_window;
        let dl = kit.dl();
        dl.shadow(
            win,
            radius,
            t.pxf(sh.blur),
            [0.0, t.pxf(sh.offset[1])],
            sh.color.pm(),
        );
        dl.fill_rounded(win, radius, t.bg.pm());
        dl.stroke_rounded(win, radius, 1.0, t.border.pm());
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
        let cy = bar.min[1] + title_h / 2.0;
        // Logo: accent pill with a dot, then "poly".
        let lx = bar.min[0] + t.sp(5);
        let logo = Rect::new(lx, (cy - t.px(11.0)).round(), t.px(22.0), t.px(22.0));
        let top = t.accent.mix(Color::WHITE, 0.15).pm();
        let bottom = t.accent.mix(Color::BLACK, 0.15).pm();
        kit.dl()
            .gradient_rounded(logo, t.pxf(t.radius.md), top, bottom);
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
        let bg = t.danger.alpha(0.85 * hv).pm();
        kit.dl().fill_rounded(close_r, t.pxf(t.radius.md), bg);
        let ic = t.text_muted.mix(Color::WHITE, hv);
        kit.icon(
            Icon::Close,
            [close_r.min[0] + close_r.width() / 2.0, cy],
            14.0,
            ic,
        );
        kit.focus_ring(hash(wid, "close"), close_r, t.pxf(t.radius.md));
        kit.dl().rect(
            Rect::new(bar.min[0], bar.max[1] - 1.0, bar.width(), 1.0),
            t.border.pm(),
        );

        // ---------------------------------------------------------- sidebar
        let collapsed = self.state.sidebar_collapsed;
        let k = kit.anim_bool(hash(wid, "collapse"), collapsed);
        let side_w = (t.px(196.0) + (t.px(60.0) - t.px(196.0)) * k).round();
        let footer_h = t.px(30.0);
        let side = Rect {
            min: [win.min[0], bar.max[1]],
            max: [win.min[0] + side_w, win.max[1] - footer_h],
        };
        let side_bg = t.bg.mix(Color::BLACK, 0.25).pm();
        kit.dl().fill_rounded(side, 0.0, side_bg);
        kit.dl().rect(
            Rect::new(side.max[0] - 1.0, side.min[1], 1.0, side.height()),
            t.border.pm(),
        );
        let item_h = t.px(38.0);
        let pad = t.sp(2);
        let mut page_changed = false;
        let mut y = side.min[1] + t.sp(3);
        let active_y = side.min[1] + t.sp(3) + item_h * self.state.page as f32;
        let ind_y = kit.anim(hash(wid, "ind"), active_y);
        // Active indicator: tinted pill + accent bar, sliding between items.
        let ind = Rect::new(
            side.min[0] + pad,
            ind_y,
            side_w - 2.0 * pad,
            item_h - t.px(4.0),
        );
        if !nav.is_empty() {
            kit.dl()
                .fill_rounded(ind, t.pxf(t.radius.md), t.accent_soft().pm());
            let bar_r = Rect::new(
                side.min[0] + pad,
                ind_y + t.px(8.0),
                t.px(3.0),
                item_h - t.px(20.0),
            );
            kit.dl().fill_rounded(bar_r, t.px(1.5), t.accent.pm());
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
                .fill_rounded(r, t.pxf(t.radius.md), t.surface_hover.alpha(hv * 0.7).pm());
            let icy = r.min[1] + r.height() / 2.0;
            let ix = r.min[0] + t.px(22.0);
            let c = if active {
                t.accent.mix(Color::WHITE, 0.3)
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
            .fill_rounded(tr, t.pxf(t.radius.md), t.surface_hover.alpha(hv * 0.7).pm());
        let icon = if collapsed {
            Icon::ChevronRight
        } else {
            Icon::ChevronLeft
        };
        let tcy = tr.min[1] + tr.height() / 2.0;
        kit.icon(icon, [tr.min[0] + t.px(22.0), tcy], 14.0, t.text_muted);
        if k < 0.6 {
            let a = 1.0 - k / 0.6;
            kit.text_mid(
                TextStyle::Caption,
                tr.min[0] + t.px(40.0),
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
        let r = t.pxf(t.radius.xl);
        let bg = t.bg.mix(Color::BLACK, 0.25).pm();
        kit.dl().fill_rounded(f, Corners::bottom(r), bg);
        kit.dl()
            .rect(Rect::new(f.min[0], f.min[1], f.width(), 1.0), t.border.pm());
        let cy = f.min[1] + f.height() / 2.0;
        let dot = if ok { t.success } else { t.danger };
        let dx = f.min[0] + t.sp(5);
        kit.dl().circle([dx, cy], t.px(3.5), dot.alpha(0.25).pm());
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
            t.text_faint
        };
        for i in 1..=3 {
            let d = g * i as f32 / 3.5;
            let a = [grip.max[0] - d, grip.max[1]];
            let b = [grip.max[0], grip.max[1] - d];
            kit.dl().line(a, b, 1.0, c.pm());
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
        let st = WindowState {
            pos: Some([120.0, 80.0]),
            size: [900.0, 640.0],
            sidebar_collapsed: true,
            page: 3,
            accent: 2,
        };
        assert_eq!(WindowState::from_text(&st.to_text()), st);
        let bad = WindowState::from_text("w=10\nh=nan\npage=x\njunk");
        assert_eq!(bad.size, [WindowState::MIN[0], 700.0]);
        assert_eq!(bad.pos, None);
    }
}
