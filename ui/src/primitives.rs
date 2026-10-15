//! Anti-aliased vector primitives on top of [`DrawList`]: convex fills with a feathered
//! fringe (1 px = anti-aliasing, wider = soft shadow), rounded rectangles with per-corner
//! radii, circles, polylines, gradients and a small built-in icon set.
//!
//! Anti-aliasing is geometric (imgui style): every shape gets an extra ring of vertices
//! whose colour fades to transparent over the fringe width. No MSAA, no shader change, the
//! white texel of the font atlas is the only texture, so shapes and text share draw calls.
//! All coordinates are physical pixels; shapes are wound clockwise in screen space.

use std::cell::RefCell;
use std::f32::consts::{PI, TAU};

use super::draw::{DrawList, FONT_TEXTURE, Rect, Vertex, WHITE_UV};

type P = [f32; 2];

thread_local! {
    /// Scratch paths (no allocation per shape after warm-up).
    static SCRATCH: RefCell<(Vec<P>, Vec<P>)> = const { RefCell::new((Vec::new(), Vec::new())) };
}

/// Per-corner radii: top-left, top-right, bottom-right, bottom-left.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Corners(pub [f32; 4]);

impl Corners {
    pub const fn all(r: f32) -> Self {
        Self([r; 4])
    }
    pub const fn top(r: f32) -> Self {
        Self([r, r, 0.0, 0.0])
    }
    fn clamped(self, rect: &Rect) -> Self {
        let m = (rect.width().min(rect.height()) / 2.0).max(0.0);
        Self(self.0.map(|r| r.clamp(0.0, m)))
    }
}

impl From<f32> for Corners {
    fn from(r: f32) -> Self {
        Self::all(r)
    }
}

/// Built-in icons, drawn as strokes in a 24×24 design box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Crosshair,
    Shield,
    Eye,
    Palette,
    Gear,
    User,
    Run,
    Search,
    ChevronDown,
    ChevronRight,
    ChevronLeft,
    ChevronUp,
    Check,
    Close,
    Info,
    Warning,
    Menu,
    Target,
    Keyboard,
    Grid,
}

impl Icon {
    pub const ALL: [Icon; 20] = [
        Icon::Crosshair,
        Icon::Shield,
        Icon::Eye,
        Icon::Palette,
        Icon::Gear,
        Icon::User,
        Icon::Run,
        Icon::Search,
        Icon::ChevronDown,
        Icon::ChevronRight,
        Icon::ChevronLeft,
        Icon::ChevronUp,
        Icon::Check,
        Icon::Close,
        Icon::Info,
        Icon::Warning,
        Icon::Menu,
        Icon::Target,
        Icon::Keyboard,
        Icon::Grid,
    ];

    /// Icon for a `features::layout::Page::icon` name.
    pub fn from_name(name: &str) -> Icon {
        match name {
            "crosshair" | "rage" => Icon::Crosshair,
            "shield" => Icon::Shield,
            "eye" | "visuals" => Icon::Eye,
            "palette" | "skins" => Icon::Palette,
            "settings" | "gear" => Icon::Gear,
            "user" => Icon::User,
            "run" | "movement" => Icon::Run,
            "search" => Icon::Search,
            "target" | "misc" => Icon::Target,
            "keyboard" => Icon::Keyboard,
            _ => Icon::Grid,
        }
    }
}

fn arc(out: &mut Vec<P>, c: P, r: f32, a0: f32, a1: f32) {
    if r <= 0.0 {
        out.push(c);
        return;
    }
    // Segment count from the arc length: ~3 px per segment, at least 2.
    let n = ((r * (a1 - a0).abs() / 3.0).ceil() as usize).clamp(2, 24);
    for i in 0..=n {
        let a = a0 + (a1 - a0) * i as f32 / n as f32;
        out.push([c[0] + r * a.cos(), c[1] + r * a.sin()]);
    }
}

/// Clockwise outline of a rounded rectangle.
pub fn rounded_path(out: &mut Vec<P>, rect: Rect, corners: Corners) {
    let [tl, tr, br, bl] = corners.clamped(&rect).0;
    let [x0, y0] = rect.min;
    let [x1, y1] = rect.max;
    out.clear();
    arc(out, [x0 + tl, y0 + tl], tl, PI, 1.5 * PI);
    arc(out, [x1 - tr, y0 + tr], tr, 1.5 * PI, TAU);
    arc(out, [x1 - br, y1 - br], br, 0.0, 0.5 * PI);
    arc(out, [x0 + bl, y1 - bl], bl, 0.5 * PI, PI);
    // Pills: neighbouring arcs meet in one point; zero-length edges would skew normals.
    let close = |a: P, b: P| (a[0] - b[0]).abs() < 0.01 && (a[1] - b[1]).abs() < 0.01;
    out.dedup_by(|b, a| close(*a, *b));
    while out.len() > 1 && close(out[0], out[out.len() - 1]) {
        out.pop();
    }
}

fn circle_path(out: &mut Vec<P>, c: P, r: f32) {
    out.clear();
    let n = ((r * TAU / 3.0).ceil() as usize).clamp(8, 64);
    for i in 0..n {
        let a = TAU * i as f32 / n as f32;
        out.push([c[0] + r * a.cos(), c[1] + r * a.sin()]);
    }
}

fn norm(v: P) -> P {
    let l = (v[0] * v[0] + v[1] * v[1]).sqrt();
    if l > 1e-6 {
        [v[0] / l, v[1] / l]
    } else {
        [0.0, 0.0]
    }
}

/// Averaged outward normals of a closed clockwise polygon (scaled for constant width).
fn vertex_normals(pts: &[P], out: &mut Vec<P>) {
    out.clear();
    let n = pts.len();
    let edge = |i: usize| {
        let (a, b) = (pts[i], pts[(i + 1) % n]);
        let d = norm([b[0] - a[0], b[1] - a[1]]);
        [d[1], -d[0]]
    };
    let mut prev = edge(n - 1);
    for i in 0..n {
        let next = edge(i);
        let mut m = [(prev[0] + next[0]) * 0.5, (prev[1] + next[1]) * 0.5];
        let d2 = m[0] * m[0] + m[1] * m[1];
        if d2 > 1e-6 {
            let inv = (1.0 / d2).min(4.0);
            m = [m[0] * inv, m[1] * inv];
        }
        out.push(m);
        prev = next;
    }
}

fn vertex(pos: P, color: u32) -> Vertex {
    Vertex {
        pos,
        uv: WHITE_UV,
        color,
    }
}

fn premul_scale(c: u32, f: f32) -> u32 {
    let f = f.clamp(0.0, 1.0);
    let ch = |s: u32| (((c >> s) & 0xFF) as f32 * f).round() as u32;
    ch(0) | ch(8) << 8 | ch(16) << 16 | ch(24) << 24
}

/// Linear blend of two premultiplied colours.
pub fn lerp_color(a: u32, b: u32, t: f32) -> u32 {
    let t = t.clamp(0.0, 1.0);
    let ch = |s: u32| {
        let (x, y) = (((a >> s) & 0xFF) as f32, ((b >> s) & 0xFF) as f32);
        (x + (y - x) * t).round() as u32
    };
    ch(0) | ch(8) << 8 | ch(16) << 16 | ch(24) << 24
}

/// Shapes on a [`DrawList`].
pub trait Shapes {
    /// Convex polygon (clockwise) with a fringe of `feather` px fading out; the fringe
    /// straddles the outline (half inside, half outside) so 1 px gives exact AA coverage.
    fn fill_convex(&mut self, pts: &[P], feather: f32, color: &dyn Fn(P) -> u32);
    fn fill_rounded(&mut self, rect: Rect, corners: impl Into<Corners>, color: u32);
    /// Border of `width` px inside `rect`.
    fn stroke_rounded(&mut self, rect: Rect, corners: impl Into<Corners>, width: f32, color: u32);
    /// Vertical gradient (top → bottom colour) in a rounded rect.
    fn gradient_rounded(&mut self, rect: Rect, corners: impl Into<Corners>, top: u32, bottom: u32);
    /// Axis-aligned quad with one colour per corner (tl, tr, br, bl); no AA (inner fills).
    fn gradient_rect(&mut self, rect: Rect, colors: [u32; 4]);
    fn circle(&mut self, center: P, r: f32, color: u32);
    fn circle_stroke(&mut self, center: P, r: f32, width: f32, color: u32);
    /// Anti-aliased polyline of `width` px (round-ish joins via miter clamp).
    fn polyline(&mut self, pts: &[P], closed: bool, width: f32, color: u32);
    fn line(&mut self, a: P, b: P, width: f32, color: u32) {
        self.polyline(&[a, b], false, width, color);
    }
    /// Soft shadow of a rounded rect: solid core, `blur` px falloff, shifted by `offset`.
    fn shadow(&mut self, rect: Rect, radius: f32, blur: f32, offset: P, color: u32);
    /// `icon` centred in a square of `size` px with stroke `width` px.
    fn icon(&mut self, icon: Icon, center: P, size: f32, width: f32, color: u32);
    /// Polyline whose colour depends on the point and its outward unit normal (lit rims).
    fn polyline_lit(&mut self, pts: &[P], closed: bool, width: f32, color: &dyn Fn(P, P) -> u32);
    /// Border like [`Self::stroke_rounded`] with a per-point colour `color(p, normal)`.
    fn stroke_rounded_lit(
        &mut self,
        rect: Rect,
        corners: impl Into<Corners>,
        width: f32,
        color: &dyn Fn(P, P) -> u32,
    );
    /// Soft band along the outline of a rounded rect from offset `d0` to `d1` px along the
    /// outward normal (negative = inside), colour `c0` fading to `c1` (inner glow, halo).
    fn band_rounded(
        &mut self,
        rect: Rect,
        corners: impl Into<Corners>,
        d: [f32; 2],
        c0: u32,
        c1: u32,
    );
    /// Replaces the UVs of every vertex from index `start` on (glass, gradients).
    fn uv_from(&mut self, start: usize, uv: &dyn Fn(P) -> [f32; 2]);
}

/// Linear gradient mapping: `t = dot(p - origin, dir)` (0 at one side of the span,
/// 1 at the other), CSS-style angle (0° = left → right, 90° = top → bottom).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GradMap {
    origin: P,
    dir: P,
}

impl GradMap {
    /// Gradient across `span` at `angle_deg`.
    pub fn over(span: Rect, angle_deg: f32) -> Self {
        let a = angle_deg.to_radians();
        let d = [a.cos(), a.sin()];
        let c = [
            (span.min[0] + span.max[0]) / 2.0,
            (span.min[1] + span.max[1]) / 2.0,
        ];
        let half = (span.width() / 2.0 * d[0].abs() + span.height() / 2.0 * d[1].abs()).max(0.5);
        let dir = [d[0] / (2.0 * half), d[1] / (2.0 * half)];
        Self {
            origin: [c[0] - d[0] * half, c[1] - d[1] * half],
            dir,
        }
    }
    pub fn t(&self, p: P) -> f32 {
        (p[0] - self.origin[0]) * self.dir[0] + (p[1] - self.origin[1]) * self.dir[1]
    }
}

impl Shapes for DrawList {
    fn fill_convex(&mut self, pts: &[P], feather: f32, color: &dyn Fn(P) -> u32) {
        let n = pts.len();
        if n < 3 {
            return;
        }
        let mut normals = Vec::new();
        SCRATCH.with(|s| {
            if let Ok(mut s) = s.try_borrow_mut() {
                normals = std::mem::take(&mut s.1);
            }
        });
        vertex_normals(pts, &mut normals);
        let half = feather * 0.5;
        // Inner ring at full colour, outer ring transparent.
        let base = self.vertices.len() as u32;
        let add = (n - 2) * 3 + n * 6;
        self.begin_shapes(add as u32);
        for (p, m) in pts.iter().zip(&normals) {
            let inner = [p[0] - m[0] * half, p[1] - m[1] * half];
            let outer = [p[0] + m[0] * half, p[1] + m[1] * half];
            let c = color(*p);
            self.vertices.push(vertex(inner, c));
            self.vertices.push(vertex(outer, premul_scale(c, 0.0)));
        }
        for i in 2..n as u32 {
            self.indices
                .extend([base, base + 2 * (i - 1), base + 2 * i]);
        }
        for i in 0..n as u32 {
            let j = (i + 1) % n as u32;
            let (ii, io, ji, jo) = (
                base + 2 * i,
                base + 2 * i + 1,
                base + 2 * j,
                base + 2 * j + 1,
            );
            self.indices.extend([ii, io, jo, ii, jo, ji]);
        }
        SCRATCH.with(|s| {
            if let Ok(mut s) = s.try_borrow_mut() {
                s.1 = normals;
            }
        });
    }

    fn fill_rounded(&mut self, rect: Rect, corners: impl Into<Corners>, color: u32) {
        if color >> 24 == 0 || rect.width() <= 0.0 || rect.height() <= 0.0 {
            return;
        }
        with_path(|path| {
            rounded_path(path, rect, corners.into());
            self.fill_convex(path, 1.0, &|_| color);
        });
    }

    fn stroke_rounded(&mut self, rect: Rect, corners: impl Into<Corners>, width: f32, color: u32) {
        if color >> 24 == 0 {
            return;
        }
        let h = width * 0.5;
        let inner = rect.shrink(h);
        let c = corners.into();
        let c = Corners(c.0.map(|r| (r - h).max(0.0)));
        with_path(|path| {
            rounded_path(path, inner, c);
            self.polyline(path, true, width, color);
        });
    }

    fn gradient_rounded(&mut self, rect: Rect, corners: impl Into<Corners>, top: u32, bottom: u32) {
        let (y0, h) = (rect.min[1], rect.height().max(1.0));
        with_path(|path| {
            rounded_path(path, rect, corners.into());
            self.fill_convex(path, 1.0, &|p| lerp_color(top, bottom, (p[1] - y0) / h));
        });
    }

    fn gradient_rect(&mut self, rect: Rect, colors: [u32; 4]) {
        let base = self.vertices.len() as u32;
        self.begin_shapes(6);
        self.vertices.extend([
            vertex(rect.min, colors[0]),
            vertex([rect.max[0], rect.min[1]], colors[1]),
            vertex(rect.max, colors[2]),
            vertex([rect.min[0], rect.max[1]], colors[3]),
        ]);
        self.indices
            .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    fn circle(&mut self, center: P, r: f32, color: u32) {
        if color >> 24 == 0 || r <= 0.0 {
            return;
        }
        with_path(|path| {
            circle_path(path, center, r);
            self.fill_convex(path, 1.0, &|_| color);
        });
    }

    fn circle_stroke(&mut self, center: P, r: f32, width: f32, color: u32) {
        with_path(|path| {
            circle_path(path, center, r);
            self.polyline(path, true, width, color);
        });
    }

    fn polyline(&mut self, pts: &[P], closed: bool, width: f32, color: u32) {
        if color >> 24 == 0 {
            return;
        }
        self.polyline_lit(pts, closed, width, &|_, _| color);
    }

    fn polyline_lit(&mut self, pts: &[P], closed: bool, width: f32, color: &dyn Fn(P, P) -> u32) {
        let n = pts.len();
        if n < 2 {
            return;
        }
        // Thin lines: keep a 1 px core and fade the coverage instead of thinning.
        let (core, fade) = if width < 1.0 {
            (0.5, width)
        } else {
            ((width - 1.0) * 0.5 + 0.5, 1.0)
        };
        let half_core = (core - 0.5).max(0.0);
        let outer = half_core + 1.0;
        let segs = if closed { n } else { n - 1 };
        let seg_normal = |i: usize| {
            let (a, b) = (pts[i % n], pts[(i + 1) % n]);
            let d = norm([b[0] - a[0], b[1] - a[1]]);
            [d[1], -d[0]]
        };
        // 1 px lines have a zero-width core: three vertices per point, two strips.
        let thin = half_core <= 0.0;
        let per = if thin { 3 } else { 4 };
        let base = self.vertices.len() as u32;
        self.begin_shapes((segs * 6 * (per - 1)) as u32);
        for (i, &p) in pts.iter().enumerate() {
            let m = if !closed && i == 0 {
                seg_normal(0)
            } else if !closed && i == n - 1 {
                seg_normal(n - 2)
            } else {
                let (a, b) = (seg_normal((i + n - 1) % n), seg_normal(i));
                let mut m = [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
                let d2 = m[0] * m[0] + m[1] * m[1];
                if d2 > 1e-6 {
                    let inv = (1.0 / d2).min(3.0);
                    m = [m[0] * inv, m[1] * inv];
                }
                m
            };
            let at = |d: f32| [p[0] + m[0] * d, p[1] + m[1] * d];
            let c = color(p, norm(m));
            let c = if fade < 1.0 { premul_scale(c, fade) } else { c };
            self.vertices.push(vertex(at(outer), 0));
            self.vertices.push(vertex(at(half_core), c));
            if !thin {
                self.vertices.push(vertex(at(-half_core), c));
            }
            self.vertices.push(vertex(at(-outer), 0));
        }
        let per = per as u32;
        for s in 0..segs as u32 {
            let a = base + per * s;
            let b = base + per * ((s + 1) % n as u32);
            for k in 0..per - 1 {
                self.indices
                    .extend([a + k, a + k + 1, b + k + 1, a + k, b + k + 1, b + k]);
            }
        }
    }

    fn stroke_rounded_lit(
        &mut self,
        rect: Rect,
        corners: impl Into<Corners>,
        width: f32,
        color: &dyn Fn(P, P) -> u32,
    ) {
        let h = width * 0.5;
        let inner = rect.shrink(h);
        let c = corners.into();
        let c = Corners(c.0.map(|r| (r - h).max(0.0)));
        with_path(|path| {
            rounded_path(path, inner, c);
            self.polyline_lit(path, true, width, color);
        });
    }

    fn band_rounded(
        &mut self,
        rect: Rect,
        corners: impl Into<Corners>,
        d: [f32; 2],
        c0: u32,
        c1: u32,
    ) {
        if (c0 | c1) >> 24 == 0 || rect.width() <= 0.0 || rect.height() <= 0.0 {
            return;
        }
        let corners = corners.into();
        with_path(|path| {
            rounded_path(path, rect, corners);
            let n = path.len();
            if n < 3 {
                return;
            }
            let mut normals = Vec::new();
            SCRATCH.with(|s| {
                if let Ok(mut s) = s.try_borrow_mut() {
                    normals = std::mem::take(&mut s.1);
                }
            });
            vertex_normals(path, &mut normals);
            let base = self.vertices.len() as u32;
            self.begin_shapes((n * 6) as u32);
            for (p, m) in path.iter().zip(&normals) {
                self.vertices
                    .push(vertex([p[0] + m[0] * d[0], p[1] + m[1] * d[0]], c0));
                self.vertices
                    .push(vertex([p[0] + m[0] * d[1], p[1] + m[1] * d[1]], c1));
            }
            for i in 0..n as u32 {
                let j = (i + 1) % n as u32;
                let (a0, a1, b0, b1) = (
                    base + 2 * i,
                    base + 2 * i + 1,
                    base + 2 * j,
                    base + 2 * j + 1,
                );
                self.indices.extend([a0, a1, b1, a0, b1, b0]);
            }
            SCRATCH.with(|s| {
                if let Ok(mut s) = s.try_borrow_mut() {
                    s.1 = normals;
                }
            });
        });
    }

    fn uv_from(&mut self, start: usize, uv: &dyn Fn(P) -> [f32; 2]) {
        for v in self.vertices.iter_mut().skip(start) {
            v.uv = uv(v.pos);
        }
    }

    fn shadow(&mut self, rect: Rect, radius: f32, blur: f32, offset: P, color: u32) {
        if color >> 24 == 0 {
            return;
        }
        let r = Rect {
            min: [rect.min[0] + offset[0], rect.min[1] + offset[1]],
            max: [rect.max[0] + offset[0], rect.max[1] + offset[1]],
        };
        with_path(|path| {
            rounded_path(path, r, Corners::all(radius + blur * 0.25));
            self.fill_convex(path, blur.max(1.0), &|_| color);
        });
    }

    fn icon(&mut self, icon: Icon, center: P, size: f32, width: f32, color: u32) {
        let k = size / 24.0;
        let p = |x: f32, y: f32| [center[0] + (x - 12.0) * k, center[1] + (y - 12.0) * k];
        let w = width;
        match icon {
            Icon::Crosshair => {
                self.circle_stroke(p(12.0, 12.0), 7.0 * k, w, color);
                self.line(p(12.0, 2.0), p(12.0, 7.5), w, color);
                self.line(p(12.0, 16.5), p(12.0, 22.0), w, color);
                self.line(p(2.0, 12.0), p(7.5, 12.0), w, color);
                self.line(p(16.5, 12.0), p(22.0, 12.0), w, color);
                self.circle(p(12.0, 12.0), 1.3 * k, color);
            }
            Icon::Shield => {
                let pts = [
                    p(12.0, 2.5),
                    p(19.5, 5.5),
                    p(19.5, 11.0),
                    p(18.2, 15.2),
                    p(15.5, 18.6),
                    p(12.0, 21.5),
                    p(8.5, 18.6),
                    p(5.8, 15.2),
                    p(4.5, 11.0),
                    p(4.5, 5.5),
                ];
                self.polyline(&pts, true, w, color);
            }
            Icon::Eye => {
                let mut pts = Vec::with_capacity(26);
                for i in 0..=12 {
                    let t = i as f32 / 12.0;
                    pts.push(p(2.0 + 20.0 * t, 12.0 - 7.0 * (PI * t).sin()));
                }
                for i in 1..12 {
                    let t = 1.0 - i as f32 / 12.0;
                    pts.push(p(2.0 + 20.0 * t, 12.0 + 7.0 * (PI * t).sin()));
                }
                self.polyline(&pts, true, w, color);
                self.circle_stroke(p(12.0, 12.0), 3.2 * k, w, color);
            }
            Icon::Palette => {
                let mut pts = Vec::new();
                arc(&mut pts, p(12.0, 12.0), 9.0 * k, 0.15 * PI, 1.95 * PI);
                pts.push(p(17.0, 12.5));
                pts.push(p(16.0, 15.5));
                self.polyline(&pts, true, w, color);
                for (x, y) in [(8.0, 9.0), (12.0, 7.0), (16.0, 9.0), (8.0, 14.5)] {
                    self.circle(p(x, y), 1.5 * k, color);
                }
            }
            Icon::Gear => {
                let mut pts = Vec::new();
                let teeth = 8;
                for i in 0..teeth * 4 {
                    let a = TAU * i as f32 / (teeth * 4) as f32 - PI / 2.0;
                    let r = if (i / 2) % 2 == 0 { 9.5 } else { 7.2 };
                    pts.push(p(12.0 + r * a.cos(), 12.0 + r * a.sin()));
                }
                self.polyline(&pts, true, w, color);
                self.circle_stroke(p(12.0, 12.0), 3.0 * k, w, color);
            }
            Icon::User => {
                self.circle_stroke(p(12.0, 8.0), 4.0 * k, w, color);
                let mut pts = Vec::new();
                arc(&mut pts, p(12.0, 21.0), 7.5 * k, PI, TAU);
                self.polyline(&pts, false, w, color);
            }
            Icon::Run => {
                self.circle(p(15.0, 4.5), 2.0 * k, color);
                self.polyline(
                    &[p(9.0, 9.5), p(13.5, 8.0), p(16.0, 11.5), p(19.5, 12.5)],
                    false,
                    w,
                    color,
                );
                self.polyline(
                    &[p(13.5, 8.0), p(11.5, 14.0), p(15.0, 17.0), p(14.0, 21.5)],
                    false,
                    w,
                    color,
                );
                self.polyline(
                    &[p(11.5, 14.0), p(9.0, 18.0), p(4.5, 18.5)],
                    false,
                    w,
                    color,
                );
            }
            Icon::Search => {
                self.circle_stroke(p(10.5, 10.5), 6.5 * k, w, color);
                self.line(p(15.5, 15.5), p(20.5, 20.5), w, color);
            }
            Icon::ChevronDown => {
                self.polyline(&[p(6.0, 9.0), p(12.0, 15.0), p(18.0, 9.0)], false, w, color)
            }
            Icon::ChevronUp => self.polyline(
                &[p(6.0, 15.0), p(12.0, 9.0), p(18.0, 15.0)],
                false,
                w,
                color,
            ),
            Icon::ChevronRight => {
                self.polyline(&[p(9.0, 6.0), p(15.0, 12.0), p(9.0, 18.0)], false, w, color)
            }
            Icon::ChevronLeft => self.polyline(
                &[p(15.0, 6.0), p(9.0, 12.0), p(15.0, 18.0)],
                false,
                w,
                color,
            ),
            Icon::Check => self.polyline(
                &[p(5.0, 12.5), p(10.0, 17.5), p(19.0, 7.0)],
                false,
                w,
                color,
            ),
            Icon::Close => {
                self.line(p(6.0, 6.0), p(18.0, 18.0), w, color);
                self.line(p(18.0, 6.0), p(6.0, 18.0), w, color);
            }
            Icon::Info => {
                self.circle_stroke(p(12.0, 12.0), 9.5 * k, w, color);
                self.line(p(12.0, 11.0), p(12.0, 17.0), w, color);
                self.circle(p(12.0, 7.5), 1.3 * k, color);
            }
            Icon::Warning => {
                self.polyline(&[p(12.0, 3.0), p(22.0, 20.5), p(2.0, 20.5)], true, w, color);
                self.line(p(12.0, 9.5), p(12.0, 14.5), w, color);
                self.circle(p(12.0, 17.5), 1.2 * k, color);
            }
            Icon::Menu => {
                for y in [6.0, 12.0, 18.0] {
                    self.line(p(4.0, y), p(20.0, y), w, color);
                }
            }
            Icon::Target => {
                self.circle_stroke(p(12.0, 12.0), 9.0 * k, w, color);
                self.circle_stroke(p(12.0, 12.0), 5.0 * k, w, color);
                self.circle(p(12.0, 12.0), 1.6 * k, color);
            }
            Icon::Keyboard => {
                let r = Rect {
                    min: p(2.5, 6.0),
                    max: p(21.5, 18.0),
                };
                self.stroke_rounded(r, 2.0 * k, w, color);
                for x in [6.5, 10.0, 13.5, 17.0] {
                    self.circle(p(x, 10.0), 0.9 * k, color);
                }
                self.line(p(7.5, 14.5), p(16.5, 14.5), w, color);
            }
            Icon::Grid => {
                for (x, y) in [(4.0, 4.0), (13.0, 4.0), (4.0, 13.0), (13.0, 13.0)] {
                    let r = Rect {
                        min: p(x, y),
                        max: p(x + 7.0, y + 7.0),
                    };
                    self.stroke_rounded(r, 1.5 * k, w, color);
                }
            }
        }
    }
}

fn with_path(f: impl FnOnce(&mut Vec<P>)) {
    let mut path = Vec::new();
    SCRATCH.with(|s| {
        if let Ok(mut s) = s.try_borrow_mut() {
            path = std::mem::take(&mut s.0);
        }
    });
    f(&mut path);
    SCRATCH.with(|s| {
        if let Ok(mut s) = s.try_borrow_mut() {
            s.0 = path;
        }
    });
}

/// Command bookkeeping shared with [`DrawList`]'s own quads.
trait BeginShapes {
    fn begin_shapes(&mut self, indices: u32);
}

impl BeginShapes for DrawList {
    fn begin_shapes(&mut self, indices: u32) {
        self.open_command(FONT_TEXTURE, indices);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::rgba;

    fn valid(list: &DrawList) {
        assert!(
            list.indices
                .iter()
                .all(|&i| (i as usize) < list.vertices.len())
        );
        let covered: u32 = list.commands.iter().map(|c| c.indices.len() as u32).sum();
        assert_eq!(
            covered as usize,
            list.indices.len(),
            "commands cover all indices"
        );
    }

    #[test]
    fn shapes_produce_consistent_meshes() {
        let mut list = DrawList::default();
        let r = Rect::new(10.0, 10.0, 100.0, 40.0);
        list.fill_rounded(r, Corners([0.0, 4.0, 12.0, 30.0]), rgba(255, 0, 0, 255));
        list.stroke_rounded(r, 8.0, 1.0, rgba(0, 255, 0, 255));
        list.circle([50.0, 50.0], 10.0, rgba(0, 0, 255, 255));
        list.shadow(r, 8.0, 12.0, [0.0, 4.0], rgba(0, 0, 0, 100));
        list.gradient_rounded(r, 6.0, rgba(255, 255, 255, 255), rgba(0, 0, 0, 255));
        for icon in Icon::ALL {
            list.icon(icon, [40.0, 40.0], 20.0, 1.5, rgba(255, 255, 255, 255));
        }
        valid(&list);
        assert_eq!(
            list.commands.len(),
            1,
            "everything batches into one draw call"
        );
    }

    #[test]
    fn fringe_fades_outward() {
        let mut list = DrawList::default();
        list.fill_rounded(
            Rect::new(0.0, 0.0, 20.0, 20.0),
            5.0,
            rgba(255, 255, 255, 255),
        );
        // Inner vertices opaque, outer transparent, alternating.
        assert_eq!(list.vertices[0].color >> 24, 255);
        assert_eq!(list.vertices[1].color >> 24, 0);
        let out = list.vertices[1].pos;
        let inn = list.vertices[0].pos;
        assert!(out[0] < inn[0] && out[1] < inn[1], "outer ring outside");
    }
}
