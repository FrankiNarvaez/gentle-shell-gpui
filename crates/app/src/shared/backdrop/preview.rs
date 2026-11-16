//! A theme's scene drawn offscreen, so the agent that wrote it can look at
//! what it made instead of guessing: four moments of the scene in a
//! 1280 × 800 window, with the sky, image and veil as the app draws them,
//! as one PNG, and a short report of what a frame costs and how often it
//! will be drawn.
//!
//! A frame's commands (`kit.js`) are written as SVG and rasterized with
//! resvg, one shape per command, the way [`super::paint_frame`] paints them.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use gpui_kit::ImageFormat;
use resvg::{tiny_skia, usvg};

use super::model::image_rect;
use super::scene::{Engine, STRIDE, pace};
use super::subpaths;
use crate::shared::themes::Theme;

/// The window a preview draws, in logical pixels.
const WINDOW: (f32, f32) = (1280., 800.);
/// The scene times shown, left to right, top to bottom.
const MOMENTS: [f64; 4] = [2., 8., 20., 45.];
/// Consecutive frames timed for the cost line, a second at 24 fps.
const TIMED: usize = 24;
/// Each moment is shown at this scale in the grid.
const TILE_SCALE: f32 = 0.5;
/// The line between moments, in grid pixels.
const GAP: u32 = 4;
/// More shapes than this in a frame is called out as heavy.
const HEAVY: usize = 4000;

/// What the preview of one theme came to.
pub struct Look {
    /// The 2 × 2 grid as PNG, unless the scene failed before drawing.
    pub png: Option<Vec<u8>>,
    pub report: String,
}

/// Draw `theme`'s scene (or its image's camera), or `None` when it has neither.
pub fn render(theme: &Theme) -> Option<Look> {
    let backdrop = theme.background.as_ref()?;
    let natural = backdrop.image.as_ref().and(backdrop.image_size);
    let (name, code) = match &backdrop.scene {
        Some(scene) => (scene.name.as_str(), scene.code.clone()),
        None if backdrop.camera.is_some() && natural.is_some() => {
            ("none (the image's camera only)", super::STILL.clone())
        }
        None => return None,
    };
    let mut report = format!(
        "Theme: {}\nScene: {name}\nRendered: {}\n",
        theme.name,
        utc_now()
    );
    if let Some((w, h)) = natural {
        let _ = writeln!(
            report,
            "Image: {w} x {h} px, {}.",
            match &backdrop.camera {
                Some(c) => format!(
                    "camera zoom {} pan {} period {} s focus [{}, {}]",
                    c.zoom, c.pan, c.period, c.focus.0, c.focus.1
                ),
                None => "no camera".to_string(),
            }
        );
    }
    let colors = super::model::scene_colors(theme);
    let engine = match Engine::new(&code, &colors, backdrop.cell) {
        Ok(engine) => engine,
        Err(error) => {
            let _ = writeln!(report, "\nThe scene failed to load: {error}");
            return Some(Look { png: None, report });
        }
    };
    let mut tiles = Vec::new();
    let mut busiest = (0, 0);
    let mut empty = Vec::new();
    for t in MOMENTS {
        let image = natural.map(|n| image_rect(WINDOW, n, backdrop.camera.as_ref(), t));
        let (commands, points) = match engine.frame(WINDOW, t, image) {
            Ok(frame) => frame,
            Err(error) => {
                let _ = writeln!(report, "\nThe scene failed at t = {t} s: {error}");
                return Some(Look { png: None, report });
            }
        };
        let shapes = commands.len() / STRIDE;
        if shapes == 0 && backdrop.scene.is_some() {
            empty.push(t);
        }
        busiest = busiest.max((shapes, points.len() / 2));
        let svg = frame_svg(theme, &commands, &points, image);
        match rasterize(&svg) {
            Ok(tile) => tiles.push(tile),
            Err(error) => {
                let _ = writeln!(report, "\nThe preview couldn't be drawn: {error}");
                return Some(Look { png: None, report });
            }
        }
    }
    let costs = timed(&engine);
    let average = costs.iter().sum::<Duration>() / costs.len().max(1) as u32;
    let worst = costs.iter().max().copied().unwrap_or_default();
    let interval = pace(average, Duration::ZERO);
    let fps = 1000. / interval.as_millis().max(1) as f64;
    let _ = writeln!(
        report,
        "Moments: t = 2, 8, 20 and 45 s, left to right and top to bottom, each a 1280 x 800 window shown at half size.\n\
         Busiest frame: {} shapes, {} path points.\n\
         Frame cost: {:.1} ms on average, {:.1} ms at worst ({TIMED} frames).\n\
         Runs at: {fps:.0} fps (24 is the most; frames of 3 ms or less keep it).",
        busiest.0,
        busiest.1,
        average.as_secs_f64() * 1000.,
        worst.as_secs_f64() * 1000.,
    );
    let mut notes = Vec::new();
    if !empty.is_empty() {
        notes.push(format!(
            "Nothing is drawn at t = {}: the scene looks empty then.",
            empty
                .iter()
                .map(|t| format!("{t} s"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if busiest.0 > HEAVY {
        notes.push(format!(
            "{} shapes in one frame is heavy; aim for a few hundred to about 2000.",
            busiest.0
        ));
    }
    if fps < 23. {
        notes.push(format!(
            "At {fps:.0} fps motion looks choppy: precompute what doesn't change and draw fewer shapes."
        ));
    }
    for note in &notes {
        let _ = writeln!(report, "Note: {note}");
    }
    let png = grid(&tiles).and_then(|grid| grid.encode_png().ok());
    Some(Look { png, report })
}

/// The folder previews are written to, inside the app's themes folder.
pub const FOLDER: &str = "previews";

/// Keep `<dir>/previews/<stem>.png` and `.txt` current for every app theme
/// in `themes` that has a scene. A theme is drawn again when it differs from
/// the one drawn last (`drawn`, kept by the caller across calls) or, for
/// one not drawn since the app started, when its files are newer than its
/// preview. Runs on a background thread: a preview takes about a second.
pub fn refresh(themes: &[Theme], dir: &std::path::Path, drawn: &mut HashMap<String, Theme>) {
    let out = dir.join(FOLDER);
    for theme in themes {
        let Some(file) = theme.file.as_deref() else {
            continue;
        };
        let Some(backdrop) = theme.background.as_ref() else {
            continue;
        };
        // The camera alone (no scene) is previewed too.
        let moving_image = backdrop.camera.is_some() && backdrop.image_size.is_some();
        if backdrop.scene.is_none() && !moving_image {
            continue;
        }
        if file.parent() != Some(dir) || drawn.get(&theme.name) == Some(theme) {
            continue;
        }
        let stem = super::model::file_stem(&theme.name);
        let (png, txt) = (
            out.join(format!("{stem}.png")),
            out.join(format!("{stem}.txt")),
        );
        let modified =
            |path: &std::path::Path| std::fs::metadata(path).and_then(|m| m.modified()).ok();
        let scene_file = backdrop
            .scene
            .as_ref()
            .map(|s| dir.join(format!("{}.js", s.name)));
        let newest_input = std::iter::once(file.to_path_buf())
            .chain(scene_file)
            .filter_map(|p| modified(&p))
            .max();
        let current = !drawn.contains_key(&theme.name)
            && modified(&txt).is_some_and(|at| newest_input.is_none_or(|input| at >= input));
        if !current {
            let Some(look) = render(theme) else { continue };
            if std::fs::create_dir_all(&out).is_err() {
                return;
            }
            match &look.png {
                Some(bytes) => {
                    let _ = std::fs::write(&png, bytes);
                }
                None => {
                    let _ = std::fs::remove_file(&png);
                }
            }
            let _ = std::fs::write(&txt, &look.report);
        }
        drawn.insert(theme.name.clone(), theme.clone());
    }
}

/// Time `TIMED` consecutive frames from t = 20 s.
fn timed(engine: &Engine) -> Vec<Duration> {
    (0..TIMED)
        .filter_map(|i| {
            let started = Instant::now();
            engine.frame(WINDOW, 20. + i as f64 / 24., None).ok()?;
            Some(started.elapsed())
        })
        .collect()
}

/// The system fonts, loaded once: scenes draw glyphs in the app's monospace font.
static FONTS: LazyLock<Arc<usvg::fontdb::Database>> = LazyLock::new(|| {
    let mut db = usvg::fontdb::Database::new();
    db.load_system_fonts();
    Arc::new(db)
});

fn rasterize(svg: &str) -> Result<tiny_skia::Pixmap, String> {
    let options = usvg::Options {
        fontdb: FONTS.clone(),
        font_family: crate::shared::ui::MONO.to_string(),
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_str(svg, &options).map_err(|e| e.to_string())?;
    let (w, h) = (
        (WINDOW.0 * TILE_SCALE) as u32,
        (WINDOW.1 * TILE_SCALE) as u32,
    );
    let mut pixmap = tiny_skia::Pixmap::new(w, h).ok_or("no pixmap")?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(TILE_SCALE, TILE_SCALE),
        &mut pixmap.as_mut(),
    );
    Ok(pixmap)
}

/// The four moments as a 2 × 2 grid.
fn grid(tiles: &[tiny_skia::Pixmap]) -> Option<tiny_skia::Pixmap> {
    let first = tiles.first()?;
    let (w, h) = (first.width(), first.height());
    let mut grid = tiny_skia::Pixmap::new(w * 2 + GAP, h * 2 + GAP)?;
    grid.fill(tiny_skia::Color::from_rgba8(12, 12, 14, 255));
    for (i, tile) in tiles.iter().enumerate() {
        let (col, row) = ((i % 2) as u32, (i / 2) as u32);
        grid.draw_pixmap(
            (col * (w + GAP)) as i32,
            (row * (h + GAP)) as i32,
            tile.as_ref(),
            &tiny_skia::PixmapPaint::default(),
            tiny_skia::Transform::identity(),
            None,
        );
    }
    Some(grid)
}

fn hex(color: f64) -> String {
    format!("#{:06x}", color as u32 & 0xffffff)
}

fn opacity(alpha: f64) -> f64 {
    alpha.clamp(0., 1.)
}

/// An SVG linear gradient's end points in the shape's bounding box for a
/// CSS-style `angle` (180 = top to bottom, 90 = left to right).
fn direction(angle: f64) -> (f64, f64, f64, f64) {
    let (sin, cos) = angle.to_radians().sin_cos();
    (
        0.5 - 0.5 * sin,
        0.5 + 0.5 * cos,
        0.5 + 0.5 * sin,
        0.5 - 0.5 * cos,
    )
}

/// Gradient definitions and shapes for one frame's SVG.
#[derive(Default)]
struct Svg {
    defs: String,
    body: String,
    ids: usize,
    /// One radial gradient per glow color and strength, shared by every glow.
    glows: HashMap<(u32, u32), usize>,
}

impl Svg {
    fn id(&mut self) -> usize {
        self.ids += 1;
        self.ids
    }

    fn linear(&mut self, angle: f64, stops: &[(f64, f64, f64)]) -> usize {
        let id = self.id();
        let (x1, y1, x2, y2) = direction(angle);
        let _ = write!(
            self.defs,
            r#"<linearGradient id="g{id}" x1="{x1}" y1="{y1}" x2="{x2}" y2="{y2}">"#
        );
        for (at, color, alpha) in stops {
            let _ = write!(
                self.defs,
                r#"<stop offset="{at}" stop-color="{}" stop-opacity="{}"/>"#,
                hex(*color),
                opacity(*alpha)
            );
        }
        self.defs.push_str("</linearGradient>");
        id
    }

    /// The glow's falloff: the app stacks 28 faint discs, densest at the
    /// middle, so the alpha at a radius is what the discs over it add up to.
    fn glow(&mut self, color: f64, alpha: f64) -> usize {
        const RINGS: f64 = 28.;
        let key = (color as u32, (alpha * 1000.) as u32);
        if let Some(id) = self.glows.get(&key) {
            return *id;
        }
        let id = self.id();
        let each = alpha * 0.085;
        let _ = write!(self.defs, r#"<radialGradient id="g{id}">"#);
        for step in 0..=8 {
            let at = f64::from(step) / 8.;
            let over = (RINGS * (1. - at)).ceil();
            let alpha = 1. - (1. - each).powf(over);
            let _ = write!(
                self.defs,
                r#"<stop offset="{at}" stop-color="{}" stop-opacity="{}"/>"#,
                hex(color),
                opacity(alpha)
            );
        }
        self.defs.push_str("</radialGradient>");
        self.glows.insert(key, id);
        id
    }
}

fn escape(ch: char) -> String {
    match ch {
        '&' => "&amp;".into(),
        '<' => "&lt;".into(),
        '>' => "&gt;".into(),
        '"' => "&quot;".into(),
        other => other.to_string(),
    }
}

/// One frame as the window shows it: background, sky, image (at `image`,
/// `[x, y, w, h]`, when its size is known), the scene's commands, then the veil.
fn frame_svg(theme: &Theme, commands: &[f64], points: &[f64], image: Option<[f32; 4]>) -> String {
    let (w, h) = WINDOW;
    let mut svg = Svg::default();
    let _ = write!(
        svg.body,
        r#"<rect width="{w}" height="{h}" fill="{}"/>"#,
        hex(f64::from(theme.bg))
    );
    if let Some(backdrop) = &theme.background {
        let sky = &backdrop.sky;
        if !sky.is_empty() {
            let last = (sky.len() - 1).max(1) as f64;
            let stops: Vec<_> = sky
                .iter()
                .enumerate()
                .map(|(i, c)| (i as f64 / last, f64::from(*c), 1.))
                .collect();
            let id = svg.linear(180., &stops);
            let _ = write!(
                svg.body,
                r#"<rect width="{w}" height="{h}" fill="url(#g{id})"/>"#
            );
        }
        if let Some(picture) = &backdrop.image
            && let Some(mime) = mime(picture.format)
        {
            let data = base64::engine::general_purpose::STANDARD.encode(&picture.bytes);
            let place = match image {
                Some([x, y, iw, ih]) => format!(
                    r#"x="{x}" y="{y}" width="{iw}" height="{ih}" preserveAspectRatio="none""#
                ),
                None => format!(r#"width="{w}" height="{h}" preserveAspectRatio="xMidYMid slice""#),
            };
            let _ = write!(
                svg.body,
                r#"<image {place} href="data:{mime};base64,{data}"/>"#
            );
            if backdrop.dim > 0. {
                let _ = write!(
                    svg.body,
                    r#"<rect width="{w}" height="{h}" fill="{}" fill-opacity="{}"/>"#,
                    hex(f64::from(theme.bg)),
                    backdrop.dim
                );
            }
        }
    }
    for c in commands.as_chunks::<STRIDE>().0 {
        command(&mut svg, c, points);
    }
    if let Some(veil) = theme.background.as_ref().map(|b| b.veil)
        && veil > 0.
    {
        let stops: Vec<_> = (0..=32)
            .map(|i| {
                let u = i as f32 / 32.;
                (
                    f64::from(u),
                    f64::from(theme.bg),
                    super::veil_alpha(veil, u),
                )
            })
            .collect();
        let id = svg.linear(90., &stops);
        let _ = write!(
            svg.body,
            r#"<rect width="{w}" height="{h}" fill="url(#g{id})"/>"#
        );
    }
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}"><defs>{}</defs>{}</svg>"#,
        svg.defs, svg.body
    )
}

fn mime(format: ImageFormat) -> Option<&'static str> {
    Some(match format {
        ImageFormat::Png => "image/png",
        ImageFormat::Jpeg => "image/jpeg",
        ImageFormat::Webp => "image/webp",
        ImageFormat::Gif => "image/gif",
        _ => return None,
    })
}

/// One drawing command as SVG, matching [`super::paint_frame`].
fn command(svg: &mut Svg, c: &[f64; STRIDE], points: &[f64]) {
    let body = &mut svg.body;
    match c[0] as u32 {
        // rect: x, y, w, h, radius, color, alpha
        1 if c[7] >= 0.005 => {
            let _ = write!(
                body,
                r#"<rect x="{}" y="{}" width="{}" height="{}" rx="{}" fill="{}" fill-opacity="{}"/>"#,
                c[1],
                c[2],
                c[3].max(0.),
                c[4].max(0.),
                c[5].max(0.),
                hex(c[6]),
                opacity(c[7])
            );
        }
        // ellipse: drawn as the app draws it, a box rounded by the smaller radius
        2 if c[6] >= 0.005 => {
            let _ = write!(
                body,
                r#"<rect x="{}" y="{}" width="{}" height="{}" rx="{}" fill="{}" fill-opacity="{}"/>"#,
                c[1] - c[3],
                c[2] - c[4],
                (c[3] * 2.).max(0.),
                (c[4] * 2.).max(0.),
                c[3].min(c[4]).max(0.),
                hex(c[5]),
                opacity(c[6])
            );
        }
        // glyph: x (center), y (middle), size, code point, color, alpha, bold
        3 if c[6] >= 0.01 => {
            let Some(ch) = char::from_u32(c[4] as u32) else {
                return;
            };
            let _ = write!(
                body,
                r#"<text x="{}" y="{}" font-size="{}" font-weight="{}" text-anchor="middle" dominant-baseline="central" fill="{}" fill-opacity="{}">{}</text>"#,
                c[1],
                c[2],
                c[3].clamp(4., 200.),
                if c[7] != 0. { "bold" } else { "normal" },
                hex(c[5]),
                opacity(c[6]),
                escape(ch)
            );
        }
        // glow: x, y, r, color, alpha
        4 => {
            let id = svg.glow(c[4], c[5]);
            let _ = write!(
                svg.body,
                r#"<circle cx="{}" cy="{}" r="{}" fill="url(#g{id})"/>"#,
                c[1],
                c[2],
                c[3].max(0.)
            );
        }
        // gradient: x, y, w, h, angle, from, to, from alpha, to alpha
        5 => {
            let id = svg.linear(c[5], &[(0., c[6], c[8]), (1., c[7], c[9])]);
            let _ = write!(
                svg.body,
                r#"<rect x="{}" y="{}" width="{}" height="{}" fill="url(#g{id})"/>"#,
                c[1],
                c[2],
                c[3].max(0.),
                c[4].max(0.)
            );
        }
        // line: x1, y1, x2, y2, width, color, alpha
        6 => {
            let _ = write!(
                body,
                r#"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke-width="{}" stroke="{}" stroke-opacity="{}"/>"#,
                c[1],
                c[2],
                c[3],
                c[4],
                c[5].max(0.5),
                hex(c[6]),
                opacity(c[7])
            );
        }
        // path (fill or stroke) and path gradient
        7 | 8 => {
            let gradient = c[0] as u32 == 8;
            let width = if gradient { 0. } else { c[3] };
            let mut d = String::new();
            for (run, closed) in subpaths(points, c[1], c[2]) {
                let [first, rest @ ..] = run else { continue };
                if rest.is_empty() {
                    continue;
                }
                let _ = write!(d, "M{} {}", first[0], first[1]);
                for p in rest {
                    let _ = write!(d, "L{} {}", p[0], p[1]);
                }
                if closed || width == 0. {
                    d.push('Z');
                }
            }
            if d.is_empty() {
                return;
            }
            let paint = if gradient {
                let id = svg.linear(c[3], &[(0., c[4], c[6]), (1., c[5], c[7])]);
                format!(r#"fill="url(#g{id})" fill-rule="evenodd""#)
            } else if width > 0. {
                format!(
                    r#"fill="none" stroke="{}" stroke-opacity="{}" stroke-width="{width}""#,
                    hex(c[4]),
                    opacity(c[5])
                )
            } else {
                format!(
                    r#"fill="{}" fill-opacity="{}" fill-rule="evenodd""#,
                    hex(c[4]),
                    opacity(c[5])
                )
            };
            let _ = write!(svg.body, r#"<path d="{d}" {paint}/>"#);
        }
        _ => {}
    }
}

/// Now as `YYYY-MM-DD HH:MM:SS UTC`.
fn utc_now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let (days, rest) = ((secs / 86_400) as i64, secs % 86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02} UTC",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::themes::{Source, parse_pi_theme};

    fn theme(scene: &str) -> Theme {
        let dir = std::env::temp_dir().join(format!(
            "gsg-preview-{}-{}",
            std::process::id(),
            scene.len()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("s.js"), scene).unwrap();
        let file = dir.join("t.json");
        let text = r##"{"name": "Probe", "bg": "#101018", "text": "#f0f0f0", "accent": "#e070a0",
            "background": {"scene": "s.js", "sky": ["#101830", "#301020"], "veil": 0.5}}"##;
        std::fs::write(&file, text).unwrap();
        parse_pi_theme(text, Source::App, Some(&file)).unwrap()
    }

    fn pixel(png: &[u8], x: u32, y: u32) -> [u8; 4] {
        let image = image::load_from_memory(png).unwrap().to_rgba8();
        image.get_pixel(x, y).0
    }

    #[test]
    fn draws_the_scene_four_times_as_the_window_would() {
        // A big accent disc in the top-left corner of every moment.
        let look = render(&theme(
            "SCENES.p = { draw(g) { g.circle(100, 100, 80, '#ff0000'); \
             path().rect(1000, 600, 200, 150).fill('#00ff00'); } }",
        ))
        .unwrap();
        let png = look.png.expect("a preview");
        let tile = (WINDOW.0 * TILE_SCALE) as u32;
        // The disc's middle, in each of the four tiles, is red.
        for (col, row) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            let [r, g, b, _] = pixel(&png, col * (tile + GAP) + 50, row * (400 + GAP) + 50);
            assert!(r > 200 && g < 60 && b < 60, "tile {col},{row}: {r},{g},{b}");
        }
        // The path's rectangle shows too.
        let [r, g, _, _] = pixel(&png, 550, 335);
        assert!(g > 200 && r < 60, "{r},{g}");
        assert!(look.report.contains("Runs at: 24 fps"), "{}", look.report);
    }

    #[test]
    fn a_broken_scene_reports_why_without_a_picture() {
        let look = render(&theme("SCENES.p = { draw(g) { g.nope(); } }")).unwrap();
        assert!(look.png.is_none());
        assert!(look.report.contains("failed at t = 2 s"), "{}", look.report);
    }
}
