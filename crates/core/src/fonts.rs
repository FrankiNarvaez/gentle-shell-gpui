//! The fonts the user picks in Configuration › Appearance: the interface
//! font, the code font (monospace text in the interface) and the terminal
//! font. `None` everywhere means "the app's default", so an untouched
//! setting renders exactly as before the choice existed.

use crate::fuzzy::{Needle, fuzzy_score};

/// A family name as stored or typed: trimmed, `None` when blank.
pub fn normalize(name: &str) -> Option<String> {
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// `choice` as the system spells it, when it is installed (names compare
/// case-insensitively); `None` when it isn't, so the default applies.
pub fn installed<'a>(choice: &str, installed: &'a [String]) -> Option<&'a str> {
    let choice = choice.trim();
    if choice.is_empty() {
        return None;
    }
    installed
        .iter()
        .find(|name| name.eq_ignore_ascii_case(choice))
        .map(String::as_str)
}

/// Size limits of a font slot, in px at 100 % interface zoom.
pub const MIN_SIZE: f32 = 8.;
pub const MAX_SIZE: f32 = 32.;
/// Defaults that reproduce the sizes the app used before they were chosen.
pub const DEFAULT_INTERFACE_SIZE: f32 = 16.;
pub const DEFAULT_CODE_SIZE: f32 = 13.;
pub const DEFAULT_TERMINAL_SIZE: f32 = 13.;

/// A stored size: finite and within limits, rounded to a tenth of a px.
/// Anything else (NaN, zero, negative, out of range) is `None`: the default.
pub fn normalize_size(size: f64) -> Option<f32> {
    if !size.is_finite() || size < f64::from(MIN_SIZE) || size > f64::from(MAX_SIZE) {
        return None;
    }
    Some(((size * 10.).round() / 10.) as f32)
}

/// A size typed or stepped by the user: out-of-range values clamp, NaN is
/// `None` (the default).
pub fn clamp_size(size: f64) -> Option<f32> {
    if size.is_nan() {
        return None;
    }
    normalize_size(size.clamp(f64::from(MIN_SIZE), f64::from(MAX_SIZE)))
}

/// The families the picker lists for `query`, in order. Names are deduped
/// (case-insensitively), hidden ones (starting with `.`) dropped and the
/// rest sorted case-insensitively; a query keeps the fuzzy matches, best
/// first. With `mono_first`, monospace families (per `is_mono`, asked only
/// about the names left after the search) come before the others.
pub fn picker_rows(
    names: &[String],
    query: &str,
    mono_first: bool,
    is_mono: impl Fn(&str) -> bool,
) -> Vec<String> {
    let mut sorted: Vec<(String, &String)> = names
        .iter()
        .filter(|name| !name.starts_with('.'))
        .map(|name| (name.to_lowercase(), name))
        .collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    sorted.dedup_by(|a, b| a.0 == b.0);
    let needle = Needle::new(query.trim());
    let mut rows: Vec<(bool, usize, String, &String)> = sorted
        .into_iter()
        .filter_map(|(lower, name)| {
            let score = fuzzy_score(&needle.lower, name)?;
            Some((mono_first && !is_mono(name), score, lower, name))
        })
        .collect();
    rows.sort_by(|a, b| (a.0, a.1, &a.2).cmp(&(b.0, b.1, &b.2)));
    rows.into_iter().map(|(.., name)| name.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn stored_sizes_must_be_finite_and_in_range() {
        assert_eq!(normalize_size(14.), Some(14.));
        assert_eq!(normalize_size(13.04), Some(13.));
        assert_eq!(normalize_size(8.), Some(8.));
        assert_eq!(normalize_size(32.), Some(32.));
        for bad in [0., -3., 7.9, 32.1, f64::NAN, f64::INFINITY] {
            assert_eq!(normalize_size(bad), None, "{bad}");
        }
    }

    #[test]
    fn typed_sizes_clamp_but_nan_is_the_default() {
        assert_eq!(clamp_size(2.), Some(MIN_SIZE));
        assert_eq!(clamp_size(99.), Some(MAX_SIZE));
        assert_eq!(clamp_size(f64::INFINITY), Some(MAX_SIZE));
        assert_eq!(clamp_size(12.5), Some(12.5));
        assert_eq!(clamp_size(f64::NAN), None);
    }

    #[test]
    fn normalize_trims_and_drops_blank_names() {
        assert_eq!(normalize("  Fira Code "), Some("Fira Code".to_string()));
        assert_eq!(normalize("   "), None);
        assert_eq!(normalize(""), None);
    }

    #[test]
    fn installed_matches_case_insensitively_with_the_system_spelling() {
        let list = names(&["JetBrainsMono Nerd Font", "Menlo"]);
        assert_eq!(
            installed("jetbrainsmono nerd font", &list),
            Some("JetBrainsMono Nerd Font")
        );
        assert_eq!(installed("Menlo", &list), Some("Menlo"));
    }

    #[test]
    fn a_font_that_is_not_installed_falls_back_to_the_default() {
        let list = names(&["Menlo"]);
        assert_eq!(installed("Hack Nerd Font", &list), None);
        assert_eq!(installed("", &list), None);
    }

    fn is_mono(name: &str) -> bool {
        name.contains("Mono") || name == "Menlo"
    }

    #[test]
    fn picker_dedupes_drops_hidden_names_and_sorts_ignoring_case() {
        let list = names(&[
            ".SystemUIFont",
            "zapfino",
            "Menlo",
            "arial",
            "Arial",
            "Zapfino",
            ".Hidden",
        ]);
        assert_eq!(
            picker_rows(&list, "", false, is_mono),
            ["arial", "Menlo", "zapfino"]
        );
    }

    #[test]
    fn picker_searches_fuzzily_best_match_first() {
        let list = names(&["Source Code Pro", "Courier", "Cousine", "Code2000"]);
        assert_eq!(
            picker_rows(&list, "cod", false, is_mono),
            ["Code2000", "Source Code Pro"]
        );
        assert_eq!(picker_rows(&list, "  ", false, is_mono).len(), 4);
        assert!(picker_rows(&list, "zzz", false, is_mono).is_empty());
    }

    #[test]
    fn picker_lists_monospace_first_keeping_each_group_sorted() {
        let list = names(&["Arial", "Menlo", "Baskerville", "Fira Mono", "Courier"]);
        assert_eq!(
            picker_rows(&list, "", true, is_mono),
            ["Fira Mono", "Menlo", "Arial", "Baskerville", "Courier"]
        );
    }

    #[test]
    fn picker_only_asks_about_the_names_the_search_kept() {
        let list = names(&["Arial", "Menlo", "Fira Mono"]);
        let asked = std::cell::RefCell::new(Vec::new());
        let rows = picker_rows(&list, "fira", true, |n| {
            asked.borrow_mut().push(n.to_string());
            true
        });
        assert_eq!(rows, ["Fira Mono"]);
        assert_eq!(*asked.borrow(), ["Fira Mono"]);
        // Without `mono_first` nobody is asked.
        let rows = picker_rows(&list, "", false, |_| panic!("not needed"));
        assert_eq!(rows, ["Arial", "Fira Mono", "Menlo"]);
    }
}
