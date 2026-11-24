use super::freehand::{self, StrokeOptions};
use super::geom::{self, Rect};
use super::*;

fn shape(kind: Kind) -> Shape {
    Shape {
        id: 1,
        kind,
        style: Style::default(),
    }
}

#[test]
fn monoline_outline_stays_half_a_size_from_its_line() {
    let points: Vec<P> = (0..=50).map(|i| [i as f32 * 4., 100.]).collect();
    let o = StrokeOptions {
        size: 10.,
        thinning: 0.,
        last: true,
        ..StrokeOptions::default()
    };
    let outline = freehand::outline(&points, &o);
    assert!(outline.len() > 10);
    for p in &outline {
        assert!(p[0].is_finite() && p[1].is_finite());
        // Within the round caps' reach of the line.
        assert!(geom::polyline_distance(*p, &points) <= 5.01, "{p:?}");
    }
    let ys: Vec<f32> = outline.iter().map(|p| p[1]).collect();
    let (lo, hi) = ys
        .iter()
        .fold((f32::MAX, f32::MIN), |(a, b), &y| (a.min(y), b.max(y)));
    assert!((hi - lo - 10.).abs() < 0.5, "width {}", hi - lo);
}

#[test]
fn a_click_draws_a_dot_and_nothing_draws_nothing() {
    let o = StrokeOptions {
        last: true,
        ..StrokeOptions::default()
    };
    let dot = freehand::outline(&[[5., 5.]], &o);
    assert!(dot.len() >= 12);
    for p in &dot {
        assert!((p[0] - 5.).hypot(p[1] - 5.) < o.size);
    }
    assert!(freehand::outline(&[], &o).is_empty());
}

#[test]
fn simulated_pressure_thins_fast_strokes() {
    let slow: Vec<P> = (0..=200).map(|i| [i as f32 * 0.5, 0.]).collect();
    let fast: Vec<P> = (0..=10).map(|i| [i as f32 * 10., 0.]).collect();
    let width = |pts: &[P]| {
        let o = stroke_options(Pen::Pen, false, 4., true);
        let out = freehand::outline(pts, &o);
        // Width at the middle of the stroke.
        out.iter()
            .filter(|p| (p[0] - 50.).abs() < 5.)
            .map(|p| p[1].abs())
            .fold(0., f32::max)
    };
    assert!(width(&fast) < width(&slow));
}

#[test]
fn hatch_lines_stay_inside_the_shape() {
    let square = Rect {
        x: 0.,
        y: 0.,
        w: 100.,
        h: 100.,
    };
    let lines = geom::hatch(&square.corners(), 10.);
    assert!(lines.len() > 10);
    for (a, b) in lines {
        for p in [a, b] {
            assert!(square.expand(0.01).contains(p), "{p:?}");
        }
    }
}

#[test]
fn camera_zoom_keeps_the_point_under_the_pointer() {
    let mut c = Camera {
        x: 30.,
        y: -20.,
        zoom: 1.5,
    };
    let screen = [200., 120.];
    let before = c.to_page(screen);
    c.zoom_at(2., screen);
    let after = c.to_page(screen);
    assert!((before[0] - after[0]).abs() < 1e-3 && (before[1] - after[1]).abs() < 1e-3);
    let back = c.to_screen(after);
    assert!((back[0] - screen[0]).abs() < 1e-3);
    c.zoom_at(1000., screen);
    assert_eq!(c.zoom, MAX_ZOOM);
}

#[test]
fn fit_centers_content_and_never_zooms_past_100() {
    let content = Rect {
        x: 100.,
        y: 100.,
        w: 50.,
        h: 50.,
    };
    let c = Camera::fitting(&content, 800., 600., 40.);
    assert_eq!(c.zoom, 1.);
    let center = c.to_screen(content.center());
    assert!((center[0] - 400.).abs() < 1e-3 && (center[1] - 300.).abs() < 1e-3);
}

#[test]
fn an_empty_rectangle_is_picked_by_its_outline_only() {
    let rect = Rect {
        x: 0.,
        y: 0.,
        w: 100.,
        h: 60.,
    };
    let mut s = shape(Kind::Rect {
        rect,
        label: String::new(),
    });
    assert!(s.hit([0., 30.], 2.));
    assert!(!s.hit([50., 30.], 2.));
    s.style.fill = Fill::Solid;
    assert!(s.hit([50., 30.], 2.));
    let e = shape(Kind::Ellipse {
        rect,
        label: String::new(),
    });
    assert!(e.hit([100., 30.], 2.));
    assert!(!e.hit([50., 30.], 2.));
}

#[test]
fn lasso_takes_shapes_wholly_inside() {
    let ink = shape(Kind::Ink {
        points: vec![[10., 10.], [20., 20.], [30., 10.]],
        highlighter: false,
        closed: false,
    });
    let around = [[0., 0.], [50., 0.], [50., 50.], [0., 50.]];
    let half = [[0., 0.], [25., 0.], [25., 50.], [0., 50.]];
    assert!(ink.inside_loop(&around));
    assert!(!ink.inside_loop(&half));
}

#[test]
fn resizing_stretches_points_and_scales_text() {
    let mut ink = shape(Kind::Ink {
        points: vec![[0., 0.], [10., 10.]],
        highlighter: false,
        closed: false,
    });
    let from = Rect {
        x: 0.,
        y: 0.,
        w: 10.,
        h: 10.,
    };
    let to = Rect {
        x: 100.,
        y: 0.,
        w: 20.,
        h: 40.,
    };
    ink.resize(&from, &to);
    let Kind::Ink { points, .. } = &ink.kind else {
        unreachable!()
    };
    assert_eq!(points, &vec![[100., 0.], [120., 40.]]);
    let mut text = shape(Kind::Text {
        at: [0., 0.],
        text: "hi".into(),
        scale: 1.,
    });
    text.resize(&from, &to);
    assert_eq!(text.font_size(), Style::default().font_size() * 4.);
}

#[test]
fn undo_and_redo_walk_the_snapshots() {
    let mut doc = Doc::default();
    let mut history = History::default();
    history.record(doc.shapes.clone());
    doc.paste(
        vec![shape(Kind::Line {
            a: [0., 0.],
            b: [1., 1.],
        })],
        [0., 0.],
    );
    assert!(history.undo(&mut doc.shapes));
    assert!(doc.shapes.is_empty());
    assert!(history.redo(&mut doc.shapes));
    assert_eq!(doc.shapes.len(), 1);
    assert!(!history.redo(&mut doc.shapes));
    history.record(doc.shapes.clone());
    assert!(!history.can_redo());
}

#[test]
fn copied_shapes_paste_back_and_plain_text_does_not() {
    let s = shape(Kind::Note {
        rect: Rect::default(),
        text: "todo".into(),
    });
    let text = to_clipboard(std::slice::from_ref(&s));
    assert_eq!(from_clipboard(&text), Some(vec![s]));
    assert_eq!(from_clipboard("[]"), None);
}

#[test]
fn pasted_shapes_get_fresh_ids_on_top() {
    let mut doc = Doc::default();
    let a = doc.paste(
        vec![shape(Kind::Line {
            a: [0., 0.],
            b: [1., 0.],
        })],
        [0., 0.],
    );
    let b = doc.duplicate(&a, [10., 10.]);
    assert_ne!(a, b);
    let Kind::Line { a: start, .. } = doc.get(b[0]).unwrap().kind else {
        unreachable!()
    };
    assert_eq!(start, [10., 10.]);
}

#[test]
fn a_file_without_optional_fields_still_loads() {
    let json = r#"{"shapes":[{"id":3,"type":"text","at":[1,2],"text":"x",
        "style":{"color":"light_slate","opacity":50,"size":2}}]}"#;
    let doc = Doc::parse(json).unwrap();
    let s = &doc.shapes[0];
    assert_eq!(s.style.color, Color::LightSlate);
    assert_eq!(s.style.pen, Pen::Pen);
    assert_eq!(s.font_size(), FONT_SIZES[2]);
    assert_eq!(Doc::parse(&doc.to_json()).unwrap().shapes, doc.shapes);
}

#[test]
fn only_strokes_that_come_back_close_into_loops() {
    let circle: Vec<P> = (0..=40)
        .map(|i| {
            let a = i as f32 / 40. * std::f32::consts::TAU;
            [50. * a.cos(), 50. * a.sin()]
        })
        .collect();
    assert!(geom::closes_loop(&circle, 12.));
    let arc = &circle[..20];
    assert!(!geom::closes_loop(arc, 12.));
}

#[test]
fn only_shapes_with_an_inside_take_a_fill_and_only_pen_ink_a_pen() {
    let ink = |highlighter, closed| Kind::Ink {
        points: vec![[0., 0.], [10., 10.]],
        highlighter,
        closed,
    };
    let rect = Rect {
        x: 0.,
        y: 0.,
        w: 10.,
        h: 10.,
    };
    let fills = |k: Kind| (k.controls().fill, k.controls().pen);
    assert_eq!(fills(ink(false, true)), (true, true));
    assert_eq!(fills(ink(false, false)), (false, true));
    // The highlighter draws the same whatever the pen type.
    assert_eq!(fills(ink(true, true)), (false, false));
    let label = String::new;
    assert_eq!(
        fills(Kind::Rect {
            rect,
            label: label()
        }),
        (true, false)
    );
    assert_eq!(
        fills(Kind::Ellipse {
            rect,
            label: label()
        }),
        (true, false)
    );
    assert_eq!(
        fills(Kind::Line {
            a: [0., 0.],
            b: [1., 1.]
        }),
        (false, false)
    );
    assert_eq!(
        fills(Kind::Arrow {
            a: [0., 0.],
            b: [1., 1.]
        }),
        (false, false)
    );
    let note = Kind::Note {
        rect,
        text: label(),
    };
    assert_eq!(note.controls(), Controls::MARK);
    let image = Kind::Image {
        rect,
        path: label(),
    };
    assert_eq!(
        image.controls(),
        Controls {
            opacity: true,
            ..Controls::default()
        }
    );
}

#[test]
fn a_style_adopts_only_what_its_shape_shows() {
    let from = Style {
        color: Color::Red,
        fill_color: Some(Color::Blue),
        fill: Fill::Pattern,
        opacity: 50,
        size: 4,
        pen: Pen::Fountain,
    };
    let mut line = Style::default();
    line.adopt(&from, Controls::MARK);
    assert_eq!(
        line,
        Style {
            color: Color::Red,
            opacity: 50,
            size: 4,
            ..Style::default()
        }
    );
    let mut image = Style::default();
    image.adopt(&from, Controls::IMAGE);
    assert_eq!(
        image,
        Style {
            opacity: 50,
            ..Style::default()
        }
    );
    let mut rect = Style::default();
    rect.adopt(&from, Controls::GEO);
    assert_eq!(
        rect,
        Style {
            pen: Pen::Pen,
            ..from
        }
    );
}
