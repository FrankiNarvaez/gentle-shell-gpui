//! Model configuration is already provided by the dedicated models module.

use gpui_kit::component::v_flex;
use gpui_kit::*;

use super::super::{models, snapshot::Snapshot};

pub(super) fn render(snapshot: &Snapshot) -> Div {
    v_flex()
        .gap_3()
        .child(models::models_section(snapshot))
}
