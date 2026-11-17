//! What moves on a diagram and when it may: the agent's new version easing
//! in over the previous one, the viewport easing to a fit or zoom, and the
//! marching dashes of async and data edges. Frames are asked for only while
//! something moves; see [`frame_delay`].

use std::collections::HashMap;
use std::time::{Duration, Instant};

use gentle_core::genui::diagram::{Diagram, EdgeStyle, Node};
use gentle_core::genui::layout::{Cubic, EdgePath, Layout, Point as P, Rect};
use gpui_kit::{Point, point};

use crate::shared::backdrop::Motion;

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
    if !marching_on_screen || !marches(window_active, reduce_motion, motion) {
        return None;
    }
    Some(match motion {
        Motion::Smooth => MARCH_SMOOTH,
        _ => MARCH_CALM,
    })
}

/// Whether animated edges' dashes move at all (else they hold still).
pub(super) fn marches(window_active: bool, reduce_motion: bool, motion: Motion) -> bool {
    window_active && !reduce_motion && motion != Motion::Still
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
        assert!(!marches(true, false, Motion::Still));
        assert!(!marches(false, false, Motion::Smooth));
        assert!(!marches(true, true, Motion::Calm));
        assert!(marches(true, false, Motion::Calm));
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
