//! What the pointer and keyboard do on the page: draw, erase, point the
//! laser, make shapes, select (click, box, lasso), move, resize, pan, zoom,
//! and the edit commands (undo, copy, paste, duplicate, delete).

use std::time::Instant;

use gentle_core::canvas::{self, Kind, P, Rect, Shape, geom};

use super::paint::{Affine, HANDLE, LASER_LIFE};
use super::*;

/// A press that moves less than this (px) is a click.
const CLICK_SLOP: f32 = 3.;
/// How close (px) a click must come to a shape to pick it.
const PICK: f32 = 5.;
/// The eraser's radius, px.
pub(super) const ERASER: f32 = 8.;
/// A new note's side and the size a clicked (not dragged) shape gets.
const NOTE: f32 = 200.;
const CLICKED_SHAPE: f32 = 100.;

pub(super) enum Gesture {
    Pan {
        last: P,
    },
    Ink {
        points: Vec<P>,
        highlighter: bool,
    },
    Erase {
        hits: Vec<u64>,
        last: P,
    },
    Laser,
    Create {
        tool: Tool,
        start: P,
        end: P,
        /// Screen distance travelled, to tell a click.
        moved: bool,
    },
    Move {
        start: P,
        delta: P,
        moved: bool,
        /// A click on a shape already in a multi-selection picks it alone
        /// unless it turns into a drag.
        click_picks: Option<u64>,
    },
    Resize {
        /// The corner that stays put.
        anchor: P,
        from: Rect,
        to: Rect,
    },
    Box {
        start: P,
        end: P,
        base: Vec<u64>,
    },
    Lasso {
        points: Vec<P>,
    },
    Minimap,
}

impl CanvasView {
    /// The pointer in canvas px.
    pub(super) fn local(&self, position: Point<Pixels>) -> P {
        let o = self.bounds.get().origin;
        [f32::from(position.x - o.x), f32::from(position.y - o.y)]
    }

    pub(super) fn page(&self, position: Point<Pixels>) -> P {
        self.doc.camera.to_page(self.local(position))
    }

    /// The selection's box (page units).
    pub(super) fn selection_bounds(&self) -> Option<Rect> {
        canvas::bounds_of(
            self.doc
                .shapes
                .iter()
                .filter(|s| self.selected.contains(&s.id)),
        )
    }

    pub(super) fn mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus, cx);
        if self.editing.is_some() {
            self.finish_edit(window, cx);
            if !matches!(self.tool, Tool::Text | Tool::Note) {
                return;
            }
        }
        let local = self.local(event.position);
        let page = self.doc.camera.to_page(local);
        let zoom = self.doc.camera.zoom;
        if event.button == MouseButton::Middle || self.space_pan || self.tool == Tool::Hand {
            self.gesture = Some(Gesture::Pan { last: local });
            cx.notify();
            return;
        }
        if event.button != MouseButton::Left {
            return;
        }
        let shift = event.modifiers.shift;
        match self.tool {
            Tool::Select => {
                if let Some(frame) = self.selection_bounds() {
                    let screen = Affine::camera(&self.doc.camera).rect(&frame);
                    let corners = screen.corners();
                    if let Some(i) = corners.iter().position(|c| {
                        (c[0] - local[0]).abs() <= HANDLE && (c[1] - local[1]).abs() <= HANDLE
                    }) {
                        let anchor = frame.corners()[(i + 2) % 4];
                        self.gesture = Some(Gesture::Resize {
                            anchor,
                            from: frame,
                            to: frame,
                        });
                        cx.notify();
                        return;
                    }
                }
                match self.doc.hit(page, PICK / zoom) {
                    Some(id) => {
                        if event.click_count >= 2
                            && let Some(
                                Kind::Text { .. }
                                | Kind::Note { .. }
                                | Kind::Rect { .. }
                                | Kind::Ellipse { .. },
                            ) = self.doc.get(id).map(|s| &s.kind)
                        {
                            self.start_edit(EditTarget::Shape(id), window, cx);
                            return;
                        }
                        let mut click_picks = None;
                        if shift {
                            if let Some(i) = self.selected.iter().position(|s| *s == id) {
                                self.selected.remove(i);
                                cx.notify();
                                return;
                            }
                            let mut ids = self.selected.clone();
                            ids.push(id);
                            self.select(ids, cx);
                        } else if !self.selected.contains(&id) {
                            self.select(vec![id], cx);
                        } else if self.selected.len() > 1 {
                            click_picks = Some(id);
                        }
                        self.gesture = Some(Gesture::Move {
                            start: page,
                            delta: [0., 0.],
                            moved: false,
                            click_picks,
                        });
                    }
                    None => {
                        if event.click_count >= 2 {
                            self.new_text(page, window, cx);
                            return;
                        }
                        let base = if shift {
                            self.selected.clone()
                        } else {
                            Vec::new()
                        };
                        self.selected = base.clone();
                        self.gesture = Some(Gesture::Box {
                            start: page,
                            end: page,
                            base,
                        });
                    }
                }
            }
            Tool::Lasso => {
                if !shift {
                    self.selected.clear();
                }
                self.gesture = Some(Gesture::Lasso { points: vec![page] });
            }
            Tool::Hand => {}
            Tool::Pen | Tool::Highlighter => {
                self.gesture = Some(Gesture::Ink {
                    points: vec![page],
                    highlighter: self.tool == Tool::Highlighter,
                });
            }
            Tool::Eraser => {
                self.gesture = Some(Gesture::Erase {
                    hits: Vec::new(),
                    last: page,
                });
                self.erase_to(page);
            }
            Tool::Laser => {
                self.laser.push((page, Instant::now()));
                self.gesture = Some(Gesture::Laser);
            }
            Tool::Rect | Tool::Ellipse | Tool::Line | Tool::Arrow => {
                self.gesture = Some(Gesture::Create {
                    tool: self.tool,
                    start: page,
                    end: page,
                    moved: false,
                });
            }
            Tool::Text => {
                match self.doc.hit(page, PICK / zoom) {
                    Some(id)
                        if matches!(
                            self.doc.get(id).map(|s| &s.kind),
                            Some(
                                Kind::Text { .. }
                                    | Kind::Note { .. }
                                    | Kind::Rect { .. }
                                    | Kind::Ellipse { .. }
                            )
                        ) =>
                    {
                        self.start_edit(EditTarget::Shape(id), window, cx)
                    }
                    _ => self.new_text(page, window, cx),
                }
                return;
            }
            Tool::Note => {
                let rect = Rect {
                    x: page[0] - NOTE / 2.,
                    y: page[1] - NOTE / 2.,
                    w: NOTE,
                    h: NOTE,
                };
                self.start_edit(
                    EditTarget::New(Kind::Note {
                        rect,
                        text: String::new(),
                    }),
                    window,
                    cx,
                );
                return;
            }
            Tool::Image => self.pick_images(Some(page), window, cx),
        }
        cx.notify();
    }

    /// A text field at `page` for a new text shape.
    fn new_text(&mut self, page: P, window: &mut Window, cx: &mut Context<Self>) {
        let font = self.style.font_size();
        self.start_edit(
            EditTarget::New(Kind::Text {
                at: [page[0], page[1] - font * 0.65],
                text: String::new(),
                scale: 1.,
            }),
            window,
            cx,
        );
    }

    /// The pointer moved, anywhere in the window, during a gesture.
    pub(super) fn drag(
        &mut self,
        position: Point<Pixels>,
        modifiers: Modifiers,
        cx: &mut Context<Self>,
    ) {
        let local = self.local(position);
        let page = self.doc.camera.to_page(local);
        let zoom = self.doc.camera.zoom;
        self.hover = Some(local);
        let Some(gesture) = self.gesture.as_mut() else {
            return;
        };
        match gesture {
            Gesture::Pan { last } => {
                let d = [local[0] - last[0], local[1] - last[1]];
                *last = local;
                self.doc.camera.pan(d);
            }
            Gesture::Ink { points, .. } => {
                if points.last() != Some(&page) {
                    points.push(page);
                }
            }
            Gesture::Erase { .. } => self.erase_to(page),
            Gesture::Laser => self.laser.push((page, Instant::now())),
            Gesture::Create {
                tool,
                start,
                end,
                moved,
            } => {
                let mut to = page;
                if modifiers.shift {
                    to = match tool {
                        Tool::Line | Tool::Arrow => geom::snap_angle(*start, page),
                        _ => geom::square_corner(*start, page),
                    };
                }
                *end = to;
                *moved |= (end[0] - start[0]).hypot(end[1] - start[1]) * zoom > CLICK_SLOP;
            }
            Gesture::Move {
                start,
                delta,
                moved,
                ..
            } => {
                let mut d = [page[0] - start[0], page[1] - start[1]];
                if modifiers.shift {
                    if d[0].abs() > d[1].abs() {
                        d[1] = 0.;
                    } else {
                        d[0] = 0.;
                    }
                }
                *delta = d;
                *moved |= d[0].hypot(d[1]) * zoom > CLICK_SLOP;
            }
            Gesture::Resize { anchor, from, to } => {
                let mut corner = page;
                if modifiers.shift && from.w > 0. && from.h > 0. {
                    let k = ((page[0] - anchor[0]).abs() / from.w)
                        .max((page[1] - anchor[1]).abs() / from.h);
                    corner = [
                        anchor[0] + from.w * k * (page[0] - anchor[0]).signum(),
                        anchor[1] + from.h * k * (page[1] - anchor[1]).signum(),
                    ];
                }
                let mut r = Rect::from_corners(*anchor, corner);
                r.w = r.w.max(1.);
                r.h = r.h.max(1.);
                *to = r;
            }
            Gesture::Box { start, end, base } => {
                *end = page;
                let area = Rect::from_corners(*start, *end);
                let mut ids = base.clone();
                for s in &self.doc.shapes {
                    if !ids.contains(&s.id) && s.touches_box(&area) {
                        ids.push(s.id);
                    }
                }
                self.selected = ids;
            }
            Gesture::Lasso { points } => {
                if points.last() != Some(&page) {
                    points.push(page);
                }
            }
            Gesture::Minimap => {
                self.minimap_jump(position);
            }
        }
        cx.notify();
    }

    /// Marks the shapes the eraser passes over on its way to `page`.
    fn erase_to(&mut self, page: P) {
        let zoom = self.doc.camera.zoom;
        let Some(Gesture::Erase { hits, last }) = self.gesture.as_mut() else {
            return;
        };
        let r = ERASER / zoom;
        let reach = Rect::from_corners(*last, page).expand(r);
        let geo = self.geo.borrow();
        // Only shapes whose box the eraser's path comes near.
        let near: Vec<&Shape> = self
            .doc
            .shapes
            .iter()
            .filter(|s| !hits.contains(&s.id))
            .filter(|s| geo.get(&s.id).is_none_or(|g| g.bounds.intersects(&reach)))
            .collect();
        for p in geom::sample_segment(*last, page, r) {
            for s in &near {
                if !hits.contains(&s.id) && s.hit(p, r) {
                    hits.push(s.id);
                }
            }
        }
        *last = page;
    }

    pub(super) fn mouse_up(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(gesture) = self.gesture.take() else {
            return;
        };
        match gesture {
            Gesture::Pan { .. } | Gesture::Minimap => self.camera_moved(cx),
            Gesture::Laser => {}
            Gesture::Ink {
                points,
                highlighter,
            } => {
                let reach = (self.style.width() * 3.).max(14. / self.doc.camera.zoom);
                let closed = !highlighter && geom::closes_loop(&points, reach);
                let shape = Shape {
                    id: 0,
                    kind: Kind::Ink {
                        points,
                        highlighter,
                        closed,
                    },
                    style: self.style,
                };
                self.add(shape, cx);
            }
            Gesture::Erase { hits, .. } => {
                if !hits.is_empty() {
                    self.record();
                    self.doc.remove(&hits);
                    self.touched(&hits, cx);
                }
            }
            Gesture::Create {
                tool,
                start,
                end,
                moved,
            } => {
                let kind = match tool {
                    Tool::Rect | Tool::Ellipse => {
                        let rect = if moved {
                            Rect::from_corners(start, end)
                        } else {
                            Rect {
                                x: start[0] - CLICKED_SHAPE / 2.,
                                y: start[1] - CLICKED_SHAPE / 2.,
                                w: CLICKED_SHAPE,
                                h: CLICKED_SHAPE,
                            }
                        };
                        if tool == Tool::Rect {
                            Kind::Rect {
                                rect,
                                label: String::new(),
                            }
                        } else {
                            Kind::Ellipse {
                                rect,
                                label: String::new(),
                            }
                        }
                    }
                    Tool::Line if moved => Kind::Line { a: start, b: end },
                    Tool::Arrow if moved => Kind::Arrow { a: start, b: end },
                    _ => {
                        cx.notify();
                        return;
                    }
                };
                let id = self.add(
                    Shape {
                        id: 0,
                        kind,
                        style: self.style,
                    },
                    cx,
                );
                self.after_create(id, cx);
            }
            Gesture::Move {
                delta,
                moved,
                click_picks,
                ..
            } => {
                if moved {
                    self.record();
                    let ids = self.selected.clone();
                    for s in self.doc.shapes.iter_mut().filter(|s| ids.contains(&s.id)) {
                        s.translate(delta);
                    }
                    self.touched(&ids, cx);
                } else if let Some(id) = click_picks {
                    self.select(vec![id], cx);
                }
            }
            Gesture::Resize { from, to, .. } => {
                if from != to {
                    self.record();
                    let ids = self.selected.clone();
                    for s in self.doc.shapes.iter_mut().filter(|s| ids.contains(&s.id)) {
                        s.resize(&from, &to);
                    }
                    self.touched(&ids, cx);
                }
            }
            Gesture::Box { .. } => {
                let ids = self.selected.clone();
                self.select(ids, cx);
            }
            Gesture::Lasso { points } => {
                if points.len() > 2 {
                    let mut ids = self.selected.clone();
                    for s in &self.doc.shapes {
                        if !ids.contains(&s.id) && s.inside_loop(&points) {
                            ids.push(s.id);
                        }
                    }
                    if !ids.is_empty() {
                        self.tool = Tool::Select;
                    }
                    self.select(ids, cx);
                }
            }
        }
        let _ = window;
        cx.notify();
    }

    /// The pointer moved over the page with nothing pressed.
    pub(super) fn hover_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if self.gesture.is_some() {
            return;
        }
        let local = self.local(event.position);
        let was = self.hover.replace(local);
        if matches!(self.tool, Tool::Eraser | Tool::Laser) || was.is_none() {
            cx.notify();
        }
    }

    pub(super) fn scroll(
        &mut self,
        event: &ScrollEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let delta = event.delta.pixel_delta(window.line_height());
        let (dx, dy) = (f32::from(delta.x), f32::from(delta.y));
        if event.modifiers.secondary(cx) || event.modifiers.alt {
            let factor = (1. + dy / 240.).clamp(0.5, 2.);
            self.doc.camera.zoom_at(factor, self.local(event.position));
        } else if event.modifiers.shift && dx == 0. {
            self.doc.camera.pan([dy, 0.]);
        } else {
            self.doc.camera.pan([dx, dy]);
        }
        cx.stop_propagation();
        self.camera_moved(cx);
    }

    pub(super) fn pinch(&mut self, event: &PinchEvent, cx: &mut Context<Self>) {
        self.doc
            .camera
            .zoom_at(1. + event.delta, self.local(event.position));
        cx.stop_propagation();
        self.camera_moved(cx);
    }

    /// The minimap was pressed or dragged: center the view there.
    pub(super) fn minimap_jump(&mut self, position: Point<Pixels>) {
        let map = self.minimap_bounds.get();
        let Some((fit, origin)) = self.minimap_fit() else {
            return;
        };
        let local = position - map.origin;
        let page = [
            origin[0] + f32::from(local.x) / fit,
            origin[1] + f32::from(local.y) / fit,
        ];
        let (w, h) = self.size();
        let zoom = self.doc.camera.zoom;
        self.doc.camera.x = page[0] - w / 2. / zoom;
        self.doc.camera.y = page[1] - h / 2. / zoom;
    }

    // ── Keyboard ───────────────────────────────────────────────────────────

    pub(super) fn key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editing.is_some() {
            return;
        }
        let ks = &event.keystroke;
        let m = &ks.modifiers;
        if m.platform || m.control || m.alt || m.function {
            return;
        }
        let tool = match ks.key.as_str() {
            "v" => Some(Tool::Select),
            "s" => Some(Tool::Lasso),
            "h" => Some(Tool::Hand),
            "p" | "d" | "b" => Some(Tool::Pen),
            "m" => Some(Tool::Highlighter),
            "e" => Some(Tool::Eraser),
            "k" => Some(Tool::Laser),
            "r" => Some(Tool::Rect),
            "o" => Some(Tool::Ellipse),
            "a" => Some(Tool::Arrow),
            "l" => Some(Tool::Line),
            "t" => Some(Tool::Text),
            "n" => Some(Tool::Note),
            _ => None,
        };
        if let Some(tool) = tool
            && !m.shift
        {
            self.set_tool(tool, window, cx);
            cx.stop_propagation();
            return;
        }
        let step = if m.shift { 10. } else { 1. };
        let nudge = match ks.key.as_str() {
            "left" => Some([-step, 0.]),
            "right" => Some([step, 0.]),
            "up" => Some([0., -step]),
            "down" => Some([0., step]),
            _ => None,
        };
        if let Some(d) = nudge {
            if !self.selected.is_empty() {
                self.record();
                let ids = self.selected.clone();
                for s in self.doc.shapes.iter_mut().filter(|s| ids.contains(&s.id)) {
                    s.translate(d);
                }
                self.touched(&ids, cx);
            } else {
                let z = self.doc.camera.zoom;
                self.doc.camera.pan([-d[0] * 4. * z, -d[1] * 4. * z]);
                self.camera_moved(cx);
            }
            cx.stop_propagation();
            return;
        }
        match (ks.key.as_str(), m.shift) {
            ("space", _) => {
                if !self.space_pan {
                    self.space_pan = true;
                    cx.notify();
                }
            }
            ("=" | "+", _) => self.zoom_center(1.25, cx),
            ("-", false) => self.zoom_center(0.8, cx),
            ("1", true) => self.fit(cx),
            ("0", true) => self.zoom_to(1., cx),
            ("enter", false) => {
                if let [id] = self.selected[..]
                    && matches!(
                        self.doc.get(id).map(|s| &s.kind),
                        Some(
                            Kind::Text { .. }
                                | Kind::Note { .. }
                                | Kind::Rect { .. }
                                | Kind::Ellipse { .. }
                        )
                    )
                {
                    self.start_edit(EditTarget::Shape(id), window, cx);
                }
            }
            ("]", _) => self.reorder(true, cx),
            ("[", _) => self.reorder(false, cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    pub(super) fn key_up(&mut self, event: &KeyUpEvent, cx: &mut Context<Self>) {
        if event.keystroke.key == "space" && self.space_pan {
            self.space_pan = false;
            cx.notify();
        }
    }

    pub(super) fn reorder(&mut self, to_front: bool, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            return;
        }
        self.record();
        self.doc.reorder(&self.selected, to_front);
        self.touched(&[], cx);
    }

    // ── Commands ───────────────────────────────────────────────────────────

    pub(super) fn undo(&mut self, _: &Undo, window: &mut Window, cx: &mut Context<Self>) {
        self.finish_edit(window, cx);
        if self.history.undo(&mut self.doc.shapes) {
            self.touched_all(cx);
        }
    }

    pub(super) fn redo(&mut self, _: &Redo, window: &mut Window, cx: &mut Context<Self>) {
        self.finish_edit(window, cx);
        if self.history.redo(&mut self.doc.shapes) {
            self.touched_all(cx);
        }
    }

    pub(super) fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.tool = Tool::Select;
        let ids = self.doc.shapes.iter().map(|s| s.id).collect();
        self.select(ids, cx);
    }

    pub(super) fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            return;
        }
        self.record();
        let ids = std::mem::take(&mut self.selected);
        self.doc.remove(&ids);
        self.touched(&ids, cx);
    }

    pub(super) fn cancel(&mut self, _: &Cancel, window: &mut Window, cx: &mut Context<Self>) {
        if self.gesture.take().is_some() {
        } else if !self.selected.is_empty() {
            self.selected.clear();
        } else if self.tool != Tool::Select {
            self.set_tool(Tool::Select, window, cx);
        } else {
            cx.propagate();
        }
        cx.notify();
    }

    pub(super) fn duplicate(&mut self, _: &Duplicate, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            return;
        }
        self.record();
        let ids = self.doc.duplicate(&self.selected, [16., 16.]);
        self.touched(&ids, cx);
        self.select(ids, cx);
    }

    fn selected_shapes(&self) -> Vec<Shape> {
        self.doc
            .shapes
            .iter()
            .filter(|s| self.selected.contains(&s.id))
            .cloned()
            .collect()
    }

    pub(super) fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        let shapes = self.selected_shapes();
        if !shapes.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(canvas::to_clipboard(&shapes)));
        }
    }

    pub(super) fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        self.copy(&Copy, window, cx);
        self.delete(&Delete, window, cx);
    }

    /// Shapes copied from a canvas paste as shapes (in place, or in the
    /// middle of the view when that place is out of sight); images as
    /// images; other text as a text shape.
    pub(super) fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = cx.read_from_clipboard() else {
            return;
        };
        let entries = item.entries();
        // An image copied from a page also carries its address as text.
        if let Some(image) = entries.iter().find_map(|e| match e {
            ClipboardEntry::Image(image) => Some(image),
            _ => None,
        }) {
            self.insert_image_bytes(image, cx);
            return;
        }
        if let Some(paths) = entries.iter().find_map(|e| match e {
            ClipboardEntry::ExternalPaths(paths) => Some(paths),
            _ => None,
        }) {
            self.insert_images(paths.paths().to_vec(), None, cx);
            return;
        }
        let Some(text) = item.text() else {
            return;
        };
        if let Some(shapes) = canvas::from_clipboard(&text) {
            let (w, h) = self.size();
            let view = self.doc.camera.visible(w, h);
            let offset = match canvas::bounds_of(shapes.iter()) {
                Some(b) if view.intersects(&b) => [16., 16.],
                Some(b) => {
                    let (c, v) = (b.center(), view.center());
                    [v[0] - c[0], v[1] - c[1]]
                }
                None => [0., 0.],
            };
            self.record();
            let ids = self.doc.paste(shapes, offset);
            self.touched(&ids, cx);
            self.tool = Tool::Select;
            self.select(ids, cx);
        } else if !text.trim().is_empty() {
            let at = self.view_center();
            let id = self.add(
                Shape {
                    id: 0,
                    kind: Kind::Text {
                        at,
                        text: text.trim_end().to_string(),
                        scale: 1.,
                    },
                    style: self.style,
                },
                cx,
            );
            self.tool = Tool::Select;
            self.select(vec![id], cx);
        }
    }

    /// The laser trail with each point's age; old points dropped.
    pub(super) fn laser_trail(&mut self) -> Vec<(P, f32)> {
        self.laser.retain(|(_, t)| paint::age(*t) < LASER_LIFE);
        self.laser
            .iter()
            .map(|(p, t)| (*p, paint::age(*t)))
            .collect()
    }
}
