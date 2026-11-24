//! Sidebar visibility transitions never interpolate the persisted resize width.

use super::*;
use gentle_core::workspaces::{SidebarSlide, shown_sidebar_width};
use std::time::Instant;

#[derive(Default)]
pub(super) struct SidebarMotion {
    slide: Option<(Instant, SidebarSlide)>,
    pub(super) revision: u64,
    allocation_start: Option<(f32, bool)>,
    pub(super) active_drag: std::rc::Weak<std::cell::Cell<u64>>,
}

impl SidebarMotion {
    pub(super) fn retarget(&mut self, was_open: bool, reduce_motion: bool) {
        self.revision += 1;
        self.allocation_start = None;
        let now = Instant::now();
        let from = self.fraction(was_open, now);
        self.slide = (!reduce_motion).then(|| (now, SidebarSlide::new(from, !was_open)));
    }

    // A rail and an expanded panel can have the same visibility fraction but
    // occupy very different widths. Carry the actual row allocation forward.
    pub(super) fn preserve_allocation(&mut self, width: f32, open: bool) {
        self.allocation_start = self.slide.map(|_| (width, open));
    }

    pub(super) fn resize_immediately(&mut self) {
        self.allocation_start = None;
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
        if reduce_motion
            || self.slide.is_some_and(|(at, _)| {
                now.duration_since(at).as_secs_f32() >= SidebarSlide::SECONDS
            })
        {
            self.slide = None;
            self.allocation_start = None;
        }
        self.slide.is_some()
    }
}

impl Workspace {
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

    /// The full content width and the fraction allocated to it in the row.
    pub(super) fn sidebar_geometry(&self, side: Side, window: f32, now: Instant) -> (f32, f32) {
        let (saved, open, motion) = match side {
            Side::Tabs => (
                self.tabs_sidebar_width,
                self.tab_layout == TabLayout::Vertical,
                &self.tabs_sidebar_motion,
            ),
            Side::Files => (
                self.files_sidebar_width,
                self.sidebar_visible,
                &self.files_sidebar_motion,
            ),
        };
        let fraction = if matches!(side, Side::Files) && self.sidebar.is_none() {
            0.
        } else {
            motion.fraction(open, now)
        };
        let width = if matches!(side, Side::Tabs) && self.sidebar_compact {
            COMPACT_SIDEBAR_WIDTH
        } else {
            shown_sidebar_width(saved, window)
        };
        (width, fraction)
    }
}
