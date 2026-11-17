//! Motion for the panel kinds: a value tween (charts) and glides (rows and
//! cards sliding to a new place, new ones fading in). They run only while
//! something moves, with the window active, and not at all with reduced
//! motion or while backdrop motion is Still: each frame asks for the next
//! one only when it still has motion to draw, so an idle or off-screen card
//! costs nothing.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use gpui_kit::*;

use crate::shared::backdrop::{self, Motion};
use crate::shared::ui;

/// Values tweening to a new version.
const TWEEN: Duration = Duration::from_millis(300);
/// Rows and cards gliding to their new place.
const GLIDE: Duration = Duration::from_millis(340);
/// How far (px at 100 %) a new row rises while it fades in.
const RISE: f32 = 6.;

/// Whether anything may move now.
pub(super) fn allowed(window: &Window, cx: &App) -> bool {
    !cx.reduce_motion() && backdrop::motion_now(cx) != Motion::Still && window.is_window_active()
}

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
