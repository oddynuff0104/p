//! Widgets of the kit. Setting rows put the label left and the control right; every
//! control animates hover/press/toggle state, joins the Tab order and reacts to
//! Enter/Space (and arrows where it makes sense). Popups (dropdown, colour picker,
//! dialog) draw on the popup layer and close with Esc or a click outside.

use super::{Id, Key, Kit, Resp, hash};
use crate::draw::Rect;
use crate::primitives::{Corners, Icon, Shapes};
use crate::theme::{Color, GlassKind, TextStyle};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonKind {
    Primary,
    Secondary,
    Ghost,
    Danger,
}

/// Header of a card.
#[derive(Clone, Copy, Debug, Default)]
pub struct CardHead<'a> {
    pub title: &'a str,
    pub icon: Option<Icon>,
    pub badge: Option<&'a str>,
    pub hint: &'a str,
}

fn center(r: &Rect) -> [f32; 2] {
    [(r.min[0] + r.max[0]) / 2.0, (r.min[1] + r.max[1]) / 2.0]
}

/// Display name of a Windows virtual-key code (0 = none).
pub fn vk_name(vk: u32) -> String {
    match vk {
        0 => "None".into(),
        0x01 => "Mouse 1".into(),
        0x02 => "Mouse 2".into(),
        0x04 => "Mouse 3".into(),
        0x05 => "Mouse 4".into(),
        0x06 => "Mouse 5".into(),
        0x08 => "Backspace".into(),
        0x09 => "Tab".into(),
        0x0D => "Enter".into(),
        0x10 | 0xA0 | 0xA1 => "Shift".into(),
        0x11 | 0xA2 | 0xA3 => "Ctrl".into(),
        0x12 | 0xA4 | 0xA5 => "Alt".into(),
        0x14 => "Caps Lock".into(),
        0x1B => "Esc".into(),
        0x20 => "Space".into(),
        0x21 => "Page Up".into(),
        0x22 => "Page Down".into(),
        0x23 => "End".into(),
        0x24 => "Home".into(),
        0x25 => "Left".into(),
        0x26 => "Up".into(),
        0x27 => "Right".into(),
        0x28 => "Down".into(),
        0x2D => "Insert".into(),
        0x2E => "Delete".into(),
        0x30..=0x39 | 0x41..=0x5A => char::from_u32(vk).map_or("?".into(), |c| c.to_string()),
        0x60..=0x69 => format!("Num {}", vk - 0x60),
        0x70..=0x87 => format!("F{}", vk - 0x6F),
        0xC0 => "`".into(),
        _ => format!("0x{vk:02X}"),
    }
}

impl Kit {
    // ------------------------------------------------------------------ basics

    pub fn label(&mut self, text: &str) {
        let h = self.font(TextStyle::Body).line_height;
        let r = self.alloc(h);
        let c = self.theme.text;
        self.text_at(TextStyle::Body, r.min, text, c);
    }

    /// Muted wrapped paragraph.
    pub fn paragraph(&mut self, text: &str) {
        let [x0, x1] = self.region_x();
        let font = self.font(TextStyle::Caption);
        let lines = font.wrap(text, x1 - x0);
        let lh = (font.line_height * 1.2).round();
        let r = self.alloc(lh * lines.len() as f32);
        let c = self.theme.text_muted;
        for (i, l) in lines.iter().enumerate() {
            self.text_at(
                TextStyle::Caption,
                [r.min[0], r.min[1] + lh * i as f32],
                l,
                c,
            );
        }
    }

    /// Section caption inside a card ("ACCURACY").
    pub fn subheader(&mut self, text: &str) {
        let h = self.theme.px(22.0);
        let r = self.alloc(h);
        let c = self.theme.text_faint;
        let up = text.to_uppercase();
        self.text_mid(
            TextStyle::Caption,
            r.min[0],
            r.max[1] - self.theme.px(8.0),
            &up,
            c,
        );
    }

    pub fn separator(&mut self) {
        let h = self.theme.sp(3);
        let r = self.alloc(h);
        let y = (r.min[1] + h / 2.0).round();
        let c = self.theme.border.pm();
        self.dl().rect(Rect::new(r.min[0], y, r.width(), 1.0), c);
    }

    /// Small pill with text ("beta"); returns its width.
    pub fn badge_at(&mut self, x: f32, cy: f32, text: &str, color: Color) -> f32 {
        let t = &self.theme;
        let (pad, h) = (t.px(6.0), t.px(16.0));
        let w = self.text_width(TextStyle::Caption, text) + 2.0 * pad;
        let r = Rect::new(x, (cy - h / 2.0).round(), w.round(), h);
        let (bg, fg) = (color.alpha(0.16).pm(), color);
        self.dl().fill_rounded(r, h / 2.0, bg);
        self.text_mid(TextStyle::Caption, x + pad, cy, text, fg);
        w
    }

    pub fn button(&mut self, label: &str, kind: ButtonKind) -> bool {
        let t = self.theme.clone();
        let h = t.px(t.control);
        let w = self.text_width(TextStyle::Body, label) + 2.0 * t.sp(4);
        let row = self.alloc(h);
        let r = Rect::new(row.min[0], row.min[1], w.min(row.width()), h);
        self.button_at(label, r, kind)
    }

    pub fn button_at(&mut self, label: &str, r: Rect, kind: ButtonKind) -> bool {
        let id = self.id(label);
        self.debug(label, r);
        let resp = self.interact(id, r, true);
        let t = self.theme.clone();
        let hover = self.anim_bool(hash(id, "h"), resp.hovered);
        // Springy press: the capsule sinks in and pops back.
        let press = self.spring_bool(hash(id, "p"), resp.held);
        let s = (t.pxf(1.5) * press).clamp(-t.pxf(0.6), t.pxf(2.0));
        let r = Rect {
            min: [r.min[0] + s, r.min[1] + s * 0.6],
            max: [r.max[0] - s, r.max[1] - s * 0.6],
        };
        let radius = r.height() / 2.0;
        let fg = match kind {
            ButtonKind::Primary => {
                self.grad_glow(r, radius, t.pxf(5.0 + 5.0 * hover), 0.18 + 0.17 * hover);
                self.grad_fill(r, radius, 1.0, r);
                // Darken a little for white text on bright stops (yellow, mint).
                let shade = Color::BLACK.alpha(0.20 - 0.08 * hover + 0.10 * press.max(0.0));
                self.dl().fill_rounded(r, radius, shade.pm());
                t.on_accent
            }
            ButtonKind::Danger => {
                let bg = t
                    .danger
                    .mix(Color::WHITE, 0.10 * hover)
                    .mix(Color::BLACK, 0.15 * press.max(0.0));
                self.dl().fill_rounded(r, radius, bg.pm());
                Color::WHITE
            }
            ButtonKind::Secondary => {
                let bg = Color::WHITE.alpha(0.08 + 0.06 * hover);
                self.dl().fill_rounded(r, radius, bg.pm());
                t.text
            }
            ButtonKind::Ghost => {
                let bg = Color::WHITE.alpha(0.07 * hover);
                self.dl().fill_rounded(r, radius, bg.pm());
                t.text_muted.mix(t.text, hover)
            }
        };
        if kind != ButtonKind::Ghost {
            let sheen = Color::WHITE.alpha(0.16).pm();
            self.dl().gradient_rounded(r, radius, sheen, 0);
            self.lit_rim(r, radius, 0.8);
        }
        let tw = self.text_width(TextStyle::Body, label);
        let c = center(&r);
        let x = (c[0] - tw / 2.0).round();
        if matches!(kind, ButtonKind::Primary | ButtonKind::Danger) {
            self.text_shadowed(TextStyle::Body, x, c[1], label, fg);
        } else {
            self.text_mid(TextStyle::Body, x, c[1], label, fg);
        }
        self.focus_ring(id, r, radius);
        resp.activated
    }

    /// Text with a soft dark drop (white text on bright gradient stops).
    pub fn text_shadowed(&mut self, style: TextStyle, x: f32, cy: f32, s: &str, color: Color) {
        let o = self.theme.pxf(1.0).max(1.0);
        let shadow = Color::BLACK.alpha(0.38 * color.0[3] as f32 / 255.0);
        self.text_mid(style, x, cy + o, s, shadow);
        self.text_mid(style, x, cy, s, color);
    }

    // ------------------------------------------------------------------ rows

    /// Row of `h` px with label on the left; returns (row rect, control rect right part).
    fn row_label(&mut self, label: &str, h: f32, control_w: f32) -> (Rect, Rect) {
        let row = self.alloc(h);
        let t = &self.theme;
        let cy = row.min[1] + h / 2.0;
        let ctrl = Rect {
            min: [row.max[0] - control_w, row.min[1]],
            max: [row.max[0], row.max[1]],
        };
        let max = (ctrl.min[0] - row.min[0] - t.sp(3)).max(0.0);
        let c = t.text;
        self.text_fit(TextStyle::Body, row.min[0], cy, label, max, c);
        (row, ctrl)
    }

    /// Animated switch in `rect` (track size from the theme, right aligned).
    pub fn switch_at(&mut self, id: Id, area: Rect, value: &mut bool) -> bool {
        let t = self.theme.clone();
        let (w, h) = (t.px(38.0), t.px(22.0));
        let c = center(&area);
        let track = Rect::new(area.max[0] - w, (c[1] - h / 2.0).round(), w, h);
        let resp = self.interact(id, area, true);
        let changed = resp.activated;
        if changed {
            *value = !*value;
        }
        let on = self.anim_bool(hash(id, "on"), *value);
        // Knob travel and press stretch are springs: a little overshoot, never slow.
        let pos = self.spring_bool(hash(id, "pos"), *value).clamp(-0.08, 1.08);
        let stretch = self.spring_bool(hash(id, "s"), resp.held).max(0.0);
        let hover = self.anim_bool(hash(id, "h"), resp.hovered);
        let off_bg = Color::WHITE.alpha(0.13 + 0.06 * hover);
        self.dl().fill_rounded(track, h / 2.0, off_bg.pm());
        if on > 0.0 {
            self.grad_glow(track, h / 2.0, t.pxf(5.0), 0.3 * on * hover);
            self.grad_fill(track, h / 2.0, on, track);
        }
        let knob_r = h / 2.0 - t.px(2.5);
        let extra = t.px(6.0) * stretch;
        let kx = track.min[0] + h / 2.0 + (w - h) * pos;
        // Stretch toward the travel direction while held.
        let (x0, x1) = if *value {
            (kx - knob_r - extra, kx + knob_r)
        } else {
            (kx - knob_r, kx + knob_r + extra)
        };
        let knob = Rect::new(x0, c[1] - knob_r, x1 - x0, knob_r * 2.0);
        let shadow = Color::BLACK.alpha(0.30).pm();
        self.dl()
            .shadow(knob, knob_r, t.pxf(3.0), [0.0, t.pxf(1.0)], shadow);
        self.dl().fill_rounded(knob, knob_r, Color::WHITE.pm());
        self.focus_ring(id, track, h / 2.0);
        changed
    }

    /// Setting row with a toggle switch.
    pub fn toggle(&mut self, label: &str, value: &mut bool) -> bool {
        let id = self.id(label);
        let h = self.theme.px(self.theme.row);
        let (row, _) = self.row_label(label, h, self.theme.px(40.0));
        self.debug(label, row);
        let r = self.switch_at(id, row, value);
        self.last_rect = row;
        r
    }

    pub fn checkbox(&mut self, label: &str, value: &mut bool) -> bool {
        let id = self.id(label);
        let t = self.theme.clone();
        let h = t.px(t.row);
        let row = self.alloc(h);
        let s = t.px(16.0);
        let cy = row.min[1] + h / 2.0;
        let bx = Rect::new(row.min[0], (cy - s / 2.0).round(), s, s);
        self.debug(label, row);
        let resp = self.interact(id, row, true);
        if resp.activated {
            *value = !*value;
        }
        let on = self.anim_bool(hash(id, "on"), *value);
        let pop = self.spring_bool(hash(id, "pop"), *value).max(0.0);
        let hover = self.anim_bool(hash(id, "h"), resp.hovered);
        let r = t.pxf(t.radius.sm);
        let bg = Color::WHITE.alpha(0.10 + 0.05 * hover);
        self.dl().fill_rounded(bx, r, bg.pm());
        self.grad_fill(bx, r, on, bx);
        let border = Color::WHITE.alpha((0.22 + 0.15 * hover) * (1.0 - on));
        self.dl().stroke_rounded(bx, r, 1.0, border.pm());
        self.lit_rim(bx, r, 0.6 * on);
        if on > 0.01 {
            let c = center(&bx);
            let size = s * (0.55 + 0.45 * pop);
            let w = t.pxf(2.0);
            let col = t.on_accent.alpha(on).pm();
            self.dl().icon(Icon::Check, c, size, w, col);
        }
        let x = bx.max[0] + t.sp(3);
        let max = row.max[0] - x;
        self.text_fit(TextStyle::Body, x, cy, label, max, t.text);
        self.focus_ring(id, bx, r);
        resp.activated
    }

    /// Slider row: label and value pill on top, filled track below. Drag (Shift = 10×
    /// finer), click to jump, arrows step (Shift = 10 steps), Home/End to the ends.
    pub fn slider_f32(
        &mut self,
        label: &str,
        value: &mut f32,
        range: (f32, f32),
        step: f32,
        unit: &str,
    ) -> bool {
        let id = self.id(label);
        let t = self.theme.clone();
        let h = t.px(46.0);
        let row = self.alloc(h);
        let (lo, hi) = (range.0.min(range.1), range.0.max(range.1));
        let decimals = decimals(step);
        let text = format!("{:.*}{}", decimals, *value, unit);
        let top_cy = row.min[1] + t.px(13.0);
        let pill_w = self.text_width(TextStyle::Caption, &text) + t.px(14.0);
        let max_label = row.width() - pill_w - t.sp(3);
        self.text_fit(
            TextStyle::Body,
            row.min[0],
            top_cy,
            label,
            max_label,
            t.text,
        );
        let track_y = row.min[1] + t.px(34.0);
        let knob_r = t.px(8.0);
        let track = Rect {
            min: [row.min[0] + knob_r, track_y - t.px(3.0)],
            max: [row.max[0] - knob_r, track_y + t.px(3.0)],
        };
        let hit = Rect {
            min: [row.min[0], track_y - t.px(11.0)],
            max: [row.max[0], track_y + t.px(11.0)],
        };
        self.debug(label, hit);
        let resp = self.interact(id, hit, true);
        let before = *value;
        let span = (hi - lo).max(f32::EPSILON);
        let w = track.width().max(1.0);
        if resp.held {
            let mx = self.input.mouse[0];
            if resp.pressed {
                let jump = !(self.input.shift);
                if jump {
                    *value = lo + ((mx - track.min[0]) / w).clamp(0.0, 1.0) * span;
                }
                self.drag_anchor = Some((mx, *value));
            }
            if let Some((ax, av)) = self.drag_anchor {
                let fine = if self.input.shift { 0.1 } else { 1.0 };
                *value = av + (mx - ax) / w * span * fine;
                if !self.input.shift {
                    // Re-anchor so releasing Shift does not jump.
                    self.drag_anchor = Some((mx, *value));
                } else {
                    self.drag_anchor = Some((ax, av));
                }
            }
        }
        if resp.focused {
            let n = if self.input.shift { 10.0 } else { 1.0 };
            if self.take_key(Key::Left) || self.take_key(Key::Down) {
                *value -= step * n;
            }
            if self.take_key(Key::Right) || self.take_key(Key::Up) {
                *value += step * n;
            }
            if self.take_key(Key::Home) {
                *value = lo;
            }
            if self.take_key(Key::End) {
                *value = hi;
            }
        }
        if *value != before {
            *value = snap(*value, lo, step).clamp(lo, hi);
        }
        let frac = ((*value - lo) / span).clamp(0.0, 1.0);
        // Dragging follows the mouse 1:1; jumps and keys glide with a spring.
        let shown = if resp.held && !resp.pressed {
            self.springs.insert(hash(id, "v"), [frac, 0.0]);
            frac
        } else {
            self.spring(hash(id, "v"), frac).clamp(-0.01, 1.01)
        };
        let hover = self.anim_bool(hash(id, "h"), resp.hovered || resp.held);
        let grab = self.spring_bool(hash(id, "g"), resp.held);
        // Value pill.
        let pill = Rect::new(
            row.max[0] - pill_w,
            (top_cy - t.px(10.0)).round(),
            pill_w,
            t.px(20.0),
        );
        let pill_bg = Color::WHITE.alpha(0.08 + 0.05 * hover);
        self.dl()
            .fill_rounded(pill, pill.height() / 2.0, pill_bg.pm());
        let pc = center(&pill);
        let tw = self.text_width(TextStyle::Caption, &text);
        let vc = t.text.mix(t.accent_text(), hover);
        self.text_mid(
            TextStyle::Caption,
            (pc[0] - tw / 2.0).round(),
            pc[1],
            &text,
            vc,
        );
        // Track: the gradient is laid over the whole track, the fill reveals it.
        let rr = track.height() / 2.0;
        let bg = Color::WHITE.alpha(0.13).pm();
        self.dl().fill_rounded(track, rr, bg);
        let kx = track.min[0] + w * shown;
        let fill = Rect {
            min: track.min,
            max: [kx.max(track.min[0] + rr * 2.0), track.max[1]],
        };
        self.grad_glow(fill, rr, t.pxf(4.0), 0.25 * hover);
        self.grad_fill(fill, rr, 1.0, track);
        // Glass knob: grows when grabbed, gradient halo while hovered.
        let kr = knob_r + t.pxf(2.0) * grab.max(-0.3);
        let knob = Rect::new(kx - kr, track_y - kr, kr * 2.0, kr * 2.0);
        self.grad_glow(knob, kr, t.pxf(6.0) * hover, 0.35 * hover);
        let shadow = Color::BLACK.alpha(0.32).pm();
        self.dl()
            .shadow(knob, kr, t.pxf(3.0), [0.0, t.pxf(1.0)], shadow);
        self.dl().circle([kx, track_y], kr, Color::WHITE.pm());
        self.focus_ring(
            id,
            Rect::new(kx - knob_r, track_y - knob_r, knob_r * 2.0, knob_r * 2.0),
            knob_r,
        );
        self.last_rect = row;
        self.last_id = id;
        self.last_hovered = resp.hovered;
        *value != before
    }

    pub fn slider_i32(
        &mut self,
        label: &str,
        value: &mut i32,
        range: (i32, i32),
        step: i32,
        unit: &str,
    ) -> bool {
        let mut v = *value as f32;
        let changed = self.slider_f32(
            label,
            &mut v,
            (range.0 as f32, range.1 as f32),
            step.max(1) as f32,
            unit,
        );
        let nv = v.round() as i32;
        if changed && nv != *value {
            *value = nv;
            return true;
        }
        false
    }

    /// Segmented control; sits right of the label when it fits, else full width below.
    pub fn segmented(&mut self, label: &str, index: &mut usize, options: &[&str]) -> bool {
        let id = self.id(label);
        let t = self.theme.clone();
        let pad = t.sp(3);
        let need: f32 = options
            .iter()
            .map(|o| self.text_width(TextStyle::Caption, o) + 2.0 * pad)
            .sum::<f32>()
            .max(t.px(80.0));
        let [x0, x1] = self.region_x();
        let label_w = self.text_width(TextStyle::Body, label) + t.sp(4);
        let inline = need + label_w <= x1 - x0;
        let h = t.px(t.control);
        let rh = t.px(t.row);
        let seg = if inline {
            let (row, _) = self.row_label(label, rh, need);
            Rect::new(
                row.max[0] - need,
                (row.min[1] + (rh - h) / 2.0).round(),
                need,
                h,
            )
        } else {
            let (_, _) = self.row_label(label, t.px(22.0), 0.0);
            let r = self.alloc(h);
            Rect::new(r.min[0], r.min[1], r.width(), h)
        };
        self.debug(label, seg);
        let resp = self.interact(id, seg, true);
        let n = options.len().max(1);
        let before = *index;
        let cell = seg.width() / n as f32;
        let m = self.input.mouse;
        if resp.clicked {
            *index = (((m[0] - seg.min[0]) / cell) as usize).min(n - 1);
        }
        if resp.focused {
            if self.take_key(Key::Left) {
                *index = index.saturating_sub(1);
            }
            if self.take_key(Key::Right) {
                *index = (*index + 1).min(n - 1);
            }
        }
        let r = h / 2.0;
        self.dl()
            .fill_rounded(seg, r, Color::WHITE.alpha(0.08).pm());
        let x = self
            .spring(hash(id, "x"), *index as f32)
            .clamp(-0.1, (n - 1) as f32 + 0.1);
        let inset = t.px(2.0);
        let sel = Rect::new(
            (seg.min[0] + cell * x + inset).round(),
            seg.min[1] + inset,
            (cell - 2.0 * inset).round(),
            h - 2.0 * inset,
        );
        // Selection: a gradient capsule (colour by position across the control), shaded
        // a little so white text stays readable on bright stops.
        self.grad_fill(sel, r - inset, 0.9, seg);
        let shade = Color::BLACK.alpha(0.22).pm();
        self.dl().fill_rounded(sel, r - inset, shade);
        let sheen = Color::WHITE.alpha(0.14).pm();
        self.dl().gradient_rounded(sel, r - inset, sheen, 0);
        self.lit_rim(sel, r - inset, 0.7);
        for (i, o) in options.iter().enumerate() {
            let cx = seg.min[0] + cell * (i as f32 + 0.5);
            let max = cell - t.px(6.0);
            let text = self.font(TextStyle::Caption).ellipsize(o, max).into_owned();
            let tw = self.text_width(TextStyle::Caption, &text);
            let hovered = resp.hovered && m[0] >= cx - cell / 2.0 && m[0] < cx + cell / 2.0;
            let near = 1.0 - (x - i as f32).abs().min(1.0);
            let c = t
                .text_muted
                .mix(t.text, hovered as u8 as f32)
                .mix(t.on_accent, near);
            let (tx, ty) = ((cx - tw / 2.0).round(), seg.min[1] + h / 2.0);
            if near > 0.5 {
                self.text_shadowed(TextStyle::Caption, tx, ty, &text, c);
            } else {
                self.text_mid(TextStyle::Caption, tx, ty, &text, c);
            }
        }
        self.focus_ring(id, seg, r);
        self.last_rect = seg;
        *index != before
    }

    /// Dropdown with a popup list; more than 8 options get a search field (type to filter,
    /// arrows move, Enter picks, Esc closes).
    pub fn dropdown(&mut self, label: &str, index: &mut usize, options: &[&str]) -> bool {
        let id = self.id(label);
        let t = self.theme.clone();
        let rh = t.px(t.row);
        let [x0, x1] = self.region_x();
        let w = ((x1 - x0) * 0.5)
            .clamp(t.px(120.0), t.px(220.0))
            .min(x1 - x0);
        let (row, _) = self.row_label(label, rh, w);
        let h = t.px(t.control);
        let field = Rect::new(row.max[0] - w, (row.min[1] + (rh - h) / 2.0).round(), w, h);
        self.debug(label, field);
        self.dropdown_at(id, field, index, options)
    }

    pub fn dropdown_at(
        &mut self,
        id: Id,
        field: Rect,
        index: &mut usize,
        options: &[&str],
    ) -> bool {
        let t = self.theme.clone();
        let resp = self.interact(id, field, true);
        let open = self.is_open(id);
        if resp.activated {
            if open {
                self.close_popup();
            } else {
                self.open_popup(id, field);
                self.popup_highlight = *index;
                self.springs.insert(hash(id, "drop"), [0.0, 0.0]);
            }
        }
        let open = self.is_open(id);
        let hover = self.anim_bool(hash(id, "h"), resp.hovered || open);
        let lit = self.anim_bool(hash(id, "lit"), open);
        let r = t.pxf(t.radius.md);
        let bg = Color::WHITE.alpha(0.07 + 0.05 * hover);
        self.dl().fill_rounded(field, r, bg.pm());
        let border = Color::WHITE.alpha(0.14 * (1.0 - lit));
        self.dl().stroke_rounded(field, r, 1.0, border.pm());
        if lit > 0.0 {
            self.grad_glow(field, r, t.pxf(5.0), 0.25 * lit);
            self.grad_stroke(field, r, t.pxf(1.2), lit, field);
        }
        let cy = field.min[1] + field.height() / 2.0;
        let current = options.get(*index).copied().unwrap_or("–");
        let max = field.width() - t.sp(3) * 2.0 - t.px(16.0);
        self.text_fit(
            TextStyle::Body,
            field.min[0] + t.sp(3),
            cy,
            current,
            max,
            t.text,
        );
        let rot = self.anim_bool(hash(id, "rot"), open);
        let icon = if rot > 0.5 {
            Icon::ChevronUp
        } else {
            Icon::ChevronDown
        };
        self.icon(
            icon,
            [field.max[0] - t.sp(3) - t.px(6.0), cy],
            14.0,
            t.text_muted,
        );
        self.focus_ring(id, field, r);
        self.last_rect = field;
        let mut changed = false;
        if open {
            changed = self.dropdown_popup(id, field, index, options);
        }
        changed
    }

    fn dropdown_popup(&mut self, id: Id, field: Rect, index: &mut usize, options: &[&str]) -> bool {
        let t = self.theme.clone();
        let searchable = options.len() > 8;
        if searchable {
            // Typed text goes to the filter while the popup is open.
            let typed = std::mem::take(&mut self.input.text);
            self.popup_query
                .extend(typed.chars().filter(|c| !c.is_control()));
            if self.take_key(Key::Backspace) {
                self.popup_query.pop();
            }
        }
        let q = self.popup_query.to_lowercase();
        let shown: Vec<usize> = (0..options.len())
            .filter(|&i| q.is_empty() || options[i].to_lowercase().contains(&q))
            .collect();
        let item_h = t.px(28.0);
        let search_h = if searchable { t.px(34.0) } else { 0.0 };
        let pad = t.px(4.0);
        let visible = shown.len().clamp(1, 9) as f32;
        let h = search_h + item_h * visible + 2.0 * pad;
        let w = field.width().max(t.px(180.0));
        let below = field.max[1] + t.px(4.0);
        let y = if below + h > self.size[1] {
            field.min[1] - t.px(4.0) - h
        } else {
            below
        };
        let rect = Rect::new(field.min[0], y.max(0.0), w, h);
        // Opening: a springy drop from the field.
        let drop = self.spring(hash(id, "drop"), 1.0);
        let fade = (0.4 + 0.6 * drop).clamp(0.0, 1.0);
        let lift = (1.0 - drop) * t.px(10.0) * if y < field.min[1] { 1.0 } else { -1.0 };
        let rect = Rect {
            min: [rect.min[0], rect.min[1] + lift],
            max: [rect.max[0], rect.max[1] + lift],
        };
        // Keyboard: arrows move the highlight over the filtered list, Enter picks.
        let mut hl = shown
            .iter()
            .position(|&i| i == self.popup_highlight)
            .unwrap_or(0);
        if self.take_key(Key::Down) {
            hl = (hl + 1).min(shown.len().saturating_sub(1));
        }
        if self.take_key(Key::Up) {
            hl = hl.saturating_sub(1);
        }
        let mut changed = false;
        let mut pick = None;
        if self.take_key(Key::Enter) && !shown.is_empty() {
            pick = Some(shown[hl.min(shown.len() - 1)]);
        }
        let list_h = item_h * visible;
        let total = item_h * shown.len() as f32;
        let max_scroll = (total - list_h).max(0.0);
        // Keep the highlight visible.
        let hl_y = item_h * hl as f32;
        if hl_y < self.popup_scroll {
            self.popup_scroll = hl_y;
        } else if hl_y + item_h > self.popup_scroll + list_h {
            self.popup_scroll = hl_y + item_h - list_h;
        }
        self.popup_layer(rect, |k| {
            let m = k.input.mouse;
            if rect.contains(m) && k.input.wheel != 0.0 {
                k.popup_scroll -= k.input.wheel * item_h * 3.0;
                k.input.wheel = 0.0;
            }
            k.popup_scroll = k.popup_scroll.clamp(0.0, max_scroll);
            let r = t.pxf(t.radius.md) + pad;
            k.glass(rect, r, GlassKind::Popup, fade);
            let r = t.pxf(t.radius.md);
            if searchable {
                let s = Rect::new(
                    rect.min[0] + pad,
                    rect.min[1] + pad,
                    w - 2.0 * pad,
                    search_h - pad,
                );
                k.dl().fill_rounded(s, r, Color::WHITE.alpha(0.08).pm());
                let cy = s.min[1] + s.height() / 2.0;
                k.icon(
                    Icon::Search,
                    [s.min[0] + t.px(14.0), cy],
                    14.0,
                    t.text_muted,
                );
                let x = s.min[0] + t.px(26.0);
                if k.popup_query.is_empty() {
                    k.text_mid(TextStyle::Body, x, cy, "Type to search…", t.text_faint);
                } else {
                    let q = k.popup_query.clone();
                    let adv = k.text_mid(TextStyle::Body, x, cy, &q, t.text);
                    let blink = (k.time * 2.0).fract() < 0.6;
                    if blink {
                        let caret = Rect::new(x + adv + 1.0, cy - t.px(7.0), 1.0, t.px(14.0));
                        k.dl().rect(caret, t.accent_text().pm());
                    }
                }
            }
            let list = Rect::new(rect.min[0], rect.min[1] + pad + search_h, w, list_h);
            k.dl().push_clip(list);
            if shown.is_empty() {
                let cy = list.min[1] + item_h / 2.0;
                k.text_mid(
                    TextStyle::Body,
                    list.min[0] + t.sp(3),
                    cy,
                    "No matches",
                    t.text_faint,
                );
            }
            let first = (k.popup_scroll / item_h) as usize;
            for (row, &i) in shown.iter().enumerate().skip(first).take(10) {
                let iy = list.min[1] + item_h * row as f32 - k.popup_scroll;
                let ir = Rect::new(list.min[0] + pad, iy, w - 2.0 * pad, item_h);
                let iid = hash(id, &format!("item{i}"));
                k.debug(&format!("item:{}", options[i]), ir);
                let resp = k.interact(iid, ir, false);
                if resp.hovered {
                    hl = row;
                }
                if resp.clicked {
                    pick = Some(i);
                }
                let selected = i == *index;
                if row == hl {
                    k.dl().fill_rounded(ir, r, Color::WHITE.alpha(0.10).pm());
                }
                let cy = iy + item_h / 2.0;
                if selected {
                    let at = [ir.max[0] - t.px(14.0), cy];
                    k.icon(Icon::Check, at, 14.0, t.accent_text());
                }
                let color = if selected {
                    t.text
                } else {
                    t.text.mix(t.text_muted, 0.3)
                };
                let max = ir.width() - t.px(36.0);
                k.text_fit(
                    TextStyle::Body,
                    ir.min[0] + t.sp(2),
                    cy,
                    options[i],
                    max,
                    color,
                );
            }
            k.dl().pop_clip();
        });
        self.popup_highlight = shown.get(hl).copied().unwrap_or(self.popup_highlight);
        if let Some(i) = pick {
            changed = *index != i;
            *index = i;
            self.close_popup();
            self.set_focus(id);
        }
        changed
    }

    /// Colour row: hex text and swatch; the popup has an SV square, a hue bar, a hex field
    /// and presets.
    pub fn color_edit(&mut self, label: &str, color: &mut Color, presets: &[Color]) -> bool {
        let id = self.id(label);
        let t = self.theme.clone();
        let rh = t.px(t.row);
        let sw = t.px(40.0);
        let (row, _) = self.row_label(label, rh, sw + t.px(70.0));
        let h = t.px(22.0);
        let swatch = Rect::new(
            row.max[0] - sw,
            (row.min[1] + (rh - h) / 2.0).round(),
            sw,
            h,
        );
        self.debug(label, swatch);
        let hex = color.to_hex();
        let cy = swatch.min[1] + h / 2.0;
        let tw = self.text_width(TextStyle::Caption, &hex);
        self.text_mid(
            TextStyle::Caption,
            swatch.min[0] - t.sp(2) - tw,
            cy,
            &hex,
            t.text_muted,
        );
        let (changed, resp) = self.color_swatch_at(id, swatch, color, presets);
        self.last_rect = row;
        self.last_id = id;
        self.last_hovered = resp.hovered;
        changed
    }

    /// Row of colour swatches (gradient stops): label left, one swatch per colour right,
    /// each opening the colour picker. Returns whether a colour changed.
    pub fn color_row(&mut self, label: &str, colors: &mut [Color], presets: &[Color]) -> bool {
        let id = self.id(label);
        let t = self.theme.clone();
        let rh = t.px(t.row);
        let (sw, gap) = (t.px(28.0), t.px(6.0));
        let n = colors.len().max(1) as f32;
        let w = sw * n + gap * (n - 1.0);
        let (row, _) = self.row_label(label, rh, w);
        let h = t.px(22.0);
        let y = (row.min[1] + (rh - h) / 2.0).round();
        let mut changed = false;
        let mut hovered = false;
        for (i, c) in colors.iter_mut().enumerate() {
            let x = row.max[0] - w + (sw + gap) * i as f32;
            let swatch = Rect::new(x.round(), y, sw, h);
            self.debug(&format!("{label} {}", i + 1), swatch);
            let (ch, resp) = self.color_swatch_at(hash(id, &format!("{i}")), swatch, c, presets);
            changed |= ch;
            hovered |= resp.hovered;
        }
        self.last_rect = row;
        self.last_id = id;
        self.last_hovered = hovered;
        changed
    }

    /// A swatch that opens the colour picker below it; (changed, response).
    pub fn color_swatch_at(
        &mut self,
        id: Id,
        swatch: Rect,
        color: &mut Color,
        presets: &[Color],
    ) -> (bool, Resp) {
        let t = self.theme.clone();
        let resp = self.interact(id, swatch, true);
        if resp.activated {
            if self.is_open(id) {
                self.close_popup();
            } else {
                self.open_popup(id, swatch);
                self.hex_edit = color.to_hex();
            }
        }
        let hover = self.anim_bool(hash(id, "h"), resp.hovered);
        let r = t.pxf(t.radius.sm);
        let open = self.is_open(id);
        let lift = self.spring_bool(hash(id, "lift"), resp.hovered || open);
        let g = t.pxf(1.5) * lift.max(0.0);
        let swatch = Rect {
            min: [swatch.min[0] - g, swatch.min[1] - g],
            max: [swatch.max[0] + g, swatch.max[1] + g],
        };
        let sh = Color::BLACK.alpha(0.3).pm();
        self.dl()
            .shadow(swatch, r, t.pxf(2.0 + 3.0 * lift), [0.0, t.pxf(1.0)], sh);
        self.dl().fill_rounded(swatch, r, color.pm());
        let sheen = Color::WHITE.alpha(0.18).pm();
        self.dl().gradient_rounded(swatch, r, sheen, 0);
        let ring = Color::WHITE.alpha(0.18 + 0.3 * hover);
        self.dl().stroke_rounded(swatch, r, 1.0, ring.pm());
        self.focus_ring(id, swatch, r);
        let changed = self.is_open(id) && self.color_popup(id, swatch, color, presets);
        (changed, resp)
    }

    fn color_popup(&mut self, id: Id, anchor: Rect, color: &mut Color, presets: &[Color]) -> bool {
        let t = self.theme.clone();
        let pad = t.sp(3);
        let sq = t.px(168.0);
        let bar = t.px(14.0);
        let field_h = t.px(26.0);
        let sw = t.px(18.0);
        let per_row = 8usize;
        let preset_rows = presets.len().div_ceil(per_row) as f32;
        let w = pad * 3.0 + sq + bar;
        let h = pad * 4.0 + sq + field_h + preset_rows * (sw + t.px(6.0));
        let mut x = anchor.max[0] - w;
        let mut y = anchor.max[1] + t.px(6.0);
        if y + h > self.size[1] {
            y = anchor.min[1] - t.px(6.0) - h;
        }
        x = x.clamp(0.0, (self.size[0] - w).max(0.0));
        y = y.max(0.0);
        let rect = Rect::new(x.round(), y.round(), w, h);
        let before = *color;
        let [mut hue, mut s, mut v] = color.to_hsv();
        // Keep the hue while saturation/value hit 0 (grey has no hue).
        let hue_key = hash(id, "hue");
        if s <= 0.001 || v <= 0.001 {
            hue = self.anim_hue(hue_key).unwrap_or(hue);
        }
        self.popup_layer(rect, |k| {
            let r = t.pxf(t.radius.lg);
            k.glass(rect, r, GlassKind::Popup, 1.0);
            let sq_r = Rect::new(rect.min[0] + pad, rect.min[1] + pad, sq, sq);
            let bar_r = Rect::new(sq_r.max[0] + pad, sq_r.min[1], bar, sq);
            // SV square: white → hue horizontally, then transparent → black vertically.
            let pure = Color::from_hsv(hue, 1.0, 1.0).pm();
            let white = Color::WHITE.pm();
            k.dl().gradient_rect(sq_r, [white, pure, pure, white]);
            let clear = Color::rgba(0, 0, 0, 0).pm();
            let black = Color::BLACK.pm();
            k.dl().gradient_rect(sq_r, [clear, clear, black, black]);
            k.dl()
                .stroke_rounded(sq_r, 0.0, 1.0, Color::WHITE.alpha(0.16).pm());
            // Hue bar: six gradient segments.
            for i in 0..6 {
                let seg_h = sq / 6.0;
                let a = Color::from_hsv(i as f32 / 6.0, 1.0, 1.0).pm();
                let b = Color::from_hsv((i + 1) as f32 / 6.0, 1.0, 1.0).pm();
                let sr = Rect::new(
                    bar_r.min[0],
                    bar_r.min[1] + seg_h * i as f32,
                    bar,
                    seg_h + 0.5,
                );
                k.dl().gradient_rect(sr, [a, a, b, b]);
            }
            let m = k.input.mouse;
            let sv = k.interact(hash(id, "sv"), sq_r, false);
            let hb = k.interact(hash(id, "hb"), bar_r, false);
            if sv.pressed {
                k.picker_drag = 1;
            }
            if hb.pressed {
                k.picker_drag = 2;
            }
            if k.input.down[0] && k.picker_drag == 1 {
                s = ((m[0] - sq_r.min[0]) / sq).clamp(0.0, 1.0);
                v = 1.0 - ((m[1] - sq_r.min[1]) / sq).clamp(0.0, 1.0);
            }
            if k.input.down[0] && k.picker_drag == 2 {
                hue = ((m[1] - bar_r.min[1]) / sq).clamp(0.0, 0.9999);
            }
            // Markers.
            let mx = sq_r.min[0] + s * sq;
            let my = sq_r.min[1] + (1.0 - v) * sq;
            k.dl()
                .circle_stroke([mx, my], t.px(6.0), t.pxf(2.0), Color::WHITE.pm());
            k.dl()
                .circle_stroke([mx, my], t.px(7.5), 1.0, Color::rgba(0, 0, 0, 120).pm());
            let hy = bar_r.min[1] + hue * sq;
            let hr = Rect::new(bar_r.min[0] - 2.0, hy - 3.0, bar + 4.0, 6.0);
            k.dl().fill_rounded(hr, 3.0, Color::WHITE.pm());
            k.dl()
                .stroke_rounded(hr, 3.0, 1.0, Color::rgba(0, 0, 0, 120).pm());
            // Hex field.
            let fy = sq_r.max[1] + pad;
            let preview = Rect::new(sq_r.min[0], fy, field_h, field_h);
            let fr = Rect::new(
                preview.max[0] + t.sp(2),
                fy,
                w - pad * 2.0 - field_h - t.sp(2),
                field_h,
            );
            let hid = hash(id, "hex");
            k.debug("hex", fr);
            let mut hex = std::mem::take(&mut k.hex_edit);
            let edited = k.text_edit_at(hid, fr, &mut hex, 7, "#RRGGBB");
            if edited && let Some(c) = Color::parse_hex(&hex) {
                [hue, s, v] = c.to_hsv();
            }
            k.hex_edit = hex;
            let current = Color::from_hsv(hue, s, v);
            k.dl()
                .fill_rounded(preview, t.pxf(t.radius.sm), current.pm());
            // Presets.
            let py = fy + field_h + pad;
            for (i, p) in presets.iter().enumerate() {
                let (cx, cy) = ((i % per_row) as f32, (i / per_row) as f32);
                let gap = (w - 2.0 * pad - sw * per_row as f32) / (per_row - 1) as f32;
                let pr = Rect::new(
                    rect.min[0] + pad + cx * (sw + gap),
                    py + cy * (sw + t.px(6.0)),
                    sw,
                    sw,
                );
                let resp = k.interact(hash(id, &format!("preset{i}")), pr, false);
                if resp.clicked {
                    [hue, s, v] = p.to_hsv();
                }
                k.dl().circle(center(&pr), sw / 2.0, p.pm());
                let ring = if resp.hovered {
                    t.text
                } else {
                    t.border_strong
                };
                k.dl().circle_stroke(center(&pr), sw / 2.0, 1.0, ring.pm());
            }
        });
        let next = Color::from_hsv(hue, s, v);
        self.anims.insert(hue_key, hue);
        if next != before && (s > 0.001 || v > 0.001 || next.0 != before.0) {
            *color = Color([next.0[0], next.0[1], next.0[2], before.0[3]]);
            if self.focus != hash(id, "hex") {
                self.hex_edit = color.to_hex();
            }
        }
        *color != before
    }

    fn anim_hue(&self, key: Id) -> Option<f32> {
        self.anims.get(&key).copied()
    }

    /// Keybind capture: click (or Enter), then press a key or mouse button; Esc cancels,
    /// Backspace/Delete clears. `vk` is a Windows virtual-key code (0 = none).
    pub fn keybind(&mut self, label: &str, vk: &mut u32) -> bool {
        let id = self.id(label);
        let t = self.theme.clone();
        let rh = t.px(t.row);
        let capturing = self.capturing == id;
        let text = if capturing {
            "Press a key…".to_string()
        } else {
            vk_name(*vk)
        };
        let w = (self.text_width(TextStyle::Caption, &text) + t.sp(4) * 2.0).max(t.px(72.0));
        let (row, _) = self.row_label(label, rh, w);
        let h = t.px(24.0);
        let pill = Rect::new(row.max[0] - w, (row.min[1] + (rh - h) / 2.0).round(), w, h);
        let before = *vk;
        if capturing {
            // Raw keys first: one press finishes the capture.
            let pressed = std::mem::take(&mut self.input.pressed_vk);
            if self.take_key(Key::Escape) || pressed.contains(&0x1B) {
                self.capturing = 0;
            } else if self.take_key(Key::Backspace)
                || self.take_key(Key::Delete)
                || pressed.iter().any(|&k| k == 0x08 || k == 0x2E)
            {
                *vk = 0;
                self.capturing = 0;
            } else if let Some(&k) = pressed.iter().find(|&&k| k != 0x01) {
                *vk = k;
                self.capturing = 0;
            } else if self.mouse_pressed && !pill.contains(self.input.mouse) {
                self.capturing = 0;
            }
            self.input
                .keys
                .retain(|k| !matches!(k, Key::Enter | Key::Space));
        }
        self.debug(label, pill);
        let resp = self.interact(id, pill, true);
        if resp.activated && self.capturing == 0 && before == *vk && !capturing {
            self.capturing = id;
        }
        let capturing = self.capturing == id;
        let pulse = if capturing {
            0.5 + 0.5 * (self.time * 6.0).sin()
        } else {
            0.0
        };
        let hover = self.anim_bool(hash(id, "h"), resp.hovered);
        let r = h / 2.0;
        let bg = Color::WHITE.alpha(0.08 + 0.05 * hover);
        self.dl().fill_rounded(pill, r, bg.pm());
        if capturing {
            self.grad_glow(pill, r, t.pxf(4.0 + 4.0 * pulse), 0.3);
            self.grad_fill(pill, r, 0.22, pill);
            self.grad_stroke(pill, r, t.pxf(1.2), 0.55 + 0.45 * pulse, pill);
        } else {
            self.dl()
                .stroke_rounded(pill, r, 1.0, Color::WHITE.alpha(0.14).pm());
        }
        let tw = self.text_width(TextStyle::Caption, &text);
        let c = center(&pill);
        let fg = if capturing {
            t.text
        } else if *vk == 0 {
            t.text_faint
        } else {
            t.text
        };
        self.text_mid(
            TextStyle::Caption,
            (c[0] - tw / 2.0).round(),
            c[1],
            &text,
            fg,
        );
        self.focus_ring(id, pill, r);
        self.last_rect = row;
        *vk != before
    }

    // ------------------------------------------------------------------ text

    /// Single-line text editing in `rect` (caret, Backspace/Delete, Home/End, Ctrl+A).
    /// Returns true when the text changed.
    pub fn text_edit_at(
        &mut self,
        id: Id,
        rect: Rect,
        text: &mut String,
        max: usize,
        placeholder: &str,
    ) -> bool {
        self.text_edit_core(id, rect, text, max, placeholder, true)
    }

    /// Text editing; `framed` draws the field background and (gradient) focus border.
    fn text_edit_core(
        &mut self,
        id: Id,
        rect: Rect,
        text: &mut String,
        max: usize,
        placeholder: &str,
        framed: bool,
    ) -> bool {
        let t = self.theme.clone();
        let resp = self.interact(id, rect, true);
        let focused = self.focus == id;
        let before = text.clone();
        if focused {
            let len = text.chars().count();
            if self.caret > len {
                self.caret = len;
            }
            let typed = std::mem::take(&mut self.input.text);
            for ch in typed.chars().filter(|c| !c.is_control()) {
                if self.select_all {
                    text.clear();
                    self.caret = 0;
                    self.select_all = false;
                }
                if text.chars().count() < max {
                    let b = byte_at(text, self.caret);
                    text.insert(b, ch);
                    self.caret += 1;
                }
            }
            let keys = std::mem::take(&mut self.input.keys);
            let mut rest = Vec::new();
            for key in keys {
                let len = text.chars().count();
                match key {
                    Key::SelectAll => self.select_all = true,
                    Key::Backspace | Key::Delete if self.select_all => {
                        text.clear();
                        self.caret = 0;
                        self.select_all = false;
                    }
                    Key::Backspace if self.caret > 0 => {
                        let b = byte_at(text, self.caret - 1);
                        text.remove(b);
                        self.caret -= 1;
                    }
                    Key::Delete if self.caret < len => {
                        let b = byte_at(text, self.caret);
                        text.remove(b);
                    }
                    Key::Left => self.caret = self.caret.saturating_sub(1),
                    Key::Right => self.caret = (self.caret + 1).min(len),
                    Key::Home => self.caret = 0,
                    Key::End => self.caret = len,
                    Key::Enter => self.focus = 0,
                    Key::Backspace | Key::Delete | Key::Space => {}
                    other => rest.push(other),
                }
                if !matches!(key, Key::SelectAll) {
                    self.select_all = self.select_all && matches!(key, Key::Space);
                }
            }
            self.input.keys = rest;
        }
        let hover = self.anim_bool(hash(id, "h"), resp.hovered);
        let f = self.anim_bool(hash(id, "f"), focused);
        let r = t.pxf(t.radius.md);
        if framed {
            let bg = Color::WHITE.alpha(0.06 + 0.03 * hover + 0.03 * f);
            self.dl().fill_rounded(rect, r, bg.pm());
            let border = Color::WHITE.alpha(0.14 * (1.0 - f));
            self.dl().stroke_rounded(rect, r, 1.0, border.pm());
            if f > 0.0 {
                self.grad_glow(rect, r, t.pxf(5.0), 0.28 * f);
                self.grad_stroke(rect, r, t.pxf(1.2), f, rect);
            }
        }
        let cy = rect.min[1] + rect.height() / 2.0;
        let x = rect.min[0] + t.sp(2);
        self.dl().push_clip(rect.shrink(1.0));
        if text.is_empty() && !focused {
            self.text_mid(TextStyle::Body, x, cy, placeholder, t.text_faint);
        } else {
            if self.select_all && focused {
                let w = self.text_width(TextStyle::Body, text);
                let sel = Rect::new(x, cy - t.px(8.0), w, t.px(16.0));
                self.grad_fill(sel, t.pxf(3.0), 0.4, sel);
            }
            self.text_mid(TextStyle::Body, x, cy, text, t.text);
            if focused && (self.time * 2.0).fract() < 0.6 {
                let pre: String = text.chars().take(self.caret).collect();
                let cx = x + self.text_width(TextStyle::Body, &pre);
                self.dl().rect(
                    Rect::new(cx.round(), cy - t.px(7.0), t.pxf(1.5).max(1.0), t.px(14.0)),
                    t.accent_text().pm(),
                );
            }
        }
        self.dl().pop_clip();
        *text != before
    }

    /// Search field with icon, placeholder and a clear button; `rect` is the field.
    pub fn search_field_at(
        &mut self,
        id: Id,
        rect: Rect,
        query: &mut String,
        placeholder: &str,
    ) -> bool {
        let t = self.theme.clone();
        let r = rect.height() / 2.0;
        let icon_w = t.px(28.0);
        let edit = Rect {
            min: [rect.min[0] + icon_w - t.sp(2), rect.min[1]],
            max: [rect.max[0] - icon_w, rect.max[1]],
        };
        let focused = self.focus == id;
        let f = self.anim_bool(hash(id, "f"), focused);
        let inside = rect.contains(self.input.mouse);
        let hover = self.anim_bool(hash(id, "sh"), inside);
        let bg = Color::WHITE.alpha(0.07 + 0.03 * hover + 0.04 * f);
        self.dl().fill_rounded(rect, r, bg.pm());
        let border = Color::WHITE.alpha(0.12 * (1.0 - f));
        self.dl().stroke_rounded(rect, r, 1.0, border.pm());
        if f > 0.0 {
            self.grad_glow(rect, r, t.pxf(6.0), 0.3 * f);
            self.grad_stroke(rect, r, t.pxf(1.2), f, rect);
        }
        self.lit_rim(rect, r, 0.5);
        let cy = rect.min[1] + rect.height() / 2.0;
        let ic = t.text_faint.mix(t.accent_text(), f);
        self.icon(Icon::Search, [rect.min[0] + t.px(15.0), cy], 15.0, ic);
        // The inner editor draws without its own frame.
        let ph = if focused { "" } else { placeholder };
        let changed = self.text_edit_core(id, edit, query, 48, ph, false);
        let mut changed = changed;
        if !query.is_empty() {
            let cr = Rect::new(rect.max[0] - icon_w, rect.min[1], icon_w, rect.height());
            let resp = self.interact(hash(id, "clear"), cr, false);
            let c = if resp.hovered { t.text } else { t.text_muted };
            self.icon(Icon::Close, center(&cr), 12.0, c);
            if resp.clicked {
                query.clear();
                changed = true;
            }
        }
        if focused && self.key(Key::Escape) {
            self.take_key(Key::Escape);
            if query.is_empty() {
                self.focus = 0;
            } else {
                query.clear();
                changed = true;
            }
        }
        changed
    }

    /// Text field row (label left, field right).
    pub fn text_field(&mut self, label: &str, text: &mut String, max: usize) -> bool {
        let id = self.id(label);
        let t = self.theme.clone();
        let rh = t.px(t.row);
        let [x0, x1] = self.region_x();
        let w = ((x1 - x0) * 0.55).max(t.px(120.0)).min(x1 - x0);
        let (row, _) = self.row_label(label, rh, w);
        let h = t.px(t.control);
        let field = Rect::new(row.max[0] - w, (row.min[1] + (rh - h) / 2.0).round(), w, h);
        self.text_edit_at(id, field, text, max, "")
    }

    // ------------------------------------------------------------------ structure

    /// Horizontal tabs with an animated underline; Left/Right when focused.
    pub fn tabs(&mut self, label: &str, index: &mut usize, names: &[&str]) -> bool {
        let id = self.id(label);
        let t = self.theme.clone();
        let h = t.px(34.0);
        let row = self.alloc(h);
        let before = *index;
        self.debug(label, row);
        let resp = self.interact(id, row, true);
        let gap = t.sp(5);
        let mut x = row.min[0];
        let mut spans = Vec::with_capacity(names.len());
        for (i, n) in names.iter().enumerate() {
            let w = self.text_width(TextStyle::Body, n);
            let cell = Rect::new(x - gap / 2.0, row.min[1], w + gap, h);
            if resp.clicked && cell.contains(self.input.mouse) {
                *index = i;
            }
            spans.push((x, w));
            x += w + gap;
        }
        if resp.focused {
            if self.take_key(Key::Left) {
                *index = index.saturating_sub(1);
            }
            if self.take_key(Key::Right) {
                *index = (*index + 1).min(names.len().saturating_sub(1));
            }
        }
        let base = Rect::new(row.min[0], row.max[1] - 1.0, row.width(), 1.0);
        self.dl().rect(base, Color::WHITE.alpha(0.08).pm());
        let m = self.input.mouse;
        for (i, (n, (sx, sw))) in names.iter().zip(&spans).enumerate() {
            let sel = self.anim_bool(hash(id, &format!("t{i}")), i == *index);
            let hovered = resp.hovered && m[0] >= sx - gap / 2.0 && m[0] < sx + sw + gap / 2.0;
            let c = t
                .text_muted
                .mix(t.text, sel.max(hovered as u8 as f32 * 0.6));
            self.text_mid(TextStyle::Body, *sx, row.min[1] + h / 2.0 - 1.0, n, c);
        }
        if let Some(&(sx, sw)) = spans.get(*index) {
            let x = self.spring(hash(id, "x"), sx);
            let w = self.spring(hash(id, "w"), sw).max(t.px(8.0));
            let bar = Rect::new(x, row.max[1] - t.px(3.0), w, t.px(3.0));
            self.grad_glow(bar, t.px(1.5), t.pxf(5.0), 0.3);
            self.grad_fill(bar, t.px(1.5), 1.0, row);
        }
        self.focus_ring(id, row, t.pxf(t.radius.sm));
        *index != before
    }

    /// Card with header (icon, title, badge, optional enable switch). Returns whether the
    /// switch changed. Always close with [`Self::end_card`].
    pub fn card(&mut self, head: CardHead, switch: Option<&mut bool>) -> bool {
        let id = self.id(head.title);
        self.push_id(head.title);
        let t = self.theme.clone();
        let outer = self.card_begin(id);
        let hh = t.px(30.0);
        let r = self.alloc(hh);
        let cy = r.min[1] + hh / 2.0 - t.px(2.0);
        let mut x = r.min[0];
        if let Some(icon) = head.icon {
            let s = t.px(26.0);
            let ib = Rect::new(x, (cy - s / 2.0).round(), s, s);
            let ir = t.pxf(8.0);
            self.grad_fill(ib, ir, 0.85, ib);
            let shade = Color::BLACK.alpha(0.18).pm();
            self.dl().fill_rounded(ib, ir, shade);
            self.lit_rim(ib, ir, 0.9);
            self.icon(icon, center(&ib), 15.0, Color::WHITE);
            x += s + t.sp(3);
        }
        let right = if switch.is_some() { t.px(44.0) } else { 0.0 };
        let max = r.max[0] - x - right;
        let title_w = self.text_width(TextStyle::Section, head.title).min(max);
        self.text_fit(TextStyle::Section, x, cy, head.title, max, t.text);
        if let Some(b) = head.badge {
            self.badge_at(x + title_w + t.sp(2), cy, b, t.warning);
        }
        // Hint on the title.
        let title_r = Rect::new(x, r.min[1], title_w, hh);
        let hid = hash(id, "title");
        self.interact(hid, title_r, false);
        self.hint(head.hint);
        let mut changed = false;
        if let Some(on) = switch {
            let area = Rect::new(r.max[0] - right, r.min[1], right, hh - t.px(4.0));
            self.debug(&format!("{}:switch", head.title), area);
            changed = self.switch_at(hash(id, "switch"), area, on);
            self.last_rect = area;
        }
        let _ = outer;
        let line_y = r.max[1] - t.px(2.0);
        let line = Rect::new(r.min[0], line_y, r.width(), 1.0);
        self.dl().rect(line, Color::WHITE.alpha(0.07).pm());
        self.space(t.sp(1));
        changed
    }

    pub fn end_card(&mut self) -> Rect {
        let r = self.card_end();
        self.pop_id();
        r
    }

    /// Collapsible section header; content follows when it returns true and must be
    /// closed with [`Self::end_collapsible`]. Height animates open/closed.
    pub fn collapsible(&mut self, title: &str, default_open: bool) -> bool {
        let id = self.id(title);
        let t = self.theme.clone();
        let key = hash(id, "open");
        let open = *self
            .heights
            .entry(key)
            .or_insert(if default_open { 1.0 } else { 0.0 })
            > 0.5;
        let h = t.px(28.0);
        let row = self.alloc(h);
        self.debug(title, row);
        let resp = self.interact(id, row, true);
        let mut open = open;
        if resp.activated {
            open = !open;
            self.heights.insert(key, open as u8 as f32);
        }
        let k = self.anim_bool(hash(id, "k"), open);
        let cy = row.min[1] + h / 2.0;
        let hover = self.anim_bool(hash(id, "h"), resp.hovered);
        let c = t.text_muted.mix(t.text, hover);
        let icon = if k > 0.5 {
            Icon::ChevronDown
        } else {
            Icon::ChevronRight
        };
        self.icon(icon, [row.min[0] + t.px(7.0), cy], 14.0, c);
        let up = title.to_uppercase();
        self.text_mid(TextStyle::Caption, row.min[0] + t.px(20.0), cy, &up, c);
        self.focus_ring(id, row, t.pxf(t.radius.sm));
        if k <= 0.001 {
            return false;
        }
        let top = self.cursor_y();
        let full = self
            .heights
            .get(&hash(id, "h_full"))
            .copied()
            .unwrap_or(0.0);
        let clipped = k < 0.999;
        if clipped {
            let [x0, x1] = self.region_x();
            let clip = Rect::new(x0 - t.px(4.0), top, x1 - x0 + t.px(8.0), full * k);
            self.dl().push_clip(clip);
        }
        self.collapses.push(super::Collapse { id, top, clipped });
        true
    }

    pub fn end_collapsible(&mut self) {
        let Some(c) = self.collapses.pop() else {
            return;
        };
        let full = self.cursor_y() - c.top;
        self.heights.insert(hash(c.id, "h_full"), full);
        if c.clipped {
            self.dl().pop_clip();
            let k = self.anims.get(&hash(c.id, "k")).copied().unwrap_or(1.0);
            self.region().y = c.top + full * k;
        }
    }

    /// Modal confirm dialog for `label` (open it with [`Self::open_confirm`]). Returns
    /// `Some(true)` on confirm (button or Enter), `Some(false)` on cancel (button, Esc).
    pub fn confirm(
        &mut self,
        label: &str,
        title: &str,
        body: &str,
        confirm: &str,
        danger: bool,
    ) -> Option<bool> {
        let id = self.id(label);
        if !self.is_open(id) {
            return None;
        }
        let t = self.theme.clone();
        self.mark_modal();
        let area = self
            .modal_area
            .unwrap_or(Rect::new(0.0, 0.0, self.size[0], self.size[1]));
        let w = t.px(380.0).min(area.width() - t.sp(5));
        let font = self.font(TextStyle::Body);
        let lines = font.wrap(body, w - t.sp(5) * 2.0);
        let lh = (font.line_height * 1.25).round();
        let h = t.sp(5) * 2.0 + t.px(28.0) + lh * lines.len() as f32 + t.sp(5) + t.px(t.control);
        let rect = Rect::new(
            (area.min[0] + (area.width() - w) / 2.0).round(),
            (area.min[1] + (area.height() - h) / 2.0).round(),
            w,
            h,
        );
        let mut result = None;
        if self.take_key(Key::Escape) {
            result = Some(false);
        } else if self.take_key(Key::Enter) {
            result = Some(true);
        }
        let shown = self.spring(hash(id, "in"), 1.0);
        self.popup_layer(area, |k| {
            let scrim = t.scrim.alpha(shown.clamp(0.0, 1.0));
            k.dl().fill_rounded(area, t.pxf(t.radius.xl), scrim.pm());
            let lift = (1.0 - shown) * t.px(16.0);
            let rect = Rect {
                min: [rect.min[0], rect.min[1] + lift],
                max: [rect.max[0], rect.max[1] + lift],
            };
            let r = t.pxf(t.radius.xl);
            k.glass(rect, r, GlassKind::Popup, (0.3 + shown).clamp(0.0, 1.0));
            let pad = t.sp(5);
            let (icon, ic) = if danger {
                (Icon::Warning, t.danger)
            } else {
                (Icon::Info, t.accent_text())
            };
            let ty = rect.min[1] + pad + t.px(12.0);
            k.icon(icon, [rect.min[0] + pad + t.px(9.0), ty], 18.0, ic);
            k.text_mid(
                TextStyle::Section,
                rect.min[0] + pad + t.px(28.0),
                ty,
                title,
                t.text,
            );
            let by = rect.min[1] + pad + t.px(28.0);
            for (i, l) in lines.iter().enumerate() {
                k.text_at(
                    TextStyle::Body,
                    [rect.min[0] + pad, by + lh * i as f32],
                    l,
                    t.text_muted,
                );
            }
            let bh = t.px(t.control);
            let bw = t.px(96.0);
            let y = rect.max[1] - pad - bh;
            let ok = Rect::new(rect.max[0] - pad - bw, y, bw, bh);
            let cancel = Rect::new(ok.min[0] - t.sp(2) - bw, y, bw, bh);
            k.push_id(label);
            if k.button_at("Cancel", cancel, ButtonKind::Secondary) {
                result = Some(false);
            }
            let kind = if danger {
                ButtonKind::Danger
            } else {
                ButtonKind::Primary
            };
            if k.button_at(confirm, ok, kind) {
                result = Some(true);
            }
            k.pop_id();
        });
        if result.is_some() {
            self.close_popup();
            self.springs.remove(&hash(id, "in"));
        }
        result
    }

    pub fn open_confirm(&mut self, label: &str) {
        let id = self.id(label);
        self.open_popup(id, Rect::default());
        self.springs.insert(hash(id, "in"), [0.0, 0.0]);
    }

    /// Response of the last widget (tests, tooling).
    pub fn last(&self) -> Resp {
        Resp {
            rect: self.last_rect,
            hovered: self.last_hovered,
            ..Default::default()
        }
    }
}

fn decimals(step: f32) -> usize {
    if step >= 1.0 {
        0
    } else if step >= 0.1 {
        1
    } else if step >= 0.01 {
        2
    } else {
        3
    }
}

fn snap(v: f32, lo: f32, step: f32) -> f32 {
    if step <= 0.0 {
        return v;
    }
    let s = ((v - lo) / step).round() * step + lo;
    (s * 1e5).round() / 1e5
}

fn byte_at(s: &str, chars: usize) -> usize {
    s.char_indices().nth(chars).map_or(s.len(), |(b, _)| b)
}

/// Corners helper re-exported for shell code.
pub fn corners_top(r: f32) -> Corners {
    Corners::top(r)
}
