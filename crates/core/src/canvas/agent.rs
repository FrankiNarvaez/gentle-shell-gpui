//! What an agent sees of a canvas and how it changes one, after Tern's
//! `canvas_read`, `canvas_edit` and `canvas_shot` tools. The agent's
//! extension drops a request (JSON) beside the canvas file; the open canvas
//! answers it with [`handle`]: the shapes in a flat, readable form, edits
//! applied as one undo step, or an SVG of an area the app turns into a PNG.

use serde_json::{Map, Value, json};

use super::freehand;
use super::geom::{self, Rect};
use super::{
    Color, Doc, Fill, Kind, OPACITIES, P, Pen, Shape, Style, arrow_head_size, bounds_of,
    stroke_options,
};

/// The most points an ink stroke shows in a read.
const READ_POINTS: usize = 40;
/// A shot's longest side, px.
const SHOT_MAX: f32 = 1600.;

/// What the canvas answers, and what the app does about it.
#[derive(Debug, Default)]
pub struct Response {
    pub json: Value,
    /// The shapes changed: one undo step was recorded before.
    pub changed: bool,
    /// Move the view to show this page area.
    pub show: Option<Rect>,
    /// Rasterize this SVG (of `area`, `scale` px per unit) to a PNG.
    pub shot: Option<Shot>,
}

#[derive(Debug)]
pub struct Shot {
    pub svg: String,
    pub width: u32,
    pub height: u32,
    pub scale: f32,
}

/// Answers `request` (`{"action": "read" | "edit" | "shot", …}`) on `doc`,
/// whose user sees the page area `view`. An edit that fails changes nothing.
pub fn handle(doc: &mut Doc, view: Rect, light: bool, request: &Value) -> Response {
    let action = request.get("action").and_then(Value::as_str).unwrap_or("");
    let result = match action {
        "read" => Ok(Response {
            json: describe(doc, view),
            ..Response::default()
        }),
        "edit" => edit(doc, view, request.get("ops")),
        "shot" => shot(doc, view, light, request.get("area")),
        other => Err(format!("unknown action {other:?}: use read, edit or shot")),
    };
    result.unwrap_or_else(|error| Response {
        json: json!({ "ok": false, "error": error }),
        ..Response::default()
    })
}

// ── Read ───────────────────────────────────────────────────────────────────

/// One decimal, rounded in f64 so the JSON reads `-27.6`, not `-27.600000381`.
fn round(v: f32) -> Value {
    json!((f64::from(v) * 10.).round() / 10.)
}

fn rect_json(r: &Rect) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("x".into(), round(r.x));
    m.insert("y".into(), round(r.y));
    m.insert("w".into(), round(r.w));
    m.insert("h".into(), round(r.h));
    m
}

fn name<T: serde::Serialize>(v: T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

/// Every shape back to front, and the page area the user sees.
pub fn describe(doc: &Doc, view: Rect) -> Value {
    json!({
        "ok": true,
        "view": Value::Object(rect_json(&view)),
        "shapes": doc.shapes.iter().map(shape_json).collect::<Vec<_>>(),
    })
}

fn shape_json(s: &Shape) -> Value {
    let mut m = Map::new();
    m.insert("id".into(), json!(s.id));
    let kind = match &s.kind {
        Kind::Ink {
            points,
            highlighter,
            closed,
        } => {
            m.extend(rect_json(
                &Rect::around(points.iter().copied()).unwrap_or_default(),
            ));
            let step = points.len().div_ceil(READ_POINTS).max(1);
            let mut shown: Vec<Value> = points
                .iter()
                .step_by(step)
                .map(|p| json!([p[0].round(), p[1].round()]))
                .collect();
            if let Some(last) = points.last()
                && points.len() > 1
                && (points.len() - 1) % step != 0
            {
                shown.push(json!([last[0].round(), last[1].round()]));
            }
            m.insert("points".into(), Value::Array(shown));
            if *highlighter {
                m.insert("highlighter".into(), json!(true));
            }
            if *closed {
                m.insert("closed".into(), json!(true));
            }
            m.insert("pen".into(), name(s.style.pen));
            "ink"
        }
        Kind::Rect { rect, label } | Kind::Ellipse { rect, label } => {
            m.extend(rect_json(rect));
            if !label.is_empty() {
                m.insert("text".into(), json!(label));
            }
            if matches!(s.kind, Kind::Rect { .. }) {
                "rect"
            } else {
                "ellipse"
            }
        }
        Kind::Line { a, b } | Kind::Arrow { a, b } => {
            m.insert("x1".into(), round(a[0]));
            m.insert("y1".into(), round(a[1]));
            m.insert("x2".into(), round(b[0]));
            m.insert("y2".into(), round(b[1]));
            if matches!(s.kind, Kind::Line { .. }) {
                "line"
            } else {
                "arrow"
            }
        }
        Kind::Text { at, text, .. } => {
            m.extend(rect_json(&s.bounds()));
            m.insert("x".into(), round(at[0]));
            m.insert("y".into(), round(at[1]));
            m.insert("text".into(), json!(text));
            m.insert("font_size".into(), round(s.font_size()));
            "text"
        }
        Kind::Note { rect, text } => {
            m.extend(rect_json(rect));
            m.insert("text".into(), json!(text));
            "note"
        }
        Kind::Image { rect, path } => {
            m.extend(rect_json(rect));
            m.insert("path".into(), json!(path));
            "image"
        }
    };
    m.insert("type".into(), json!(kind));
    m.insert("color".into(), name(s.style.color));
    if s.closed() || s.style.fill != Fill::None {
        m.insert("fill".into(), name(s.style.fill));
    }
    if let Some(c) = s.style.fill_color {
        m.insert("fill_color".into(), name(c));
    }
    m.insert("size".into(), json!(s.style.size));
    if s.style.opacity != 100 {
        m.insert("opacity".into(), json!(s.style.opacity));
    }
    Value::Object(m)
}

// ── Edit ───────────────────────────────────────────────────────────────────

fn num(v: &Value, key: &str) -> Result<Option<f32>, String> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(n) => n
            .as_f64()
            .map(|n| Some(n as f32))
            .filter(|n| n.is_some_and(f32::is_finite))
            .ok_or_else(|| format!("{key} must be a number")),
    }
}

fn need(v: &Value, key: &str, what: &str) -> Result<f32, String> {
    num(v, key)?.ok_or_else(|| format!("{what} needs {key}"))
}

fn text(v: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|k| v.get(*k).and_then(Value::as_str))
        .map(str::to_string)
}

/// A palette name as written by a person or a model ("light slate",
/// "Light-Slate", "light_slate").
fn color(v: &Value, key: &str) -> Result<Option<Color>, String> {
    let Some(raw) = v.get(key).and_then(Value::as_str) else {
        return Ok(None);
    };
    let wanted = raw.trim().to_lowercase().replace([' ', '-'], "_");
    Color::ALL
        .into_iter()
        .find(|c| name(*c).as_str() == Some(wanted.as_str()))
        .map(Some)
        .ok_or_else(|| {
            let names: Vec<String> = Color::ALL
                .iter()
                .filter_map(|c| name(*c).as_str().map(str::to_string))
                .collect();
            format!("unknown color {raw:?}; use one of {}", names.join(", "))
        })
}

fn restyle(style: &mut Style, v: &Value) -> Result<(), String> {
    if let Some(c) = color(v, "color")? {
        style.color = c;
    }
    if v.get("fill_color").is_some_and(Value::is_null) {
        style.fill_color = None;
    } else if let Some(c) = color(v, "fill_color")? {
        style.fill_color = Some(c);
        if style.fill == Fill::None {
            style.fill = Fill::Semi;
        }
    }
    if let Some(f) = v.get("fill").and_then(Value::as_str) {
        style.fill = match f {
            "none" => Fill::None,
            "semi" => Fill::Semi,
            "solid" => Fill::Solid,
            "pattern" => Fill::Pattern,
            other => {
                return Err(format!(
                    "unknown fill {other:?}: none, semi, solid or pattern"
                ));
            }
        };
    }
    if let Some(p) = v.get("pen").and_then(Value::as_str) {
        style.pen = match p {
            "pen" => Pen::Pen,
            "monoline" => Pen::Monoline,
            "fountain" => Pen::Fountain,
            other => return Err(format!("unknown pen {other:?}: pen, monoline or fountain")),
        };
    }
    if let Some(size) = num(v, "size")? {
        style.size = size.round().clamp(0., 5.) as u8;
    }
    if let Some(o) = num(v, "opacity")? {
        // The nearest of the panel's steps.
        let o = o.clamp(0., 100.);
        style.opacity = OPACITIES
            .into_iter()
            .min_by_key(|s| (f32::from(*s) - o).abs() as u32)
            .unwrap_or(100);
    }
    Ok(())
}

fn rect_of(v: &Value, what: &str, default: Option<(f32, f32)>) -> Result<Rect, String> {
    let (w, h) = match default {
        Some((w, h)) => (num(v, "w")?.unwrap_or(w), num(v, "h")?.unwrap_or(h)),
        None => (need(v, "w", what)?, need(v, "h", what)?),
    };
    Ok(Rect::from_corners(
        [need(v, "x", what)?, need(v, "y", what)?],
        [need(v, "x", what)? + w, need(v, "y", what)? + h],
    ))
}

fn points_of(v: &Value) -> Result<Vec<P>, String> {
    let list = v
        .get("points")
        .and_then(Value::as_array)
        .ok_or("ink needs points: [[x, y], …]")?;
    let points: Option<Vec<P>> = list
        .iter()
        .map(|p| {
            let p = p.as_array()?;
            Some([p.first()?.as_f64()? as f32, p.get(1)?.as_f64()? as f32])
        })
        .collect();
    points
        .filter(|p| !p.is_empty())
        .ok_or_else(|| "ink points are [x, y] pairs".into())
}

/// A new shape from its flat form.
fn new_shape(v: &Value) -> Result<Shape, String> {
    let kind = v.get("type").and_then(Value::as_str).unwrap_or("");
    let label = || text(v, &["text", "label"]).unwrap_or_default();
    let kind = match kind {
        "rect" | "rectangle" => Kind::Rect {
            rect: rect_of(v, "rect", None)?,
            label: label(),
        },
        "ellipse" | "circle" => Kind::Ellipse {
            rect: rect_of(v, "ellipse", None)?,
            label: label(),
        },
        "line" | "arrow" => {
            let a = [need(v, "x1", kind)?, need(v, "y1", kind)?];
            let b = [need(v, "x2", kind)?, need(v, "y2", kind)?];
            if kind == "line" {
                Kind::Line { a, b }
            } else {
                Kind::Arrow { a, b }
            }
        }
        "text" => Kind::Text {
            at: [need(v, "x", "text")?, need(v, "y", "text")?],
            text: text(v, &["text"]).ok_or("text needs text")?,
            scale: 1.,
        },
        "note" => Kind::Note {
            rect: rect_of(v, "note", Some((200., 200.)))?,
            text: label(),
        },
        "ink" => Kind::Ink {
            points: points_of(v)?,
            highlighter: v
                .get("highlighter")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            closed: v.get("closed").and_then(Value::as_bool).unwrap_or(false),
        },
        "image" => Kind::Image {
            rect: rect_of(v, "image", None)?,
            path: text(v, &["path"]).ok_or("image needs path")?,
        },
        other => {
            return Err(format!(
                "unknown type {other:?}: rect, ellipse, line, arrow, text, note, ink or image"
            ));
        }
    };
    let mut style = Style::default();
    if matches!(kind, Kind::Note { .. }) {
        style.color = Color::Yellow;
    }
    restyle(&mut style, v)?;
    Ok(Shape { id: 0, kind, style })
}

/// Applies the fields `v` sets to an existing shape.
fn update(shape: &mut Shape, v: &Value) -> Result<(), String> {
    let set = |dst: &mut f32, key: &str| -> Result<(), String> {
        if let Some(n) = num(v, key)? {
            *dst = n;
        }
        Ok(())
    };
    match &mut shape.kind {
        Kind::Rect { rect, label } | Kind::Ellipse { rect, label } => {
            set(&mut rect.x, "x")?;
            set(&mut rect.y, "y")?;
            set(&mut rect.w, "w")?;
            set(&mut rect.h, "h")?;
            if let Some(t) = text(v, &["text", "label"]) {
                *label = t;
            }
        }
        Kind::Note { rect, text: t } => {
            set(&mut rect.x, "x")?;
            set(&mut rect.y, "y")?;
            set(&mut rect.w, "w")?;
            set(&mut rect.h, "h")?;
            if let Some(new) = text(v, &["text"]) {
                *t = new;
            }
        }
        Kind::Image { rect, .. } => {
            set(&mut rect.x, "x")?;
            set(&mut rect.y, "y")?;
            set(&mut rect.w, "w")?;
            set(&mut rect.h, "h")?;
        }
        Kind::Line { a, b } | Kind::Arrow { a, b } => {
            set(&mut a[0], "x1")?;
            set(&mut a[1], "y1")?;
            set(&mut b[0], "x2")?;
            set(&mut b[1], "y2")?;
        }
        Kind::Text { at, text: t, .. } => {
            set(&mut at[0], "x")?;
            set(&mut at[1], "y")?;
            if let Some(new) = text(v, &["text"]) {
                *t = new;
            }
        }
        Kind::Ink { points, .. } => {
            if v.get("points").is_some() {
                *points = points_of(v)?;
            }
            // x, y move the stroke's box there.
            let b = Rect::around(points.iter().copied()).unwrap_or_default();
            let dx = num(v, "x")?.map_or(0., |x| x - b.x);
            let dy = num(v, "y")?.map_or(0., |y| y - b.y);
            for p in points.iter_mut() {
                p[0] += dx;
                p[1] += dy;
            }
        }
    }
    if let Kind::Rect { rect, .. }
    | Kind::Ellipse { rect, .. }
    | Kind::Note { rect, .. }
    | Kind::Image { rect, .. } = &mut shape.kind
    {
        *rect = Rect::from_corners([rect.x, rect.y], [rect.x + rect.w, rect.y + rect.h]);
    }
    restyle(&mut shape.style, v)
}

fn ids(v: &Value) -> Result<Vec<u64>, String> {
    if let Some(id) = v.get("id").and_then(Value::as_u64) {
        return Ok(vec![id]);
    }
    v.get("ids")
        .and_then(Value::as_array)
        .and_then(|list| list.iter().map(Value::as_u64).collect())
        .ok_or_else(|| "needs id or ids".into())
}

fn edit(doc: &mut Doc, view: Rect, ops: Option<&Value>) -> Result<Response, String> {
    let ops = ops
        .and_then(Value::as_array)
        .ok_or("edit needs ops: [{op: …}, …]")?;
    // On a copy: one failing op leaves the page as it was.
    let mut next = doc.clone();
    let (mut added, mut changed, mut deleted) = (Vec::new(), Vec::new(), Vec::new());
    let mut show = None;
    for (i, op) in ops.iter().enumerate() {
        let at = |e: String| format!("op {i}: {e}");
        let kind = op.get("op").and_then(Value::as_str).unwrap_or("");
        let missing = |next: &Doc, ids: &[u64]| -> Result<(), String> {
            match ids.iter().find(|id| next.get(**id).is_none()) {
                Some(id) => Err(format!("no shape {id}")),
                None => Ok(()),
            }
        };
        match kind {
            "add" => {
                let list: Vec<&Value> = match op.get("shapes").and_then(Value::as_array) {
                    Some(list) => list.iter().collect(),
                    None => vec![op.get("shape").unwrap_or(op)],
                };
                for v in list {
                    let shape = new_shape(v).map_err(at)?;
                    added.extend(next.paste(vec![shape], [0., 0.]));
                }
            }
            "update" => {
                let id = ids(op).map_err(at)?;
                missing(&next, &id).map_err(at)?;
                for id in id {
                    let shape = next
                        .get_mut(id)
                        .ok_or_else(|| at(format!("no shape {id}")))?;
                    update(shape, op).map_err(at)?;
                    changed.push(id);
                }
            }
            "move" => {
                let id = ids(op).map_err(at)?;
                missing(&next, &id).map_err(at)?;
                let d = [
                    num(op, "dx").map_err(at)?.unwrap_or(0.),
                    num(op, "dy").map_err(at)?.unwrap_or(0.),
                ];
                for s in next.shapes.iter_mut().filter(|s| id.contains(&s.id)) {
                    s.translate(d);
                }
                changed.extend(id);
            }
            "delete" => {
                let id = ids(op).map_err(at)?;
                missing(&next, &id).map_err(at)?;
                next.remove(&id);
                deleted.extend(id);
            }
            "clear" => {
                deleted.extend(next.shapes.iter().map(|s| s.id));
                next.shapes.clear();
            }
            "front" | "back" => {
                let id = ids(op).map_err(at)?;
                missing(&next, &id).map_err(at)?;
                next.reorder(&id, kind == "front");
                changed.extend(id);
            }
            "show" => {
                show = Some(if op.get("x").is_some() {
                    rect_of(op, "show", None).map_err(at)?
                } else {
                    let id = ids(op).map_err(at)?;
                    bounds_of(next.shapes.iter().filter(|s| id.contains(&s.id)))
                        .ok_or_else(|| at("no such shapes".into()))?
                });
            }
            other => {
                return Err(at(format!(
                    "unknown op {other:?}: add, update, move, delete, clear, front, back or show"
                )));
            }
        }
    }
    // New shapes the user can't see: bring them into view.
    if show.is_none() {
        let new = bounds_of(next.shapes.iter().filter(|s| added.contains(&s.id)));
        if let Some(b) = new.filter(|b| !view.contains_rect(b)) {
            show = Some(b.union(&view_intersection(&view, b)));
        }
    }
    let touched = !added.is_empty() || !changed.is_empty() || !deleted.is_empty();
    *doc = next;
    Ok(Response {
        json: json!({ "ok": true, "added": added, "changed": changed, "deleted": deleted }),
        changed: touched,
        show,
        shot: None,
    })
}

/// What of `view` stays useful to keep in sight with `b` (none: just `b`).
fn view_intersection(view: &Rect, b: Rect) -> Rect {
    if view.intersects(&b) { *view } else { b }
}

// ── Shot ───────────────────────────────────────────────────────────────────

fn shot(doc: &Doc, view: Rect, light: bool, area: Option<&Value>) -> Result<Response, String> {
    let area = match area {
        None | Some(Value::Null) => view,
        Some(Value::String(s)) if s == "view" => view,
        Some(Value::String(s)) if s == "all" => {
            doc.content_bounds().map_or(view, |b| b.expand(24.))
        }
        Some(v @ Value::Object(_)) => rect_of(v, "area", None)?,
        Some(_) => return Err("area is \"view\", \"all\" or {x, y, w, h}".into()),
    };
    if area.w < 1. || area.h < 1. {
        return Err("the area is empty".into());
    }
    let scale = (SHOT_MAX / area.w.max(area.h)).min(2.);
    let (width, height) = (
        (area.w * scale).ceil() as u32,
        (area.h * scale).ceil() as u32,
    );
    Ok(Response {
        json: json!({ "ok": true, "area": Value::Object(rect_json(&area)) }),
        shot: Some(Shot {
            svg: svg(doc, &area, light),
            width,
            height,
            scale,
        }),
        ..Response::default()
    })
}

fn hex(c: Color, light: bool) -> String {
    format!("#{:06x}", c.rgb(light))
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The smoothed loop through `points`, as an SVG path.
fn loop_d(points: &[P]) -> String {
    let n = points.len();
    if n < 3 {
        return String::new();
    }
    let mid = |a: P, b: P| [(a[0] + b[0]) / 2., (a[1] + b[1]) / 2.];
    let m = mid(points[0], points[1]);
    let mut d = format!("M{:.2} {:.2}", m[0], m[1]);
    for i in 1..=n {
        let c = points[i % n];
        let to = mid(c, points[(i + 1) % n]);
        d.push_str(&format!(
            " Q{:.2} {:.2} {:.2} {:.2}",
            c[0], c[1], to[0], to[1]
        ));
    }
    d.push('Z');
    d
}

fn fill_attr(style: &Style, light: bool) -> String {
    let c = hex(style.fill_color(), light);
    match style.fill {
        Fill::None => "fill=\"none\"".into(),
        Fill::Solid => format!("fill=\"{c}\""),
        Fill::Semi | Fill::Pattern => format!("fill=\"{c}\" fill-opacity=\"0.22\""),
    }
}

fn hatch_svg(out: &mut String, s: &Shape, light: bool) {
    if s.style.fill != Fill::Pattern {
        return;
    }
    let Some(poly) = s.fill_polygon() else { return };
    let c = hex(s.style.fill_color(), light);
    let w = (s.style.width() * 0.7).clamp(1., 5.);
    for (a, b) in geom::hatch(&poly, 9f32.max(s.style.width() * 3.)) {
        out.push_str(&format!(
            "<line x1=\"{:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\" stroke=\"{c}\" stroke-width=\"{w}\" stroke-linecap=\"round\"/>",
            a[0], a[1], b[0], b[1]
        ));
    }
}

fn text_svg(
    out: &mut String,
    lines: &[String],
    x: f32,
    y: f32,
    font: f32,
    color: &str,
    anchor: &str,
) {
    out.push_str(&format!(
        "<text font-family=\"Helvetica, Arial, sans-serif\" font-size=\"{font:.1}\" fill=\"{color}\" text-anchor=\"{anchor}\">"
    ));
    for (i, line) in lines.iter().enumerate() {
        out.push_str(&format!(
            "<tspan x=\"{x:.1}\" y=\"{:.1}\">{}</tspan>",
            y + font * (0.95 + 1.3 * i as f32),
            esc(line)
        ));
    }
    out.push_str("</text>");
}

/// `text` broken into lines that fit `width` at `font` (estimated metrics).
fn wrap(text: &str, width: f32, font: f32) -> Vec<String> {
    let per_line = ((width / (font * 0.55)).floor() as usize).max(1);
    let mut lines = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        for word in para.split(' ') {
            if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > per_line {
                lines.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        lines.push(line);
    }
    lines
}

/// The shapes in `area` on the page color, as an SVG `area.w`×`area.h`
/// units big (no grid, no selection).
pub fn svg(doc: &Doc, area: &Rect, light: bool) -> String {
    let mut out = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w:.0}\" height=\"{h:.0}\" viewBox=\"{x:.2} {y:.2} {w:.2} {h:.2}\"><rect x=\"{x:.2}\" y=\"{y:.2}\" width=\"{w:.2}\" height=\"{h:.2}\" fill=\"{bg}\"/>",
        x = area.x,
        y = area.y,
        w = area.w,
        h = area.h,
        bg = hex(Color::Paper, light),
    );
    for s in doc.shapes.iter().filter(|s| area.intersects(&s.bounds())) {
        let style = &s.style;
        let stroke = hex(style.color, light);
        let width = style.width();
        out.push_str(&format!("<g opacity=\"{:.2}\">", style.alpha()));
        match &s.kind {
            Kind::Ink {
                points,
                highlighter,
                closed,
            } => {
                if *closed && style.fill != Fill::None && !highlighter {
                    out.push_str(&format!(
                        "<path d=\"{}\" {}/>",
                        loop_d(points),
                        fill_attr(style, light)
                    ));
                    hatch_svg(&mut out, s, light);
                }
                let outline = freehand::outline(
                    points,
                    &stroke_options(style.pen, *highlighter, width, true),
                );
                let alpha = if *highlighter { 0.42 } else { 1. };
                out.push_str(&format!(
                    "<path d=\"{}\" fill=\"{stroke}\" fill-opacity=\"{alpha}\"/>",
                    loop_d(&outline)
                ));
            }
            Kind::Rect { rect, label } | Kind::Ellipse { rect, label } => {
                let fill = fill_attr(style, light);
                if matches!(s.kind, Kind::Rect { .. }) {
                    out.push_str(&format!(
                        "<rect x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" {fill} stroke=\"{stroke}\" stroke-width=\"{width}\" stroke-linejoin=\"round\"/>",
                        rect.x, rect.y, rect.w, rect.h
                    ));
                } else {
                    let [cx, cy] = rect.center();
                    out.push_str(&format!(
                        "<ellipse cx=\"{cx:.1}\" cy=\"{cy:.1}\" rx=\"{:.1}\" ry=\"{:.1}\" {fill} stroke=\"{stroke}\" stroke-width=\"{width}\"/>",
                        rect.w / 2.,
                        rect.h / 2.
                    ));
                }
                hatch_svg(&mut out, s, light);
                if !label.is_empty() {
                    let font = style.font_size();
                    let lines = wrap(label, rect.w - 16., font);
                    let [cx, cy] = rect.center();
                    let top = cy - lines.len() as f32 * font * 1.3 / 2.;
                    let color = hex(style.color, light);
                    text_svg(&mut out, &lines, cx, top, font, &color, "middle");
                }
            }
            Kind::Line { a, b } | Kind::Arrow { a, b } => {
                out.push_str(&format!(
                    "<line x1=\"{:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\" stroke=\"{stroke}\" stroke-width=\"{width}\" stroke-linecap=\"round\"/>",
                    a[0], a[1], b[0], b[1]
                ));
                if matches!(s.kind, Kind::Arrow { .. })
                    && let Some((l, r)) = geom::arrow_head(*a, *b, arrow_head_size(width))
                {
                    out.push_str(&format!(
                        "<polyline points=\"{:.1},{:.1} {:.1},{:.1} {:.1},{:.1}\" fill=\"none\" stroke=\"{stroke}\" stroke-width=\"{width}\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/>",
                        l[0], l[1], b[0], b[1], r[0], r[1]
                    ));
                }
            }
            Kind::Text { at, text, .. } => {
                let lines: Vec<String> = text.split('\n').map(str::to_string).collect();
                text_svg(
                    &mut out,
                    &lines,
                    at[0],
                    at[1],
                    s.font_size(),
                    &stroke,
                    "start",
                );
            }
            Kind::Note { rect, text } => {
                out.push_str(&format!(
                    "<rect x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" rx=\"6\" fill=\"{stroke}\"/>",
                    rect.x, rect.y, rect.w, rect.h
                ));
                let font = style.font_size();
                let c = style.color.rgb(light);
                let lum = 0.299 * ((c >> 16) & 0xff) as f32
                    + 0.587 * ((c >> 8) & 0xff) as f32
                    + 0.114 * (c & 0xff) as f32;
                let ink = if lum > 150. { "#1d1f23" } else { "#f4f4f5" };
                let lines = wrap(text, rect.w - 28., font);
                text_svg(
                    &mut out,
                    &lines,
                    rect.x + 14.,
                    rect.y + 14.,
                    font,
                    ink,
                    "start",
                );
            }
            Kind::Image { rect, path } => {
                // The picture itself isn't embedded: its frame and name.
                out.push_str(&format!(
                    "<rect x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" fill=\"none\" stroke=\"{stroke}\" stroke-dasharray=\"6 4\"/>",
                    rect.x, rect.y, rect.w, rect.h
                ));
                let file = path.rsplit(['/', '\\']).next().unwrap_or(path);
                text_svg(
                    &mut out,
                    &[format!("image: {file}")],
                    rect.x + 8.,
                    rect.y + 8.,
                    14.,
                    &stroke,
                    "start",
                );
            }
        }
        out.push_str("</g>");
    }
    out.push_str("</svg>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> Rect {
        Rect {
            x: 0.,
            y: 0.,
            w: 800.,
            h: 600.,
        }
    }

    #[test]
    fn an_edit_adds_labelled_boxes_and_arrows_the_read_shows_back() {
        let mut doc = Doc::default();
        let r = handle(
            &mut doc,
            view(),
            false,
            &json!({"action": "edit", "ops": [{"op": "add", "shapes": [
                {"type": "rect", "x": 10, "y": 10, "w": 120, "h": 60, "text": "API", "color": "Light Slate", "fill": "semi"},
                {"type": "arrow", "x1": 130, "y1": 40, "x2": 260, "y2": 40, "color": "red"},
                {"type": "note", "x": 300, "y": 10, "text": "todo"}
            ]}]}),
        );
        assert!(r.changed && r.show.is_none(), "{}", r.json);
        assert_eq!(r.json["added"], json!([1, 2, 3]));
        let read = describe(&doc, view());
        let shapes = read["shapes"].as_array().unwrap();
        assert_eq!(shapes[0]["type"], "rect");
        assert_eq!(shapes[0]["text"], "API");
        assert_eq!(shapes[0]["color"], "light_slate");
        assert_eq!(shapes[0]["fill"], "semi");
        assert_eq!(shapes[1]["x2"], json!(260.0));
        assert_eq!(shapes[2]["color"], "yellow");
        assert_eq!(shapes[2]["w"], json!(200.0));
    }

    #[test]
    fn a_failing_op_leaves_the_page_untouched() {
        let mut doc = Doc::default();
        handle(
            &mut doc,
            view(),
            false,
            &json!({"action": "edit", "ops": [{"op": "add", "type": "line", "x1": 0, "y1": 0, "x2": 5, "y2": 5}]}),
        );
        let before = doc.clone();
        let r = handle(
            &mut doc,
            view(),
            false,
            &json!({"action": "edit", "ops": [
                {"op": "delete", "id": 1},
                {"op": "update", "id": 99, "x": 3}
            ]}),
        );
        assert_eq!(r.json["ok"], false);
        assert!(
            r.json["error"]
                .as_str()
                .unwrap()
                .contains("op 1: no shape 99")
        );
        assert!(!r.changed);
        assert_eq!(doc, before);
    }

    #[test]
    fn updates_move_restyle_and_retext_in_place() {
        let mut doc = Doc::default();
        handle(
            &mut doc,
            view(),
            false,
            &json!({"action": "edit", "ops": [{"op": "add", "shapes": [
                {"type": "ink", "points": [[0, 0], [10, 10], [20, 0]]},
                {"type": "text", "x": 50, "y": 50, "text": "a"}
            ]}]}),
        );
        let r = handle(
            &mut doc,
            view(),
            false,
            &json!({"action": "edit", "ops": [
                {"op": "update", "id": 1, "x": 100, "y": 200, "color": "blue", "pen": "fountain"},
                {"op": "update", "id": 2, "text": "b", "size": 9}
            ]}),
        );
        assert_eq!(r.json["changed"], json!([1, 2]));
        let Kind::Ink { points, .. } = &doc.shapes[0].kind else {
            unreachable!()
        };
        assert_eq!(points[0], [100., 200.]);
        assert_eq!(doc.shapes[0].style.color, Color::Blue);
        assert_eq!(doc.shapes[0].style.pen, Pen::Fountain);
        assert!(matches!(&doc.shapes[1].kind, Kind::Text { text, .. } if text == "b"));
        assert_eq!(doc.shapes[1].style.size, 5);
    }

    #[test]
    fn shapes_added_out_of_sight_are_brought_into_view() {
        let mut doc = Doc::default();
        let r = handle(
            &mut doc,
            view(),
            false,
            &json!({"action": "edit", "ops": [{"op": "add", "type": "rect", "x": 5000, "y": 5000, "w": 10, "h": 10}]}),
        );
        let show = r.show.expect("moved to show it");
        assert!(show.contains([5005., 5005.]));
    }

    #[test]
    fn long_ink_reads_as_a_few_points_with_its_ends() {
        let mut doc = Doc::default();
        let points: Vec<Value> = (0..500).map(|i| json!([i, 0])).collect();
        handle(
            &mut doc,
            view(),
            false,
            &json!({"action": "edit", "ops": [{"op": "add", "type": "ink", "points": points}]}),
        );
        let read = describe(&doc, view());
        let shown = read["shapes"][0]["points"].as_array().unwrap();
        assert!(shown.len() <= READ_POINTS + 1);
        assert_eq!(shown.first().unwrap(), &json!([0.0, 0.0]));
        assert_eq!(shown.last().unwrap(), &json!([499.0, 0.0]));
    }

    #[test]
    fn a_shot_of_all_frames_the_content_and_escapes_text() {
        let mut doc = Doc::default();
        handle(
            &mut doc,
            view(),
            false,
            &json!({"action": "edit", "ops": [{"op": "add", "type": "text", "x": 100, "y": 100, "text": "a<b & c"}]}),
        );
        let r = handle(
            &mut doc,
            view(),
            true,
            &json!({"action": "shot", "area": "all"}),
        );
        let shot = r.shot.expect("a picture");
        assert!(shot.svg.contains("a&lt;b &amp; c"));
        assert!(shot.svg.contains("fill=\"#ffffff\""), "light page");
        assert_eq!(r.json["area"]["x"], json!(76.0));
        assert!(shot.width > 0 && shot.height > 0);
    }

    #[test]
    fn unknown_names_say_what_is_allowed() {
        let mut doc = Doc::default();
        let r = handle(
            &mut doc,
            view(),
            false,
            &json!({"action": "edit", "ops": [{"op": "add", "type": "rect", "x": 0, "y": 0, "w": 1, "h": 1, "color": "magenta"}]}),
        );
        let error = r.json["error"].as_str().unwrap();
        assert!(error.contains("magenta") && error.contains("light_slate"));
    }
}
