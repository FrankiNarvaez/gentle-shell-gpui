//! A diagram's viewport (pan, zoom, fit, minimap) and what the pointer and
//! keyboard do on it: select, box-select, drag nodes and groups, connect
//! from a handle, rename, delete, undo and redo.

use std::time::Instant;

use gentle_core::genui::changes::{self, Change, NodePatch};
use gentle_core::genui::layout::{self, Rect};
use gpui_kit::*;

use super::diagram::{DiagramEvent, DiagramView, Drag, MAX_ZOOM, MIN_ZOOM, Selection, snap};
use super::motion::ViewTween;
use super::paint::minimap_fit;

/// A press that moves less than this (px) is a click, not a drag.
const CLICK_SLOP: f32 = 3.;
/// How close (px) a click must be to an edge to pick it.
const EDGE_PICK: f32 = 6.;
/// Zoom buttons' step.
pub(super) const ZOOM_STEP: f32 = 1.25;

impl DiagramView {
    /// Pixels per diagram unit: the zoom times the interface size.
    pub(super) fn scale(&self, rem: Pixels) -> f32 {
        self.viewport.zoom * f32::from(rem) / 16.
    }

    /// A point on the canvas (relative to its top-left) in diagram units.
    pub(super) fn to_world(&self, local: Point<Pixels>, rem: Pixels) -> layout::Point {
        let s = self.scale(rem);
        layout::Point {
            x: (f32::from(local.x) - self.viewport.pan.x) / s,
            y: (f32::from(local.y) - self.viewport.pan.y) / s,
        }
    }

    /// A diagram point on the window.
    #[cfg(test)]
    pub(super) fn to_window(&self, p: layout::Point, rem: Pixels) -> Point<Pixels> {
        let s = self.scale(rem);
        self.bounds.get().origin
            + point(
                px(p.x * s + self.viewport.pan.x),
                px(p.y * s + self.viewport.pan.y),
            )
    }

    pub(super) fn local(&self, window_position: Point<Pixels>) -> Point<Pixels> {
        window_position - self.bounds.get().origin
    }

    /// The canvas in diagram units.
    pub(super) fn visible(&self, rem: Pixels) -> Rect {
        let s = self.scale(rem);
        let size = self.bounds.get().size;
        Rect {
            x: -self.viewport.pan.x / s,
            y: -self.viewport.pan.y / s,
            w: f32::from(size.width) / s,
            h: f32::from(size.height) / s,
        }
    }

    /// The pan and zoom that fit the diagram in the canvas, at most 100 %
    /// and at least `floor`. Below the floor text is too small to read: the
    /// view keeps the floor and starts at the diagram's top-left on the
    /// axes that don't fit.
    fn fit_target(&self, floor: f32, rem: Pixels) -> Option<(Point<f32>, f32)> {
        let size = self.bounds.get().size;
        let (w, h) = (f32::from(size.width), f32::from(size.height));
        if w < 1. || h < 1. {
            return None;
        }
        let content = self.layout.bounds.inflate(24.);
        let unit = f32::from(rem) / 16.;
        let zoom = if content.w <= 0. || content.h <= 0. {
            1.
        } else {
            (w / content.w).min(h / content.h) / unit
        }
        .clamp(floor, 1.);
        let s = zoom * unit;
        let axis = |view: f32, start: f32, length: f32| {
            if length * s <= view {
                view / 2. - (start + length / 2.) * s
            } else {
                -start * s
            }
        };
        Some((
            point(axis(w, content.x, content.w), axis(h, content.y, content.h)),
            zoom,
        ))
    }

    /// Fit at once (the first frame, a resize, new content).
    pub(super) fn fit(&mut self, floor: f32, rem: Pixels) {
        if let Some((pan, zoom)) = self.fit_target(floor, rem) {
            self.view_tween = None;
            self.viewport.pan = pan;
            self.viewport.zoom = zoom;
            self.viewport.fitted_for = Some(self.bounds.get().size);
        }
    }

    /// The fit button: ease there.
    pub(super) fn fit_animated(&mut self, rem: Pixels, cx: &App) {
        if let Some(target) = self.fit_target(MIN_ZOOM, rem) {
            self.viewport.touched = false;
            self.viewport.fitted_for = Some(self.bounds.get().size);
            self.ease_view_to(target, cx);
        }
    }

    fn ease_view_to(&mut self, target: (Point<f32>, f32), cx: &App) {
        if cx.reduce_motion() {
            self.view_tween = None;
            (self.viewport.pan, self.viewport.zoom) = target;
        } else {
            let from = (self.viewport.pan, self.viewport.zoom);
            self.view_tween = Some(ViewTween::new(from, target, Instant::now()));
        }
    }

    /// Advance the viewport's tween to `now`; whether it still runs.
    pub(super) fn advance_view(&mut self, now: Instant) -> bool {
        let Some(tween) = &self.view_tween else {
            return false;
        };
        let ((pan, zoom), done) = tween.at(now);
        self.viewport.pan = pan;
        self.viewport.zoom = zoom;
        if done {
            self.view_tween = None;
        }
        !done
    }

    /// Jump every running animation to its end.
    #[cfg(test)]
    pub(super) fn settle(&mut self) {
        if let Some(tween) = self.view_tween.take() {
            (self.viewport.pan, self.viewport.zoom) = tween.target();
        }
        self.arrival = None;
    }

    /// Zoom by `factor` keeping the diagram point under `anchor` (canvas
    /// px) still, at once (wheel, pinch).
    pub(super) fn zoom_at(&mut self, factor: f32, anchor: Point<Pixels>, rem: Pixels) {
        self.view_tween = None;
        let before = self.to_world(anchor, rem);
        self.viewport.zoom = (self.viewport.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let s = self.scale(rem);
        self.viewport.pan = point(
            f32::from(anchor.x) - before.x * s,
            f32::from(anchor.y) - before.y * s,
        );
        self.viewport.touched = true;
    }

    /// The zoom buttons: ease around the canvas' center (from where a
    /// running tween was heading, so quick clicks add up).
    pub(super) fn zoom_center(&mut self, factor: f32, rem: Pixels, cx: &App) {
        let (pan, zoom) = self
            .view_tween
            .as_ref()
            .map_or((self.viewport.pan, self.viewport.zoom), ViewTween::target);
        let size = self.bounds.get().size;
        let anchor = point(f32::from(size.width) / 2., f32::from(size.height) / 2.);
        let unit = f32::from(rem) / 16.;
        let world = point(
            (anchor.x - pan.x) / (zoom * unit),
            (anchor.y - pan.y) / (zoom * unit),
        );
        let zoom = (zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let s = zoom * unit;
        let pan = point(anchor.x - world.x * s, anchor.y - world.y * s);
        self.viewport.touched = true;
        self.ease_view_to((pan, zoom), cx);
    }

    /// Center the canvas on the minimap point under `position` (window px).
    fn navigate_minimap(&mut self, position: Point<Pixels>, rem: Pixels) {
        let map = self.minimap.get();
        let content = self.layout.bounds.union(&self.visible(rem));
        let (k, offset) = minimap_fit(content, map.size);
        if k <= 0. {
            return;
        }
        let local = position - map.origin;
        let world = layout::Point {
            x: (f32::from(local.x) - offset.x) / k,
            y: (f32::from(local.y) - offset.y) / k,
        };
        let s = self.scale(rem);
        let size = self.bounds.get().size;
        self.view_tween = None;
        self.viewport.pan = point(
            f32::from(size.width) / 2. - world.x * s,
            f32::from(size.height) / 2. - world.y * s,
        );
        self.viewport.touched = true;
    }

    /// Commit what the rename and details fields hold before the pointer
    /// does something else.
    fn settle_inputs(&mut self, cx: &mut Context<Self>) {
        self.finish_rename(true, cx);
        self.commit_fields(cx);
    }

    // ── Pointer ────────────────────────────────────────────────────────────

    /// A press on the canvas background: shift starts a selection box; an
    /// edge near it is selected; otherwise the view starts panning.
    pub(super) fn press_background(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus, cx);
        self.settle_inputs(cx);
        let rem = window.rem_size();
        let world = self.to_world(self.local(event.position), rem);
        if event.modifiers.shift {
            self.drag = Some(Drag::Select {
                from: world,
                to: world,
            });
            cx.notify();
            return;
        }
        let tolerance = EDGE_PICK / self.scale(rem);
        if let Some(ix) = self.layout.edge_at(world, tolerance)
            && let Some(edge) = self.working.edges.get(ix)
        {
            let selection = Selection::Edge(edge.id.clone());
            if event.click_count >= 2 {
                self.start_rename(selection.clone(), window, cx);
            }
            self.selection = Some(selection);
            cx.notify();
            return;
        }
        self.view_tween = None;
        self.drag = Some(Drag::Pan {
            from: event.position,
            pan: self.viewport.pan,
            moved: false,
        });
        cx.notify();
    }

    /// Start moving `nodes` from the press at `event`.
    fn grab(&mut self, nodes: &[String], event: &MouseDownEvent, rem: Pixels) {
        if self.locked {
            return;
        }
        let starts = nodes
            .iter()
            .filter_map(|id| {
                let r = self.layout.nodes.get(self.working.node_index(id)?)?;
                Some((id.clone(), layout::Point { x: r.x, y: r.y }))
            })
            .collect();
        self.drag = Some(Drag::Nodes {
            starts,
            before: self.layout.clone(),
            grab: self.to_world(self.local(event.position), rem),
            from: event.position,
            moved: false,
        });
    }

    pub(super) fn press_node(
        &mut self,
        id: &str,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        window.focus(&self.focus, cx);
        let selection = Selection::Node(id.to_string());
        if event.click_count >= 2 {
            self.selection = Some(selection.clone());
            self.start_rename(selection, window, cx);
            return;
        }
        self.settle_inputs(cx);
        let mut nodes = self
            .selection
            .as_ref()
            .map(|s| s.nodes().to_vec())
            .unwrap_or_default();
        if event.modifiers.shift {
            match nodes.iter().position(|n| n == id) {
                Some(i) => {
                    nodes.remove(i);
                }
                None => nodes.push(id.to_string()),
            }
            self.selection = Selection::of_nodes(nodes);
            cx.notify();
            return;
        }
        // Pressing one of several selected nodes drags them all.
        if !nodes.iter().any(|n| n == id) {
            nodes = vec![id.to_string()];
            self.selection = Some(selection);
        }
        self.grab(&nodes, event, window.rem_size());
        cx.notify();
    }

    /// A press on a group's title: select it (rename on double-click) and
    /// drag its members.
    pub(super) fn press_group(
        &mut self,
        id: &str,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        window.focus(&self.focus, cx);
        let selection = Selection::Group(id.to_string());
        if event.click_count >= 2 {
            self.selection = Some(selection.clone());
            self.start_rename(selection, window, cx);
            return;
        }
        self.settle_inputs(cx);
        self.selection = Some(selection);
        let members: Vec<String> = self
            .working
            .nodes
            .iter()
            .filter(|n| n.group.as_deref() == Some(id))
            .map(|n| n.id.clone())
            .collect();
        self.grab(&members, event, window.rem_size());
        cx.notify();
    }

    pub(super) fn press_handle(
        &mut self,
        id: &str,
        outgoing: bool,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        window.focus(&self.focus, cx);
        self.settle_inputs(cx);
        if self.locked {
            return;
        }
        let to = self.to_world(self.local(event.position), window.rem_size());
        self.drag = Some(Drag::Connect {
            node: id.to_string(),
            outgoing,
            to,
        });
        cx.notify();
    }

    pub(super) fn press_minimap(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        window.focus(&self.focus, cx);
        self.navigate_minimap(event.position, window.rem_size());
        self.drag = Some(Drag::MiniMap);
        cx.notify();
    }

    pub(super) fn pointer_moved(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let rem = window.rem_size();
        let world = self.to_world(self.local(event.position), rem);
        let far = |from: Point<Pixels>| {
            let d = event.position - from;
            f32::from(d.x).hypot(f32::from(d.y)) >= CLICK_SLOP
        };
        let mut moves = None;
        match &mut self.drag {
            Some(Drag::Pan { from, pan, moved }) => {
                *moved |= far(*from);
                let d = event.position - *from;
                self.viewport.pan = point(pan.x + f32::from(d.x), pan.y + f32::from(d.y));
                self.viewport.touched = true;
            }
            Some(Drag::Nodes {
                starts,
                grab,
                from,
                moved,
                ..
            }) => {
                if !*moved && !far(*from) {
                    return;
                }
                let first = !*moved;
                *moved = true;
                let (dx, dy) = (world.x - grab.x, world.y - grab.y);
                let changes: Vec<Change> = starts
                    .iter()
                    .map(|(node, start)| Change::Move {
                        node: node.clone(),
                        x: snap(start.x + dx),
                        y: snap(start.y + dy),
                    })
                    .collect();
                moves = Some((first, changes));
            }
            Some(Drag::Connect { to, .. }) | Some(Drag::Select { to, .. }) => *to = world,
            Some(Drag::MiniMap) => self.navigate_minimap(event.position, rem),
            None => {
                // Hover: highlight the edge under the pointer.
                let tolerance = EDGE_PICK / self.scale(rem);
                let hover = self
                    .layout
                    .node_at(world)
                    .is_none()
                    .then(|| self.layout.edge_at(world, tolerance))
                    .flatten();
                if hover != self.hover_edge {
                    self.hover_edge = hover;
                    cx.notify();
                }
                return;
            }
        }
        if let Some((first, changes)) = moves {
            // The whole drag is one undo step.
            if first {
                let before = self.working.clone();
                self.remember(before);
                self.edited();
            }
            for change in &changes {
                let _ = changes::apply(&mut self.working, change);
            }
            self.relayout();
        }
        cx.notify();
    }

    pub(super) fn released(&mut self, cx: &mut Context<Self>) {
        match self.drag.take() {
            Some(Drag::Pan { moved: false, .. }) => self.selection = None,
            Some(Drag::Nodes {
                starts,
                before,
                moved: true,
                ..
            }) => {
                // Dropped into or out of a group: part of the same undo step.
                let mut regrouped = false;
                for (node, _) in &starts {
                    let after = self.layout.clone();
                    if let Some(group) =
                        changes::group_on_drop(&self.working, &before, &after, node)
                    {
                        let change = Change::UpdateNode {
                            node: node.clone(),
                            patch: NodePatch {
                                group: Some(group),
                                ..NodePatch::default()
                            },
                        };
                        regrouped |= changes::apply(&mut self.working, &change).is_ok();
                    }
                }
                if regrouped {
                    self.relayout();
                }
            }
            Some(Drag::Connect { node, outgoing, to }) => {
                if let Some(ix) = self.layout.node_at(to)
                    && let Some(other) = self.working.nodes.get(ix).map(|n| n.id.clone())
                {
                    if outgoing {
                        self.connect(&node, &other, cx);
                    } else {
                        self.connect(&other, &node, cx);
                    }
                }
            }
            Some(Drag::Select { from, to }) => {
                let area = Rect {
                    x: from.x.min(to.x),
                    y: from.y.min(to.y),
                    w: (to.x - from.x).abs(),
                    h: (to.y - from.y).abs(),
                };
                let picked = self
                    .working
                    .nodes
                    .iter()
                    .zip(&self.layout.nodes)
                    .filter(|(_, r)| r.intersects(&area))
                    .map(|(n, _)| n.id.clone())
                    .collect();
                self.selection = Selection::of_nodes(picked);
            }
            _ => {}
        }
        cx.notify();
    }

    pub(super) fn scrolled(
        &mut self,
        event: &ScrollEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let rem = window.rem_size();
        let delta = event.delta.pixel_delta(rem);
        if event.modifiers.secondary(cx) {
            // ⌘ / Ctrl + wheel zooms around the pointer.
            let factor = (1. + f32::from(delta.y) / 300.).clamp(0.5, 2.);
            self.zoom_at(factor, self.local(event.position), rem);
        } else if self.expanded {
            self.view_tween = None;
            self.viewport.pan.x += f32::from(delta.x);
            self.viewport.pan.y += f32::from(delta.y);
            self.viewport.touched = true;
        } else {
            // In the conversation a plain wheel scrolls the conversation.
            return;
        }
        cx.stop_propagation();
        cx.notify();
    }

    pub(super) fn pinched(
        &mut self,
        event: &PinchEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let rem = window.rem_size();
        self.zoom_at(1. + event.delta, self.local(event.position), rem);
        cx.stop_propagation();
        cx.notify();
    }

    pub(super) fn key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let keystroke = &event.keystroke;
        let command = keystroke.modifiers.secondary(cx);
        let shift = keystroke.modifiers.shift;
        match keystroke.key.as_str() {
            "escape" if self.rename.is_some() => {
                self.finish_rename(false, cx);
                window.focus(&self.focus, cx);
            }
            // Keys typed into the rename, note or details fields are theirs.
            _ if self.rename.is_some() || self.typing(window, cx) => return,
            "backspace" | "delete" if self.selection.is_some() => self.delete_selection(cx),
            "z" if command && !shift => self.undo(cx),
            "z" if command && shift => self.redo(cx),
            "y" if command => self.redo(cx),
            "a" if command => {
                let all = self.working.nodes.iter().map(|n| n.id.clone()).collect();
                self.selection = Selection::of_nodes(all);
                cx.notify();
            }
            "enter"
                if matches!(
                    self.selection,
                    Some(Selection::Node(_) | Selection::Group(_))
                ) =>
            {
                if let Some(selection) = self.selection.clone() {
                    self.start_rename(selection, window, cx);
                }
            }
            "escape" if self.selection.is_some() => {
                self.selection = None;
                cx.notify();
            }
            "escape" if self.expanded => cx.emit(DiagramEvent::Collapse),
            _ => return,
        }
        cx.stop_propagation();
    }

    /// Whether the note or a details field has the keyboard.
    fn typing(&self, window: &Window, cx: &App) -> bool {
        let focused = |input: &Entity<gpui_kit::component::input::InputState>| {
            input.read(cx).focus_handle(cx).is_focused(window)
        };
        self.note.as_ref().is_some_and(focused)
            || self
                .fields
                .as_ref()
                .is_some_and(|f| focused(&f.description) || f.tech.as_ref().is_some_and(focused))
    }
}
