//! Appearance page. Reuses the existing theme UI, import and drop handlers.

use gpui_kit::component::v_flex;
use gpui_kit::*;

use super::super::SettingsView;
use crate::shared::ui;

pub(super) fn render(view: &SettingsView, cx: &mut Context<SettingsView>) -> Div {
    v_flex()
        .gap_3()
        .child(
            div()
                .id("theme-drop")
                .rounded_xl()
                .drag_over::<ExternalPaths>(|style, _, _, _| {
                    style.bg(ui::surface_hover())
                })
                .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                    this.theme_drop(paths, cx)
                }))
                .child(view.theme_section(cx)),
        )
}
