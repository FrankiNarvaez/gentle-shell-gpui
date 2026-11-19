//! The diagram, driven through a real (headless) window: native pointer
//! and key events reach the view's own listeners.

use std::cell::RefCell;
use std::rc::Rc;

use gentle_core::genui::Outcome;
use gentle_core::genui::Snapshot;
use gentle_core::genui::changes::Change;
use gentle_core::genui::diagram::{Direction, NodeKind, Status};
use gentle_core::genui::layout;
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, Context, Entity, InputEvent as _, IntoElement,
    KeyDownEvent, KeyUpEvent, Keystroke, Modifiers, ModifiersChangedEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement as _, Pixels, Point, Render,
    ScrollDelta, ScrollEvent, Styled as _, Subscription, TestAppContext, VisualTestContext, Window,
    WindowBounds, WindowOptions, div, point, px, size,
};
use serde_json::{Value, json};

use super::super::diagram::{DiagramEvent, DiagramView, Selection, SendState};
use super::{Seen, event_json, ops};

/// The conversation's stand-in: shows the surface and records its events.
struct Host {
    view: Entity<DiagramView>,
    _events: Subscription,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.view.clone())
    }
}

struct Harness {
    window: AnyWindowHandle,
    view: Entity<DiagramView>,
    seen: Rc<RefCell<Vec<Seen>>>,
}

fn spec() -> Value {
    json!({
        "groups": [{"id": "back", "label": "Backend"}],
        "nodes": [
            {"id": "web", "label": "Web", "kind": "client"},
            {"id": "api", "label": "API", "group": "back"},
            {"id": "db", "label": "Orders DB", "kind": "database", "group": "back"},
            {"id": "mail", "label": "Mail", "kind": "external"},
        ],
        "edges": [
            {"from": "web", "to": "api", "label": "HTTPS"},
            {"from": "api", "to": "db", "style": "data"},
        ],
    })
}

fn open(cx: &mut TestAppContext) -> Harness {
    cx.update(gpui_kit::init);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let log = seen.clone();
    let mut surface = None;
    let window = cx
        .update(|cx| {
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: Point::default(),
                        size: size(px(1100.), px(720.)),
                    })),
                    ..Default::default()
                },
                cx,
                |_, cx| {
                    let snapshot = Snapshot {
                        id: "ui-1".into(),
                        version: 1,
                        kind: "diagram".into(),
                        title: "Checkout".into(),
                        spec: spec(),
                        closed: false,
                    };
                    let view = cx.new(|cx| DiagramView::new(&snapshot, cx));
                    surface = Some(view.clone());
                    cx.new(|cx| Host {
                        _events: cx.subscribe(&view, move |_, _, event: &DiagramEvent, _| {
                            log.borrow_mut().push(Seen::from(event))
                        }),
                        view,
                    })
                },
            )
        })
        .expect("open the test window")
        .0;
    let harness = Harness {
        window,
        view: surface.expect("the surface was built"),
        seen,
    };
    // The first frame measures the canvas, the second fits the diagram in it.
    harness.frame(cx);
    harness.frame(cx);
    harness
}

impl Harness {
    fn with<R>(&self, cx: &mut TestAppContext, f: impl FnOnce(&mut Window, &mut App) -> R) -> R {
        cx.update_window(self.window, |_, window, cx| f(window, cx))
            .expect("the window is open")
    }

    fn frame(&self, cx: &mut TestAppContext) {
        self.with(cx, draw);
    }

    fn read<R>(&self, cx: &mut TestAppContext, f: impl FnOnce(&DiagramView) -> R) -> R {
        cx.read(|cx| f(self.view.read(cx)))
    }

    fn update<R>(
        &self,
        cx: &mut TestAppContext,
        f: impl FnOnce(&mut DiagramView, &mut Context<DiagramView>) -> R,
    ) -> R {
        let r = cx.update(|cx| self.view.update(cx, f));
        self.frame(cx);
        r
    }

    /// Finish the running fit/zoom/arrival animations.
    fn settle(&self, cx: &mut TestAppContext) {
        self.update(cx, |view, _| view.settle());
    }

    /// Where a diagram point is on the window.
    fn on_window(&self, cx: &mut TestAppContext, p: layout::Point) -> Point<Pixels> {
        let view = self.view.clone();
        self.with(cx, |window, cx| {
            view.read(cx).to_window(p, window.rem_size())
        })
    }

    fn node_rect(&self, cx: &mut TestAppContext, id: &str) -> layout::Rect {
        self.read(cx, |v| v.layout.nodes[v.working.node_index(id).unwrap()])
    }

    /// A node's center on the window.
    fn node(&self, cx: &mut TestAppContext, id: &str) -> Point<Pixels> {
        let center = self.node_rect(cx, id).center();
        self.on_window(cx, center)
    }

    /// A canvas point with no node, edge or floating panel under it.
    fn empty(&self, cx: &mut TestAppContext) -> Point<Pixels> {
        let view = self.view.clone();
        self.with(cx, |window, cx| {
            let view = view.read(cx);
            let rem = window.rem_size();
            let canvas = view.bounds.get();
            let (w, h) = (f32::from(canvas.size.width), f32::from(canvas.size.height));
            let mut y = 30.;
            while y < h * 0.6 {
                let mut x = 30.;
                while x < w * 0.6 {
                    let world = view.to_world(point(px(x), px(y)), rem);
                    let clear = view.layout.node_at(world).is_none()
                        && view.layout.edge_at(world, 20.).is_none()
                        && view
                            .layout
                            .groups
                            .iter()
                            .flatten()
                            .all(|g| !g.contains(world));
                    if clear {
                        return canvas.origin + point(px(x), px(y));
                    }
                    x += 20.;
                }
                y += 20.;
            }
            panic!("no empty spot on the canvas")
        })
    }

    /// Click (`count` times) on a node's center.
    fn click_node(&self, cx: &mut TestAppContext, id: &str, count: usize) {
        let at = self.node(cx, id);
        self.click(cx, at, count, Modifiers::none());
    }

    fn click(
        &self,
        cx: &mut TestAppContext,
        at: Point<Pixels>,
        count: usize,
        modifiers: Modifiers,
    ) {
        self.with(cx, |window, cx| {
            pointer(
                window,
                MouseMoveEvent {
                    position: at,
                    pressed_button: None,
                    modifiers,
                },
                cx,
            );
            for click_count in 1..=count {
                pointer(
                    window,
                    MouseDownEvent {
                        button: MouseButton::Left,
                        position: at,
                        modifiers,
                        click_count,
                        first_mouse: false,
                    },
                    cx,
                );
                pointer(
                    window,
                    MouseUpEvent {
                        button: MouseButton::Left,
                        position: at,
                        modifiers,
                        click_count,
                    },
                    cx,
                );
            }
        });
    }

    fn hover(&self, cx: &mut TestAppContext, at: Point<Pixels>) {
        self.with(cx, |window, cx| {
            pointer(
                window,
                MouseMoveEvent {
                    position: at,
                    pressed_button: None,
                    modifiers: Modifiers::none(),
                },
                cx,
            )
        });
    }

    /// Press at `from`, move there in eight steps, release at `to`.
    fn drag_with(
        &self,
        cx: &mut TestAppContext,
        from: Point<Pixels>,
        to: Point<Pixels>,
        modifiers: Modifiers,
    ) {
        self.with(cx, |window, cx| {
            pointer(
                window,
                MouseMoveEvent {
                    position: from,
                    pressed_button: None,
                    modifiers,
                },
                cx,
            );
            pointer(
                window,
                MouseDownEvent {
                    button: MouseButton::Left,
                    position: from,
                    modifiers,
                    click_count: 1,
                    first_mouse: false,
                },
                cx,
            );
            for step in 1..=8 {
                let t = step as f32 / 8.;
                let position = point(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t);
                pointer(
                    window,
                    MouseMoveEvent {
                        position,
                        pressed_button: Some(MouseButton::Left),
                        modifiers,
                    },
                    cx,
                );
            }
            pointer(
                window,
                MouseUpEvent {
                    button: MouseButton::Left,
                    position: to,
                    modifiers,
                    click_count: 1,
                },
                cx,
            );
        });
    }

    fn drag(&self, cx: &mut TestAppContext, from: Point<Pixels>, to: Point<Pixels>) {
        self.drag_with(cx, from, to, Modifiers::none());
    }

    /// Where the element with this debug selector was drawn last frame.
    fn find(&self, cx: &mut TestAppContext, selector: &'static str) -> Option<Bounds<Pixels>> {
        self.frame(cx);
        VisualTestContext::from_window(self.window, cx).debug_bounds(selector)
    }

    fn has(&self, cx: &mut TestAppContext, selector: &'static str) -> bool {
        self.find(cx, selector).is_some()
    }

    /// Click a button by its id (also its debug selector).
    fn button(&self, cx: &mut TestAppContext, id: &'static str) {
        let bounds = self
            .find(cx, id)
            .unwrap_or_else(|| panic!("no {id} on screen"));
        self.click(cx, bounds.center(), 1, Modifiers::none());
    }

    /// A key as GPUI parses it (`secondary-z` is ⌘Z on macOS, Ctrl-Z
    /// elsewhere); Enter runs its bindings like a native one.
    fn press(&self, cx: &mut TestAppContext, key: &str) {
        self.with(cx, |window, cx| {
            let keystroke =
                Keystroke::parse_with_os(key, cx.platform_info().os).expect("a valid keystroke");
            let keystroke = if keystroke.key == "enter" {
                window.dispatch_event(
                    KeyDownEvent {
                        keystroke: keystroke.clone(),
                        is_held: false,
                        prefer_character_input: false,
                    }
                    .to_platform_input(),
                    cx,
                );
                keystroke
            } else {
                let keystroke = keystroke.with_simulated_ime();
                window.dispatch_keystroke(keystroke.clone(), cx);
                keystroke
            };
            window.dispatch_event(KeyUpEvent { keystroke }.to_platform_input(), cx);
            draw(window, cx);
        });
    }

    /// Type `text` into whatever has the keyboard.
    fn input(&self, cx: &mut TestAppContext, text: &str) {
        self.with(cx, |window, cx| {
            for c in text.chars() {
                let text = c.to_string();
                let mut keystroke = Keystroke::parse_with_os(&text, cx.platform_info().os)
                    .expect("a character is a keystroke");
                keystroke.key_char = Some(text);
                window.dispatch_keystroke(keystroke, cx);
                draw(window, cx);
            }
        });
    }

    fn modifiers(&self, cx: &mut TestAppContext, modifiers: Modifiers) {
        self.with(cx, |window, cx| {
            window.dispatch_event(
                ModifiersChangedEvent {
                    modifiers,
                    capslock: Default::default(),
                }
                .to_platform_input(),
                cx,
            );
            draw(window, cx);
        });
    }

    fn sends(&self) -> Vec<Value> {
        self.seen
            .borrow()
            .iter()
            .filter_map(|seen| match seen {
                Seen::Send(command) => Some(event_json(command)),
                _ => None,
            })
            .collect()
    }
}

/// Lay out and paint a frame, as the platform would after a change.
fn draw(window: &mut Window, cx: &mut App) {
    window.refresh();
    window.draw(cx).clear(cx);
}

/// One native pointer event, then a frame.
fn pointer(window: &mut Window, event: impl gpui_kit::InputEvent, cx: &mut App) {
    window.dispatch_event(event.to_platform_input(), cx);
    draw(window, cx);
}

#[gpui::test]
fn dragging_the_background_pans_and_a_click_clears_the_selection(cx: &mut TestAppContext) {
    let h = open(cx);
    h.click_node(cx, "api", 1);
    assert_eq!(
        h.read(cx, |v| v.selection.clone()),
        Some(Selection::Node("api".into()))
    );

    let pan = h.read(cx, |v| v.viewport.pan);
    let from = h.empty(cx);
    h.drag(cx, from, from + point(px(60.), px(-25.)));
    let after = h.read(cx, |v| v.viewport.pan);
    assert_eq!((after.x - pan.x, after.y - pan.y), (60., -25.));
    assert!(h.read(cx, |v| v.viewport.touched));
    // A pan is not a click: the selection stays; a plain click clears it.
    assert!(h.read(cx, |v| v.selection.is_some()));
    let empty = h.empty(cx);
    h.click(cx, empty, 1, Modifiers::none());
    assert_eq!(h.read(cx, |v| v.selection.clone()), None);
    assert_eq!(h.read(cx, |v| v.pending), 0);
}

#[gpui::test]
fn zoom_buttons_ease_and_secondary_scroll_zooms_at_the_pointer(cx: &mut TestAppContext) {
    let h = open(cx);
    let zoom = h.read(cx, |v| v.viewport.zoom);
    h.button(cx, "genui-zoom-in");
    h.settle(cx);
    let zoomed = h.read(cx, |v| v.viewport.zoom);
    assert!((zoomed - zoom * 1.25).abs() < 1e-4, "{zoom} → {zoomed}");
    h.button(cx, "genui-zoom-out");
    h.settle(cx);
    assert!((h.read(cx, |v| v.viewport.zoom) - zoom).abs() < 1e-4);

    // ⌘/Ctrl-scroll zooms keeping the point under the pointer still.
    let at = h.node(cx, "db");
    let world = |h: &Harness, cx: &mut TestAppContext| {
        let view = h.view.clone();
        h.with(cx, |window, cx| {
            let view = view.read(cx);
            view.to_world(view.local(at), window.rem_size())
        })
    };
    let before = world(&h, cx);
    h.with(cx, |window, cx| {
        window.dispatch_event(
            ScrollEvent {
                position: at,
                delta: ScrollDelta::Pixels(point(px(0.), px(60.))),
                modifiers: Modifiers::secondary_key(cx),
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
    });
    assert!(h.read(cx, |v| v.viewport.zoom) > zoom);
    let after = world(&h, cx);
    assert!((after.x - before.x).abs() < 0.5 && (after.y - before.y).abs() < 0.5);

    // In the conversation a plain wheel is the conversation's.
    let pan = h.read(cx, |v| v.viewport.pan);
    h.with(cx, |window, cx| {
        window.dispatch_event(
            ScrollEvent {
                position: at,
                delta: ScrollDelta::Pixels(point(px(0.), px(60.))),
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
    });
    assert_eq!(h.read(cx, |v| v.viewport.pan), pan);

    // Fit eases back to the whole diagram.
    h.button(cx, "genui-fit");
    h.settle(cx);
    assert!(!h.read(cx, |v| v.viewport.touched));
}

#[gpui::test]
fn selecting_a_node_shows_its_editable_details(cx: &mut TestAppContext) {
    let h = open(cx);
    assert!(!h.has(cx, "genui-rename"));
    h.click_node(cx, "db", 1);
    assert_eq!(
        h.read(cx, |v| v.selection.clone()),
        Some(Selection::Node("db".into()))
    );
    for id in [
        "genui-rename",
        "genui-ask",
        "genui-delete",
        "genui-field-kind",
        "genui-field-status",
        "genui-field-group",
    ] {
        assert!(h.has(cx, id), "{id}");
    }
    // Tech and description fields commit when the pointer goes elsewhere.
    let view = h.view.clone();
    h.with(cx, |window, cx| {
        let fields = view
            .read(cx)
            .fields
            .as_ref()
            .map(|f| (f.tech.clone().unwrap(), f.description.clone()));
        let (tech, description) = fields.expect("the node's fields");
        tech.update(cx, |state, cx| state.set_value("Postgres 16", window, cx));
        description.update(cx, |state, cx| {
            state.set_value("All the orders", window, cx)
        });
    });
    let empty = h.empty(cx);
    h.click(cx, empty, 1, Modifiers::none());
    h.read(cx, |v| {
        let db = v.working.node("db").unwrap();
        assert_eq!(db.tech.as_deref(), Some("Postgres 16"));
        assert_eq!(db.description.as_deref(), Some("All the orders"));
        assert_eq!(v.pending, 1);
    });
    // Kind, status and group come from the panel's menus.
    h.update(cx, |view, cx| {
        let patch = |kind, status| gentle_core::genui::changes::NodePatch {
            kind,
            status,
            ..Default::default()
        };
        view.patch_node("db", patch(Some(NodeKind::Storage), None), cx);
        view.patch_node("db", patch(None, Some(Some(Status::Risk))), cx);
        view.patch_node(
            "db",
            gentle_core::genui::changes::NodePatch {
                group: Some(None),
                ..Default::default()
            },
            cx,
        );
    });
    h.read(cx, |v| {
        let db = v.working.node("db").unwrap();
        assert_eq!(
            (db.kind, db.status, db.group.as_deref()),
            (NodeKind::Storage, Some(Status::Risk), None)
        );
    });
}

#[gpui::test]
fn dragging_a_node_records_one_snapped_move(cx: &mut TestAppContext) {
    let h = open(cx);
    let from = h.node(cx, "mail");
    let start = h.node_rect(cx, "mail");
    h.drag(cx, from, from + point(px(97.), px(41.)));
    h.read(cx, |v| {
        let mail = v.working.node("mail").unwrap();
        let (x, y) = (mail.x.unwrap(), mail.y.unwrap());
        assert_eq!((x % 20., y % 20.), (0., 0.), "snapped to the grid");
        let s = v.viewport.zoom; // interface size 16 px: scale = zoom
        assert!((x - (start.x + 97. / s)).abs() <= 10.);
        assert!((y - (start.y + 41. / s)).abs() <= 10.);
        assert_eq!(v.pending, 1);
        assert_eq!(v.selection, Some(Selection::Node("mail".into())));
    });
    // The whole drag is one undo step; redo brings it back.
    h.button(cx, "genui-undo");
    assert_eq!(h.read(cx, |v| v.pending), 0);
    h.button(cx, "genui-redo");
    assert_eq!(h.read(cx, |v| v.pending), 1);
    h.press(cx, "secondary-z");
    assert_eq!(h.read(cx, |v| v.pending), 0);
    h.press(cx, "secondary-shift-z");
    assert_eq!(h.read(cx, |v| v.pending), 1);
}

#[gpui::test]
fn double_click_renames_a_node(cx: &mut TestAppContext) {
    let h = open(cx);
    h.click_node(cx, "api", 2);
    assert!(h.read(cx, |v| v.rename.is_some()));
    h.input(cx, "Gateway");
    h.press(cx, "enter");
    h.read(cx, |v| {
        assert!(v.rename.is_none());
        assert_eq!(v.working.node("api").unwrap().label, "Gateway");
        assert_eq!(v.pending, 1);
    });
}

#[gpui::test]
fn dragging_from_a_handle_connects_two_nodes(cx: &mut TestAppContext) {
    let h = open(cx);
    // Web's source handle sits on its right side (left to right).
    let web = h.node_rect(cx, "web");
    let handle = h.on_window(
        cx,
        layout::Point {
            x: web.right(),
            y: web.center().y,
        },
    );
    let mail = h.node(cx, "mail");
    h.drag(cx, handle, mail);
    h.read(cx, |v| {
        let edge = v.working.edge("web->mail").expect("a new edge");
        assert_eq!((edge.from.as_str(), edge.to.as_str()), ("web", "mail"));
        assert_eq!(v.selection, Some(Selection::Edge("web->mail".into())));
        // Web didn't move: the press was the handle's, not the node's.
        assert!(v.working.node("web").unwrap().x.is_none());
    });
    // The target handle connects the other way.
    let db = h.node_rect(cx, "db");
    let target = h.on_window(
        cx,
        layout::Point {
            x: db.x,
            y: db.center().y,
        },
    );
    let mail = h.node(cx, "mail");
    h.drag(cx, target, mail);
    assert!(h.read(cx, |v| v.working.edge("mail->db").is_some()));
}

#[gpui::test]
fn delete_add_and_discard(cx: &mut TestAppContext) {
    let h = open(cx);
    h.click_node(cx, "db", 1);
    h.press(cx, "backspace");
    h.read(cx, |v| {
        assert!(v.working.node("db").is_none());
        assert!(v.working.edges.iter().all(|e| e.to != "db"));
        assert_eq!(v.selection, None);
    });
    h.button(cx, "genui-add");
    h.read(cx, |v| {
        let node = v.working.node("new-node").expect("the new node");
        assert!(node.x.is_some() && node.y.is_some());
        assert_eq!(v.selection, Some(Selection::Node("new-node".into())));
        assert!(v.rename.is_some());
        assert_eq!(v.pending, 2);
    });
    h.press(cx, "escape");
    h.button(cx, "genui-discard");
    h.read(cx, |v| {
        assert_eq!(v.pending, 0);
        assert_eq!(v.working, v.base);
    });
    // Discard is undoable too.
    h.button(cx, "genui-undo");
    assert_eq!(h.read(cx, |v| v.pending), 2);
}

#[gpui::test]
fn send_waits_for_the_extensions_answer(cx: &mut TestAppContext) {
    let h = open(cx);
    h.click_node(cx, "web", 1);
    h.press(cx, "backspace");
    h.button(cx, "genui-send");
    let sent = h.sends();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["id"], "ui-1");
    assert_eq!(sent[0]["version"], 1);
    assert_eq!(ops(&sent[0]), ["remove_node"]);
    assert_eq!(sent[0]["summary"][0], "removed Web (and its 1 connection)");
    assert!(
        sent[0]["spec"]["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|n| n["id"] != "web")
    );
    assert!(h.read(cx, |v| matches!(
        v.send,
        SendState::Sending { version: 1, .. }
    )));

    // Pi ran the command but the extension rejected the edits: they stay,
    // with the reason and Retry.
    let rejected = Outcome {
        id: "ui-1".into(),
        version: 1,
        result: Err("The agent changed this diagram meanwhile".into()),
    };
    h.update(cx, |view, cx| view.outcome(&rejected, cx));
    h.update(cx, |view, cx| view.sent(Ok(()), cx));
    h.read(cx, |v| {
        assert!(matches!(&v.send, SendState::Failed(e) if e.starts_with("The agent changed")));
        assert_eq!(v.pending, 1);
    });
    assert!(h.has(cx, "genui-send-error"));

    // An edit made while it was being sent stays pending after the answer.
    h.button(cx, "genui-send");
    assert_eq!(h.sends().len(), 2);
    h.click_node(cx, "mail", 1);
    h.press(cx, "backspace");
    let accepted = Outcome {
        id: "ui-1".into(),
        version: 1,
        result: Ok(2),
    };
    // An answer about another version is not this one's.
    h.update(cx, |view, cx| {
        view.outcome(
            &Outcome {
                version: 7,
                ..accepted.clone()
            },
            cx,
        )
    });
    assert!(h.read(cx, |v| matches!(v.send, SendState::Sending { .. })));
    h.update(cx, |view, cx| view.outcome(&accepted, cx));
    h.read(cx, |v| {
        assert!(matches!(v.send, SendState::Sent));
        assert_eq!(v.version, 2);
        assert!(v.base.node("web").is_none() && v.base.node("mail").is_some());
        assert_eq!(v.pending, 1, "removing Mail is still unsent");
    });
    // The extension's echo of the accepted version changes nothing.
    let echo = Snapshot {
        id: "ui-1".into(),
        version: 2,
        kind: "diagram".into(),
        title: "Checkout".into(),
        spec: h.read(cx, |v| v.base.to_value()),
        closed: false,
    };
    h.update(cx, |view, cx| view.set_snapshot(&echo, false, cx));
    assert_eq!(h.read(cx, |v| v.pending), 1);
}

#[gpui::test]
fn an_agent_update_keeps_unsent_edits_and_eases_in(cx: &mut TestAppContext) {
    let h = open(cx);
    h.click_node(cx, "api", 2);
    h.input(cx, "Gateway");
    h.press(cx, "enter");
    let mut next = spec();
    next["nodes"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id": "cache", "kind": "cache"}));
    next["edges"]
        .as_array_mut()
        .unwrap()
        .push(json!({"from": "api", "to": "cache"}));
    let snapshot = Snapshot {
        id: "ui-1".into(),
        version: 2,
        kind: "diagram".into(),
        title: "Checkout".into(),
        spec: next,
        closed: false,
    };
    h.update(cx, |view, cx| view.set_snapshot(&snapshot, false, cx));
    h.read(cx, |v| {
        assert!(v.arrival.is_some(), "the new node eases in");
        assert_eq!(v.working.node("api").unwrap().label, "Gateway");
        assert!(v.working.node("cache").is_some());
        assert_eq!(v.pending, 1);
    });
    h.settle(cx);
    assert!(h.read(cx, |v| v.arrival.is_none()));
}

#[gpui::test]
fn ask_about_this_and_expand(cx: &mut TestAppContext) {
    let h = open(cx);
    h.click_node(cx, "api", 1);
    h.button(cx, "genui-ask");
    h.button(cx, "genui-expand");
    h.update(cx, |view, cx| view.set_expanded(true, cx));
    h.button(cx, "genui-expand");
    assert_eq!(
        *h.seen.borrow(),
        [
            Seen::Ask("About “API” (api) in the “Checkout” diagram (ui-1): ".into()),
            Seen::Expand,
            Seen::Collapse,
        ]
    );
}

#[gpui::test]
fn box_selection_groups_and_drags_nodes(cx: &mut TestAppContext) {
    let h = open(cx);
    // Shift-drag a box around Web and Mail only (the ungrouped band).
    let rects = [h.node_rect(cx, "web"), h.node_rect(cx, "mail")];
    let area = rects[0].union(&rects[1]);
    let (from, to) = (
        h.on_window(
            cx,
            layout::Point {
                x: area.x - 8.,
                y: area.y - 8.,
            },
        ),
        h.on_window(
            cx,
            layout::Point {
                x: area.right() + 8.,
                y: area.bottom() + 8.,
            },
        ),
    );
    h.modifiers(cx, Modifiers::shift());
    h.drag_with(cx, from, to, Modifiers::shift());
    h.modifiers(cx, Modifiers::none());
    let picked = h.read(cx, |v| v.selection.clone());
    let Some(Selection::Nodes(mut ids)) = picked else {
        panic!("several nodes selected: {picked:?}")
    };
    ids.sort();
    assert_eq!(ids, ["mail", "web"]);

    // Group them and name the group.
    h.button(cx, "genui-group");
    h.input(cx, "Edge");
    h.press(cx, "enter");
    h.read(cx, |v| {
        let group = v
            .working
            .groups
            .iter()
            .find(|g| g.label == "Edge")
            .expect("the new group");
        for id in ["web", "mail"] {
            assert_eq!(
                v.working.node(id).unwrap().group.as_deref(),
                Some(group.id.as_str())
            );
        }
        assert_eq!(v.selection, Some(Selection::Group(group.id.clone())));
    });

    // Drag Mail onto the API: it moves into Backend, in the same undo step.
    let undo = h.read(cx, |v| v.can_undo());
    assert!(undo);
    let (mail, api) = (h.node(cx, "mail"), h.node(cx, "api"));
    h.drag(cx, mail, api);
    assert_eq!(
        h.read(cx, |v| v.working.node("mail").unwrap().group.clone()),
        Some("back".into())
    );
    h.button(cx, "genui-undo");
    h.read(cx, |v| {
        let mail = v.working.node("mail").unwrap();
        assert!(mail.x.is_none() && mail.group.as_deref() != Some("back"));
    });

    // Selecting a group and deleting it ungroups its members.
    h.button(cx, "genui-redo");
    h.update(cx, |view, cx| {
        view.selection = Some(Selection::Group("back".into()));
        view.delete_selection(cx);
    });
    h.read(cx, |v| {
        assert!(v.working.group("back").is_none());
        assert!(
            v.working
                .nodes
                .iter()
                .all(|n| n.group.as_deref() != Some("back"))
        );
    });
}

#[gpui::test]
fn direction_toggle_minimap_and_lock(cx: &mut TestAppContext) {
    let h = open(cx);
    h.button(cx, "genui-direction");
    h.read(cx, |v| {
        assert_eq!(v.working.direction, Direction::TopBottom);
        assert_eq!(
            gentle_core::genui::changes::diff(&v.base, &v.working),
            [Change::SetDirection {
                direction: Direction::TopBottom
            }]
        );
    });

    // Pressing the minimap moves the view there.
    let pan = h.read(cx, |v| v.viewport.pan);
    let map = h.read(cx, |v| v.minimap.get());
    assert!(map.size.width > px(0.));
    h.click(cx, map.origin + point(px(4.), px(4.)), 1, Modifiers::none());
    assert_ne!(h.read(cx, |v| v.viewport.pan), pan);

    // Locked: nothing moves, connects or gets deleted; selecting still works.
    h.button(cx, "genui-lock");
    let pending = h.read(cx, |v| v.pending);
    let from = h.node(cx, "mail");
    h.drag(cx, from, from + point(px(80.), px(40.)));
    assert_eq!(
        h.read(cx, |v| v.selection.clone()),
        Some(Selection::Node("mail".into()))
    );
    h.press(cx, "backspace");
    h.read(cx, |v| {
        assert_eq!(v.pending, pending);
        assert!(v.working.node("mail").unwrap().x.is_none());
    });
    h.button(cx, "genui-lock");
    assert!(!h.read(cx, |v| v.locked));
}

#[gpui::test]
fn hovering_an_edge_highlights_it(cx: &mut TestAppContext) {
    let h = open(cx);
    // Off the label pill: the point along the edge's middle stretch furthest
    // from where the layout put its label.
    let p = h.read(cx, |v| {
        let path = &v.layout.edges[0];
        let label = v.layout.labels[0].expect("the edge is labelled");
        let away = |q: &layout::Point| (q.x - label.x).hypot(q.y - label.y);
        (4..=16)
            .map(|k| path.point_at(k as f32 / 20.))
            .max_by(|a, b| away(a).total_cmp(&away(b)))
            .unwrap()
    });
    let at = h.on_window(cx, p);
    h.hover(cx, at);
    assert_eq!(h.read(cx, |v| v.hover_edge), Some(0));
    // A click there selects the edge.
    h.click(cx, at, 1, Modifiers::none());
    assert_eq!(
        h.read(cx, |v| v.selection.clone()),
        Some(Selection::Edge("web->api".into()))
    );
}
