//! UI templates: a surface saved by the user (kind, title and spec) to show
//! again in any conversation. Each is one JSON file,
//! `{"format": 1, "name", "kind", "title", "spec", "savedAt"}`, named after
//! a slug of its name; inserting one sends the extension an `insert` event
//! that turns it into a new surface the agent sees as user-provided.

use serde_json::{Value, json};

use super::EVENT_COMMAND;

/// The file format's version.
pub const FORMAT: u64 = 1;
/// The longest name kept (characters).
pub const MAX_NAME: usize = 80;
/// Kinds a template may hold.
pub const KINDS: [&str; 7] = [
    "diagram",
    "table",
    "form",
    "checklist",
    "kanban",
    "chart",
    "html",
];

#[derive(Debug, Clone, PartialEq)]
pub struct Template {
    pub name: String,
    pub kind: String,
    pub title: String,
    pub spec: Value,
    /// Unix seconds.
    pub saved_at: u64,
}

impl Template {
    /// A template from its file's text.
    pub fn parse(text: &str) -> Result<Template, String> {
        let value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let field = |key: &str| value.get(key).and_then(Value::as_str).map(str::trim);
        if value
            .get("format")
            .and_then(Value::as_u64)
            .unwrap_or(FORMAT)
            > FORMAT
        {
            return Err("saved by a newer version of the app".into());
        }
        let kind = field("kind").unwrap_or_default();
        if !KINDS.contains(&kind) {
            return Err(format!("unknown kind \"{kind}\""));
        }
        let Some(spec) = value.get("spec").filter(|s| s.is_object()) else {
            return Err("\"spec\" must be an object".into());
        };
        let title = field("title").unwrap_or_default().to_string();
        let name = field("name")
            .filter(|n| !n.is_empty())
            .map_or_else(|| title.clone(), str::to_string);
        if name.is_empty() {
            return Err("a template needs a name".into());
        }
        Ok(Template {
            name: clean_name(&name),
            kind: kind.to_string(),
            title,
            spec: spec.clone(),
            saved_at: value.get("savedAt").and_then(Value::as_u64).unwrap_or(0),
        })
    }

    /// The file's text.
    pub fn to_json(&self) -> String {
        let value = json!({
            "format": FORMAT,
            "name": self.name,
            "kind": self.kind,
            "title": self.title,
            "spec": self.spec,
            "savedAt": self.saved_at,
        });
        let mut text = serde_json::to_string_pretty(&value).unwrap_or_default();
        text.push('\n');
        text
    }

    /// `/gentle-ui:event {"op":"insert", …}`: the extension shows it as a
    /// new surface (`id` if free, else `ui-<n>`) and tells the agent.
    pub fn insert_command(&self) -> String {
        let event = json!({
            "op": "insert",
            "id": stem(&self.name),
            "version": 0,
            "kind": self.kind,
            "title": self.title,
            "template": self.name,
            "spec": self.spec,
        });
        format!("/{EVENT_COMMAND} {event}")
    }
}

/// One line, trimmed, at most [`MAX_NAME`] characters.
pub fn clean_name(name: &str) -> String {
    let one_line = name.split_whitespace().collect::<Vec<_>>().join(" ");
    super::diagram::truncate(&one_line, MAX_NAME)
}

/// A file stem for `name`: a lowercase slug (`template` when none is left),
/// also a free surface id.
pub fn stem(name: &str) -> String {
    let mut slug = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug: String = slug.trim_end_matches('-').chars().take(48).collect();
    if slug.is_empty() {
        "template".into()
    } else {
        slug
    }
}

/// `name`, or `name (2)`, `name (3)`… until `taken` says no.
pub fn free_name(name: &str, taken: impl Fn(&str) -> bool) -> String {
    let name = clean_name(name);
    if !taken(&name) && !taken(&stem(&name)) {
        return name;
    }
    (2..)
        .map(|n| format!("{name} ({n})"))
        .find(|candidate| !taken(candidate) && !taken(&stem(candidate)))
        .expect("an unused suffix exists")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Template {
        Template {
            name: "Release plan".into(),
            kind: "checklist".into(),
            title: "Release".into(),
            spec: json!({"items": [{"id": "tag", "label": "Tag it"}]}),
            saved_at: 1_700_000_000,
        }
    }

    #[test]
    fn round_trips_its_file() {
        let t = sample();
        assert_eq!(Template::parse(&t.to_json()).unwrap(), t);
    }

    #[test]
    fn rejects_bad_files() {
        assert!(Template::parse("not json").is_err());
        assert!(Template::parse(r#"{"kind": "spaceship", "name": "x", "spec": {}}"#).is_err());
        assert!(Template::parse(r#"{"kind": "table", "name": "x", "spec": []}"#).is_err());
        assert!(Template::parse(r#"{"kind": "table", "spec": {}}"#).is_err());
        assert!(
            Template::parse(r#"{"format": 9, "kind": "table", "name": "x", "spec": {}}"#).is_err()
        );
        // The title stands in for a missing name.
        let t = Template::parse(r#"{"kind": "table", "title": " Flaky ", "spec": {}}"#).unwrap();
        assert_eq!((t.name.as_str(), t.saved_at), ("Flaky", 0));
    }

    #[test]
    fn insert_command_is_one_line_with_the_spec() {
        let command = sample().insert_command();
        let json = command
            .strip_prefix("/gentle-ui:event ")
            .expect("an event command");
        assert!(!json.contains('\n'));
        let event: Value = serde_json::from_str(json).unwrap();
        assert_eq!(event["op"], "insert");
        assert_eq!(event["id"], "release-plan");
        assert_eq!(event["kind"], "checklist");
        assert_eq!(event["spec"]["items"][0]["id"], "tag");
    }

    #[test]
    fn names_and_stems() {
        assert_eq!(stem("Release plan — Q3!"), "release-plan-q3");
        assert_eq!(stem("!!"), "template");
        assert_eq!(clean_name("  two\n lines "), "two lines");
        let taken = ["Plan", "plan-2"];
        assert_eq!(free_name("Plan", |n| taken.contains(&n)), "Plan (3)");
        assert_eq!(free_name("Other", |n| taken.contains(&n)), "Other");
    }
}
