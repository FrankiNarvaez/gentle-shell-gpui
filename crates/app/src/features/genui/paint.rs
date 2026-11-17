//! Paints what sits under a diagram's nodes: the dotted grid, group boxes,
//! the edges with their arrowheads (async and data ones with marching
//! dashes), the connection being drawn and the selection box; and the
//! minimap. Nodes and labels are elements on top (text, hover, clicks).
//! Only what the canvas shows is painted.

use gentle_core::genui::diagram::EdgeStyle;
use gentle_core::genui::layout::{self, EdgePath, Rect};
use gpui_kit::*;

use super::diagram::GRID;
use crate::shared::ui;

/// Dots closer than this (px) are thinned out to every other line.
const MIN_DOT_GAP: f32 = 10.;
/// Points per Bézier segment when an edge is dashed by hand.
const FLATTEN: usize = 16;

#[derive(Clone, Copy, PartialEq)]
pub(super) enum EdgeLook {
    Normal,
    Hovered,
    Selected,
}

pub(super) struct EdgeDraw {
    pub path: EdgePath,
    pub style: EdgeStyle,
    pub alpha: f32,
    pub look: EdgeLook,
}

/// What one frame paints, captured when the view renders.
pub(super) struct Frame {
    pub groups: Vec<(Rect, bool)>,
    pub edges: Vec<EdgeDraw>,
    /// Pixels per diagram unit, and the pan (px).
    pub scale: f32,
    pub pan: Point<f32>,
    /// How far marching dashes have marched (diagram units), when they move.
    pub march: Option<f32>,
    /// From a node's handle to the pointer (diagram units).
    pub connect: Option<(layout::Point, layout::Point)>,
    /// The shift-drag selection box (diagram units).
    pub select: Option<Rect>,
}

/// Whether async/data edges march.
pub(super) fn animated(style: EdgeStyle) -> bool {
    matches!(style, EdgeStyle::Async | EdgeStyle::Data)
}

impl Frame {
    fn at(&self, origin: Point<Pixels>, p: layout::Point) -> Point<Pixels> {
        point(
            origin.x + px(p.x * self.scale + self.pan.x),
            origin.y + px(p.y * self.scale + self.pan.y),
        )
    }

    /// The canvas in diagram units, to skip what's off screen.
    fn visible(&self, bounds: Bounds<Pixels>) -> Rect {
        Rect {
            x: -self.pan.x / self.scale,
            y: -self.pan.y / self.scale,
            w: f32::from(bounds.size.width) / self.scale,
            h: f32::from(bounds.size.height) / self.scale,
        }
    }

    pub fn paint(&self, bounds: Bounds<Pixels>, window: &mut Window) {
        let origin = bounds.origin;
        let visible = self.visible(bounds);
        let s = self.scale;
        self.grid(bounds, window);
        for (rect, selected) in &self.groups {
            if !rect.intersects(&visible) {
                continue;
            }
            window.paint_quad(quad(
                Bounds::new(
                    self.at(
                        origin,
                        layout::Point {
                            x: rect.x,
                            y: rect.y,
                        },
                    ),
                    size(px(rect.w * s), px(rect.h * s)),
                ),
                px(14. * s),
                ui::surface().opacity(0.55),
                px(if *selected { 1.5 } else { 1. }),
                if *selected {
                    ui::accent()
                } else {
                    ui::border()
                },
                BorderStyle::Solid,
            ));
        }
        for edge in &self.edges {
            if edge.alpha <= 0.01 || !edge.path.bounds().inflate(12.).intersects(&visible) {
                continue;
            }
            self.edge(origin, edge, window);
        }
        if let Some((from, to)) = self.connect {
            let mut path = PathBuilder::stroke(px(1.5)).dash_array(&[px(5.), px(4.)]);
            path.move_to(self.at(origin, from));
            path.line_to(self.at(origin, to));
            if let Ok(path) = path.build() {
                window.paint_path(path, ui::accent());
            }
        }
        if let Some(r) = self.select {
            window.paint_quad(quad(
                Bounds::new(
                    self.at(origin, layout::Point { x: r.x, y: r.y }),
                    size(px(r.w * s), px(r.h * s)),
                ),
                px(4.),
                ui::accent().opacity(0.08),
                px(1.),
                ui::accent().opacity(0.6),
                BorderStyle::Dashed,
            ));
        }
    }

    /// React Flow's dotted background: one dot per grid crossing, moving
    /// and scaling with the view.
    fn grid(&self, bounds: Bounds<Pixels>, window: &mut Window) {
        let mut gap = GRID * self.scale;
        while gap < MIN_DOT_GAP {
            gap *= 2.;
        }
        let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
        let dot = (1.6 * self.scale.min(1.)).max(1.);
        let color = ui::border_strong();
        let mut y = self.pan.y.rem_euclid(gap);
        while y < h {
            let mut x = self.pan.x.rem_euclid(gap);
            while x < w {
                window.paint_quad(fill(
                    Bounds::new(
                        bounds.origin + point(px(x - dot / 2.), px(y - dot / 2.)),
                        size(px(dot), px(dot)),
                    ),
                    color,
                ));
                x += gap;
            }
            y += gap;
        }
    }

    fn edge(&self, origin: Point<Pixels>, edge: &EdgeDraw, window: &mut Window) {
        let s = self.scale.max(0.4);
        let (color, width, dash): (Hsla, f32, Option<[f32; 2]>) = match edge.style {
            EdgeStyle::Sync => (ui::muted(), 1.5, None),
            EdgeStyle::Async => (ui::warning(), 1.5, Some([6., 4.])),
            EdgeStyle::Data => (ui::info(), 2.25, Some([10., 5.])),
            EdgeStyle::Dependency => (ui::faint(), 1.25, Some([2., 4.])),
        };
        let color = match edge.look {
            EdgeLook::Selected => ui::accent(),
            EdgeLook::Hovered => ui::accent().opacity(0.75),
            EdgeLook::Normal => color,
        }
        .opacity(edge.alpha);
        let width = match edge.look {
            EdgeLook::Normal => width,
            _ => width + 1.,
        } * s.min(1.5);
        // Stop the line at the arrowhead's base so its end doesn't poke out.
        let head = 9. * s.min(1.5);
        let dir = edge.path.end_direction();
        let tip = self.at(origin, edge.path.end());
        let end = edge.path.end();
        let base_world = layout::Point {
            x: end.x - dir.x * head / self.scale,
            y: end.y - dir.y * head / self.scale,
        };
        let base = self.at(origin, base_world);
        let last = edge.path.segments.len() - 1;
        let mut path = PathBuilder::stroke(px(width));
        match dash {
            None => {
                path.move_to(self.at(origin, edge.path.start()));
                for (i, seg) in edge.path.segments.iter().enumerate() {
                    let to = if i == last {
                        base
                    } else {
                        self.at(origin, seg.to)
                    };
                    path.cubic_bezier_to(to, self.at(origin, seg.c1), self.at(origin, seg.c2));
                }
            }
            Some([on, off]) => {
                // Dashed by hand: GPUI's dashes can't be offset, and
                // animated edges march by shifting where the pattern starts.
                let mut points: Vec<Point<Pixels>> = Vec::new();
                for (i, seg) in edge.path.segments.iter().enumerate() {
                    let start = if i == 0 { 0 } else { 1 };
                    for k in start..=FLATTEN {
                        let t = k as f32 / FLATTEN as f32;
                        let u = 1. - t;
                        let (a, b, c, d) = (u * u * u, 3. * u * u * t, 3. * u * t * t, t * t * t);
                        let to = if i == last { base_world } else { seg.to };
                        let p = layout::Point {
                            x: a * seg.from.x + b * seg.c1.x + c * seg.c2.x + d * to.x,
                            y: a * seg.from.y + b * seg.c1.y + c * seg.c2.y + d * to.y,
                        };
                        points.push(self.at(origin, p));
                    }
                }
                let shift = if animated(edge.style) {
                    self.march.unwrap_or(0.) * self.scale
                } else {
                    0.
                };
                dash_polyline(&mut path, &points, on * s, off * s, shift);
            }
        }
        if let Ok(path) = path.build() {
            window.paint_path(path, color);
        }
        let side = point(px(-dir.y * head * 0.5), px(dir.x * head * 0.5));
        let mut arrow = PathBuilder::fill();
        arrow.add_polygon(&[tip, base + side, base - side], true);
        if let Ok(arrow) = arrow.build() {
            window.paint_path(arrow, color);
        }
    }
}

/// Dashes `on` long every `on + off` along `points`, the pattern shifted
/// forward by `shift` (px).
fn dash_polyline(path: &mut PathBuilder, points: &[Point<Pixels>], on: f32, off: f32, shift: f32) {
    let period = on + off;
    if period <= 0. || points.len() < 2 {
        return;
    }
    // Distance into the pattern at the start of the line.
    let mut phase = (-shift).rem_euclid(period);
    for pair in points.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let (dx, dy) = (f32::from(b.x - a.x), f32::from(b.y - a.y));
        let len = dx.hypot(dy);
        if len <= f32::EPSILON {
            continue;
        }
        let at = |d: f32| point(a.x + px(dx * d / len), a.y + px(dy * d / len));
        let mut d = 0.;
        while d < len {
            let (in_dash, left) = if phase < on {
                (true, on - phase)
            } else {
                (false, period - phase)
            };
            let step = left.min(len - d);
            if in_dash {
                path.move_to(at(d));
                path.line_to(at(d + step));
            }
            d += step;
            phase = (phase + step) % period;
        }
    }
}

/// The scale and offset (px) that fit `content` (diagram units) in a
/// minimap of `size`, centered with a small margin.
pub(super) fn minimap_fit(content: Rect, size: Size<Pixels>) -> (f32, layout::Point) {
    const MARGIN: f32 = 6.;
    let (w, h) = (
        f32::from(size.width) - 2. * MARGIN,
        f32::from(size.height) - 2. * MARGIN,
    );
    if w <= 0. || h <= 0. || content.w <= 0. || content.h <= 0. {
        return (0., layout::Point::default());
    }
    let k = (w / content.w).min(h / content.h);
    (
        k,
        layout::Point {
            x: MARGIN + (w - content.w * k) / 2. - content.x * k,
            y: MARGIN + (h - content.h * k) / 2. - content.y * k,
        },
    )
}

/// The minimap: the diagram's nodes as small boxes and the canvas' view.
pub(super) struct MiniMap {
    pub nodes: Vec<(Rect, Hsla)>,
    pub view: Rect,
    pub content: Rect,
}

impl MiniMap {
    pub fn paint(&self, bounds: Bounds<Pixels>, window: &mut Window) {
        let (k, offset) = minimap_fit(self.content, bounds.size);
        if k <= 0. {
            return;
        }
        let at = |r: &Rect| {
            Bounds::new(
                bounds.origin + point(px(r.x * k + offset.x), px(r.y * k + offset.y)),
                size(px((r.w * k).max(2.)), px((r.h * k).max(2.))),
            )
        };
        for (rect, color) in &self.nodes {
            window.paint_quad(fill(at(rect), *color).corner_radii(px(2.)));
        }
        window.paint_quad(quad(
            at(&self.view),
            px(3.),
            ui::accent().opacity(0.08),
            px(1.),
            ui::accent().opacity(0.7),
            BorderStyle::Solid,
        ));
    }
}

#[cfg(test)]
mod tests {
    use gentle_core::genui::layout::Rect;
    use gpui_kit::{px, size};

    use super::minimap_fit;

    #[test]
    fn minimap_fits_and_centers_content() {
        let content = Rect {
            x: -100.,
            y: 0.,
            w: 400.,
            h: 100.,
        };
        let (k, offset) = minimap_fit(content, size(px(212.), px(112.)));
        assert_eq!(k, 0.5);
        // Left edge at the margin, vertically centered.
        assert_eq!(content.x * k + offset.x, 6.);
        assert_eq!(content.y * k + offset.y, 6. + (100. - 50.) / 2.);
        assert_eq!(minimap_fit(Rect::default(), size(px(10.), px(10.))).0, 0.);
    }
}
