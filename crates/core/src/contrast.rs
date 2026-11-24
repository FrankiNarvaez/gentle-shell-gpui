//! WCAG 2 contrast for translucent glass: relative luminance, contrast
//! ratio, compositing a translucent color over what is behind it, and the
//! two corrections the palette makes so text stays readable (AA): the least
//! opacity a text backing needs over a worst-case backdrop, and the least
//! nudge a text color needs over an opaque backing.
//!
//! Colors are `0xRRGGBB`; blending happens on the sRGB-encoded channels, the
//! way the window's renderer and the system compositor blend.

/// WCAG AA for body text.
pub const AA_TEXT: f64 = 4.5;
/// WCAG AA for user-interface components and graphical objects (input
/// borders, icons that carry meaning, focus indicators).
pub const AA_UI: f64 = 3.0;

/// The worst backdrops behind the window: whatever the desktop or the
/// theme's picture shows, a pixel behind a pane lies between these two, and
/// a blur only averages pixels (a uniform white window stays white).
pub const WORST_BACKDROPS: [u32; 2] = [0x000000, 0xffffff];

/// An sRGB color with channels 0–1, so compositing several layers doesn't
/// round at each step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgb(pub [f64; 3]);

impl Rgb {
    pub fn hex(hex: u32) -> Self {
        let channel = |shift: u32| f64::from((hex >> shift) & 0xff) / 255.;
        Self([channel(16), channel(8), channel(0)])
    }

    pub fn to_hex(self) -> u32 {
        let [r, g, b] = self.0.map(|c| (c.clamp(0., 1.) * 255.).round() as u32);
        (r << 16) | (g << 8) | b
    }

    /// This color at `alpha` (0–1) drawn over `below`.
    pub fn over(self, alpha: f64, below: Rgb) -> Self {
        let a = alpha.clamp(0., 1.);
        Self(std::array::from_fn(|i| {
            below.0[i] + (self.0[i] - below.0[i]) * a
        }))
    }

    /// WCAG 2 relative luminance: 0 black, 1 white.
    pub fn luminance(self) -> f64 {
        let linear = |c: f64| {
            if c <= 0.040_45 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        let [r, g, b] = self.0.map(linear);
        0.2126 * r + 0.7152 * g + 0.0722 * b
    }
}

/// WCAG 2 contrast ratio between two opaque colors: 1 (none) to 21.
pub fn ratio(a: Rgb, b: Rgb) -> f64 {
    let (la, lb) = (a.luminance(), b.luminance());
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// The least alpha (0–1) at which `backing`, drawn over each of `below`,
/// gives every `(text, target)` its target ratio; 1 when even opaque it
/// can't (the text color itself is too close, see [`readable`]).
///
/// Contrast doesn't always grow with alpha (a mid backdrop under a mid
/// backing), so the search walks up in small steps rather than bisecting.
pub fn min_alpha(backing: Rgb, below: &[Rgb], texts: &[(Rgb, f64)]) -> f64 {
    const STEPS: u32 = 200;
    let meets = |alpha: f64| {
        below.iter().all(|&under| {
            let behind = backing.over(alpha, under);
            texts
                .iter()
                .all(|&(text, target)| ratio(text, behind) >= target)
        })
    };
    (0..=STEPS)
        .map(|i| f64::from(i) / f64::from(STEPS))
        .find(|&alpha| meets(alpha))
        .unwrap_or(1.)
}

/// `text`, nudged toward white or black by the least amount that reaches
/// `target` over every one of `backings`, so its hue and character stay.
/// Unchanged when it already reads; when neither direction gets there, the
/// direction that comes closest, all the way to white or black.
///
/// Candidates are checked as the `0xRRGGBB` they end up as, so rounding
/// never leaves the result a hair under the target.
pub fn readable(text: Rgb, backings: &[Rgb], target: f64) -> Rgb {
    let worst = |c: Rgb| {
        backings
            .iter()
            .map(|&b| ratio(c, b))
            .fold(f64::INFINITY, f64::min)
    };
    if worst(text) >= target {
        return text;
    }
    const STEPS: u32 = 100;
    let toward = |end: Rgb| {
        (1..=STEPS)
            .map(|i| Rgb::hex(end.over(f64::from(i) / f64::from(STEPS), text).to_hex()))
            .enumerate()
            .find(|&(_, c)| worst(c) >= target)
    };
    let (white, black) = (Rgb([1.; 3]), Rgb([0.; 3]));
    match (toward(white), toward(black)) {
        (Some((up, a)), Some((down, b))) => {
            if up <= down {
                a
            } else {
                b
            }
        }
        (Some((_, c)), None) | (None, Some((_, c))) => c,
        (None, None) if worst(white) >= worst(black) => white,
        (None, None) => black,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 0.01
    }

    #[test]
    fn ratio_matches_wcag_reference_values() {
        let (black, white) = (Rgb::hex(0x000000), Rgb::hex(0xffffff));
        assert!(close(ratio(black, white), 21.));
        assert!(close(ratio(white, white), 1.));
        // Order doesn't matter.
        assert_eq!(ratio(white, black), ratio(black, white));
        // #767676 on white is the classic smallest AA gray: 4.54:1.
        assert!(close(ratio(Rgb::hex(0x767676), white), 4.54));
        assert!(ratio(Rgb::hex(0x777777), white) < AA_TEXT);
    }

    #[test]
    fn compositing_blends_channels() {
        let red = Rgb::hex(0xff0000);
        let blue = Rgb::hex(0x0000ff);
        assert_eq!(red.over(1., blue), red);
        assert_eq!(red.over(0., blue), blue);
        assert_eq!(red.over(0.5, blue).to_hex(), 0x800080);
        // Out-of-range alpha clamps.
        assert_eq!(red.over(2., blue), red);
        assert_eq!(Rgb::hex(0x123456).to_hex(), 0x123456);
    }

    #[test]
    fn min_alpha_is_the_least_that_reads_over_every_backdrop() {
        let backing = Rgb::hex(0x1a1a1a);
        let text = Rgb::hex(0xe6e6e6);
        let below = WORST_BACKDROPS.map(Rgb::hex);
        let alpha = min_alpha(backing, &below, &[(text, AA_TEXT)]);
        assert!(alpha > 0. && alpha < 1.);
        // At that alpha it reads over both, just below it fails over white.
        for under in below {
            assert!(ratio(text, backing.over(alpha, under)) >= AA_TEXT);
        }
        assert!(ratio(text, backing.over(alpha - 0.01, below[1])) < AA_TEXT);
        // Text that reads over any backdrop needs no backing at all…
        let black_text = Rgb::hex(0x000000);
        let white_bg = [Rgb::hex(0xffffff)];
        assert_eq!(min_alpha(backing, &white_bg, &[(black_text, AA_TEXT)]), 0.);
        // …and text too close to its own backing can't be helped by opacity.
        let gray = Rgb::hex(0x333333);
        assert_eq!(min_alpha(backing, &below, &[(gray, AA_TEXT)]), 1.);
    }

    #[test]
    fn readable_nudges_only_as_far_as_needed() {
        let dark = [Rgb::hex(0x202020), Rgb::hex(0x262626)];
        // Already readable: untouched.
        let light = Rgb::hex(0xdddddd);
        assert_eq!(readable(light, &dark, AA_TEXT), light);
        // A dim gray on a dark backing brightens just enough.
        let dim = Rgb::hex(0x707070);
        let fixed = readable(dim, &dark, AA_TEXT);
        assert!(fixed.luminance() > dim.luminance());
        assert!(dark.iter().all(|&b| ratio(fixed, b) >= AA_TEXT));
        assert!(ratio(fixed, dark[1]) < AA_TEXT + 0.3, "overshot: {fixed:?}");
        // On a light backing it darkens instead, keeping its hue.
        let paper = [Rgb::hex(0xfafafa)];
        let pink = Rgb::hex(0xf095c8);
        let fixed = readable(pink, &paper, AA_TEXT);
        assert!(fixed.luminance() < pink.luminance());
        assert!(ratio(fixed, paper[0]) >= AA_TEXT);
        let [r, g, b] = fixed.0;
        assert!(r > g && b > g, "lost its pink: {fixed:?}");
    }
}
