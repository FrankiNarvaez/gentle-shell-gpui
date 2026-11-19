//! The real canvas in a headless window, driven by pointer and key events:
//! drawing, shapes, selection, erasing, undo and saving to its file.

use std::path::PathBuf;
use std::time::Duration;

use gentle_core::canvas::{Doc, Kind};
use gpui_kit::{
    Entity, Modifiers, MouseButton, Point, TestAppContext, VisualTestContext, point, px,
};

use super::{CanvasView, Tool};

fn open(cx: &mut TestAppContext) -> (Entity<CanvasView>, &mut VisualTestContext, PathBuf) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        super::bind_keys(cx);
    });
    let dir = std::env::temp_dir().join(format!(
        "gentle-canvas-test-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let path = dir.join("canvas.json");
    let _ = std::fs::remove_dir_all(&dir);
    let file = path.clone();
    let (view, cx) = cx.add_window_view(|window, cx| CanvasView::open(file, window, cx));
    for _ in 0..2 {
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
    }
    cx.update(|window, cx| {
        let focus = view.read(cx).focus.clone();
        window.focus(&focus, cx);
    });
    (view, cx, path)
}

/// Press, move through `points`, release: a hand on a mouse.
fn stroke(cx: &mut VisualTestContext, points: &[Point<gpui_kit::Pixels>]) {
    cx.simulate_mouse_down(points[0], MouseButton::Left, Modifiers::none());
    for p in &points[1..] {
        cx.simulate_mouse_move(*p, Some(MouseButton::Left), Modifiers::none());
        cx.run_until_parked();
    }
    cx.simulate_mouse_up(
        *points.last().unwrap(),
        MouseButton::Left,
        Modifiers::none(),
    );
    cx.run_until_parked();
}

fn kinds(view: &Entity<CanvasView>, cx: &mut VisualTestContext) -> Vec<Kind> {
    cx.update(|_, cx| {
        view.read(cx)
            .doc
            .shapes
            .iter()
            .map(|s| s.kind.clone())
            .collect()
    })
}

fn undo_keys() -> &'static str {
    if cfg!(target_os = "macos") {
        "cmd-z"
    } else {
        "ctrl-z"
    }
}

#[gpui::test]
fn a_loop_drawn_with_the_pen_closes_and_undo_takes_it_back(cx: &mut TestAppContext) {
    let (view, cx, _) = open(cx);
    let circle: Vec<_> = (0..=48)
        .map(|i| {
            let a = i as f32 / 48. * std::f32::consts::TAU;
            point(px(400. + 120. * a.cos()), px(300. + 120. * a.sin()))
        })
        .collect();
    stroke(cx, &circle);
    match kinds(&view, cx).as_slice() {
        [Kind::Ink { points, closed, .. }] => {
            assert_eq!(points.len(), circle.len());
            assert!(closed);
        }
        other => panic!("one ink stroke, got {other:?}"),
    }
    cx.simulate_keystrokes(undo_keys());
    cx.run_until_parked();
    assert!(kinds(&view, cx).is_empty());
}

#[gpui::test]
fn a_rectangle_is_drawn_selected_moved_and_erased(cx: &mut TestAppContext) {
    let (view, cx, _) = open(cx);
    cx.simulate_keystrokes("r");
    stroke(
        cx,
        &[
            point(px(100.), px(100.)),
            point(px(200.), px(150.)),
            point(px(300.), px(200.)),
        ],
    );
    let Some(Kind::Rect { rect, .. }) = kinds(&view, cx).pop() else {
        panic!("a rectangle");
    };
    assert_eq!((rect.x, rect.y, rect.w, rect.h), (100., 100., 200., 100.));
    let (tool, selected) = cx.update(|_, cx| {
        let v = view.read(cx);
        (v.tool, v.selected.len())
    });
    assert_eq!((tool, selected), (Tool::Select, 1));

    // Grab its left edge and drag it 50 px right.
    stroke(
        cx,
        &[
            point(px(100.), px(150.)),
            point(px(130.), px(150.)),
            point(px(150.), px(150.)),
        ],
    );
    let Some(Kind::Rect { rect, .. }) = kinds(&view, cx).pop() else {
        panic!("still a rectangle");
    };
    assert_eq!((rect.x, rect.y), (150., 100.));

    // The eraser across its top edge removes it.
    cx.simulate_keystrokes("e");
    stroke(
        cx,
        &[
            point(px(200.), px(80.)),
            point(px(200.), px(100.)),
            point(px(200.), px(120.)),
        ],
    );
    assert!(kinds(&view, cx).is_empty());
}

#[gpui::test]
fn the_page_is_saved_to_its_file_after_a_change(cx: &mut TestAppContext) {
    let (_view, cx, path) = open(cx);
    stroke(
        cx,
        &[
            point(px(50.), px(50.)),
            point(px(90.), px(70.)),
            point(px(140.), px(60.)),
        ],
    );
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    let doc = Doc::parse(&std::fs::read_to_string(&path).expect("saved")).unwrap();
    assert_eq!(doc.shapes.len(), 1);
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[gpui::test]
fn text_and_notes_take_what_is_typed(cx: &mut TestAppContext) {
    let (view, cx, _) = open(cx);
    cx.simulate_keystrokes("t");
    cx.simulate_click(point(px(200.), px(200.)), Modifiers::none());
    cx.run_until_parked();
    assert!(
        cx.update(|_, cx| view.read(cx).editing.is_some()),
        "the text field stays open after the click"
    );
    cx.simulate_input("hola");
    cx.run_until_parked();
    // Clicking the page elsewhere ends typing.
    cx.simulate_click(point(px(600.), px(500.)), Modifiers::none());
    cx.run_until_parked();
    match kinds(&view, cx).as_slice() {
        [Kind::Text { text, .. }] => assert_eq!(text, "hola"),
        other => panic!("one text, got {other:?}"),
    }

    cx.simulate_keystrokes("n");
    cx.simulate_click(point(px(400.), px(400.)), Modifiers::none());
    cx.run_until_parked();
    cx.simulate_input("nota");
    cx.run_until_parked();
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    cx.simulate_click(point(px(900.), px(700.)), Modifiers::none());
    cx.run_until_parked();
    let notes: Vec<String> = kinds(&view, cx)
        .into_iter()
        .filter_map(|k| match k {
            Kind::Note { text, .. } => Some(text),
            _ => None,
        })
        .collect();
    assert_eq!(notes, ["nota"]);
}

/// Drops a request where the agent's extension would and waits for the
/// canvas to answer it.
fn ask(
    cx: &mut VisualTestContext,
    canvas: &std::path::Path,
    id: &str,
    request: serde_json::Value,
) -> serde_json::Value {
    let inbox = canvas.with_extension("agent");
    std::fs::create_dir_all(&inbox).unwrap();
    std::fs::write(inbox.join(format!("{id}.req.json")), request.to_string()).unwrap();
    cx.executor().advance_clock(Duration::from_millis(500));
    cx.run_until_parked();
    let answer = std::fs::read_to_string(inbox.join(format!("{id}.res.json"))).expect("answered");
    serde_json::from_str(&answer).unwrap()
}

#[gpui::test]
fn the_paired_agent_draws_sees_and_its_edit_undoes_in_one_step(cx: &mut TestAppContext) {
    let (view, cx, path) = open(cx);
    let answer = ask(
        cx,
        &path,
        "1",
        serde_json::json!({"action": "edit", "ops": [{"op": "add", "shapes": [
            {"type": "rect", "x": 40, "y": 40, "w": 160, "h": 80, "text": "API"},
            {"type": "arrow", "x1": 200, "y1": 80, "x2": 320, "y2": 80}
        ]}]}),
    );
    assert_eq!(answer["added"], serde_json::json!([1, 2]), "{answer}");
    assert!(matches!(
        kinds(&view, cx).first(),
        Some(Kind::Rect { label, .. }) if label == "API"
    ));

    let read = ask(cx, &path, "2", serde_json::json!({"action": "read"}));
    assert_eq!(read["shapes"][0]["text"], "API");

    let shot = ask(
        cx,
        &path,
        "3",
        serde_json::json!({"action": "shot", "area": "all"}),
    );
    let png = std::fs::read(shot["image"].as_str().expect("a picture")).unwrap();
    assert_eq!(&png[1..4], b"PNG");

    cx.simulate_keystrokes(undo_keys());
    cx.run_until_parked();
    assert!(kinds(&view, cx).is_empty());
}
