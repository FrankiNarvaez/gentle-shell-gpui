//! The user's keymap file (`<data>/gentle-shell-gpui/keymap.json`): parsing
//! and validation, without touching GPUI or the disk.
//!
//! The file is forgiving: a bad entry is reported with its index and the rest
//! still apply; an unreadable file means the defaults plus one error.

use serde_json::Value;

/// The leader prefix when the file doesn't set a valid one.
pub const DEFAULT_LEADER: &str = "ctrl-space";

/// One `bindings` entry that passed the syntax checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    /// Position in the file's `bindings` array, for later error messages.
    pub index: usize,
    /// A keystroke or a space-separated sequence, e.g. `ctrl-space o`.
    pub keys: String,
    /// Registered action name (`gentle::OpenPalette`); `None` unbinds `keys`.
    pub action: Option<String>,
    /// Key context predicate, e.g. `GentleWorkspace`; `None` is everywhere.
    pub context: Option<String>,
}

/// Something wrong in the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    /// The `bindings` entry it is about; `None` for the file or the leader.
    pub index: Option<usize>,
    pub message: String,
}

impl Issue {
    /// One line for the user, e.g. `keymap.json bindings[2]: …`.
    pub fn describe(&self) -> String {
        match self.index {
            Some(i) => format!("keymap.json bindings[{i}]: {}", self.message),
            None => format!("keymap.json: {}", self.message),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keymap {
    pub leader: String,
    pub bindings: Vec<Binding>,
    pub errors: Vec<Issue>,
}

impl Default for Keymap {
    fn default() -> Self {
        Keymap {
            leader: DEFAULT_LEADER.into(),
            bindings: Vec::new(),
            errors: Vec::new(),
        }
    }
}

const MODIFIERS: [&str; 7] = ["cmd", "ctrl", "alt", "shift", "fn", "function", "secondary"];

/// Parse the file's text. Never fails: problems land in `errors`.
pub fn parse(text: &str) -> Keymap {
    let mut keymap = Keymap::default();
    if text.trim().is_empty() {
        return keymap;
    }
    let root: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => {
            keymap
                .errors
                .push(file_issue(format!("not valid JSON ({e})")));
            return keymap;
        }
    };
    let Some(root) = root.as_object() else {
        keymap.errors.push(file_issue(
            "expected an object with \"leader\" and \"bindings\"".into(),
        ));
        return keymap;
    };
    if let Some(leader) = root.get("leader") {
        match leader_ok(leader) {
            Ok(l) => keymap.leader = l,
            Err(e) => keymap
                .errors
                .push(file_issue(format!("leader: {e}; using {DEFAULT_LEADER}"))),
        }
    }
    match root.get("bindings") {
        None | Some(Value::Null) => {}
        Some(Value::Array(entries)) => {
            for (index, entry) in entries.iter().enumerate() {
                match binding(index, entry) {
                    Ok(b) => keymap.bindings.push(b),
                    Err(message) => keymap.errors.push(Issue {
                        index: Some(index),
                        message,
                    }),
                }
            }
        }
        Some(_) => keymap
            .errors
            .push(file_issue("bindings: expected an array".into())),
    }
    keymap
}

fn file_issue(message: String) -> Issue {
    Issue {
        index: None,
        message,
    }
}

fn leader_ok(value: &Value) -> Result<String, String> {
    let keys = value.as_str().ok_or("expected a string")?;
    validate_keys(keys)?;
    if keys.split_whitespace().count() != 1 {
        return Err("must be a single keystroke".into());
    }
    let keys = keys.trim();
    // A bare key as the prefix would swallow typing everywhere.
    if !MODIFIERS.iter().any(|m| keys.starts_with(&format!("{m}-"))) {
        return Err("needs a modifier (ctrl, alt, cmd…)".into());
    }
    Ok(keys.into())
}

fn binding(index: usize, entry: &Value) -> Result<Binding, String> {
    let entry = entry.as_object().ok_or("expected an object")?;
    let keys = entry
        .get("keys")
        .and_then(Value::as_str)
        .ok_or("\"keys\" must be a string")?;
    validate_keys(keys).map_err(|e| format!("keys {keys:?}: {e}"))?;
    let action = match entry.get("action") {
        Some(Value::Null) => None,
        Some(Value::String(name)) if !name.trim().is_empty() => Some(name.trim().to_string()),
        _ => return Err("\"action\" must be an action name or null".into()),
    };
    let context = match entry.get("context") {
        None | Some(Value::Null) => None,
        Some(Value::String(c)) if !c.trim().is_empty() => Some(c.trim().to_string()),
        Some(_) => return Err("\"context\" must be a string".into()),
    };
    Ok(Binding {
        index,
        keys: keys.split_whitespace().collect::<Vec<_>>().join(" "),
        action,
        context,
    })
}

/// Check a keystroke or space-separated sequence; the error says what's wrong.
pub fn validate_keys(keys: &str) -> Result<(), String> {
    let mut any = false;
    for stroke in keys.split_whitespace() {
        any = true;
        validate_keystroke(stroke)?;
    }
    if any { Ok(()) } else { Err("empty".into()) }
}

/// `modifier-…-key`; the key may be `-` itself (`cmd--`).
fn validate_keystroke(stroke: &str) -> Result<(), String> {
    let mut rest = stroke;
    while let Some((head, tail)) = rest.split_once('-') {
        if tail.is_empty() || !MODIFIERS.contains(&head) {
            break;
        }
        rest = tail;
    }
    if rest.is_empty() || (rest.contains('-') && rest != "-") {
        return Err(format!("{stroke:?} is not modifiers followed by one key"));
    }
    if MODIFIERS.contains(&rest) {
        return Err(format!("{stroke:?} has no key after its modifiers"));
    }
    Ok(())
}

/// The file written by "Open keymap file" when none exists.
pub fn template(leader: &str) -> String {
    format!("{{\n  \"leader\": \"{leader}\",\n  \"bindings\": []\n}}\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(index: usize, keys: &str, action: Option<&str>, context: Option<&str>) -> Binding {
        Binding {
            index,
            keys: keys.into(),
            action: action.map(Into::into),
            context: context.map(Into::into),
        }
    }

    #[test]
    fn no_file_content_means_defaults() {
        assert_eq!(parse(""), Keymap::default());
        assert_eq!(Keymap::default().leader, "ctrl-space");
    }

    #[test]
    fn valid_leader_overrides_the_default() {
        let k = parse(r#"{"leader": "ctrl-a"}"#);
        assert_eq!(k.leader, "ctrl-a");
        assert!(k.errors.is_empty());
    }

    #[test]
    fn invalid_leader_falls_back_with_an_error() {
        for bad in [r#""ctrl-a b""#, r#""ctrl-""#, r#""a""#, "3"] {
            let k = parse(&format!(r#"{{"leader": {bad}}}"#));
            assert_eq!(k.leader, DEFAULT_LEADER, "{bad}");
            assert_eq!(k.errors.len(), 1, "{bad}");
            assert_eq!(k.errors[0].index, None);
            assert!(k.errors[0].message.contains("leader"), "{bad}");
        }
    }

    #[test]
    fn valid_binding_keeps_keys_action_and_context() {
        let k = parse(
            r#"{"bindings": [
              {"keys": "cmd-shift-p", "action": "gentle::OpenPalette"},
              {"keys": "ctrl-space o", "action": "gentle::FocusFiles", "context": "GentleWorkspace"}
            ]}"#,
        );
        assert!(k.errors.is_empty());
        assert_eq!(
            k.bindings,
            vec![
                b(0, "cmd-shift-p", Some("gentle::OpenPalette"), None),
                b(
                    1,
                    "ctrl-space o",
                    Some("gentle::FocusFiles"),
                    Some("GentleWorkspace")
                ),
            ]
        );
    }

    #[test]
    fn null_action_unbinds() {
        let k = parse(r#"{"bindings": [{"keys": "cmd-l", "action": null}]}"#);
        assert!(k.errors.is_empty());
        assert_eq!(k.bindings, vec![b(0, "cmd-l", None, None)]);
    }

    #[test]
    fn invalid_entry_is_reported_with_its_index_and_the_rest_kept() {
        let k = parse(
            r#"{"bindings": [
              {"keys": "cmd-p", "action": "gentle::OpenPalette"},
              {"keys": "ctrl-", "action": "gentle::OpenPalette"},
              {"action": "gentle::OpenPalette"},
              {"keys": "cmd-k", "action": 5},
              "nope",
              {"keys": "cmd-j", "action": "gentle::NewTab"}
            ]}"#,
        );
        let indexes: Vec<_> = k.errors.iter().map(|e| e.index).collect();
        assert_eq!(indexes, vec![Some(1), Some(2), Some(3), Some(4)]);
        let kept: Vec<_> = k.bindings.iter().map(|b| b.index).collect();
        assert_eq!(kept, vec![0, 5]);
        assert!(
            k.errors[0]
                .describe()
                .starts_with("keymap.json bindings[1]:")
        );
    }

    #[test]
    fn malformed_json_gives_defaults_and_one_error() {
        let k = parse("{ not json");
        assert_eq!(k.leader, DEFAULT_LEADER);
        assert!(k.bindings.is_empty());
        assert_eq!(k.errors.len(), 1);
        assert_eq!(k.errors[0].index, None);
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let k = parse(
            r#"{"version": 2, "bindings": [
              {"keys": "cmd-j", "action": "gentle::NewTab", "note": "x"}
            ]}"#,
        );
        assert!(k.errors.is_empty());
        assert_eq!(k.bindings.len(), 1);
    }

    #[test]
    fn keystroke_syntax() {
        for ok in [
            "a",
            "cmd-shift-p",
            "ctrl-space h",
            "cmd--",
            "shift-;",
            "f2",
            "alt-enter",
        ] {
            assert_eq!(validate_keys(ok), Ok(()), "{ok}");
        }
        for bad in ["", "   ", "ctrl-", "ctrl-shift", "cmd-p-q", "ctrl- p"] {
            assert!(validate_keys(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn template_is_a_valid_file_with_the_leader() {
        let k = parse(&template("ctrl-a"));
        assert_eq!(k.leader, "ctrl-a");
        assert!(k.errors.is_empty() && k.bindings.is_empty());
    }
}
