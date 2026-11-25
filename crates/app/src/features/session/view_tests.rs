//! Interaction tests: a real [`SessionView`] (no backend started) in a
//! headless window, its Desktop channel fed the records Gentle Shell would
//! send, driven by simulated clicks.

// Not a glob: GPUI exports a `test` attribute that would shadow `#[test]`.
use std::cell::RefCell;
use std::rc::Rc;

use gentle_core::commands::Screen;
use gpui_kit::component::input::Position;
use gpui_kit::{
    AppContext as _, Context, Entity, FileDropEvent, InteractiveElement as _, IntoElement,
    Modifiers, MouseButton, ParentElement as _, Render, StatefulInteractiveElement as _,
    Styled as _, Subscription, TestAppContext, VisualTestContext, Window,
    component::{WindowExt as _, h_flex, v_flex},
    div, point, px,
};
use pi_rpc::desktop_rpc::{PROMPT_COMMAND, SCHEMA, WIDGET_KEY};
use pi_rpc::{UiMethod, UiRequest};
use serde_json::{Value, json};

use super::attachments::{AttachedFile, Attachment, Loaded};
use super::{AgentsTab, SessionView};
use crate::features::files::FileDrag;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";
const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 0, 0x10];

fn widget(record: Value) -> UiRequest {
    UiRequest {
        id: "ui-1".into(),
        timeout: None,
        method: UiMethod::SetWidget {
            key: WIDGET_KEY.into(),
            lines: Some(vec![record.to_string()]),
            placement: None,
        },
    }
}

fn response(request_id: &str, data: Value) -> UiRequest {
    widget(json!({
        "schema": SCHEMA, "kind": "response", "requestId": request_id, "ok": true, "data": data,
    }))
}

/// A tab in a headless window that never starts a backend (drawing a tab
/// starts one otherwise).
fn tab(cx: &mut TestAppContext) -> (Entity<SessionView>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
    });
    let cwd = std::env::temp_dir();
    cx.add_window_view(|window, cx| {
        let mut view = SessionView::new(cwd, None, window, cx, false);
        view.backend.started = true;
        view
    })
}

/// The same tab in a window with the app's root, so a warning it raises is
/// shown (and can be counted): [`warnings`].
fn rooted_tab(cx: &mut TestAppContext) -> (Entity<SessionView>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
    });
    let cwd = std::env::temp_dir();
    let mut built = None;
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let tab = cx.new(|cx| {
            let mut view = SessionView::new(cwd, None, window, cx, false);
            view.backend.started = true;
            view
        });
        built = Some(tab.clone());
        gpui_kit::base::Root::new(tab, window, cx)
    });
    (built.expect("the tab was built"), cx)
}

/// The warnings the window is showing: what a refusal raises.
fn warnings(cx: &mut VisualTestContext) -> usize {
    cx.update(|window, cx| window.notifications(cx).len())
}

/// What the composer holds.
fn composer_text(view: &Entity<SessionView>, cx: &mut VisualTestContext) -> String {
    view.read_with(cx, |view, cx| view.composer.read(cx).value().to_string())
}

/// Put `text` in the composer, as typing it would.
fn type_text(view: &Entity<SessionView>, cx: &mut VisualTestContext, text: &str) {
    view.update_in(cx, |view, window, cx| {
        view.composer
            .update(cx, |state, cx| state.set_value(text, window, cx));
    });
    cx.run_until_parked();
}

/// Press Enter: the send path the composer's key and its button share.
fn press_enter(view: &Entity<SessionView>, cx: &mut VisualTestContext) {
    view.update_in(cx, |view, window, cx| view.submit(window, cx));
}

/// A tab whose channel is probed with `methods`, and whose job and helper
/// lists hold one running job (`job-1`) and one helper (`agent-1`) the
/// `gentle-agents` widget has no thread for.
fn open<'a>(
    methods: &[&str],
    cx: &'a mut TestAppContext,
) -> (Entity<SessionView>, &'a mut VisualTestContext) {
    let (view, cx) = tab(cx);
    let mut all = vec![
        "capabilities",
        "get_snapshot",
        "list_jobs",
        "list_subagents",
    ];
    all.extend_from_slice(methods);
    view.update(cx, |view, _| {
        let desktop = &mut view.desktop;
        let probe = desktop.ready(true).sends.remove(0);
        let snapshot = desktop
            .ui(&response(
                probe.request.request_id(),
                json!({
                    "schema": SCHEMA, "command": PROMPT_COMMAND, "widget": WIDGET_KEY,
                    "methods": all, "events": ["jobs_update", "agents_update"],
                }),
            ))
            .sends
            .remove(0);
        desktop.ui(&response(
            snapshot.request.request_id(),
            json!({"schema": SCHEMA, "sequence": 1, "state": {}}),
        ));
        let agents = desktop.supervision_seed().sends.remove(0);
        desktop.supervision_apply(&response(
            agents.request.request_id(),
            json!({"agents": [{
                "id": "agent-1", "agent": "worker", "status": "running",
                "model": "anthropic/claude-sonnet-4", "lastStep": "writing",
                "usage": {"total": 100, "cost": 0.01},
            }]}),
        ));
        let jobs = desktop.supervision_seed().sends.remove(0);
        desktop.supervision_apply(&response(
            jobs.request.request_id(),
            json!({"jobs": [{"id": "job-1", "label": "npm test", "status": "running", "startedAt": 1}]}),
        ));
        assert!(desktop.jobs_ready() && desktop.subagents_ready());
    });
    (view, cx)
}

fn drawn(cx: &mut VisualTestContext, selector: &'static str) -> bool {
    cx.run_until_parked();
    cx.debug_bounds(selector).is_some()
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let at = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("nothing drawn as {selector}"))
        .center();
    cx.simulate_click(at, Modifiers::none());
    cx.run_until_parked();
}

/// Answer the output read the slice owes now, as Gentle Shell would.
fn answer(view: &Entity<SessionView>, cx: &mut VisualTestContext, data: Value) {
    view.update(cx, |view, cx| {
        let sends = view.desktop.output_pump().sends;
        let id = sends[0].request.request_id().to_string();
        view.desktop.output_apply(&response(&id, data));
        cx.notify();
    });
    cx.run_until_parked();
}

#[gpui::test]
fn a_job_row_folds_its_output_open_and_shut(cx: &mut TestAppContext) {
    let (view, cx) = open(&["get_job_output"], cx);
    view.update(cx, |view, cx| view.open_agents_panel(AgentsTab::Jobs, cx));
    assert!(drawn(cx, "job-row-job-1"));
    assert!(!drawn(cx, "job-output"));

    click(cx, "job-row-job-1");
    // The tab has no backend, so the first read was dropped; the well shows
    // that it is reading, and the next owed read gets its answer.
    assert!(drawn(cx, "job-output"));
    view.update(cx, |view, _| {
        assert!(view.desktop.output_mut().tick_job());
    });
    answer(
        &view,
        cx,
        json!({
            "job": {"id": "job-1", "label": "npm test", "status": "running", "startedAt": 1},
            "output": "✓ lint\nrunning tests",
            "truncated": true,
        }),
    );
    view.read_with(cx, |view, _| {
        let job = view.desktop.output().job().unwrap();
        assert_eq!(job.output.as_ref().unwrap().output, "✓ lint\nrunning tests");
    });
    assert!(drawn(cx, "job-output"));

    // Clicking it again folds it and stops reading.
    click(cx, "job-row-job-1");
    assert!(!drawn(cx, "job-output"));
    view.read_with(cx, |view, _| assert!(view.desktop.output().job().is_none()));
}

#[gpui::test]
fn without_get_job_output_a_job_row_stays_as_it_was(cx: &mut TestAppContext) {
    let (view, cx) = open(&[], cx);
    view.update(cx, |view, cx| view.open_agents_panel(AgentsTab::Jobs, cx));
    click(cx, "job-row-job-1");
    assert!(!drawn(cx, "job-output"));
    view.read_with(cx, |view, _| assert!(view.desktop.output().job().is_none()));
}

#[gpui::test]
fn the_activity_tab_reads_a_thread_the_widget_did_not_publish(cx: &mut TestAppContext) {
    let (view, cx) = open(&["get_subagent_thread"], cx);
    view.update(cx, |view, cx| {
        view.open_agents_panel(AgentsTab::Activity, cx);
    });
    cx.run_until_parked();
    view.update(cx, |view, _| {
        let thread = view.desktop.output().thread().expect("watched on open");
        assert_eq!(thread.id, "agent-1");
        // No backend here: the dropped first read is asked again, as an
        // `agents_update` would.
        view.desktop.output_mut().refresh_thread();
    });
    assert!(!drawn(cx, "agent-thread"));
    answer(
        &view,
        cx,
        json!({
            "id": "agent-1", "version": 3, "dropped": 0, "truncated": false,
            "items": [{"kind": "text", "text": "Mapping the repository layout."}],
        }),
    );
    assert!(drawn(cx, "agent-thread"));

    // Leaving the tab stops reading it.
    view.update(cx, |view, cx| view.close_agents(cx));
    view.read_with(cx, |view, _| {
        assert!(view.desktop.output().thread().is_none())
    });
}

#[gpui::test]
fn without_get_subagent_thread_the_activity_tab_keeps_its_empty_state(cx: &mut TestAppContext) {
    let (view, cx) = open(&[], cx);
    view.update(cx, |view, cx| {
        view.open_agents_panel(AgentsTab::Activity, cx)
    });
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert!(view.desktop.output().thread().is_none())
    });
    assert!(!drawn(cx, "agent-thread"));
}

/// The session feed's sections drive the todo island and the command menu;
/// a feed-listed Pi built-in that only works in the terminal is never sent.
#[gpui::test]
fn the_session_feed_drives_the_todo_list_and_the_command_menu(cx: &mut TestAppContext) {
    let (view, cx) = tab(cx);
    view.update(cx, |view, cx| {
        let probe = view.desktop.ready(true).sends.remove(0);
        let snapshot = view
            .desktop
            .ui(&response(
                probe.request.request_id(),
                json!({
                    "schema": SCHEMA, "command": PROMPT_COMMAND, "widget": WIDGET_KEY,
                    "methods": ["capabilities", "get_snapshot"], "events": ["session_snapshot"],
                }),
            ))
            .sends
            .remove(0);
        view.desktop.ui(&response(
            snapshot.request.request_id(),
            json!({"schema": SCHEMA, "sequence": 1, "state": {"session": {
                "context": {"percent": 61.0, "tokens": 122000, "window": 200000},
                "todo": {
                    "items": [{"id": 1, "title": "Wire the feed", "state": "in_progress", "note": null}],
                    "progress": {"done": 0, "total": 1, "open": 1}, "current": "Wire the feed", "truncated": false,
                },
                "activity": {"state": "idle", "streaming": false, "phase": null, "phaseLabel": null,
                             "blockedLabel": null, "agents": 0, "jobs": 0, "label": "Idle"},
                "pendingQuestion": null,
                "commands": {"items": [
                    {"name": "compact", "description": "Pi compact", "args": null, "terminal": true, "source": "builtin"},
                    {"name": "model", "description": "Select model", "args": null, "terminal": true, "source": "builtin"},
                ], "truncated": false},
            }}}),
        ));
        view.sync_sections(cx);
    });
    cx.run_until_parked();
    view.update(cx, |view, cx| {
        let titles: Vec<&str> = view.todo.tasks.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, ["Wire the feed"]);
        assert_eq!(view.usage.context_percent, Some(61.0));

        let commands = view.commands();
        let compact = commands.iter().find(|c| c.name == "compact").unwrap();
        assert!(!compact.terminal, "the app's own /compact wins");
        let model = commands.iter().find(|c| c.name == "model").unwrap();
        assert!(model.terminal);

        let before = view.transcript.items.len();
        view.run_prompt("/model".into(), cx);
        assert_eq!(view.transcript.items.len(), before + 1);
        assert!(matches!(
            view.transcript.items.last(),
            Some(gentle_core::transcript::Item::Notice { text, .. })
                if text == "/model only works in Gentle Shell's terminal."
        ));
    });
}

/// A tab under the window's text-selection layer, as the app's Root draws it.
struct Selectable(Entity<SessionView>);

impl gpui_kit::Render for Selectable {
    fn render(
        &mut self,
        _: &mut gpui_kit::Window,
        _: &mut gpui_kit::Context<Self>,
    ) -> impl gpui_kit::IntoElement {
        use gpui_kit::{ParentElement as _, Styled as _};
        gpui_kit::div()
            .size_full()
            .child(gpui_kit::base::TextSelectionLayer)
            .child(self.0.clone())
    }
}

/// A message you sent can be selected with the mouse, like an answer, and
/// what is selected is its text.
#[gpui::test]
fn a_sent_prompt_can_be_selected(cx: &mut TestAppContext) {
    use gpui_kit::{AppContext as _, MouseButton, base::TextSelection};

    struct FocusedSession {
        session: Entity<SessionView>,
        focus: gpui_kit::FocusHandle,
    }

    impl gpui_kit::Render for FocusedSession {
        fn render(
            &mut self,
            _: &mut gpui_kit::Window,
            _: &mut gpui_kit::Context<Self>,
        ) -> impl gpui_kit::IntoElement {
            use gpui_kit::{InteractiveElement as _, ParentElement as _, Styled as _};
            gpui_kit::div()
                .id("focus")
                .track_focus(&self.focus)
                .size_full()
                .child(self.session.clone())
        }
    }

    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
    });
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let session = cx.new(|cx| {
            let mut view = SessionView::new(std::env::temp_dir(), None, window, cx, false);
            view.backend.started = true;
            view.transcript
                .items
                .push(gentle_core::transcript::Item::User {
                    text: "fix the parser".into(),
                    images: Vec::new(),
                });
            view.sync_view(cx);
            view
        });
        let view = cx.new(|_| FocusedSession { session, focus });
        gpui_kit::base::Root::new(view, window, cx)
    });
    cx.run_until_parked();
    let bounds = cx
        .debug_bounds("user-text-0")
        .expect("the sent prompt is drawn");
    let (start, end) = (
        gpui_kit::point(bounds.left() + gpui_kit::px(1.), bounds.center().y),
        gpui_kit::point(bounds.right() - gpui_kit::px(1.), bounds.center().y),
    );
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(end, Some(MouseButton::Left), Modifiers::none());
    cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::none());
    cx.update(|window, cx| {
        let _ = window.draw(cx);
        assert_eq!(TextSelection::selected_text(window, cx), "fix the parser\n");
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("sentinel".into()));
    });
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-c"
    } else {
        "ctrl-c"
    });
    cx.run_until_parked();
    assert_eq!(
        cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text())),
        Some("fix the parser".into())
    );
}

/// Selecting a sent bubble copies the displayed text, without matched delimiters.
#[gpui::test]
fn user_bubble_inline_code_selection_omits_matched_delimiters(cx: &mut TestAppContext) {
    use gpui_kit::{AppContext as _, MouseButton, base::TextSelection};

    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
    });
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let session = cx.new(|cx| {
            let mut view = SessionView::new(std::env::temp_dir(), None, window, cx, false);
            view.backend.started = true;
            view.transcript
                .items
                .push(gentle_core::transcript::Item::User {
                    text: "fix `café` and `x` then `open".into(),
                    images: Vec::new(),
                });
            view.sync_view(cx);
            view
        });
        Selectable(session)
    });
    cx.run_until_parked();
    let bounds = cx.debug_bounds("user-text-0").expect("sent prompt");
    let start = gpui_kit::point(bounds.left() + gpui_kit::px(1.), bounds.center().y);
    let end = gpui_kit::point(bounds.right() - gpui_kit::px(1.), bounds.center().y);
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(end, Some(MouseButton::Left), Modifiers::none());
    cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::none());
    cx.update(|window, cx| {
        let _ = window.draw(cx);
        assert_eq!(
            TextSelection::selected_text(window, cx),
            "fix café and x then `open\n"
        );
    });
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-c"
    } else {
        "ctrl-c"
    });
    cx.run_until_parked();
    assert_eq!(
        cx.update(|_, cx| cx
            .read_from_clipboard()
            .and_then(|item| item.text())
            .unwrap_or_default()),
        "fix café and x then `open"
    );
}

/// Authored soft breaks stay visible and selection copies the formatted text.
#[gpui::test]
fn user_bubble_authored_newlines_and_clipboard(cx: &mut TestAppContext) {
    use gpui_kit::{AppContext as _, MouseButton, point, px};

    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
    });
    let (_, cx) = cx.add_window_view(|window, cx| {
        let session = cx.new(|cx| {
            let mut view = SessionView::new(std::env::temp_dir(), None, window, cx, false);
            view.backend.started = true;
            view.transcript
                .items
                .push(gentle_core::transcript::Item::User {
                    text: "1- `code`\n2- **bold**\n3- ~~strike~~".into(),
                    images: Vec::new(),
                });
            view.sync_view(cx);
            view
        });
        Selectable(session)
    });
    cx.run_until_parked();
    let bounds = cx.debug_bounds("user-text-0").expect("sent prompt");
    let start = point(bounds.left() + px(1.), bounds.top() + px(5.));
    let first_end = point(bounds.right() - px(1.), bounds.top() + px(5.));
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(first_end, Some(MouseButton::Left), Modifiers::none());
    cx.simulate_mouse_up(first_end, MouseButton::Left, Modifiers::none());
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-c"
    } else {
        "ctrl-c"
    });
    cx.run_until_parked();
    let partial = cx.update(|_, cx| {
        cx.read_from_clipboard()
            .and_then(|item| item.text())
            .unwrap_or_default()
    });
    assert_eq!(partial, "1- code");

    let end = point(bounds.right() - px(1.), bounds.bottom() - px(5.));
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(end, Some(MouseButton::Left), Modifiers::none());
    cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::none());
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-c"
    } else {
        "ctrl-c"
    });
    cx.run_until_parked();
    let clipboard = cx.update(|_, cx| {
        cx.read_from_clipboard()
            .and_then(|item| item.text())
            .unwrap_or_default()
    });
    assert!(
        bounds.size.height > px(40.),
        "three authored lines must occupy three rows"
    );
    assert_eq!(clipboard, "1- code\n2- bold\n3- strike");
}

/// Existing list and fenced-code blocks retain their rendered lines and copy text.
#[gpui::test]
fn user_bubble_list_and_fenced_code_preserved(cx: &mut TestAppContext) {
    use gpui_kit::{AppContext as _, MouseButton, point, px};

    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
    });
    for (source, expected) in [
        ("- one\n- two\n- three", "one\ntwo\nthree"),
        ("```\none\ntwo\nthree\n```", "one\ntwo\nthree"),
    ] {
        let (_, cx) = cx.add_window_view(|window, cx| {
            let session = cx.new(|cx| {
                let mut view = SessionView::new(std::env::temp_dir(), None, window, cx, false);
                view.backend.started = true;
                view.transcript
                    .items
                    .push(gentle_core::transcript::Item::User {
                        text: source.into(),
                        images: Vec::new(),
                    });
                view.sync_view(cx);
                view
            });
            Selectable(session)
        });
        cx.run_until_parked();
        let bounds = cx.debug_bounds("user-text-0").expect("sent prompt");
        assert!(
            bounds.size.height > px(40.),
            "block must occupy multiple rows: {source:?}"
        );
        let start = point(bounds.left() + px(1.), bounds.top() + px(5.));
        let end = point(bounds.right() - px(1.), bounds.bottom() - px(5.));
        cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(end, Some(MouseButton::Left), Modifiers::none());
        cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::none());
        cx.simulate_keystrokes(if cfg!(target_os = "macos") {
            "cmd-c"
        } else {
            "ctrl-c"
        });
        cx.run_until_parked();
        let clipboard = cx.update(|_, cx| {
            cx.read_from_clipboard()
                .and_then(|item| item.text())
                .unwrap_or_default()
        });
        assert_eq!(clipboard, expected, "source: {source:?}");
    }
}

fn assert_user_bubble_copy(
    cx: &mut TestAppContext,
    source: &str,
    expected: &str,
    start_inset: Option<f32>,
    end_inset: f32,
) {
    use gpui_kit::{AppContext as _, MouseButton, base::TextSelection, point, px};

    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
    });
    let (_, cx) = cx.add_window_view(|window, cx| {
        let session = cx.new(|cx| {
            let mut view = SessionView::new(std::env::temp_dir(), None, window, cx, false);
            view.backend.started = true;
            view.transcript
                .items
                .push(gentle_core::transcript::Item::User {
                    text: source.into(),
                    images: Vec::new(),
                });
            view.sync_view(cx);
            view
        });
        Selectable(session)
    });
    cx.run_until_parked();
    let bounds = cx.debug_bounds("user-text-0").expect("sent prompt");
    let end = point(bounds.right() - px(end_inset), bounds.bottom() - px(10.));
    let start_inset = start_inset.unwrap_or_else(|| {
        // Find a hit inside the first glyph's leading half rather than assuming
        // the list marker's indentation and glyph metrics in pixels.
        (1..200)
            .find(|x| {
                let start = point(bounds.left() + px(*x as f32), bounds.top() + px(10.));
                cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
                cx.simulate_mouse_move(end, Some(MouseButton::Left), Modifiers::none());
                cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::none());
                cx.update(|window, cx| {
                    let _ = window.draw(cx);
                    TextSelection::selected_text(window, cx).trim_end_matches('\n') == expected
                })
            })
            .expect("selection reached the first glyph") as f32
    });
    let start = point(bounds.left() + px(start_inset), bounds.top() + px(10.));
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(end, Some(MouseButton::Left), Modifiers::none());
    cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::none());
    let selected = cx.update(|window, cx| {
        let _ = window.draw(cx);
        TextSelection::selected_text(window, cx)
    });
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-c"
    } else {
        "ctrl-c"
    });
    cx.run_until_parked();
    let clipboard = cx.update(|_, cx| {
        cx.read_from_clipboard()
            .and_then(|item| item.text())
            .unwrap_or_default()
    });
    assert_eq!(selected.trim_end_matches('\n'), expected, "selection");
    assert_eq!(clipboard, expected, "clipboard");
}

#[gpui::test]
fn user_bubble_complete_html_fragments_are_literal(cx: &mut TestAppContext) {
    for (source, expected) in [
        (
            "<div><span>hola</span></div>",
            "<div><span>hola</span></div>",
        ),
        (
            "**outside** <span title=\"**attr**\">**inside**</span> **after**",
            "outside <span title=\"**attr**\">**inside**</span> after",
        ),
        (
            "<div><br><span title=\"a > b\">**hola**</span></div>",
            "<div><br><span title=\"a > b\">**hola**</span></div>",
        ),
    ] {
        assert_user_bubble_copy(cx, source, expected, Some(1.), 1.);
    }
}

#[gpui::test]
fn user_bubble_html_comment_does_not_close_fragment(cx: &mut TestAppContext) {
    let source = "<div><!-- </div> -->**inside**</div>";
    assert_user_bubble_copy(cx, source, source, Some(1.), 1.);
}

#[gpui::test]
fn user_bubble_folded_html_keeps_content_literal(cx: &mut TestAppContext) {
    let source = "<div>\n**one**\n**two**\n**three**\n**four**\n**five**\n**six**\n**seven**\n**eight**\n**nine**\n</div>";
    let expected = "<div>\n**one**\n**two**\n**three**\n**four**\n**five**\n**six**\n**seven**";
    assert_user_bubble_copy(cx, source, expected, Some(1.), 1.);
}

#[gpui::test]
fn user_bubble_html_indented_content_has_no_escape_backslashes(cx: &mut TestAppContext) {
    let source = "<div>\n\n    **literal**\n</div>";
    assert_user_bubble_copy(cx, source, source, Some(1.), 1.);
}

#[gpui::test]
fn user_bubble_html_tab_indentation_is_literal(cx: &mut TestAppContext) {
    let source = "<div>\n\n\t**literal**\n</div>";
    assert_user_bubble_copy(cx, source, source, Some(1.), 1.);
}

#[gpui::test]
fn user_bubble_raw_html_is_visible(cx: &mut TestAppContext) {
    assert_user_bubble_copy(
        cx,
        "<span title=\"**important**\">text</span>",
        "<span title=\"**important**\">text</span>",
        Some(1.),
        1.,
    );
    assert_user_bubble_copy(
        cx,
        "hello <div>world</div>",
        "hello <div>world</div>",
        Some(1.),
        1.,
    );
    assert_user_bubble_copy(
        cx,
        "<div>\nhello\n</div>",
        "<div>\nhello\n</div>",
        Some(1.),
        1.,
    );
    assert_user_bubble_copy(
        cx,
        "**bold** `<div>` [link](https://example.com) &amp;",
        "bold <div> link &",
        Some(1.),
        1.,
    );
}

#[gpui::test]
fn user_bubble_container_fenced_code_copy(cx: &mut TestAppContext) {
    assert_user_bubble_copy(
        cx,
        "- item\n\n  ```\n  one\n  two\n  ```",
        "item\none\ntwo",
        None,
        8.,
    );
}

#[gpui::test]
fn user_bubble_triple_backtick_inline_copy(cx: &mut TestAppContext) {
    assert_user_bubble_copy(
        cx,
        "```code```\nnext\nlast",
        "code\nnext\nlast",
        Some(1.),
        8.,
    );
}

#[gpui::test]
fn user_bubble_backslash_hard_break_copy(cx: &mut TestAppContext) {
    assert_user_bubble_copy(cx, "one\\\ntwo", "one\ntwo", Some(1.), 1.);
}

/// S7: a message sent before images left the composer still carries
/// `[Image #N]`; its bubble shows the chip, as it did then.
#[gpui::test]
fn user_bubble_shows_old_image_tokens_as_chips(cx: &mut TestAppContext) {
    assert_user_bubble_copy(cx, "see [Image #1]", "see 🖼 #1", None, 1.);
}

/// The transcript items of a view showing `messages` as sent prompts, drawn
/// in a headless window.
fn prompts<'a>(messages: &[&str], cx: &'a mut TestAppContext) -> &'a mut VisualTestContext {
    use gpui_kit::AppContext as _;

    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
    });
    let messages: Vec<String> = messages.iter().map(|m| m.to_string()).collect();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let session = cx.new(|cx| {
            let mut view = SessionView::new(std::env::temp_dir(), None, window, cx, false);
            view.backend.started = true;
            for text in messages {
                view.transcript
                    .items
                    .push(gentle_core::transcript::Item::User {
                        text,
                        images: Vec::new(),
                    });
            }
            view.sync_view(cx);
            view
        });
        Selectable(session)
    });
    cx.run_until_parked();
    cx
}

/// S9: a sent prompt's files show as tiles beside the images, and never as
/// the `Attached files:` block in the text: the bubble reads as what was typed
/// (and copies as it), so a long block adds no lines.
#[gpui::test]
fn sent_files_show_as_tiles_and_stay_out_of_the_text(cx: &mut TestAppContext) {
    use gpui_kit::px;

    let message =
        "Qué te envio?\n\nAttached files:\n- /Users/me/Comu 3/Laboratorio 1.6.2.pdf\n- notes.txt";
    assert_user_bubble_copy(cx, message, "Qué te envio?", Some(1.), 1.);

    let cx = prompts(&[message], cx);
    let bounds = cx.debug_bounds("user-text-0").expect("the sent prompt");
    assert!(
        bounds.size.height < px(40.),
        "the text is one line, not the block's three: {}",
        bounds.size.height
    );
    assert!(cx.debug_bounds("user-file-0-0").is_some(), "first card");
    assert!(cx.debug_bounds("user-file-0-1").is_some(), "second card");
    assert!(
        cx.debug_bounds("user-file-0-2").is_none(),
        "one card per file"
    );
}

/// S9: a prompt that was only files shows their cards and no empty text
/// block, as an image-only one does.
#[gpui::test]
fn a_files_only_prompt_shows_cards_without_an_empty_text(cx: &mut TestAppContext) {
    let cx = prompts(&["Attached files:\n- report.pdf"], cx);
    assert!(cx.debug_bounds("user-file-0-0").is_some(), "the card");
    assert!(
        cx.debug_bounds("user-text-0").is_none(),
        "no empty text block"
    );
}

/// S13: a strip is even. In a sent bubble an image and two files (one with a
/// folder, one at the root) are tiles of the same size, on the image's row.
#[gpui::test]
fn a_sent_strip_is_one_row_of_equal_tiles(cx: &mut TestAppContext) {
    use gpui_kit::AppContext as _;

    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
    });
    let text = "Mira\n\nAttached files:\n- ~/Documents/documento.pdf\n- test.txt";
    let (_, cx) = cx.add_window_view(|window, cx| {
        let session = cx.new(|cx| {
            let mut view = SessionView::new(std::env::temp_dir(), None, window, cx, false);
            view.backend.started = true;
            view.transcript
                .items
                .push(gentle_core::transcript::Item::User {
                    text: text.to_string(),
                    images: vec![gentle_core::transcript::UserImage::new(
                        "image/png",
                        "iVBORw0KGgo=".into(),
                    )],
                });
            view.sync_view(cx);
            view
        });
        Selectable(session)
    });
    cx.run_until_parked();

    let image = bounds(cx, "user-image-0-0");
    let with_folder = bounds(cx, "user-file-0-0");
    let at_root = bounds(cx, "user-file-0-1");
    assert_eq!(
        image.size, with_folder.size,
        "a file tile is the image tile's size"
    );
    assert_eq!(image.size, at_root.size, "both files, the same tile");
    assert_eq!(
        (image.origin.y, image.bottom()),
        (with_folder.origin.y, with_folder.bottom()),
        "the files sit on the image's row"
    );
    assert_eq!(
        (image.origin.y, image.bottom()),
        (at_root.origin.y, at_root.bottom())
    );
}

/// S13: the composer's strip is even the same way: an attached image and two
/// files draw tiles of one size, on one row.
#[gpui::test]
fn the_composer_strip_is_one_row_of_equal_tiles(cx: &mut TestAppContext) {
    let (view, cx) = tab(cx);
    let image = Attachment::from_bytes("a.png", PNG.to_vec()).unwrap();
    let file = |name: &str, path: &str, bytes: u64| {
        Ok(Loaded::File(AttachedFile {
            name: name.to_string(),
            path: std::path::PathBuf::from(path),
            canonical: std::path::PathBuf::from(path),
            bytes,
        }))
    };
    view.update_in(cx, |view, window, cx| {
        view.attach(
            vec![
                Ok(Loaded::Image(image)),
                file("documento.pdf", "/Users/me/Documents/documento.pdf", 2048),
                file("test.txt", "/Users/me/test.txt", 12),
            ],
            window,
            cx,
        );
    });

    let image = bounds(cx, "composer-image-0");
    let with_folder = bounds(cx, "composer-file-0");
    let at_root = bounds(cx, "composer-file-1");
    assert_eq!(
        image.size, with_folder.size,
        "a file tile is the image tile's size"
    );
    assert_eq!(image.size, at_root.size, "both files, the same tile");
    assert_eq!(
        (image.origin.y, image.bottom()),
        (with_folder.origin.y, with_folder.bottom()),
        "the files sit on the image's row"
    );
    assert_eq!(
        (image.origin.y, image.bottom()),
        (at_root.origin.y, at_root.bottom())
    );
}

/// S13: a file tile keeps the × its card had, in the same corner an image's ×
/// sits in, and it removes that file.
#[gpui::test]
fn a_composer_file_tile_keeps_its_remove_button(cx: &mut TestAppContext) {
    let (view, cx) = tab(cx);
    let image = Attachment::from_bytes("a.png", PNG.to_vec()).unwrap();
    let file = |name: &str, path: &str| {
        Ok(Loaded::File(AttachedFile {
            name: name.to_string(),
            path: std::path::PathBuf::from(path),
            canonical: std::path::PathBuf::from(path),
            bytes: 12,
        }))
    };
    view.update_in(cx, |view, window, cx| {
        view.attach(
            vec![
                Ok(Loaded::Image(image)),
                file("documento.pdf", "/work/documento.pdf"),
                file("test.txt", "/work/test.txt"),
            ],
            window,
            cx,
        );
    });

    let corner = |tile: gpui_kit::Bounds<gpui_kit::Pixels>,
                  remove: gpui_kit::Bounds<gpui_kit::Pixels>| {
        (remove.left() - tile.left(), remove.top() - tile.top())
    };
    let on_image = bounds(cx, "attachment-remove-0");
    let image_tile = bounds(cx, "composer-image-0");
    let on_file = bounds(cx, "file-remove-0");
    let file_tile = bounds(cx, "composer-file-0");
    assert_eq!(
        corner(image_tile, on_image),
        corner(file_tile, on_file),
        "a file's × sits where an image's does"
    );
    assert!(
        file_tile.contains(&on_file.center()),
        "the × is inside the tile"
    );

    click(cx, "file-remove-0");
    assert_eq!(
        files(&view, cx),
        vec![std::path::PathBuf::from("/work/test.txt")],
        "the × removes its own file"
    );
}

/// Attaching an image types nothing into the composer, so the strip above it
/// is the only list of what will be sent; removing one from the strip leaves
/// the rest, in the strip's own order.
#[gpui::test]
fn images_attach_without_text_and_the_strip_is_what_is_sent(cx: &mut TestAppContext) {
    let (view, cx) = tab(cx);
    let first = Attachment::from_bytes("a.png", PNG.to_vec()).unwrap();
    let second = Attachment::from_bytes("b.jpeg", JPEG.to_vec()).unwrap();
    let second_key = second.key;
    view.update_in(cx, |view, window, cx| {
        view.attach(
            vec![Ok(Loaded::Image(first)), Ok(Loaded::Image(second))],
            window,
            cx,
        );
    });
    let text = |cx: &mut VisualTestContext| {
        view.read_with(cx, |view, cx| view.composer.read(cx).value().to_string())
    };
    assert_eq!(text(cx), "", "attaching an image types nothing");
    assert_eq!(view.read_with(cx, |view, _| view.attachments.len()), 2);

    view.update_in(cx, |view, _, cx| view.remove_attachment(0, cx));
    assert_eq!(text(cx), "", "removing one types nothing either");
    let sent: Vec<u64> = view.read_with(cx, |view, _| {
        view.attachments.iter().map(|a| a.key).collect()
    });
    assert_eq!(sent, vec![second_key], "the strip's images, in its order");
}

/// S1: a copied file that is not an image pastes as a file attachment, like
/// a dropped one, and types nothing into the composer.
#[gpui::test]
fn pasting_a_copied_file_attaches_it(cx: &mut TestAppContext) {
    let (view, cx) = tab(cx);
    let dir = std::env::temp_dir().join(format!("gs-paste-file-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let report = dir.join("report.pdf");
    std::fs::write(&report, b"%PDF-1.7\n").unwrap();
    let item = gpui_kit::ClipboardItem {
        entries: vec![gpui_kit::ClipboardEntry::ExternalPaths(
            gpui_kit::ExternalPaths(vec![report.clone()].into()),
        )],
    };

    let handled = view.update_in(cx, |view, window, cx| {
        view.paste_clipboard(&item, window, cx)
    });
    cx.run_until_parked();

    assert!(handled, "the paste is taken as an attachment");
    let files: Vec<_> = view.read_with(cx, |view, _| {
        view.files.iter().map(|file| file.path.clone()).collect()
    });
    assert_eq!(files, vec![report]);
    assert_eq!(
        view.read_with(cx, |view, cx| view.composer.read(cx).value().to_string()),
        "",
        "pasting a file types nothing"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

/// S11: the prompt names files as the tab's backend reads them: a WSL tab
/// maps both the file and the folder (so one inside the folder stays
/// relative and one outside it is that backend's path), while a tab with no
/// backend yet keeps this computer's spelling.
#[gpui::test]
fn a_prompt_names_its_files_as_the_tabs_backend_reads_them(cx: &mut TestAppContext) {
    let (view, cx) = tab(cx);
    let attached = |name: &str, path: &str| AttachedFile {
        name: name.to_string(),
        path: std::path::PathBuf::from(path),
        canonical: std::path::PathBuf::from(path),
        bytes: 0,
    };
    view.update(cx, |view, _| view.cwd = std::path::PathBuf::from("/work"));
    let files = [
        attached("main.rs", "/work/src/main.rs"),
        attached("a.pdf", "/media/a.pdf"),
    ];
    assert_eq!(
        view.read_with(cx, |view, _| view.prompt_message("look", &files)),
        "look\n\nAttached files:\n- src/main.rs\n- /media/a.pdf"
    );

    view.update(cx, |view, _| {
        view.cwd = std::path::PathBuf::from(r"C:\proj");
        view.runs_on = Some(gentle_core::backend::Backend::Wsl {
            distro: "Ubuntu".to_string(),
        });
    });
    let files = [
        attached("main.rs", r"C:\proj\src\main.rs"),
        attached("a.pdf", r"C:\Users\x\Downloads\a.pdf"),
    ];
    assert_eq!(
        view.read_with(cx, |view, _| view.prompt_message("look", &files)),
        "look\n\nAttached files:\n- src/main.rs\n- /mnt/c/Users/x/Downloads/a.pdf"
    );
}

/// S11, S14/R4: attaching the same file twice keeps one card, whatever route it came
/// by (its path for a file, its bytes for an image), and removing the card
/// lets it be attached again.
#[gpui::test]
fn attaching_the_same_file_twice_keeps_one_card(cx: &mut TestAppContext) {
    let (view, cx) = tab(cx);
    let dir = std::env::temp_dir().join(format!("gs-attach-twice-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let report = dir.join("report.pdf");
    std::fs::write(&report, b"%PDF-1.7\n").unwrap();
    let shot = dir.join("shot.png");
    std::fs::write(&shot, PNG).unwrap();

    view.update_in(cx, |view, window, cx| {
        view.attach_paths(
            vec![report.clone(), report.clone(), shot.clone(), shot.clone()],
            window,
            cx,
        );
    });
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |view, _| view.files.len()), 1);
    assert_eq!(view.read_with(cx, |view, _| view.attachments.len()), 1);

    view.update_in(cx, |view, window, cx| {
        view.remove_file(0, cx);
        view.attach_paths(vec![report.clone()], window, cx);
    });
    cx.run_until_parked();
    assert_eq!(
        view.read_with(cx, |view, _| view.files.len()),
        1,
        "removing the card lets the file be attached again"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

/// S14/R4: one file reached by another spelling is one tile — `./` in its
/// path, a symlink to it — and the tile, the tooltip and the prompt still
/// name the path as it was attached.
#[gpui::test]
fn another_spelling_of_one_file_keeps_one_tile(cx: &mut TestAppContext) {
    let (view, cx) = tab(cx);
    let dir = std::env::temp_dir().join(format!("gs-attach-spelled-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let report = dir.join("report.pdf");
    std::fs::write(&report, b"%PDF-1.7\n").unwrap();
    let dotted = dir.join("./report.pdf");

    view.update_in(cx, |view, window, cx| {
        view.attach_paths(vec![dotted.clone(), report.clone()], window, cx);
    });
    cx.run_until_parked();
    assert_eq!(
        files(&view, cx),
        vec![dotted.clone()],
        "`./report.pdf` and `report.pdf` are one tile, named as it was attached"
    );

    view.update_in(cx, |view, _, cx| view.remove_file(0, cx));
    #[cfg(unix)]
    {
        let link = dir.join("report-link.pdf");
        std::os::unix::fs::symlink(&report, &link).unwrap();
        view.update_in(cx, |view, window, cx| {
            view.attach_paths(vec![report.clone(), link.clone()], window, cx);
        });
        cx.run_until_parked();
        assert_eq!(
            files(&view, cx),
            vec![report],
            "a symlink to an attached file is the same file"
        );
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// S11, S14/R3-2: a file gone from disk when the prompt would be sent stops
/// the send on the path Enter takes, and names it; the text and the tile stay
/// so the user can remove it. A second Enter while the check is in flight is
/// dropped, so one send is stopped once.
#[gpui::test]
fn a_file_gone_before_sending_stops_the_send(cx: &mut TestAppContext) {
    let (view, cx) = rooted_tab(cx);
    let dir = std::env::temp_dir().join(format!("gs-attach-gone-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let report = dir.join("report.pdf");
    std::fs::write(&report, b"%PDF-1.7\n").unwrap();

    view.update_in(cx, |view, window, cx| {
        view.attach_paths(vec![report.clone()], window, cx);
    });
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |view, _| view.files.len()), 1);

    // The file is there: Enter finds nothing to refuse and clears nothing,
    // since this tab never started a backend to send to.
    type_text(&view, cx, "send the report");
    press_enter(&view, cx);
    cx.run_until_parked();
    assert_eq!(warnings(cx), 0, "a file that is there stops nothing");
    assert_eq!(
        composer_text(&view, cx),
        "send the report",
        "with no backend the text stays where it was"
    );
    assert_eq!(view.read_with(cx, |view, _| view.files.len()), 1);

    std::fs::remove_file(&report).unwrap();
    press_enter(&view, cx);
    // A second Enter arrives while the check is in flight: it starts no send.
    press_enter(&view, cx);
    cx.run_until_parked();
    assert_eq!(
        warnings(cx),
        1,
        "the send is stopped once, with a warning naming the file"
    );
    assert_eq!(
        composer_text(&view, cx),
        "send the report",
        "nothing was cleared: the text is still there to send"
    );
    assert_eq!(
        view.read_with(cx, |view, _| view.files.len()),
        1,
        "the tile stays, for the user to remove it"
    );
    assert_eq!(files(&view, cx), vec![report], "and it is still that file");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// T12/S14: the press that asked the disk is the press that may still send.
/// Removing the file's tile while the check runs ends that press: nothing is
/// sent, cleared or remembered, and no warning explains a change the user
/// made (Enter again starts the check over).
#[gpui::test]
fn a_composer_changed_while_the_files_are_checked_sends_nothing(cx: &mut TestAppContext) {
    let (view, cx) = rooted_tab(cx);
    let dir = std::env::temp_dir().join(format!("gs-attach-changed-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let report = dir.join("report.pdf");
    std::fs::write(&report, b"%PDF-1.7\n").unwrap();

    view.update_in(cx, |view, window, cx| {
        view.attach_paths(vec![report], window, cx);
    });
    cx.run_until_parked();
    assert_eq!(files(&view, cx).len(), 1);

    type_text(&view, cx, "send the report");
    let history = view.read_with(cx, |view, _| view.prompt_history.clone());
    press_enter(&view, cx);
    // While the disk is asked, the tile goes.
    view.update(cx, |view, cx| view.remove_file(0, cx));
    cx.run_until_parked();

    assert_eq!(
        warnings(cx),
        0,
        "the user changed it: nothing to warn about"
    );
    assert_eq!(
        composer_text(&view, cx),
        "send the report",
        "nothing was cleared"
    );
    assert_eq!(
        view.read_with(cx, |view, _| view.prompt_history.clone()),
        history,
        "nothing was remembered"
    );
    assert!(
        files(&view, cx).is_empty(),
        "the tile the user removed stays removed"
    );
    assert!(
        !view.read_with(cx, |view, _| view.checking_files),
        "and the next Enter may ask the disk again"
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

/// ↑ recalls a sent prompt while an image waits: only the text changes, so
/// the strip still sends the image it shows.
#[gpui::test]
fn history_recall_leaves_a_waiting_image_in_the_strip(cx: &mut TestAppContext) {
    let (view, cx) = tab(cx);
    let image = Attachment::from_bytes("a.png", PNG.to_vec()).unwrap();
    let key = image.key;
    view.update(cx, |view, _| view.prompt_history.record("first"));
    view.update_in(cx, |view, window, cx| {
        view.attach(vec![Ok(Loaded::Image(image))], window, cx);
    });
    cx.update(|window, cx| {
        let composer = view.read(cx).composer.clone();
        composer.update(cx, |state, cx| {
            state.focus(window, cx);
            state.set_value("draft", window, cx);
        });
    });
    cx.simulate_keystrokes("up");
    cx.run_until_parked();
    assert_eq!(
        view.read_with(cx, |view, cx| view.composer.read(cx).value().to_string()),
        "first"
    );
    let sent: Vec<u64> = view.read_with(cx, |view, _| {
        view.attachments.iter().map(|a| a.key).collect()
    });
    assert_eq!(sent, vec![key]);
}

/// S9: the ↑ history of a resumed conversation holds what was typed, never
/// the `Attached files:` block a prompt carried; a prompt that was only
/// files is not recalled at all.
#[gpui::test]
fn history_recall_leaves_the_file_block_out(cx: &mut TestAppContext) {
    let (view, cx) = tab(cx);
    view.update(cx, |view, _| {
        for text in [
            "look\n\nAttached files:\n- a.pdf",
            "Attached files:\n- b.txt",
            "plain",
        ] {
            view.transcript
                .items
                .push(gentle_core::transcript::Item::User {
                    text: text.into(),
                    images: Vec::new(),
                });
        }
        view.seed_prompt_history();
    });
    cx.update(|window, cx| {
        let composer = view.read(cx).composer.clone();
        composer.update(cx, |state, cx| {
            state.focus(window, cx);
            state.set_value("draft", window, cx);
        });
    });
    let composer = |cx: &mut VisualTestContext| {
        cx.run_until_parked();
        view.read_with(cx, |view, cx| view.composer.read(cx).value().to_string())
    };
    cx.simulate_keystrokes("up");
    assert_eq!(composer(cx), "plain");
    cx.simulate_keystrokes("up");
    assert_eq!(composer(cx), "look", "the block is not recalled");
    cx.simulate_keystrokes("up");
    assert_eq!(composer(cx), "look", "a files-only prompt is not recalled");
}

/// ↑ and ↓ in the composer walk the prompts sent before and come back to
/// the draft; inside a multi-line prompt ↑ moves the cursor first.
#[gpui::test]
fn arrows_recall_sent_prompts_and_return_to_the_draft(cx: &mut TestAppContext) {
    let (view, cx) = tab(cx);
    view.update(cx, |view, _| {
        view.prompt_history.record("first");
        view.prompt_history.record("second\nline two");
    });
    cx.update(|window, cx| {
        let composer = view.read(cx).composer.clone();
        composer.update(cx, |state, cx| {
            state.focus(window, cx);
            state.set_value("draft", window, cx);
        });
    });
    let composer = |cx: &mut VisualTestContext| {
        cx.run_until_parked();
        view.read_with(cx, |view, cx| view.composer.read(cx).value().to_string())
    };
    assert_eq!(composer(cx), "draft");

    cx.simulate_keystrokes("up");
    assert_eq!(composer(cx), "second\nline two");
    // The cursor sits at the end, on the second line: ↑ moves it up first.
    cx.simulate_keystrokes("up");
    assert_eq!(composer(cx), "second\nline two");
    cx.simulate_keystrokes("up");
    assert_eq!(composer(cx), "first");
    cx.simulate_keystrokes("up");
    assert_eq!(composer(cx), "first", "nothing older than the oldest");
    cx.simulate_keystrokes("down");
    assert_eq!(composer(cx), "second\nline two");
    cx.simulate_keystrokes("down");
    assert_eq!(composer(cx), "draft");
}

/// Esc closes what is open first; with nothing open it stops the running
/// turn, unless it comes right after an Esc that closed something.
#[gpui::test]
fn escape_closes_first_then_stops_the_turn(cx: &mut TestAppContext) {
    let (view, cx) = tab(cx);
    let escape = |cx: &mut VisualTestContext| {
        cx.update(|window, cx| {
            view.update(cx, |view, cx| view.abort(&super::Abort, window, cx));
        });
    };
    let stopping = |cx: &mut VisualTestContext| view.read_with(cx, |view, _| view.live.stopping);
    view.update(cx, |view, cx| {
        view.transcript.running = true;
        view.sync_view(cx);
        view.agents_panel.open = true;
    });
    assert!(view.read_with(cx, |view, _| view.turn_since().is_some()));

    escape(cx);
    assert!(view.read_with(cx, |view, _| !view.agents_panel.open));
    assert_eq!(stopping(cx), None, "the first Esc only closed the panel");
    escape(cx);
    assert_eq!(stopping(cx), None, "a quick second Esc doesn't stop");

    view.update(cx, |view, _| view.live.escape_closed = None);
    escape(cx);
    let turn = view.read_with(cx, |view, _| view.turn_since());
    assert_eq!(stopping(cx), turn, "with nothing open, Esc stops the turn");
}

/// Moving an agent to another folder asks first only when that would lose
/// something: what was said, a running turn or a conversation about to resume.
#[gpui::test]
fn has_conversation_counts_turns_and_pending_resumes(cx: &mut TestAppContext) {
    let (view, cx) = tab(cx);
    let has = |cx: &mut VisualTestContext| view.read_with(cx, |view, _| view.has_conversation());
    assert!(!has(cx), "a fresh tab has nothing to lose");

    view.update(cx, |view, _| view.transcript.running = true);
    assert!(has(cx), "a running turn counts");

    view.update(cx, |view, _| {
        view.transcript.running = false;
        view.backend.pending_session = Some("saved.jsonl".into());
    });
    assert!(has(cx), "a conversation waiting to be resumed counts");
}

/// Changing folder starts fresh: a conversation waiting to be resumed is
/// dropped instead of resuming in the new folder.
#[gpui::test]
fn change_cwd_drops_a_pending_resume(cx: &mut TestAppContext) {
    let (view, cx) = tab(cx);
    // Not started yet: the next start would resume it (`--session`).
    view.update(cx, |view, _| {
        view.backend.started = false;
        view.backend.pending_session = Some("saved.jsonl".into());
    });
    let folder = std::env::temp_dir().join("elsewhere");
    view.update_in(cx, |view, window, cx| {
        view.change_cwd(folder.clone(), window, cx)
    });
    view.read_with(cx, |view, _| {
        assert_eq!(view.cwd(), &folder);
        assert!(view.backend.pending_session.is_none());
        assert!(!view.has_conversation());
    });
}

// ── The drop target (S10) ──────────────────────────────────────────────────

/// The drawn bounds of `selector`, after a frame.
fn bounds(
    cx: &mut VisualTestContext,
    selector: &'static str,
) -> gpui_kit::Bounds<gpui_kit::Pixels> {
    cx.run_until_parked();
    cx.debug_bounds(selector)
        .unwrap_or_else(|| panic!("nothing drawn as {selector}"))
}

/// A tab's attached file paths.
fn files(view: &Entity<SessionView>, cx: &mut VisualTestContext) -> Vec<std::path::PathBuf> {
    view.read_with(cx, |view, _| {
        view.files.iter().map(|file| file.path.clone()).collect()
    })
}

/// S10: files dragged over the tab veil it — one layer over the conversation
/// and the composer, icon and words in the middle — the drop attaches them and
/// takes the veil down, and a drag that leaves the tab (or ends without
/// dropping) leaves no veil behind. Nothing about the veil stops the drop.
#[gpui::test]
fn a_file_drag_veils_the_tab_and_the_drop_attaches(cx: &mut TestAppContext) {
    let (view, cx) = tab(cx);
    let dir = std::env::temp_dir().join(format!("gs-drop-veil-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let report = dir.join("report.pdf");
    std::fs::write(&report, b"%PDF-1.7\n").unwrap();
    let dragged = || gpui_kit::ExternalPaths(vec![report.clone()].into());

    let pane = bounds(cx, super::view::DROP_TARGET);
    cx.simulate_event(FileDropEvent::Entered {
        position: pane.center(),
        paths: dragged(),
    });
    let veil = bounds(cx, super::view::DROP_VEIL);
    assert_eq!(
        (veil.origin, veil.size),
        (pane.origin, pane.size),
        "the veil covers the whole tab, the composer included"
    );

    // A drag that leaves the window takes the veil with it.
    cx.simulate_event(FileDropEvent::Exited);
    assert!(
        cx.debug_bounds(super::view::DROP_VEIL).is_none(),
        "no veil once the drag leaves"
    );

    // A drag that moves off the tab, the window still there, clears it too.
    cx.simulate_event(FileDropEvent::Entered {
        position: pane.center(),
        paths: dragged(),
    });
    assert!(cx.debug_bounds(super::view::DROP_VEIL).is_some());
    cx.simulate_event(FileDropEvent::Pending {
        position: point(pane.origin.x + pane.size.width + px(200.), pane.center().y),
    });
    assert!(
        cx.debug_bounds(super::view::DROP_VEIL).is_none(),
        "a drag off the tab takes the veil with it"
    );
    cx.simulate_event(FileDropEvent::Ended);

    // Dropped on the tab: attached, and the veil goes with the drop.
    cx.simulate_event(FileDropEvent::Entered {
        position: pane.center(),
        paths: dragged(),
    });
    assert!(cx.debug_bounds(super::view::DROP_VEIL).is_some());
    cx.simulate_event(FileDropEvent::Submit {
        position: pane.center(),
    });
    cx.run_until_parked();
    assert!(
        cx.debug_bounds(super::view::DROP_VEIL).is_none(),
        "the veil goes once the files are dropped"
    );
    assert_eq!(files(&view, cx), vec![report], "the drop attached the file");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A tab beside sources of file-tree drags, as the tree's rows are: these
/// tests start the drag there and land it on the tab.
struct DragHost {
    tab: Entity<SessionView>,
    file: FileDrag,
    folder: FileDrag,
}

impl Render for DragHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .size_full()
            .child(
                v_flex()
                    .w(px(120.))
                    .h_full()
                    .child(drag_row("file-tree-like-file", self.file.clone()))
                    .child(drag_row("file-tree-like-folder", self.folder.clone())),
            )
            .child(div().flex_1().h_full().child(self.tab.clone()))
    }
}

/// A row that drags `drag`, as the tree's own rows do.
fn drag_row(selector: &'static str, drag: FileDrag) -> impl IntoElement {
    div()
        .id(selector)
        .debug_selector(move || selector.into())
        .h(px(24.))
        .on_drag(drag, |drag, _, _, cx| cx.new(|_| drag.clone()))
}

/// A window with the tab beside a file row and a folder row it can be dragged
/// from.
fn drag_host(
    cx: &mut TestAppContext,
    file: std::path::PathBuf,
    folder: std::path::PathBuf,
) -> (Entity<DragHost>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
    });
    let cwd = std::env::temp_dir();
    let mut built = None;
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let tab = cx.new(|cx| {
            let mut tab = SessionView::new(cwd, None, window, cx, false);
            tab.backend.started = true;
            tab
        });
        let host = cx.new(|_| DragHost {
            tab,
            file: FileDrag::new(file, "report.pdf", false),
            folder: FileDrag::new(folder, "notes", true),
        });
        built = Some(host.clone());
        // The window's root, so a warning (a refused folder) can be shown.
        gpui_kit::base::Root::new(host, window, cx)
    });
    (built.expect("the drag host was built"), cx)
}

/// Drag from the row drawn as `from` toward the middle of the tab, in steps,
/// as a hand would; the caller lets go (a `mouse_up` on the target).
fn drag_onto_the_tab(cx: &mut VisualTestContext, from: &'static str) {
    let start = bounds(cx, from).center();
    let to = bounds(cx, super::view::DROP_TARGET).center();
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
    for step in 1..=6 {
        let t = step as f32 / 6.;
        let at = point(
            start.x + (to.x - start.x) * t,
            start.y + (to.y - start.y) * t,
        );
        cx.simulate_mouse_move(at, Some(MouseButton::Left), Modifiers::none());
        cx.run_until_parked();
    }
}

/// S12: a file dragged out of the tree onto the tab raises the same veil a
/// drag from Finder does and attaches like it; a folder row is refused there
/// too, so it adds no card.
#[gpui::test]
fn dragging_a_file_out_of_the_tree_attaches_it(cx: &mut TestAppContext) {
    let dir = std::env::temp_dir().join(format!("gs-tree-drag-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let report = dir.join("report.pdf");
    std::fs::write(&report, b"%PDF-1.7\n").unwrap();
    let notes = dir.join("notes");
    std::fs::create_dir_all(&notes).unwrap();
    let (host, cx) = drag_host(cx, report.clone(), notes);

    drag_onto_the_tab(cx, "file-tree-like-file");
    assert!(
        cx.debug_bounds(super::view::DROP_VEIL).is_some(),
        "the tree's drag raises the tab's veil"
    );
    let target = bounds(cx, super::view::DROP_TARGET).center();
    cx.simulate_mouse_up(target, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
    assert!(
        cx.debug_bounds(super::view::DROP_VEIL).is_none(),
        "the veil goes with the drop"
    );
    let tab = host.read_with(cx, |host, _| host.tab.clone());
    assert_eq!(
        files(&tab, cx),
        vec![report],
        "dropping the dragged file attached it"
    );

    drag_onto_the_tab(cx, "file-tree-like-folder");
    assert!(
        cx.debug_bounds(super::view::DROP_VEIL).is_some(),
        "the folder drag raises it too: the tab refuses on the rules, not the target"
    );
    let target = bounds(cx, super::view::DROP_TARGET).center();
    cx.simulate_mouse_up(target, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
    assert_eq!(
        files(&tab, cx),
        vec![dir.join("report.pdf")],
        "a folder is refused, so it adds no card"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

// ── The `/` menu (S2, S3) ──────────────────────────────────────────────────

/// A tab whose `/` menu offers `names`, and the screens it asked its window
/// to open, in order.
struct SlashTab {
    view: Entity<SessionView>,
    screens: Rc<RefCell<Vec<Screen>>>,
    _opened: Subscription,
}

impl SlashTab {
    /// The screens the window was asked to open.
    fn opened(&self) -> Vec<Screen> {
        self.screens.borrow().clone()
    }
}

fn slash_tab<'a>(
    names: &[&str],
    cx: &'a mut TestAppContext,
) -> (SlashTab, &'a mut VisualTestContext) {
    let (view, cx) = tab(cx);
    view.update(cx, |view, _| {
        view.commands = names
            .iter()
            .map(|name| ((*name).to_string(), format!("what /{name} does")))
            .collect();
    });
    let screens: Rc<RefCell<Vec<Screen>>> = Rc::default();
    let opened = screens.clone();
    let opened = cx.cx.update(|cx| {
        cx.subscribe(&view, move |_, event: &super::SessionEvent, _| {
            if let super::SessionEvent::OpenScreen(screen) = event {
                opened.borrow_mut().push(*screen);
            }
        })
    });
    (
        SlashTab {
            view,
            screens,
            _opened: opened,
        },
        cx,
    )
}

/// Put `text` in the composer as typing it leaves it: focused, with the `/`
/// menu up for it.
fn type_in_composer(view: &Entity<SessionView>, cx: &mut VisualTestContext, text: &str) {
    let end = text.chars().map(char::len_utf16).sum::<usize>() as u32;
    cx.update(|window, cx| {
        let composer = view.read(cx).composer.clone();
        composer.update(cx, |state, cx| {
            state.focus(window, cx);
            state.set_value(text, window, cx);
            state.set_cursor_position(Position::new(0, end), window, cx);
        });
    });
    cx.run_until_parked();
}

/// The prompts the conversation holds: what a send would have added.
fn sent_prompts(view: &Entity<SessionView>, cx: &mut VisualTestContext) -> usize {
    view.read_with(cx, |view, _| {
        view.transcript
            .items
            .iter()
            .filter(|item| matches!(item, gentle_core::transcript::Item::User { .. }))
            .count()
    })
}

/// The composer's inline tokens as `(id, text, range)`.
fn composer_tokens(
    view: &Entity<SessionView>,
    cx: &mut VisualTestContext,
) -> Vec<(String, String, std::ops::Range<usize>)> {
    view.read_with(cx, |view, cx| {
        view.composer
            .read(cx)
            .tokens()
            .iter()
            .map(|span| {
                (
                    span.token().id().to_string(),
                    span.token().text().to_string(),
                    span.range(),
                )
            })
            .collect()
    })
}

/// S2: one Enter on the highlighted command opens its screen, as sending the
/// command does: it neither fills the text in to wait for a second press nor
/// sends anything twice. Tab still only fills it in.
#[gpui::test]
fn enter_opens_a_highlighted_native_screen_once(cx: &mut TestAppContext) {
    let (tab, cx) = slash_tab(&["skill:architecture-docs", "gentle:profiles"], cx);
    type_in_composer(&tab.view, cx, "/gentle:pro");

    cx.simulate_keystrokes("enter");
    cx.run_until_parked();

    assert_eq!(
        tab.opened(),
        vec![Screen::Profiles],
        "one press opens the screen, once"
    );
    assert_eq!(
        composer_text(&tab.view, cx),
        "",
        "the command left the composer, as a sent one does"
    );
    assert_eq!(sent_prompts(&tab.view, cx), 0, "nothing was sent to Pi");
    assert!(
        tab.view.read_with(cx, |view, cx| !view.slash_menu_open(cx)),
        "the menu is gone with the text"
    );

    // The screen is open and the composer is empty: another Enter sends
    // nothing more, so one press can never dispatch twice.
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(tab.opened(), vec![Screen::Profiles], "and only once");
}

/// S2: Tab completes a command with a native screen without opening it, and
/// Enter fills in a command that has none (a built-in runs on the next
/// press, as before).
#[gpui::test]
fn tab_completes_without_opening_and_other_commands_still_only_fill_in(cx: &mut TestAppContext) {
    let (tab, cx) = slash_tab(&["gentle:profiles"], cx);
    type_in_composer(&tab.view, cx, "/gentle:pro");
    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    assert_eq!(composer_text(&tab.view, cx), "/gentle:profiles ");
    assert!(tab.opened().is_empty(), "Tab completes, it does not open");

    type_in_composer(&tab.view, cx, "/comp");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(
        composer_text(&tab.view, cx),
        "/compact ",
        "a built-in is filled in, not run"
    );
    assert!(tab.opened().is_empty());
    assert_eq!(sent_prompts(&tab.view, cx), 0);
}

/// S2: a command typed in full still opens its screen on one Enter, as it did.
#[gpui::test]
fn a_command_typed_in_full_still_opens_on_enter(cx: &mut TestAppContext) {
    let (tab, cx) = slash_tab(&["gentle:profiles"], cx);
    type_in_composer(&tab.view, cx, "/gentle:profiles");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(tab.opened(), vec![Screen::Profiles]);
    assert_eq!(composer_text(&tab.view, cx), "");
}

/// S3: picking a skill leaves its own `/skill:name` in the prompt as one
/// inline reference marked for the composer, the caret after it for the
/// instructions; nothing opens and nothing is sent.
#[gpui::test]
fn picking_a_skill_inserts_its_reference_and_waits_for_instructions(cx: &mut TestAppContext) {
    let (tab, cx) = slash_tab(&["skill:architecture-docs", "gentle:profiles"], cx);
    type_in_composer(&tab.view, cx, "/skill:arch");

    cx.simulate_keystrokes("enter");
    cx.run_until_parked();

    assert_eq!(composer_text(&tab.view, cx), "/skill:architecture-docs ");
    assert_eq!(
        composer_tokens(&tab.view, cx),
        vec![(
            "skill-reference".to_string(),
            "/skill:architecture-docs".to_string(),
            0.."/skill:architecture-docs".len(),
        )],
        "the command itself, kept as one marked reference"
    );
    let cursor = tab
        .view
        .read_with(cx, |view, cx| view.composer.read(cx).cursor());
    assert_eq!(
        cursor,
        "/skill:architecture-docs ".len(),
        "the caret waits where the instructions go"
    );
    assert!(tab.opened().is_empty(), "a skill opens nothing");
    assert_eq!(sent_prompts(&tab.view, cx), 0, "and nothing is sent");
    assert!(tab.view.read_with(cx, |view, cx| !view.slash_menu_open(cx)));

    // Typed at the caret: the instructions follow the reference, which stays
    // one reference in the prompt.
    tab.view.update_in(cx, |view, window, cx| {
        view.composer.update(cx, |state, cx| {
            state.replace("summarize the ADRs", window, cx)
        });
    });
    cx.run_until_parked();
    assert_eq!(
        composer_text(&tab.view, cx),
        "/skill:architecture-docs summarize the ADRs"
    );
    assert_eq!(composer_tokens(&tab.view, cx).len(), 1);
}

/// S2 and S3: a click on a row does what Enter on it does, so the pointer and
/// the keyboard leave the composer in the same state.
#[gpui::test]
fn clicking_a_row_does_what_enter_does(cx: &mut TestAppContext) {
    let (tab, cx) = slash_tab(&["skill:architecture-docs", "gentle:profiles"], cx);

    type_in_composer(&tab.view, cx, "/skill:arch");
    let row = bounds(cx, "slash-command-0").center();
    cx.simulate_mouse_down(row, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(row, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
    assert_eq!(composer_text(&tab.view, cx), "/skill:architecture-docs ");
    assert_eq!(composer_tokens(&tab.view, cx).len(), 1);
    assert!(tab.opened().is_empty());

    type_in_composer(&tab.view, cx, "/gentle:pro");
    let row = bounds(cx, "slash-command-0").center();
    cx.simulate_mouse_down(row, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(row, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
    assert_eq!(tab.opened(), vec![Screen::Profiles]);
    assert_eq!(composer_text(&tab.view, cx), "");
}

/// S3: a pick replaces only the `/query` at the caret — `/skill:arch`, or the
/// `/arch` the menu matched by name — leaving the prose and a paste's chip
/// around it; with no query at the caret nothing is picked.
#[gpui::test]
fn picking_a_skill_replaces_only_the_query_at_the_caret(cx: &mut TestAppContext) {
    let (tab, cx) = slash_tab(&["skill:architecture-docs"], cx);
    let item = gpui_kit::ClipboardItem::new_string("fn main() {}\n".repeat(8));
    let (prefix, rest, keep) = ("[Pasted text #1] ", "keep /skill:arch and this", "keep ");
    let name = "/skill:architecture-docs";
    let caret = format!("{prefix}{keep}/skill:arch").len() as u32;
    tab.view.update_in(cx, |view, window, cx| {
        view.paste_clipboard(&item, window, cx);
        view.composer.update(cx, |state, cx| {
            state.replace(rest, window, cx);
            state.set_cursor_position(Position::new(0, caret), window, cx)
        });
        view.insert_skill_reference("skill:architecture-docs", window, cx);
    });
    let picked = format!("{prefix}{keep}{name} and this");
    assert_eq!(composer_text(&tab.view, cx), picked);
    let (tokens, start) = (composer_tokens(&tab.view, cx), prefix.len() + keep.len());
    assert_eq!(tokens.len(), 2, "the paste's chip and the skill's");
    assert_eq!(tokens[0].0, "pasted-text");
    assert_eq!(tokens[1].0, "skill-reference");
    assert_eq!(tokens[1].1, name);
    assert_eq!(tokens[1].2, start..start + name.len());
    // No query at the caret: nothing is picked.
    type_in_composer(&tab.view, cx, "draft");
    tab.view.update_in(cx, |view, window, cx| {
        view.insert_skill_reference("skill:docs", window, cx)
    });
    assert_eq!(composer_text(&tab.view, cx), "draft");
    type_in_composer(&tab.view, cx, "/arch");
    cx.simulate_keystrokes("enter");
    let completed = format!("{name} ");
    assert_eq!(composer_text(&tab.view, cx), completed);
    assert_eq!(composer_tokens(&tab.view, cx).len(), 1);
}

/// S3: the reference keeps its marking when the composer is rebuilt from its
/// text — removing a paste card takes that chip out and leaves the skill's.
#[gpui::test]
fn removing_a_paste_card_keeps_the_skill_reference_marked(cx: &mut TestAppContext) {
    let (tab, cx) = slash_tab(&["skill:architecture-docs"], cx);
    type_in_composer(&tab.view, cx, "/skill:arch");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();

    let item = gpui_kit::ClipboardItem::new_string("fn main() {}\n".repeat(8));
    tab.view.update_in(cx, |view, window, cx| {
        view.paste_clipboard(&item, window, cx)
    });
    cx.run_until_parked();
    assert_eq!(
        composer_text(&tab.view, cx),
        "/skill:architecture-docs [Pasted text #1] "
    );

    tab.view
        .update_in(cx, |view, window, cx| view.remove_pasted(0, window, cx));
    cx.run_until_parked();
    assert_eq!(
        composer_text(&tab.view, cx),
        "/skill:architecture-docs ",
        "the paste's card and token went"
    );
    assert_eq!(
        composer_tokens(&tab.view, cx),
        vec![(
            "skill-reference".to_string(),
            "/skill:architecture-docs".to_string(),
            0.."/skill:architecture-docs".len(),
        )],
        "the skill keeps its marking through the rebuild"
    );
}

/// S3: a prompt recalled with ↑ keeps the marking of the skill it invokes,
/// and leaves a lookalike the backend never listed as plain text.
#[gpui::test]
fn a_recalled_prompt_keeps_its_skill_reference_marked(cx: &mut TestAppContext) {
    let (tab, cx) = slash_tab(&["skill:architecture-docs"], cx);
    tab.view.update(cx, |view, _| {
        view.prompt_history
            .record("/skill:architecture-docs summarize the ADRs");
    });
    type_in_composer(&tab.view, cx, "draft");

    cx.simulate_keystrokes("up");
    cx.run_until_parked();
    assert_eq!(
        composer_text(&tab.view, cx),
        "/skill:architecture-docs summarize the ADRs"
    );
    assert_eq!(
        composer_tokens(&tab.view, cx),
        vec![(
            "skill-reference".to_string(),
            "/skill:architecture-docs".to_string(),
            0.."/skill:architecture-docs".len(),
        )],
        "the recalled command is still the known skill's reference"
    );
}

/// S3: a command that only looks like a skill stays plain — one the backend
/// never listed, a listed one with more letters after it, and one glued to
/// the word before it.
#[gpui::test]
fn lookalike_commands_stay_plain_through_a_rebuild(cx: &mut TestAppContext) {
    let (tab, cx) = slash_tab(&["skill:architecture-docs"], cx);
    for prompt in [
        "/skill:unknown-thing summarize",
        "/skill:architecture-docsx summarize",
        "see/skill:architecture-docs",
        "/skills:architecture-docs",
    ] {
        tab.view.update(cx, |view, _| {
            view.prompt_history.record(prompt);
        });
        type_in_composer(&tab.view, cx, "draft");
        cx.simulate_keystrokes("up");
        cx.run_until_parked();
        assert_eq!(composer_text(&tab.view, cx), prompt);
        assert!(
            composer_tokens(&tab.view, cx).is_empty(),
            "{prompt} is no known skill's reference"
        );
        // Back to the draft, ready for the next prompt.
        cx.simulate_keystrokes("down");
        cx.run_until_parked();
        assert_eq!(composer_text(&tab.view, cx), "draft");
    }
}

/// A long paste keeps its own `#N` chip (the token the skill reference shares
/// the renderer with) and still goes out expanded.
#[gpui::test]
fn a_long_paste_keeps_its_chip(cx: &mut TestAppContext) {
    let (view, cx) = tab(cx);
    let pasted = "fn main() {}\n".repeat(8);
    let item = gpui_kit::ClipboardItem::new_string(pasted);
    let kept = view.update_in(cx, |view, window, cx| {
        view.paste_clipboard(&item, window, cx)
    });
    cx.run_until_parked();
    assert!(kept, "eight lines are a long paste");
    assert_eq!(composer_text(&view, cx), "[Pasted text #1] ");
    assert_eq!(
        composer_tokens(&view, cx),
        vec![(
            "pasted-text".to_string(),
            "[Pasted text #1]".to_string(),
            0.."[Pasted text #1]".len(),
        )]
    );
}
