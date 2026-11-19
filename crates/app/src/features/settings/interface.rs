//! Tab placement controls for Configuration → Appearance.
//! State belongs to the workspace's existing saved layout.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Icon, Selectable as _, Sizable as _, h_flex};
use gpui_kit::*;

use super::layout::section;
use super::{SettingsEvent, SettingsView};
use crate::features::workspaces::TabLayout;
use crate::shared::ui;

impl SettingsView {
    pub(super) fn interface_section(&self, cx: &mut Context<Self>) -> Div {
        section(IconName::PanelLeft, "Tab placement")
            .child(
                h_flex()
                    .pt_2()
                    .gap_3()
                    .items_center()
                    .child(
                        div()
                            .w(rems(120. / 16.))
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(ui::muted())
                            .child("Tabs"),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Button::new("settings-horizontal-tabs")
                                    .ghost()
                                    .small()
                                    .selected(self.tab_layout == TabLayout::Horizontal)
                                    .icon(Icon::new(IconName::List))
                                    .label("Horizontal")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.select_tab_layout(TabLayout::Horizontal, cx)
                                    })),
                            )
                            .child(
                                Button::new("settings-vertical-tabs")
                                    .ghost()
                                    .small()
                                    .selected(self.tab_layout == TabLayout::Vertical)
                                    .icon(Icon::new(IconName::PanelLeft))
                                    .label("Vertical")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.select_tab_layout(TabLayout::Vertical, cx)
                                    })),
                            ),
                    ),
            )
            .child(
                div()
                    .pt_2()
                    .text_xs()
                    .text_color(ui::faint())
                    .child("Tabs appear at the top or on the left. Files always opens on the right."),
            )
    }

    fn select_tab_layout(&mut self, layout: TabLayout, cx: &mut Context<Self>) {
        if self.tab_layout == layout {
            return;
        }
        self.tab_layout = layout;
        cx.emit(SettingsEvent::TabLayoutChanged(layout));
        cx.notify();
    }
}
