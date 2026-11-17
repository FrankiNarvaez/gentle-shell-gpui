//! Agent-related controls reuse the existing background and hydration flows.

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
        .child(view.hydration_section(cx))
        .child(view.gentle_ai_section(snapshot, cx))
        .child(view.agent_ui_section(cx))
}
