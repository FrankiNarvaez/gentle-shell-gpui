//! The fonts the user picks in Configuration › Appearance: the interface
//! font, the code font (monospace text in the interface) and the terminal
//! font. `None` everywhere means "the app's default", so an untouched
//! setting renders exactly as before the choice existed.

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

/// The terminal's family: its own choice, else the code font's, else
/// `None` (the default monospace).
pub fn terminal<'a>(terminal: Option<&'a str>, code: Option<&'a str>) -> Option<&'a str> {
    terminal.or(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
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

    #[test]
    fn the_terminal_follows_the_code_font_unless_it_has_its_own() {
        assert_eq!(terminal(Some("Hack"), Some("Fira Code")), Some("Hack"));
        assert_eq!(terminal(None, Some("Fira Code")), Some("Fira Code"));
        assert_eq!(terminal(None, None), None);
    }
}
