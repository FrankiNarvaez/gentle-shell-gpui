//! Render the selected Configuration page.
//!
//! This module composes the existing settings sections without changing their
//! state, event handling, persistence, or backend integrations.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::navigation::SettingsPage;
use super::snapshot::Snapshot;
use super::{SettingsView, app, layout::{row, section}, models};
use crate::shared::ui;

impl SettingsView {
    pub(super) fn render_page(
        &self,
        page: SettingsPage,
        snapshot: &Snapshot,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let sections: Div = match page {
            SettingsPage::General => v_flex()
                .gap_3()
                .child(self.general_section(snapshot))
                .child(
                    section(IconName::CircleCheck, "Setup & requirements")
                        .child(self.setup.clone()),
                ),
            SettingsPage::Appearance => v_flex()
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
                        .child(self.theme_section(cx)),
                ),
            SettingsPage::Agents => v_flex()
                .gap_3()
                .child(self.hydration_section(cx))
                .child(self.gentle_ai_section(snapshot, cx)),
            SettingsPage::Models => v_flex()
                .gap_3()
                .child(models::models_section(snapshot)),
            SettingsPage::Devices => v_flex()
                .gap_3()
                .child(self.devices_section(cx)),
            SettingsPage::Integrations => v_flex()
                .gap_3()
                .child(self.maestro_section(cx))
                .child(self.music_section(cx))
                .child(self.bridge_section(snapshot, cx)),
            SettingsPage::Notifications => v_flex()
                .gap_3()
                .child(self.notifications_section(cx)),
            SettingsPage::Updates => v_flex()
                .gap_3()
                .child(app::app_section(cx))
                .child(self.backend_updates_section(snapshot, cx)),
            SettingsPage::Advanced => v_flex()
                .gap_3()
                .child(self.packages_section(snapshot, cx)),
        };

        ui::appear("settings-content", sections.p_4()).into_any_element()
    }

    fn general_section(&self, snapshot: &Snapshot) -> Div {
        section(IconName::SquareTerminal, "Gentle Shell")
            .child(row(
                "Home",
                if snapshot.home_mode.is_empty() {
                    snapshot.agent.display().to_string()
                } else {
                    snapshot.home_mode.clone()
                },
            ))
            .child(row("Agent dir", snapshot.agent.display().to_string()))
            .child(row("Config dir", snapshot.config.display().to_string()))
            .child(row(
                "Launcher",
                snapshot
                    .launcher
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "not found".into()),
            ))
            .child(row("Versions", snapshot.versions.join(" · ")))
            .child(
                h_flex()
                    .pt_2()
                    .gap_2()
                    .flex_wrap()
                    .child({
                        let dir = snapshot.agent.clone();
                        Button::new("open-agent-dir")
                            .ghost()
                            .small()
                            .icon(Icon::new(IconName::FolderOpen))
                            .label("Open agent dir")
                            .on_click(move |_, _, cx| {
                                cx.open_url(&format!("file://{}", dir.display()))
                            })
                    })
                    .child({
                        let dir = snapshot.config.clone();
                        Button::new("open-config-dir")
                            .ghost()
                            .small()
                            .icon(Icon::new(IconName::FolderOpen))
                            .label("Open config dir")
                            .on_click(move |_, _, cx| {
                                cx.open_url(&format!("file://{}", dir.display()))
                            })
                    }),
            )
    }

    fn backend_updates_section(
        &self,
        snapshot: &Snapshot,
        cx: &mut Context<Self>,
    ) -> Div {
        section(IconName::RefreshCw, "Gentle Shell and Engram")
            .child(row("Versions", snapshot.versions.join(" · ")))
            .when_some(snapshot.update.clone(), |card, offer| {
                card.child(self.render_update(offer, cx))
            })
            .children(self.upgrade.as_ref().map(super::upgrade::upgrade_result))
            .child(self.render_engram_update(snapshot.engram_update.clone(), cx))
    }
}
