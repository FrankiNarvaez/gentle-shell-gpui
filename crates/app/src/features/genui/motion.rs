//! What moves on a generative UI surface and when it may.
//!
//! - Diagrams: the agent's new version easing in over the previous one
//!   ([`Arrival`]), the viewport easing to a fit or zoom ([`ViewTween`]), and
//!   the marching dashes of async and data edges; see [`frame_delay`].
//! - Panels: a value tween (charts, [`Tween`]) and glides (rows and cards
//!   sliding to a new place, new ones fading in, [`Glides`]).
//!
//! Everything runs only while something moves, and nothing with reduced
//! motion or while backdrop motion is Still ([`may_move`]): each frame asks
//! for the next one only when it still has motion to draw, so an idle or
//! off-screen card costs nothing.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use gentle_core::genui::diagram::{Diagram, EdgeStyle, Node};
use gentle_core::genui::layout::{Cubic, EdgePath, Layout, Point as P, Rect};
// Not a glob: GPUI exports a `test` attribute that would shadow `#[test]`.
use gpui_kit::{App, Point, Styled, Window, ease_out_quint, point, rems};

use crate::shared::backdrop::{self, Motion};
use crate::shared::ui;

/// Whether looping or decorative motion may run: the window is active, the
/// system doesn't ask for reduced motion and backdrop motion isn't Still.
pub(super) fn may_move(window_active: bool, reduce_motion: bool, motion: Motion) -> bool {
    window_active && !reduce_motion && motion != Motion::Still
}

/// [`may_move`] for `window` now.
pub(super) fn allowed(window: &Window, cx: &App) -> bool {
    may_move(
        window.is_window_active(),
        cx.reduce_motion(),
        backdrop::motion_now(cx),
    )
}

/// How long the agent's update takes to ease in.
const ARRIVAL: Duration = Duration::from_millis(300);
/// How long a fit or a zoom button takes.
const VIEW: Duration = Duration::from_millis(260);
/// The frame pace while something tweens.
const TWEEN_FRAME: Duration = Duration::from_millis(16);
/// Marching dashes' pace by motion setting (~30 and ~12 fps).
const MARCH_SMOOTH: Duration = Duration::from_millis(33);
const MARCH_CALM: Duration = Duration::from_millis(83);
/// How fast dashes march (diagram units per second).
pub(super) const MARCH_SPEED: f32 = 24.;

pub(super) fn ease_out(t: f32) -> f32 {
    1. - (1. - t.clamp(0., 1.)).powi(3)
}

fn fraction(start: Instant, length: Duration, now: Instant) -> f32 {
    (now.saturating_duration_since(start).as_secs_f32() / length.as_secs_f32()).min(1.)
}

/// The next frame a diagram needs, or `None` to stay idle. Tweens run to
/// their end (they are short); marching dashes only while some are on
/// screen, the window is active and motion is allowed. Reduced motion
/// stops everything.
pub(super) fn frame_delay(
    tweening: bool,
    marching_on_screen: bool,
    window_active: bool,
    reduce_motion: bool,
    motion: Motion,
) -> Option<Duration> {
    if reduce_motion {
        return None;
    }
    if tweening {
        return Some(TWEEN_FRAME);
    }
    if !marching_on_screen || !may_move(window_active, reduce_motion, motion) {
        return None;
    }
    Some(match motion {
        Motion::Smooth => MARCH_SMOOTH,
        _ => MARCH_CALM,
    })
}

/// A fit or zoom button easing the viewport.
pub(super) struct ViewTween {
    start: Instant,
    from: (Point<f32>, f32),
    to: (Point<f32>, f32),
}

impl ViewTween {
    pub fn new(from: (Point<f32>, f32), to: (Point<f32>, f32), now: Instant) -> Self {
        Self {
            start: now,
            from,
            to,
        }
    }

    pub fn target(&self) -> (Point<f32>, f32) {
        self.to
    }

    /// The pan and zoom at `now`, and whether it's over.
    pub fn at(&self, now: Instant) -> ((Point<f32>, f32), bool) {
        let t = fraction(self.start, VIEW, now);
        let e = ease_out(t);
        let mix = |a: f32, b: f32| a + (b - a) * e;
        let pan = point(
            mix(self.from.0.x, self.to.0.x),
            mix(self.from.0.y, self.to.0.y),
        );
        ((pan, mix(self.from.1, self.to.1)), t >= 1.)
    }
}

/// The agent's new version easing in: kept nodes slide from where they
/// were, new ones fade and grow in, removed ones fade out where they were.
pub(super) struct Arrival {
    start: Instant,
    nodes: HashMap<String, Rect>,
    edges: HashMap<String, EdgePath>,
    pub leaving_nodes: Vec<(Node, Rect)>,
    pub leaving_edges: Vec<(EdgePath, EdgeStyle)>,
}

impl Arrival {
    /// None when nothing would move.
    pub fn between(
        old: &Diagram,
        old_layout: &Layout,
        new: &Diagram,
        new_layout: &Layout,
    ) -> Option<Self> {
        Self::between_at(old, old_layout, new, new_layout, Instant::now())
    }

    fn between_at(
        old: &Diagram,
        old_layout: &Layout,
        new: &Diagram,
        new_layout: &Layout,
        now: Instant,
    ) -> Option<Self> {
        let nodes: HashMap<String, Rect> = old
            .nodes
            .iter()
            .zip(&old_layout.nodes)
            .map(|(n, r)| (n.id.clone(), *r))
            .collect();
        let edges: HashMap<String, EdgePath> = old
            .edges
            .iter()
            .zip(&old_layout.edges)
            .map(|(e, p)| (e.id.clone(), p.clone()))
            .collect();
        let leaving_nodes: Vec<(Node, Rect)> = old
            .nodes
            .iter()
            .zip(&old_layout.nodes)
            .filter(|(n, _)| new.node(&n.id).is_none())
            .map(|(n, r)| (n.clone(), *r))
            .collect();
        let leaving_edges: Vec<(EdgePath, EdgeStyle)> = old
            .edges
            .iter()
            .zip(&old_layout.edges)
            .filter(|(e, _)| new.edge(&e.id).is_none())
            .map(|(e, p)| (p.clone(), e.style))
            .collect();
        let moves = new
            .nodes
            .iter()
            .zip(&new_layout.nodes)
            .any(|(n, r)| nodes.get(&n.id) != Some(r));
        (moves || !leaving_nodes.is_empty() || !leaving_edges.is_empty()).then_some(Self {
            start: now,
            nodes,
            edges,
            leaving_nodes,
            leaving_edges,
        })
    }

    /// Eased progress at `now`, 0..=1.
    pub fn progress(&self, now: Instant) -> f32 {
        ease_out(fraction(self.start, ARRIVAL, now))
    }

    pub fn done(&self, now: Instant) -> bool {
        fraction(self.start, ARRIVAL, now) >= 1.
    }

    /// Where node `id` shows at progress `p` and how opaque.
    pub fn node(&self, id: &str, target: Rect, p: f32) -> (Rect, f32) {
        match self.nodes.get(id) {
            Some(from) => (lerp_rect(*from, target, p), 1.),
            // New: grows from 92 % around its center.
            None => {
                let k = 0.92 + 0.08 * p;
                let c = target.center();
                let (w, h) = (target.w * k, target.h * k);
                (
                    Rect {
                        x: c.x - w / 2.,
                        y: c.y - h / 2.,
                        w,
                        h,
                    },
                    p,
                )
            }
        }
    }

    /// Edge `id`'s path at progress `p` and how opaque: morphed from its
    /// old path when both have as many segments, faded in when new.
    pub fn edge(&self, id: &str, target: &EdgePath, p: f32) -> (EdgePath, f32) {
        match self.edges.get(id) {
            Some(from) if from.segments.len() == target.segments.len() => {
                let segments = from
                    .segments
                    .iter()
                    .zip(&target.segments)
                    .map(|(a, b)| Cubic {
                        from: lerp(a.from, b.from, p),
                        c1: lerp(a.c1, b.c1, p),
                        c2: lerp(a.c2, b.c2, p),
                        to: lerp(a.to, b.to, p),
                    })
                    .collect();
                (EdgePath { segments }, 1.)
            }
            Some(_) => (target.clone(), 1.),
            None => (target.clone(), p),
        }
    }
}

fn lerp(a: P, b: P, t: f32) -> P {
    P {
        x: a.x + (b.x - a.x) * t,
        y: a.y + (b.y - a.y) * t,
    }
}

fn lerp_rect(a: Rect, b: Rect, t: f32) -> Rect {
    Rect {
        x: a.x + (b.x - a.x) * t,
        y: a.y + (b.y - a.y) * t,
        w: a.w + (b.w - a.w) * t,
        h: a.h + (b.h - a.h) * t,
    }
}

/// Values tweening to a new version.
const TWEEN: Duration = Duration::from_millis(300);
/// Rows and cards gliding to their new place.
const GLIDE: Duration = Duration::from_millis(340);
/// How far (px at 100 %) a new row rises while it fades in.
const RISE: f32 = 6.;

/// One 0 → 1 run (eased out), restarted by [`Tween::start`].
#[derive(Default)]
pub(super) struct Tween {
    started: Option<Instant>,
}

impl Tween {
    pub fn start(&mut self) {
        self.started = Some(Instant::now());
    }

    /// Where the run is (1 when it isn't running); asks for the next frame
    /// while it runs.
    pub fn progress(&mut self, window: &Window, cx: &App) -> f32 {
        let Some(started) = self.started else {
            return 1.;
        };
        let t = started.elapsed().as_secs_f32() / TWEEN.as_secs_f32();
        if t >= 1. || !allowed(window, cx) {
            self.started = None;
            return 1.;
        }
        window.request_animation_frame();
        ease_out_quint()(t)
    }
}

struct Glide {
    /// Where it is drawn when the glide starts, from its place (px at 100 %).
    from: (f32, f32),
    fade: bool,
    started: Instant,
}

impl Glide {
    fn t(&self, now: Instant) -> f32 {
        (now.duration_since(self.started).as_secs_f32() / GLIDE.as_secs_f32()).min(1.)
    }

    fn offset(&self, now: Instant) -> (f32, f32) {
        // The spring settles with a little overshoot.
        let left = 1. - ui::spring(self.t(now));
        (self.from.0 * left, self.from.1 * left)
    }
}

/// Things (rows, cards) gliding from their old place to their new one, by id.
#[derive(Default)]
pub(super) struct Glides {
    moving: HashMap<String, Glide>,
}

impl Glides {
    /// `id` moved from `old` to `new` (its places, px at 100 %): it starts
    /// where it is drawn now and glides to `new`.
    pub fn shift(&mut self, id: &str, old: (f32, f32), new: (f32, f32)) {
        let now = Instant::now();
        let (dx, dy) = self.offset(id, now);
        let from = (old.0 + dx - new.0, old.1 + dy - new.1);
        if from.0.abs() < 0.5 && from.1.abs() < 0.5 {
            self.moving.remove(id);
            return;
        }
        self.moving.insert(
            id.to_string(),
            Glide {
                from,
                fade: false,
                started: now,
            },
        );
    }

    /// Glide every id of `old` whose place differs in `new`.
    pub fn shift_all(
        &mut self,
        old: &HashMap<String, (f32, f32)>,
        new: &HashMap<String, (f32, f32)>,
    ) {
        for (id, to) in new {
            if let Some(from) = old.get(id)
                && from != to
            {
                self.shift(id, *from, *to);
            }
        }
    }

    /// `id` is new: it rises into place and fades in.
    pub fn appear(&mut self, id: &str) {
        self.moving.insert(
            id.to_string(),
            Glide {
                from: (0., RISE),
                fade: true,
                started: Instant::now(),
            },
        );
    }

    /// Drop finished glides; ask for the next frame while any runs. Call
    /// once per render, before reading offsets.
    pub fn tick(&mut self, window: &Window, cx: &App) {
        if self.moving.is_empty() {
            return;
        }
        if !allowed(window, cx) {
            self.moving.clear();
            return;
        }
        let now = Instant::now();
        self.moving.retain(|_, glide| glide.t(now) < 1.);
        if !self.moving.is_empty() {
            window.request_animation_frame();
        }
    }

    fn offset(&self, id: &str, now: Instant) -> (f32, f32) {
        self.moving.get(id).map_or((0., 0.), |g| g.offset(now))
    }

    /// Stop `id`'s glide (it is drawn at its place).
    pub fn forget(&mut self, id: &str) {
        self.moving.remove(id);
    }

    /// Where `id` is drawn from its place (px at 100 %), and its opacity.
    pub fn at(&self, id: &str) -> (f32, f32, f32) {
        let now = Instant::now();
        match self.moving.get(id) {
            Some(glide) => {
                let (x, y) = glide.offset(now);
                let opacity = if glide.fade {
                    ease_out_quint()(glide.t(now))
                } else {
                    1.
                };
                (x, y, opacity)
            }
            None => (0., 0., 1.),
        }
    }
}

/// `element` drawn `(x, y)` px (at 100 %) from its place with `opacity`,
/// without moving its neighbours.
pub(super) fn placed<E: Styled>(element: E, (x, y, opacity): (f32, f32, f32)) -> E {
    if x == 0. && y == 0. && opacity >= 1. {
        return element;
    }
    element
        .relative()
        .left(rems(x / 16.))
        .top(rems(y / 16.))
        .opacity(opacity)
}

#[cfg(test)]
mod tests {
    use gentle_core::genui::layout::layout;
    use serde_json::json;

    use super::*;

    #[test]
    fn idle_diagrams_ask_for_no_frames() {
        let all = Motion::ALL;
        for motion in all {
            // Nothing moving and no animated edge on screen: idle.
            assert_eq!(frame_delay(false, false, true, false, motion), None);
            // Reduced motion stops tweens and marching alike.
            assert_eq!(frame_delay(true, true, true, true, motion), None);
            // A tween runs even in an inactive window; it ends by itself.
            assert_eq!(
                frame_delay(true, false, false, false, motion),
                Some(TWEEN_FRAME)
            );
            // Marching dashes need an active window.
            assert_eq!(frame_delay(false, true, false, false, motion), None);
        }
        assert_eq!(frame_delay(false, true, true, false, Motion::Still), None);
        assert_eq!(
            frame_delay(false, true, true, false, Motion::Smooth),
            Some(MARCH_SMOOTH)
        );
        assert_eq!(
            frame_delay(false, true, true, false, Motion::Calm),
            Some(MARCH_CALM)
        );
        assert!(!may_move(true, false, Motion::Still));
        assert!(!may_move(false, false, Motion::Smooth));
        assert!(!may_move(true, true, Motion::Calm));
        assert!(may_move(true, false, Motion::Calm));
    }

    #[test]
    fn arrival_moves_kept_nodes_and_fades_the_rest() {
        let old = Diagram::parse(&json!({
            "nodes": [{"id": "a"}, {"id": "b"}, {"id": "gone"}],
            "edges": [{"from": "a", "to": "b"}, {"from": "a", "to": "gone"}],
        }))
        .unwrap();
        let new = Diagram::parse(&json!({
            "direction": "TB",
            "nodes": [{"id": "a"}, {"id": "b"}, {"id": "c"}],
            "edges": [{"from": "a", "to": "b"}, {"from": "b", "to": "c"}],
        }))
        .unwrap();
        let (ol, nl) = (layout(&old), layout(&new));
        let now = Instant::now();
        let arrival = Arrival::between_at(&old, &ol, &new, &nl, now).unwrap();
        assert_eq!(arrival.leaving_nodes.len(), 1);
        assert_eq!(arrival.leaving_edges.len(), 1);
        assert_eq!(arrival.progress(now), 0.);
        assert!(arrival.done(now + ARRIVAL));
        // Kept: from the old place to the new one.
        assert_eq!(arrival.node("b", nl.nodes[1], 0.), (ol.nodes[1], 1.));
        assert_eq!(arrival.node("b", nl.nodes[1], 1.), (nl.nodes[1], 1.));
        // New: transparent and smaller at first.
        let (rect, alpha) = arrival.node("c", nl.nodes[2], 0.);
        assert_eq!(alpha, 0.);
        assert!(rect.w < nl.nodes[2].w);
        assert_eq!(arrival.edge("b->c", &nl.edges[1], 0.5).1, 0.5);
        // Nothing changed: no arrival.
        assert!(Arrival::between_at(&new, &nl, &new, &nl, now).is_none());
    }

    #[test]
    fn view_tween_ends_on_its_target() {
        let now = Instant::now();
        let tween = ViewTween::new((point(0., 0.), 1.), (point(100., -50.), 2.), now);
        assert_eq!(tween.at(now), ((point(0., 0.), 1.), false));
        let (end, done) = tween.at(now + VIEW);
        assert!(done);
        assert_eq!(end, tween.target());
    }
}
