//! The diagrams' shared look, after Eraser: every element takes a tone (its
//! `color`, else its kind's) and wears it as a soft fill, a colored hairline
//! and a solid icon badge; groups are tinted containers with a label tag;
//! edge labels sit on the canvas color so the line breaks around them;
//! arrowheads are open chevrons. Architecture diagrams, state machines,
//! sequence diagrams, data models and timelines all draw from here, so they
//! read as one family.

use gentle_core::genui::style::Tint;
use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::*;

use crate::shared::ui;

/// A card's corner radius (px at 100 %).
pub(super) const CARD_RADIUS: f32 = 12.;
/// A group container's corner radius.
pub(super) const GROUP_RADIUS: f32 = 16.;
/// The icon badge's side.
pub(super) const BADGE: f32 = 26.;
/// An open arrowhead's length and half-width (px at 100 %).
pub(super) const ARROW_LEN: f32 = 8.;
pub(super) const ARROW_HALF: f32 = 4.5;
/// An edge's stroke width at 100 %.
pub(super) const EDGE_WIDTH: f32 = 1.5;

/// The bundled icon a diagram element names (`gentle_core::genui::style::ICONS`).
pub(super) fn icon(name: &str) -> Option<IconName> {
    let path = format!("icons/{name}.svg");
    IconName::ALL
        .iter()
        .copied()
        .find(|icon| icon.path().as_ref() == path)
}

/// An element's tone: its `color`, else `fallback` (its kind's).
pub(super) fn tone(color: Option<Tint>, fallback: Hsla) -> Hsla {
    color.map_or(fallback, ui::tint)
}

/// A card's fill: the surface washed with its tone.
pub(super) fn fill(tone: Hsla) -> Hsla {
    ui::surface().blend(tone.opacity(0.14))
}

/// A card's fill under the pointer: the hover surface washed like [`fill`],
/// so the card lifts a step and keeps its tint.
pub(super) fn hover_fill(tone: Hsla) -> Hsla {
    ui::surface_hover().blend(tone.opacity(0.14))
}

/// A card's hairline in its tone.
pub(super) fn edge_color(tone: Hsla) -> Hsla {
    tone.opacity(0.55)
}

/// What sits on a solid tone (a badge's icon): the floating surface, dark on
/// dark themes and light on light ones, so it reads on any tone.
pub(super) fn on_tone() -> Hsla {
    ui::popover_bg()
}

/// The soft drop shadow under cards (scaled with the canvas).
pub(super) fn shadow(s: f32) -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: hsla(0., 0., 0., 0.28),
        offset: point(px(0.), px(3. * s)),
        blur_radius: px(10. * s),
        spread_radius: px(0.),
        inset: false,
    }]
}

/// A card at canvas scale `s` in `tone`: tinted fill, colored hairline,
/// rounded corners and the shadow. Callers add size, padding and content.
pub(super) fn card(tone: Hsla, s: f32) -> Div {
    div()
        .bg(fill(tone))
        .border_1()
        .border_color(edge_color(tone))
        .rounded(px(CARD_RADIUS * s))
        .shadow(shadow(s))
}

/// The solid icon badge (side `size`·`s`) in `tone`, `icon` on it.
pub(super) fn badge(tone: Hsla, icon: IconName, size: f32, s: f32) -> Div {
    div()
        .flex_none()
        .size(px(size * s))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(size * 0.3 * s))
        .bg(tone)
        .child(
            Icon::new(icon)
                .size(px(size * 0.58 * s))
                .text_color(on_tone()),
        )
}

/// A group container's fill and hairline in `tone`.
pub(super) fn group_fill(tone: Hsla) -> Hsla {
    tone.opacity(0.07)
}

pub(super) fn group_border(tone: Hsla) -> Hsla {
    tone.opacity(0.42)
}

/// A group's label tag (top-left inside the container): `label` in caps on
/// its tone, with `icon` before it when given.
pub(super) fn group_tag(tone: Hsla, label: &str, icon: Option<IconName>, s: f32) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(4. * s))
        .px(px(6. * s))
        .h(px(18. * s))
        .rounded(px(5. * s))
        .bg(tone.opacity(0.16))
        .border_1()
        .border_color(tone.opacity(0.4))
        .text_size(px(9.5 * s))
        .line_height(px(12. * s))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(tone)
        .whitespace_nowrap()
        .children(icon.map(|icon| Icon::new(icon).size(px(10. * s)).text_color(tone)))
        .child(label.to_uppercase())
}

/// An edge label's pill: on the canvas color (the line breaks around it)
/// with a faint hairline.
pub(super) fn label_pill(s: f32) -> Div {
    div()
        .px(px(6. * s))
        .py(px(1.5 * s))
        .rounded(px(6. * s))
        .bg(ui::well())
        .border_1()
        .border_color(ui::border())
        .text_size(px(10.5 * s))
        .line_height(px(13. * s))
        .text_color(ui::fg().opacity(0.82))
}

/// An edge's default color: soft enough that the cards lead.
pub(super) fn line() -> Hsla {
    ui::muted().opacity(0.85)
}

/// The two strokes of an open chevron arrowhead ending at `tip`, pointing
/// along `dir` (a unit vector), at scale `s`: the polyline `left → tip →
/// right`.
pub(super) fn chevron(tip: Point<Pixels>, dir: (f32, f32), s: f32) -> [Point<Pixels>; 3] {
    let (dx, dy) = dir;
    let (len, half) = (ARROW_LEN * s, ARROW_HALF * s);
    let back = point(tip.x - px(dx * len), tip.y - px(dy * len));
    [
        point(back.x - px(dy * half), back.y + px(dx * half)),
        tip,
        point(back.x + px(dy * half), back.y - px(dx * half)),
    ]
}
