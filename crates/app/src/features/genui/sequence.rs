//! The sequence kind on the shared canvas: participant cards on top of
//! dashed lifelines, messages as arrows down the page (sync filled, async
//! open, returns dashed, self calls looping), fragments as labeled boxes
//! and activation bars. Drag a header sideways or a message up and down to
//! reorder, drag from one lifeline to another to add a message,
//! double-click to rename; the details island changes a message's kind.

use std::rc::Rc;

use gentle_core::genui::edits::{Editable as _, Edits};
use gentle_core::genui::layout::{Cubic, EdgePath, Point as P, Rect};
use gentle_core::genui::sequence::{
    self as seq, Change, HEAD_H, Message, MessageKind, Participant, ParticipantKind, SELF_LOOP,
    Sequence, SequenceLayout,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::{Icon, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::canvas::{self, Canvas, Drawn, Head, Shape, moved};
use super::panel::{Body, PanelView};
use crate::shared::ui;

pub(super) enum SeqDrag {
    /// A participant's header moving sideways (`x`: the pointer, diagram units).
    Participant {
        id: String,
        from: Point<Pixels>,
        x: f32,
        moved: bool,
    },
    /// A message moving up or down.
    Message {
        id: String,
        from: Point<Pixels>,
        y: f32,
        moved: bool,
    },
    /// A new message drawn from `lane`'s lifeline at `start`.
    Connect {
        lane: usize,
        from: Point<Pixels>,
        start: P,
        to: P,
        moved: bool,
    },
}

pub(super) struct SequenceState {
    pub edits: Edits<Sequence>,
    pub canvas: Canvas,
    /// `p:<id>` or `m:<id>`.
    pub selected: Option<String>,
    pub drag: Option<SeqDrag>,
    /// What was laid out last and its layout.
    laid: Option<(Sequence, Rc<SequenceLayout>)>,
}

impl SequenceState {
    pub fn new(spec: Sequence) -> Self {
        Self {
            edits: Edits::new(spec),
            canvas: Canvas::new(),
            selected: None,
            drag: None,
            laid: None,
        }
    }

    pub fn rebase(&mut self, spec: Sequence) {
        self.edits.rebase(spec);
        self.drag = None;
        let working = self.edits.working();
        let gone = match self.selected.as_deref() {
            Some(key) => match key.split_once(':') {
                Some(("p", id)) => working.participant(id).is_none(),
                Some((_, id)) => working.message(id).is_none(),
                None => true,
            },
            None => false,
        };
        if gone {
            self.selected = None;
        }
    }

    /// The working spec, with a drag's reorder previewed.
    fn shown(&self, layout: &SequenceLayout) -> Sequence {
        let working = self.edits.working();
        let mut shown = working.clone();
        match &self.drag {
            Some(SeqDrag::Participant {
                id, x, moved: true, ..
            }) => {
                let order = Sequence::moved(
                    working.participants.iter().map(|p| p.id.as_str()),
                    id,
                    layout.lane_at(*x),
                );
                let _ = shown.apply(&Change::OrderParticipants {
                    participants: order,
                });
            }
            Some(SeqDrag::Message {
                id, y, moved: true, ..
            }) => {
                let index = layout.insert_at(*y, working.index(id));
                let order =
                    Sequence::moved(working.messages.iter().map(|m| m.id.as_str()), id, index);
                let _ = shown.apply(&Change::OrderMessages { messages: order });
            }
            _ => {}
        }
        shown
    }

    fn layout_of(&mut self, spec: &Sequence) -> Rc<SequenceLayout> {
        match &self.laid {
            Some((laid, layout)) if laid == spec => layout.clone(),
            _ => {
                let layout = Rc::new(seq::layout(spec));
                self.laid = Some((spec.clone(), layout.clone()));
                layout
            }
        }
    }
}

fn state(this: &mut PanelView) -> Option<&mut SequenceState> {
    match &mut this.body {
        Some(Body::Sequence(s)) => Some(s),
        _ => None,
    }
}

fn kind_icon(kind: ParticipantKind) -> IconName {
    match kind {
        ParticipantKind::Actor => IconName::User,
        ParticipantKind::Service => IconName::Server,
        ParticipantKind::Db => IconName::Database,
        ParticipantKind::Queue => IconName::Layers,
        ParticipantKind::External => IconName::Globe,
    }
}

fn kind_color(kind: ParticipantKind) -> Hsla {
    match kind {
        ParticipantKind::Actor => ui::accent(),
        ParticipantKind::Service => ui::info(),
        ParticipantKind::Db => ui::success(),
        ParticipantKind::Queue => ui::warning(),
        ParticipantKind::External => ui::muted(),
    }
}

fn fragment_name(kind: seq::FragmentKind) -> &'static str {
    kind.name()
}

/// Where a message's label and rename field sit.
fn label_rect(layout: &SequenceLayout, index: usize) -> Rect {
    let p = layout.messages[index].label;
    Rect {
        x: p.x - 80.,
        y: p.y - 12.,
        w: 160.,
        h: 24.,
    }
}

impl PanelView {
    fn sequence(&self) -> Option<&SequenceState> {
        match &self.body {
            Some(Body::Sequence(s)) => Some(s),
            _ => None,
        }
    }

    fn sequence_edit(&mut self, change: Change, cx: &mut Context<Self>) -> bool {
        self.edit_with(cx, |body| match body {
            Body::Sequence(s) => Some(s.edits.edit(&change)),
            _ => None,
        })
    }

    pub(super) fn render_sequence(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let rem = window.rem_size();
        let Some(s) = state(self) else {
            return div().into_any_element();
        };
        // A drag previews its reorder on the layout without it.
        let shown = match &s.drag {
            Some(_) => s.shown(&seq::layout(s.edits.working())),
            None => s.edits.working().clone(),
        };
        let l = s.layout_of(&shown);
        let tweening = s.canvas.prepare(l.bounds, rem);
        self.canvas_frames(tweening, window, cx);
        let drawn = self.sequence_drawn(&shown, &l, rem, cx);
        self.render_canvas(drawn, rem, cx)
    }

    fn sequence_drawn(
        &self,
        shown: &Sequence,
        l: &SequenceLayout,
        rem: Pixels,
        cx: &mut Context<Self>,
    ) -> Drawn {
        let Some(s) = self.sequence() else {
            return Drawn {
                shapes: Vec::new(),
                children: Vec::new(),
                minimap: Vec::new(),
                details: None,
                cursor: None,
                empty: None,
            };
        };
        let c = &s.canvas;
        let scale = c.scale(rem);
        let visible = c.visible(rem).inflate(60.);
        let selected = s.selected.as_deref();
        let mut shapes = Vec::new();
        let mut children: Vec<AnyElement> = Vec::new();

        for f in &l.fragments {
            shapes.push(Shape::Box {
                rect: f.rect,
                fill: ui::surface().opacity(0.35),
                border: Some(ui::border_strong()),
                radius: 8.,
                dashed: false,
            });
            for (y, _) in &f.branches {
                shapes.push(Shape::Line {
                    from: P { x: f.rect.x, y: *y },
                    to: P {
                        x: f.rect.right(),
                        y: *y,
                    },
                    color: ui::border_strong(),
                    width: 1.,
                    dash: Some([5., 4.]),
                });
            }
        }
        // Lifelines.
        for head in &l.heads {
            let x = head.x + head.w / 2.;
            shapes.push(Shape::Line {
                from: P { x, y: HEAD_H },
                to: P {
                    x,
                    y: l.lifeline_end,
                },
                color: ui::border_strong(),
                width: 1.25,
                dash: Some([6., 5.]),
            });
        }
        for (_, bar) in &l.bars {
            shapes.push(Shape::Box {
                rect: *bar,
                fill: ui::info().opacity(0.18),
                border: Some(ui::info().opacity(0.6)),
                radius: 2.,
                dashed: false,
            });
        }
        for (m, g) in shown.messages.iter().zip(&l.messages) {
            let picked = selected == Some(&format!("m:{}", m.id));
            let color = if picked {
                ui::accent()
            } else if m.kind == MessageKind::Return {
                ui::muted()
            } else {
                ui::fg().opacity(0.75)
            };
            let path = if g.self_loop {
                EdgePath {
                    segments: vec![Cubic {
                        from: g.from,
                        c1: P {
                            x: g.from.x + SELF_LOOP * 1.3,
                            y: g.from.y - 6.,
                        },
                        c2: P {
                            x: g.to.x + SELF_LOOP * 1.3,
                            y: g.to.y + 6.,
                        },
                        to: g.to,
                    }],
                }
            } else {
                canvas::straight(g.from, g.to)
            };
            shapes.push(Shape::Curve {
                path,
                color,
                width: if picked { 2. } else { 1.4 },
                dash: (m.kind == MessageKind::Return).then_some([5., 4.]),
                head: match m.kind {
                    MessageKind::Sync | MessageKind::SelfCall => Head::Filled,
                    MessageKind::Async | MessageKind::Return => Head::Open,
                },
            });
        }
        if let Some(SeqDrag::Connect {
            start,
            to,
            moved: true,
            ..
        }) = &s.drag
        {
            shapes.push(Shape::Rubber {
                from: *start,
                to: *to,
            });
        }

        // Lifeline grips (under everything else): drag one to another.
        for (lane, (p, head)) in shown.participants.iter().zip(&l.heads).enumerate() {
            let x = head.x + head.w / 2.;
            let (left, top) = c.at(x - 9., HEAD_H, rem);
            let id = p.id.clone();
            children.push(
                div()
                    .id(("genui-seq-lifeline", lane))
                    .debug_selector(move || format!("genui-seq-lifeline-{id}"))
                    .absolute()
                    .left(left)
                    .top(top)
                    .w(px(18. * scale))
                    .h(px((l.lifeline_end - HEAD_H) * scale))
                    .cursor(CursorStyle::Crosshair)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            this.press_lifeline(lane, event, window, cx)
                        }),
                    )
                    .into_any_element(),
            );
        }

        // Fragment tabs: the kind, then the condition.
        for (f, g) in shown.fragments.iter().zip(&l.fragments) {
            if !g.rect.intersects(&visible) {
                continue;
            }
            let (x, y) = c.at(g.rect.x, g.rect.y, rem);
            children.push(
                h_flex()
                    .absolute()
                    .left(x)
                    .top(y)
                    .gap(px(6. * scale))
                    .px(px(6. * scale))
                    .h(px(20. * scale))
                    .rounded_tl(px(8. * scale))
                    .rounded_br(px(6. * scale))
                    .bg(ui::surface())
                    .border_1()
                    .border_color(ui::border_strong())
                    .text_size(px(11. * scale))
                    .child(
                        div()
                            .font_family(ui::mono())
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ui::accent())
                            .child(fragment_name(f.kind)),
                    )
                    .children(
                        f.label
                            .clone()
                            .map(|label| div().text_color(ui::muted()).child(format!("[{label}]"))),
                    )
                    .into_any_element(),
            );
            for (y, label) in &g.branches {
                let Some(label) = label else { continue };
                let (x, y) = c.at(g.rect.x + 8., *y + 2., rem);
                children.push(
                    div()
                        .absolute()
                        .left(x)
                        .top(y)
                        .text_size(px(11. * scale))
                        .text_color(ui::muted())
                        .child(format!("[{label}]"))
                        .into_any_element(),
                );
            }
        }

        // Messages: a band to grab, the label as a pill on the arrow.
        for (ix, (m, g)) in shown.messages.iter().zip(&l.messages).enumerate() {
            if !g.row.intersects(&visible) {
                continue;
            }
            let key = format!("m:{}", m.id);
            let picked = selected == Some(key.as_str());
            let dragging = picked && matches!(&s.drag, Some(SeqDrag::Message { moved: true, .. }));
            let id = m.id.clone();
            let band = Rect {
                x: g.row.x,
                y: g.from.y - 14.,
                w: g.row.w.max(24.),
                h: 24.,
            };
            let (x, y) = c.at(band.x, band.y, rem);
            children.push(
                div()
                    .id(("genui-seq-message", ix))
                    .debug_selector({
                        let id = m.id.clone();
                        move || format!("genui-seq-message-{id}")
                    })
                    .absolute()
                    .left(x)
                    .top(y)
                    .w(px(band.w * scale))
                    .h(px(band.h * scale))
                    .rounded(px(6. * scale))
                    .cursor(CursorStyle::OpenHand)
                    .when(picked, |d| d.bg(ui::accent().opacity(0.06)))
                    .when(dragging, |d| d.bg(ui::accent().opacity(0.1)))
                    .hover(|d| d.bg(ui::accent().opacity(0.06)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            this.press_message(&id, event, window, cx)
                        }),
                    )
                    .into_any_element(),
            );
            if m.label.is_empty() || scale < 0.35 {
                continue;
            }
            let rect = label_rect(l, ix);
            let (x, y) = c.at(rect.x, rect.y, rem);
            children.push(
                h_flex()
                    .absolute()
                    .left(x)
                    .top(y)
                    .w(px(rect.w * scale))
                    .h(px(rect.h * scale))
                    .justify_center()
                    .when(g.self_loop, |d| d.justify_start())
                    .child(
                        div()
                            .px(px(6. * scale))
                            .rounded_full()
                            .bg(ui::well())
                            .max_w_full()
                            .truncate()
                            .text_size(px(11. * scale))
                            .text_color(if picked { ui::accent() } else { ui::fg() })
                            .child(m.label.clone()),
                    )
                    .into_any_element(),
            );
        }

        // Participant cards.
        for (p, head) in shown.participants.iter().zip(&l.heads) {
            let key = format!("p:{}", p.id);
            let picked = selected == Some(key.as_str());
            let color = kind_color(p.kind);
            let (x, y) = c.at(head.x, head.y, rem);
            let id = p.id.clone();
            children.push(
                h_flex()
                    .id(SharedString::from(format!(
                        "genui-seq-participant-{}",
                        p.id
                    )))
                    .debug_selector({
                        let id = p.id.clone();
                        move || format!("genui-seq-participant-{id}")
                    })
                    .absolute()
                    .left(x)
                    .top(y)
                    .w(px(head.w * scale))
                    .h(px(head.h * scale))
                    .gap(px(8. * scale))
                    .px(px(10. * scale))
                    .rounded(px(10. * scale))
                    .bg(ui::surface())
                    .border_1()
                    .border_color(if picked {
                        ui::accent()
                    } else {
                        ui::border_strong()
                    })
                    .shadow_sm()
                    .cursor(CursorStyle::OpenHand)
                    .hover(|d| d.bg(ui::surface_hover()))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            this.press_participant(&id, event, window, cx)
                        }),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .size(px(24. * scale))
                            .rounded(px(7. * scale))
                            .bg(color.opacity(0.14))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                Icon::new(kind_icon(p.kind))
                                    .size(px(14. * scale))
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
                                    .text_size(px(13. * scale))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(ui::fg())
                                    .child(p.label.clone()),
                            )
                            .when(scale >= 0.5, |d| {
                                d.child(
                                    div()
                                        .text_size(px(10.5 * scale))
                                        .text_color(ui::muted())
                                        .child(p.kind.name()),
                                )
                            }),
                    )
                    .into_any_element(),
            );
        }

        let mut minimap: Vec<(Rect, Hsla)> = shown
            .participants
            .iter()
            .zip(&l.heads)
            .map(|(p, r)| (*r, kind_color(p.kind).opacity(0.7)))
            .collect();
        minimap.extend(l.bars.iter().map(|(_, r)| (*r, ui::info().opacity(0.5))));
        let details = selected.and_then(|key| self.sequence_details(shown, key, cx));
        Drawn {
            shapes,
            children,
            minimap,
            details,
            cursor: matches!(s.drag, Some(SeqDrag::Connect { .. }))
                .then_some(CursorStyle::Crosshair),
            empty: shown
                .participants
                .is_empty()
                .then_some("Nothing drawn yet. Add a participant with +."),
        }
    }

    fn sequence_details(
        &self,
        shown: &Sequence,
        key: &str,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let (what, id) = key.split_once(':')?;
        let id = id.to_string();
        let rename = {
            let key = key.to_string();
            canvas::action("genui-seq-rename", IconName::Pencil, "Rename").on_click(
                cx.listener(move |this, _, window, cx| this.sequence_rename(&key, window, cx)),
            )
        };
        if what == "p" {
            let p = shown.participant(&id)?;
            let subject = format!("participant “{}”", p.label);
            let pid = id.clone();
            return Some(canvas::details(
                kind_icon(p.kind),
                kind_color(p.kind),
                p.label.clone(),
                vec![
                    canvas::pills(
                        "genui-seq-kind",
                        &ParticipantKind::ALL.map(ParticipantKind::name),
                        p.kind.name(),
                        move |this, kind, cx| {
                            this.sequence_edit(
                                Change::UpdateParticipant {
                                    participant: pid.clone(),
                                    label: None,
                                    kind: ParticipantKind::parse(kind),
                                },
                                cx,
                            );
                        },
                        cx,
                    ),
                    canvas::actions(vec![
                        rename,
                        canvas::action("genui-seq-ask", IconName::MessageSquare, "Ask about this")
                            .on_click(cx.listener(move |this, _, _, cx| this.ask(&subject, cx))),
                    ]),
                ],
                cx,
            ));
        }
        let m = shown.message(&id)?;
        let who = |p: &str| {
            shown
                .participant(p)
                .map_or(p, |p| p.label.as_str())
                .to_string()
        };
        let title = if m.label.is_empty() {
            format!("{} → {}", who(&m.from), who(&m.to))
        } else {
            m.label.clone()
        };
        let subject = format!("message “{title}”");
        let mut rows = vec![
            div()
                .text_color(ui::muted())
                .child(format!(
                    "{} → {} · step {}",
                    who(&m.from),
                    who(&m.to),
                    shown.index(&id).map_or(0, |i| i + 1)
                ))
                .into_any_element(),
        ];
        if m.from != m.to {
            let mid = id.clone();
            rows.push(canvas::pills(
                "genui-seq-message-kind",
                &["sync", "async", "return"],
                m.kind.name(),
                move |this, kind, cx| {
                    this.sequence_edit(
                        Change::update_message(&mid, None, MessageKind::parse(kind), None),
                        cx,
                    );
                },
                cx,
            ));
        }
        rows.extend(m.note.clone().map(|note| {
            div()
                .text_color(ui::muted())
                .whitespace_normal()
                .child(note)
                .into_any_element()
        }));
        let gone = id.clone();
        rows.push(canvas::actions(vec![
            rename,
            canvas::action("genui-seq-ask", IconName::MessageSquare, "Ask about this")
                .on_click(cx.listener(move |this, _, _, cx| this.ask(&subject, cx))),
            canvas::action("genui-seq-delete", IconName::Trash, "Delete")
                .on_click(cx.listener(move |this, _, _, cx| this.remove_message(&gone, cx))),
        ]));
        Some(canvas::details(
            IconName::ArrowLeftRight,
            ui::info(),
            title,
            rows,
            cx,
        ))
    }

    fn press_participant(
        &mut self,
        id: &str,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        window.focus(&self.focus, cx);
        self.finish_canvas_edit(true, cx);
        let key = format!("p:{id}");
        if event.click_count >= 2 {
            self.sequence_rename(&key, window, cx);
            return;
        }
        let rem = window.rem_size();
        if let Some(s) = state(self) {
            s.selected = Some(key);
            let x = s.canvas.to_world(event.position, rem).x;
            s.drag = Some(SeqDrag::Participant {
                id: id.to_string(),
                from: event.position,
                x,
                moved: false,
            });
        }
        cx.notify();
    }

    fn press_message(
        &mut self,
        id: &str,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        window.focus(&self.focus, cx);
        self.finish_canvas_edit(true, cx);
        let key = format!("m:{id}");
        if event.click_count >= 2 {
            self.sequence_rename(&key, window, cx);
            return;
        }
        let rem = window.rem_size();
        if let Some(s) = state(self) {
            s.selected = Some(key);
            let y = s.canvas.to_world(event.position, rem).y;
            s.drag = Some(SeqDrag::Message {
                id: id.to_string(),
                from: event.position,
                y,
                moved: false,
            });
        }
        cx.notify();
    }

    fn press_lifeline(
        &mut self,
        lane: usize,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        window.focus(&self.focus, cx);
        self.finish_canvas_edit(true, cx);
        let rem = window.rem_size();
        if let Some(s) = state(self) {
            let at = s.canvas.to_world(event.position, rem);
            let start = P {
                x: seq::lane_center(lane),
                y: at.y,
            };
            s.drag = Some(SeqDrag::Connect {
                lane,
                from: event.position,
                start,
                to: at,
                moved: false,
            });
        }
        cx.notify();
    }

    /// The background: nothing of its own (it pans).
    pub(super) fn sequence_down(
        &mut self,
        _: &MouseDownEvent,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> bool {
        false
    }

    pub(super) fn sequence_move(
        &mut self,
        event: &MouseMoveEvent,
        pressed: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let rem = window.rem_size();
        let Some(s) = state(self) else { return };
        let at = s.canvas.to_world(event.position, rem);
        match &mut s.drag {
            Some(SeqDrag::Participant {
                from, x, moved: m, ..
            }) if pressed => {
                *m |= moved(*from, event.position);
                *x = at.x;
            }
            Some(SeqDrag::Message {
                from, y, moved: m, ..
            }) if pressed => {
                *m |= moved(*from, event.position);
                *y = at.y;
            }
            Some(SeqDrag::Connect {
                from, to, moved: m, ..
            }) if pressed => {
                *m |= moved(*from, event.position);
                *to = at;
            }
            _ => return,
        }
        cx.notify();
    }

    pub(super) fn sequence_up(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(s) = state(self) else { return };
        let Some(drag) = s.drag.take() else { return };
        let working = s.edits.working().clone();
        let l = seq::layout(&working);
        let change = match drag {
            SeqDrag::Participant {
                id, x, moved: true, ..
            } => {
                let order = Sequence::moved(
                    working.participants.iter().map(|p| p.id.as_str()),
                    &id,
                    l.lane_at(x),
                );
                Change::OrderParticipants {
                    participants: order,
                }
            }
            SeqDrag::Message {
                id, y, moved: true, ..
            } => {
                let index = l.insert_at(y, working.index(&id));
                Change::OrderMessages {
                    messages: Sequence::moved(
                        working.messages.iter().map(|m| m.id.as_str()),
                        &id,
                        index,
                    ),
                }
            }
            SeqDrag::Connect {
                lane,
                start,
                to,
                moved: true,
                ..
            } => {
                let target = l.lane_at(to.x);
                let (Some(a), Some(b)) = (
                    working.participants.get(lane),
                    working.participants.get(target),
                ) else {
                    return;
                };
                let message = Message::new(working.fresh_message_id(), &a.id, &b.id, "");
                let id = message.id.clone();
                let index = l.insert_at(start.y, None);
                if self.sequence_edit(Change::AddMessage { message, index }, cx) {
                    let key = format!("m:{id}");
                    if let Some(s) = state(self) {
                        s.selected = Some(key.clone());
                    }
                    self.sequence_rename(&key, window, cx);
                }
                return;
            }
            _ => return,
        };
        self.sequence_edit(change, cx);
    }

    /// The rename field for `key` (`p:<id>` or `m:<id>`).
    fn sequence_rename(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(s) = self.sequence() else { return };
        let working = s.edits.working().clone();
        let l = seq::layout(&working);
        let Some((what, id)) = key.split_once(':') else {
            return;
        };
        let (text, at) = match what {
            "p" => {
                let Some(lane) = working.lane(id) else { return };
                (working.participants[lane].label.clone(), l.heads[lane])
            }
            _ => {
                let Some(ix) = working.index(id) else { return };
                (working.messages[ix].label.clone(), label_rect(&l, ix))
            }
        };
        if let Some(s) = state(self) {
            s.selected = Some(key.to_string());
            s.drag = None;
        }
        self.start_canvas_edit(key.to_string(), text, at, window, cx);
    }

    pub(super) fn sequence_text(&mut self, target: &str, text: String, cx: &mut Context<Self>) {
        let Some(s) = self.sequence() else { return };
        let working = s.edits.working();
        let change = match target.split_once(':') {
            Some(("p", id)) => {
                let same = working.participant(id).is_none_or(|p| p.label == text);
                if same || text.is_empty() {
                    return;
                }
                Change::UpdateParticipant {
                    participant: id.to_string(),
                    label: Some(text),
                    kind: None,
                }
            }
            Some((_, id)) => {
                if working.message(id).is_none_or(|m| m.label == text) {
                    return;
                }
                Change::update_message(id, Some(text), None, None)
            }
            None => return,
        };
        self.sequence_edit(change, cx);
    }

    fn remove_message(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.sequence_edit(
            Change::RemoveMessage {
                message: id.to_string(),
            },
            cx,
        ) && let Some(s) = state(self)
        {
            s.selected = None;
        }
    }

    /// The header's +: a participant at the right end, named at once.
    pub(super) fn add_participant(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(s) = self.sequence() else { return };
        let working = s.edits.working();
        let label = format!("Participant {}", working.participants.len() + 1);
        let participant = Participant {
            id: working.fresh_participant_id(&label),
            label,
            kind: ParticipantKind::Service,
        };
        let key = format!("p:{}", participant.id);
        if self.sequence_edit(Change::AddParticipant { participant }, cx) {
            self.sequence_rename(&key, window, cx);
        }
    }

    pub(super) fn sequence_key(
        &mut self,
        keystroke: &Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.canvas_key(keystroke, window, cx) {
            return true;
        }
        if self.typing(window, cx) {
            return false;
        }
        let Some(s) = self.sequence() else {
            return false;
        };
        let Some(key) = s.selected.clone() else {
            return false;
        };
        let working = s.edits.working().clone();
        let Some((what, id)) = key.split_once(':') else {
            return false;
        };
        let alt = keystroke.modifiers.alt;
        match (keystroke.key.as_str(), what) {
            ("backspace" | "delete", "m") => self.remove_message(id, cx),
            ("enter", _) => self.sequence_rename(&key, window, cx),
            ("escape", _) => self.canvas_select(None, cx),
            ("up" | "down", "m") if alt => {
                let Some(at) = working.index(id) else {
                    return false;
                };
                let to = if keystroke.key == "up" {
                    at.checked_sub(1)
                } else {
                    Some(at + 1).filter(|i| *i < working.messages.len())
                };
                let Some(to) = to else { return true };
                let order = Sequence::moved(working.messages.iter().map(|m| m.id.as_str()), id, to);
                self.sequence_edit(Change::OrderMessages { messages: order }, cx);
            }
            ("left" | "right", "p") if alt => {
                let Some(at) = working.lane(id) else {
                    return false;
                };
                let to = if keystroke.key == "left" {
                    at.checked_sub(1)
                } else {
                    Some(at + 1).filter(|i| *i < working.participants.len())
                };
                let Some(to) = to else { return true };
                let order =
                    Sequence::moved(working.participants.iter().map(|p| p.id.as_str()), id, to);
                self.sequence_edit(
                    Change::OrderParticipants {
                        participants: order,
                    },
                    cx,
                );
            }
            _ => return false,
        }
        true
    }
}
