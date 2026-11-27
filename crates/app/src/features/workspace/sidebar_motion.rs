//! Sidebar visibility transitions never interpolate the persisted resize width.

use super::*;
use gentle_core::workspaces::{SidebarSlide, SidebarWidthSlide, shown_sidebar_width};
use std::time::Instant;

#[derive(Default)]
pub(super) struct SidebarMotion {
    slide: Option<(Instant, SidebarSlide)>,
    width_slide: Option<(Instant, SidebarWidthSlide)>,
    pub(super) revision: u64,
    allocation_start: Option<(f32, bool)>,
    pub(super) active_drag: std::rc::Weak<std::cell::Cell<u64>>,
    /// The window width (px at 100 %) of the last layout, so a change made
    /// outside a render (a menu, a button) can slide from what is on screen.
    /// 0 until the first layout.
    pub(super) window: f32,
}

impl SidebarMotion {
    pub(super) fn retarget(&mut self, was_open: bool, reduce_motion: bool) {
        self.revision += 1;
        self.allocation_start = None;
        let now = Instant::now();
        let from = self.fraction(was_open, now);
        self.slide = (!reduce_motion).then(|| (now, SidebarSlide::new(from, !was_open)));
    }

    /// Move the width to `to` over the visibility transition, starting from
    /// what the drag shows now; equal ends need no transition.
    pub(super) fn retarget_width(&mut self, from: f32, to: f32, reduce_motion: bool) {
        self.width_slide = (!reduce_motion && (from - to).abs() >= 0.5)
            .then(|| (Instant::now(), SidebarWidthSlide::new(from, to)));
    }

    pub(super) fn width_moving(&self) -> bool {
        self.width_slide.is_some()
    }

    /// The width to lay out now: the target, or where its transition is.
    fn width(&self, target: f32, now: Instant) -> f32 {
        self.width_slide.map_or(target, |(at, slide)| {
            slide.width(now.saturating_duration_since(at).as_secs_f32())
        })
    }

    // A rail and an expanded panel can have the same visibility fraction but
    // occupy very different widths. Carry the actual row allocation forward.
    pub(super) fn preserve_allocation(&mut self, width: f32, open: bool) {
        self.allocation_start = self.slide.map(|_| (width, open));
    }

    fn allocation(&self, width: f32, fraction: f32, now: Instant) -> f32 {
        if let (Some((at, _)), Some((start, open))) = (self.slide, self.allocation_start) {
            let remaining = SidebarSlide::new(1., false)
                .fraction(now.saturating_duration_since(at).as_secs_f32());
            let end = if open { width } else { 0. };
            end + (start - end) * remaining
        } else {
            width * fraction
        }
    }

    fn fraction(&self, open: bool, now: Instant) -> f32 {
        self.slide
            .map_or(if open { 1. } else { 0. }, |(at, slide)| {
                slide.fraction(now.duration_since(at).as_secs_f32())
            })
    }

    pub(super) fn advance(&mut self, now: Instant, reduce_motion: bool) -> bool {
        let finished =
            |at: Instant| now.saturating_duration_since(at).as_secs_f32() >= SidebarSlide::SECONDS;
        if reduce_motion || self.slide.is_some_and(|(at, _)| finished(at)) {
            self.slide = None;
            self.allocation_start = None;
        }
        if reduce_motion || self.width_slide.is_some_and(|(at, _)| finished(at)) {
            self.width_slide = None;
        }
        self.slide.is_some() || self.width_slide.is_some()
    }
}

impl Workspace {
    /// The visible panel and inner gap, shared by layout and painting.
    pub(super) fn visible_sidebar_geometry(
        &self,
        side: Side,
        window: f32,
        now: Instant,
    ) -> (f32, f32) {
        let (width, fraction) = self.sidebar_geometry(side, window, now);
        (
            self.sidebar_allocation(side, width, fraction, now),
            6. * fraction,
        )
    }

    pub(super) fn sidebar_allocation(
        &self,
        side: Side,
        width: f32,
        fraction: f32,
        now: Instant,
    ) -> f32 {
        match side {
            Side::Tabs => &self.tabs_sidebar_motion,
            Side::Files => &self.files_sidebar_motion,
        }
        .allocation(width, fraction, now)
    }

    pub(super) fn sidebar_motion_mut(&mut self, side: Side) -> &mut SidebarMotion {
        match side {
            Side::Tabs => &mut self.tabs_sidebar_motion,
            Side::Files => &mut self.files_sidebar_motion,
        }
    }

    pub(super) fn sidebar_drag_active(&self, side: Side, cx: &App) -> bool {
        let motion = match side {
            Side::Tabs => &self.tabs_sidebar_motion,
            Side::Files => &self.files_sidebar_motion,
        };
        cx.has_active_drag()
            && motion
                .active_drag
                .upgrade()
                .is_some_and(|revision| revision.get() == motion.revision)
    }

    /// The width the panel is headed to: the rail while it shows as the icon
    /// rail, else the saved expanded width within the window's share.
    pub(super) fn sidebar_target_width(&self, side: Side, window: f32) -> f32 {
        match side {
            Side::Tabs if self.tabs_sidebar_rail() => COMPACT_SIDEBAR_WIDTH,
            Side::Tabs => shown_sidebar_width(self.tabs_sidebar_width, window),
            Side::Files => shown_sidebar_width(self.files_sidebar_width, window),
        }
    }

    /// A drag that changes where the panel is headed (the rail, the expanded
    /// width) moves there over the visibility transition instead of jumping,
    /// starting from what the drag shows now. A plain resize keeps following
    /// the pointer.
    pub(super) fn animate_sidebar_width(
        &mut self,
        side: Side,
        window: f32,
        from: f32,
        reduce_motion: bool,
    ) {
        let target = self.sidebar_target_width(side, window);
        let motion = self.sidebar_motion_mut(side);
        if motion.width_moving() || (from - target).abs() >= 0.5 {
            motion.retarget_width(from, target, reduce_motion);
        }
    }

    /// The full content width and the fraction allocated to it in the row.
    pub(super) fn sidebar_geometry(&self, side: Side, window: f32, now: Instant) -> (f32, f32) {
        let (open, motion) = match side {
            Side::Tabs => (
                self.tab_layout == TabLayout::Vertical,
                &self.tabs_sidebar_motion,
            ),
            Side::Files => (self.sidebar_visible, &self.files_sidebar_motion),
        };
        let fraction = if matches!(side, Side::Files) && self.sidebar.is_none() {
            0.
        } else {
            motion.fraction(open, now)
        };
        let width = motion.width(self.sidebar_target_width(side, window), now);
        (width, fraction)
    }
}
