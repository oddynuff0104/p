//! UI kit: themed immediate-mode widgets with anti-aliased vector drawing, animations,
//! keyboard focus, popups, toasts and a window shell.
//!
//! Frame: [`Kit::begin`] with a [`KitInput`], widget calls, [`Kit::end`] → one
//! [`DrawList`] (base layer, popups, tooltips/toasts on top) plus texture changes.
//! Hit-testing of popups uses the popup rectangles of the previous frame (as the old `Ui`).

pub mod gallery;
pub mod shell;
pub mod text;
pub mod widgets;

#[cfg(test)]
mod tests;

use std::collections::HashMap;

use crate::draw::{DrawList, Rect, TextureDelta};
use crate::primitives::{Icon, Shapes};
use crate::theme::{Color, TextStyle, Theme};
use text::FontSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Tab,
    Enter,
    Space,
    Escape,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    Backspace,
    Delete,
    /// Ctrl+A.
    SelectAll,
}

/// Input of one frame in physical pixels.
#[derive(Clone, Debug, Default)]
pub struct KitInput {
    pub mouse: [f32; 2],
    /// Left, right button held.
    pub down: [bool; 2],
    /// Wheel notches, positive = up.
    pub wheel: f32,
    pub keys: Vec<Key>,
    pub text: String,
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    /// Raw virtual-key codes pressed this frame (keyboard and mouse), for keybind capture.
    pub pressed_vk: Vec<u32>,
    /// Seconds since the last frame (animations); 0 = first frame.
    pub dt: f32,
}

pub type Id = u64;

pub fn hash(seed: Id, text: &str) -> Id {
    text.bytes().fold(seed ^ 0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100_0000_01b3)
    })
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Resp {
    pub rect: Rect,
    pub hovered: bool,
    /// Mouse went down on it this frame.
    pub pressed: bool,
    /// Mouse held after pressing it.
    pub held: bool,
    /// Released over it after pressing it.
    pub clicked: bool,
    pub focused: bool,
    /// Clicked, or Enter/Space while focused.
    pub activated: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Success,
    Error,
    Info,
}

struct Toast {
    text: String,
    kind: ToastKind,
    age: f32,
}

pub const TOAST_SECONDS: f32 = 4.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Layer {
    Base,
    Overlay,
    Top,
}

pub struct Region {
    pub x0: f32,
    pub x1: f32,
    pub y: f32,
    pub gap: f32,
}

struct Grid {
    x0: f32,
    col_w: f32,
    gap: f32,
    cols: Vec<f32>,
}

struct Card {
    id: Id,
    saved: DrawList,
    rect_x: [f32; 2],
    top: f32,
    column: Option<usize>,
}

struct Collapse {
    id: Id,
    top: f32,
    clipped: bool,
}

struct Scroll {
    id: Id,
    rect: Rect,
    top: f32,
}

/// Per-frame cost numbers.
#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub vertices: usize,
    pub indices: usize,
    pub draw_calls: usize,
}

pub struct Kit {
    pub theme: Theme,
    pub fonts: FontSet,
    textures: Vec<TextureDelta>,
    list: DrawList,
    overlay: DrawList,
    top: DrawList,
    out: DrawList,
    pool: Vec<DrawList>,
    layer: Layer,
    pub input: KitInput,
    prev_down: bool,
    mouse_pressed: bool,
    mouse_released: bool,
    pub size: [f32; 2],
    pub time: f32,
    pub hot: Id,
    pub active: Id,
    pub focus: Id,
    pub focus_visible: bool,
    focus_order: Vec<Id>,
    prev_focus_order: Vec<Id>,
    /// Open popup (dropdown, colour picker, dialog) and the widget rect that owns it.
    pub open: Id,
    pub open_anchor: Rect,
    popup_rects: Vec<Rect>,
    prev_popup_rects: Vec<Rect>,
    pub in_popup: bool,
    modal: bool,
    prev_modal: bool,
    /// Area a modal scrim covers and toasts stack in (the window; screen by default).
    pub modal_area: Option<Rect>,
    anims: HashMap<Id, f32>,
    pub heights: HashMap<Id, f32>,
    scroll: HashMap<Id, f32>,
    ids: Vec<Id>,
    pub regions: Vec<Region>,
    grids: Vec<Grid>,
    cards: Vec<Card>,
    collapses: Vec<Collapse>,
    scrolls: Vec<Scroll>,
    tooltip: Option<(Id, String, Rect)>,
    hover_id: Id,
    hover_t: f32,
    toasts: Vec<Toast>,
    pub caret: usize,
    pub select_all: bool,
    pub capturing: Id,
    pub drag_anchor: Option<(f32, f32)>,
    pub popup_query: String,
    pub popup_highlight: usize,
    pub popup_scroll: f32,
    pub picker_drag: u8,
    pub hex_edit: String,
    pub last_rect: Rect,
    pub last_id: Id,
    pub last_hovered: bool,
    pub stats: Stats,
    /// Record the clickable rect of each labelled widget into [`Self::debug_rects`]
    /// (tests, previews; off in the live menu).
    pub debug_capture: bool,
    pub debug_rects: Vec<(String, Rect)>,
}

impl Kit {
    /// Rasterises the font sizes of `theme`; the atlas upload is in the first frame.
    pub fn new(theme: Theme) -> Result<Self, String> {
        let (fonts, atlas) = FontSet::new(&theme)?;
        Ok(Self {
            theme,
            fonts,
            textures: vec![atlas],
            list: DrawList::default(),
            overlay: DrawList::default(),
            top: DrawList::default(),
            out: DrawList::default(),
            pool: Vec::new(),
            layer: Layer::Base,
            input: KitInput::default(),
            prev_down: false,
            mouse_pressed: false,
            mouse_released: false,
            size: [0.0; 2],
            time: 0.0,
            hot: 0,
            active: 0,
            focus: 0,
            focus_visible: false,
            focus_order: Vec::new(),
            prev_focus_order: Vec::new(),
            open: 0,
            open_anchor: Rect::default(),
            popup_rects: Vec::new(),
            prev_popup_rects: Vec::new(),
            in_popup: false,
            modal: false,
            prev_modal: false,
            modal_area: None,
            anims: HashMap::new(),
            heights: HashMap::new(),
            scroll: HashMap::new(),
            ids: vec![0],
            regions: Vec::new(),
            grids: Vec::new(),
            cards: Vec::new(),
            collapses: Vec::new(),
            scrolls: Vec::new(),
            tooltip: None,
            hover_id: 0,
            hover_t: 0.0,
            toasts: Vec::new(),
            caret: 0,
            select_all: false,
            capturing: 0,
            drag_anchor: None,
            popup_query: String::new(),
            popup_highlight: 0,
            popup_scroll: 0.0,
            picker_drag: 0,
            hex_edit: String::new(),
            last_rect: Rect::default(),
            last_id: 0,
            last_hovered: false,
            stats: Stats::default(),
            debug_capture: false,
            debug_rects: Vec::new(),
        })
    }

    /// Re-rasterises fonts for a new DPI scale (keeps all widget state).
    pub fn set_scale(&mut self, scale: f32) -> Result<(), String> {
        if (self.theme.scale - scale).abs() < 0.01 {
            return Ok(());
        }
        self.theme.scale = scale;
        let (fonts, atlas) = FontSet::new(&self.theme)?;
        self.fonts = fonts;
        self.textures.push(atlas);
        self.heights.clear();
        Ok(())
    }

    // ------------------------------------------------------------------ frame

    pub fn begin(&mut self, input: KitInput, size_px: [u32; 2]) {
        for l in [&mut self.list, &mut self.overlay, &mut self.top] {
            l.clear();
        }
        self.layer = Layer::Base;
        self.input = input;
        self.input.dt = self.input.dt.clamp(0.0, 0.25);
        self.time += self.input.dt;
        self.size = [size_px[0] as f32, size_px[1] as f32];
        let down = self.input.down[0];
        self.mouse_pressed = down && !self.prev_down;
        self.mouse_released = !down && self.prev_down;
        self.prev_down = down;
        self.hot = 0;
        self.tooltip = None;
        self.debug_rects.clear();
        self.prev_popup_rects = std::mem::take(&mut self.popup_rects);
        self.prev_modal = std::mem::take(&mut self.modal);
        self.prev_focus_order = std::mem::take(&mut self.focus_order);
        self.ids.truncate(1);
        self.regions.clear();
        self.grids.clear();
        self.cards.clear();
        self.collapses.clear();
        self.scrolls.clear();
        let full = Rect::new(0.0, 0.0, self.size[0], self.size[1]);
        for l in [&mut self.list, &mut self.overlay, &mut self.top] {
            l.push_clip(full);
        }
        self.regions.push(Region {
            x0: 0.0,
            x1: self.size[0],
            y: 0.0,
            gap: 0.0,
        });
        // Esc: closes the open popup first (keybind capture handles its own Esc).
        if self.capturing == 0 && self.open != 0 && !self.prev_modal && self.take_key(Key::Escape) {
            self.close_popup();
        }
        // Tab / Shift+Tab move the keyboard focus in last frame's order.
        if self.capturing == 0 && self.take_key(Key::Tab) && !self.prev_focus_order.is_empty() {
            let order = &self.prev_focus_order;
            let n = order.len();
            let next = match order.iter().position(|&i| i == self.focus) {
                Some(i) if self.input.shift => (i + n - 1) % n,
                Some(i) => (i + 1) % n,
                None if self.input.shift => n - 1,
                None => 0,
            };
            self.set_focus(order[next]);
            self.focus_visible = true;
        }
    }

    /// Finishes the frame: tooltips and toasts on top, then everything in one list.
    pub fn end(&mut self) -> (&DrawList, Vec<TextureDelta>) {
        self.draw_tooltip();
        self.draw_toasts();
        // Click outside the open popup (and outside the widget that owns it) closes it.
        let m = self.input.mouse;
        if self.mouse_pressed
            && self.open != 0
            && !self.prev_modal
            && !self.popup_rects.iter().any(|r| r.contains(m))
            && !self.open_anchor.contains(m)
        {
            self.close_popup();
        }
        if self.hot != self.hover_id {
            self.hover_id = self.hot;
            self.hover_t = 0.0;
        } else {
            self.hover_t += self.input.dt;
        }
        if !self.input.down[0] {
            self.active = 0;
            self.drag_anchor = None;
            self.picker_drag = 0;
        }
        if self.mouse_pressed && self.hot == 0 && self.open == 0 {
            // Clicking empty space drops the keyboard focus.
            self.focus = 0;
        }
        self.out.clear();
        self.out.append(&self.list);
        self.out.append(&self.overlay);
        self.out.append(&self.top);
        self.stats = Stats {
            vertices: self.out.vertices.len(),
            indices: self.out.indices.len(),
            draw_calls: self.out.commands.len(),
        };
        (&self.out, std::mem::take(&mut self.textures))
    }

    // ------------------------------------------------------------------ ids, input

    pub fn id(&self, label: &str) -> Id {
        hash(*self.ids.last().unwrap_or(&0), label)
    }
    pub fn push_id(&mut self, label: &str) {
        let id = self.id(label);
        self.ids.push(id);
    }
    pub fn pop_id(&mut self) {
        if self.ids.len() > 1 {
            self.ids.pop();
        }
    }

    pub fn mouse(&self) -> [f32; 2] {
        self.input.mouse
    }
    pub fn mouse_down(&self) -> bool {
        self.input.down[0]
    }
    pub fn key(&self, k: Key) -> bool {
        self.input.keys.contains(&k)
    }
    pub fn take_key(&mut self, k: Key) -> bool {
        match self.input.keys.iter().position(|x| *x == k) {
            Some(i) => {
                self.input.keys.remove(i);
                true
            }
            None => false,
        }
    }
    pub fn dt(&self) -> f32 {
        self.input.dt
    }

    pub fn set_focus(&mut self, id: Id) {
        if self.focus != id {
            self.focus = id;
            self.caret = usize::MAX;
            self.select_all = false;
        }
    }

    pub fn open_popup(&mut self, id: Id, anchor: Rect) {
        self.open = id;
        self.open_anchor = anchor;
        self.popup_query.clear();
        self.popup_highlight = 0;
        self.popup_scroll = 0.0;
        self.hex_edit.clear();
    }
    pub fn close_popup(&mut self) {
        self.open = 0;
        self.open_anchor = Rect::default();
        self.popup_query.clear();
    }
    pub fn is_open(&self, id: Id) -> bool {
        self.open == id
    }

    fn blocked(&self, m: [f32; 2]) -> bool {
        if self.in_popup {
            return false;
        }
        self.prev_modal || self.prev_popup_rects.iter().any(|r| r.contains(m))
    }

    /// Hit-test and click logic of one widget; `focusable` joins the Tab order.
    pub fn interact(&mut self, id: Id, rect: Rect, focusable: bool) -> Resp {
        if focusable {
            self.focus_order.push(id);
        }
        let m = self.input.mouse;
        let clip = self.dl().current_clip();
        let hovered = rect.contains(m)
            && clip.contains(m)
            && !self.blocked(m)
            && (self.active == 0 || self.active == id);
        if hovered {
            self.hot = id;
        }
        let pressed = hovered && self.mouse_pressed;
        if pressed {
            self.active = id;
            if focusable {
                self.set_focus(id);
            }
            self.focus_visible = false;
        }
        let held = self.active == id && self.input.down[0];
        let clicked = self.active == id && self.mouse_released && hovered;
        let focused = self.focus == id;
        let key_activate =
            focused && self.focus_visible && (self.key(Key::Enter) || self.key(Key::Space));
        if key_activate {
            self.take_key(Key::Enter);
            self.take_key(Key::Space);
        }
        self.last_rect = rect;
        self.last_id = id;
        self.last_hovered = hovered;
        Resp {
            rect,
            hovered,
            pressed,
            held,
            clicked,
            focused,
            activated: clicked || key_activate,
        }
    }

    /// Records `rect` under `label` while [`Self::debug_capture`] is on.
    pub fn debug(&mut self, label: &str, rect: Rect) {
        if self.debug_capture {
            self.debug_rects.push((label.to_string(), rect));
        }
    }

    // ------------------------------------------------------------------ animation

    /// Value approaching `target` exponentially (frame-rate independent).
    pub fn anim(&mut self, key: Id, target: f32) -> f32 {
        let k = 1.0 - (-self.theme.anim_rate * self.input.dt).exp();
        let v = self.anims.entry(key).or_insert(target);
        *v += (target - *v) * k;
        if (target - *v).abs() < 0.002 {
            *v = target;
        }
        *v
    }
    pub fn anim_bool(&mut self, key: Id, on: bool) -> f32 {
        self.anim(key, if on { 1.0 } else { 0.0 })
    }

    // ------------------------------------------------------------------ drawing

    pub fn dl(&mut self) -> &mut DrawList {
        match self.layer {
            Layer::Base => &mut self.list,
            Layer::Overlay => &mut self.overlay,
            Layer::Top => &mut self.top,
        }
    }

    /// Runs `f` with drawing redirected to the popup layer (no popup blocking for hits).
    pub fn popup_layer<R>(&mut self, rect: Rect, f: impl FnOnce(&mut Self) -> R) -> R {
        let (layer, in_popup) = (self.layer, self.in_popup);
        self.layer = Layer::Overlay;
        self.in_popup = true;
        self.popup_rects.push(rect);
        let r = f(self);
        self.layer = layer;
        self.in_popup = in_popup;
        r
    }
    pub fn mark_modal(&mut self) {
        self.modal = true;
    }

    pub fn font(&self, style: TextStyle) -> &text::SizedFont {
        self.fonts.get(style)
    }
    pub fn text_width(&self, style: TextStyle, s: &str) -> f32 {
        self.fonts.get(style).width(s)
    }
    /// Text with the line box top-left at `pos`.
    pub fn text_at(&mut self, style: TextStyle, pos: [f32; 2], s: &str, color: Color) -> f32 {
        let list = match self.layer {
            Layer::Base => &mut self.list,
            Layer::Overlay => &mut self.overlay,
            Layer::Top => &mut self.top,
        };
        self.fonts.get(style).draw(list, pos, s, color.pm())
    }
    /// Text vertically centred on `cy` (cap-height centring).
    pub fn text_mid(&mut self, style: TextStyle, x: f32, cy: f32, s: &str, color: Color) -> f32 {
        let top = self.fonts.get(style).top_for_center(cy);
        self.text_at(style, [x, top], s, color)
    }
    /// Text cut with "…" to `max` px, centred on `cy`.
    pub fn text_fit(&mut self, style: TextStyle, x: f32, cy: f32, s: &str, max: f32, color: Color) {
        let fitted = self.fonts.get(style).ellipsize(s, max).into_owned();
        self.text_mid(style, x, cy, &fitted, color);
    }
    pub fn icon(&mut self, icon: Icon, center: [f32; 2], size_pt: f32, color: Color) {
        let (s, w) = (self.theme.pxf(size_pt), self.theme.pxf(1.6).max(1.0));
        self.dl().icon(icon, center, s, w, color.pm());
    }
    /// Focus ring around `rect` while keyboard navigation is in use.
    pub fn focus_ring(&mut self, id: Id, rect: Rect, radius: f32) {
        if self.focus == id && self.focus_visible {
            let g = self.theme.pxf(2.0);
            let r = Rect {
                min: [rect.min[0] - g, rect.min[1] - g],
                max: [rect.max[0] + g, rect.max[1] + g],
            };
            let c = self.theme.accent.alpha(0.8).pm();
            let w = self.theme.pxf(1.5);
            self.dl().stroke_rounded(r, radius + g, w, c);
        }
    }

    // ------------------------------------------------------------------ layout

    pub fn region(&mut self) -> &mut Region {
        self.regions.last_mut().expect("root region")
    }
    pub fn push_region(&mut self, x0: f32, x1: f32, y: f32, gap: f32) {
        self.regions.push(Region { x0, x1, y, gap });
    }
    /// Pops the region; returns its final cursor y.
    pub fn pop_region(&mut self) -> f32 {
        if self.regions.len() > 1 {
            self.regions.pop().map_or(0.0, |r| r.y)
        } else {
            self.regions[0].y
        }
    }
    pub fn cursor_y(&self) -> f32 {
        self.regions.last().map_or(0.0, |r| r.y)
    }
    pub fn region_x(&self) -> [f32; 2] {
        self.regions.last().map_or([0.0; 2], |r| [r.x0, r.x1])
    }
    /// Next `h` px of the current region (full width).
    pub fn alloc(&mut self, h: f32) -> Rect {
        let r = self.region();
        let rect = Rect::new(r.x0, r.y, r.x1 - r.x0, h);
        r.y += h + r.gap;
        rect
    }
    pub fn space(&mut self, h: f32) {
        self.region().y += h;
    }
    pub fn indent(&mut self, px: f32) {
        let r = self.region();
        let (x0, x1, y, gap) = (r.x0 + px, r.x1, r.y, r.gap);
        self.push_region(x0, x1, y, gap);
    }
    pub fn unindent(&mut self) {
        let y = self.pop_region();
        self.region().y = y;
    }

    /// Masonry grid of cards: as many columns of at least `min_col` px as fit (1..=3).
    pub fn begin_grid(&mut self, min_col: f32, max_cols: usize) {
        let gap = self.theme.sp(4);
        let r = self.region();
        let (x0, w, y) = (r.x0, r.x1 - r.x0, r.y);
        let cols = (((w + gap) / (min_col + gap)).floor() as usize).clamp(1, max_cols.max(1));
        let col_w = ((w - gap * (cols - 1) as f32) / cols as f32).floor();
        self.grids.push(Grid {
            x0,
            col_w,
            gap,
            cols: vec![y; cols],
        });
    }
    pub fn end_grid(&mut self) {
        if let Some(g) = self.grids.pop() {
            let bottom = g.cols.iter().copied().fold(0.0, f32::max);
            self.region().y = bottom;
        }
    }
    pub fn grid_columns(&self) -> usize {
        self.grids.last().map_or(1, |g| g.cols.len())
    }

    /// Starts a card (rounded surface with shadow) in the current grid column with the
    /// least height, or full width without a grid. Content goes into a region inside.
    pub fn card_begin(&mut self, id: Id) -> Rect {
        let pad = self.theme.sp(4);
        let (x, w, y, column) = match self.grids.last() {
            Some(g) => {
                let (c, y) = g
                    .cols
                    .iter()
                    .copied()
                    .enumerate()
                    .fold((0, f32::MAX), |a, (i, y)| if y < a.1 { (i, y) } else { a });
                (g.x0 + c as f32 * (g.col_w + g.gap), g.col_w, y, Some(c))
            }
            None => {
                let r = self.region();
                (r.x0, r.x1 - r.x0, r.y, None)
            }
        };
        let mut fresh = self.pool.pop().unwrap_or_default();
        fresh.clear();
        fresh.push_clip(self.list.current_clip());
        let saved = std::mem::replace(&mut self.list, fresh);
        self.cards.push(Card {
            id,
            saved,
            rect_x: [x, x + w],
            top: y,
            column,
        });
        let gap = self.theme.sp(2);
        self.push_region(x + pad, x + w - pad, y + pad, gap);
        Rect::new(x, y, w, 0.0)
    }

    pub fn card_end(&mut self) -> Rect {
        let pad = self.theme.sp(4);
        let bottom = self.pop_region() - self.theme.sp(2) + pad;
        let Some(card) = self.cards.pop() else {
            return Rect::default();
        };
        let rect = Rect {
            min: [card.rect_x[0], card.top],
            max: [card.rect_x[1], bottom.max(card.top + pad * 2.0)],
        };
        self.heights.insert(card.id, rect.height());
        let content = std::mem::replace(&mut self.list, card.saved);
        let t = &self.theme;
        let (radius, sh) = (t.pxf(t.radius.lg), t.shadow_card);
        let (blur, off) = (t.pxf(sh.blur), [t.pxf(sh.offset[0]), t.pxf(sh.offset[1])]);
        let (shadow, surface, border, hl) = (
            sh.color.pm(),
            t.surface.pm(),
            t.border.pm(),
            Color::rgba(255, 255, 255, 10).pm(),
        );
        self.list.shadow(rect, radius, blur, off, shadow);
        self.list.fill_rounded(rect, radius, surface);
        self.list.stroke_rounded(rect, radius, 1.0, border);
        // Subtle top highlight: lit edge as on raised glass.
        let hl_rect = Rect {
            min: [rect.min[0] + radius, rect.min[1]],
            max: [rect.max[0] - radius, rect.min[1] + 1.0],
        };
        self.list.rect(hl_rect, hl);
        self.list.append(&content);
        self.pool.push(content);
        match (card.column, self.grids.last_mut()) {
            (Some(c), Some(g)) => g.cols[c] = rect.max[1] + g.gap,
            _ => {
                let gap = self.theme.sp(4);
                self.region().y = rect.max[1] + gap;
            }
        }
        rect
    }

    /// Vertical scroll area: content placed from its top, wheel scrolls, thin scrollbar.
    pub fn begin_scroll(&mut self, label: &str, rect: Rect) {
        let id = self.id(label);
        let target = *self.scroll.get(&id).unwrap_or(&0.0);
        let off = self.anim(hash(id, "smooth"), target).round();
        self.dl().push_clip(rect);
        let gap = self.region().gap;
        self.push_region(rect.min[0], rect.max[0], rect.min[1] - off, gap);
        self.scrolls.push(Scroll {
            id,
            rect,
            top: rect.min[1] - off,
        });
    }
    pub fn end_scroll(&mut self) {
        let end = self.pop_region();
        let Some(s) = self.scrolls.pop() else { return };
        self.dl().pop_clip();
        let content = end - s.top;
        let max = (content - s.rect.height()).max(0.0);
        let m = self.input.mouse;
        let mut off = *self.scroll.get(&s.id).unwrap_or(&0.0);
        if s.rect.contains(m) && !self.blocked(m) && self.input.wheel != 0.0 {
            off -= self.input.wheel * self.theme.px(72.0);
            self.input.wheel = 0.0;
        }
        off = off.clamp(0.0, max);
        self.scroll.insert(s.id, off);
        if max > 0.0 {
            let t = &self.theme;
            let w = t.px(4.0);
            let track = s.rect.height();
            let thumb = (track * track / content).max(t.px(24.0));
            let shown = self
                .anims
                .get(&hash(s.id, "smooth"))
                .copied()
                .unwrap_or(off);
            let y = s.rect.min[1] + (track - thumb) * (shown / max).clamp(0.0, 1.0);
            let r = Rect::new(s.rect.max[0] - w - t.px(3.0), y, w, thumb);
            let c = t.border_strong.pm();
            self.dl().fill_rounded(r, w / 2.0, c);
        }
    }
    pub fn scroll_to_top(&mut self, label: &str) {
        let id = self.id(label);
        self.scroll.insert(id, 0.0);
    }

    // ------------------------------------------------------------------ tooltips, toasts

    /// Tooltip for the last widget while hovered (shown after `theme.tooltip_delay`).
    pub fn hint(&mut self, text: &str) {
        if self.last_hovered && !text.is_empty() {
            self.tooltip = Some((self.last_id, text.to_string(), self.last_rect));
        }
    }

    fn draw_tooltip(&mut self) {
        let Some((id, text, anchor)) = self.tooltip.take() else {
            return;
        };
        if id != self.hover_id || self.hover_t < self.theme.tooltip_delay || self.active != 0 {
            return;
        }
        let fade = ((self.hover_t - self.theme.tooltip_delay) / 0.12).clamp(0.0, 1.0);
        let t = self.theme.clone();
        let pad = [t.sp(3), t.sp(2)];
        let max_w = t.px(300.0).min(self.size[0] * 0.5);
        let font = self.fonts.get(TextStyle::Caption);
        let lines = font.wrap(&text, max_w - 2.0 * pad[0]);
        let lh = (font.line_height * 1.15).round();
        let w = lines.iter().map(|l| font.width(l)).fold(0.0, f32::max) + 2.0 * pad[0];
        let font_lh = font.line_height;
        let h = lh * lines.len() as f32 + 2.0 * pad[1];
        let m = self.input.mouse;
        let mut x = m[0] + t.px(12.0);
        if x + w > self.size[0] {
            x = m[0] - t.px(12.0) - w;
        }
        let mut y = anchor.max[1] + t.px(6.0);
        if y + h > self.size[1] {
            y = anchor.min[1] - t.px(6.0) - h;
        }
        let x = x.clamp(0.0, (self.size[0] - w).max(0.0)).round();
        let y = y.clamp(0.0, (self.size[1] - h).max(0.0)).round();
        let rect = Rect::new(x, y, w, h);
        let r = t.pxf(t.radius.md);
        let layer = self.layer;
        self.layer = Layer::Top;
        let sh = t.shadow_popup;
        self.top.shadow(
            rect,
            r,
            t.pxf(sh.blur),
            [0.0, t.pxf(sh.offset[1])],
            sh.color.alpha(fade).pm(),
        );
        self.top
            .fill_rounded(rect, r, t.surface_raised.alpha(fade).pm());
        self.top
            .stroke_rounded(rect, r, 1.0, t.border_strong.alpha(fade).pm());
        for (i, line) in lines.iter().enumerate() {
            let pos = [
                x + pad[0],
                y + pad[1] + lh * i as f32 + (lh - font_lh) / 2.0,
            ];
            self.text_at(TextStyle::Caption, pos, line, t.text.alpha(fade));
        }
        self.layer = layer;
    }

    pub fn toast(&mut self, text: impl Into<String>, kind: ToastKind) {
        self.toasts.push(Toast {
            text: text.into(),
            kind,
            age: 0.0,
        });
        if self.toasts.len() > 4 {
            self.toasts.remove(0);
        }
    }
    pub fn toast_count(&self) -> usize {
        self.toasts.len()
    }

    fn draw_toasts(&mut self) {
        let dt = self.input.dt;
        for toast in &mut self.toasts {
            toast.age += dt;
        }
        self.toasts.retain(|t| t.age < TOAST_SECONDS);
        if self.toasts.is_empty() {
            return;
        }
        let t = self.theme.clone();
        let area = self
            .modal_area
            .unwrap_or(Rect::new(0.0, 0.0, self.size[0], self.size[1]));
        let w = t.px(300.0).min(area.width() - t.sp(5) * 2.0);
        let h = t.px(44.0);
        let mut y = area.max[1] - t.sp(5) - t.px(28.0);
        let layer = self.layer;
        self.layer = Layer::Top;
        let toasts: Vec<(String, ToastKind, f32)> = self
            .toasts
            .iter()
            .rev()
            .map(|t| (t.text.clone(), t.kind, t.age))
            .collect();
        for (text, kind, age) in toasts {
            let enter = (age / 0.18).clamp(0.0, 1.0);
            let ease = 1.0 - (1.0 - enter).powi(3);
            let fade = ease * ((TOAST_SECONDS - age) / 0.3).clamp(0.0, 1.0);
            let x = area.max[0] - t.sp(5) - w + (1.0 - ease) * t.px(40.0);
            let rect = Rect::new(x.round(), (y - h).round(), w, h);
            let (icon, color) = match kind {
                ToastKind::Success => (Icon::Check, t.success),
                ToastKind::Error => (Icon::Warning, t.danger),
                ToastKind::Info => (Icon::Info, t.accent),
            };
            let r = t.pxf(t.radius.lg);
            let sh = t.shadow_popup;
            self.top.shadow(
                rect,
                r,
                t.pxf(sh.blur),
                [0.0, t.pxf(sh.offset[1])],
                sh.color.alpha(fade).pm(),
            );
            self.top
                .fill_rounded(rect, r, t.surface_raised.alpha(fade).pm());
            self.top
                .stroke_rounded(rect, r, 1.0, t.border_strong.alpha(fade).pm());
            let bar = Rect::new(rect.min[0], rect.min[1], t.px(3.0), h);
            self.top.fill_rounded(
                bar,
                crate::primitives::Corners::left(r),
                color.alpha(fade).pm(),
            );
            let cy = rect.min[1] + h / 2.0;
            let ix = rect.min[0] + t.sp(4) + t.px(8.0);
            self.icon(icon, [ix, cy], 16.0, color.alpha(fade));
            let tx = ix + t.px(8.0) + t.sp(3);
            let max = rect.max[0] - tx - t.sp(3);
            self.text_fit(TextStyle::Body, tx, cy, &text, max, t.text.alpha(fade));
            y -= h + t.sp(2);
        }
        self.layer = layer;
    }
}
