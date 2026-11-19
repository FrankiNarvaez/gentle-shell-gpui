//! Pressure-shaped ink outlines: a port of Steve Ruiz's perfect-freehand
//! (MIT), the algorithm tldraw and Excalidraw draw with and whose options
//! (`size`, `thinning`, `smoothing`, `streamline`, `easing`) Tern's canvas
//! ink stores. Input points are where the pointer went; the outline is a
//! closed polygon around them, filled with the nonzero rule.

use std::f32::consts::PI;

/// How fast simulated pressure follows the pointer's speed.
const RATE_OF_PRESSURE_CHANGE: f32 = 0.275;
/// π nudged, as upstream does, so caps don't land exactly on their start.
const FIXED_PI: f32 = PI + 0.0001;
/// Steps of a half-circle cap or a sharp corner.
const CAP_STEPS: f32 = 13.;

type V = [f32; 2];

/// Maps pressure (0..1) to the share of the radius it draws.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Easing {
    Linear,
    /// `sin(t·π/2)`: the pen and highlighter.
    OutSine,
    /// `t³` mirrored: the fountain pen's thin hairlines and full swells.
    InOutCubic,
}

impl Easing {
    fn apply(self, t: f32) -> f32 {
        match self {
            Easing::Linear => t,
            Easing::OutSine => (t * PI / 2.).sin(),
            Easing::InOutCubic => {
                if t < 0.5 {
                    4. * t * t * t
                } else {
                    1. - (-2. * t + 2.).powi(3) / 2.
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StrokeOptions {
    /// Diameter at pressure 0.5.
    pub size: f32,
    /// How much pressure thins the stroke (0: constant width, 1: a lot).
    pub thinning: f32,
    /// How far outline points must be apart, as a share of `size`.
    pub smoothing: f32,
    /// How much each point trails the pointer (0 none … 1 a lot).
    pub streamline: f32,
    pub easing: Easing,
    /// Pressure from the pointer's speed (no real pressure from a mouse).
    pub simulate_pressure: bool,
    /// Length tapered at the start and end (0: a round cap).
    pub taper_start: f32,
    pub taper_end: f32,
    /// The stroke is finished: its last point is the pointer's.
    pub last: bool,
}

impl Default for StrokeOptions {
    fn default() -> Self {
        Self {
            size: 16.,
            thinning: 0.5,
            smoothing: 0.5,
            streamline: 0.5,
            easing: Easing::Linear,
            simulate_pressure: true,
            taper_start: 0.,
            taper_end: 0.,
            last: false,
        }
    }
}

/// One input point after streamlining.
#[derive(Clone, Copy, Debug)]
struct StrokePoint {
    point: V,
    pressure: f32,
    /// Unit vector back toward the previous point.
    vector: V,
    distance: f32,
    running_length: f32,
}

/// The outline polygon around `points` (page units), empty for none.
pub fn outline(points: &[V], options: &StrokeOptions) -> Vec<V> {
    outline_points(&stroke_points(points, options), options)
}

fn add(a: V, b: V) -> V {
    [a[0] + b[0], a[1] + b[1]]
}
fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1]]
}
fn mul(a: V, n: f32) -> V {
    [a[0] * n, a[1] * n]
}
fn neg(a: V) -> V {
    [-a[0], -a[1]]
}
/// Perpendicular, rotated a quarter turn.
fn per(a: V) -> V {
    [a[1], -a[0]]
}
fn dot(a: V, b: V) -> f32 {
    a[0] * b[0] + a[1] * b[1]
}
fn len(a: V) -> f32 {
    a[0].hypot(a[1])
}
fn dist(a: V, b: V) -> f32 {
    len(sub(a, b))
}
fn dist2(a: V, b: V) -> f32 {
    let d = sub(a, b);
    d[0] * d[0] + d[1] * d[1]
}
fn uni(a: V) -> V {
    let l = len(a);
    if l == 0. {
        [0., 0.]
    } else {
        [a[0] / l, a[1] / l]
    }
}
fn lrp(a: V, b: V, t: f32) -> V {
    add(a, mul(sub(b, a), t))
}
fn prj(a: V, b: V, c: f32) -> V {
    add(a, mul(b, c))
}
/// `a` rotated `r` radians around `c`.
fn rot_around(a: V, c: V, r: f32) -> V {
    let (s, co) = r.sin_cos();
    let p = sub(a, c);
    [p[0] * co - p[1] * s + c[0], p[0] * s + p[1] * co + c[1]]
}

fn radius(size: f32, thinning: f32, pressure: f32, easing: Easing) -> f32 {
    size * easing.apply(0.5 - thinning * (0.5 - pressure))
}

fn simulated(prev: f32, distance: f32, size: f32) -> f32 {
    let sp = (distance / size).min(1.);
    let rp = (1. - sp).min(1.);
    (prev + (rp - prev) * (sp * RATE_OF_PRESSURE_CHANGE)).min(1.)
}

fn stroke_points(input: &[V], options: &StrokeOptions) -> Vec<StrokePoint> {
    if input.is_empty() {
        return Vec::new();
    }
    let t = 0.15 + (1. - options.streamline) * 0.85;
    let mut pts: Vec<V> = input.to_vec();
    // Two points: interpolate so the outline has something to shape.
    if pts.len() == 2 {
        let last = pts[1];
        pts.truncate(1);
        for i in 1..5 {
            pts.push(lrp(pts[0], last, i as f32 / 4.));
        }
    }
    if pts.len() == 1 {
        pts.push(add(pts[0], [1., 1.]));
    }
    let mut out = vec![StrokePoint {
        point: pts[0],
        pressure: 0.25,
        vector: [1., 1.],
        distance: 0.,
        running_length: 0.,
    }];
    let mut reached_min = false;
    let mut running_length = 0.;
    let mut prev = out[0];
    let max = pts.len() - 1;
    for (i, &p) in pts.iter().enumerate().skip(1) {
        let point = if options.last && i == max {
            p
        } else {
            lrp(prev.point, p, t)
        };
        if point == prev.point {
            continue;
        }
        let distance = dist(point, prev.point);
        running_length += distance;
        if i < max && !reached_min {
            if running_length < options.size {
                continue;
            }
            reached_min = true;
        }
        prev = StrokePoint {
            point,
            pressure: 0.5,
            vector: uni(sub(prev.point, point)),
            distance,
            running_length,
        };
        out.push(prev);
    }
    out[0].vector = out.get(1).map_or([0., 0.], |p| p.vector);
    out
}

fn outline_points(points: &[StrokePoint], o: &StrokeOptions) -> Vec<V> {
    let size = o.size;
    if points.is_empty() || size <= 0. {
        return Vec::new();
    }
    let last = points.len() - 1;
    let total_length = points[last].running_length;
    let taper_start = o.taper_start;
    let taper_end = o.taper_end;
    let min_distance = (size * o.smoothing).powi(2);
    let mut left: Vec<V> = Vec::new();
    let mut right: Vec<V> = Vec::new();

    let mut prev_pressure = points.iter().take(10).fold(points[0].pressure, |acc, p| {
        let pressure = if o.simulate_pressure {
            simulated(acc, p.distance, size)
        } else {
            p.pressure
        };
        (acc + pressure) / 2.
    });
    let mut r = radius(size, o.thinning, points[last].pressure, o.easing);
    let mut first_radius: Option<f32> = None;
    let mut prev_vector = points[0].vector;
    let mut pl = points[0].point;
    let mut pr = pl;
    let mut tl = pl;
    let mut tr = pr;
    let mut prev_sharp = false;

    for i in 0..points.len() {
        let StrokePoint {
            point,
            mut pressure,
            vector,
            distance,
            running_length,
        } = points[i];
        // Skip the jittery last few points, unless it's the last.
        if i < last && total_length - running_length < 3. {
            continue;
        }
        if o.thinning != 0. {
            if o.simulate_pressure {
                pressure = simulated(prev_pressure, distance, size);
            }
            r = radius(size, o.thinning, pressure, o.easing);
        } else {
            r = size / 2.;
        }
        if first_radius.is_none() {
            first_radius = Some(r);
        }
        let ts = if running_length < taper_start {
            let t = running_length / taper_start;
            t * (2. - t)
        } else {
            1.
        };
        let te = if total_length - running_length < taper_end {
            let t = (total_length - running_length) / taper_end - 1.;
            t * t * t + 1.
        } else {
            1.
        };
        r = (r * ts.min(te)).max(0.01);

        let next_vector = if i < last {
            points[i + 1].vector
        } else {
            vector
        };
        let next_dpr = if i < last {
            dot(vector, next_vector)
        } else {
            1.
        };
        let prev_dpr = dot(vector, prev_vector);
        let sharp = prev_dpr < 0. && !prev_sharp;
        let next_sharp = next_dpr < 0.;
        if sharp || next_sharp {
            let offset = mul(per(prev_vector), r);
            let mut t = 0.;
            while t <= 1. {
                tl = rot_around(sub(point, offset), point, FIXED_PI * t);
                left.push(tl);
                tr = rot_around(add(point, offset), point, FIXED_PI * -t);
                right.push(tr);
                t += 1. / CAP_STEPS;
            }
            pl = tl;
            pr = tr;
            if next_sharp {
                prev_sharp = true;
            }
            continue;
        }
        prev_sharp = false;
        if i == last {
            let offset = mul(per(vector), r);
            left.push(sub(point, offset));
            right.push(add(point, offset));
            continue;
        }
        let offset = mul(per(lrp(next_vector, vector, next_dpr)), r);
        tl = sub(point, offset);
        if i <= 1 || dist2(pl, tl) > min_distance {
            left.push(tl);
            pl = tl;
        }
        tr = add(point, offset);
        if i <= 1 || dist2(pr, tr) > min_distance {
            right.push(tr);
            pr = tr;
        }
        prev_pressure = pressure;
        prev_vector = vector;
    }

    let first_point = points[0].point;
    let last_point = if points.len() > 1 {
        points[last].point
    } else {
        add(points[0].point, [1., 1.])
    };

    // A single point: a dot.
    if points.len() == 1 {
        if (taper_start == 0. && taper_end == 0.) || o.last {
            let start = prj(
                first_point,
                uni(per(sub(first_point, last_point))),
                -first_radius.unwrap_or(r),
            );
            let mut dot = Vec::new();
            let mut t = 1. / CAP_STEPS;
            while t <= 1. {
                dot.push(rot_around(start, first_point, FIXED_PI * 2. * t));
                t += 1. / CAP_STEPS;
            }
            return dot;
        }
        return Vec::new();
    }
    if left.is_empty() || right.is_empty() {
        return Vec::new();
    }

    let mut start_cap = Vec::new();
    if taper_start == 0. {
        let mut t = 1. / CAP_STEPS;
        while t <= 1. {
            start_cap.push(rot_around(right[0], first_point, FIXED_PI * t));
            t += 1. / CAP_STEPS;
        }
    }
    let mut end_cap = Vec::new();
    let direction = per(neg(points[last].vector));
    if taper_end > 0. {
        end_cap.push(last_point);
    } else {
        let start = prj(last_point, direction, r);
        let mut t = 1. / 29.;
        while t < 1. {
            end_cap.push(rot_around(start, last_point, FIXED_PI * 3. * t));
            t += 1. / 29.;
        }
    }

    let mut out = left;
    out.extend(end_cap);
    out.extend(right.into_iter().rev());
    out.extend(start_cap);
    out
}
