//! Drawing the page: shapes are tessellated once, in page units, and kept
//! by id; each frame only moves their vertices onto the screen (camera,
//! and the live move or resize of a selection), so panning, zooming and
//! dragging never re-tessellate. The dotted grid, the live stroke and the
//! selection chrome are painted on top.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

use gentle_core::canvas::{self, Fill, Kind, P, Pen, Rect, Shape, Style, freehand, geom};
use gpui_kit::*;

use crate::shared::ui;

/// Lyon's flattening tolerance, page units. The outlines' control points
/// are already dense, so a coarse bound looks the same and keeps a stroke to
/// a few hundred vertices (0.02 made pages of ink cost ~1 GB).
const TOLERANCE: f32 = 0.25;
/// Pattern fill: hatch spacing and line width, page units.
const HATCH_GAP: f32 = 9.;
/// Grid spacing at 100 %, and the closest dots may get on screen (px).
const GRID: f32 = 24.;
const GRID_MIN_PX: f32 = 16.;
/// How long a laser point glows.
pub(super) const LASER_LIFE: f32 = 1.1;

/// A shape's tessellated pieces, in page units.
#[derive(Default)]
pub(super) struct Geo {
    /// The shape's box, kept so culling and erasing never walk its points.
    pub bounds: Rect,
    fill: Option<Path<Pixels>>,
    hatch: Option<Path<Pixels>>,
    stroke: Option<Path<Pixels>>,
}

pub(super) type GeoCache = Rc<RefCell<HashMap<u64, Geo>>>;

/// Page → canvas px: `x' = x·sx + tx`, `y' = y·sy + ty`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Affine {
    pub sx: f32,
    pub sy: f32,
    pub tx: f32,
    pub ty: f32,
}

impl Affine {
    pub fn camera(camera: &canvas::Camera) -> Self {
        Self {
            sx: camera.zoom,
            sy: camera.zoom,
            tx: -camera.x * camera.zoom,
            ty: -camera.y * camera.zoom,
        }
    }

    /// First move by `d` (page units), then this.
    pub fn after_move(self, d: P) -> Self {
        Self {
            tx: self.tx + d[0] * self.sx,
            ty: self.ty + d[1] * self.sy,
            ..self
        }
    }

    /// First stretch the box `from` into `to` (page units), then this.
    pub fn after_resize(self, from: &Rect, to: &Rect) -> Self {
        let kx = if from.w.abs() < 1e-3 {
            1.
        } else {
            to.w / from.w
        };
        let ky = if from.h.abs() < 1e-3 {
            1.
        } else {
            to.h / from.h
        };
        Self {
            sx: self.sx * kx,
            sy: self.sy * ky,
            tx: self.tx + (to.x - from.x * kx) * self.sx,
            ty: self.ty + (to.y - from.y * ky) * self.sy,
        }
    }

    pub fn apply(&self, p: P) -> P {
        [p[0] * self.sx + self.tx, p[1] * self.sy + self.ty]
    }

    pub fn rect(&self, r: &Rect) -> Rect {
        Rect::from_corners(self.apply([r.x, r.y]), self.apply([r.right(), r.bottom()]))
    }

    fn place(&self, path: &Path<Pixels>, origin: Point<Pixels>) -> Path<Pixels> {
        let (ox, oy) = (f32::from(origin.x), f32::from(origin.y));
        let map = |p: Point<Pixels>| {
            point(
                px(f32::from(p.x) * self.sx + self.tx + ox),
                px(f32::from(p.y) * self.sy + self.ty + oy),
            )
        };
        let mut out = path.clone();
        for v in &mut out.vertices {
            v.xy_position = map(v.xy_position);
        }
        let a = map(path.bounds.origin);
        let b = map(path.bounds.bottom_right());
        out.bounds = Bounds::from_corners(
            point(a.x.min(b.x), a.y.min(b.y)),
            point(a.x.max(b.x), a.y.max(b.y)),
        );
        out
    }
}

fn pt(p: P) -> Point<Pixels> {
    point(px(p[0]), px(p[1]))
}

fn fill_builder() -> PathBuilder {
    PathBuilder::fill().with_style(PathStyle::Fill(
        FillOptions::non_zero().with_tolerance(TOLERANCE),
    ))
}

/// A closed loop through `points` smoothed with quadratic curves through
/// their midpoints (how perfect-freehand outlines are meant to be drawn).
fn smooth_loop(builder: &mut PathBuilder, points: &[P]) {
    let n = points.len();
    if n < 3 {
        return;
    }
    let mid = |a: P, b: P| [(a[0] + b[0]) / 2., (a[1] + b[1]) / 2.];
    builder.move_to(pt(mid(points[0], points[1])));
    for i in 1..=n {
        let c = points[i % n];
        let next = points[(i + 1) % n];
        builder.curve_to(pt(mid(c, next)), pt(c));
    }
    builder.close();
}

/// A segment with round ends, `r` thick on each side.
fn capsule(builder: &mut PathBuilder, a: P, b: P, r: f32) {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len = dx.hypot(dy);
    let (nx, ny) = if len < 1e-4 {
        (0., 1.)
    } else {
        (-dy / len, dx / len)
    };
    let angle = ny.atan2(nx);
    const STEPS: usize = 10;
    let mut ring = Vec::with_capacity(STEPS * 2 + 2);
    for i in 0..=STEPS {
        let t = angle + std::f32::consts::PI * i as f32 / STEPS as f32;
        ring.push([b[0] + r * t.cos(), b[1] + r * t.sin()]);
    }
    for i in 0..=STEPS {
        let t = angle + std::f32::consts::PI + std::f32::consts::PI * i as f32 / STEPS as f32;
        ring.push([a[0] + r * t.cos(), a[1] + r * t.sin()]);
    }
    builder.move_to(pt(ring[0]));
    for p in &ring[1..] {
        builder.line_to(pt(*p));
    }
    builder.close();
}

fn polygon_path(points: &[P]) -> Option<Path<Pixels>> {
    let mut b = fill_builder();
    b.move_to(pt(points[0]));
    for p in &points[1..] {
        b.line_to(pt(*p));
    }
    b.close();
    b.build().ok()
}

/// An ink outline as a filled path.
pub(super) fn ink_path(
    points: &[P],
    style: &Style,
    highlighter: bool,
    last: bool,
) -> Option<Path<Pixels>> {
    let outline = freehand::outline(
        points,
        &canvas::stroke_options(style.pen, highlighter, style.width(), last),
    );
    if outline.len() < 3 {
        return None;
    }
    let mut b = fill_builder();
    smooth_loop(&mut b, &outline);
    b.build().ok()
}

/// The pen previews' squiggle box (page units) and its size (index into
/// `canvas::SIZES`).
const PREVIEW_W: f32 = 80.;
const PREVIEW_H: f32 = 26.;
const PREVIEW_SIZE: u8 = 2;

thread_local! {
    /// Each pen's preview, tessellated once.
    static PEN_PREVIEWS: [Option<Path<Pixels>>; 3] = Pen::ALL.map(pen_preview_path);
}

/// One wave drawn slow at its ends and fast in the middle, as a hand
/// would, so the pen's speed-driven width shows.
fn pen_preview_path(pen: Pen) -> Option<Path<Pixels>> {
    const N: usize = 24;
    let points: Vec<P> = (0..N)
        .map(|i| {
            let t = i as f32 / (N - 1) as f32;
            let s = (1. - (t * std::f32::consts::PI).cos()) / 2.;
            [
                8. + s * (PREVIEW_W - 16.),
                PREVIEW_H / 2. - (s * std::f32::consts::TAU).sin() * (PREVIEW_H / 2. - 7.),
            ]
        })
        .collect();
    let style = Style {
        pen,
        size: PREVIEW_SIZE,
        ..Style::default()
    };
    ink_path(&points, &style, false, true)
}

/// `pen`'s preview fitted into `bounds`, drawn the way its strokes are.
pub(super) fn pen_preview(pen: Pen, color: Hsla, bounds: Bounds<Pixels>, window: &mut Window) {
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let k = (w / PREVIEW_W).min(h / PREVIEW_H);
    let at = Affine {
        sx: k,
        sy: k,
        tx: (w - PREVIEW_W * k) / 2.,
        ty: (h - PREVIEW_H * k) / 2.,
    };
    let i = Pen::ALL.iter().position(|p| *p == pen).unwrap_or(0);
    PEN_PREVIEWS.with(|paths| {
        if let Some(path) = &paths[i] {
            window.paint_path(at.place(path, bounds.origin), color);
        }
    });
}

fn hatch_path(poly: &[P], width: f32) -> Option<Path<Pixels>> {
    let lines = geom::hatch(poly, HATCH_GAP.max(width * 3.));
    if lines.is_empty() {
        return None;
    }
    let mut b = fill_builder();
    let r = (width * 0.35).clamp(0.5, 2.5);
    for (a, c) in lines {
        capsule(&mut b, a, c, r);
    }
    b.build().ok()
}

/// Tessellates a shape and measures its box; text, notes and images are
/// elements and get only the box.
pub(super) fn build(shape: &Shape) -> Geo {
    let style = &shape.style;
    let half = style.width() / 2.;
    let mut geo = Geo {
        bounds: shape.bounds(),
        ..Geo::default()
    };
    let poly = shape.fill_polygon();
    match &shape.kind {
        Kind::Ink {
            points,
            highlighter,
            ..
        } => {
            geo.stroke = ink_path(points, style, *highlighter, true);
            if let Some(poly) = poly.as_ref().filter(|_| style.fill != Fill::None) {
                let mut b = fill_builder();
                smooth_loop(&mut b, poly);
                geo.fill = b.build().ok();
            }
        }
        Kind::Rect { .. } | Kind::Ellipse { .. } => {
            let poly = poly.unwrap_or_default();
            if poly.len() >= 3 {
                let mut ring = PathBuilder::stroke(px(style.width()));
                ring.add_polygon(&poly.iter().map(|p| pt(*p)).collect::<Vec<_>>(), true);
                geo.stroke = ring.build().ok();
                if style.fill != Fill::None {
                    geo.fill = polygon_path(&poly);
                }
            }
        }
        Kind::Line { a, b } => {
            let mut path = fill_builder();
            capsule(&mut path, *a, *b, half);
            geo.stroke = path.build().ok();
        }
        Kind::Arrow { a, b } => {
            let mut path = fill_builder();
            capsule(&mut path, *a, *b, half);
            if let Some((l, r)) = geom::arrow_head(*a, *b, canvas::arrow_head_size(style.width())) {
                capsule(&mut path, *b, l, half);
                capsule(&mut path, *b, r, half);
            }
            geo.stroke = path.build().ok();
        }
        Kind::Text { .. } | Kind::Note { .. } | Kind::Image { .. } => {}
    }
    if style.fill == Fill::Pattern
        && let Some(poly) = shape.fill_polygon()
    {
        geo.hatch = hatch_path(&poly, style.width());
    }
    geo
}

pub(super) fn light() -> bool {
    crate::shared::themes::palette(|p| p.light)
}

pub(super) fn color(c: canvas::Color) -> Hsla {
    rgb(c.rgb(light())).into()
}

/// One shape to paint: where it goes and how faded (erasing).
#[derive(Clone, Copy)]
pub(super) struct Item {
    pub id: u64,
    pub style: Style,
    pub highlighter: bool,
    pub at: Affine,
    pub fade: f32,
}

pub(super) fn paint_items(
    items: &[Item],
    cache: &GeoCache,
    origin: Point<Pixels>,
    window: &mut Window,
) {
    let cache = cache.borrow();
    let mut batch = Batch::default();
    for item in items {
        if let Some(geo) = cache.get(&item.id) {
            paint_geo(geo, item, origin, &mut batch, window);
        }
    }
    batch.flush(window);
}

/// Consecutive opaque paths of one color drawn as a single path: every path
/// is a primitive the GPU renders on its own, so a page of handwriting would
/// cost one per stroke. Order is kept (only neighbours merge), and only
/// opaque ones do: translucent overlaps must still darken where they cross.
#[derive(Default)]
struct Batch(Option<(Hsla, Path<Pixels>)>);

impl Batch {
    fn draw(&mut self, path: Path<Pixels>, color: Hsla, window: &mut Window) {
        if color.a < 1. {
            self.flush(window);
            window.paint_path(path, color);
            return;
        }
        match &mut self.0 {
            Some((current, merged)) if *current == color => {
                merged.bounds = merged.bounds.union(&path.bounds);
                merged.vertices.extend(path.vertices);
            }
            _ => {
                self.flush(window);
                self.0 = Some((color, path));
            }
        }
    }

    fn flush(&mut self, window: &mut Window) {
        if let Some((color, path)) = self.0.take() {
            window.paint_path(path, color);
        }
    }
}

fn paint_geo(
    geo: &Geo,
    item: &Item,
    origin: Point<Pixels>,
    batch: &mut Batch,
    window: &mut Window,
) {
    let style = &item.style;
    let alpha = style.alpha() * item.fade;
    let stroke = color(style.color);
    let fill = color(style.fill_color());
    if let Some(path) = &geo.fill {
        let a = match style.fill {
            Fill::Semi | Fill::Pattern => 0.22,
            _ => 1.,
        };
        batch.draw(item.at.place(path, origin), fill.opacity(alpha * a), window);
    }
    if let Some(path) = &geo.hatch {
        batch.draw(item.at.place(path, origin), fill.opacity(alpha), window);
    }
    if let Some(path) = &geo.stroke {
        let a = if item.highlighter { 0.42 } else { 1. };
        batch.draw(
            item.at.place(path, origin),
            stroke.opacity(alpha * a),
            window,
        );
    }
}

/// The dotted page: dots on a grid that doubles as you zoom out.
pub(super) fn grid(bounds: Bounds<Pixels>, camera: &canvas::Camera, window: &mut Window) {
    let mut step = GRID;
    while step * camera.zoom < GRID_MIN_PX {
        step *= 2.;
    }
    let screen_step = step * camera.zoom;
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let first_x = (camera.x / step).ceil() * step;
    let first_y = (camera.y / step).ceil() * step;
    let dot = 1.6;
    let color = ui::muted().opacity(0.28);
    let mut y = (first_y - camera.y) * camera.zoom;
    while y < h {
        let mut x = (first_x - camera.x) * camera.zoom;
        while x < w {
            window.paint_quad(fill(
                Bounds::new(
                    bounds.origin + point(px(x - dot / 2.), px(y - dot / 2.)),
                    size(px(dot), px(dot)),
                ),
                color,
            ));
            x += screen_step;
        }
        y += screen_step;
    }
}

/// What floats over the shapes this frame (canvas px unless noted).
#[derive(Default)]
pub(super) struct Chrome {
    /// The stroke being drawn: its path in page units and how it looks.
    pub live: Option<(Path<Pixels>, Style, bool)>,
    /// The shape being dragged out, in page units.
    pub preview: Option<(Geo, Style)>,
    pub camera: Affine,
    /// Selection: each shape's box, and the box with handles around all.
    pub boxes: Vec<Rect>,
    pub frame: Option<Rect>,
    pub handles: bool,
    /// Box select.
    pub marquee: Option<Rect>,
    /// Lasso loop.
    pub lasso: Vec<P>,
    /// Eraser or laser pointer under the cursor.
    pub eraser: Option<(P, f32)>,
    pub laser: Vec<(P, f32)>,
}

impl Default for Affine {
    fn default() -> Self {
        Self {
            sx: 1.,
            sy: 1.,
            tx: 0.,
            ty: 0.,
        }
    }
}

pub(super) const HANDLE: f32 = 8.;

fn outline_rect(r: &Rect, origin: Point<Pixels>, color: Hsla, window: &mut Window) {
    window.paint_quad(quad(
        Bounds::new(
            origin + point(px(r.x), px(r.y)),
            size(px(r.w.max(1.)), px(r.h.max(1.))),
        ),
        px(0.),
        transparent_black(),
        px(1.),
        color,
        BorderStyle::Solid,
    ));
}

impl Chrome {
    pub fn paint(&self, bounds: Bounds<Pixels>, window: &mut Window) {
        let origin = bounds.origin;
        if let Some((geo, style)) = &self.preview {
            let item = Item {
                id: 0,
                style: *style,
                highlighter: false,
                at: self.camera,
                fade: 1.,
            };
            let mut batch = Batch::default();
            paint_geo(geo, &item, origin, &mut batch, window);
            batch.flush(window);
        }
        if let Some((path, style, highlighter)) = &self.live {
            let a = if *highlighter { 0.42 } else { 1. };
            window.paint_path(
                self.camera.place(path, origin),
                color(style.color).opacity(style.alpha() * a),
            );
        }
        let accent = ui::accent();
        for r in &self.boxes {
            outline_rect(r, origin, accent.opacity(0.45), window);
        }
        if let Some(frame) = &self.frame {
            outline_rect(frame, origin, accent, window);
            if self.handles {
                for [x, y] in frame.corners() {
                    let h = HANDLE;
                    window.paint_quad(quad(
                        Bounds::new(
                            origin + point(px(x - h / 2.), px(y - h / 2.)),
                            size(px(h), px(h)),
                        ),
                        px(2.),
                        ui::popover_bg(),
                        px(1.),
                        accent,
                        BorderStyle::Solid,
                    ));
                }
            }
        }
        if let Some(r) = &self.marquee {
            window.paint_quad(quad(
                Bounds::new(origin + point(px(r.x), px(r.y)), size(px(r.w), px(r.h))),
                px(0.),
                accent.opacity(0.08),
                px(1.),
                accent.opacity(0.7),
                BorderStyle::Solid,
            ));
        }
        if self.lasso.len() > 1 {
            let mut b = PathBuilder::stroke(px(1.5)).dash_array(&[px(4.), px(4.)]);
            b.move_to(pt(self.lasso[0]) + origin);
            for p in &self.lasso[1..] {
                b.line_to(pt(*p) + origin);
            }
            if let Ok(path) = b.build() {
                window.paint_path(path, accent);
            }
            if self.lasso.len() > 2
                && let Some(path) = polygon_path(&self.lasso)
            {
                let ident = Affine::default();
                window.paint_path(ident.place(&path, origin), accent.opacity(0.06));
            }
        }
        if let Some((p, r)) = self.eraser {
            window.paint_quad(quad(
                Bounds::new(
                    origin + point(px(p[0] - r), px(p[1] - r)),
                    size(px(r * 2.), px(r * 2.)),
                ),
                px(r),
                ui::fg().opacity(0.08),
                px(1.),
                ui::fg().opacity(0.6),
                BorderStyle::Solid,
            ));
        }
        self.paint_laser(origin, window);
    }

    /// A red trail that thins and fades with age.
    fn paint_laser(&self, origin: Point<Pixels>, window: &mut Window) {
        let red: Hsla = rgb(0xff3b30).into();
        for w in self.laser.windows(2) {
            let ((a, age_a), (b, _)) = (w[0], w[1]);
            let life = (1. - age_a / LASER_LIFE).clamp(0., 1.);
            if life <= 0. {
                continue;
            }
            for (r, alpha) in [(6. * life + 2., 0.18 * life), (2.2 * life + 0.8, life)] {
                let mut path = fill_builder();
                capsule(&mut path, a, b, r);
                if let Ok(path) = path.build() {
                    let ident = Affine::default();
                    window.paint_path(
                        ident.place(&path, origin),
                        if alpha < 0.5 {
                            red.opacity(alpha)
                        } else {
                            red.blend(Hsla::white().opacity(0.35)).opacity(alpha)
                        },
                    );
                }
            }
        }
    }
}

/// Seconds since `t`.
pub(super) fn age(t: Instant) -> f32 {
    t.elapsed().as_secs_f32()
}
