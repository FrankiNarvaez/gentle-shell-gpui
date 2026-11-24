//! `get_status`: the facts `/gentle:status` prints for this session's
//! project (persona, the `gentle-ai` dev-binary override, managed assets,
//! model routing) with the severity `/gentle:status` warns at. No path,
//! command hint or display text crosses the wire.

use super::{DesktopRpcError, decode_data};
use serde::Deserialize;
use serde_json::Value;

wire_enum! {
    /// The `gentle-ai` dev-binary override: none, one in use, or one declared
    /// but unusable.
    DevBinaryState, "state" {
        Inactive => "inactive",
        Active => "active",
        Invalid => "invalid",
    }
}

wire_enum! {
    /// Whether the global `models.json` exists.
    ModelConfigPresence, "global" {
        Present => "present",
        Missing => "missing",
    }
}

wire_enum! {
    /// The saved model routing.
    RoutingState, "routing" {
        Missing => "missing",
        Invalid => "invalid",
        Valid => "valid",
    }
}

wire_enum! {
    /// `warning` exactly when `/gentle:status` warns: invalid routing, an
    /// active or invalid dev binary, stale assets or active agent overrides.
    StatusSeverity, "severity" {
        Info => "info",
        Warning => "warning",
    }
}

/// The dev-binary override. The binary is never run, so no version is
/// reported; `sha256_prefix` (16 hex characters) only while `active`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DevBinary {
    pub state: DevBinaryState,
    pub sha256_prefix: Option<String>,
}

/// One owner's managed assets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetStatus {
    /// Missing from the agent home or different from the packaged copy;
    /// `/gentle:install-<owner> --force` refreshes them.
    pub stale: u64,
    /// Installed files the user replaced: preserved, not drift.
    pub user_overrides: u64,
    /// Active non-builtin agents that shadow the owner's package agents.
    pub active_agent_overrides: u64,
}

/// The managed assets of each owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct StatusAssets {
    pub delegation: AssetStatus,
    pub review: AssetStatus,
}

/// The model configuration. Per-agent routing is read with `get_config`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct ModelConfigStatus {
    pub global: ModelConfigPresence,
    pub routing: RoutingState,
}

/// The decoded `get_status` data. `package` and `odd` are `active` on every
/// Shell that answers; `persona` is the one in effect (the project's wins).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellStatus {
    pub package: String,
    pub odd: String,
    pub persona: String,
    pub dev_binary: DevBinary,
    pub assets: StatusAssets,
    pub model_config: ModelConfigStatus,
    pub severity: StatusSeverity,
}

impl ShellStatus {
    pub(super) fn from_data(data: &Value) -> Result<Self, DesktopRpcError> {
        decode_data(data)
    }
}
