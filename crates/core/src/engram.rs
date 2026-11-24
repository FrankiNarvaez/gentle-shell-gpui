//! Engram's HTTP protocol, pure side: the observation model, decoding API
//! answers and percent-encoding query values. The app does the requests.
//! Also what gentle-engram's session status points at ([`status_target`]).

use serde::Deserialize;

/// Engram's `store.Observation` (internal/store/store.go), the fields shown here.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Observation {
    pub id: i64,
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub content: String,
    pub project: Option<String>,
    #[serde(default)]
    pub scope: String,
    pub topic_key: Option<String>,
    #[serde(default)]
    pub revision_count: i64,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

pub fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

pub fn parse_observations(value: serde_json::Value) -> Vec<Observation> {
    match value {
        serde_json::Value::Array(items) => items
            .into_iter()
            .filter_map(|v| serde_json::from_value(v).ok())
            .collect(),
        _ => Vec::new(),
    }
}

/// What the Memory screen opens on: a project's memories, and one of them
/// selected (its detail shown) when known.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryTarget {
    pub project: Option<String>,
    pub observation: Option<i64>,
}

/// gentle-engram's session status (key `engram`, `🧠 <project> · <state>`)
/// as a place in the Memory screen: the project it names (`unknown` is
/// none) and, after a save, update, read or review, that observation.
/// A deleted one or a saved prompt is no observation to show. `None` for
/// any other text, `🧠 memory disabled` included.
pub fn status_target(status: &str) -> Option<MemoryTarget> {
    let (project, state) = status.trim().strip_prefix('🧠')?.split_once(" · ")?;
    let project = project.trim();
    let observation = ["saved #", "updated #", "observation #", "reviewed #"]
        .iter()
        .find_map(|verb| state.trim().strip_prefix("✓ ")?.strip_prefix(verb))
        .and_then(|id| id.parse().ok());
    Some(MemoryTarget {
        project: (!project.is_empty() && project != "unknown").then(|| project.to_string()),
        observation,
    })
}

#[cfg(test)]
mod tests {
    use super::{MemoryTarget, encode, parse_observations, status_target};

    #[test]
    fn observations_and_search_results_parse() {
        // `/search` returns SearchResult = Observation + rank, flattened.
        let value = serde_json::json!([
            {"id": 7, "sync_id": "s", "session_id": "x", "type": "decision", "title": "Use GPUI", "content": "Because…",
             "project": "gentle-shell-gpui", "scope": "project", "topic_key": "arch/ui", "revision_count": 2,
             "duplicate_count": 0, "created_at": "2026-11-06T10:00:00Z", "updated_at": "2026-11-06T10:05:00Z", "rank": -1.5},
            {"unexpected": true}
        ]);
        let items = parse_observations(value);
        assert_eq!(items.len(), 1);
        assert_eq!(
            (
                items[0].id,
                items[0].kind.as_str(),
                items[0].topic_key.as_deref()
            ),
            (7, "decision", Some("arch/ui"))
        );
        assert!(parse_observations(serde_json::json!({"error": "x"})).is_empty());
    }

    #[test]
    fn query_values_are_percent_encoded() {
        assert_eq!(encode("/Users/a b/c&d"), "%2FUsers%2Fa%20b%2Fc%26d");
    }

    #[test]
    fn engram_status_points_at_its_project_and_observation() {
        let target = |project: Option<&str>, observation| {
            Some(MemoryTarget {
                project: project.map(str::to_string),
                observation,
            })
        };
        assert_eq!(
            status_target("🧠 gentle-shell-gpui · ✓ saved #42"),
            target(Some("gentle-shell-gpui"), Some(42))
        );
        assert_eq!(
            status_target("🧠 app · ✓ updated #7"),
            target(Some("app"), Some(7))
        );
        assert_eq!(
            status_target("🧠 app · ✓ observation #3"),
            target(Some("app"), Some(3))
        );
        assert_eq!(
            status_target("🧠 app · ✓ reviewed #9"),
            target(Some("app"), Some(9))
        );
        // Gone or not an observation: the project's memories.
        assert_eq!(
            status_target("🧠 app · ✓ deleted #4"),
            target(Some("app"), None)
        );
        assert_eq!(
            status_target("🧠 app · ✓ prompt #5"),
            target(Some("app"), None)
        );
        assert_eq!(
            status_target("🧠 app · ✓ 3 results"),
            target(Some("app"), None)
        );
        assert_eq!(status_target("🧠 app · ready"), target(Some("app"), None));
        assert_eq!(status_target("🧠 app · save…"), target(Some("app"), None));
        assert_eq!(status_target("🧠 unknown · offline"), target(None, None));
        assert_eq!(status_target("🧠 memory disabled"), None);
        assert_eq!(status_target("server ready"), None);
    }
}
