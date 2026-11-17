//! One diagram surface: the agent's latest spec, the user's local edits on
//! top of it, the viewport and the pointer interactions. Edits go through
//! `gentle_core::genui::changes::apply`, so what the user does is exactly
//! what the change list sent to the agent describes.

use std::cell::Cell;
use std::rc::Rc;

use gentle_core::genui::changes::{self, Change, NodePatch};
use gentle_core::genui::diagram::{Diagram, Edge, EdgeStyle, Node, NodeKind};
use gentle_core::genui::layout::{self, Layout};
use gentle_core::genui::{self, Snapshot};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;

/// Zoom limits, as a factor of the interface size.
pub(super) const MIN_ZOOM: f32 = 0.2;
pub(super) const MAX_ZOOM: f32 = 2.5;
/// The automatic fit doesn't go below this: labels stay readable.
pub(super) const READABLE_ZOOM: f32 = 0.6;
/// Undo keeps this many steps.
const UNDO_DEPTH: usize = 100;
/// A press that moves less than this (px) is a click, not a drag.
const CLICK_SLOP: f32 = 3.;
/// How close (px) a click must be to an edge to pick it.
const EDGE_PICK: f32 = 6.;

/// What a surface asks of the conversation that shows it.
pub enum DiagramEvent {
    /// Run this `/gentle-ui:event …` command line; answer with
    /// [`DiagramView::sent`].
    Send(String),
    /// Put this reference in the composer.
    Ask(String),
    Expand,
    Collapse,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Selection {
    Node(String),
    Edge(String),
}

pub(super) enum Drag {
    /// Moving the view: where the press started and the pan then.
    Pan {
        from: Point<Pixels>,
        pan: Point<f32>,
        moved: bool,
    },
    /// Moving a node: the press point inside it (world units).
    Node {
        id: String,
        grab: layout::Point,
        from: Point<Pixels>,
        moved: bool,
    },
    /// Drawing a connection from a node's handle to the pointer.
    Connect { from: String, to: layout::Point },
}

pub(super) enum SendState {
    Idle,
    Sending,
    /// The extension took the edits; says so until the next edit.
    Sent,
    Failed(String),
}

pub(super) struct Rename {
    pub target: Selection,
    pub input: Entity<InputState>,
    _subscription: Subscription,
}

/// Pan (px, from the canvas' top-left) and zoom (× interface size).
pub(super) struct Viewport {
    pub pan: Point<f32>,
    pub zoom: f32,
    /// The canvas size it was last fitted to (`None`: fit at next frame).
    pub fitted_for: Option<Size<Pixels>>,
    /// The user panned or zoomed: a resize no longer refits.
    pub touched: bool,
}

pub struct DiagramView {
    pub(super) id: String,
    pub(super) title: String,
    pub(super) version: u64,
    /// The agent's latest spec (tool result or the extension's echo of the
    /// user's last sent edits).
    pub(super) base: Diagram,
    /// `base` plus the edits not sent yet.
    pub(super) working: Diagram,
    pub(super) layout: Rc<Layout>,
    /// Changes between `base` and `working`.
    pub(super) pending: usize,
    undo: Vec<Diagram>,
    /// Why the latest spec could not be drawn (the previous one stays).
    pub(super) error: Option<String>,
    /// Why the last edit was refused.
    pub(super) refused: Option<String>,
    pub(super) viewport: Viewport,
    pub(super) selection: Option<Selection>,
    pub(super) drag: Option<Drag>,
    pub(super) rename: Option<Rename>,
    pub(super) note: Option<Entity<InputState>>,
    pub(super) send: SendState,
    /// The canvas' bounds last frame, written while painting.
    pub(super) bounds: Rc<Cell<Bounds<Pixels>>>,
    /// Shown in place of the conversation instead of in its card.
    pub(super) expanded: bool,
    pub(super) focus: FocusHandle,
}

impl EventEmitter<DiagramEvent> for DiagramView {}

impl DiagramView {
    pub fn new(snapshot: &Snapshot, cx: &mut Context<Self>) -> Self {
        let (base, error) = match Diagram::parse(&snapshot.spec) {
            Ok(diagram) => (diagram, None),
            Err(errors) => (Diagram::default(), Some(errors.join("\n"))),
        };
        Self {
            id: snapshot.id.clone(),
            title: snapshot.title.clone(),
            version: snapshot.version,
            layout: Rc::new(layout::layout(&base)),
            working: base.clone(),
            base,
            pending: 0,
            undo: Vec::new(),
            error,
            refused: None,
            viewport: Viewport {
                pan: Point::default(),
                zoom: 1.,
                fitted_for: None,
                touched: false,
            },
            selection: None,
            drag: None,
            rename: None,
            note: None,
            send: SendState::Idle,
            bounds: Rc::new(Cell::new(Bounds::default())),
            expanded: false,
            focus: cx.focus_handle(),
        }
    }

    /// A newer version from the transcript. Unsent edits are replayed on
    /// top of it (those that still apply); the viewport stays. `replaced`:
    /// the conversation was reloaded or replaced, so the same id may now
    /// carry another (or an older) version; an identical one changes nothing.
    pub fn set_snapshot(&mut self, snapshot: &Snapshot, replaced: bool, cx: &mut Context<Self>) {
        let same = snapshot.version == self.version && snapshot.title == self.title;
        if !replaced && (same || snapshot.version < self.version) {
            return;
        }
        let parsed = Diagram::parse(&snapshot.spec);
        if same && parsed.as_ref().is_ok_and(|diagram| *diagram == self.base) {
            return;
        }
        self.version = snapshot.version;
        self.title = snapshot.title.clone();
        match parsed {
            Ok(diagram) => {
                self.error = None;
                let pending = changes::diff(&self.base, &self.working);
                let mut working = diagram.clone();
                for change in &pending {
                    // An edit the new version makes impossible is dropped.
                    let _ = changes::apply(&mut working, change);
                }
                self.base = diagram;
                self.working = working;
                self.undo.clear();
            }
            Err(errors) => self.error = Some(errors.join("\n")),
        }
        // A view the user hasn't moved follows the new content.
        if !self.viewport.touched {
            self.viewport.fitted_for = None;
        }
        self.forget_missing();
        self.relayout();
        cx.notify();
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn set_expanded(&mut self, expanded: bool, cx: &mut Context<Self>) {
        if self.expanded != expanded {
            self.expanded = expanded;
            cx.notify();
        }
    }

    /// The extension's answer to [`DiagramEvent::Send`].
    pub fn sent(&mut self, result: Result<(), String>, cx: &mut Context<Self>) {
        self.send = match result {
            Ok(()) => {
                self.note = None;
                SendState::Sent
            }
            Err(error) => SendState::Failed(error),
        };
        cx.notify();
    }

    // ── Edits ──────────────────────────────────────────────────────────────

    pub(super) fn relayout(&mut self) {
        self.layout = Rc::new(layout::layout(&self.working));
        self.pending = changes::diff(&self.base, &self.working).len();
    }

    /// Drop a selection or rename whose target no longer exists.
    fn forget_missing(&mut self) {
        let exists = |selection: &Selection| match selection {
            Selection::Node(id) => self.working.node(id).is_some(),
            Selection::Edge(id) => self.working.edge(id).is_some(),
        };
        if self.selection.as_ref().is_some_and(|s| !exists(s)) {
            self.selection = None;
        }
        if self.rename.as_ref().is_some_and(|r| !exists(&r.target)) {
            self.rename = None;
        }
    }

    fn remember(&mut self, before: Diagram) {
        self.undo.push(before);
        if self.undo.len() > UNDO_DEPTH {
            self.undo.remove(0);
        }
    }

    /// Apply one user edit; a refused one leaves everything as it was.
    pub(super) fn edit(&mut self, change: Change, cx: &mut Context<Self>) -> bool {
        let before = self.working.clone();
        let applied = match changes::apply(&mut self.working, &change) {
            Ok(()) => {
                self.remember(before);
                self.refused = None;
                if matches!(self.send, SendState::Failed(_) | SendState::Sent) {
                    self.send = SendState::Idle;
                }
                self.forget_missing();
                self.relayout();
                true
            }
            Err(error) => {
                self.refused = Some(error);
                false
            }
        };
        cx.notify();
        applied
    }

    pub(super) fn undo(&mut self, cx: &mut Context<Self>) {
        if let Some(previous) = self.undo.pop() {
            self.working = previous;
            self.rename = None;
            self.refused = None;
            self.forget_missing();
            self.relayout();
            cx.notify();
        }
    }

    pub(super) fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub(super) fn discard(&mut self, cx: &mut Context<Self>) {
        if self.pending == 0 {
            return;
        }
        let before = self.working.clone();
        self.remember(before);
        self.working = self.base.clone();
        self.rename = None;
        self.refused = None;
        self.forget_missing();
        self.relayout();
        cx.notify();
    }

    pub(super) fn delete_selection(&mut self, cx: &mut Context<Self>) {
        let change = match self.selection.clone() {
            Some(Selection::Node(node)) => Change::RemoveNode { node },
            Some(Selection::Edge(edge)) => Change::RemoveEdge { edge },
            None => return,
        };
        if self.edit(change, cx) {
            self.selection = None;
        }
    }

    /// A new node in the middle of the view, selected and being named.
    pub(super) fn add_node(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let bounds = self.bounds.get();
        let center = self.to_world(
            point(bounds.size.width / 2., bounds.size.height / 2.),
            window.rem_size(),
        );
        let id = self.working.fresh_node_id("new-node");
        let mut node = Node::new(id.clone(), "New node", NodeKind::Service);
        node.x = Some((center.x - layout::NODE_W / 2.).round());
        node.y = Some((center.y - layout::NODE_H / 2.).round());
        if self.edit(Change::AddNode { node }, cx) {
            self.selection = Some(Selection::Node(id.clone()));
            self.start_rename(Selection::Node(id), window, cx);
        }
    }

    pub(super) fn connect(&mut self, from: &str, to: &str, cx: &mut Context<Self>) {
        if from == to {
            return;
        }
        let id = self.working.fresh_edge_id(from, to);
        let edge = Edge {
            id: id.clone(),
            from: from.to_string(),
            to: to.to_string(),
            label: None,
            style: EdgeStyle::default(),
        };
        if self.edit(Change::AddEdge { edge }, cx) {
            self.selection = Some(Selection::Edge(id));
        }
    }

    pub(super) fn start_rename(
        &mut self,
        target: Selection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current = match &target {
            Selection::Node(id) => self.working.node(id).map(|n| n.label.clone()),
            Selection::Edge(id) => self
                .working
                .edge(id)
                .map(|e| e.label.clone().unwrap_or_default()),
        };
        let Some(current) = current else { return };
        let input = cx.new(|cx| InputState::new(window, cx).default_value(current));
        let subscription = cx.subscribe_in(
            &input,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { .. } => {
                    this.finish_rename(true, cx);
                    window.focus(&this.focus, cx);
                }
                InputEvent::Blur => this.finish_rename(true, cx),
                _ => {}
            },
        );
        input.update(cx, |state, cx| {
            state.focus(window, cx);
            state.select_all(window, cx);
        });
        self.rename = Some(Rename {
            target,
            input,
            _subscription: subscription,
        });
        cx.notify();
    }

    pub(super) fn finish_rename(&mut self, keep: bool, cx: &mut Context<Self>) {
        let Some(rename) = self.rename.take() else {
            return;
        };
        cx.notify();
        if !keep {
            return;
        }
        let text = rename.input.read(cx).value().trim().to_string();
        let change = match rename.target {
            Selection::Node(node) => {
                let unchanged = self.working.node(&node).is_none_or(|n| n.label == text);
                if text.is_empty() || unchanged {
                    return;
                }
                Change::UpdateNode {
                    node,
                    patch: NodePatch {
                        label: Some(text),
                        ..NodePatch::default()
                    },
                }
            }
            Selection::Edge(edge) => {
                let label = (!text.is_empty()).then_some(text);
                if self.working.edge(&edge).is_none_or(|e| e.label == label) {
                    return;
                }
                Change::UpdateEdge {
                    edge,
                    label: Some(label),
                    style: None,
                }
            }
        };
        self.edit(change, cx);
    }

    /// Send the unsent edits (and the note, if any) to the agent.
    pub(super) fn send(&mut self, cx: &mut Context<Self>) {
        if self.pending == 0 || matches!(self.send, SendState::Sending) {
            return;
        }
        let (changes, summary) = changes::summarize(&self.base, &self.working);
        let note = self.note.as_ref().map(|n| n.read(cx).value().to_string());
        let command = genui::event_command(
            &self.id,
            self.version,
            &changes,
            &summary,
            &self.working,
            note.as_deref(),
        );
        self.send = SendState::Sending;
        cx.emit(DiagramEvent::Send(command));
        cx.notify();
    }

    /// "Ask about this": a reference to the selection for the composer.
    pub(super) fn ask(&mut self, cx: &mut Context<Self>) {
        let subject = match &self.selection {
            Some(Selection::Node(id)) => self
                .working
                .node(id)
                .map(|n| format!("“{}” ({id})", n.label)),
            Some(Selection::Edge(id)) => self.working.edge(id).map(|e| {
                let name = |node: &str| {
                    self.working
                        .node(node)
                        .map_or(node.to_string(), |n| n.label.clone())
                };
                format!("the connection {} → {} ({id})", name(&e.from), name(&e.to))
            }),
            None => None,
        };
        let Some(subject) = subject else { return };
        cx.emit(DiagramEvent::Ask(format!(
            "About {subject} in the “{}” diagram ({}): ",
            self.title, self.id
        )));
    }

    pub(super) fn note_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.note.is_none() {
            self.note = Some(cx.new(|cx| {
                InputState::new(window, cx).placeholder("Add a note for the agent (optional)")
            }));
        }
    }

    // ── Viewport ───────────────────────────────────────────────────────────

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

    pub(super) fn local(&self, window_position: Point<Pixels>) -> Point<Pixels> {
        window_position - self.bounds.get().origin
    }

    /// Fit the diagram in the canvas, at most 100 % and at least `floor`.
    /// Below the floor text is too small to read: the view keeps the floor
    /// and starts at the diagram's top-left on the axes that don't fit.
    pub(super) fn fit(&mut self, floor: f32, rem: Pixels) {
        let size = self.bounds.get().size;
        let (w, h) = (f32::from(size.width), f32::from(size.height));
        if w < 1. || h < 1. {
            return;
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
        self.viewport.zoom = zoom;
        self.viewport.pan = point(axis(w, content.x, content.w), axis(h, content.y, content.h));
        self.viewport.fitted_for = Some(size);
    }

    /// Zoom by `factor` keeping the diagram point under `anchor` (canvas px) still.
    pub(super) fn zoom_at(&mut self, factor: f32, anchor: Point<Pixels>, rem: Pixels) {
        let before = self.to_world(anchor, rem);
        self.viewport.zoom = (self.viewport.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let s = self.scale(rem);
        self.viewport.pan = point(
            f32::from(anchor.x) - before.x * s,
            f32::from(anchor.y) - before.y * s,
        );
        self.viewport.touched = true;
    }

    pub(super) fn zoom_center(&mut self, factor: f32, rem: Pixels) {
        let size = self.bounds.get().size;
        self.zoom_at(factor, point(size.width / 2., size.height / 2.), rem);
    }

    // ── Pointer ────────────────────────────────────────────────────────────

    /// A press on the canvas background: an edge near it is selected,
    /// otherwise the view starts panning.
    pub(super) fn press_background(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus, cx);
        self.finish_rename(true, cx);
        let rem = window.rem_size();
        let local = self.local(event.position);
        let world = self.to_world(local, rem);
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
        self.drag = Some(Drag::Pan {
            from: event.position,
            pan: self.viewport.pan,
            moved: false,
        });
        cx.notify();
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
        let rem = window.rem_size();
        let selection = Selection::Node(id.to_string());
        if event.click_count >= 2 {
            self.selection = Some(selection.clone());
            self.start_rename(selection, window, cx);
            return;
        }
        self.finish_rename(true, cx);
        let Some(ix) = self.working.node_index(id) else {
            return;
        };
        let world = self.to_world(self.local(event.position), rem);
        let rect = self.layout.nodes[ix];
        self.drag = Some(Drag::Node {
            id: id.to_string(),
            grab: layout::Point {
                x: world.x - rect.x,
                y: world.y - rect.y,
            },
            from: event.position,
            moved: false,
        });
        self.selection = Some(selection);
        cx.notify();
    }

    pub(super) fn press_handle(
        &mut self,
        id: &str,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        window.focus(&self.focus, cx);
        self.finish_rename(true, cx);
        let to = self.to_world(self.local(event.position), window.rem_size());
        self.drag = Some(Drag::Connect {
            from: id.to_string(),
            to,
        });
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
        let mut node_move = None;
        match &mut self.drag {
            Some(Drag::Pan { from, pan, moved }) => {
                *moved |= far(*from);
                let d = event.position - *from;
                self.viewport.pan = point(pan.x + f32::from(d.x), pan.y + f32::from(d.y));
                self.viewport.touched = true;
            }
            Some(Drag::Node {
                id,
                grab,
                from,
                moved,
            }) => {
                if !*moved && !far(*from) {
                    return;
                }
                let first = !*moved;
                *moved = true;
                let change = Change::Move {
                    node: id.clone(),
                    x: (world.x - grab.x).round(),
                    y: (world.y - grab.y).round(),
                };
                node_move = Some((first, change));
            }
            Some(Drag::Connect { to, .. }) => *to = world,
            None => return,
        }
        if let Some((first, change)) = node_move {
            // The whole drag is one undo step.
            if first {
                let before = self.working.clone();
                self.remember(before);
                self.refused = None;
                if matches!(self.send, SendState::Failed(_) | SendState::Sent) {
                    self.send = SendState::Idle;
                }
            }
            if changes::apply(&mut self.working, &change).is_ok() {
                self.relayout();
            }
        }
        cx.notify();
    }

    pub(super) fn released(&mut self, cx: &mut Context<Self>) {
        match self.drag.take() {
            Some(Drag::Pan { moved: false, .. }) => self.selection = None,
            Some(Drag::Connect { from, to }) => {
                if let Some(ix) = self.layout.node_at(to)
                    && let Some(target) = self.working.nodes.get(ix).map(|n| n.id.clone())
                {
                    self.connect(&from, &target, cx);
                }
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
        match keystroke.key.as_str() {
            "escape" if self.rename.is_some() => {
                self.finish_rename(false, cx);
                window.focus(&self.focus, cx);
            }
            // Keys typed into the rename or note field are theirs.
            _ if self.rename.is_some() || self.note_focused(window, cx) => return,
            "backspace" | "delete" if self.selection.is_some() => self.delete_selection(cx),
            "z" if command && !keystroke.modifiers.shift => self.undo(cx),
            "enter" if matches!(self.selection, Some(Selection::Node(_))) => {
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

    fn note_focused(&self, window: &Window, cx: &App) -> bool {
        self.note
            .as_ref()
            .is_some_and(|note| note.read(cx).focus_handle(cx).is_focused(window))
    }
}
