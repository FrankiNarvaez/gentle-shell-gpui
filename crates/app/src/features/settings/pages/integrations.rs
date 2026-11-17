//! Integration page composition, retaining the underlying feature state.

use gpui_kit::component::v_flex;
use gpui_kit::*;

use super::super::{SettingsView, snapshot::Snapshot};

pub(super) fn render(
    view: &SettingsView,
    snapshot: &Snapshot,
    cx: &mut Context<SettingsView>,
) -> Div {
    v_flex()
        .gap_3()
        .child(view.maestro_section(cx))
        .child(view.music_section(cx))
        .child(view.bridge_section(snapshot, cx))
}
