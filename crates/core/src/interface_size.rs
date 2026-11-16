//! The interface size (zoom): a whole percent from [`MIN`] to [`MAX`],
//! [`DEFAULT`] meaning the app as designed. The shortcuts move it a
//! [`STEP`] at a time; a hand-edited setting may hold any percent in range
//! (125 %, say, to match a desktop scaled to 125 %), and stepping from there
//! lands on the next multiple of the step.

/// Smallest size, in percent.
pub const MIN: u16 = 80;
/// Largest size, in percent.
pub const MAX: u16 = 150;
/// The app as designed.
pub const DEFAULT: u16 = 100;
/// One shortcut press.
pub const STEP: u16 = 10;

/// Any percent, kept in range.
pub fn clamp(percent: u16) -> u16 {
    percent.clamp(MIN, MAX)
}

/// The persisted factor (`1.25` is 125 %), rounded to a whole percent and
/// kept in range; `None` when it isn't a usable number.
pub fn from_factor(factor: f64) -> Option<u16> {
    if !factor.is_finite() || factor <= 0. {
        return None;
    }
    let percent = (factor * 100.).round().min(f64::from(MAX));
    Some(clamp(percent as u16))
}

/// The factor persisted for `percent` (`1.5` for 150 %).
pub fn factor(percent: u16) -> f64 {
    f64::from(clamp(percent)) / 100.
}

/// One step bigger: the next multiple of [`STEP`] above `percent`.
pub fn bigger(percent: u16) -> u16 {
    clamp((clamp(percent) / STEP + 1) * STEP)
}

/// One step smaller: the previous multiple of [`STEP`] below `percent`.
pub fn smaller(percent: u16) -> u16 {
    let percent = clamp(percent);
    clamp((percent.div_ceil(STEP) - 1) * STEP)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_stop_at_the_ends() {
        assert_eq!(bigger(100), 110);
        assert_eq!(smaller(100), 90);
        assert_eq!(bigger(MAX), MAX);
        assert_eq!(smaller(MIN), MIN);
        assert_eq!(bigger(140), MAX);
        assert_eq!(smaller(90), MIN);
    }

    #[test]
    fn steps_from_an_odd_size_land_on_the_grid() {
        assert_eq!(bigger(125), 130);
        assert_eq!(smaller(125), 120);
        assert_eq!(bigger(81), 90);
        assert_eq!(smaller(149), 140);
    }

    #[test]
    fn persisted_factor_rounds_and_clamps() {
        assert_eq!(from_factor(1.5), Some(150));
        assert_eq!(from_factor(1.25), Some(125));
        assert_eq!(from_factor(1.104), Some(110));
        assert_eq!(from_factor(3.), Some(MAX));
        assert_eq!(from_factor(0.2), Some(MIN));
        assert_eq!(from_factor(0.), None);
        assert_eq!(from_factor(f64::NAN), None);
        assert_eq!(factor(150), 1.5);
        assert_eq!(factor(10), 0.8);
    }
}
