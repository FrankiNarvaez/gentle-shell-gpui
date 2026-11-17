//! Code refs, pinning, the Export menu's actions and templates: real views
//! in headless windows, with the clipboard, the save dialog and the data
//! folder captured in a temporary directory.

use std::path::{Path, PathBuf};

use gentle_core::genui::Snapshot;
use gentle_core::genui::export::Format;
use gentle_core::genui::template::Template;
use gentle_core::transcript::{Item, ToolStatus};
// Not a glob: GPUI exports a `test` attribute that would shadow `#[test]`.
use gpui_kit::{
    AppContext as _, Context, Entity, IntoElement, Modifiers, ParentElement as _, Render,
    Styled as _, Subscription, TestAppContext, VisualTestContext, Window, div, px,
};
use serde_json::{Value, json};

use super::super::GenUi;
use super::super::diagram::DiagramEvent;
use super::super::panel::PanelView;
use super::super::share::{self, Action, RefState};
use super::super::store::{self, StoreDir};
use super::super::templates::{TemplatesEvent, TemplatesView};
use super::Seen;

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("genui-links-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("docs")).unwrap();
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/a.rs"), "fn a() {}\n").unwrap();
    dir
}

struct Host {
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

fn init(cx: &mut TestAppContext, root: &Path) {
    let data = root.join("data");
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
        cx.set_global(StoreDir(data));
    });
}

fn open<'a>(
    cx: &'a mut TestAppContext,
    kind: &str,
    spec: Value,
    root: &Path,
) -> (Entity<Host>, &'a mut VisualTestContext) {
    init(cx, root);
    let snapshot = Snapshot {
        id: format!("ui-{kind}"),
        version: 1,
        kind: kind.into(),
        title: format!("Test {kind}"),
        spec,
        closed: false,
    };
    let root = root.to_path_buf();
    let (host, cx) = cx.add_window_view(|_, cx| {
        let view = cx.new(|cx| {
            let mut view = PanelView::new(&snapshot, cx);
            view.env.root = Some(root);
            view
        });
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

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    cx.run_until_parked();
    let at = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("nothing drawn as {selector}"))
        .center();
    cx.simulate_click(at, Modifiers::none());
    cx.run_until_parked();
}

fn seen(host: &Entity<Host>, cx: &mut VisualTestContext) -> Vec<Seen> {
    cx.update(|_, cx| host.update(cx, |host, _| std::mem::take(&mut host.seen)))
}

fn run(host: &Entity<Host>, action: Action, cx: &mut VisualTestContext) {
    cx.update(|_, cx| {
        let view = host.read(cx).view.clone();
        view.update(cx, |view, cx| share::run(action, view.source(), cx));
    });
    cx.run_until_parked();
}

#[gpui::test]
fn clicking_a_code_ref_asks_to_open_it(cx: &mut TestAppContext) {
    let root = temp("ref");
    let (host, cx) = open(
        cx,
        "checklist",
        json!({"items": [
            {"id": "a", "label": "Wire it", "ref": {"path": "src/a.rs", "line": 3}},
            {"id": "b", "label": "Later", "ref": "src/missing.rs"}
        ]}),
        &root,
    );
    click(cx, "genui-item-ref-a");
    assert_eq!(seen(&host, cx), [Seen::OpenRef("src/a.rs:3".into())]);
    let states = cx.update(|_, cx| {
        let view = host.read(cx).view.read(cx);
        let list = view.source().spec;
        ["a", "b"].map(|id| {
            let item = list["items"]
                .as_array()
                .unwrap()
                .iter()
                .find(|i| i["id"] == id)
                .unwrap();
            let code_ref = gentle_core::genui::refs::CodeRef::parse(&item["ref"]).expect("a ref");
            view.env.ref_state(&code_ref)
        })
    });
    assert_eq!(states[0], RefState::Found(root.join("src/a.rs")));
    assert_eq!(states[1], RefState::Missing(root.join("src/missing.rs")));
    // The missing one still asks: the conversation says it isn't there.
    click(cx, "genui-item-ref-b");
    assert_eq!(seen(&host, cx), [Seen::OpenRef("src/missing.rs".into())]);
    let _ = std::fs::remove_dir_all(root);
}

#[gpui::test]
fn an_erd_tables_ref_opens_from_its_header(cx: &mut TestAppContext) {
    let root = temp("erd-ref");
    let (host, cx) = open(
        cx,
        "erd",
        json!({"tables": [
            {"id": "users", "ref": "src/a.rs:1", "columns": [{"name": "id", "type": "uuid", "pk": true}]},
            {"id": "orders", "columns": [{"name": "id", "pk": true}, {"name": "user_id", "fk": "users.id"}]}
        ]}),
        &root,
    );
    // The first frame measures the canvas, the second fits the tables in it.
    for _ in 0..2 {
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
    }
    assert!(cx.debug_bounds("genui-erd-table-orders").is_some());
    assert!(cx.debug_bounds("genui-erd-ref-orders").is_none(), "no ref");
    click(cx, "genui-erd-ref-users");
    assert_eq!(seen(&host, cx), [Seen::OpenRef("src/a.rs:1".into())]);
    let _ = std::fs::remove_dir_all(root);
}

#[gpui::test]
fn a_review_exports_as_a_patch(cx: &mut TestAppContext) {
    let root = temp("export-review");
    let (host, cx) = open(
        cx,
        "review",
        json!({"files": [{"path": "src/a.rs", "hunks": [{"oldStart": 1, "newStart": 1, "lines": ["-fn a() {}", "+fn b() {}"]}]}]}),
        &root,
    );
    run(&host, Action::Copy(Format::Patch), cx);
    let copied = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(
        copied.as_deref(),
        Some(
            "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,1 +1,1 @@\n-fn a() {}\n+fn b() {}\n"
        )
    );
    assert!(matches!(seen(&host, cx).as_slice(), [Seen::Notice(Ok(_))]));
    let _ = std::fs::remove_dir_all(root);
}

#[gpui::test]
fn the_pin_button_pins_and_unpins(cx: &mut TestAppContext) {
    let root = temp("pin");
    let (host, cx) = open(cx, "checklist", json!({"items": ["One"]}), &root);
    click(cx, "genui-pin");
    assert_eq!(seen(&host, cx), [Seen::Pin(true)]);
    cx.update(|_, cx| {
        let view = host.read(cx).view.clone();
        view.update(cx, |view, cx| {
            view.env.pinned = true;
            cx.notify();
        });
    });
    click(cx, "genui-pin");
    assert_eq!(seen(&host, cx), [Seen::Pin(false)]);
    let _ = std::fs::remove_dir_all(root);
}

fn tool_item(id: &str, kind: &str, spec: Value) -> Item {
    Item::Tool {
        call_id: format!("call-{id}"),
        name: gentle_core::genui::TOOL.into(),
        args: json!({"action": "show", "id": id, "kind": kind}),
        status: ToolStatus::Done,
        output: String::new(),
        details: json!({"id": id, "version": 1, "kind": kind, "title": id, "spec": spec}),
        duration_ms: None,
    }
}

#[gpui::test]
fn pins_show_in_the_side_panel_and_come_back_with_the_session(cx: &mut TestAppContext) {
    let root = temp("pins");
    init(cx, &root);
    let session = root.join("session.jsonl");
    std::fs::write(&session, "").unwrap();
    let items = vec![
        tool_item("plan", "checklist", json!({"items": ["One"]})),
        tool_item(
            "board",
            "kanban",
            json!({"columns": ["To do"], "cards": []}),
        ),
    ];
    cx.update(|cx| {
        let mut genui = GenUi::default();
        genui.set_root(&root, cx);
        genui.sync(&items, 0, true, cx);
        assert!(genui.render_pinned(cx).is_none());
        genui.pin("plan", true, Some(&session), cx);
        genui.pin("board", true, Some(&session), cx);
        assert_eq!(genui.pins(), ["plan", "board"]);
        assert!(genui.render_pinned(cx).is_some());
        genui.pin("board", false, Some(&session), cx);
        assert_eq!(genui.pins(), ["plan"]);

        // Reopening the conversation pins it again.
        let mut reopened = GenUi::default();
        reopened.sync(&items, 0, true, cx);
        reopened.restore_pins(Some(&session), cx);
        assert_eq!(reopened.pins(), ["plan"]);
        reopened.pin("plan", false, Some(&session), cx);
    });
    assert!(store::pins(&root.join("data"), &session).is_empty());
    let _ = std::fs::remove_dir_all(root);
}

fn table_spec() -> Value {
    json!({
        "columns": [{"key": "name", "label": "Name"}, {"key": "fails", "label": "Fails", "type": "number"}],
        "rows": [{"id": "login", "cells": {"name": "login, flow", "fails": 4}, "ref": "src/a.rs:1"}]
    })
}

#[gpui::test]
fn export_copies_saves_asks_for_docs_and_saves_templates(cx: &mut TestAppContext) {
    let root = temp("export");
    let (host, cx) = open(cx, "table", table_spec(), &root);

    run(&host, Action::Copy(Format::Csv), cx);
    let copied = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(
        copied.as_deref(),
        Some("Name,Fails,ref\n\"login, flow\",4,src/a.rs:1\n")
    );
    assert!(matches!(seen(&host, cx).as_slice(), [Seen::Notice(Ok(_))]));

    run(&host, Action::Save(Format::Markdown), cx);
    assert!(cx.did_prompt_for_new_path());
    let target = root.join("docs/flaky.md");
    let expected_dir = root.join("docs");
    cx.simulate_new_path_selection(|dir| {
        assert_eq!(dir, expected_dir);
        Some(target.clone())
    });
    cx.run_until_parked();
    let saved = std::fs::read_to_string(&target).expect("the file was written");
    assert!(saved.starts_with("| Name | Fails | Code |\n| --- | ---: | --- |\n"));
    assert!(matches!(seen(&host, cx).as_slice(), [Seen::Notice(Ok(_))]));

    run(&host, Action::Docs, cx);
    let request = match seen(&host, cx).as_slice() {
        [Seen::Request(command)] => command.clone(),
        other => panic!("expected a docs request, got {other:?}"),
    };
    let event = super::event_json(&request);
    assert_eq!(
        (
            event["op"].as_str(),
            event["id"].as_str(),
            event["format"].as_str()
        ),
        (Some("docs"), Some("ui-table"), Some("Markdown"))
    );

    run(&host, Action::Template, cx);
    assert!(matches!(seen(&host, cx).as_slice(), [Seen::Notice(Ok(_))]));
    let saved = store::templates(&root.join("data"));
    assert_eq!(saved.len(), 1);
    assert_eq!(
        (saved[0].1.name.as_str(), saved[0].1.kind.as_str()),
        ("Test table", "table")
    );
    assert_eq!(saved[0].1.spec["rows"][0]["ref"]["path"], "src/a.rs");
    let _ = std::fs::remove_dir_all(root);
}

struct Manager {
    view: Entity<TemplatesView>,
    inserted: Vec<Template>,
    _subscription: Subscription,
}

impl Render for Manager {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(440.)).child(self.view.clone())
    }
}

#[gpui::test]
fn templates_insert_rename_and_delete(cx: &mut TestAppContext) {
    let root = temp("templates");
    init(cx, &root);
    let data = root.join("data");
    let template = Template {
        name: "Release plan".into(),
        kind: "checklist".into(),
        title: "Release".into(),
        spec: json!({"items": [{"id": "tag", "label": "Tag it"}]}),
        saved_at: 1,
    };
    store::save_template(&data, template.clone()).unwrap();
    let (manager, cx) = cx.add_window_view(|_, cx| {
        let view = cx.new(TemplatesView::new);
        let subscription = cx.subscribe(&view, |m: &mut Manager, _, event, _| {
            if let TemplatesEvent::Insert(t) = event {
                m.inserted.push(t.clone());
            }
        });
        Manager {
            view,
            inserted: Vec::new(),
            _subscription: subscription,
        }
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("genui-template-0").is_some());
    let view = cx.update(|_, cx| manager.read(cx).view.clone());
    cx.update(|_, cx| view.update(cx, |view, cx| view.insert(0, cx)));
    let inserted = cx.update(|_, cx| manager.read(cx).inserted.clone());
    assert_eq!(inserted, std::slice::from_ref(&template));
    // What the conversation sends for it.
    let command = GenUi::default().insert_command(&inserted[0]);
    let event = super::event_json(&command);
    assert_eq!(
        (event["op"].as_str(), event["id"].as_str()),
        (Some("insert"), Some("release-plan"))
    );

    let path = cx.update(|_, cx| view.read(cx).list()[0].0.clone());
    cx.update(|_, cx| view.update(cx, |view, cx| view.rename(&path, "Ship it", cx)));
    let names = cx.update(|_, cx| view.read(cx).names());
    assert_eq!(names, ["Ship it"]);
    cx.update(|_, cx| view.update(cx, |view, cx| view.delete(0, cx)));
    assert!(cx.update(|_, cx| view.read(cx).names()).is_empty());
    assert!(store::templates(&data).is_empty());
    let _ = std::fs::remove_dir_all(root);
}
