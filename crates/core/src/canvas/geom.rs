//! Plane geometry for the canvas: rectangles, distances, polygons, the
//! hatch lines of the Pattern fill and arrow heads. Page units throughout.

use serde::{Deserialize, Serialize};

pub type P = [f32; 2];

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    /// The rectangle spanned by two corners, in any order.
    pub fn from_corners(a: P, b: P) -> Self {
        Self {
            x: a[0].min(b[0]),
            y: a[1].min(b[1]),
            w: (a[0] - b[0]).abs(),
            h: (a[1] - b[1]).abs(),
        }
    }

    /// The bounding box of `points`; `None` for none.
    pub fn around(points: impl IntoIterator<Item = P>) -> Option<Self> {
        let mut it = points.into_iter();
        let first = it.next()?;
        let (mut x0, mut y0, mut x1, mut y1) = (first[0], first[1], first[0], first[1]);
        for p in it {
            x0 = x0.min(p[0]);
            y0 = y0.min(p[1]);
            x1 = x1.max(p[0]);
            y1 = y1.max(p[1]);
        }
        Some(Self {
            x: x0,
            y: y0,
            w: x1 - x0,
            h: y1 - y0,
        })
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
    pub fn center(&self) -> P {
        [self.x + self.w / 2., self.y + self.h / 2.]
    }

    pub fn expand(&self, by: f32) -> Self {
        Self {
            x: self.x - by,
            y: self.y - by,
            w: self.w + by * 2.,
            h: self.h + by * 2.,
        }
    }

    pub fn union(&self, other: &Rect) -> Self {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Self {
            x,
            y,
            w: self.right().max(other.right()) - x,
            h: self.bottom().max(other.bottom()) - y,
        }
    }

    pub fn contains(&self, p: P) -> bool {
        p[0] >= self.x && p[0] <= self.right() && p[1] >= self.y && p[1] <= self.bottom()
    }

    pub fn contains_rect(&self, other: &Rect) -> bool {
        other.x >= self.x
            && other.y >= self.y
            && other.right() <= self.right()
            && other.bottom() <= self.bottom()
    }

    pub fn intersects(&self, other: &Rect) -> bool {
        self.x <= other.right()
            && other.x <= self.right()
            && self.y <= other.bottom()
            && other.y <= self.bottom()
    }

    pub fn corners(&self) -> [P; 4] {
        [
            [self.x, self.y],
            [self.right(), self.y],
            [self.right(), self.bottom()],
            [self.x, self.bottom()],
        ]
    }
}

/// Distance from `p` to the segment `a`–`b`.
pub fn segment_distance(p: P, a: P, b: P) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0. {
        0.
    } else {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0., 1.)
    };
    (p[0] - (a[0] + t * dx)).hypot(p[1] - (a[1] + t * dy))
}

/// Distance from `p` to an open polyline; infinite for no points.
pub fn polyline_distance(p: P, points: &[P]) -> f32 {
    match points {
        [] => f32::INFINITY,
        [only] => (p[0] - only[0]).hypot(p[1] - only[1]),
        _ => points
            .windows(2)
            .map(|w| segment_distance(p, w[0], w[1]))
            .fold(f32::INFINITY, f32::min),
    }
}

/// Even-odd point in polygon.
pub fn point_in_polygon(p: P, poly: &[P]) -> bool {
    let mut inside = false;
    let n = poly.len();
    if n < 3 {
        return false;
    }
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (poly[i], poly[j]);
        if (a[1] > p[1]) != (b[1] > p[1])
            && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
        {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// `steps` points around the ellipse inscribed in `r`, clockwise from the right.
pub fn ellipse_points(r: &Rect, steps: usize) -> Vec<P> {
    let [cx, cy] = r.center();
    let (rx, ry) = (r.w / 2., r.h / 2.);
    (0..steps)
        .map(|i| {
            let a = i as f32 / steps as f32 * std::f32::consts::TAU;
            [cx + rx * a.cos(), cy + ry * a.sin()]
        })
        .collect()
}

/// Distance from `p` to the outline of the ellipse inscribed in `r`
/// (approximate: exact on the axes, close elsewhere), and whether it's inside.
pub fn ellipse_distance(p: P, r: &Rect) -> (f32, bool) {
    let [cx, cy] = r.center();
    let (rx, ry) = ((r.w / 2.).max(0.01), (r.h / 2.).max(0.01));
    let (dx, dy) = (p[0] - cx, p[1] - cy);
    let k = ((dx / rx).powi(2) + (dy / ry).powi(2)).sqrt();
    if k == 0. {
        return (rx.min(ry), true);
    }
    // The outline point on the ray from the center through `p`.
    let (ox, oy) = (dx / k, dy / k);
    ((dx - ox).hypot(dy - oy), k <= 1.)
}

/// Diagonal (45°) hatch segments covering `poly` (even-odd), `spacing` apart:
/// the Pattern fill.
pub fn hatch(poly: &[P], spacing: f32) -> Vec<(P, P)> {
    if poly.len() < 3 || spacing <= 0. {
        return Vec::new();
    }
    // Rotate by -45° so the hatch lines are horizontal scanlines.
    let (s, c) = (
        std::f32::consts::FRAC_1_SQRT_2,
        std::f32::consts::FRAC_1_SQRT_2,
    );
    let rot: Vec<P> = poly
        .iter()
        .map(|p| [p[0] * c + p[1] * s, -p[0] * s + p[1] * c])
        .collect();
    let back = |q: P| [q[0] * c - q[1] * s, q[0] * s + q[1] * c];
    let Some(b) = Rect::around(rot.iter().copied()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let first = (b.y / spacing).floor() * spacing + spacing / 2.;
    let mut y = first;
    let n = rot.len();
    while y < b.bottom() {
        let mut xs: Vec<f32> = Vec::new();
        for i in 0..n {
            let (a, d) = (rot[i], rot[(i + 1) % n]);
            if (a[1] > y) != (d[1] > y) {
                xs.push(a[0] + (y - a[1]) / (d[1] - a[1]) * (d[0] - a[0]));
            }
        }
        xs.sort_by(f32::total_cmp);
        for [x0, x1] in xs.as_chunks::<2>().0 {
            out.push((back([*x0, y]), back([*x1, y])));
        }
        y += spacing;
    }
    out
}

/// The two wing tips of an open arrow head at `b`, pointing from `a`.
pub fn arrow_head(a: P, b: P, size: f32) -> Option<(P, P)> {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len = dx.hypot(dy);
    if len < 0.01 {
        return None;
    }
    let size = size.min(len * 0.6);
    let (ux, uy) = (dx / len, dy / len);
    let spread = 0.5_f32; // ~29°
    let (s, c) = spread.sin_cos();
    let wing = |sign: f32| {
        let (rx, ry) = (ux * c - uy * s * sign, uy * c + ux * s * sign);
        [b[0] - rx * size, b[1] - ry * size]
    };
    Some((wing(1.), wing(-1.)))
}

/// `b` moved so `a`→`b` turns in 15° steps (Shift while drawing a line).
pub fn snap_angle(a: P, b: P) -> P {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len = dx.hypot(dy);
    let step = std::f32::consts::PI / 12.;
    let angle = (dy.atan2(dx) / step).round() * step;
    [a[0] + len * angle.cos(), a[1] + len * angle.sin()]
}

/// The corner opposite `a` making a square with the drag to `b` (Shift).
pub fn square_corner(a: P, b: P) -> P {
    let side = (b[0] - a[0]).abs().max((b[1] - a[1]).abs());
    [
        a[0] + side * (b[0] - a[0]).signum(),
        a[1] + side * (b[1] - a[1]).signum(),
    ]
}

/// A pen stroke whose end comes back near its start closes into a loop
/// (Tern: "Pen strokes that end where they began close into fillable
/// loops"): the ends within `reach` and the path several times longer.
pub fn closes_loop(points: &[P], reach: f32) -> bool {
    let (Some(first), Some(last)) = (points.first(), points.last()) else {
        return false;
    };
    if points.len() < 8 {
        return false;
    }
    let length: f32 = points
        .windows(2)
        .map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]))
        .sum();
    let gap = (last[0] - first[0]).hypot(last[1] - first[1]);
    gap <= reach && length > reach * 6.
}

/// The segment from `a` to `b` sampled every `step` (both ends included).
pub fn sample_segment(a: P, b: P, step: f32) -> Vec<P> {
    let len = (b[0] - a[0]).hypot(b[1] - a[1]);
    let n = ((len / step.max(0.01)).ceil() as usize).max(1);
    (0..=n)
        .map(|i| {
            let t = i as f32 / n as f32;
            [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
        })
        .collect()
}
