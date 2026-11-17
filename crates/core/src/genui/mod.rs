//! Generative UI: interactive surfaces the agent shows through the
//! `gentle_ui` tool (phase 1: architecture diagrams) and the user's edits
//! sent back to it as `gentle-ui:event` commands.

pub mod changes;
pub mod diagram;
pub mod layout;

use serde_json::{Map, Value, json};

pub const TOOL: &str = "gentle_ui";
pub const EVENT_MESSAGE: &str = "gentle-ui.event";
pub const EVENT_COMMAND: &str = "gentle-ui:event";

/// One version of a surface: a gentle_ui tool result's details or a gentle-ui.event message's details.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub id: String,
    pub version: u64,
    pub kind: String,
    pub title: String,
    pub spec: Value,
    pub closed: bool,
}

/// None unless `details.id` is a non-empty string.
pub fn snapshot(details: &Value) -> Option<Snapshot> {
    let id = details.get("id")?.as_str().filter(|id| !id.is_empty())?;
    let text = |key: &str| details.get(key).and_then(Value::as_str);
    Some(Snapshot {
        id: id.to_string(),
        version: details.get("version").and_then(Value::as_u64).unwrap_or(0),
        kind: text("kind").unwrap_or("diagram").to_string(),
        title: text("title").unwrap_or_default().to_string(),
        spec: details.get("spec").cloned().unwrap_or(Value::Null),
        closed: details.get("closed") == Some(&Value::Bool(true)),
    })
}

/// `/gentle-ui:event {json}` for the user's edits; the JSON stays on one line
/// because the extension reads everything after the first space.
pub fn event_command(
    id: &str,
    version: u64,
    changes: &[changes::Change],
    summary: &[String],
    spec: &diagram::Diagram,
    note: Option<&str>,
) -> String {
    let mut payload = Map::new();
    payload.insert("id".into(), json!(id));
    payload.insert("version".into(), json!(version));
    payload.insert(
        "changes".into(),
        Value::Array(changes.iter().map(changes::Change::to_value).collect()),
    );
    payload.insert("summary".into(), json!(summary));
    payload.insert("spec".into(), spec.to_value());
    if let Some(note) = note.map(str::trim).filter(|n| !n.is_empty()) {
        payload.insert("note".into(), json!(note));
    }
    let json = serde_json::to_string(&Value::Object(payload)).expect("JSON values serialize");
    format!("/{EVENT_COMMAND} {json}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_reads_details_with_defaults() {
        assert_eq!(snapshot(&json!({"id": ""})), None);
        assert_eq!(snapshot(&json!({"version": 2})), None);
        assert_eq!(
            snapshot(&json!({"id": "ui-1"})),
            Some(Snapshot {
                id: "ui-1".into(),
                version: 0,
                kind: "diagram".into(),
                title: String::new(),
                spec: Value::Null,
                closed: false,
            })
        );
        let closed = snapshot(&json!({
            "id": "ui-2", "version": 3, "kind": "diagram", "title": "Checkout",
            "spec": null, "closed": true,
        }))
        .unwrap();
        assert_eq!(
            (closed.version, closed.title.as_str(), closed.closed),
            (3, "Checkout", true)
        );
    }

    #[test]
    fn event_command_is_one_line_with_optional_note() {
        let d = diagram::Diagram::parse(&json!({"nodes": [{"id": "a"}]})).unwrap();
        let moved = changes::Change::Move {
            node: "a".into(),
            x: 1.,
            y: 2.,
        };
        let summary = vec!["moved a\nsomewhere".to_string()];
        let cmd = event_command("ui-1", 3, &[moved], &summary, &d, Some("  "));
        assert!(!cmd.contains('\n'));
        let json = cmd.strip_prefix("/gentle-ui:event ").unwrap();
        let value: Value = serde_json::from_str(json).unwrap();
        assert_eq!(value["version"], 3);
        assert_eq!(value["changes"][0]["op"], "move");
        assert_eq!(value["spec"]["nodes"][0]["id"], "a");
        assert!(value.get("note").is_none());

        let with_note = event_command("ui-1", 3, &[], &[], &d, Some("keep it small"));
        assert!(with_note.ends_with(r#""note":"keep it small"}"#));
    }
}
