//! The canvas tab's whiteboard, after Tern's canvas blocks: ink drawn with
//! perfect-freehand ([`freehand`]), rectangles, ellipses, lines, arrows,
//! text, sticky notes and images on an infinite page; the style each takes
//! (Tern's 16-color palette that follows the light or dark theme, fill
//! style, opacity, pen type and size); the camera; hit testing; and the
//! undo history. The file is JSON (`Doc`), one per canvas tab.

pub mod agent;
pub mod freehand;
pub mod geom;
#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};

use freehand::{Easing, StrokeOptions};
pub use geom::{P, Rect};

/// Tern's palette; colors follow the light or dark theme.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Color {
    #[default]
    Ink,
    Red,
    Orange,
    Yellow,
    Green,
    Emerald,
    Cyan,
    Sky,
    Blue,
    Violet,
    Pink,
    Indigo,
    Slate,
    LightSlate,
    PaleSlate,
    Paper,
}

impl Color {
    pub const ALL: [Color; 16] = [
        Color::Ink,
        Color::Red,
        Color::Orange,
        Color::Yellow,
        Color::Green,
        Color::Emerald,
        Color::Cyan,
        Color::Sky,
        Color::Blue,
        Color::Violet,
        Color::Pink,
        Color::Indigo,
        Color::Slate,
        Color::LightSlate,
        Color::PaleSlate,
        Color::Paper,
    ];

    /// `0xRRGGBB` on a light or dark page.
    pub fn rgb(self, light: bool) -> u32 {
        let (dark, on_light) = match self {
            Color::Ink => (0xe8eaed, 0x1d1f23),
            Color::Red => (0xf87171, 0xdc2626),
            Color::Orange => (0xfb923c, 0xea580c),
            Color::Yellow => (0xfacc15, 0xca8a04),
            Color::Green => (0x4ade80, 0x16a34a),
            Color::Emerald => (0x34d399, 0x059669),
            Color::Cyan => (0x22d3ee, 0x0891b2),
            Color::Sky => (0x38bdf8, 0x0284c7),
            Color::Blue => (0x60a5fa, 0x2563eb),
            Color::Violet => (0xa78bfa, 0x7c3aed),
            Color::Pink => (0xf472b6, 0xdb2777),
            Color::Indigo => (0x818cf8, 0x4f46e5),
            Color::Slate => (0x94a3b8, 0x64748b),
            Color::LightSlate => (0xcbd5e1, 0x94a3b8),
            Color::PaleSlate => (0xe2e8f0, 0xcbd5e1),
            Color::Paper => (0x1c1d21, 0xffffff),
        };
        if light { on_light } else { dark }
    }

    pub fn name(self) -> &'static str {
        match self {
            Color::Ink => "Ink",
            Color::Red => "Red",
            Color::Orange => "Orange",
            Color::Yellow => "Yellow",
            Color::Green => "Green",
            Color::Emerald => "Emerald",
            Color::Cyan => "Cyan",
            Color::Sky => "Sky",
            Color::Blue => "Blue",
            Color::Violet => "Violet",
            Color::Pink => "Pink",
            Color::Indigo => "Indigo",
            Color::Slate => "Slate",
            Color::LightSlate => "Light slate",
            Color::PaleSlate => "Pale slate",
            Color::Paper => "Paper",
        }
    }
}

/// How a closed shape is filled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fill {
    #[default]
    None,
    /// The fill color, translucent.
    Semi,
    Solid,
    /// Diagonal hatch lines.
    Pattern,
}

impl Fill {
    pub const ALL: [Fill; 4] = [Fill::None, Fill::Semi, Fill::Solid, Fill::Pattern];
}

/// Tern's pen types.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pen {
    /// Width follows the stroke's speed.
    #[default]
    Pen,
    /// One even width.
    Monoline,
    /// Thin hairlines, full swells.
    Fountain,
}

impl Pen {
    pub const ALL: [Pen; 3] = [Pen::Pen, Pen::Monoline, Pen::Fountain];

    pub fn name(self) -> &'static str {
        match self {
            Pen::Pen => "Pen",
            Pen::Monoline => "Monoline",
            Pen::Fountain => "Fountain pen",
        }
    }
}

/// Stroke widths (page units) of the six sizes.
pub const SIZES: [f32; 6] = [1.5, 2.5, 4., 6., 9., 13.];
/// Text font sizes of the six sizes.
pub const FONT_SIZES: [f32; 6] = [14., 18., 24., 32., 44., 60.];
pub const OPACITIES: [u8; 4] = [25, 50, 75, 100];

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Style {
    pub color: Color,
    /// `None`: the stroke color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill_color: Option<Color>,
    #[serde(default)]
    pub fill: Fill,
    /// Percent.
    pub opacity: u8,
    /// Index into [`SIZES`] / [`FONT_SIZES`].
    pub size: u8,
    #[serde(default)]
    pub pen: Pen,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            color: Color::Ink,
            fill_color: None,
            fill: Fill::None,
            opacity: 100,
            size: 1,
            pen: Pen::Pen,
        }
    }
}

impl Style {
    pub fn width(&self) -> f32 {
        SIZES[(self.size as usize).min(SIZES.len() - 1)]
    }
    pub fn font_size(&self) -> f32 {
        FONT_SIZES[(self.size as usize).min(FONT_SIZES.len() - 1)]
    }
    pub fn fill_color(&self) -> Color {
        self.fill_color.unwrap_or(self.color)
    }
    pub fn alpha(&self) -> f32 {
        f32::from(self.opacity.clamp(1, 100)) / 100.
    }
}

/// The freehand options a pen draws with at `width`.
pub fn stroke_options(pen: Pen, highlighter: bool, width: f32, last: bool) -> StrokeOptions {
    if highlighter {
        return StrokeOptions {
            size: width * 4. + 6.,
            thinning: 0.,
            smoothing: 0.5,
            streamline: 0.5,
            easing: Easing::OutSine,
            simulate_pressure: false,
            last,
            ..StrokeOptions::default()
        };
    }
    match pen {
        Pen::Pen => StrokeOptions {
            size: width * 1.6 + 0.8,
            thinning: 0.5,
            smoothing: 0.62,
            streamline: 0.55,
            easing: Easing::OutSine,
            simulate_pressure: true,
            last,
            ..StrokeOptions::default()
        },
        Pen::Monoline => StrokeOptions {
            size: width + 0.5,
            thinning: 0.,
            smoothing: 0.62,
            streamline: 0.55,
            easing: Easing::Linear,
            simulate_pressure: false,
            last,
            ..StrokeOptions::default()
        },
        Pen::Fountain => StrokeOptions {
            size: width * 2.2 + 1.,
            thinning: 0.8,
            smoothing: 0.6,
            streamline: 0.5,
            easing: Easing::InOutCubic,
            simulate_pressure: true,
            taper_end: width * 4.,
            last,
            ..StrokeOptions::default()
        },
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Kind {
    Ink {
        points: Vec<P>,
        #[serde(default)]
        highlighter: bool,
        /// Ends where it began: fillable.
        #[serde(default)]
        closed: bool,
    },
    Rect {
        rect: Rect,
        /// Text centered inside.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        label: String,
    },
    Ellipse {
        rect: Rect,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        label: String,
    },
    Line {
        a: P,
        b: P,
    },
    Arrow {
        a: P,
        b: P,
    },
    /// Text from its top-left corner; the font size is the style's.
    Text {
        at: P,
        text: String,
        /// Font size scale from resizing (1: the style's size).
        #[serde(default = "one")]
        scale: f32,
    },
    Note {
        rect: Rect,
        text: String,
    },
    Image {
        rect: Rect,
        /// An absolute path.
        path: String,
    },
}

fn one() -> f32 {
    1.
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Shape {
    pub id: u64,
    #[serde(flatten)]
    pub kind: Kind,
    pub style: Style,
}

/// Text box size estimate (no font metrics here): ~0.55 em per char, 1.3 em lines.
pub fn text_size(text: &str, font: f32) -> (f32, f32) {
    let lines = text.split('\n');
    let mut w: f32 = 0.;
    let mut n = 0;
    for line in lines {
        n += 1;
        w = w.max(line.chars().count() as f32 * font * 0.55);
    }
    (w.max(font * 0.6), n.max(1) as f32 * font * 1.3)
}

/// Where `p` lands when a box `from` is stretched into `to`.
fn map_point(p: P, from: &Rect, to: &Rect) -> P {
    let sx = if from.w.abs() < 1e-3 {
        1.
    } else {
        to.w / from.w
    };
    let sy = if from.h.abs() < 1e-3 {
        1.
    } else {
        to.h / from.h
    };
    [to.x + (p[0] - from.x) * sx, to.y + (p[1] - from.y) * sy]
}

fn map_rect(r: &Rect, from: &Rect, to: &Rect) -> Rect {
    Rect::from_corners(
        map_point([r.x, r.y], from, to),
        map_point([r.right(), r.bottom()], from, to),
    )
}

impl Shape {
    pub fn font_size(&self) -> f32 {
        match &self.kind {
            Kind::Text { scale, .. } => self.style.font_size() * scale,
            _ => self.style.font_size(),
        }
    }

    /// The box the shape covers, stroke included.
    pub fn bounds(&self) -> Rect {
        let half = self.style.width() / 2.;
        match &self.kind {
            Kind::Ink {
                points,
                highlighter,
                ..
            } => {
                let size =
                    stroke_options(self.style.pen, *highlighter, self.style.width(), true).size;
                Rect::around(points.iter().copied())
                    .unwrap_or_default()
                    .expand(size / 2.)
            }
            Kind::Rect { rect, .. } | Kind::Ellipse { rect, .. } => rect.expand(half),
            Kind::Line { a, b } => Rect::from_corners(*a, *b).expand(half),
            Kind::Arrow { a, b } => {
                let head = arrow_head_size(self.style.width());
                Rect::from_corners(*a, *b).expand(half + head * 0.6)
            }
            Kind::Text { at, text, .. } => {
                let (w, h) = text_size(text, self.font_size());
                Rect {
                    x: at[0],
                    y: at[1],
                    w,
                    h,
                }
            }
            Kind::Note { rect, .. } | Kind::Image { rect, .. } => *rect,
        }
    }

    pub fn translate(&mut self, d: P) {
        let mv = |p: &mut P| {
            p[0] += d[0];
            p[1] += d[1];
        };
        match &mut self.kind {
            Kind::Ink { points, .. } => points.iter_mut().for_each(mv),
            Kind::Rect { rect, .. }
            | Kind::Ellipse { rect, .. }
            | Kind::Note { rect, .. }
            | Kind::Image { rect, .. } => {
                rect.x += d[0];
                rect.y += d[1];
            }
            Kind::Line { a, b } | Kind::Arrow { a, b } => {
                mv(a);
                mv(b);
            }
            Kind::Text { at, .. } => mv(at),
        }
    }

    /// Stretch the shape as its selection box goes from `from` to `to`.
    /// Text keeps its aspect: its font scales with the height.
    pub fn resize(&mut self, from: &Rect, to: &Rect) {
        match &mut self.kind {
            Kind::Ink { points, .. } => {
                for p in points.iter_mut() {
                    *p = map_point(*p, from, to);
                }
            }
            Kind::Rect { rect, .. }
            | Kind::Ellipse { rect, .. }
            | Kind::Note { rect, .. }
            | Kind::Image { rect, .. } => *rect = map_rect(rect, from, to),
            Kind::Line { a, b } | Kind::Arrow { a, b } => {
                *a = map_point(*a, from, to);
                *b = map_point(*b, from, to);
            }
            Kind::Text { at, scale, .. } => {
                *at = map_point(*at, from, to);
                if from.h > 1e-3 {
                    *scale = (*scale * (to.h / from.h).abs()).clamp(0.1, 50.);
                }
            }
        }
    }

    /// Whether the shape has an inside to fill.
    pub fn closed(&self) -> bool {
        match &self.kind {
            Kind::Ink {
                closed,
                highlighter,
                ..
            } => *closed && !highlighter,
            Kind::Rect { .. } | Kind::Ellipse { .. } => true,
            _ => false,
        }
    }

    /// The fillable outline (page units) of a closed shape.
    pub fn fill_polygon(&self) -> Option<Vec<P>> {
        if !self.closed() {
            return None;
        }
        match &self.kind {
            Kind::Ink { points, .. } => Some(points.clone()),
            Kind::Rect { rect, .. } => Some(rect.corners().to_vec()),
            Kind::Ellipse { rect, .. } => Some(geom::ellipse_points(rect, 64)),
            _ => None,
        }
    }

    /// Whether `p` touches the shape, within `tolerance` (page units).
    pub fn hit(&self, p: P, tolerance: f32) -> bool {
        if !self.bounds().expand(tolerance).contains(p) {
            return false;
        }
        let half = self.style.width() / 2.;
        let filled = self.style.fill != Fill::None;
        match &self.kind {
            Kind::Ink {
                points,
                highlighter,
                closed,
            } => {
                let size =
                    stroke_options(self.style.pen, *highlighter, self.style.width(), true).size;
                geom::polyline_distance(p, points) <= size / 2. + tolerance
                    || (*closed && filled && geom::point_in_polygon(p, points))
            }
            Kind::Rect { rect, label } => {
                let corners = rect.corners();
                let mut ring = corners.to_vec();
                ring.push(corners[0]);
                geom::polyline_distance(p, &ring) <= half + tolerance
                    || ((filled || !label.is_empty()) && rect.contains(p))
            }
            Kind::Ellipse { rect, label } => {
                let (d, inside) = geom::ellipse_distance(p, rect);
                d <= half + tolerance || ((filled || !label.is_empty()) && inside)
            }
            Kind::Line { a, b } => geom::segment_distance(p, *a, *b) <= half + tolerance,
            Kind::Arrow { a, b } => {
                let mut near = geom::segment_distance(p, *a, *b) <= half + tolerance;
                if let Some((l, r)) = geom::arrow_head(*a, *b, arrow_head_size(self.style.width()))
                {
                    near |= geom::segment_distance(p, *b, l) <= half + tolerance
                        || geom::segment_distance(p, *b, r) <= half + tolerance;
                }
                near
            }
            Kind::Text { .. } | Kind::Note { .. } | Kind::Image { .. } => {
                self.bounds().expand(tolerance).contains(p)
            }
        }
    }

    /// Points along the shape, for lasso and box selection.
    pub fn samples(&self) -> Vec<P> {
        match &self.kind {
            Kind::Ink { points, .. } => {
                let step = (points.len() / 64).max(1);
                points.iter().step_by(step).copied().collect()
            }
            Kind::Ellipse { rect, .. } => geom::ellipse_points(rect, 24),
            Kind::Line { a, b } | Kind::Arrow { a, b } => geom::sample_segment(*a, *b, 8.),
            _ => {
                let b = self.bounds();
                let mut out = b.corners().to_vec();
                out.push(b.center());
                out
            }
        }
    }

    /// Box selection: the shape is inside `area` or crosses it.
    pub fn touches_box(&self, area: &Rect) -> bool {
        let b = self.bounds();
        if !area.intersects(&b) {
            return false;
        }
        area.contains_rect(&b) || self.samples().into_iter().any(|p| area.contains(p))
    }

    /// Lasso selection: the shape lies inside the loop.
    pub fn inside_loop(&self, lasso: &[P]) -> bool {
        let samples = self.samples();
        !samples.is_empty()
            && samples
                .into_iter()
                .all(|p| geom::point_in_polygon(p, lasso))
    }
}

/// An arrow head's wing length for a stroke `width`.
pub fn arrow_head_size(width: f32) -> f32 {
    10. + width * 3.
}

/// The view onto the page: the page point at the canvas' top-left and pixels
/// per page unit.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Camera {
    pub x: f32,
    pub y: f32,
    pub zoom: f32,
}

pub const MIN_ZOOM: f32 = 0.1;
pub const MAX_ZOOM: f32 = 8.;

impl Default for Camera {
    fn default() -> Self {
        Self {
            x: 0.,
            y: 0.,
            zoom: 1.,
        }
    }
}

impl Camera {
    /// A point on the canvas (px from its top-left) on the page.
    pub fn to_page(&self, screen: P) -> P {
        [
            self.x + screen[0] / self.zoom,
            self.y + screen[1] / self.zoom,
        ]
    }

    pub fn to_screen(&self, page: P) -> P {
        [
            (page[0] - self.x) * self.zoom,
            (page[1] - self.y) * self.zoom,
        ]
    }

    /// Zoom by `factor` keeping the page point under `screen` in place.
    pub fn zoom_at(&mut self, factor: f32, screen: P) {
        let before = self.to_page(screen);
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        self.x = before[0] - screen[0] / self.zoom;
        self.y = before[1] - screen[1] / self.zoom;
    }

    pub fn pan(&mut self, screen_delta: P) {
        self.x -= screen_delta[0] / self.zoom;
        self.y -= screen_delta[1] / self.zoom;
    }

    /// The page area a `w`×`h` px canvas shows.
    pub fn visible(&self, w: f32, h: f32) -> Rect {
        Rect {
            x: self.x,
            y: self.y,
            w: w / self.zoom,
            h: h / self.zoom,
        }
    }

    /// The camera showing `content` centered in a `w`×`h` px canvas with
    /// `margin` px around, at most 100 %.
    pub fn fitting(content: &Rect, w: f32, h: f32, margin: f32) -> Self {
        let zoom = ((w - margin * 2.) / content.w.max(1.))
            .min((h - margin * 2.) / content.h.max(1.))
            .clamp(MIN_ZOOM, 1.);
        let [cx, cy] = content.center();
        Self {
            x: cx - w / 2. / zoom,
            y: cy - h / 2. / zoom,
            zoom,
        }
    }
}

/// A canvas file.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Doc {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub shapes: Vec<Shape>,
    #[serde(default)]
    pub camera: Camera,
}

pub const VERSION: u32 = 1;

impl Doc {
    pub fn parse(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(&Doc {
            version: VERSION,
            ..self.clone()
        })
        .unwrap_or_default()
    }

    /// An id no shape has.
    pub fn next_id(&self) -> u64 {
        self.shapes.iter().map(|s| s.id).max().map_or(1, |m| m + 1)
    }

    /// The box around every shape; `None` on an empty page.
    pub fn content_bounds(&self) -> Option<Rect> {
        bounds_of(self.shapes.iter())
    }

    /// The topmost shape under `p`.
    pub fn hit(&self, p: P, tolerance: f32) -> Option<u64> {
        self.shapes
            .iter()
            .rev()
            .find(|s| s.hit(p, tolerance))
            .map(|s| s.id)
    }

    pub fn get(&self, id: u64) -> Option<&Shape> {
        self.shapes.iter().find(|s| s.id == id)
    }

    pub fn get_mut(&mut self, id: u64) -> Option<&mut Shape> {
        self.shapes.iter_mut().find(|s| s.id == id)
    }

    /// Copies of `ids` (in page order) under fresh ids, moved by `offset`.
    pub fn duplicate(&mut self, ids: &[u64], offset: P) -> Vec<u64> {
        let copies: Vec<Shape> = self
            .shapes
            .iter()
            .filter(|s| ids.contains(&s.id))
            .cloned()
            .collect();
        self.paste(copies, offset)
    }

    /// Adds `shapes` on top under fresh ids, moved by `offset`; their ids.
    pub fn paste(&mut self, shapes: Vec<Shape>, offset: P) -> Vec<u64> {
        let mut out = Vec::new();
        for (id, mut s) in (self.next_id()..).zip(shapes) {
            s.id = id;
            s.translate(offset);
            out.push(id);
            self.shapes.push(s);
        }
        out
    }

    pub fn remove(&mut self, ids: &[u64]) -> bool {
        let before = self.shapes.len();
        self.shapes.retain(|s| !ids.contains(&s.id));
        self.shapes.len() != before
    }

    /// Moves `ids` to the top (or the bottom), keeping their order.
    pub fn reorder(&mut self, ids: &[u64], to_front: bool) {
        let (picked, rest): (Vec<Shape>, Vec<Shape>) = std::mem::take(&mut self.shapes)
            .into_iter()
            .partition(|s| ids.contains(&s.id));
        self.shapes = if to_front {
            rest.into_iter().chain(picked).collect()
        } else {
            picked.into_iter().chain(rest).collect()
        };
    }
}

pub fn bounds_of<'a>(shapes: impl IntoIterator<Item = &'a Shape>) -> Option<Rect> {
    shapes
        .into_iter()
        .map(Shape::bounds)
        .reduce(|a, b| a.union(&b))
}

/// What the system clipboard carries for copied shapes: a marker line, then
/// the shapes as JSON, so pasting elsewhere gives text and pasting back
/// gives editable shapes.
const CLIPBOARD_MARK: &str = "gentle-canvas/1\n";

pub fn to_clipboard(shapes: &[Shape]) -> String {
    format!(
        "{CLIPBOARD_MARK}{}",
        serde_json::to_string(shapes).unwrap_or_default()
    )
}

pub fn from_clipboard(text: &str) -> Option<Vec<Shape>> {
    serde_json::from_str(text.strip_prefix(CLIPBOARD_MARK)?).ok()
}

/// Undo and redo as whole-page snapshots: every change records the shapes
/// as they were.
#[derive(Default)]
pub struct History {
    undo: Vec<Vec<Shape>>,
    redo: Vec<Vec<Shape>>,
}

const HISTORY_LIMIT: usize = 200;

impl History {
    /// Call before a change, with the shapes it starts from.
    pub fn record(&mut self, before: Vec<Shape>) {
        self.undo.push(before);
        if self.undo.len() > HISTORY_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    pub fn undo(&mut self, shapes: &mut Vec<Shape>) -> bool {
        let Some(prev) = self.undo.pop() else {
            return false;
        };
        self.redo.push(std::mem::replace(shapes, prev));
        true
    }

    pub fn redo(&mut self, shapes: &mut Vec<Shape>) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo.push(std::mem::replace(shapes, next));
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}
