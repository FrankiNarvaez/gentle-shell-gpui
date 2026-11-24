//! Interaction tests: a real [`SessionView`] (no backend started) in a
//! headless window, its Desktop channel fed the records Gentle Shell would
//! send, driven by simulated clicks.

// Not a glob: GPUI exports a `test` attribute that would shadow `#[test]`.
use gpui_kit::{Entity, Modifiers, TestAppContext, VisualTestContext};
use pi_rpc::desktop_rpc::{PROMPT_COMMAND, SCHEMA, WIDGET_KEY};
use pi_rpc::{UiMethod, UiRequest};
use serde_json::{Value, json};

use super::attachments::{Attachment, Loaded};
use super::{AgentsTab, SessionView};

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
