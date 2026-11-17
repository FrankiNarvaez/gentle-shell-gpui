//! Paints what sits under a diagram's nodes: group boxes, the edges with
//! their arrowheads and the connection being drawn. Nodes and labels are
//! elements on top (text, hover, clicks). Only what the canvas shows is
//! painted.

use std::rc::Rc;

use gentle_core::genui::diagram::EdgeStyle;
use gentle_core::genui::layout::{self, EdgePath, Layout, Rect};
use gpui_kit::*;

use crate::shared::ui;

/// What one frame paints, captured when the view renders.
pub(super) struct Frame {
    pub layout: Rc<Layout>,
    pub styles: Vec<EdgeStyle>,
    /// Pixels per diagram unit, and the pan (px).
    pub scale: f32,
    pub pan: Point<f32>,
    pub selected_edge: Option<usize>,
    /// From a node's handle to the pointer (diagram units).
    pub connect: Option<(layout::Point, layout::Point)>,
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
        for group in &self.layout.groups {
            let Some(rect) = group.filter(|r| r.intersects(&visible)) else {
                continue;
            };
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
                px(1.),
                ui::border(),
                BorderStyle::Solid,
            ));
        }
        for (ix, (edge, style)) in self.layout.edges.iter().zip(&self.styles).enumerate() {
            if !edge.bounds().inflate(12.).intersects(&visible) {
                continue;
            }
            self.edge(origin, edge, *style, self.selected_edge == Some(ix), window);
        }
        if let Some((from, to)) = self.connect {
            let mut path = PathBuilder::stroke(px(1.5)).dash_array(&[px(5.), px(4.)]);
            path.move_to(self.at(origin, from));
            path.line_to(self.at(origin, to));
            if let Ok(path) = path.build() {
                window.paint_path(path, ui::accent());
            }
        }
    }

    fn edge(
        &self,
        origin: Point<Pixels>,
        edge: &EdgePath,
        style: EdgeStyle,
        selected: bool,
        window: &mut Window,
    ) {
        let s = self.scale.max(0.4);
        let (color, width, dash): (Hsla, f32, Option<[f32; 2]>) = match style {
            EdgeStyle::Sync => (ui::muted(), 1.5, None),
            EdgeStyle::Async => (ui::warning(), 1.5, Some([6., 4.])),
            EdgeStyle::Data => (ui::info(), 2.25, None),
            EdgeStyle::Dependency => (ui::faint(), 1.25, Some([2., 4.])),
        };
        let color = if selected { ui::accent() } else { color };
        let width = if selected { width + 1. } else { width } * s.min(1.5);
        // Stop the line at the arrowhead's base so its end doesn't poke out.
        let head = 9. * s.min(1.5);
        let dir = edge.end_direction();
        let tip = self.at(origin, edge.to);
        let base = point(tip.x - px(dir.x * head), tip.y - px(dir.y * head));
        let mut path = PathBuilder::stroke(px(width));
        if let Some([on, off]) = dash {
            path = path.dash_array(&[px(on * s), px(off * s)]);
        }
        path.move_to(self.at(origin, edge.from));
        path.cubic_bezier_to(base, self.at(origin, edge.c1), self.at(origin, edge.c2));
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
