//! The diagram spec: typed model, lenient parsing with readable errors, and
//! serialization back to the wire JSON the agent reads.

use std::collections::HashSet;

use serde_json::{Map, Value, json};

pub const MAX_NODES: usize = 200;
pub const MAX_EDGES: usize = 400;
pub const MAX_GROUPS: usize = 50;
pub const MAX_ID: usize = 64;
pub const MAX_LABEL: usize = 80;
pub const MAX_TEXT: usize = 1000;

/// Hard errors past this many are dropped: the agent only needs enough to fix the spec.
const MAX_ERRORS: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum NodeKind {
    #[default]
    Service,
    Database,
    Queue,
    Cache,
    Client,
    External,
    Storage,
    Function,
    Note,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Status {
    Planned,
    Existing,
    Deprecated,
    Risk,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum EdgeStyle {
    #[default]
    Sync,
    Async,
    Data,
    Dependency,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum Direction {
    #[default]
    LeftRight,
    TopBottom,
}

/// `ALL`, `name` (wire name) and case-insensitive `parse` for a wire enum.
macro_rules! wire_enum {
    ($ty:ident, $n:literal, [$($variant:ident => $name:literal),+ $(,)?]) => {
        impl $ty {
            pub const ALL: [Self; $n] = [$(Self::$variant),+];

            pub fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => $name),+
                }
            }

            pub fn parse(s: &str) -> Option<Self> {
                let s = s.trim();
                Self::ALL.into_iter().find(|v| v.name().eq_ignore_ascii_case(s))
            }
        }
    };
}

wire_enum!(NodeKind, 9, [
    Service => "service",
    Database => "database",
    Queue => "queue",
    Cache => "cache",
    Client => "client",
    External => "external",
    Storage => "storage",
    Function => "function",
    Note => "note",
]);
wire_enum!(Status, 4, [
    Planned => "planned",
    Existing => "existing",
    Deprecated => "deprecated",
    Risk => "risk",
]);
wire_enum!(EdgeStyle, 4, [
    Sync => "sync",
    Async => "async",
    Data => "data",
    Dependency => "dependency",
]);
wire_enum!(Direction, 2, [LeftRight => "LR", TopBottom => "TB"]);

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub id: String,
    pub label: String,
    pub kind: NodeKind,
    pub group: Option<String>,
    pub description: Option<String>,
    pub tech: Option<String>,
    pub status: Option<Status>,
    pub x: Option<f32>,
    pub y: Option<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Edge {
    pub id: String,
    pub from: String,
    pub to: String,
    pub label: Option<String>,
    pub style: EdgeStyle,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Diagram {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub groups: Vec<Group>,
    pub direction: Direction,
}

impl Node {
    pub fn new(id: impl Into<String>, label: impl Into<String>, kind: NodeKind) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            kind,
            group: None,
            description: None,
            tech: None,
            status: None,
            x: None,
            y: None,
        }
    }

    pub fn to_value(&self) -> Value {
        let mut map = Map::new();
        map.insert("id".into(), json!(self.id));
        map.insert("label".into(), json!(self.label));
        map.insert("kind".into(), json!(self.kind.name()));
        insert_some(&mut map, "group", self.group.as_deref());
        insert_some(&mut map, "description", self.description.as_deref());
        insert_some(&mut map, "tech", self.tech.as_deref());
        insert_some(&mut map, "status", self.status.map(Status::name));
        if let (Some(x), Some(y)) = (self.x, self.y) {
            map.insert("x".into(), whole(x));
            map.insert("y".into(), whole(y));
        }
        Value::Object(map)
    }
}

impl Edge {
    pub fn to_value(&self) -> Value {
        let mut map = Map::new();
        map.insert("id".into(), json!(self.id));
        map.insert("from".into(), json!(self.from));
        map.insert("to".into(), json!(self.to));
        insert_some(&mut map, "label", self.label.as_deref());
        map.insert("style".into(), json!(self.style.name()));
        Value::Object(map)
    }
}

impl Group {
    pub fn to_value(&self) -> Value {
        let mut map = Map::new();
        map.insert("id".into(), json!(self.id));
        map.insert("label".into(), json!(self.label));
        insert_some(&mut map, "description", self.description.as_deref());
        Value::Object(map)
    }
}

impl Diagram {
    pub fn parse(spec: &Value) -> Result<Diagram, Vec<String>> {
        let Some(spec) = spec.as_object() else {
            return Err(vec!["the spec must be a JSON object".into()]);
        };
        let mut errors = Errors::default();

        let nodes_raw = match spec.get("nodes") {
            None | Some(Value::Null) => &[][..],
            Some(Value::Array(items)) => items.as_slice(),
            Some(_) => {
                errors.push("\"nodes\" must be an array".into());
                &[][..]
            }
        };
        // Edges and groups of the wrong type are ignored rather than rejected (lenient).
        let edges_raw = array(spec.get("edges"));
        let groups_raw = array(spec.get("groups"));
        for (what, count, max) in [
            ("nodes", nodes_raw.len(), MAX_NODES),
            ("edges", edges_raw.len(), MAX_EDGES),
            ("groups", groups_raw.len(), MAX_GROUPS),
        ] {
            if count > max {
                errors.push(format!("too many {what} ({count}, at most {max})"));
            }
        }

        let direction = spec
            .get("direction")
            .and_then(Value::as_str)
            .and_then(Direction::parse)
            .unwrap_or_default();

        let mut groups: Vec<Group> = Vec::new();
        for (i, raw) in groups_raw.iter().enumerate() {
            let Some(id) = raw.get("id").and_then(id_of) else {
                errors.push(format!("group #{} has no id", i + 1));
                continue;
            };
            if groups.iter().any(|g| g.id == id) {
                errors.push(format!("duplicate group id \"{id}\""));
                continue;
            }
            groups.push(Group {
                label: text(raw.get("label"), MAX_LABEL).unwrap_or_else(|| id.clone()),
                description: text(raw.get("description"), MAX_TEXT),
                id,
            });
        }

        let mut nodes: Vec<Node> = Vec::new();
        let mut node_ids: HashSet<String> = HashSet::new();
        for (i, raw) in nodes_raw.iter().enumerate() {
            let Some(id) = raw.get("id").and_then(id_of) else {
                errors.push(format!("node #{} has no id", i + 1));
                continue;
            };
            if !node_ids.insert(id.clone()) {
                errors.push(format!("duplicate node id \"{id}\""));
                continue;
            }
            let group = raw.get("group").and_then(id_of);
            if let Some(group) = &group
                && !groups.iter().any(|g| &g.id == group)
            {
                errors.push(format!("node \"{id}\": group \"{group}\" is not in groups"));
            }
            let (x, y) = match (number(raw.get("x")), number(raw.get("y"))) {
                (Some(x), Some(y)) => (Some(x), Some(y)),
                _ => (None, None),
            };
            nodes.push(Node {
                label: text(raw.get("label"), MAX_LABEL).unwrap_or_else(|| id.clone()),
                kind: raw
                    .get("kind")
                    .and_then(Value::as_str)
                    .and_then(NodeKind::parse)
                    .unwrap_or_default(),
                group,
                description: text(raw.get("description"), MAX_TEXT),
                tech: text(raw.get("tech"), MAX_LABEL),
                status: raw
                    .get("status")
                    .and_then(Value::as_str)
                    .and_then(Status::parse),
                x,
                y,
                id,
            });
        }

        // Explicit ids are reserved first so a generated id never collides with a later explicit one.
        let explicit: HashSet<String> = edges_raw
            .iter()
            .filter_map(|raw| raw.get("id").and_then(id_of))
            .collect();
        let mut edges: Vec<Edge> = Vec::new();
        let mut edge_ids: HashSet<String> = HashSet::new();
        for (i, raw) in edges_raw.iter().enumerate() {
            let from = raw.get("from").and_then(id_of);
            let to = raw.get("to").and_then(id_of);
            let (Some(from), Some(to)) = (from, to) else {
                errors.push(format!("edge #{} needs both \"from\" and \"to\"", i + 1));
                continue;
            };
            let id = match raw.get("id").and_then(id_of) {
                Some(id) => {
                    if !edge_ids.insert(id.clone()) {
                        errors.push(format!("duplicate edge id \"{id}\""));
                        continue;
                    }
                    id
                }
                None => {
                    let id = first_free_edge_id(&from, &to, |id| {
                        explicit.contains(id) || edge_ids.contains(id)
                    });
                    edge_ids.insert(id.clone());
                    id
                }
            };
            for end in [&from, &to] {
                if !node_ids.contains(end) {
                    errors.push(format!("edge \"{id}\": node \"{end}\" does not exist"));
                }
            }
            edges.push(Edge {
                label: text(raw.get("label"), MAX_LABEL),
                style: raw
                    .get("style")
                    .and_then(Value::as_str)
                    .and_then(EdgeStyle::parse)
                    .unwrap_or_default(),
                id,
                from,
                to,
            });
        }

        if !errors.0.is_empty() {
            return Err(errors.0);
        }
        Ok(Diagram {
            nodes,
            edges,
            groups,
            direction,
        })
    }

    pub fn to_value(&self) -> Value {
        json!({
            "direction": self.direction.name(),
            "groups": self.groups.iter().map(Group::to_value).collect::<Vec<_>>(),
            "nodes": self.nodes.iter().map(Node::to_value).collect::<Vec<_>>(),
            "edges": self.edges.iter().map(Edge::to_value).collect::<Vec<_>>(),
        })
    }

    pub fn node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn node_index(&self, id: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.id == id)
    }

    pub fn edge(&self, id: &str) -> Option<&Edge> {
        self.edges.iter().find(|e| e.id == id)
    }

    pub fn group(&self, id: &str) -> Option<&Group> {
        self.groups.iter().find(|g| g.id == id)
    }

    /// Slug of `base` ("New node" → "new-node"), then "-2", "-3"… until unused.
    pub fn fresh_node_id(&self, base: &str) -> String {
        fresh_slug(base, "node", |id| self.node(id).is_some())
    }

    /// Like [`Self::fresh_node_id`], among the group ids.
    pub fn fresh_group_id(&self, base: &str) -> String {
        fresh_slug(base, "group", |id| self.group(id).is_some())
    }

    /// `"<from>-><to>"`, or `"#2"`, `"#3"`… appended when taken.
    pub fn fresh_edge_id(&self, from: &str, to: &str) -> String {
        first_free_edge_id(from, to, |id| self.edge(id).is_some())
    }
}

fn first_free_edge_id(from: &str, to: &str, taken: impl Fn(&str) -> bool) -> String {
    let base = format!("{from}->{to}");
    if !taken(&base) {
        return base;
    }
    (2..)
        .map(|n| format!("{base}#{n}"))
        .find(|id| !taken(id))
        .expect("an unused suffix exists")
}

fn fresh_slug(base: &str, fallback: &str, taken: impl Fn(&str) -> bool) -> String {
    let mut slug = String::new();
    for c in base.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() || c == '_' {
            slug.push(c);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_end_matches('-');
    // Leave room for a "-N" suffix within MAX_ID.
    let slug: String = if slug.is_empty() {
        fallback.into()
    } else {
        slug.chars().take(MAX_ID - 4).collect()
    };
    if !taken(&slug) {
        return slug;
    }
    (2..)
        .map(|n| format!("{slug}-{n}"))
        .find(|id| !taken(id))
        .expect("an unused suffix exists")
}

#[derive(Default)]
pub(super) struct Errors(pub(super) Vec<String>);

impl Errors {
    pub(super) fn push(&mut self, error: String) {
        if self.0.len() < MAX_ERRORS {
            self.0.push(error);
        }
    }
}

pub(super) fn array(value: Option<&Value>) -> &[Value] {
    value.and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

/// A non-empty string or a number (models sometimes number their nodes), cut to MAX_ID.
pub(super) fn id_of(value: &Value) -> Option<String> {
    let id = match value {
        Value::String(s) => s.trim().to_string(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    if id.is_empty() {
        return None;
    }
    Some(id.chars().take(MAX_ID).collect())
}

/// Trimmed non-empty text, cut to `max` chars with a trailing "…".
pub(super) fn text(value: Option<&Value>, max: usize) -> Option<String> {
    let s = value?.as_str()?.trim();
    (!s.is_empty()).then(|| truncate(s, max))
}

pub(crate) fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn number(value: Option<&Value>) -> Option<f32> {
    let n = value?.as_f64()? as f32;
    n.is_finite().then_some(n)
}

fn whole(n: f32) -> Value {
    json!(n.round() as i64)
}

fn insert_some(map: &mut Map<String, Value>, key: &str, value: Option<&str>) {
    if let Some(value) = value {
        map.insert(key.into(), json!(value));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(spec: Value) -> Diagram {
        Diagram::parse(&spec).expect("valid spec")
    }

    fn errors(spec: Value) -> Vec<String> {
        Diagram::parse(&spec).expect_err("invalid spec")
    }

    #[test]
    fn rejects_non_objects_and_bad_nodes() {
        assert_eq!(errors(json!([])).len(), 1);
        assert_eq!(
            errors(json!({"nodes": "a"})),
            ["\"nodes\" must be an array"]
        );
        assert_eq!(
            errors(json!({"nodes": [{"label": "x"}]})),
            ["node #1 has no id"]
        );
    }

    #[test]
    fn reports_duplicates_missing_endpoints_and_unknown_groups() {
        let errs = errors(json!({
            "groups": [{"id": "g"}, {"id": "g"}],
            "nodes": [{"id": "api", "group": "core"}, {"id": "api"}],
            "edges": [{"from": "api", "to": "db"}, {"id": "e", "from": "api", "to": "api"}, {"id": "e", "from": "api", "to": "api"}],
        }));
        assert_eq!(
            errs,
            [
                "duplicate group id \"g\"",
                "node \"api\": group \"core\" is not in groups",
                "duplicate node id \"api\"",
                "edge \"api->db\": node \"db\" does not exist",
                "duplicate edge id \"e\"",
            ]
        );
    }

    #[test]
    fn enforces_limits_and_caps_error_count() {
        let nodes: Vec<Value> = (0..250).map(|i| json!({"id": format!("n{i}")})).collect();
        assert_eq!(
            errors(json!({"nodes": nodes})),
            ["too many nodes (250, at most 200)"]
        );
        let edges: Vec<Value> = (0..30).map(|i| json!({"from": "a", "to": i})).collect();
        assert_eq!(
            errors(json!({"nodes": [{"id": "a"}], "edges": edges})).len(),
            MAX_ERRORS
        );
    }

    #[test]
    fn normalizes_leniently() {
        let long = "x".repeat(100);
        let d = parse(json!({
            "direction": "tb",
            "nodes": [
                {"id": 1, "label": "  ", "kind": "spaceship", "status": "meh", "x": 10},
                {"id": "b", "label": long, "kind": "Database", "x": 1.4, "y": 2.6},
            ],
            "edges": [
                {"from": 1, "to": "b", "style": "weird"},
                {"from": "1", "to": "b", "label": ""},
                {"id": "1->b#3", "from": "b", "to": "1", "style": "async"},
                {"from": "1", "to": "b"},
            ],
        }));
        assert_eq!(d.direction, Direction::TopBottom);
        let a = &d.nodes[0];
        assert_eq!((a.id.as_str(), a.label.as_str()), ("1", "1"));
        assert_eq!(
            (a.kind, a.status, a.x, a.y),
            (NodeKind::Service, None, None, None)
        );
        let b = &d.nodes[1];
        assert_eq!(b.kind, NodeKind::Database);
        assert_eq!(b.label.chars().count(), MAX_LABEL);
        assert!(b.label.ends_with('…'));
        let ids: Vec<&str> = d.edges.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["1->b", "1->b#2", "1->b#3", "1->b#4"]);
        assert_eq!(d.edges[0].style, EdgeStyle::Sync);
        assert_eq!(d.edges[1].label, None);
        assert_eq!(d.edges[2].style, EdgeStyle::Async);
    }

    #[test]
    fn truncation_is_char_safe() {
        let s = "é".repeat(MAX_TEXT + 5);
        let t = truncate(&s, MAX_TEXT);
        assert_eq!(t.chars().count(), MAX_TEXT);
        assert!(t.ends_with('…'));
    }

    #[test]
    fn to_value_round_trips() {
        let d = parse(json!({
            "direction": "LR",
            "groups": [{"id": "core", "label": "Core", "description": "the core"}],
            "nodes": [
                {"id": "api", "label": "API", "group": "core", "tech": "Rust", "status": "risk", "x": 10, "y": 20},
                {"id": "db", "label": "DB", "kind": "database", "description": "orders"},
            ],
            "edges": [{"from": "api", "to": "db", "label": "reads", "style": "data"}],
        }));
        let value = d.to_value();
        assert_eq!(value["edges"][0]["id"], "api->db");
        assert_eq!(value["nodes"][1]["kind"], "database");
        assert!(value["nodes"][1].get("x").is_none());
        assert_eq!(parse(value), d);
    }

    #[test]
    fn fresh_ids_skip_used_ones() {
        let d = parse(json!({
            "nodes": [{"id": "new-node"}, {"id": "new-node-2"}, {"id": "b"}],
            "edges": [{"from": "new-node", "to": "b"}],
        }));
        assert_eq!(d.fresh_node_id("New node"), "new-node-3");
        assert_eq!(d.fresh_node_id("  Cache!! "), "cache");
        assert_eq!(d.fresh_node_id("!!"), "node");
        assert_eq!(d.fresh_edge_id("new-node", "b"), "new-node->b#2");
        assert_eq!(d.fresh_edge_id("b", "new-node"), "b->new-node");
        let mut grouped = d.clone();
        grouped.groups.push(Group {
            id: "group".into(),
            label: "Group".into(),
            description: None,
        });
        assert_eq!(grouped.fresh_group_id("Back end"), "back-end");
        assert_eq!(grouped.fresh_group_id(""), "group-2");
    }
}
