//! Drawing a diagram surface (header, canvas with its nodes and labels,
//! details, footer) and the conversation rows around it: a surface being
//! drawn, one that failed, older versions and the user's sent edits.

use gentle_core::genui::diagram::{Diagram, EdgeStyle, NodeKind, Status};
use gentle_core::genui::layout::{self, GROUP_PAD, Rect};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::{Disableable as _, Icon, Sizable as _, Size, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::diagram::{
    DiagramEvent, DiagramView, Drag, MIN_ZOOM, READABLE_ZOOM, Selection, SendState,
};
use super::paint::Frame;
use crate::shared::ui;

/// The card's height in the conversation (px at 100 %): a fixed size, so
/// the surface can be a cached view that only redraws when it changes.
pub(super) const CARD_HEIGHT: f32 = 460.;

fn kind_icon(kind: NodeKind) -> IconName {
    match kind {
        NodeKind::Service => IconName::Server,
        NodeKind::Database => IconName::Database,
        NodeKind::Queue => IconName::Layers,
        NodeKind::Cache => IconName::Zap,
        NodeKind::Client => IconName::Monitor,
        NodeKind::External => IconName::Globe,
        NodeKind::Storage => IconName::HardDrive,
        NodeKind::Function => IconName::SquareFunction,
        NodeKind::Note => IconName::StickyNote,
    }
}

fn kind_color(kind: NodeKind) -> Hsla {
    match kind {
        NodeKind::Service | NodeKind::Function => ui::info(),
        NodeKind::Database | NodeKind::Storage => ui::success(),
        NodeKind::Queue => ui::warning(),
        NodeKind::Cache => ui::champagne(),
        NodeKind::Client => ui::accent(),
        NodeKind::External => ui::muted(),
        NodeKind::Note => ui::faint(),
    }
}

fn status_color(status: Status) -> Hsla {
    match status {
        Status::Planned => ui::info(),
        Status::Existing => ui::success(),
        Status::Deprecated => ui::faint(),
        Status::Risk => ui::danger(),
    }
}

fn style_name(style: EdgeStyle) -> &'static str {
    match style {
        EdgeStyle::Sync => "sync call",
        EdgeStyle::Async => "async",
        EdgeStyle::Data => "data flow",
        EdgeStyle::Dependency => "dependency",
    }
}

fn label_of<'a>(diagram: &'a Diagram, id: &'a str) -> &'a str {
    diagram.node(id).map_or(id, |n| n.label.as_str())
}

impl Render for DiagramView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rem = window.rem_size();
        let size = self.bounds.get().size;
        let refit = self.viewport.fitted_for.is_none()
            || (!self.viewport.touched && self.viewport.fitted_for != Some(size));
        if refit && size.width > px(1.) && size.height > px(1.) {
            self.fit(READABLE_ZOOM, rem);
        }
        v_flex()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .size_full()
            .rounded_xl()
            .bg(ui::surface())
            .border_1()
            .border_color(ui::border())
            .overflow_hidden()
            .child(self.render_header(cx))
            .child(self.render_canvas(rem, cx))
            .child(self.render_footer(cx))
    }
}

impl DiagramView {
    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tool = |id: &'static str, icon: IconName, tip: &'static str| {
            Button::new(id)
                .ghost()
                .xsmall()
                .text_color(ui::muted())
                .icon(Icon::new(icon))
                .tooltip(tip)
        };
        let nodes = self.working.nodes.len();
        let edges = self.working.edges.len();
        h_flex()
            .px_3()
            .py_1p5()
            .gap_2()
            .min_w_0()
            .child(Icon::new(IconName::Workflow).small().text_color(ui::info()))
            .child(
                div()
                    .min_w_0()
                    .flex_shrink(1.)
                    .truncate()
                    .text_size(rems(13. / 16.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ui::champagne())
                    .child(if self.title.is_empty() {
                        "Diagram".to_string()
                    } else {
                        self.title.clone()
                    }),
            )
            .child(ui::chip(format!("v{}", self.version), ui::info()).flex_shrink_0())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(ui::MONO)
                    .text_size(rems(12. / 16.))
                    .text_color(ui::muted())
                    .child(format!(
                        "{nodes} node{} · {edges} edge{}",
                        if nodes == 1 { "" } else { "s" },
                        if edges == 1 { "" } else { "s" }
                    )),
            )
            .child(
                h_flex()
                    .flex_shrink_0()
                    .gap_0p5()
                    .child(
                        tool("genui-add", IconName::Plus, "Add node")
                            .on_click(cx.listener(|this, _, window, cx| this.add_node(window, cx))),
                    )
                    .child(
                        tool("genui-undo", IconName::Undo2, "Undo")
                            .disabled(!self.can_undo())
                            .on_click(cx.listener(|this, _, _, cx| this.undo(cx))),
                    )
                    .child(
                        tool("genui-zoom-out", IconName::ZoomOut, "Zoom out").on_click(
                            cx.listener(|this, _, window, cx| {
                                this.zoom_center(1. / 1.25, window.rem_size());
                                cx.notify();
                            }),
                        ),
                    )
                    .child(tool("genui-zoom-in", IconName::ZoomIn, "Zoom in").on_click(
                        cx.listener(|this, _, window, cx| {
                            this.zoom_center(1.25, window.rem_size());
                            cx.notify();
                        }),
                    ))
                    .child(
                        tool("genui-fit", IconName::Scan, "Fit the whole diagram").on_click(
                            cx.listener(|this, _, window, cx| {
                                this.viewport.touched = false;
                                this.fit(MIN_ZOOM, window.rem_size());
                                cx.notify();
                            }),
                        ),
                    )
                    .child(
                        tool(
                            "genui-expand",
                            if self.expanded {
                                IconName::Minimize2
                            } else {
                                IconName::Maximize2
                            },
                            if self.expanded {
                                "Back to the conversation"
                            } else {
                                "Expand"
                            },
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            cx.emit(if this.expanded {
                                DiagramEvent::Collapse
                            } else {
                                DiagramEvent::Expand
                            })
                        })),
                    ),
            )
    }

    fn render_canvas(&self, rem: Pixels, cx: &mut Context<Self>) -> impl IntoElement {
        let s = self.scale(rem);
        let pan = self.viewport.pan;
        let bounds = self.bounds.get();
        let visible = Rect {
            x: -pan.x / s,
            y: -pan.y / s,
            w: f32::from(bounds.size.width) / s,
            h: f32::from(bounds.size.height) / s,
        }
        .inflate(40.);
        let at = |x: f32, y: f32| (px(x * s + pan.x), px(y * s + pan.y));
        let selected_edge = match &self.selection {
            Some(Selection::Edge(id)) => self.working.edges.iter().position(|e| &e.id == id),
            _ => None,
        };
        let connect = match &self.drag {
            Some(Drag::Connect { from, to }) => self
                .working
                .node_index(from)
                .map(|ix| (self.layout.nodes[ix].center(), *to)),
            _ => None,
        };
        let frame = Frame {
            layout: self.layout.clone(),
            styles: self.working.edges.iter().map(|e| e.style).collect(),
            scale: s,
            pan,
            selected_edge,
            connect,
        };
        let cell = self.bounds.clone();
        let entity = cx.entity_id();
        let painter = canvas(
            move |bounds, _, cx| {
                let old = cell.get();
                cell.set(bounds);
                if (old.size.width - bounds.size.width).abs() > px(1.)
                    || (old.size.height - bounds.size.height).abs() > px(1.)
                {
                    cx.notify(entity);
                }
            },
            move |bounds, _, window, _| frame.paint(bounds, window),
        )
        .absolute()
        .size_full();

        let mut body = div()
            .id("genui-canvas")
            .relative()
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .bg(ui::well())
            .cursor(match self.drag {
                Some(Drag::Pan { .. }) => CursorStyle::ClosedHand,
                Some(Drag::Connect { .. }) => CursorStyle::Crosshair,
                _ => CursorStyle::OpenHand,
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    this.press_background(event, window, cx)
                }),
            )
            .on_mouse_move(cx.listener(Self::pointer_moved))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.released(cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.released(cx)),
            )
            .on_scroll(cx.listener(Self::scrolled))
            .on_pinch(cx.listener(Self::pinched))
            .child(painter);

        // Group titles, above their boxes' top padding.
        for (group, rect) in self.working.groups.iter().zip(&self.layout.groups) {
            let Some(rect) = rect.filter(|r| r.intersects(&visible)) else {
                continue;
            };
            let (x, y) = at(rect.x + GROUP_PAD * 0.75, rect.y + 6.);
            body = body.child(
                div()
                    .absolute()
                    .left(x)
                    .top(y)
                    .max_w(px((rect.w - GROUP_PAD) * s))
                    .truncate()
                    .text_size(px(11. * s))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ui::muted())
                    .child(group.label.clone()),
            );
        }

        // Edge labels, centered on the curve.
        if s >= 0.35 {
            for (ix, (edge, path)) in self
                .working
                .edges
                .iter()
                .zip(&self.layout.edges)
                .enumerate()
            {
                let Some(label) = edge.label.as_ref().filter(|l| !l.is_empty()) else {
                    continue;
                };
                let p = path.label_at();
                if !visible.contains(p) {
                    continue;
                }
                let width = 140.;
                let (x, y) = at(p.x - width / 2., p.y - 9.);
                let selected = selected_edge == Some(ix);
                body = body.child(
                    h_flex()
                        .absolute()
                        .left(x)
                        .top(y)
                        .w(px(width * s))
                        .justify_center()
                        .child(
                            div()
                                .px(px(5. * s))
                                .rounded(px(5. * s))
                                .bg(ui::surface())
                                .max_w_full()
                                .truncate()
                                .text_size(px(10.5 * s))
                                .text_color(if selected { ui::accent() } else { ui::muted() })
                                .child(label.clone()),
                        ),
                );
            }
        }

        let selected_node = match &self.selection {
            Some(Selection::Node(id)) => Some(id.as_str()),
            _ => None,
        };
        for (ix, node) in self.working.nodes.iter().enumerate() {
            let rect = self.layout.nodes[ix];
            if !rect.intersects(&visible) {
                continue;
            }
            let (x, y) = at(rect.x, rect.y);
            let selected = selected_node == Some(node.id.as_str());
            body = body.child(self.render_node(ix, x, y, rect, s, selected, cx));
        }

        if let Some(rename) = &self.rename {
            let place = match &rename.target {
                Selection::Node(id) => self.working.node_index(id).map(|ix| self.layout.nodes[ix]),
                Selection::Edge(id) => {
                    self.working
                        .edges
                        .iter()
                        .position(|e| &e.id == id)
                        .map(|ix| {
                            let p = self.layout.edges[ix].label_at();
                            Rect {
                                x: p.x - layout::NODE_W / 2.,
                                y: p.y - 14.,
                                w: layout::NODE_W,
                                h: 28.,
                            }
                        })
                }
            };
            if let Some(rect) = place {
                let (x, y) = at(rect.x, rect.y + rect.h / 2.);
                body = body.child(
                    div()
                        .absolute()
                        .left(x)
                        .top(y - rems(15. / 16.).to_pixels(rem))
                        .w(px(rect.w * s).max(rems(160. / 16.).to_pixels(rem)))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            ui::field()
                                .h(rems(30. / 16.))
                                .bg(ui::popover_bg())
                                .border_color(ui::accent().opacity(0.6))
                                .child(Input::new(&rename.input).appearance(false).small()),
                        ),
                );
            }
        }

        if self.working.nodes.is_empty() {
            body = body.child(
                v_flex()
                    .absolute()
                    .inset_0()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .child(Icon::new(IconName::Workflow).text_color(ui::faint()))
                    .child(
                        div()
                            .text_sm()
                            .text_color(ui::muted())
                            .child("Nothing drawn yet. Add a node with +."),
                    ),
            );
        }
        if let Some(error) = &self.error {
            body = body.child(
                div().absolute().left_2().right_2().top_2().child(
                    ui::card()
                        .px_3()
                        .py_2()
                        .gap_1()
                        .bg(ui::popover_bg())
                        .border_color(ui::danger().opacity(0.35))
                        .child(
                            h_flex()
                                .gap_2()
                                .text_size(rems(12. / 16.))
                                .text_color(ui::danger())
                                .child(Icon::new(IconName::CircleAlert).xsmall())
                                .child("The agent's latest version can't be drawn"),
                        )
                        .child(
                            div()
                                .font_family(ui::MONO)
                                .text_size(rems(11. / 16.))
                                .text_color(ui::muted())
                                .whitespace_normal()
                                .child(error.clone()),
                        ),
                ),
            );
        }
        body.children(self.render_details(cx))
    }

    #[allow(clippy::too_many_arguments)]
    fn render_node(
        &self,
        ix: usize,
        x: Pixels,
        y: Pixels,
        rect: Rect,
        s: f32,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let node = &self.working.nodes[ix];
        let color = kind_color(node.kind);
        let border = match (selected, node.status) {
            (true, _) => ui::accent(),
            (false, Some(Status::Risk)) => ui::danger().opacity(0.7),
            _ => color.opacity(0.45),
        };
        let subtitle = node
            .tech
            .clone()
            .unwrap_or_else(|| node.kind.name().to_string());
        let id = node.id.clone();
        let handle_id = node.id.clone();
        let dragging =
            matches!(&self.drag, Some(Drag::Node { id, moved: true, .. }) if *id == node.id);
        div()
            .id(("genui-node", ix))
            .group("genui-node")
            .absolute()
            .left(x)
            .top(y)
            .w(px(rect.w * s))
            .h(px(rect.h * s))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8. * s))
            .px(px(10. * s))
            .rounded(px(10. * s))
            .bg(ui::surface())
            .border(px(if selected { 2. } else { 1. }))
            .border_color(border)
            .when(node.status == Some(Status::Planned), |d| d.border_dashed())
            .when(node.status == Some(Status::Deprecated), |d| d.opacity(0.55))
            .when(dragging, |d| d.shadow_lg())
            .cursor(CursorStyle::PointingHand)
            .hover(|d| d.bg(ui::surface_hover()))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.press_node(&id, event, window, cx)
                }),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .size(px(26. * s))
                    .rounded(px(7. * s))
                    .bg(color.opacity(0.14))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        Icon::new(kind_icon(node.kind))
                            .size(px(15. * s))
                            .text_color(color),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .text_size(px(13. * s))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ui::fg())
                            .child(node.label.clone()),
                    )
                    .when(s >= 0.5, |d| {
                        d.child(
                            div()
                                .truncate()
                                .text_size(px(11. * s))
                                .text_color(ui::muted())
                                .child(subtitle),
                        )
                    }),
            )
            .when_some(node.status.filter(|_| s >= 0.6), |d, status| {
                d.child(
                    div()
                        .flex_shrink_0()
                        .size(px(7. * s))
                        .rounded_full()
                        .bg(status_color(status)),
                )
            })
            // The handle a connection is dragged from.
            .child(
                div()
                    .id(("genui-handle", ix))
                    .absolute()
                    .right(px(-6. * s))
                    .top(px((rect.h / 2. - 6.) * s))
                    .size(px(12. * s))
                    .rounded_full()
                    .bg(ui::accent())
                    .border_2()
                    .border_color(ui::well())
                    .cursor(CursorStyle::Crosshair)
                    .when(!selected, |d| {
                        d.opacity(0.).group_hover("genui-node", |d| d.opacity(1.))
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            this.press_handle(&handle_id, event, window, cx)
                        }),
                    ),
            )
            .into_any_element()
    }

    /// The selection's details, over the canvas' top-right corner.
    fn render_details(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.drag.is_some() {
            return None;
        }
        let selection = self.selection.as_ref()?;
        let action = |id: &'static str, icon: IconName, label: &'static str| {
            Button::new(id)
                .ghost()
                .xsmall()
                .text_color(ui::muted())
                .icon(Icon::new(icon))
                .label(label)
        };
        let actions = h_flex()
            .gap_1()
            .flex_wrap()
            .child(
                action("genui-rename", IconName::Pencil, "Rename").on_click(cx.listener(
                    |this, _, window, cx| {
                        if let Some(selection) = this.selection.clone() {
                            this.start_rename(selection, window, cx);
                        }
                    },
                )),
            )
            .child(
                action("genui-ask", IconName::MessageSquare, "Ask about this")
                    .on_click(cx.listener(|this, _, _, cx| this.ask(cx))),
            )
            .child(
                action("genui-delete", IconName::Trash, "Delete")
                    .on_click(cx.listener(|this, _, _, cx| this.delete_selection(cx))),
            );
        let text = |text: String| {
            div()
                .text_size(rems(12. / 16.))
                .text_color(ui::muted())
                .whitespace_normal()
                .child(text)
        };
        let mut panel = ui::island(cx)
            .absolute()
            .top_2()
            .right_2()
            .w(rems(260. / 16.))
            .max_h(rems(300. / 16.))
            .p_3()
            .gap_2()
            .flex()
            .flex_col()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_scroll(|_, _, cx| cx.stop_propagation());
        match selection {
            Selection::Node(id) => {
                let node = self.working.node(id)?;
                let group = node
                    .group
                    .as_deref()
                    .map(|g| self.working.group(g).map_or(g, |g| g.label.as_str()));
                panel = panel
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Icon::new(kind_icon(node.kind))
                                    .small()
                                    .text_color(kind_color(node.kind)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(ui::fg())
                                    .child(node.label.clone()),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .flex_wrap()
                            .child(ui::chip(node.kind.name(), kind_color(node.kind)))
                            .when_some(node.status, |d, status| {
                                d.child(ui::chip(status.name(), status_color(status)))
                            })
                            .when_some(group, |d, group| {
                                d.child(ui::chip(group.to_string(), ui::muted()))
                            }),
                    )
                    .when_some(node.tech.clone(), |d, tech| {
                        d.child(
                            div()
                                .font_family(ui::MONO)
                                .text_size(rems(11. / 16.))
                                .text_color(ui::muted())
                                .child(tech),
                        )
                    })
                    .when_some(node.description.clone(), |d, description| {
                        d.child(
                            div()
                                .id("genui-description")
                                .min_h_0()
                                .flex_shrink(1.)
                                .overflow_y_scroll()
                                .child(text(description)),
                        )
                    })
                    .child(
                        div()
                            .font_family(ui::MONO)
                            .text_size(rems(10.5 / 16.))
                            .text_color(ui::faint())
                            .child(node.id.clone()),
                    );
            }
            Selection::Edge(id) => {
                let edge = self.working.edge(id)?;
                panel = panel
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(ui::fg())
                            .whitespace_normal()
                            .child(format!(
                                "{} → {}",
                                label_of(&self.working, &edge.from),
                                label_of(&self.working, &edge.to)
                            )),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .child(ui::chip(style_name(edge.style), ui::info())),
                    )
                    .when_some(edge.label.clone(), |d, label| d.child(text(label)));
            }
        }
        Some(panel.child(actions).into_any_element())
    }

    fn render_footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let footer = h_flex()
            .px_3()
            .py_1p5()
            .gap_2()
            .min_h(rems(38. / 16.))
            .border_t_1()
            .border_color(ui::border())
            .text_size(rems(12. / 16.));
        if let Some(refused) = &self.refused {
            return footer
                .text_color(ui::danger())
                .child(Icon::new(IconName::CircleAlert).xsmall())
                .child(div().flex_1().min_w_0().truncate().child(refused.clone()));
        }
        if self.pending > 0 {
            let sending = matches!(self.send, SendState::Sending);
            return footer
                .child(ui::chip(
                    format!(
                        "{} change{}",
                        self.pending,
                        if self.pending == 1 { "" } else { "s" }
                    ),
                    ui::accent(),
                ))
                .child(match &self.note {
                    Some(note) => ui::field()
                        .flex_1()
                        .min_w_0()
                        .h(rems(26. / 16.))
                        .child(Input::new(note).appearance(false).xsmall())
                        .into_any_element(),
                    None => div()
                        .id("genui-add-note")
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .cursor_pointer()
                        .text_color(ui::faint())
                        .hover(|d| d.text_color(ui::muted()))
                        .child("Add a note…")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.note_input(window, cx);
                            if let Some(note) = &this.note {
                                note.update(cx, |state, cx| state.focus(window, cx));
                            }
                            cx.notify();
                        }))
                        .into_any_element(),
                })
                .when_some(
                    match &self.send {
                        SendState::Failed(error) => Some(error.clone()),
                        _ => None,
                    },
                    |d, error| {
                        d.child(
                            div()
                                .max_w(rems(220. / 16.))
                                .truncate()
                                .text_color(ui::danger())
                                .child(error),
                        )
                    },
                )
                .child(
                    Button::new("genui-discard")
                        .ghost()
                        .xsmall()
                        .label("Discard")
                        .disabled(sending)
                        .on_click(cx.listener(|this, _, _, cx| this.discard(cx))),
                )
                .child(
                    Button::new("genui-send")
                        .primary()
                        .xsmall()
                        .icon(Icon::new(IconName::Send))
                        .label("Send to agent")
                        .loading(sending)
                        .on_click(cx.listener(|this, _, _, cx| this.send(cx))),
                );
        }
        match &self.send {
            SendState::Sent => footer
                .text_color(ui::success())
                .child(Icon::new(IconName::CircleCheck).xsmall())
                .child("Sent to the agent"),
            _ => footer
                .text_color(ui::faint())
                .child(div().flex_1().min_w_0().truncate().child(
                "Drag to move · double-click to rename · drag ● to connect · ⌘/Ctrl-scroll to zoom",
            )),
        }
    }
}

// ── Conversation rows ──────────────────────────────────────────────────────

fn row(icon: IconName, color: Hsla, title: String, note: Option<String>) -> Div {
    ui::card()
        .flex_row()
        .w_full()
        .items_center()
        .gap_2()
        .px_3()
        .py_2()
        .child(Icon::new(icon).small().text_color(color))
        .child(
            div()
                .min_w_0()
                .flex_shrink(1.)
                .truncate()
                .text_size(rems(13. / 16.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(ui::champagne())
                .child(title),
        )
        .children(note.map(|note| {
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(rems(12. / 16.))
                .text_color(ui::muted())
                .child(note)
        }))
}

/// The agent is still sending the surface.
pub(super) fn drawing_card(title: &str) -> AnyElement {
    row(
        IconName::Workflow,
        ui::info(),
        title.to_string(),
        Some("Drawing…".into()),
    )
    .child(ui::spinner(Size::XSmall, ui::info()))
    .into_any_element()
}

/// The tool call failed: the extension's message (what the agent saw).
pub(super) fn failed_card(title: &str, error: &str) -> AnyElement {
    ui::card()
        .w_full()
        .border_color(ui::danger().opacity(0.35))
        .child(
            row(IconName::Workflow, ui::danger(), title.to_string(), None)
                .border_0()
                .bg(transparent_black()),
        )
        .child(
            div()
                .px_3()
                .pb_2()
                .font_family(ui::MONO)
                .text_size(rems(12. / 16.))
                .text_color(ui::muted())
                .whitespace_normal()
                .child(error.to_string()),
        )
        .into_any_element()
}

/// An older version of a surface shown again further down.
pub(super) fn superseded_row(title: &str, version: u64, closed: bool) -> AnyElement {
    row(
        IconName::Workflow,
        ui::faint(),
        title.to_string(),
        Some(if closed {
            "closed by the agent".into()
        } else {
            format!("v{version} · updated below")
        }),
    )
    .opacity(0.7)
    .into_any_element()
}

/// The surface is open in place of the conversation.
pub(super) fn expanded_row(view: super::Surface, title: &str) -> AnyElement {
    row(
        IconName::Workflow,
        ui::info(),
        title.to_string(),
        Some("open in the expanded view".into()),
    )
    .child(
        Button::new("genui-collapse")
            .ghost()
            .xsmall()
            .icon(Icon::new(IconName::Minimize2))
            .label("Back here")
            .on_click(move |_, _, cx| view.collapse(cx)),
    )
    .into_any_element()
}

/// The user's sent edits (the extension's `gentle-ui.event` message).
pub(super) fn event_card(details: &serde_json::Value) -> AnyElement {
    let title = details["title"]
        .as_str()
        .filter(|t| !t.is_empty())
        .unwrap_or("Diagram");
    let summary: Vec<&str> = details["summary"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| s.as_str())
        .collect();
    const SHOWN: usize = 6;
    let more = summary.len().saturating_sub(SHOWN);
    let note = details["note"].as_str().filter(|n| !n.trim().is_empty());
    ui::card()
        .w_full()
        .child(
            row(
                IconName::Pencil,
                ui::accent(),
                format!("You edited {title}"),
                None,
            )
            .border_0()
            .bg(transparent_black())
            .child(div().flex_1())
            .when_some(details["version"].as_u64(), |d, v| {
                d.child(ui::chip(format!("v{v}"), ui::info()))
            }),
        )
        .child(
            v_flex()
                .px_3()
                .pb_2()
                .gap_0p5()
                .text_size(rems(12. / 16.))
                .text_color(ui::muted())
                .children(summary.iter().take(SHOWN).map(|line| {
                    h_flex()
                        .gap_1p5()
                        .child(div().text_color(ui::faint()).child("•"))
                        .child(div().min_w_0().whitespace_normal().child(line.to_string()))
                }))
                .when(more > 0, |d| {
                    d.child(
                        div()
                            .text_color(ui::faint())
                            .child(format!("… {more} more")),
                    )
                })
                .when_some(note, |d, note| {
                    d.child(
                        div()
                            .pt_1()
                            .text_color(ui::fg())
                            .whitespace_normal()
                            .child(format!("“{note}”")),
                    )
                }),
        )
        .into_any_element()
}
