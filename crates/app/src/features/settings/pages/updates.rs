//! Application, Gentle Shell and Engram update controls.
//!
//! The existing upgrade handlers, jobs and error reporting remain unchanged.

use gpui_kit::assets::IconName;
use gpui_kit::component::v_flex;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::super::{
    SettingsView, app, layout::{row, section}, snapshot::Snapshot, upgrade,
};

pub(super) fn render(
    view: &SettingsView,
    snapshot: &Snapshot,
    cx: &mut Context<SettingsView>,
) -> Div {
    v_flex()
        .gap_3()
        .child(app::app_section(cx))
        .child(backend_updates_section(view, snapshot, cx))
}

fn backend_updates_section(
    view: &SettingsView,
    snapshot: &Snapshot,
    cx: &mut Context<SettingsView>,
) -> Div {
    section(IconName::RefreshCw, "Gentle Shell and Engram")
        .child(row("Versions", snapshot.versions.join(" · ")))
        .when_some(snapshot.update.clone(), |card, offer| {
            card.child(view.render_update(offer, cx))
        })
        .children(view.upgrade.as_ref().map(upgrade::upgrade_result))
        .child(view.render_engram_update(snapshot.engram_update.clone(), cx))
}
