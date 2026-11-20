//! Layered auto-layout for diagrams. It runs on every edit, so it is a
//! deterministic Sugiyama-style pass kept cheap: break cycles (DFS back edges
//! run reversed), rank by longest path, give each group its own band across
//! ranks (ungrouped first, then groups chained by how much they link), add
//! dummy vertices where an edge skips ranks, and order every (band, rank) cell
//! with barycenter sweeps plus adjacent transpositions, keeping the ordering
//! with the fewest crossings between adjacent ranks. Edges then run
//! orthogonally with rounded corners: straight through their dummies, across
//! in a channel of each rank gap, every edge turning in a gap getting its own.

use std::collections::HashMap;

use super::diagram::{Diagram, Direction, EdgeStyle};

/// A diagram node's box: room for the title, a two-line description and the
/// kind · tech line.
pub const NODE_W: f32 = 208.;
pub const NODE_H: f32 = 88.;
/// Wide enough between ranks for a short verb-phrase edge label; gaps widen
/// for longer ones.
pub const RANK_GAP: f32 = 120.;
pub const NODE_GAP: f32 = 32.;
pub const GROUP_PAD: f32 = 20.;
pub const GROUP_HEADER: f32 = 26.;
pub const GROUP_GAP: f32 = 40.;

/// The node box and the gap between ranks a surface lays out with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sizing {
    pub node_w: f32,
    pub node_h: f32,
    pub rank_gap: f32,
}

/// Architecture diagrams.
pub const DIAGRAM: Sizing = Sizing {
    node_w: NODE_W,
    node_h: NODE_H,
    rank_gap: RANK_GAP,
};

/// State machines: compact cards that hold an icon badge, a name and one line.
pub const STATES: Sizing = Sizing {
    node_w: 200.,
    node_h: 64.,
    rank_gap: 96.,
};

const ORDER_SWEEPS: usize = 4;
const TRANSPOSE_PASSES: usize = 4;
/// Cross-axis room a dummy takes in its cell, besides the usual gap.
const DUMMY_SIZE: f32 = 8.;
/// Corner radius of orthogonal edges, shrunk where a run is too short for it.
pub const CORNER: f32 = 10.;
/// Control-point distance, as a share of the radius, of the cubic closest to
/// a quarter circle.
const KAPPA: f32 = 0.5523;
/// Ends nearer than this across the main axis count as aligned: the edge runs
/// straight instead of jogging by a sliver.
pub(crate) const ALIGNED: f32 = 1.;
/// Room a channel keeps from the group boxes reaching into a rank gap.
const CHANNEL_INSET: f32 = 8.;
/// Where channels may start in a rank gap, from the side edges leave.
const CHANNEL_LEAD: f32 = GROUP_PAD + CHANNEL_INSET;
/// How far a self-loop reaches out of its box, and how wide it is.
const LOOP_REACH: f32 = 24.;
const LOOP_SPAN: f32 = 36.;
/// Spacing of the ports edges leave or enter one side through: a label's
/// height, so labels on neighboring runs can sit side by side.
const PORT_GAP: f32 = 24.;
/// Ports stay this far from a side's ends, clear of the card's corner.
const PORT_INSET: f32 = 14.;
/// Edge label pills, estimated in diagram units: 10.5 px text runs about 6.2
/// per character, plus the pill's padding; past the widest a label wraps
/// onto a second line.
const LABEL_CHAR: f32 = 6.2;
const LABEL_PADDING: f32 = 28.;
pub const LABEL_MAX_W: f32 = 240.;
const LABEL_H: f32 = 22.;
const LABEL_LINE: f32 = 14.;
/// A caption line under a label (a diagram edge's style).
const LABEL_CAPTION: f32 = 12.;
/// Clearance a label keeps from the ends of its run.
const LABEL_PAD: f32 = 6.;
/// How far a label slides along its run per try when it collides.
const LABEL_STEP: f32 = 8.;

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

/// One cubic Bézier piece of an edge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cubic {
    pub from: Point,
    pub c1: Point,
    pub c2: Point,
    pub to: Point,
}

impl Cubic {
    pub fn point_at(&self, t: f32) -> Point {
        let u = 1. - t;
        let (a, b, c, d) = (u * u * u, 3. * u * u * t, 3. * u * t * t, t * t * t);
        Point {
            x: a * self.from.x + b * self.c1.x + c * self.c2.x + d * self.to.x,
            y: a * self.from.y + b * self.c1.y + c * self.c2.y + d * self.to.y,
        }
    }

    fn reversed(&self) -> Cubic {
        Cubic {
            from: self.to,
            c1: self.c2,
            c2: self.c1,
            to: self.from,
        }
    }

    /// A straight run: both control points on the line from `from` to `to`.
    fn is_straight(&self) -> bool {
        let (dx, dy) = (self.to.x - self.from.x, self.to.y - self.from.y);
        let len = dx.hypot(dy);
        len > 1e-3
            && [self.c1, self.c2]
                .iter()
                .all(|c| ((c.x - self.from.x) * dy - (c.y - self.from.y) * dx).abs() / len < 1e-2)
    }
}

/// Tangent-continuous chain of cubics from the source box's border to the
/// target box's border. Never empty.
#[derive(Debug, Clone, PartialEq)]
pub struct EdgePath {
    pub segments: Vec<Cubic>,
}

impl EdgePath {
    pub fn start(&self) -> Point {
        self.segments[0].from
    }

    pub fn end(&self) -> Point {
        self.segments[self.segments.len() - 1].to
    }

    /// `t` in 0..=1 over the whole chain, each segment taking an equal share.
    pub fn point_at(&self, t: f32) -> Point {
        let count = self.segments.len();
        let s = t.clamp(0., 1.) * count as f32;
        let i = (s as usize).min(count - 1);
        self.segments[i].point_at(s - i as f32)
    }

    /// The middle of the longest straight run, so a label never sits on a
    /// corner; on a tie, a run along the first one (the main axis) wins.
    /// Paths without straight runs fall back to their middle.
    pub fn label_at(&self) -> Point {
        const TIE: f32 = 0.5;
        let horizontal = |s: &Cubic| (s.to.x - s.from.x).abs() >= (s.to.y - s.from.y).abs();
        let main = horizontal(&self.segments[0]);
        let mut best: Option<(f32, bool, &Cubic)> = None;
        for s in self.segments.iter().filter(|s| s.is_straight()) {
            let len = (s.to.x - s.from.x).hypot(s.to.y - s.from.y);
            let along = horizontal(s) == main;
            let wins = best.is_none_or(|(most, most_along, _)| {
                len > most + TIE || (len > most - TIE && along && !most_along)
            });
            if wins {
                best = Some((len, along, s));
            }
        }
        match best {
            Some((_, _, s)) => Point {
                x: (s.from.x + s.to.x) / 2.,
                y: (s.from.y + s.to.y) / 2.,
            },
            None => self.point_at(0.5),
        }
    }

    /// Sampled over 24 pieces per segment: plenty for hit-testing.
    pub fn distance_to(&self, p: Point) -> f32 {
        let mut best = f32::INFINITY;
        self.polyline(24, |a, b| best = best.min(segment_distance(p, a, b)));
        best
    }

    /// Unit vector of travel at `end()`, for the arrowhead.
    pub fn end_direction(&self) -> Point {
        // The tangent at the end is to - c2; fall back when control points coincide with the end.
        let to = self.end();
        self.segments
            .iter()
            .rev()
            .flat_map(|s| [s.c2, s.c1, s.from])
            .map(|p| Point {
                x: to.x - p.x,
                y: to.y - p.y,
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

    /// The box around all control points, which contains the curve.
    pub fn bounds(&self) -> Rect {
        self.segments
            .iter()
            .flat_map(|s| [s.from, s.c1, s.c2, s.to])
            .map(|p| Rect {
                x: p.x,
                y: p.y,
                w: 0.,
                h: 0.,
            })
            .reduce(|a, b| a.union(&b))
            .expect("an edge path has segments")
    }

    /// Calls `f` for each straight piece of the path sampled with `per_segment`
    /// pieces per cubic.
    fn polyline(&self, per_segment: usize, mut f: impl FnMut(Point, Point)) {
        let mut prev = self.start();
        for s in &self.segments {
            for i in 1..=per_segment {
                let next = s.point_at(i as f32 / per_segment as f32);
                f(prev, next);
                prev = next;
            }
        }
    }

    fn reversed(&self) -> EdgePath {
        EdgePath {
            segments: self.segments.iter().rev().map(Cubic::reversed).collect(),
        }
    }
}

/// `nodes`/`ranks` are index-aligned with `diagram.nodes`, `groups` with
/// `diagram.groups` (None: no members), `edges` and `labels` with
/// `diagram.edges` (a label's center, None for an edge without one).
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub nodes: Vec<Rect>,
    pub ranks: Vec<usize>,
    pub groups: Vec<Option<Rect>>,
    pub edges: Vec<EdgePath>,
    pub labels: Vec<Option<Point>>,
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
    layout_with(d, DIAGRAM)
}

/// [`layout`] with another node box and rank gap.
pub fn layout_with(d: &Diagram, sizing: Sizing) -> Layout {
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
    // Every edge but self-loops as (lower rank, higher rank): back edges run
    // reversed so they take part in ordering and routing too.
    let links: Vec<Option<(usize, usize)>> = ends
        .iter()
        .zip(&back)
        .map(|(e, &back)| {
            let (a, b) = (*e)?;
            (a != b).then_some(if back { (b, a) } else { (a, b) })
        })
        .collect();

    let lr = d.direction == Direction::LeftRight;
    let sizes: Vec<Option<(f32, f32)>> = d
        .edges
        .iter()
        .map(|e| {
            let text = e.label.as_deref().filter(|l| !l.is_empty())?;
            Some(label_size(text, e.style != EdgeStyle::Sync))
        })
        .collect();
    let rank_count = ranks.iter().max().map_or(0, |r| r + 1);
    let gaps = rank_gaps(rank_count, &ranks, &links, &sizes, lr, sizing);
    let main_size = if lr { sizing.node_w } else { sizing.node_h };
    let starts: Vec<f32> = gaps
        .iter()
        .scan(0., |cursor: &mut f32, gap| {
            let start = *cursor;
            *cursor += main_size + gap;
            Some(start)
        })
        .collect();
    let (mut nodes, waypoints) = place(d, &ranks, &links, sizing, &starts);
    let pinned: Vec<bool> = d
        .nodes
        .iter()
        .map(|node| node.x.is_some() && node.y.is_some())
        .collect();
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

    // Links between auto-placed nodes route through their dummies; edges
    // touching a pinned node connect directly.
    let routed: Vec<Option<(usize, usize)>> = links
        .iter()
        .map(|link| link.filter(|(lo, hi)| !pinned[*lo] && !pinned[*hi]))
        .collect();
    let paths = routes(lr, &nodes, &ranks, &routed, &waypoints, sizing, &starts);
    let edges: Vec<EdgePath> = ends
        .iter()
        .zip(&routed)
        .zip(paths)
        .map(|((e, link), path)| match (e, path) {
            (Some((a, b)), _) if a == b => self_loop(&nodes[*a], lr),
            // Back edges were routed from their lower-rank end.
            (Some((a, _)), Some(path)) => match link {
                Some((lo, _)) if lo == a => path,
                _ => path.reversed(),
            },
            (Some((a, b)), None) => connect(&nodes[*a], &nodes[*b]),
            // Only reachable for hand-built diagrams; parse rejects dangling edges.
            (None, _) => {
                let p = Point::default();
                EdgePath {
                    segments: vec![Cubic {
                        from: p,
                        c1: p,
                        c2: p,
                        to: p,
                    }],
                }
            }
        })
        .collect();

    let labels = place_labels(&edges, &sizes, &nodes, lr);

    let bounds = nodes
        .iter()
        .copied()
        .chain(groups.iter().flatten().copied())
        .chain(edges.iter().map(EdgePath::bounds))
        .chain(labels.iter().zip(&sizes).filter_map(|(p, size)| {
            let (p, (w, h)) = ((*p)?, (*size)?);
            Some(label_box(p, w, h))
        }))
        .reduce(|a, b| a.union(&b))
        .unwrap_or_default();

    Layout {
        nodes,
        ranks,
        groups,
        edges,
        labels,
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

/// Auto positions for every node, plus the cross-axis centers of each link's
/// dummies in rank order (empty for links spanning one rank). Ranks run along
/// the main axis from `starts`; across it each group gets a band, and the
/// order inside each (band, rank) cell is chosen to reduce crossings.
fn place(
    d: &Diagram,
    ranks: &[usize],
    links: &[Option<(usize, usize)>],
    sizing: Sizing,
    starts: &[f32],
) -> (Vec<Rect>, Vec<Vec<f32>>) {
    let n = d.nodes.len();
    if n == 0 {
        return (Vec::new(), vec![Vec::new(); links.len()]);
    }
    let lr = d.direction == Direction::LeftRight;
    let rank_count = ranks.iter().max().map_or(0, |r| r + 1);

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
    let (mut band_of, band_grouped) = bands(d.groups.len(), &node_group, links);

    // Dummy vertices (indices from n) carry each link through the ranks it
    // skips, in the band of its higher-rank end: switching bands right away
    // crossed fewer edges in practice than at the far end or midway.
    let mut rank_of = ranks.to_vec();
    let mut up: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut down: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut dummies = Vec::with_capacity(links.len());
    for link in links {
        let start = rank_of.len();
        if let Some((lo, hi)) = *link {
            let mut prev = lo;
            for r in ranks[lo] + 1..ranks[hi] {
                let v = rank_of.len();
                rank_of.push(r);
                band_of.push(band_of[hi]);
                up.push(vec![prev]);
                down.push(Vec::new());
                down[prev].push(v);
                prev = v;
            }
            down[prev].push(hi);
            up[hi].push(prev);
        }
        dummies.push(start..rank_of.len());
    }

    let node_cross = if lr { sizing.node_h } else { sizing.node_w };
    let size: Vec<f32> = (0..rank_of.len())
        .map(|v| if v < n { node_cross } else { DUMMY_SIZE })
        .collect();
    // cells[band][rank]: vertices in cross-axis order, initially index order.
    let mut cells = vec![vec![Vec::new(); rank_count]; band_grouped.len()];
    for (v, (&b, &r)) in band_of.iter().zip(&rank_of).enumerate() {
        cells[b][r].push(v);
    }

    // Cross-axis geometry is fixed by band sizes; only the order inside cells changes.
    let (lead, trail) = if lr {
        (GROUP_HEADER + GROUP_PAD, GROUP_PAD)
    } else {
        (GROUP_PAD, GROUP_PAD)
    };
    let mut content = Vec::with_capacity(cells.len());
    let mut content_start = Vec::with_capacity(cells.len());
    let mut cursor = 0.;
    for (row, &grouped) in cells.iter().zip(&band_grouped) {
        let extent = row
            .iter()
            .map(|cell| extent(cell, &size))
            .fold(0., f32::max);
        let (lead, trail) = if grouped { (lead, trail) } else { (0., 0.) };
        content.push(extent);
        content_start.push(cursor + lead);
        cursor += lead + extent + trail + GROUP_GAP;
    }

    let mut layers = Layers {
        cross: vec![0.; size.len()],
        cells,
        up,
        down,
        size,
        content,
        content_start,
    };
    layers.order();

    let nodes = (0..n)
        .map(|v| {
            let main = starts[ranks[v]];
            let cross = layers.cross[v] - node_cross / 2.;
            if lr {
                Rect {
                    x: main,
                    y: cross,
                    w: sizing.node_w,
                    h: sizing.node_h,
                }
            } else {
                Rect {
                    x: cross,
                    y: main,
                    w: sizing.node_w,
                    h: sizing.node_h,
                }
            }
        })
        .collect();
    let waypoints = dummies
        .into_iter()
        .map(|range| layers.cross[range].to_vec())
        .collect();
    (nodes, waypoints)
}

/// Band per node and whether each band is a group. Ungrouped nodes come first
/// (clients and external systems usually sit outside groups and feed into the
/// first one); groups then chain greedily, each next band being the one most
/// linked to the bands just placed (nearer ones weigh more), so connected
/// groups sit side by side. Ties go to the most connected group, then spec order.
fn bands(
    group_count: usize,
    node_group: &[Option<usize>],
    links: &[Option<(usize, usize)>],
) -> (Vec<usize>, Vec<bool>) {
    // Slot `group_count` stands for the ungrouped nodes.
    let k = group_count + 1;
    let slot = |v: usize| node_group[v].unwrap_or(group_count);
    let mut weight = vec![0u32; k * k];
    for &(a, b) in links.iter().flatten() {
        let (sa, sb) = (slot(a), slot(b));
        if sa != sb {
            weight[sa * k + sb] += 1;
            weight[sb * k + sa] += 1;
        }
    }
    let total = |s: usize| weight[s * k..(s + 1) * k].iter().sum::<u32>();

    let mut chain: Vec<usize> = Vec::with_capacity(k);
    if node_group.contains(&None) {
        chain.push(group_count);
    }
    let mut left: Vec<usize> = (0..group_count)
        .filter(|g| node_group.contains(&Some(*g)))
        .collect();
    while !left.is_empty() {
        let pull = |s: usize| {
            chain
                .iter()
                .rev()
                .enumerate()
                .map(|(dist, &placed)| weight[s * k + placed] as f32 / (dist + 1) as f32)
                .sum::<f32>()
        };
        let best = (0..left.len())
            .max_by(|&i, &j| {
                let (g, h) = (left[i], left[j]);
                pull(g)
                    .total_cmp(&pull(h))
                    .then(total(g).cmp(&total(h)))
                    .then(h.cmp(&g))
            })
            .expect("left is not empty");
        chain.push(left.remove(best));
    }

    let mut band_of_slot = vec![0; k];
    for (band, &s) in chain.iter().enumerate() {
        band_of_slot[s] = band;
    }
    let band = (0..node_group.len())
        .map(|v| band_of_slot[slot(v)])
        .collect();
    let grouped = chain.iter().map(|&s| s != group_count).collect();
    (band, grouped)
}

/// Cross-axis length of a cell's vertices laid end to end.
fn extent(cell: &[usize], size: &[f32]) -> f32 {
    cell.iter().map(|&v| size[v] + NODE_GAP).sum::<f32>() - NODE_GAP
}

/// Vertices (nodes, then dummies) per (band, rank) cell, their links to the
/// adjacent ranks and their cross-axis centers.
struct Layers {
    /// cells[band][rank], in cross-axis order.
    cells: Vec<Vec<Vec<usize>>>,
    up: Vec<Vec<usize>>,
    down: Vec<Vec<usize>>,
    size: Vec<f32>,
    content: Vec<f32>,
    content_start: Vec<f32>,
    cross: Vec<f32>,
}

impl Layers {
    /// Alternating barycenter sweeps, each followed by adjacent
    /// transpositions; the ordering with the fewest crossings between
    /// adjacent ranks wins.
    fn order(&mut self) {
        self.set_all_cross();
        let rank_count = self.cells[0].len();
        let mut best = self.crossings();
        let mut best_cells = self.cells.clone();
        for i in 0..2 * ORDER_SWEEPS {
            if best == 0 {
                break;
            }
            if i % 2 == 0 {
                for r in 1..rank_count {
                    self.sweep(r, true);
                }
            } else {
                for r in (0..rank_count - 1).rev() {
                    self.sweep(r, false);
                }
            }
            self.transpose();
            let count = self.crossings();
            if count < best {
                best = count;
                best_cells.clone_from(&self.cells);
            }
        }
        self.cells = best_cells;
        self.set_all_cross();
    }

    fn set_all_cross(&mut self) {
        for b in 0..self.cells.len() {
            for r in 0..self.cells[b].len() {
                self.set_cross(b, r);
            }
        }
    }

    /// Centers the cell in its band and spaces its vertices by size.
    fn set_cross(&mut self, b: usize, r: usize) {
        let cell = &self.cells[b][r];
        let mut cursor = self.content_start[b] + (self.content[b] - extent(cell, &self.size)) / 2.;
        for &v in cell {
            self.cross[v] = cursor + self.size[v] / 2.;
            cursor += self.size[v] + NODE_GAP;
        }
    }

    /// Sorts every cell of rank `r` by the mean position of its neighbors in
    /// the rank above (`down`) or below; vertices without any keep theirs.
    fn sweep(&mut self, r: usize, down: bool) {
        let mut keyed: Vec<(f32, usize)> = Vec::new();
        for b in 0..self.cells.len() {
            if self.cells[b][r].len() < 2 {
                continue;
            }
            let adjacent = if down { &self.up } else { &self.down };
            keyed.clear();
            keyed.extend(self.cells[b][r].iter().map(|&v| {
                let near = &adjacent[v];
                let key = if near.is_empty() {
                    self.cross[v]
                } else {
                    near.iter().map(|&w| self.cross[w]).sum::<f32>() / near.len() as f32
                };
                (key, v)
            }));
            keyed.sort_by(|a, b| a.0.total_cmp(&b.0));
            for (slot, &(_, v)) in self.cells[b][r].iter_mut().zip(&keyed) {
                *slot = v;
            }
            self.set_cross(b, r);
        }
    }

    /// Swaps neighbors within cells while that removes crossings.
    fn transpose(&mut self) {
        for _ in 0..TRANSPOSE_PASSES {
            let mut improved = false;
            for b in 0..self.cells.len() {
                for r in 0..self.cells[b].len() {
                    for i in 1..self.cells[b][r].len() {
                        let (u, v) = (self.cells[b][r][i - 1], self.cells[b][r][i]);
                        let (kept, swapped) = self.pair_crossings(u, v);
                        if swapped < kept {
                            self.cells[b][r].swap(i - 1, i);
                            let start = self.cross[u] - self.size[u] / 2.;
                            self.cross[v] = start + self.size[v] / 2.;
                            self.cross[u] = start + self.size[v] + NODE_GAP + self.size[u] / 2.;
                            improved = true;
                        }
                    }
                }
            }
            if !improved {
                break;
            }
        }
    }

    /// Crossings among the links of `u` and `v`, cell neighbors, with `u`
    /// first and with the two swapped.
    fn pair_crossings(&self, u: usize, v: usize) -> (usize, usize) {
        let mut counts = (0, 0);
        for adjacent in [&self.up, &self.down] {
            for &a in &adjacent[u] {
                for &b in &adjacent[v] {
                    let (ca, cb) = (self.cross[a], self.cross[b]);
                    if ca > cb {
                        counts.0 += 1;
                    } else if ca < cb {
                        counts.1 += 1;
                    }
                }
            }
        }
        counts
    }

    /// Crossings between links of adjacent ranks: inversions among the lower
    /// ends once links are sorted by their upper ends.
    fn crossings(&self) -> usize {
        let rank_count = self.cells[0].len();
        let mut pairs: Vec<(f32, f32)> = Vec::new();
        let mut lower: Vec<f32> = Vec::new();
        let mut scratch: Vec<f32> = Vec::new();
        let mut total = 0;
        for r in 0..rank_count - 1 {
            pairs.clear();
            for row in &self.cells {
                for &u in &row[r] {
                    pairs.extend(self.down[u].iter().map(|&w| (self.cross[u], self.cross[w])));
                }
            }
            pairs.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
            lower.clear();
            lower.extend(pairs.iter().map(|p| p.1));
            total += inversions(&mut lower, &mut scratch);
        }
        total
    }
}

/// Pairs `i < j` with `v[i] > v[j]`, by merge sort (`v` ends up sorted).
fn inversions(v: &mut [f32], scratch: &mut Vec<f32>) -> usize {
    if v.len() < 2 {
        return 0;
    }
    let mid = v.len() / 2;
    let mut count = inversions(&mut v[..mid], scratch) + inversions(&mut v[mid..], scratch);
    scratch.clear();
    let (mut i, mut j) = (0, mid);
    while i < mid && j < v.len() {
        if v[j] < v[i] {
            count += mid - i;
            scratch.push(v[j]);
            j += 1;
        } else {
            scratch.push(v[i]);
            i += 1;
        }
    }
    scratch.extend_from_slice(&v[i..mid]);
    scratch.extend_from_slice(&v[j..]);
    v.copy_from_slice(scratch);
    count
}

/// Orthogonal paths for the links between auto-placed nodes (None for the
/// others): out of a port on the source's far side, straight along the main
/// axis through each dummy, and across in a channel of every rank gap where
/// the cross position changes, into a port on the target's near side. Each
/// link turning in a gap gets its own channel there, so cross runs never
/// overlap. `crosses` are each link's dummies' centers, in rank order;
/// `starts` where each rank begins along the main axis.
fn routes(
    lr: bool,
    nodes: &[Rect],
    ranks: &[usize],
    links: &[Option<(usize, usize)>],
    crosses: &[Vec<f32>],
    sizing: Sizing,
    starts: &[f32],
) -> Vec<Option<EdgePath>> {
    let at = |main: f32, cross: f32| {
        if lr {
            Point { x: main, y: cross }
        } else {
            Point { x: cross, y: main }
        }
    };
    let cross_of = |r: &Rect| {
        let c = r.center();
        if lr { c.y } else { c.x }
    };
    // Ports: the links leaving (or entering) one side spread along it in the
    // order of the cross position they head to (or come from), so they
    // neither share a run nor cross on the way out; a lone link keeps the
    // middle.
    let mut outs: Vec<Vec<(f32, usize)>> = vec![Vec::new(); nodes.len()];
    let mut ins: Vec<Vec<(f32, usize)>> = vec![Vec::new(); nodes.len()];
    for (i, (link, dummies)) in links.iter().zip(crosses).enumerate() {
        let Some((lo, hi)) = *link else { continue };
        let next = dummies
            .first()
            .copied()
            .unwrap_or_else(|| cross_of(&nodes[hi]));
        let prev = dummies
            .last()
            .copied()
            .unwrap_or_else(|| cross_of(&nodes[lo]));
        outs[lo].push((next, i));
        ins[hi].push((prev, i));
    }
    let mut out_port = vec![0.; links.len()];
    let mut in_port = vec![0.; links.len()];
    for (v, rect) in nodes.iter().enumerate() {
        let side = if lr { rect.h } else { rect.w };
        spread_ports(&mut outs[v], side, &mut out_port);
        spread_ports(&mut ins[v], side, &mut in_port);
    }
    // The cross position at every rank a link spans, a step smaller than
    // ALIGNED keeping the previous one so near-aligned ends run straight.
    let steps: Vec<Vec<f32>> = links
        .iter()
        .zip(crosses)
        .enumerate()
        .map(|(i, (link, dummies))| {
            let Some((lo, hi)) = *link else {
                return Vec::new();
            };
            let mut out: Vec<f32> = Vec::with_capacity(dummies.len() + 2);
            let ends = [
                cross_of(&nodes[lo]) + out_port[i],
                cross_of(&nodes[hi]) + in_port[i],
            ];
            for c in ends[..1].iter().chain(dummies).chain(&ends[1..]) {
                let c = match out.last() {
                    Some(&prev) if (c - prev).abs() < ALIGNED => prev,
                    _ => *c,
                };
                out.push(c);
            }
            out
        })
        .collect();

    // The jogs in the gap after each rank: ((link, step), cross in, cross out).
    type Jog = ((usize, usize), f32, f32);
    let rank_count = ranks.iter().max().map_or(0, |r| r + 1);
    let mut gaps: Vec<Vec<Jog>> = vec![Vec::new(); rank_count];
    for (i, (link, cross)) in links.iter().zip(&steps).enumerate() {
        let Some((lo, _)) = *link else { continue };
        for (step, pair) in cross.windows(2).enumerate() {
            if pair[0] != pair[1] {
                gaps[ranks[lo] + step].push(((i, step), pair[0], pair[1]));
            }
        }
    }
    let main_size = if lr { sizing.node_w } else { sizing.node_h };
    let mut channel: Vec<Vec<f32>> = steps
        .iter()
        .map(|cross| vec![0.; cross.len().saturating_sub(1)])
        .collect();
    for (g, jogs) in gaps.into_iter().enumerate() {
        // The channels keep to the base gap's zone at the leaving side, so a
        // gap widened for labels leaves the room on the arriving runs.
        let start = starts[g] + main_size;
        let lo = start + CHANNEL_LEAD;
        let hi = (start + sizing.rank_gap - channel_trail(lr)).max(lo);
        for ((i, step), main) in channels(jogs, lo, hi) {
            channel[i][step] = main;
        }
    }

    links
        .iter()
        .zip(&steps)
        .zip(&channel)
        .map(|((link, cross), channel)| {
            let (lo, hi) = (*link)?;
            let (from, to) = (&nodes[lo], &nodes[hi]);
            let (start, end) = if lr {
                (from.right(), to.x)
            } else {
                (from.bottom(), to.y)
            };
            let mut points = Vec::with_capacity(2 * cross.len());
            points.push(at(start, cross[0]));
            for (pair, &main) in cross.windows(2).zip(channel) {
                if pair[0] != pair[1] {
                    points.push(at(main, pair[0]));
                    points.push(at(main, pair[1]));
                }
            }
            points.push(at(end, cross[cross.len() - 1]));
            Some(orthogonal(&points))
        })
        .collect()
}

/// Writes the cross offsets of the ports along one side `len` long into
/// `offsets` (by link). `side` holds (cross the link heads to, link) and is
/// sorted here; ports sit PORT_GAP apart around the middle, closer on a
/// side too short for that.
fn spread_ports(side: &mut [(f32, usize)], len: f32, offsets: &mut [f32]) {
    if side.len() < 2 {
        return;
    }
    side.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let last = (side.len() - 1) as f32;
    let gap = PORT_GAP.min((len - 2. * PORT_INSET).max(0.) / last);
    for (k, &(_, i)) in side.iter().enumerate() {
        offsets[i] = (k as f32 - last / 2.) * gap;
    }
}

/// How far channels stay from the side edges arrive at. Group boxes reach
/// GROUP_PAD into every gap, and in TB a group's header too: channels keep
/// clear of both so cross runs don't hug group borders or strike through
/// their titles.
fn channel_trail(lr: bool) -> f32 {
    CHANNEL_LEAD + if lr { 0. } else { GROUP_HEADER }
}

/// The gap after every rank (the last one unused): the sizing's, widened
/// where the label of a link spanning just that gap would not fit on its
/// arriving run, past the channels and a corner. A label's width counts in
/// LR, its height in TB. Longer links have room on their runs through the
/// ranks they skip.
fn rank_gaps(
    rank_count: usize,
    ranks: &[usize],
    links: &[Option<(usize, usize)>],
    sizes: &[Option<(f32, f32)>],
    lr: bool,
    sizing: Sizing,
) -> Vec<f32> {
    let mut gaps = vec![sizing.rank_gap; rank_count];
    let channels_end = sizing.rank_gap - channel_trail(lr);
    for (link, size) in links.iter().zip(sizes) {
        let (Some((lo, hi)), Some((w, h))) = (*link, *size) else {
            continue;
        };
        if ranks[hi] != ranks[lo] + 1 {
            continue;
        }
        let along = if lr { w } else { h };
        let gap = &mut gaps[ranks[lo]];
        *gap = gap.max(channels_end + CORNER + along + 2. * LABEL_PAD);
    }
    gaps
}

/// A label pill's estimated (width, height) in diagram units, `caption`
/// adding a line under the text.
pub fn label_size(text: &str, caption: bool) -> (f32, f32) {
    let w = text.chars().count() as f32 * LABEL_CHAR + LABEL_PADDING;
    let mut h = LABEL_H;
    if w > LABEL_MAX_W {
        h += LABEL_LINE;
    }
    if caption {
        h += LABEL_CAPTION;
    }
    (w.min(LABEL_MAX_W), h)
}

/// The box of a `w` × `h` label centered on `p`.
pub fn label_box(p: Point, w: f32, h: f32) -> Rect {
    Rect {
        x: p.x - w / 2.,
        y: p.y - h / 2.,
        w,
        h,
    }
}

/// Each labelled edge's label center (`sizes` from [`label_size`]).
/// Candidates lie on the edge's straight runs: main-axis runs long enough
/// for the label first, then other runs it fits, then the rest, longest
/// first within each, every run tried at its middle and then slid along it
/// a step at a time either way. Edges go in order, each taking the first
/// candidate clear of every node and of the labels placed before it with no
/// other edge's run passing under it; failing that, the one clashing least.
/// Paths without straight runs fall back to [`EdgePath::label_at`].
fn place_labels(
    edges: &[EdgePath],
    sizes: &[Option<(f32, f32)>],
    nodes: &[Rect],
    lr: bool,
) -> Vec<Option<Point>> {
    let runs: Vec<Vec<(Point, Point)>> = edges
        .iter()
        .map(|path| {
            path.segments
                .iter()
                .filter(|s| s.is_straight())
                .map(|s| (s.from, s.to))
                .collect()
        })
        .collect();
    // Whether an axis-aligned run passes through `r`'s inside.
    let under = |(a, b): (Point, Point), r: &Rect| {
        if (a.y - b.y).abs() < 1e-3 {
            a.y > r.y && a.y < r.bottom() && a.x.min(b.x) < r.right() && a.x.max(b.x) > r.x
        } else if (a.x - b.x).abs() < 1e-3 {
            a.x > r.x && a.x < r.right() && a.y.min(b.y) < r.bottom() && a.y.max(b.y) > r.y
        } else {
            false
        }
    };
    let mut placed: Vec<Rect> = Vec::new();
    edges
        .iter()
        .zip(sizes)
        .enumerate()
        .map(|(i, (path, size))| {
            let (w, h) = (*size)?;
            // (preference, length, run): preference 0 for main-axis runs
            // the label fits on, 1 for other runs it fits on, 2 for the rest.
            let mut order: Vec<(u8, f32, (Point, Point))> = runs[i]
                .iter()
                .map(|&(a, b)| {
                    let len = (b.x - a.x).hypot(b.y - a.y);
                    let horizontal = (b.x - a.x).abs() >= (b.y - a.y).abs();
                    let along = if horizontal { w } else { h };
                    let preference = match (len >= along + 2. * LABEL_PAD, horizontal == lr) {
                        (true, true) => 0,
                        (true, false) => 1,
                        (false, _) => 2,
                    };
                    (preference, len, (a, b))
                })
                .collect();
            order.sort_by(|p, q| p.0.cmp(&q.0).then(q.1.total_cmp(&p.1)));
            let mut best: Option<((usize, usize), Point)> = None;
            'runs: for &(_, len, (a, b)) in &order {
                let horizontal = (b.x - a.x).abs() >= (b.y - a.y).abs();
                let along_run = if horizontal { w } else { h };
                let dir = Point {
                    x: (b.x - a.x) / len,
                    y: (b.y - a.y) / len,
                };
                let mid = Point {
                    x: (a.x + b.x) / 2.,
                    y: (a.y + b.y) / 2.,
                };
                let room = ((len - along_run) / 2. - LABEL_PAD).max(0.);
                let tries = (room / LABEL_STEP) as usize;
                for k in 0..=2 * tries {
                    // 0, +1, -1, +2, -2… steps from the middle.
                    let step = k.div_ceil(2) as f32 * LABEL_STEP;
                    let p = along(mid, dir, if k % 2 == 1 { step } else { -step });
                    let r = label_box(p, w, h);
                    let hard = placed.iter().filter(|o| o.intersects(&r)).count()
                        + nodes.iter().filter(|n| n.intersects(&r)).count();
                    let soft = if hard > 0 {
                        0
                    } else {
                        runs.iter()
                            .enumerate()
                            .filter(|(j, _)| *j != i)
                            .flat_map(|(_, rs)| rs)
                            .filter(|run| under(**run, &r))
                            .count()
                    };
                    let score = (hard, soft);
                    if best.is_none_or(|(most, _)| score < most) {
                        best = Some((score, p));
                    }
                    if score == (0, 0) {
                        break 'runs;
                    }
                }
            }
            let p = best.map_or_else(|| path.label_at(), |(_, p)| p);
            placed.push(label_box(p, w, h));
            Some(p)
        })
        .collect()
}

/// Main-axis positions, evenly spaced from `lo` to `hi`, for the cross runs
/// of `jogs` sharing one gap: `(key, cross where it enters, cross where it
/// leaves)`, entering on the `lo` side. Two jogs cross where the one nearer
/// `lo` spans the other's entering cross, or the other spans the first's
/// leaving cross, so only their order matters: among jogs heading toward
/// higher crosses, the one entering (then leaving) furthest along goes
/// first, never crossing more than the other way round; jogs heading back
/// mirror that, and a pair heading opposite ways crosses as often either way.
pub(crate) fn channels<K: Copy + Ord>(
    mut jogs: Vec<(K, f32, f32)>,
    lo: f32,
    hi: f32,
) -> Vec<(K, f32)> {
    let ahead = |j: &(K, f32, f32)| j.2 > j.1;
    jogs.sort_by(|p, q| {
        ahead(q)
            .cmp(&ahead(p))
            .then_with(|| {
                let order = p.1.total_cmp(&q.1).then(p.2.total_cmp(&q.2));
                if ahead(p) { order.reverse() } else { order }
            })
            .then(p.0.cmp(&q.0))
    });
    let slots = jogs.len() as f32 + 1.;
    jogs.into_iter()
        .enumerate()
        .map(|(k, (key, ..))| (key, lo + (hi - lo) * (k as f32 + 1.) / slots))
        .collect()
}

/// A path along an orthogonal polyline: a straight run per leg (control
/// points at thirds) and, at every bend, a quarter circle of up to
/// [`CORNER`], shrunk to half its shorter leg. Repeated points and points
/// partway along a straight leg are dropped first, so `k` bends always give
/// `2k + 1` segments, alternating runs and corners: a leg its two corners
/// use up stays as a zero-length run, keeping the count motion tweens match on.
pub fn orthogonal(points: &[Point]) -> EdgePath {
    const EPS: f32 = 1e-3;
    let mut legs: Vec<Point> = Vec::with_capacity(points.len());
    for &p in points {
        if legs
            .last()
            .is_some_and(|q| (q.x - p.x).abs() < EPS && (q.y - p.y).abs() < EPS)
        {
            continue;
        }
        if let [.., a, b] = legs[..] {
            // `b` strictly between `a` and `p` on one axis line.
            let level = |a: f32, b: f32, p: f32| (a - b).abs() < EPS && (b - p).abs() < EPS;
            let inside = |a: f32, b: f32, p: f32| (b - a) * (p - b) > 0.;
            if (level(a.x, b.x, p.x) && inside(a.y, b.y, p.y))
                || (level(a.y, b.y, p.y) && inside(a.x, b.x, p.x))
            {
                legs.pop();
            }
        }
        legs.push(p);
    }
    let first = legs.first().copied().unwrap_or_default();
    let Some(&last) = legs.last().filter(|_| legs.len() > 1) else {
        return EdgePath {
            segments: vec![run(first, first)],
        };
    };
    let unit = |a: Point, b: Point| {
        let len = (b.x - a.x).hypot(b.y - a.y);
        (
            Point {
                x: (b.x - a.x) / len,
                y: (b.y - a.y) / len,
            },
            len,
        )
    };
    let mut segments = Vec::with_capacity(2 * legs.len() - 3);
    let mut from = first;
    for bend in legs.windows(3) {
        let (into, before) = unit(bend[0], bend[1]);
        let (out, after) = unit(bend[1], bend[2]);
        let r = CORNER.min(before / 2.).min(after / 2.);
        // A leg both corners use up must be exactly zero long, not a
        // rounding-error sliver pointing either way.
        let enter = Some(along(bend[1], into, -r))
            .filter(|p| (p.x - from.x).abs() >= EPS || (p.y - from.y).abs() >= EPS)
            .unwrap_or(from);
        let leave = along(bend[1], out, r);
        segments.push(run(from, enter));
        segments.push(Cubic {
            from: enter,
            c1: along(enter, into, KAPPA * r),
            c2: along(leave, out, -KAPPA * r),
            to: leave,
        });
        from = leave;
    }
    segments.push(run(from, last));
    EdgePath { segments }
}

fn along(p: Point, dir: Point, by: f32) -> Point {
    Point {
        x: p.x + dir.x * by,
        y: p.y + dir.y * by,
    }
}

/// A straight cubic from `a` to `b`, moving at constant speed.
fn run(a: Point, b: Point) -> Cubic {
    let third = |k: f32| Point {
        x: a.x + (b.x - a.x) * k / 3.,
        y: a.y + (b.y - a.y) * k / 3.,
    };
    Cubic {
        from: a,
        c1: third(1.),
        c2: third(2.),
        to: b,
    }
}

/// Pinned ends: out of the sides facing each other across the axis the
/// centers lie further apart along (the other one if only it has a gap
/// between the boxes), with the elbow halfway.
fn connect(a: &Rect, b: &Rect) -> EdgePath {
    let (ca, cb) = (a.center(), b.center());
    let (dx, dy) = (cb.x - ca.x, cb.y - ca.y);
    let gap_x = (b.x - a.right()).max(a.x - b.right());
    let gap_y = (b.y - a.bottom()).max(a.y - b.bottom());
    let across_x = if dx.abs() >= dy.abs() {
        gap_x > 0. || gap_y <= 0.
    } else {
        gap_x > 0. && gap_y <= 0.
    };
    let points = if across_x {
        let (from_x, to_x) = if dx >= 0. {
            (a.right(), b.x)
        } else {
            (a.x, b.right())
        };
        let mid = (from_x + to_x) / 2.;
        [
            Point { x: from_x, y: ca.y },
            Point { x: mid, y: ca.y },
            Point { x: mid, y: cb.y },
            Point { x: to_x, y: cb.y },
        ]
    } else {
        let (from_y, to_y) = if dy >= 0. {
            (a.bottom(), b.y)
        } else {
            (a.y, b.bottom())
        };
        let mid = (from_y + to_y) / 2.;
        [
            Point { x: ca.x, y: from_y },
            Point { x: ca.x, y: mid },
            Point { x: cb.x, y: mid },
            Point { x: cb.x, y: to_y },
        ]
    };
    let jog = (points[1].x - points[2].x).abs() + (points[1].y - points[2].y).abs();
    if jog < ALIGNED {
        orthogonal(&[points[0], points[3]])
    } else {
        orthogonal(&points)
    }
}

/// A rounded rectangle out of the middle of the side no routed edge uses
/// (the top in LR, the right in TB) and back, short of the next box over.
fn self_loop(r: &Rect, lr: bool) -> EdgePath {
    let c = r.center();
    let half = LOOP_SPAN / 2.;
    let points = if lr {
        let top = r.y - LOOP_REACH;
        [
            Point {
                x: c.x + half,
                y: r.y,
            },
            Point {
                x: c.x + half,
                y: top,
            },
            Point {
                x: c.x - half,
                y: top,
            },
            Point {
                x: c.x - half,
                y: r.y,
            },
        ]
    } else {
        let out = r.right() + LOOP_REACH;
        [
            Point {
                x: r.right(),
                y: c.y - half,
            },
            Point {
                x: out,
                y: c.y - half,
            },
            Point {
                x: out,
                y: c.y + half,
            },
            Point {
                x: r.right(),
                y: c.y + half,
            },
        ]
    };
    orthogonal(&points)
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

/// Pairwise crossings between edge paths, each sampled as a polyline of 8
/// pieces per segment. Edges sharing an end node never count: they meet there
/// by design.
pub fn crossings(d: &Diagram, l: &Layout) -> usize {
    const PIECES: usize = 8;
    /// End nodes, bounds and polyline pieces of one edge.
    type Sampled = ((usize, usize), Rect, Vec<(Point, Point)>);
    let paths: Vec<Sampled> = d
        .edges
        .iter()
        .zip(&l.edges)
        .filter_map(|(e, path)| {
            let ends = (d.node_index(&e.from)?, d.node_index(&e.to)?);
            let mut pieces = Vec::with_capacity(path.segments.len() * PIECES);
            path.polyline(PIECES, |a, b| pieces.push((a, b)));
            Some((ends, path.bounds(), pieces))
        })
        .collect();
    let mut count = 0;
    for (i, ((a1, b1), r1, p1)) in paths.iter().enumerate() {
        for ((a2, b2), r2, p2) in &paths[i + 1..] {
            if a1 == a2 || a1 == b2 || b1 == a2 || b1 == b2 || !r1.intersects(r2) {
                continue;
            }
            count += p1
                .iter()
                .map(|&(s, e)| {
                    p2.iter()
                        .filter(|&&(t, f)| proper_cross(s, e, t, f))
                        .count()
                })
                .sum::<usize>();
        }
    }
    count
}

/// Whether segments `ab` and `cd` cross at a point interior to both.
fn proper_cross(a: Point, b: Point, c: Point, d: Point) -> bool {
    let side = |o: Point, p: Point, q: Point| (p.x - o.x) * (q.y - o.y) - (p.y - o.y) * (q.x - o.x);
    side(a, b, c) * side(a, b, d) < 0. && side(c, d, a) * side(c, d, b) < 0.
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

    /// Groups listed in an order that interleaves their traffic, with edges
    /// skipping ranks across bands and one long cycle back to the start.
    fn platform(direction: &str) -> Diagram {
        diagram(json!({
            "direction": direction,
            "groups": [{"id": "ui"}, {"id": "data"}, {"id": "ops"}, {"id": "core"}],
            "nodes": [
                {"id": "user"}, {"id": "admin"}, {"id": "email"},
                {"id": "web", "group": "ui"}, {"id": "app", "group": "ui"},
                {"id": "console", "group": "ui"},
                {"id": "pg", "group": "data"}, {"id": "redis", "group": "data"},
                {"id": "index", "group": "data"}, {"id": "s3", "group": "data"},
                {"id": "metrics", "group": "ops"}, {"id": "alerts", "group": "ops"},
                {"id": "gateway", "group": "core"}, {"id": "orders", "group": "core"},
                {"id": "billing", "group": "core"}, {"id": "search", "group": "core"},
            ],
            "edges": [
                {"from": "user", "to": "web"}, {"from": "user", "to": "app"},
                {"from": "admin", "to": "console"}, {"from": "web", "to": "gateway"},
                {"from": "app", "to": "gateway"}, {"from": "console", "to": "gateway"},
                {"from": "gateway", "to": "orders"}, {"from": "gateway", "to": "billing"},
                {"from": "gateway", "to": "search"}, {"from": "orders", "to": "pg"},
                {"from": "billing", "to": "pg"}, {"from": "orders", "to": "redis"},
                {"from": "search", "to": "index"}, {"from": "billing", "to": "email"},
                {"from": "user", "to": "search"}, {"from": "app", "to": "s3"},
                {"from": "web", "to": "metrics"}, {"from": "gateway", "to": "metrics"},
                {"from": "pg", "to": "metrics"}, {"from": "metrics", "to": "alerts"},
                {"from": "alerts", "to": "admin"}, {"from": "console", "to": "pg"},
            ],
        }))
    }

    fn shapes() -> Vec<Diagram> {
        vec![
            chain_with_groups("LR"),
            chain_with_groups("TB"),
            cyclic(),
            platform("LR"),
            platform("TB"),
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
                assert!(on_border(a, path.start()) && on_border(b, path.end()));
                assert!(l.bounds.contains(path.label_at()));
            }
            assert!(l.labels.iter().all(Option::is_none));
        }
    }

    /// A state machine's shape: one state feeding three, two of them
    /// answering back, all labelled, plus a link skipping a rank.
    fn labelled(direction: &str) -> Diagram {
        diagram(json!({
            "direction": direction,
            "nodes": [{"id": "work"}, {"id": "blocked"}, {"id": "review"}, {"id": "done"}],
            "edges": [
                {"from": "work", "to": "blocked", "label": "asks"},
                {"from": "blocked", "to": "work", "label": "you answer"},
                {"from": "work", "to": "review", "label": "edits ready [review on]"},
                {"from": "review", "to": "work", "label": "changes requested"},
                {"from": "review", "to": "done", "label": "approved"},
                {"from": "work", "to": "done", "label": "finishes", "style": "async"},
                {"from": "done", "to": "review"},
            ],
        }))
    }

    #[test]
    fn labels_sit_on_their_edges_clear_of_each_other_and_the_nodes() {
        for (direction, lr) in [("LR", true), ("TB", false)] {
            let d = labelled(direction);
            let l = layout(&d);
            let mut boxes = Vec::new();
            for ((edge, path), spot) in d.edges.iter().zip(&l.edges).zip(&l.labels) {
                let Some(text) = &edge.label else {
                    assert_eq!(*spot, None);
                    continue;
                };
                let p = spot.expect("a labelled edge gets a spot");
                assert!(path.distance_to(p) < 0.01, "{direction} {text}");
                let (w, h) = label_size(text, edge.style != EdgeStyle::Sync);
                let r = label_box(p, w, h);
                assert!(l.bounds.contains(Point { x: r.x, y: r.y }));
                assert!(l.bounds.contains(Point {
                    x: r.right(),
                    y: r.bottom()
                }));
                for node in &l.nodes {
                    assert!(!node.intersects(&r), "{direction} {text} on a node");
                }
                boxes.push((text.clone(), r));
            }
            for (i, (a, ra)) in boxes.iter().enumerate() {
                for (b, rb) in &boxes[i + 1..] {
                    assert!(!ra.intersects(rb), "{direction}: {a} overlaps {b}");
                }
            }
            // The first gap widened for its widest label, in LR only.
            let (work, blocked) = (l.nodes[0], l.nodes[1]);
            let gap = if lr {
                blocked.x - work.right()
            } else {
                blocked.y - work.bottom()
            };
            let widest = label_size("edits ready [review on]", false).0;
            if lr {
                assert!(gap > widest, "{gap}");
            } else {
                assert_eq!(gap, RANK_GAP);
            }
        }
    }

    #[test]
    fn edges_leaving_one_side_take_their_own_ports() {
        let d = labelled("LR");
        let l = layout(&d);
        let work = l.nodes[0];
        let starts: Vec<f32> = d
            .edges
            .iter()
            .zip(&l.edges)
            .filter(|(e, _)| e.from == "work")
            .map(|(_, p)| p.start().y)
            .collect();
        assert_eq!(starts.len(), 3);
        for (i, a) in starts.iter().enumerate() {
            assert!(*a > work.y && *a < work.bottom());
            for b in &starts[i + 1..] {
                assert!((a - b).abs() > 1., "{starts:?}");
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

    #[test]
    fn fewer_crossings_than_plain_layering() {
        // Before dummy nodes and band ordering: 19 (LR) and 23 (TB).
        for (direction, before) in [("LR", 19), ("TB", 23)] {
            let d = platform(direction);
            let l = layout(&d);
            let after = crossings(&d, &l);
            assert!(after < before, "{direction}: {after} crossings");
            assert_eq!(l, layout(&d));
        }
    }

    #[test]
    fn edges_go_around_other_boxes() {
        for d in shapes() {
            let l = layout(&d);
            for (e, path) in ends(&d).iter().zip(&l.edges) {
                let (a, b) = e.unwrap();
                let mut points = vec![path.start()];
                path.polyline(16, |_, p| points.push(p));
                for (i, rect) in l.nodes.iter().enumerate() {
                    if i != a && i != b {
                        assert!(points.iter().all(|p| !rect.contains(*p)));
                    }
                }
            }
        }
    }

    #[test]
    fn edge_segments_join_smoothly() {
        let unit = |from: Point, to: Point| {
            let (x, y) = (to.x - from.x, to.y - from.y);
            let len = x.hypot(y);
            (x / len, y / len)
        };
        for d in shapes() {
            for path in layout(&d).edges {
                for pair in path.segments.windows(2) {
                    assert_eq!(pair[0].to, pair[1].from);
                }
                // Zero-length runs (legs their corners use up) carry no
                // direction: the corners on either side meet tangent.
                let moving: Vec<&Cubic> = path.segments.iter().filter(|s| s.from != s.to).collect();
                for pair in moving.windows(2) {
                    let (ix, iy) = unit(pair[0].c2, pair[0].to);
                    let (ox, oy) = unit(pair[1].from, pair[1].c1);
                    assert!((ix - ox).abs() < 1e-3 && (iy - oy).abs() < 1e-3);
                }
            }
        }
    }

    #[test]
    fn routes_are_orthogonal_with_labels_on_straight_runs() {
        const EPS: f32 = 1e-2;
        let level = |a: Point, b: Point| (a.x - b.x).abs() < EPS || (a.y - b.y).abs() < EPS;
        for d in shapes() {
            for path in layout(&d).edges {
                for s in path.segments.iter().filter(|s| s.from != s.to) {
                    if level(s.from, s.to) {
                        // A straight run along one axis.
                        assert!(s.is_straight(), "{s:?}");
                        assert!(level(s.from, s.c1) && level(s.c2, s.to), "{s:?}");
                    } else {
                        // A quarter circle turning from one axis to the other.
                        let (dx, dy) = ((s.to.x - s.from.x).abs(), (s.to.y - s.from.y).abs());
                        assert!((dx - dy).abs() < EPS, "{s:?}");
                        assert!(level(s.from, s.c1) && level(s.c2, s.to), "{s:?}");
                        let into = (s.c1.x - s.from.x, s.c1.y - s.from.y);
                        let out = (s.to.x - s.c2.x, s.to.y - s.c2.y);
                        assert!((into.0 * out.0 + into.1 * out.1).abs() < EPS, "{s:?}");
                    }
                }
                let label = path.label_at();
                assert!(
                    path.segments.iter().any(|s| {
                        s.is_straight() && segment_distance(label, s.from, s.to) < EPS
                    })
                );
            }
        }
    }

    #[test]
    fn edges_turning_in_one_gap_get_their_own_channels() {
        for (direction, lr) in [("LR", true), ("TB", false)] {
            let d = diagram(json!({
                "direction": direction,
                "nodes": [{"id": "a"}, {"id": "b"}, {"id": "c"}, {"id": "d"}],
                "edges": [{"from": "a", "to": "b"}, {"from": "a", "to": "c"}, {"from": "a", "to": "d"}],
            }));
            let l = layout(&d);
            // The target lined up with the source gets one straight run.
            assert_eq!(l.edges.iter().filter(|p| p.segments.len() == 1).count(), 1);
            let main = |p: Point| if lr { p.x } else { p.y };
            let channels: Vec<f32> = l
                .edges
                .iter()
                .filter(|p| p.segments.len() > 1)
                .map(|p| {
                    let across = p
                        .segments
                        .iter()
                        .find(|s| s.is_straight() && (main(s.from) - main(s.to)).abs() < 1e-3)
                        .expect("a run across the main axis");
                    main(across.from)
                })
                .collect();
            assert_eq!(channels.len(), 2);
            assert!((channels[0] - channels[1]).abs() > 10., "{channels:?}");
            let gap = main(Point {
                x: l.nodes[0].right(),
                y: l.nodes[0].bottom(),
            });
            for c in channels {
                assert!(c > gap + GROUP_PAD && c < gap + RANK_GAP - GROUP_PAD);
            }
        }
    }

    #[test]
    fn channels_order_jogs_to_avoid_crossings() {
        // Two jogs heading the same way, staggered: the one entering further
        // along turns first, so neither crosses the other.
        let order = channels(vec![(0, 0., 100.), (1, 50., 150.)], 0., 30.);
        assert_eq!(order, vec![(1, 10.), (0, 20.)]);
        // Heading back mirrors it.
        let order = channels(vec![(0, 150., 50.), (1, 100., 0.)], 0., 30.);
        assert_eq!(order, vec![(1, 10.), (0, 20.)]);
    }
}
