//! User edits to a diagram as discrete changes: diffed from two versions,
//! applied one by one, and described in plain sentences for the agent.

use std::collections::HashSet;

use serde_json::{Map, Value, json};

use super::diagram::{
    Diagram, Edge, EdgeStyle, MAX_EDGES, MAX_LABEL, MAX_NODES, MAX_TEXT, Node, NodeKind, Status,
    truncate,
};
use super::layout::{self, Layout};

/// None = unchanged, Some(None) = cleared.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NodePatch {
    pub label: Option<String>,
    pub kind: Option<NodeKind>,
    pub group: Option<Option<String>>,
    pub description: Option<Option<String>>,
    pub tech: Option<Option<String>>,
    pub status: Option<Option<Status>>,
}

impl NodePatch {
    fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    Move {
        node: String,
        x: f32,
        y: f32,
    },
    UpdateNode {
        node: String,
        patch: NodePatch,
    },
    AddNode {
        node: Node,
    },
    /// Also removes the node's edges.
    RemoveNode {
        node: String,
    },
    AddEdge {
        edge: Edge,
    },
    UpdateEdge {
        edge: String,
        label: Option<Option<String>>,
        style: Option<EdgeStyle>,
    },
    RemoveEdge {
        edge: String,
    },
}

impl Change {
    pub fn to_value(&self) -> Value {
        match self {
            Change::Move { node, x, y } => json!({
                "op": "move",
                "node": node,
                "x": x.round() as i64,
                "y": y.round() as i64,
            }),
            Change::UpdateNode { node, patch } => {
                let mut map = Map::new();
                map.insert("op".into(), json!("update_node"));
                map.insert("node".into(), json!(node));
                if let Some(label) = &patch.label {
                    map.insert("label".into(), json!(label));
                }
                if let Some(kind) = patch.kind {
                    map.insert("kind".into(), json!(kind.name()));
                }
                if let Some(group) = &patch.group {
                    map.insert("group".into(), json!(group));
                }
                if let Some(description) = &patch.description {
                    map.insert("description".into(), json!(description));
                }
                if let Some(tech) = &patch.tech {
                    map.insert("tech".into(), json!(tech));
                }
                if let Some(status) = patch.status {
                    map.insert("status".into(), json!(status.map(Status::name)));
                }
                Value::Object(map)
            }
            Change::AddNode { node } => json!({"op": "add_node", "node": node.to_value()}),
            Change::RemoveNode { node } => json!({"op": "remove_node", "node": node}),
            Change::AddEdge { edge } => json!({"op": "add_edge", "edge": edge.to_value()}),
            Change::UpdateEdge { edge, label, style } => {
                let mut map = Map::new();
                map.insert("op".into(), json!("update_edge"));
                map.insert("edge".into(), json!(edge));
                if let Some(label) = label {
                    map.insert("label".into(), json!(label));
                }
                if let Some(style) = style {
                    map.insert("style".into(), json!(style.name()));
                }
                Value::Object(map)
            }
            Change::RemoveEdge { edge } => json!({"op": "remove_edge", "edge": edge}),
        }
    }
}

/// Changes turning `old` into `new`, in apply order: remove_edge, remove_node,
/// add_node, update_node, move, add_edge, update_edge. Groups and direction
/// are not diffed (the UI doesn't edit them).
pub fn diff(old: &Diagram, new: &Diagram) -> Vec<Change> {
    let removed_nodes: HashSet<&str> = old
        .nodes
        .iter()
        .filter(|n| new.node(&n.id).is_none())
        .map(|n| n.id.as_str())
        .collect();
    // An edge whose endpoints changed is a different connection: remove + add.
    let same_ends = |a: &Edge, b: &Edge| a.from == b.from && a.to == b.to;
    let kept_edge = |e: &Edge| new.edge(&e.id).filter(|n| same_ends(e, n));

    let mut changes = Vec::new();
    for e in &old.edges {
        let vanished_with_node =
            removed_nodes.contains(e.from.as_str()) || removed_nodes.contains(e.to.as_str());
        if kept_edge(e).is_none() && !vanished_with_node {
            changes.push(Change::RemoveEdge { edge: e.id.clone() });
        }
    }
    for n in &old.nodes {
        if removed_nodes.contains(n.id.as_str()) {
            changes.push(Change::RemoveNode { node: n.id.clone() });
        }
    }
    for n in &new.nodes {
        if old.node(&n.id).is_none() {
            changes.push(Change::AddNode { node: n.clone() });
        }
    }
    let mut moves = Vec::new();
    for n in &new.nodes {
        let Some(o) = old.node(&n.id) else { continue };
        let patch = NodePatch {
            label: (o.label != n.label).then(|| n.label.clone()),
            kind: (o.kind != n.kind).then_some(n.kind),
            group: (o.group != n.group).then(|| n.group.clone()),
            description: (o.description != n.description).then(|| n.description.clone()),
            tech: (o.tech != n.tech).then(|| n.tech.clone()),
            status: (o.status != n.status).then_some(n.status),
        };
        if !patch.is_empty() {
            changes.push(Change::UpdateNode {
                node: n.id.clone(),
                patch,
            });
        }
        // Unpinning can't be expressed; the UI only ever pins.
        if let (Some(x), Some(y)) = (n.x, n.y)
            && (o.x, o.y) != (n.x, n.y)
        {
            moves.push(Change::Move {
                node: n.id.clone(),
                x,
                y,
            });
        }
    }
    changes.append(&mut moves);
    let mut updates = Vec::new();
    for e in &new.edges {
        let Some(o) = old.edge(&e.id).filter(|o| same_ends(o, e)) else {
            changes.push(Change::AddEdge { edge: e.clone() });
            continue;
        };
        if o.label != e.label || o.style != e.style {
            updates.push(Change::UpdateEdge {
                edge: e.id.clone(),
                label: (o.label != e.label).then(|| e.label.clone()),
                style: (o.style != e.style).then_some(e.style),
            });
        }
    }
    changes.append(&mut updates);
    changes
}

/// Applies one change; on error `d` is unchanged (everything is checked before mutating).
pub fn apply(d: &mut Diagram, change: &Change) -> Result<(), String> {
    match change {
        Change::Move { node, x, y } => {
            let i = node_index(d, node)?;
            d.nodes[i].x = Some(*x);
            d.nodes[i].y = Some(*y);
        }
        Change::UpdateNode { node, patch } => {
            let i = node_index(d, node)?;
            if let Some(Some(group)) = &patch.group {
                check_group(d, node, group)?;
            }
            let n = &mut d.nodes[i];
            if let Some(label) = &patch.label {
                n.label = clean(label, MAX_LABEL).unwrap_or_else(|| n.id.clone());
            }
            if let Some(kind) = patch.kind {
                n.kind = kind;
            }
            if let Some(group) = &patch.group {
                n.group = group.clone();
            }
            if let Some(description) = &patch.description {
                n.description = description.as_deref().and_then(|s| clean(s, MAX_TEXT));
            }
            if let Some(tech) = &patch.tech {
                n.tech = tech.as_deref().and_then(|s| clean(s, MAX_LABEL));
            }
            if let Some(status) = patch.status {
                n.status = status;
            }
        }
        Change::AddNode { node } => {
            if node.id.trim().is_empty() {
                return Err("a new node needs an id".into());
            }
            if d.node(&node.id).is_some() {
                return Err(format!("node \"{}\" already exists", node.id));
            }
            if d.nodes.len() >= MAX_NODES {
                return Err(format!("too many nodes (at most {MAX_NODES})"));
            }
            if let Some(group) = &node.group {
                check_group(d, &node.id, group)?;
            }
            d.nodes.push(node.clone());
        }
        Change::RemoveNode { node } => {
            let i = node_index(d, node)?;
            d.nodes.remove(i);
            d.edges.retain(|e| &e.from != node && &e.to != node);
        }
        Change::AddEdge { edge } => {
            if edge.id.trim().is_empty() {
                return Err("a new edge needs an id".into());
            }
            if d.edge(&edge.id).is_some() {
                return Err(format!("edge \"{}\" already exists", edge.id));
            }
            if d.edges.len() >= MAX_EDGES {
                return Err(format!("too many edges (at most {MAX_EDGES})"));
            }
            for end in [&edge.from, &edge.to] {
                if d.node(end).is_none() {
                    return Err(format!(
                        "edge \"{}\": node \"{end}\" does not exist",
                        edge.id
                    ));
                }
            }
            d.edges.push(edge.clone());
        }
        Change::UpdateEdge { edge, label, style } => {
            let e = d
                .edges
                .iter_mut()
                .find(|e| &e.id == edge)
                .ok_or_else(|| format!("unknown edge \"{edge}\""))?;
            if let Some(label) = label {
                e.label = label.as_deref().and_then(|s| clean(s, MAX_LABEL));
            }
            if let Some(style) = style {
                e.style = *style;
            }
        }
        Change::RemoveEdge { edge } => {
            let i = d
                .edges
                .iter()
                .position(|e| &e.id == edge)
                .ok_or_else(|| format!("unknown edge \"{edge}\""))?;
            d.edges.remove(i);
        }
    }
    Ok(())
}

/// One sentence about `change`, using labels rather than ids.
pub fn describe(
    change: &Change,
    before: &Diagram,
    after: &Diagram,
    after_layout: &Layout,
) -> String {
    let label = |id: &str| -> String {
        after
            .node(id)
            .or_else(|| before.node(id))
            .map_or_else(|| id.to_string(), |n| n.label.clone())
    };
    let group_label = |d: &Diagram, id: &str| {
        d.group(id)
            .map_or_else(|| id.to_string(), |g| g.label.clone())
    };
    let connection = |id: &str| -> String {
        match after.edge(id).or_else(|| before.edge(id)) {
            Some(e) => format!("{} → {}", label(&e.from), label(&e.to)),
            None => id.to_string(),
        }
    };
    match change {
        Change::Move { node, .. } => {
            let name = label(node);
            match nearest(after, after_layout, node) {
                Some(other) => format!("moved {name} next to {}", after.nodes[other].label),
                None => format!("moved {name}"),
            }
        }
        Change::UpdateNode { node, patch } => {
            let name = label(node);
            let mut parts = Vec::new();
            if let Some(new) = &patch.label {
                let old = before
                    .node(node)
                    .map_or(node.as_str(), |n| n.label.as_str());
                parts.push(format!("renamed {old} to {new}"));
            }
            if let Some(kind) = patch.kind {
                parts.push(format!("changed {name}'s kind to {}", kind.name()));
            }
            match patch.status {
                Some(Some(status)) => {
                    parts.push(format!("set {name}'s status to {}", status.name()))
                }
                Some(None) => parts.push(format!("cleared {name}'s status")),
                None => {}
            }
            match &patch.description {
                Some(Some(_)) => parts.push(format!("updated {name}'s description")),
                Some(None) => parts.push(format!("cleared {name}'s description")),
                None => {}
            }
            match &patch.tech {
                Some(Some(tech)) => parts.push(format!("updated {name}'s tech to {tech}")),
                Some(None) => parts.push(format!("cleared {name}'s tech")),
                None => {}
            }
            match &patch.group {
                Some(Some(group)) => {
                    parts.push(format!("moved {name} into {}", group_label(after, group)))
                }
                Some(None) => {
                    let old = before.node(node).and_then(|n| n.group.as_deref());
                    parts.push(match old {
                        Some(group) => format!("took {name} out of {}", group_label(before, group)),
                        None => format!("took {name} out of its group"),
                    });
                }
                None => {}
            }
            if parts.is_empty() {
                format!("updated {name}")
            } else {
                parts.join(", ")
            }
        }
        Change::AddNode { node } => format!("added {} ({})", node.label, node.kind.name()),
        Change::RemoveNode { node } => {
            let name = label(node);
            let edges = before
                .edges
                .iter()
                .filter(|e| &e.from == node || &e.to == node)
                .count();
            match edges {
                0 => format!("removed {name}"),
                1 => format!("removed {name} (and its 1 connection)"),
                n => format!("removed {name} (and its {n} connections)"),
            }
        }
        Change::AddEdge { edge } => {
            let mut s = format!("connected {} → {}", label(&edge.from), label(&edge.to));
            if let Some(l) = &edge.label {
                s.push_str(&format!(" (label: {l})"));
            }
            s
        }
        Change::UpdateEdge {
            edge,
            label: new_label,
            style,
        } => {
            let link = connection(edge);
            let mut parts = Vec::new();
            match new_label {
                Some(Some(l)) => parts.push(format!("relabeled {link} to “{l}”")),
                Some(None) => parts.push(format!("removed the label of {link}")),
                None => {}
            }
            if let Some(style) = style {
                parts.push(format!("made {link} {}", style.name()));
            }
            if parts.is_empty() {
                format!("updated {link}")
            } else {
                parts.join(", ")
            }
        }
        Change::RemoveEdge { edge } => format!("disconnected {}", connection(edge)),
    }
}

/// `diff` plus a sentence per change (laying out `new` once).
pub fn summarize(old: &Diagram, new: &Diagram) -> (Vec<Change>, Vec<String>) {
    let changes = diff(old, new);
    let after_layout = layout::layout(new);
    let sentences = changes
        .iter()
        .map(|c| describe(c, old, new, &after_layout))
        .collect();
    (changes, sentences)
}

/// The other node whose center is nearest to `id`'s.
fn nearest(d: &Diagram, l: &Layout, id: &str) -> Option<usize> {
    let i = d.node_index(id)?;
    let c = l.nodes.get(i)?.center();
    l.nodes
        .iter()
        .enumerate()
        .filter(|(j, _)| *j != i)
        .map(|(j, r)| {
            let o = r.center();
            (j, (o.x - c.x).hypot(o.y - c.y))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(j, _)| j)
}

fn node_index(d: &Diagram, id: &str) -> Result<usize, String> {
    d.node_index(id)
        .ok_or_else(|| format!("unknown node \"{id}\""))
}

fn check_group(d: &Diagram, node: &str, group: &str) -> Result<(), String> {
    match d.group(group) {
        Some(_) => Ok(()),
        None => Err(format!(
            "node \"{node}\": group \"{group}\" is not in groups"
        )),
    }
}

/// Same normalization as parsing: trimmed, empty → None, cut with "…".
fn clean(s: &str, max: usize) -> Option<String> {
    let s = s.trim();
    (!s.is_empty()).then(|| truncate(s, max))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn base() -> Diagram {
        Diagram::parse(&json!({
            "groups": [{"id": "backend", "label": "Backend"}],
            "nodes": [
                {"id": "api", "label": "API", "group": "backend", "tech": "Go", "status": "risk"},
                {"id": "db", "label": "Orders DB", "kind": "database", "description": "orders"},
                {"id": "queue", "label": "Queue", "kind": "queue"},
                {"id": "worker", "label": "Worker", "x": 400, "y": 100},
            ],
            "edges": [
                {"from": "api", "to": "db", "label": "reads"},
                {"from": "api", "to": "queue", "style": "async"},
                {"from": "queue", "to": "worker"},
                {"from": "worker", "to": "db"},
            ],
        }))
        .unwrap()
    }

    fn apply_all(d: &Diagram, changes: &[Change]) -> Diagram {
        let mut d = d.clone();
        for c in changes {
            apply(&mut d, c).unwrap();
        }
        d
    }

    fn assert_round_trip(old: &Diagram, new: &Diagram) -> Vec<Change> {
        let changes = diff(old, new);
        assert_eq!(&apply_all(old, &changes), new);
        changes
    }

    #[test]
    fn diff_then_apply_reproduces_the_new_diagram() {
        let old = base();

        let mut moved = old.clone();
        moved.nodes[0].x = Some(10.);
        moved.nodes[0].y = Some(20.);
        moved.nodes[3].x = Some(50.);
        assert_eq!(assert_round_trip(&old, &moved).len(), 2);

        let mut renamed = old.clone();
        renamed.nodes[1].label = "Orders".into();
        renamed.nodes[1].kind = NodeKind::Storage;
        renamed.nodes[0].group = None;
        renamed.nodes[0].tech = None;
        renamed.nodes[0].status = None;
        renamed.nodes[1].description = Some("all orders".into());
        renamed.nodes[2].group = Some("backend".into());
        assert_eq!(assert_round_trip(&old, &renamed).len(), 3);

        let mut grown = old.clone();
        grown.nodes.remove(3);
        grown
            .edges
            .retain(|e| e.from != "worker" && e.to != "worker");
        let mut cache = Node::new("cache", "Cache", NodeKind::Cache);
        cache.x = Some(0.);
        cache.y = Some(300.);
        grown.nodes.push(cache);
        grown.edges.push(Edge {
            id: "api->cache".into(),
            from: "api".into(),
            to: "cache".into(),
            label: Some("hot reads".into()),
            style: EdgeStyle::Data,
        });
        let changes = assert_round_trip(&old, &grown);
        assert_eq!(
            changes
                .iter()
                .map(|c| c.to_value()["op"].clone())
                .collect::<Vec<_>>(),
            [json!("remove_node"), json!("add_node"), json!("add_edge")]
        );

        let mut rewired = old.clone();
        rewired.edges[0].label = None;
        rewired.edges[1].label = Some("jobs".into());
        rewired.edges[1].style = EdgeStyle::Sync;
        rewired.edges.remove(2);
        assert_round_trip(&old, &rewired);

        assert!(diff(&old, &old).is_empty());
    }

    #[test]
    fn retargeted_edge_is_removed_and_re_added() {
        let old = base();
        let mut new = old.clone();
        new.edges[3].to = "api".into();
        let changes = assert_round_trip(&old, &new);
        assert!(matches!(changes[0], Change::RemoveEdge { .. }));
        assert!(matches!(changes[1], Change::AddEdge { .. }));
    }

    #[test]
    fn failed_apply_leaves_the_diagram_unchanged() {
        let d = base();
        let bad = [
            Change::Move {
                node: "nope".into(),
                x: 0.,
                y: 0.,
            },
            Change::UpdateNode {
                node: "api".into(),
                patch: NodePatch {
                    label: Some("New".into()),
                    group: Some(Some("frontend".into())),
                    ..NodePatch::default()
                },
            },
            Change::AddNode {
                node: Node::new("api", "API", NodeKind::Service),
            },
            Change::RemoveNode {
                node: "nope".into(),
            },
            Change::AddEdge {
                edge: Edge {
                    id: "x".into(),
                    from: "api".into(),
                    to: "ghost".into(),
                    label: None,
                    style: EdgeStyle::Sync,
                },
            },
            Change::AddEdge {
                edge: d.edges[0].clone(),
            },
            Change::UpdateEdge {
                edge: "nope".into(),
                label: None,
                style: None,
            },
            Change::RemoveEdge {
                edge: "nope".into(),
            },
        ];
        for change in &bad {
            let mut copy = d.clone();
            assert!(apply(&mut copy, change).is_err(), "{change:?}");
            assert_eq!(copy, d);
        }
        let mut copy = d.clone();
        let err = apply(&mut copy, &bad[4]).unwrap_err();
        assert_eq!(err, "edge \"x\": node \"ghost\" does not exist");
    }

    #[test]
    fn wire_format() {
        let update = Change::UpdateNode {
            node: "api".into(),
            patch: NodePatch {
                label: Some("Gateway".into()),
                status: Some(None),
                ..NodePatch::default()
            },
        };
        assert_eq!(
            update.to_value(),
            json!({"op": "update_node", "node": "api", "label": "Gateway", "status": null})
        );
        let mv = Change::Move {
            node: "api".into(),
            x: 120.4,
            y: 39.6,
        };
        assert_eq!(
            mv.to_value(),
            json!({"op": "move", "node": "api", "x": 120, "y": 40})
        );
        let edge = Change::UpdateEdge {
            edge: "a->b".into(),
            label: Some(None),
            style: Some(EdgeStyle::Async),
        };
        assert_eq!(
            edge.to_value(),
            json!({"op": "update_edge", "edge": "a->b", "label": null, "style": "async"})
        );
    }

    #[test]
    fn describes_changes_with_labels() {
        let old = base();
        let mut new = old.clone();
        new.nodes[3].x = Some(0.);
        new.nodes[3].y = Some(0.);
        new.nodes[0].group = None;
        new.nodes[1].label = "Orders".into();
        new.nodes[2].status = Some(Status::Planned);
        new.edges[1].label = Some("jobs".into());
        new.edges[1].style = EdgeStyle::Sync;
        new.edges.remove(2);
        new.edges.push(Edge {
            id: "api->worker".into(),
            from: "api".into(),
            to: "worker".into(),
            label: Some("spawns".into()),
            style: EdgeStyle::Sync,
        });
        let (changes, sentences) = summarize(&old, &new);
        assert_eq!(changes.len(), sentences.len());
        let l = layout::layout(&new);
        let worker = l.nodes[3].center();
        let distance = |r: &layout::Rect| (r.center().x - worker.x).hypot(r.center().y - worker.y);
        let nearest_label = ["API", "Orders", "Queue"]
            .into_iter()
            .zip(&l.nodes)
            .min_by(|a, b| distance(a.1).total_cmp(&distance(b.1)))
            .unwrap()
            .0;
        assert_eq!(
            sentences,
            [
                "disconnected Queue → Worker".to_string(),
                "took API out of Backend".into(),
                "renamed Orders DB to Orders".into(),
                "set Queue's status to planned".into(),
                format!("moved Worker next to {nearest_label}"),
                "connected API → Worker (label: spawns)".into(),
                "relabeled API → Queue to “jobs”, made API → Queue sync".into(),
            ]
        );

        let mut gone = old.clone();
        apply(&mut gone, &Change::RemoveNode { node: "api".into() }).unwrap();
        let l = layout::layout(&gone);
        let remove = Change::RemoveNode { node: "api".into() };
        assert_eq!(
            describe(&remove, &old, &gone, &l),
            "removed API (and its 2 connections)"
        );
        let add = Change::AddNode {
            node: Node::new("cache", "Cache", NodeKind::Cache),
        };
        assert_eq!(describe(&add, &old, &gone, &l), "added Cache (cache)");
    }
}
