//! The chart kind: bar, line, area, pie/donut and scatter charts the agent
//! shows read-only. Besides the spec, this lays the chart out (axes with
//! round ticks, bars, points, slices) and finds the mark under the pointer,
//! so the app only paints.

use std::f32::consts::{PI, TAU};

use serde_json::{Map, Value, json};

use super::diagram::{Errors, MAX_LABEL, array, id_of, text, truncate};
use super::edits::{format_number, number, plural};
use super::layout::{Point, Rect};
use super::live::Live;

pub const MAX_SERIES: usize = 12;
pub const MAX_POINTS: usize = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChartType {
    #[default]
    Bar,
    Line,
    Area,
    Pie,
    Donut,
    Scatter,
}

impl ChartType {
    pub const ALL: [Self; 6] = [
        Self::Bar,
        Self::Line,
        Self::Area,
        Self::Pie,
        Self::Donut,
        Self::Scatter,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Bar => "bar",
            Self::Line => "line",
            Self::Area => "area",
            Self::Pie => "pie",
            Self::Donut => "donut",
            Self::Scatter => "scatter",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|t| t.name().eq_ignore_ascii_case(s.trim()))
    }

    pub fn is_round(self) -> bool {
        matches!(self, Self::Pie | Self::Donut)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Series {
    pub name: String,
    /// One per label (bar, line, area, pie).
    pub values: Vec<f64>,
    /// `(x, y)` pairs (scatter).
    pub points: Vec<(f64, f64)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Chart {
    pub kind: ChartType,
    pub labels: Vec<String>,
    pub series: Vec<Series>,
    pub x_title: Option<String>,
    pub y_title: Option<String>,
    /// Bars and areas stack their series.
    pub stacked: bool,
    /// Appended to values in tooltips ("ms", "USD").
    pub unit: Option<String>,
    /// Labels and values read from a live source (see `genui::live`).
    pub live: Option<Box<Live>>,
}

fn label_of(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(truncate(s.trim(), MAX_LABEL)),
        Value::Number(n) => Some(n.as_f64().map_or_else(|| n.to_string(), format_number)),
        _ => None,
    }
}

impl Chart {
    pub fn parse(spec: &Value) -> Result<Chart, Vec<String>> {
        let Some(spec) = spec.as_object() else {
            return Err(vec!["the spec must be a JSON object".into()]);
        };
        let mut errors = Errors::default();
        let kind = match spec.get("type") {
            None | Some(Value::Null) => ChartType::Bar,
            Some(t) => match t.as_str().and_then(ChartType::parse) {
                Some(kind) => kind,
                None => {
                    return Err(vec![format!(
                        "unknown chart type {t} (one of {})",
                        ChartType::ALL.map(ChartType::name).join(", ")
                    )]);
                }
            },
        };
        let Some(series_raw) = spec.get("series").and_then(Value::as_array) else {
            return Err(vec!["\"series\" must be an array".into()]);
        };
        if series_raw.is_empty() {
            errors.push("a chart needs at least one series".into());
        }
        if series_raw.len() > MAX_SERIES {
            errors.push(format!(
                "too many series ({}, at most {MAX_SERIES})",
                series_raw.len()
            ));
        }
        let labels_raw = array(spec.get("labels"));
        if labels_raw.len() > MAX_POINTS {
            errors.push(format!(
                "too many labels ({}, at most {MAX_POINTS})",
                labels_raw.len()
            ));
        }
        let mut labels: Vec<String> = labels_raw
            .iter()
            .take(MAX_POINTS)
            .map(|l| label_of(l).unwrap_or_default())
            .collect();
        let mut series: Vec<Series> = Vec::new();
        for (i, raw) in series_raw.iter().enumerate().take(MAX_SERIES) {
            let name = text(raw.get("name").or_else(|| raw.get("label")), MAX_LABEL)
                .or_else(|| raw.get("id").and_then(id_of))
                .unwrap_or_else(|| format!("Series {}", i + 1));
            let mut s = Series {
                name,
                values: Vec::new(),
                points: Vec::new(),
            };
            if kind == ChartType::Scatter {
                let points = array(raw.get("points").or_else(|| raw.get("values")));
                if points.len() > MAX_POINTS {
                    errors.push(format!(
                        "series “{}” has too many points ({}, at most {MAX_POINTS})",
                        s.name,
                        points.len()
                    ));
                }
                for (j, p) in points.iter().enumerate().take(MAX_POINTS) {
                    let xy = match p {
                        Value::Array(xy) if xy.len() == 2 => number(&xy[0]).zip(number(&xy[1])),
                        Value::Object(_) => {
                            p.get("x").and_then(number).zip(p.get("y").and_then(number))
                        }
                        _ => None,
                    };
                    match xy {
                        Some(xy) => s.points.push(xy),
                        None => errors.push(format!(
                            "series “{}” point #{} must be [x, y] or {{x, y}} numbers",
                            s.name,
                            j + 1
                        )),
                    }
                }
            } else {
                let values = array(raw.get("values").or_else(|| raw.get("data")));
                if values.len() > MAX_POINTS {
                    errors.push(format!(
                        "series “{}” has too many values ({}, at most {MAX_POINTS})",
                        s.name,
                        values.len()
                    ));
                }
                for (j, v) in values.iter().enumerate().take(MAX_POINTS) {
                    match number(v) {
                        Some(n) if kind.is_round() && n < 0. => errors.push(format!(
                            "series “{}” value #{} is negative; a {} chart needs values ≥ 0",
                            s.name,
                            j + 1,
                            kind.name()
                        )),
                        Some(n) => s.values.push(n),
                        None => errors.push(format!(
                            "series “{}” value #{} is not a number",
                            s.name,
                            j + 1
                        )),
                    }
                }
            }
            series.push(s);
        }
        if kind != ChartType::Scatter {
            let longest = series.iter().map(|s| s.values.len()).max().unwrap_or(0);
            if labels.is_empty() {
                labels = (1..=longest).map(|n| n.to_string()).collect();
            }
            for s in &series {
                if s.values.len() != labels.len() {
                    errors.push(format!(
                        "series “{}” has {} but there are {}",
                        s.name,
                        plural(s.values.len(), "value"),
                        plural(labels.len(), "label")
                    ));
                }
            }
        }
        let live = match spec.get("live") {
            None | Some(Value::Null) => None,
            Some(_) if kind == ChartType::Scatter => {
                errors.push("a live chart needs labels: any type but scatter".into());
                None
            }
            Some(raw) => Live::parse(raw)
                .map_err(|live_errors| live_errors.into_iter().for_each(|e| errors.push(e)))
                .ok()
                .map(Box::new),
        };
        if !errors.0.is_empty() {
            return Err(errors.0);
        }
        Ok(Chart {
            kind,
            labels,
            series,
            x_title: text(spec.get("xTitle"), MAX_LABEL),
            y_title: text(spec.get("yTitle"), MAX_LABEL),
            stacked: spec.get("stacked").and_then(Value::as_bool) == Some(true)
                && matches!(kind, ChartType::Bar | ChartType::Area),
            unit: text(spec.get("unit"), 16),
            live,
        })
    }

    pub fn to_value(&self) -> Value {
        let mut map = Map::new();
        map.insert("type".into(), json!(self.kind.name()));
        if self.kind != ChartType::Scatter {
            map.insert("labels".into(), json!(self.labels));
        }
        map.insert(
            "series".into(),
            self.series
                .iter()
                .map(|s| {
                    if self.kind == ChartType::Scatter {
                        let points: Vec<[f64; 2]> = s.points.iter().map(|&(x, y)| [x, y]).collect();
                        json!({"name": s.name, "points": points})
                    } else {
                        json!({"name": s.name, "values": s.values})
                    }
                })
                .collect(),
        );
        for (key, value) in [
            ("xTitle", &self.x_title),
            ("yTitle", &self.y_title),
            ("unit", &self.unit),
        ] {
            if let Some(value) = value {
                map.insert(key.into(), json!(value));
            }
        }
        if self.stacked {
            map.insert("stacked".into(), json!(true));
        }
        if let Some(live) = &self.live {
            map.insert("live".into(), live.to_value());
        }
        Value::Object(map)
    }

    /// The same chart with every value at zero (scatter: at the lowest y),
    /// where it grows from when first shown.
    pub fn zeroed(&self) -> Chart {
        let floor = self
            .series
            .iter()
            .flat_map(|s| s.points.iter().map(|p| p.1))
            .fold(f64::INFINITY, f64::min);
        let mut out = self.clone();
        for s in &mut out.series {
            s.values.iter_mut().for_each(|v| *v = 0.);
            s.points.iter_mut().for_each(|p| p.1 = floor);
        }
        out
    }

    /// `to` with its values `t` (0..1) of the way from `from`'s; values
    /// `from` lacks start at zero (or where `to` has them, for scatter).
    pub fn lerp(from: &Chart, to: &Chart, t: f32) -> Chart {
        let t = f64::from(t.clamp(0., 1.));
        let mut out = to.clone();
        for (i, s) in out.series.iter_mut().enumerate() {
            let old = from.series.get(i).filter(|_| from.kind == to.kind);
            for (j, v) in s.values.iter_mut().enumerate() {
                let start = old.and_then(|o| o.values.get(j)).copied().unwrap_or(0.);
                *v = start + (*v - start) * t;
            }
            for (j, p) in s.points.iter_mut().enumerate() {
                let start = old.and_then(|o| o.points.get(j)).copied().unwrap_or(*p);
                *p = (start.0 + (p.0 - start.0) * t, start.1 + (p.1 - start.1) * t);
            }
        }
        out
    }

    /// The value of series `series` at `index`, for tooltips.
    pub fn mark_text(&self, series: usize, index: usize) -> Option<(String, String)> {
        let s = self.series.get(series)?;
        let unit = self
            .unit
            .as_deref()
            .map(|u| format!(" {u}"))
            .unwrap_or_default();
        if self.kind == ChartType::Scatter {
            let (x, y) = *s.points.get(index)?;
            return Some((
                s.name.clone(),
                format!("{}, {}{unit}", format_number(x), format_number(y)),
            ));
        }
        let value = *s.values.get(index)?;
        let label = self.labels.get(index).cloned().unwrap_or_default();
        let title = if self.kind.is_round() || self.series.len() == 1 {
            label
        } else {
            format!("{} · {label}", s.name)
        };
        let mut text = format!("{}{unit}", format_number(value));
        if self.kind.is_round() {
            let total: f64 = s.values.iter().sum();
            if total > 0. {
                text.push_str(&format!(" ({:.0} %)", value / total * 100.));
            }
        }
        Some((title, text))
    }
}

/// A value axis: its range and round ticks inside it.
#[derive(Debug, Clone, PartialEq)]
pub struct Axis {
    pub min: f64,
    pub max: f64,
    pub ticks: Vec<f64>,
}

impl Axis {
    /// Round ticks (1, 2, 2.5 or 5 × 10ⁿ apart), about `count` of them,
    /// the range widened to the outer ticks.
    pub fn nice(min: f64, max: f64, count: usize) -> Axis {
        let (mut min, mut max) = if min.is_finite() && max.is_finite() {
            (min.min(max), max.max(min))
        } else {
            (0., 1.)
        };
        if (max - min).abs() < f64::EPSILON {
            let pad = if min == 0. { 1. } else { min.abs() * 0.1 };
            min -= pad;
            max += pad;
        }
        let raw = (max - min) / count.max(1) as f64;
        let magnitude = 10f64.powf(raw.log10().floor());
        let step = [1., 2., 2.5, 5., 10.]
            .into_iter()
            .map(|m| m * magnitude)
            .find(|s| *s >= raw)
            .unwrap_or(10. * magnitude);
        let lo = (min / step).floor() * step;
        let hi = (max / step).ceil() * step;
        // Snap float noise (0.6000000000000001) to the step's decimals.
        let scale = 10f64.powi((1. - step.log10().floor()).max(0.) as i32);
        let ticks = (0..)
            .map(|i| lo + step * f64::from(i))
            .take_while(|t| *t <= hi + step * 1e-9)
            .map(|t| (t * scale).round() / scale)
            .collect();
        Axis {
            min: lo,
            max: hi,
            ticks,
        }
    }

    /// Where `v` falls, 0 at `min` and 1 at `max`.
    pub fn fraction(&self, v: f64) -> f32 {
        if self.max <= self.min {
            return 0.;
        }
        ((v - self.min) / (self.max - self.min)) as f32
    }
}

/// Ticks for people: 1.2k, 3.4M, 0.25.
pub fn short_number(n: f64) -> String {
    let abs = n.abs();
    let (scaled, suffix) = if abs >= 1e9 {
        (n / 1e9, "B")
    } else if abs >= 1e6 {
        (n / 1e6, "M")
    } else if abs >= 1e4 {
        (n / 1e3, "k")
    } else {
        (n, "")
    };
    let rounded = (scaled * 100.).round() / 100.;
    format!("{}{suffix}", format_number(rounded))
}

/// The axes a chart is drawn against: the value (y) axis, and x for
/// scatter. `None` for pie and donut.
#[derive(Debug, Clone, PartialEq)]
pub struct Axes {
    pub y: Axis,
    pub x: Option<Axis>,
}

impl Axes {
    pub fn of(chart: &Chart) -> Option<Axes> {
        let span = |values: &mut dyn Iterator<Item = f64>| {
            values.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
                (lo.min(v), hi.max(v))
            })
        };
        match chart.kind {
            ChartType::Pie | ChartType::Donut => None,
            ChartType::Scatter => {
                let (x0, x1) = span(
                    &mut chart
                        .series
                        .iter()
                        .flat_map(|s| s.points.iter().map(|p| p.0)),
                );
                let (y0, y1) = span(
                    &mut chart
                        .series
                        .iter()
                        .flat_map(|s| s.points.iter().map(|p| p.1)),
                );
                Some(Axes {
                    y: Axis::nice(y0, y1, 5),
                    x: Some(Axis::nice(x0, x1, 6)),
                })
            }
            _ => {
                let (lo, hi) = if chart.stacked {
                    // Positive and negative values stack apart.
                    let mut lo = 0f64;
                    let mut hi = 0f64;
                    for i in 0..chart.labels.len() {
                        let (mut up, mut down) = (0., 0.);
                        for s in &chart.series {
                            let v = s.values.get(i).copied().unwrap_or(0.);
                            if v >= 0. {
                                up += v;
                            } else {
                                down += v;
                            }
                        }
                        hi = hi.max(up);
                        lo = lo.min(down);
                    }
                    (lo, hi)
                } else {
                    span(&mut chart.series.iter().flat_map(|s| s.values.iter().copied()))
                };
                // Bars and areas grow from zero.
                let (lo, hi) = if chart.kind == ChartType::Line {
                    (lo, hi)
                } else {
                    (lo.min(0.), hi.max(0.))
                };
                Some(Axes {
                    y: Axis::nice(lo, hi, 5),
                    x: None,
                })
            }
        }
    }
}

/// One thing drawn for a value, in plot coordinates.
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    Bar(Rect),
    Dot(Point),
    /// Angles in radians, clockwise from 12 o'clock.
    Slice {
        center: Point,
        radius: f32,
        inner: f32,
        start: f32,
        end: f32,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Mark {
    pub series: usize,
    pub index: usize,
    pub shape: Shape,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Geometry {
    pub marks: Vec<Mark>,
    /// Per series, the line through its points (line, area).
    pub lines: Vec<Vec<Point>>,
    /// Per series, the area's lower edge (left to right): the previous
    /// stacked series or the zero line.
    pub floors: Vec<Vec<Point>>,
    /// Each label's x center (bar, line, area).
    pub columns: Vec<f32>,
}

impl Geometry {
    /// Lay `chart` out in `plot` against `axes`; pie and donut slices
    /// sweep `sweep` (0..1) of the circle, for the opening animation.
    pub fn new(chart: &Chart, axes: Option<&Axes>, plot: Rect, sweep: f32) -> Geometry {
        let mut g = Geometry::default();
        if chart.kind.is_round() {
            let Some(s) = chart.series.first() else {
                return g;
            };
            let total: f64 = s.values.iter().sum();
            if total <= 0. {
                return g;
            }
            let center = Point {
                x: plot.x + plot.w / 2.,
                y: plot.y + plot.h / 2.,
            };
            let radius = plot.w.min(plot.h) / 2.;
            let inner = if chart.kind == ChartType::Donut {
                radius * 0.6
            } else {
                0.
            };
            let mut start = 0.;
            for (index, v) in s.values.iter().enumerate() {
                let end = start + (*v / total) as f32 * TAU * sweep.clamp(0., 1.);
                g.marks.push(Mark {
                    series: 0,
                    index,
                    shape: Shape::Slice {
                        center,
                        radius,
                        inner,
                        start,
                        end,
                    },
                });
                start = end;
            }
            return g;
        }
        let Some(axes) = axes else {
            return g;
        };
        let y_at = |v: f64| plot.y + plot.h * (1. - axes.y.fraction(v).clamp(-0.05, 1.05));
        if chart.kind == ChartType::Scatter {
            let Some(x_axis) = &axes.x else {
                return g;
            };
            for (si, s) in chart.series.iter().enumerate() {
                for (index, &(x, y)) in s.points.iter().enumerate() {
                    let at = Point {
                        x: plot.x + plot.w * x_axis.fraction(x).clamp(-0.05, 1.05),
                        y: y_at(y),
                    };
                    g.marks.push(Mark {
                        series: si,
                        index,
                        shape: Shape::Dot(at),
                    });
                }
            }
            return g;
        }
        let n = chart.labels.len().max(1);
        let band = plot.w / n as f32;
        g.columns = (0..n).map(|i| plot.x + band * (i as f32 + 0.5)).collect();
        let zero = y_at(0.);
        // Running stack tops per label, above and below zero.
        let mut ups = vec![0f64; n];
        let mut downs = vec![0f64; n];
        let count = chart.series.len().max(1);
        for (si, s) in chart.series.iter().enumerate() {
            let mut line = Vec::new();
            let mut floor = Vec::new();
            for (index, &v) in s.values.iter().enumerate() {
                let (base, top) = if chart.stacked {
                    let slot = if v >= 0. {
                        &mut ups[index]
                    } else {
                        &mut downs[index]
                    };
                    let base = *slot;
                    *slot += v;
                    (base, *slot)
                } else {
                    (0., v)
                };
                let x = g.columns[index];
                match chart.kind {
                    ChartType::Bar => {
                        let group = band * 0.72;
                        let (left, width) = if chart.stacked {
                            (x - group / 2., group)
                        } else {
                            let w = group / count as f32;
                            (x - group / 2. + w * si as f32, w)
                        };
                        let (y0, y1) = (y_at(base), y_at(top));
                        let gap = if chart.stacked || count == 1 {
                            0.
                        } else {
                            (width * 0.12).min(3.)
                        };
                        g.marks.push(Mark {
                            series: si,
                            index,
                            shape: Shape::Bar(Rect {
                                x: left + gap / 2.,
                                y: y0.min(y1),
                                w: (width - gap).max(1.),
                                h: (y0 - y1).abs(),
                            }),
                        });
                    }
                    _ => {
                        let at = Point { x, y: y_at(top) };
                        line.push(at);
                        floor.push(Point {
                            x,
                            y: if chart.stacked { y_at(base) } else { zero },
                        });
                        g.marks.push(Mark {
                            series: si,
                            index,
                            shape: Shape::Dot(at),
                        });
                    }
                }
            }
            g.lines.push(line);
            g.floors.push(floor);
        }
        g
    }

    /// The mark under `p`: a bar or slice containing it, else the nearest
    /// dot within `reach`.
    pub fn hit(&self, p: Point, reach: f32) -> Option<usize> {
        let mut best: Option<(usize, f32)> = None;
        for (i, mark) in self.marks.iter().enumerate() {
            match &mark.shape {
                Shape::Bar(rect) => {
                    // Thin bars are easier to hit with a little slack.
                    let target = Rect {
                        x: rect.x,
                        y: rect.y - 2.,
                        w: rect.w,
                        h: rect.h + 4.,
                    };
                    if target.contains(p) {
                        return Some(i);
                    }
                }
                Shape::Slice {
                    center,
                    radius,
                    inner,
                    start,
                    end,
                } => {
                    let (dx, dy) = (p.x - center.x, p.y - center.y);
                    let r = dx.hypot(dy);
                    // Clockwise from 12 o'clock.
                    let angle = (dx.atan2(-dy) + TAU) % TAU;
                    if r <= *radius && r >= *inner && angle >= *start && angle < *end {
                        return Some(i);
                    }
                }
                Shape::Dot(at) => {
                    let d = (p.x - at.x).hypot(p.y - at.y);
                    if d <= reach && best.is_none_or(|(_, b)| d < b) {
                        best = Some((i, d));
                    }
                }
            }
        }
        best.map(|(i, _)| i)
    }
}

/// The middle angle of a slice, for placing its label.
pub fn mid_angle(start: f32, end: f32) -> f32 {
    (start + end) / 2. - PI / 2.
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bars() -> Chart {
        Chart::parse(&json!({
            "type": "bar",
            "labels": ["Jan", "Feb", "Mar"],
            "series": [
                {"name": "Revenue", "values": [10, 20, 35]},
                {"name": "Cost", "values": [5, "8", 12]}
            ],
            "yTitle": "k€",
            "unit": "k€"
        }))
        .unwrap()
    }

    const PLOT: Rect = Rect {
        x: 0.,
        y: 0.,
        w: 300.,
        h: 100.,
    };

    #[test]
    fn parses_types_and_defaults() {
        let c = bars();
        assert_eq!(c.series[1].values, vec![5., 8., 12.]);
        assert_eq!(Chart::parse(&c.to_value()).unwrap(), c);
        let line = Chart::parse(&json!({"type": "LINE", "series": [{"values": [1, 2]}]})).unwrap();
        assert_eq!(line.labels, vec!["1", "2"]);
        assert_eq!(line.series[0].name, "Series 1");
        let scatter = Chart::parse(&json!({"type": "scatter", "series": [
            {"name": "p", "points": [[1, 2], {"x": 3, "y": 4}]}
        ]}))
        .unwrap();
        assert_eq!(scatter.series[0].points, vec![(1., 2.), (3., 4.)]);
        assert_eq!(Chart::parse(&scatter.to_value()).unwrap(), scatter);
        // Stacking only applies to bars and areas.
        let stacked =
            Chart::parse(&json!({"type": "line", "stacked": true, "series": [{"values": [1]}]}))
                .unwrap();
        assert!(!stacked.stacked);
    }

    #[test]
    fn readable_errors() {
        assert_eq!(
            Chart::parse(&json!({"type": "radar", "series": []})).unwrap_err(),
            vec!["unknown chart type \"radar\" (one of bar, line, area, pie, donut, scatter)"]
        );
        let errors = Chart::parse(&json!({
            "type": "pie",
            "labels": ["a", "b", "c"],
            "series": [{"name": "S", "values": [1, -2, "x"]}]
        }))
        .unwrap_err();
        assert_eq!(
            errors,
            vec![
                "series “S” value #2 is negative; a pie chart needs values ≥ 0",
                "series “S” value #3 is not a number",
                "series “S” has 1 value but there are 3 labels",
            ]
        );
        let errors =
            Chart::parse(&json!({"type": "scatter", "series": [{"name": "p", "points": [[1]]}]}))
                .unwrap_err();
        assert_eq!(
            errors,
            vec!["series “p” point #1 must be [x, y] or {x, y} numbers"]
        );
    }

    #[test]
    fn nice_axes() {
        let a = Axis::nice(0., 35., 5);
        assert_eq!(a.ticks, vec![0., 10., 20., 30., 40.]);
        assert_eq!((a.min, a.max), (0., 40.));
        let a = Axis::nice(0.1, 0.9, 4);
        assert_eq!(a.ticks, vec![0., 0.2, 0.4, 0.6, 0.8, 1.]);
        let flat = Axis::nice(5., 5., 5);
        assert!(flat.min < 5. && flat.max > 5.);
        assert_eq!(short_number(12_500.), "12.5k");
        assert_eq!(short_number(3_400_000.), "3.4M");
        assert_eq!(short_number(950.), "950");
    }

    #[test]
    fn bars_side_by_side_and_stacked() {
        let c = bars();
        let axes = Axes::of(&c).unwrap();
        assert_eq!(axes.y.max, 40.);
        let g = Geometry::new(&c, Some(&axes), PLOT, 1.);
        assert_eq!(g.marks.len(), 6);
        let Shape::Bar(jan_revenue) = g.marks[0].shape else {
            panic!()
        };
        let Shape::Bar(jan_cost) = g.marks[3].shape else {
            panic!()
        };
        // Revenue 10 of 40 → a quarter of the plot high, left of Cost.
        assert!((jan_revenue.h - 25.).abs() < 0.01);
        assert!(jan_revenue.x < jan_cost.x);
        assert_eq!(
            g.hit(
                Point {
                    x: jan_revenue.x + 1.,
                    y: 90.
                },
                8.
            ),
            Some(0)
        );
        assert_eq!(
            g.hit(
                Point {
                    x: jan_revenue.x + 1.,
                    y: 10.
                },
                8.
            ),
            None
        );

        let mut stacked = c.clone();
        stacked.stacked = true;
        let axes = Axes::of(&stacked).unwrap();
        assert_eq!(axes.y.max, 50., "35 + 12 stacks to 47");
        let g = Geometry::new(&stacked, Some(&axes), PLOT, 1.);
        let (Shape::Bar(low), Shape::Bar(high)) = (&g.marks[0].shape, &g.marks[3].shape) else {
            panic!()
        };
        assert!((low.y - high.bottom()).abs() < 0.01, "cost sits on revenue");
    }

    #[test]
    fn lines_pies_and_scatter_hit_testing() {
        let mut c = bars();
        c.kind = ChartType::Line;
        let axes = Axes::of(&c).unwrap();
        let g = Geometry::new(&c, Some(&axes), PLOT, 1.);
        assert_eq!(g.lines.len(), 2);
        assert_eq!(g.columns, vec![50., 150., 250.]);
        let feb = g.lines[0][1];
        assert_eq!(
            g.hit(
                Point {
                    x: feb.x + 3.,
                    y: feb.y - 2.
                },
                8.
            ),
            Some(1)
        );

        let mut pie = bars();
        pie.kind = ChartType::Donut;
        let g = Geometry::new(&pie, None, PLOT, 1.);
        assert_eq!(g.marks.len(), 3);
        let Shape::Slice { end, .. } = g.marks[2].shape else {
            panic!()
        };
        assert!((end - TAU).abs() < 1e-4);
        // Right of the center at the ring: the first slice (10 of 65 ≈ 55°
        // from 12 o'clock) doesn't reach 3 o'clock; the second does.
        assert_eq!(
            g.hit(
                Point {
                    x: 150. + 45.,
                    y: 50.
                },
                8.
            ),
            Some(1)
        );
        assert_eq!(
            g.hit(Point { x: 150., y: 50. }, 8.),
            None,
            "the donut's hole"
        );
        let half = Geometry::new(&pie, None, PLOT, 0.5);
        let Shape::Slice { end, .. } = half.marks[2].shape else {
            panic!()
        };
        assert!((end - PI).abs() < 1e-4);
        assert_eq!(
            pie.mark_text(0, 2),
            Some(("Mar".into(), "35 k€ (54 %)".into()))
        );
    }

    #[test]
    fn lerp_grows_from_zero_and_tweens_between_versions() {
        let c = bars();
        let half = Chart::lerp(&c.zeroed(), &c, 0.5);
        assert_eq!(half.series[0].values, vec![5., 10., 17.5]);
        let mut next = c.clone();
        next.series[0].values = vec![20., 20., 20.];
        let mid = Chart::lerp(&c, &next, 0.5);
        assert_eq!(mid.series[0].values, vec![15., 20., 27.5]);
        assert_eq!(Chart::lerp(&c, &next, 1.), next);
        assert_eq!(
            c.mark_text(1, 0),
            Some(("Cost · Jan".into(), "5 k€".into()))
        );
    }
}
