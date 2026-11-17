//! The panel kinds: the real [`PanelView`] in a headless window, driven by
//! simulated pointer and keyboard input, checking the `/gentle-ui:event`
//! command Send hands the conversation.

use gentle_core::genui::{Outcome, Snapshot};
// Not a glob: GPUI exports a `test` attribute that would shadow `#[test]`.
use gpui_kit::{
    AppContext as _, Bounds, Context, Entity, IntoElement, Modifiers, MouseButton, MouseDownEvent,
    MouseUpEvent, ParentElement as _, Pixels, Point, Render, Styled as _, Subscription,
    TestAppContext, VisualTestContext, Window, div, point, px,
};
use serde_json::{Value, json};

use super::super::diagram::{DiagramEvent, SendState};
use super::super::panel::PanelView;
use super::{Seen, event_json, ops};

/// Stands in for the conversation: shows one surface and keeps what it asks for.
pub(super) struct Host {
    view: Entity<PanelView>,
    seen: Vec<Seen>,
    _subscription: Subscription,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .p_4()
            .child(div().w(px(900.)).h(px(460.)).child(self.view.clone()))
    }
}

pub(super) fn open<'a>(
    cx: &'a mut TestAppContext,
    kind: &str,
    spec: Value,
) -> (Entity<Host>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        // Deterministic frames: no tweens or glides.
        cx.set_reduce_motion(true);
    });
    let snapshot = Snapshot {
        id: format!("ui-{kind}"),
        version: 1,
        kind: kind.into(),
        title: format!("Test {kind}"),
        spec,
        closed: false,
    };
    let (host, cx) = cx.add_window_view(|_, cx| {
        let view = cx.new(|cx| PanelView::new(&snapshot, cx));
        let subscription = cx.subscribe(&view, |host: &mut Host, _, event: &DiagramEvent, _| {
            host.seen.push(Seen::from(event))
        });
        Host {
            view,
            seen: Vec::new(),
            _subscription: subscription,
        }
    });
    cx.run_until_parked();
    (host, cx)
}

pub(super) fn bounds(cx: &mut VisualTestContext, selector: &'static str) -> Bounds<Pixels> {
    cx.run_until_parked();
    cx.debug_bounds(selector)
        .unwrap_or_else(|| panic!("nothing drawn as {selector}"))
}

pub(super) fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let at = bounds(cx, selector).center();
    cx.simulate_click(at, Modifiers::none());
    cx.run_until_parked();
}

pub(super) fn double_click(cx: &mut VisualTestContext, selector: &'static str) {
    let position = bounds(cx, selector).center();
    for click_count in [1, 2] {
        cx.simulate_event(MouseDownEvent {
            button: MouseButton::Left,
            position,
            modifiers: Modifiers::none(),
            click_count,
            first_mouse: false,
        });
        cx.simulate_event(MouseUpEvent {
            button: MouseButton::Left,
            position,
            modifiers: Modifiers::none(),
            click_count,
        });
    }
    cx.run_until_parked();
}

pub(super) fn drag(cx: &mut VisualTestContext, from: Point<Pixels>, to: Point<Pixels>) {
    cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
    // In steps, as a hand would.
    for step in 1..=6 {
        let t = step as f32 / 6.;
        let at = point(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t);
        cx.simulate_mouse_move(at, Some(MouseButton::Left), Modifiers::none());
        cx.run_until_parked();
    }
    cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
}

pub(super) fn pending(host: &Entity<Host>, cx: &mut VisualTestContext) -> usize {
    cx.update(|_, cx| {
        host.read(cx)
            .view
            .read(cx)
            .body
            .as_ref()
            .map_or(0, |b| b.pending())
    })
}

/// The commands sent so far.
fn sent(host: &Entity<Host>, cx: &mut VisualTestContext) -> Vec<String> {
    cx.update(|_, cx| {
        host.read(cx)
            .seen
            .iter()
            .filter_map(|seen| match seen {
                Seen::Send(command) => Some(command.clone()),
                _ => None,
            })
            .collect()
    })
}

/// The last command sent, as the event's JSON.
pub(super) fn last_event(host: &Entity<Host>, cx: &mut VisualTestContext) -> Value {
    let command = sent(host, cx).pop().expect("Send produced a command");
    event_json(&command)
}

/// The extension's answer about the event sent from `version`.
fn answer(
    host: &Entity<Host>,
    version: u64,
    result: Result<u64, String>,
    cx: &mut VisualTestContext,
) {
    let view = cx.update(|_, cx| host.read(cx).view.clone());
    let outcome = Outcome {
        id: cx.update(|_, cx| view.read(cx).id().to_string()),
        version,
        result,
    };
    cx.update(|_, cx| view.update(cx, |view, cx| view.outcome(&outcome, cx)));
    cx.run_until_parked();
}

fn sending(host: &Entity<Host>, cx: &mut VisualTestContext) -> bool {
    cx.update(|_, cx| matches!(host.read(cx).view.read(cx).send, SendState::Sending { .. }))
}

#[gpui::test]
fn panels_wait_for_the_answer_and_retry_a_rejection(cx: &mut TestAppContext) {
    let (host, cx) = open(
        cx,
        "checklist",
        json!({"items": [{"id": "plan", "label": "Plan"}, {"id": "ship", "label": "Ship"}]}),
    );
    click(cx, "genui-item-check-plan");
    click(cx, "genui-panel-send");
    assert!(sending(&host, cx), "Sending until the extension answers");

    // Rejected: the edit stays, with the reason and Retry.
    answer(
        &host,
        1,
        Err("The agent changed this checklist meanwhile".into()),
        cx,
    );
    assert!(!sending(&host, cx));
    assert_eq!(pending(&host, cx), 1);
    assert!(cx.debug_bounds("genui-panel-send-error").is_some());
    click(cx, "genui-panel-send");
    assert_eq!(sent(&host, cx).len(), 2, "Retry sends again");

    // An answer about another version is ignored.
    answer(&host, 7, Ok(8), cx);
    assert!(sending(&host, cx));

    // Edited while the answer is on its way: accepted edits become the base
    // and version, the later one stays pending.
    click(cx, "genui-item-check-ship");
    answer(&host, 1, Ok(2), cx);
    assert!(!sending(&host, cx));
    assert_eq!(pending(&host, cx), 1);
    assert_eq!(cx.update(|_, cx| host.read(cx).view.read(cx).version), 2);
    click(cx, "genui-panel-send");
    let event = last_event(&host, cx);
    assert_eq!(event["version"], 2);
    assert_eq!(ops(&event), vec!["toggle"]);
}

#[gpui::test]
fn table_sort_select_edit_and_send(cx: &mut TestAppContext) {
    let (host, cx) = open(
        cx,
        "table",
        json!({
            "columns": [
                {"key": "name", "label": "Name"},
                {"key": "tests", "label": "Tests", "type": "number"},
                {"key": "state", "label": "State", "type": "status"}
            ],
            "rows": [
                {"id": "api", "cells": {"name": "API", "tests": 120, "state": "passing"}},
                {"id": "web", "cells": {"name": "Web", "tests": 8, "state": "failing"}}
            ],
            "editable": true
        }),
    );
    // Sorting only changes the view: nothing to send.
    click(cx, "genui-th-tests");
    let api = bounds(cx, "genui-row-api");
    let web = bounds(cx, "genui-row-web");
    assert!(web.origin.y < api.origin.y, "ascending: 8 before 120");
    assert_eq!(pending(&host, cx), 0);

    click(cx, "genui-check-web");
    assert_eq!(pending(&host, cx), 1);

    double_click(cx, "genui-cell-api-tests");
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("1,300");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(pending(&host, cx), 2);

    click(cx, "genui-panel-send");
    let event = last_event(&host, cx);
    assert_eq!(ops(&event), vec!["edit_cell", "select"]);
    assert_eq!(event["changes"][0]["value"], json!(1300.0));
    assert_eq!(event["changes"][1]["rows"], json!(["web"]));
    assert_eq!(event["summary"][1], "selected 1 row: Web");
    assert_eq!(event["spec"]["rows"][0]["cells"]["tests"], json!(1300.0));
    assert_eq!(event["version"], 1);
}

#[gpui::test]
fn form_validates_then_submits_every_answer(cx: &mut TestAppContext) {
    let (host, cx) = open(
        cx,
        "form",
        json!({
            "fields": [
                {"id": "name", "label": "Name", "required": true},
                {"id": "env", "type": "radio", "label": "Environment", "options": ["staging", "prod"]},
                {"id": "regions", "type": "multiselect", "options": ["eu", "us"]},
                {"id": "agree", "type": "checkbox", "label": "I agree", "required": true}
            ]
        }),
    );
    // Required answers missing: nothing is sent.
    click(cx, "genui-form-submit");
    assert!(sent(&host, cx).is_empty());
    let problems = cx.update(|_, cx| match &host.read(cx).view.read(cx).body {
        Some(super::super::panel::Body::Form(s)) => s.errors.len(),
        _ => 0,
    });
    assert_eq!(problems, 2);

    click(cx, "genui-input-name");
    cx.simulate_input("Ada");
    click(cx, "genui-option-env-prod");
    click(cx, "genui-option-regions-us");
    click(cx, "genui-option-regions-eu");
    click(cx, "genui-field-agree");
    click(cx, "genui-form-submit");

    let event = last_event(&host, cx);
    assert_eq!(ops(&event), vec!["submit"]);
    assert_eq!(
        event["changes"][0]["values"],
        json!({"name": "Ada", "env": "prod", "regions": ["eu", "us"], "agree": true})
    );
    assert_eq!(event["spec"]["submitted"], true);
    // Shown read-only while it goes out: no field to type in.
    assert!(cx.debug_bounds("genui-input-name").is_none());
}

#[gpui::test]
fn checklist_toggle_reorder_add_and_send(cx: &mut TestAppContext) {
    let (host, cx) = open(
        cx,
        "checklist",
        json!({"items": [
            {"id": "plan", "label": "Plan"},
            {"id": "build", "label": "Build"},
            {"id": "ship", "label": "Ship"}
        ]}),
    );
    click(cx, "genui-item-check-build");
    assert_eq!(pending(&host, cx), 1);

    // Drag "Ship" by its grip above "Plan".
    let grip = bounds(cx, "genui-item-grip-ship").center();
    let plan = bounds(cx, "genui-item-plan");
    drag(cx, grip, point(grip.x, plan.origin.y + px(2.)));
    let ship = bounds(cx, "genui-item-ship");
    assert!(ship.origin.y < bounds(cx, "genui-item-plan").origin.y);

    click(cx, "genui-checklist-add");
    cx.simulate_input("Write docs");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();

    click(cx, "genui-panel-send");
    let event = last_event(&host, cx);
    assert_eq!(ops(&event), vec!["add", "order", "toggle"]);
    assert_eq!(
        event["changes"][1]["items"],
        json!(["ship", "plan", "build", "write-docs"])
    );
    assert_eq!(
        event["summary"],
        json!([
            "added “Write docs”",
            "reordered the list",
            "checked “Build”"
        ])
    );
}

#[gpui::test]
fn kanban_drag_between_columns_and_keyboard_move(cx: &mut TestAppContext) {
    let (host, cx) = open(
        cx,
        "kanban",
        json!({
            "columns": [{"id": "todo", "label": "To do"}, {"id": "doing", "label": "Doing"}, {"id": "done", "label": "Done"}],
            "cards": [
                {"id": "a", "title": "Login", "column": "todo"},
                {"id": "b", "title": "Signup", "column": "todo"}
            ]
        }),
    );
    // Drag "Login" into Done.
    let card = bounds(cx, "genui-card-a").center();
    let done = bounds(cx, "genui-column-done");
    drag(cx, card, point(done.center().x, done.origin.y + px(50.)));
    let moved = bounds(cx, "genui-card-a");
    assert!(done.contains(&moved.center()), "the card sits in Done");
    assert_eq!(pending(&host, cx), 1);

    // "Signup" is selected with a click, then Alt-→ moves it to Doing.
    click(cx, "genui-card-b");
    cx.simulate_keystrokes("alt-right");
    cx.run_until_parked();
    assert!(bounds(cx, "genui-column-doing").contains(&bounds(cx, "genui-card-b").center()));

    click(cx, "genui-panel-send");
    let event = last_event(&host, cx);
    assert_eq!(ops(&event), vec!["order", "order"]);
    assert_eq!(
        event["summary"],
        json!([
            "moved “Signup” from To do to Doing",
            "moved “Login” from To do to Done"
        ])
    );
}

#[gpui::test]
fn chart_hover_select_and_ask(cx: &mut TestAppContext) {
    let (host, cx) = open(
        cx,
        "chart",
        json!({"type": "bar", "labels": ["a", "b", "c"], "series": [{"name": "Load", "values": [10, 20, 30]}], "unit": "ms"}),
    );
    // Over the middle bar, low enough to be inside it.
    let chart = bounds(cx, "genui-chart");
    let at = point(chart.center().x + px(15.), chart.bottom() - px(35.));
    cx.simulate_mouse_move(at, None, Modifiers::none());
    cx.run_until_parked();
    cx.simulate_click(at, Modifiers::none());
    cx.run_until_parked();
    click(cx, "genui-chart-ask");
    let asked = cx.update(|_, cx| {
        host.read(cx).seen.last()
            == Some(&Seen::Ask(
                "About b (20 ms) in the “Test chart” chart (ui-chart): ".into(),
            ))
    });
    assert!(asked, "Ask about this names the selected value");
    // Read-only: nothing to send.
    assert_eq!(pending(&host, cx), 0);
}
