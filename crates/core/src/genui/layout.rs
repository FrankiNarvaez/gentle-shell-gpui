//! Layered auto-layout for diagrams. It runs on every edit, so it is a
//! simple deterministic Sugiyama-style pass: break cycles, rank by longest
//! path, give each group its own band across ranks, order with barycenters.

use std::collections::HashMap;

use super::diagram::{Diagram, Direction};

pub const NODE_W: f32 = 168.;
pub const NODE_H: f32 = 60.;
pub const RANK_GAP: f32 = 96.;
pub const NODE_GAP: f32 = 32.;
pub const GROUP_PAD: f32 = 20.;
pub const GROUP_HEADER: f32 = 26.;
pub const GROUP_GAP: f32 = 40.;

const ORDER_SWEEPS: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x <= self.right() && p.y >= self.y && p.y <= self.bottom()
    }

    pub fn center(&self) -> Point {
        Point {
            x: self.x + self.w / 2.,
            y: self.y + self.h / 2.,
        }
    }

    /// Strict overlap: touching edges don't count.
    pub fn intersects(&self, o: &Rect) -> bool {
        self.x < o.right() && o.x < self.right() && self.y < o.bottom() && o.y < self.bottom()
    }

    pub fn union(&self, o: &Rect) -> Rect {
        let x = self.x.min(o.x);
        let y = self.y.min(o.y);
        Rect {
            x,
            y,
            w: self.right().max(o.right()) - x,
            h: self.bottom().max(o.bottom()) - y,
        }
    }

    pub fn inflate(&self, by: f32) -> Rect {
        Rect {
            x: self.x - by,
            y: self.y - by,
            w: self.w + 2. * by,
            h: self.h + 2. * by,
        }
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
}

/// A cubic Bézier from the source box's border to the target box's border.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EdgePath {
    pub from: Point,
    pub c1: Point,
    pub c2: Point,
    pub to: Point,
}

impl EdgePath {
    pub fn point_at(&self, t: f32) -> Point {
        let u = 1. - t;
        let (a, b, c, d) = (u * u * u, 3. * u * u * t, 3. * u * t * t, t * t * t);
        Point {
            x: a * self.from.x + b * self.c1.x + c * self.c2.x + d * self.to.x,
            y: a * self.from.y + b * self.c1.y + c * self.c2.y + d * self.to.y,
        }
    }

    pub fn label_at(&self) -> Point {
        self.point_at(0.5)
    }

    /// Sampled over 24 segments: plenty for hit-testing curves this short.
    pub fn distance_to(&self, p: Point) -> f32 {
        const SEGMENTS: usize = 24;
        let mut prev = self.from;
        let mut best = f32::INFINITY;
        for i in 1..=SEGMENTS {
            let next = self.point_at(i as f32 / SEGMENTS as f32);
            best = best.min(segment_distance(p, prev, next));
            prev = next;
        }
        best
    }

    /// Unit vector of travel at `to`, for the arrowhead.
    pub fn end_direction(&self) -> Point {
        // The tangent at t=1 is to - c2; fall back when control points coincide with the end.
        [self.c2, self.c1, self.from]
            .into_iter()
            .map(|p| Point {
                x: self.to.x - p.x,
                y: self.to.y - p.y,
            })
            .find_map(|v| {
                let len = v.x.hypot(v.y);
                (len > 1e-3).then(|| Point {
                    x: v.x / len,
                    y: v.y / len,
                })
            })
            .unwrap_or(Point { x: 1., y: 0. })
    }

    pub fn bounds(&self) -> Rect {
        let pts = [self.from, self.c1, self.c2, self.to];
        let min_x = pts.iter().map(|p| p.x).fold(f32::INFINITY, f32::min);
        let min_y = pts.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
        let max_x = pts.iter().map(|p| p.x).fold(f32::NEG_INFINITY, f32::max);
        let max_y = pts.iter().map(|p| p.y).fold(f32::NEG_INFINITY, f32::max);
        Rect {
            x: min_x,
            y: min_y,
            w: max_x - min_x,
            h: max_y - min_y,
        }
    }
}

/// `nodes`/`ranks` are index-aligned with `diagram.nodes`, `groups` with
/// `diagram.groups` (None: no members), `edges` with `diagram.edges`.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub nodes: Vec<Rect>,
    pub ranks: Vec<usize>,
    pub groups: Vec<Option<Rect>>,
    pub edges: Vec<EdgePath>,
    pub bounds: Rect,
}

impl Layout {
    /// The last (topmost) node containing `p`.
    pub fn node_at(&self, p: Point) -> Option<usize> {
        self.nodes.iter().rposition(|r| r.contains(p))
    }

    /// The nearest edge within `tolerance`.
    pub fn edge_at(&self, p: Point, tolerance: f32) -> Option<usize> {
        self.edges
            .iter()
            .enumerate()
            .map(|(i, e)| (i, e.distance_to(p)))
            .filter(|(_, d)| *d <= tolerance)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }
}

/// The automatic layout is computed for every node as if none were pinned,
/// then pinned nodes (x and y set) are overridden, so dragging one node never
/// moves the others.
pub fn layout(d: &Diagram) -> Layout {
    let n = d.nodes.len();
    let index: HashMap<&str, usize> = d
        .nodes
        .iter()
        .enumerate()
        .map(|(i, node)| (node.id.as_str(), i))
        .collect();
    let ends: Vec<Option<(usize, usize)>> = d
        .edges
        .iter()
        .map(|e| Some((*index.get(e.from.as_str())?, *index.get(e.to.as_str())?)))
        .collect();
    let back = back_edges(n, &ends);
    // Forward edges only: cycles broken, self-loops dropped.
    let forward: Vec<(usize, usize)> = ends
        .iter()
        .zip(&back)
        .filter_map(|(e, back)| e.filter(|(a, b)| !back && a != b))
        .collect();
    let ranks = longest_path_ranks(n, &forward);

    let mut nodes = place(d, &ranks, &forward);
    for (rect, node) in nodes.iter_mut().zip(&d.nodes) {
        if let (Some(x), Some(y)) = (node.x, node.y) {
            rect.x = x;
            rect.y = y;
        }
    }

    let groups: Vec<Option<Rect>> = d
        .groups
        .iter()
        .map(|g| {
            let members = d
                .nodes
                .iter()
                .zip(&nodes)
                .filter(|(node, _)| node.group.as_deref() == Some(g.id.as_str()))
                .map(|(_, r)| *r);
            let union = members.reduce(|a, b| a.union(&b))?;
            let mut r = union.inflate(GROUP_PAD);
            r.y -= GROUP_HEADER;
            r.h += GROUP_HEADER;
            Some(r)
        })
        .collect();

    let edges: Vec<EdgePath> = ends
        .iter()
        .map(|e| match e {
            Some((a, b)) if a == b => self_loop(&nodes[*a]),
            Some((a, b)) => connect(&nodes[*a], &nodes[*b]),
            // Only reachable for hand-built diagrams; parse rejects dangling edges.
            None => {
                let p = Point::default();
                EdgePath {
                    from: p,
                    c1: p,
                    c2: p,
                    to: p,
                }
            }
        })
        .collect();

    let bounds = nodes
        .iter()
        .copied()
        .chain(groups.iter().flatten().copied())
        .chain(edges.iter().map(EdgePath::bounds))
        .reduce(|a, b| a.union(&b))
        .unwrap_or_default();

    Layout {
        nodes,
        ranks,
        groups,
        edges,
        bounds,
    }
}

/// Edges a DFS (in spec order) finds pointing back into the current path.
/// Ignoring them leaves a DAG. Self-loops are not marked; callers drop them.
fn back_edges(n: usize, ends: &[Option<(usize, usize)>]) -> Vec<bool> {
    let mut out: Vec<Vec<(usize, usize)>> = vec![Vec::new(); n];
    for (i, e) in ends.iter().enumerate() {
        if let Some((a, b)) = *e
            && a != b
        {
            out[a].push((i, b));
        }
    }
    #[derive(Clone, Copy, PartialEq)]
    enum State {
        New,
        Open,
        Done,
    }
    let mut state = vec![State::New; n];
    let mut back = vec![false; ends.len()];
    let mut stack: Vec<(usize, usize)> = Vec::new();
    for root in 0..n {
        if state[root] != State::New {
            continue;
        }
        state[root] = State::Open;
        stack.push((root, 0));
        while let Some((v, next)) = stack.last_mut() {
            let v = *v;
            if let Some(&(edge, w)) = out[v].get(*next) {
                *next += 1;
                match state[w] {
                    State::New => {
                        state[w] = State::Open;
                        stack.push((w, 0));
                    }
                    State::Open => back[edge] = true,
                    State::Done => {}
                }
            } else {
                state[v] = State::Done;
                stack.pop();
            }
        }
    }
    back
}

fn longest_path_ranks(n: usize, forward: &[(usize, usize)]) -> Vec<usize> {
    let mut out: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut indegree = vec![0usize; n];
    for &(a, b) in forward {
        out[a].push(b);
        indegree[b] += 1;
    }
    let mut ranks = vec![0; n];
    let mut queue: Vec<usize> = (0..n).filter(|&v| indegree[v] == 0).collect();
    let mut head = 0;
    while let Some(&v) = queue.get(head) {
        head += 1;
        for &w in &out[v] {
            ranks[w] = ranks[w].max(ranks[v] + 1);
            indegree[w] -= 1;
            if indegree[w] == 0 {
                queue.push(w);
            }
        }
    }
    ranks
}

/// Auto positions for every node: rank along the main axis, group band and
/// barycenter order along the cross axis.
fn place(d: &Diagram, ranks: &[usize], forward: &[(usize, usize)]) -> Vec<Rect> {
    let n = d.nodes.len();
    if n == 0 {
        return Vec::new();
    }
    let lr = d.direction == Direction::LeftRight;
    let rank_count = ranks.iter().max().map_or(0, |r| r + 1);

    // Bands: ungrouped nodes first (clients and external systems usually sit
    // outside groups and feed into the first one), then groups in spec order.
    let group_index: HashMap<&str, usize> = d
        .groups
        .iter()
        .enumerate()
        .map(|(i, g)| (g.id.as_str(), i))
        .collect();
    let node_group: Vec<Option<usize>> = d
        .nodes
        .iter()
        .map(|node| group_index.get(node.group.as_deref()?).copied())
        .collect();
    let mut band_grouped: Vec<bool> = Vec::new();
    let ungrouped_band = 0;
    if node_group.contains(&None) {
        band_grouped.push(false);
    }
    let mut band_of_group = vec![None; d.groups.len()];
    for (g, band) in band_of_group.iter_mut().enumerate() {
        if node_group.contains(&Some(g)) {
            *band = Some(band_grouped.len());
            band_grouped.push(true);
        }
    }
    let band: Vec<usize> = node_group
        .iter()
        .map(|g| g.and_then(|g| band_of_group[g]).unwrap_or(ungrouped_band))
        .collect();

    // cells[band][rank]: nodes in cross-axis order, initially spec order.
    let mut cells = vec![vec![Vec::new(); rank_count]; band_grouped.len()];
    for v in 0..n {
        cells[band[v]][ranks[v]].push(v);
    }

    // Cross-axis geometry is fixed by band sizes; only the order inside cells changes.
    let node_cross = if lr { NODE_H } else { NODE_W };
    let pitch = node_cross + NODE_GAP;
    let (lead, trail) = if lr {
        (GROUP_HEADER + GROUP_PAD, GROUP_PAD)
    } else {
        (GROUP_PAD, GROUP_PAD)
    };
    let mut band_size = Vec::with_capacity(cells.len());
    let mut content_start = Vec::with_capacity(cells.len());
    let mut cursor = 0.;
    for (b, row) in cells.iter().enumerate() {
        let size = row.iter().map(Vec::len).max().unwrap_or(0);
        let (lead, trail) = if band_grouped[b] {
            (lead, trail)
        } else {
            (0., 0.)
        };
        let content = size as f32 * pitch - NODE_GAP;
        band_size.push(size);
        content_start.push(cursor + lead);
        cursor += lead + content + trail + GROUP_GAP;
    }

    let mut cross = vec![0f32; n];
    let set_cross = |cross: &mut [f32], cell: &[usize], b: usize| {
        let offset = (band_size[b] - cell.len()) as f32 / 2.;
        for (i, &v) in cell.iter().enumerate() {
            cross[v] = content_start[b] + (offset + i as f32) * pitch;
        }
    };
    for (b, row) in cells.iter().enumerate() {
        for cell in row {
            set_cross(&mut cross, cell, b);
        }
    }

    let mut neighbors: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &(a, b) in forward {
        neighbors[a].push(b);
        neighbors[b].push(a);
    }
    let sweep = |cells: &mut [Vec<Vec<usize>>], cross: &mut [f32], r: usize, down: bool| {
        for (b, row) in cells.iter_mut().enumerate() {
            let cell = &mut row[r];
            if cell.len() < 2 {
                continue;
            }
            let bary = |v: usize| {
                let (sum, count) = neighbors[v]
                    .iter()
                    .filter(|&&w| if down { ranks[w] < r } else { ranks[w] > r })
                    .fold((0., 0usize), |(s, c), &w| (s + cross[w], c + 1));
                if count == 0 {
                    cross[v]
                } else {
                    sum / count as f32
                }
            };
            let mut keyed: Vec<(f32, usize)> = cell.iter().map(|&v| (bary(v), v)).collect();
            keyed.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            *cell = keyed.into_iter().map(|(_, v)| v).collect();
            set_cross(cross, cell, b);
        }
    };
    for _ in 0..ORDER_SWEEPS {
        for r in 1..rank_count {
            sweep(&mut cells, &mut cross, r, true);
        }
        for r in (0..rank_count.saturating_sub(1)).rev() {
            sweep(&mut cells, &mut cross, r, false);
        }
    }

    (0..n)
        .map(|v| {
            if lr {
                Rect {
                    x: ranks[v] as f32 * (NODE_W + RANK_GAP),
                    y: cross[v],
                    w: NODE_W,
                    h: NODE_H,
                }
            } else {
                Rect {
                    x: cross[v],
                    y: ranks[v] as f32 * (NODE_H + RANK_GAP),
                    w: NODE_W,
                    h: NODE_H,
                }
            }
        })
        .collect()
}

/// Leaves and enters through the sides facing each other, with control points
/// pushed along the side normals so the curve meets the boxes square.
fn connect(a: &Rect, b: &Rect) -> EdgePath {
    let (ca, cb) = (a.center(), b.center());
    let (dx, dy) = (cb.x - ca.x, cb.y - ca.y);
    let (from, to, normal) = if dx.abs() >= dy.abs() {
        let sign = if dx >= 0. { 1. } else { -1. };
        let from_x = if sign > 0. { a.right() } else { a.x };
        let to_x = if sign > 0. { b.x } else { b.right() };
        (
            Point { x: from_x, y: ca.y },
            Point { x: to_x, y: cb.y },
            Point { x: sign, y: 0. },
        )
    } else {
        let sign = if dy >= 0. { 1. } else { -1. };
        let from_y = if sign > 0. { a.bottom() } else { a.y };
        let to_y = if sign > 0. { b.y } else { b.bottom() };
        (
            Point { x: ca.x, y: from_y },
            Point { x: cb.x, y: to_y },
            Point { x: 0., y: sign },
        )
    };
    let push = ((to.x - from.x).hypot(to.y - from.y) * 0.4).max(30.);
    EdgePath {
        from,
        c1: Point {
            x: from.x + normal.x * push,
            y: from.y + normal.y * push,
        },
        c2: Point {
            x: to.x - normal.x * push,
            y: to.y - normal.y * push,
        },
        to,
    }
}

fn self_loop(r: &Rect) -> EdgePath {
    let c = r.center();
    let from = Point {
        x: r.right(),
        y: c.y - r.h / 4.,
    };
    let to = Point {
        x: r.right(),
        y: c.y + r.h / 4.,
    };
    EdgePath {
        from,
        c1: Point {
            x: from.x + 48.,
            y: from.y - 20.,
        },
        c2: Point {
            x: to.x + 48.,
            y: to.y + 20.,
        },
        to,
    }
}

fn segment_distance(p: Point, a: Point, b: Point) -> f32 {
    let (vx, vy) = (b.x - a.x, b.y - a.y);
    let len2 = vx * vx + vy * vy;
    let t = if len2 == 0. {
        0.
    } else {
        (((p.x - a.x) * vx + (p.y - a.y) * vy) / len2).clamp(0., 1.)
    };
    (p.x - a.x - t * vx).hypot(p.y - a.y - t * vy)
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    fn diagram(spec: Value) -> Diagram {
        Diagram::parse(&spec).expect("valid spec")
    }

    fn chain_with_groups(direction: &str) -> Diagram {
        diagram(json!({
            "direction": direction,
            "groups": [{"id": "front"}, {"id": "back"}, {"id": "data"}],
            "nodes": [
                {"id": "web", "group": "front"}, {"id": "mobile", "group": "front"},
                {"id": "api", "group": "back"}, {"id": "worker", "group": "back"},
                {"id": "auth", "group": "back"}, {"id": "db", "group": "data"},
                {"id": "cache", "group": "data"}, {"id": "mail"}, {"id": "logs"},
            ],
            "edges": [
                {"from": "web", "to": "api"}, {"from": "mobile", "to": "api"},
                {"from": "api", "to": "auth"}, {"from": "api", "to": "db"},
                {"from": "api", "to": "cache"}, {"from": "worker", "to": "db"},
                {"from": "worker", "to": "mail"}, {"from": "api", "to": "logs"},
                {"from": "db", "to": "worker"},
            ],
        }))
    }

    fn cyclic() -> Diagram {
        diagram(json!({
            "nodes": [{"id": "a"}, {"id": "b"}, {"id": "c"}, {"id": "d"}, {"id": "e"}],
            "edges": [
                {"from": "a", "to": "b"}, {"from": "b", "to": "c"}, {"from": "c", "to": "a"},
                {"from": "c", "to": "d"}, {"from": "d", "to": "d"}, {"from": "d", "to": "b"},
                {"from": "a", "to": "e"}, {"from": "e", "to": "e"},
            ],
        }))
    }

    fn wide() -> Diagram {
        let nodes: Vec<Value> = (0..200)
            .map(|i| json!({"id": i, "group": format!("g{}", i % 5)}))
            .collect();
        let edges: Vec<Value> = (0..400)
            .map(|i| json!({"from": (i * 7) % 200, "to": (i * 13 + 1) % 200}))
            .collect();
        let groups: Vec<Value> = (0..5).map(|i| json!({"id": format!("g{i}")})).collect();
        diagram(json!({"groups": groups, "nodes": nodes, "edges": edges}))
    }

    fn shapes() -> Vec<Diagram> {
        vec![
            chain_with_groups("LR"),
            chain_with_groups("TB"),
            cyclic(),
            wide(),
        ]
    }

    fn ends(d: &Diagram) -> Vec<Option<(usize, usize)>> {
        d.edges
            .iter()
            .map(|e| Some((d.node_index(&e.from)?, d.node_index(&e.to)?)))
            .collect()
    }

    #[test]
    fn nodes_never_overlap() {
        for d in shapes() {
            let l = layout(&d);
            for i in 0..l.nodes.len() {
                for j in i + 1..l.nodes.len() {
                    assert!(!l.nodes[i].intersects(&l.nodes[j]), "{i} overlaps {j}");
                }
            }
        }
    }

    #[test]
    fn forward_edges_go_to_higher_ranks() {
        for d in shapes() {
            let l = layout(&d);
            let ends = ends(&d);
            let back = back_edges(d.nodes.len(), &ends);
            for (e, is_back) in ends.iter().zip(back) {
                let (a, b) = e.unwrap();
                if a != b && !is_back {
                    assert!(l.ranks[b] > l.ranks[a]);
                }
            }
        }
    }

    #[test]
    fn groups_contain_members_and_never_overlap() {
        for d in shapes() {
            let l = layout(&d);
            for (node, rect) in d.nodes.iter().zip(&l.nodes) {
                if let Some(g) = &node.group {
                    let g = l.groups[d.groups.iter().position(|x| &x.id == g).unwrap()].unwrap();
                    assert!(g.contains(Point {
                        x: rect.x,
                        y: rect.y
                    }));
                    assert!(g.contains(Point {
                        x: rect.right(),
                        y: rect.bottom()
                    }));
                    assert!(rect.y - g.y >= GROUP_HEADER);
                }
            }
            let rects: Vec<Rect> = l.groups.iter().flatten().copied().collect();
            for i in 0..rects.len() {
                for j in i + 1..rects.len() {
                    assert!(!rects[i].intersects(&rects[j]));
                }
                for (node, rect) in d.nodes.iter().zip(&l.nodes) {
                    if node.group.is_none() {
                        assert!(!rects[i].intersects(rect));
                    }
                }
            }
        }
    }

    #[test]
    fn ungrouped_band_comes_before_groups() {
        for (direction, lr) in [("LR", true), ("TB", false)] {
            let d = chain_with_groups(direction);
            let l = layout(&d);
            let cross = |r: &Rect| {
                if lr {
                    (r.y, r.bottom())
                } else {
                    (r.x, r.right())
                }
            };
            let first_group = l
                .groups
                .iter()
                .flatten()
                .map(|g| cross(g).0)
                .fold(f32::INFINITY, f32::min);
            for (node, rect) in d.nodes.iter().zip(&l.nodes) {
                if node.group.is_none() {
                    assert!(cross(rect).1 <= first_group);
                }
            }
        }
    }

    #[test]
    fn pinned_nodes_keep_their_place_and_others_stay_put() {
        let mut d = chain_with_groups("LR");
        let before = layout(&d);
        d.nodes[2].x = Some(-500.);
        d.nodes[2].y = Some(900.);
        let after = layout(&d);
        assert_eq!(
            after.nodes[2],
            Rect {
                x: -500.,
                y: 900.,
                w: NODE_W,
                h: NODE_H
            }
        );
        for i in (0..d.nodes.len()).filter(|&i| i != 2) {
            assert_eq!(before.nodes[i], after.nodes[i]);
        }
        let back = after.groups[1].unwrap();
        assert!(back.contains(Point { x: -500., y: 900. }));
        assert!(after.bounds.contains(Point {
            x: -500.,
            y: 900. + NODE_H
        }));
    }

    #[test]
    fn empty_diagram() {
        let l = layout(&Diagram::default());
        assert!(l.nodes.is_empty() && l.edges.is_empty());
        assert_eq!(l.bounds, Rect::default());
    }

    #[test]
    fn edge_endpoints_lie_on_box_borders() {
        let on_border = |r: &Rect, p: Point| {
            const EPS: f32 = 0.01;
            r.inflate(EPS).contains(p)
                && [p.x - r.x, r.right() - p.x, p.y - r.y, r.bottom() - p.y]
                    .iter()
                    .any(|d| d.abs() < EPS)
        };
        for d in shapes() {
            let l = layout(&d);
            for (edge, path) in d.edges.iter().zip(&l.edges) {
                let a = &l.nodes[d.node_index(&edge.from).unwrap()];
                let b = &l.nodes[d.node_index(&edge.to).unwrap()];
                assert!(on_border(a, path.from) && on_border(b, path.to));
                assert!(l.bounds.contains(path.label_at()));
            }
        }
    }

    #[test]
    fn hit_testing() {
        let d = diagram(
            json!({"nodes": [{"id": "a"}, {"id": "b"}], "edges": [{"from": "a", "to": "b"}]}),
        );
        let l = layout(&d);
        assert_eq!(l.node_at(l.nodes[1].center()), Some(1));
        assert_eq!(l.node_at(Point { x: -100., y: -100. }), None);
        assert_eq!(l.edge_at(l.edges[0].label_at(), 4.), Some(0));
        let dir = l.edges[0].end_direction();
        assert!((dir.x - 1.).abs() < 1e-3 && dir.y.abs() < 1e-3);
    }
}
