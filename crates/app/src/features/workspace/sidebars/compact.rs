//! Narrow workspace rail. It uses the same session and workspace model as
//! the expanded navigator, without changing the right-hand Files panel.

use super::super::menu::WorkspaceMenuSource;
use super::*;

impl Workspace {
    pub(super) fn render_compact_sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut list = v_flex()
            .id("compact-workspaces")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .w_full()
            .items_center()
            .gap_1();

        for space in self.sidebar_spaces() {
            let index = space.index;
            let tip = space.tooltip();
            let color = space.mark.hue();
            let row = h_flex()
                .id(("compact-workspace", index))
                .relative()
                .size(rems(34. / 16.))
                .flex_none()
                .justify_center()
                .rounded_md()
                .bg(color.opacity(0.12))
                .border_1()
                .border_color(if space.current {
                    color
                } else {
                    gpui_kit::transparent_black()
                })
                .hover(|d| d.bg(ui::surface_hover()))
                .cursor_pointer()
                .when(!self.popups_open(), |d| {
                    d.tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                })
                .on_click(cx.listener(move |this, _, window, cx| {
                    if this.current != index {
                        this.switch_workspace(index, window, cx);
                    }
                }))
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        this.open_workspace_menu(
                            index,
                            WorkspaceMenuSource::Chip,
                            event.position,
                            window,
                            cx,
                        );
                    }),
                )
                .child(space.mark.glyph(13., 2, color))
                .when_some(space.dot(), |d, (color, _)| {
                    d.child(
                        div()
                            .absolute()
                            .top(px(2.))
                            .right(px(2.))
                            .size(px(6.))
                            .rounded_full()
                            .bg(color),
                    )
                });
            list = list.child(row);
            if space.current {
                for tab in self.sidebar_tabs(index) {
                    let tip = tab.tooltip();
                    let row = h_flex()
                        .id(("compact-tab", tab.id))
                        .relative()
                        .size(rems(30. / 16.))
                        .flex_none()
                        .justify_center()
                        .rounded_md()
                        .when(tab.shown, |d| d.bg(ui::raised()))
                        .hover(|d| d.bg(ui::surface_hover()))
                        .when(!self.popups_open(), |d| {
                            d.tooltip(move |window, cx| {
                                Tooltip::new(tip.clone()).build(window, cx)
                            })
                        })
                        .child(Icon::new(tab.kind.icon()).small().text_color(tab.tone()))
                        .children(tab.status_dot().map(|dot| {
                            div().absolute().top(px(2.)).right(px(2.)).child(dot)
                        }));
                    list = list.child(self.tab_row_behavior(row, &tab, cx));
                }
                list = list.child(
                    h_flex()
                        .id("compact-add-session")
                        .size(rems(30. / 16.))
                        .flex_none()
                        .justify_center()
                        .rounded_md()
                        .cursor_pointer()
                        .hover(|d| d.bg(ui::surface_hover()))
                        .when(!self.popups_open(), |d| {
                            d.tooltip(|window, cx| {
                                Tooltip::new("New agent or terminal").build(window, cx)
                            })
                        })
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.open_workspace_add_menu(index, event.position, window, cx);
                            }),
                        )
                        .child(Icon::new(IconName::Plus).small().text_color(ui::muted())),
                );
            }
        }

        v_flex()
            .size_full()
            .items_center()
            .gap_1()
            .py_2()
            .child(
                Button::new("compact-expand")
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(IconName::PanelLeftOpen))
                    .tooltip("Expand workspace sidebar")
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar_compact(cx))),
            )
            .child(list)
            .child(
                Button::new("compact-new-workspace")
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(IconName::FolderPlus))
                    .tooltip("New workspace")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.pick_workspace_folder(None, window, cx);
                    })),
            )
            .into_any_element()
    }
}
