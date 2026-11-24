//! What the Blur slider (0–1) does to a theme's own background: a picture
//! is blurred for real, once per strength (GPUI has no blur shader, so the
//! app pre-blurs the decoded picture); an animated scene is redrawn every
//! frame, so blurring it would cost a CPU blur per frame, and it is faded
//! toward the theme's background instead.

/// The strongest blur: the Gaussian's sigma as a share of the picture's
/// longer side. The picture covers the window, so this is about 2 % of
/// the window too, whatever the picture's resolution.
pub const MAX_SIGMA_SHARE: f32 = 0.02;

/// The most of the theme's background color laid over an animated scene,
/// at full Blur: enough to calm it, not enough to hide it.
pub const MAX_SCENE_FADE: f32 = 0.5;

/// The picture is re-blurred in steps of this many percent: dragging the
/// slider makes at most 20 blurred copies, not 100.
pub const BLUR_STEP: u32 = 5;

/// `blur` (0–1) as the whole percent the picture is blurred at, rounded
/// to [`BLUR_STEP`]; 0 means sharp.
pub fn blur_percent(blur: f32) -> u32 {
    let steps = (blur.clamp(0., 1.) * (100 / BLUR_STEP) as f32).round() as u32;
    steps * BLUR_STEP
}

/// How to blur a `width`×`height` picture: shrink it by `shrink` (a whole
/// factor), then blur it with `sigma` pixels of the shrunk picture.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PictureBlur {
    pub shrink: u32,
    pub sigma: f32,
}

impl PictureBlur {
    /// The plan for `blur` (0–1), or `None` when the picture stays sharp.
    ///
    /// A blurred picture has no detail finer than its blur, so it is shrunk
    /// first: the blur costs less and the copy is small. At least to half
    /// size, so the blurred copy of an animated picture (every frame is
    /// kept) adds at most a quarter of the frames' memory; the window
    /// scales it back up with linear filtering, which only adds softness.
    pub fn new(blur: f32, width: u32, height: u32) -> Option<Self> {
        if blur.is_nan() || blur <= 0. || width < 2 || height < 2 {
            return None;
        }
        let sigma = blur.min(1.) * MAX_SIGMA_SHARE * width.max(height) as f32;
        // Two pixels of sigma per shrunk pixel keeps the blur smooth.
        let most = (width.min(height) / 2).max(1);
        let shrink = ((sigma / 2.).floor() as u32).max(2).min(most);
        Some(Self {
            shrink,
            sigma: sigma / shrink as f32,
        })
    }

    /// The shrunk picture's size.
    pub fn size(&self, width: u32, height: u32) -> (u32, u32) {
        ((width / self.shrink).max(1), (height / self.shrink).max(1))
    }
}

/// How much of the theme's background color covers an animated scene at
/// `blur` (0–1).
pub fn scene_fade(blur: f32) -> f32 {
    blur.clamp(0., 1.) * MAX_SCENE_FADE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_blur_keeps_the_picture_sharp() {
        assert_eq!(PictureBlur::new(0., 1920, 1080), None);
        assert_eq!(PictureBlur::new(-1., 1920, 1080), None);
        assert_eq!(PictureBlur::new(f32::NAN, 1920, 1080), None);
        assert_eq!(PictureBlur::new(0.5, 1, 1), None);
    }

    #[test]
    fn blur_scales_with_the_picture_not_its_resolution() {
        // The same picture at two resolutions blurs the same on screen:
        // sigma times shrink, as a share of the longer side, matches.
        for (w, h) in [(1920, 1080), (3840, 2160), (800, 1200)] {
            let plan = PictureBlur::new(1., w, h).unwrap();
            let on_picture = plan.sigma * plan.shrink as f32 / w.max(h) as f32;
            assert!(
                (on_picture - MAX_SIGMA_SHARE).abs() < 1e-4,
                "{w}x{h}: {plan:?}"
            );
        }
        // Half the slider, half the blur.
        let full = PictureBlur::new(1., 3840, 2160).unwrap();
        let half = PictureBlur::new(0.5, 3840, 2160).unwrap();
        let reach = |p: PictureBlur| p.sigma * p.shrink as f32;
        assert!((reach(half) * 2. - reach(full)).abs() < 1e-3);
        // Out of range clamps.
        assert_eq!(PictureBlur::new(5., 3840, 2160), Some(full));
    }

    #[test]
    fn blurred_copies_are_at_most_a_quarter_of_the_pixels() {
        for blur in [0.01, 0.3, 1.] {
            let plan = PictureBlur::new(blur, 1280, 720).unwrap();
            let (w, h) = plan.size(1280, 720);
            assert!(w * h * 4 <= 1280 * 720, "{blur}: {w}x{h}");
            // Strong blurs shrink much further.
            if blur == 1. {
                assert!(plan.shrink >= 8, "{plan:?}");
            }
        }
        // A tiny picture still yields at least a pixel.
        let plan = PictureBlur::new(1., 4, 4).unwrap();
        assert_eq!(plan.size(4, 4), (2, 2));
    }

    #[test]
    fn blur_rounds_to_its_step() {
        assert_eq!(blur_percent(0.), 0);
        assert_eq!(blur_percent(0.02), 0);
        assert_eq!(blur_percent(0.03), 5);
        assert_eq!(blur_percent(0.49), 50);
        assert_eq!(blur_percent(1.), 100);
        assert_eq!(blur_percent(7.), 100);
        assert_eq!(blur_percent(f32::NAN), 0);
    }

    #[test]
    fn scenes_fade_with_blur() {
        assert_eq!(scene_fade(0.), 0.);
        assert_eq!(scene_fade(1.), MAX_SCENE_FADE);
        assert_eq!(scene_fade(0.5), MAX_SCENE_FADE / 2.);
        assert_eq!(scene_fade(3.), MAX_SCENE_FADE);
    }
}
