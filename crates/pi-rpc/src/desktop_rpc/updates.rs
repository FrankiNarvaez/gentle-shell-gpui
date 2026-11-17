//! `get_update_status`: whether Gentle Shell, Pi and Pi's packages have
//! updates, from the sources that own those answers. A plain read uses only
//! what is already known; `refresh` runs the lookups once (bounded by 15
//! seconds). Nothing is ever claimed up to date without a successful lookup,
//! and running an update stays the launcher's job: the client shows `command`.

use super::{DesktopRpcError, decode_data};
use serde::Deserialize;
use serde_json::Value;

wire_enum! {
    /// How this Gentle Shell was installed.
    ShellChannel, "channel" {
        Release => "release",
        Main => "main",
        Bundled => "bundled",
        GentleAiManaged => "gentle-ai-managed",
        Unknown => "unknown",
    }
}

wire_enum! {
    /// Gentle Shell's and Pi's update state. `up_to_date` only after a
    /// successful lookup returned exactly the running version; anything not
    /// looked up, offline, failed or not comparable is `unknown`.
    UpdateState, "state" {
        UpdateAvailable => "update_available",
        UpToDate => "up_to_date",
        Unknown => "unknown",
    }
}

wire_enum! {
    /// A package's update state. Pi's check cannot tell a failed lookup from
    /// no update, so a package it did not list is `no_update_reported`, never
    /// up to date; `skipped` is a package Pi never checks.
    PackageUpdateState, "state" {
        UpdateAvailable => "update_available",
        NoUpdateReported => "no_update_reported",
        Skipped => "skipped",
        Unknown => "unknown",
    }
}

wire_enum! {
    /// Where a configured Pi package comes from.
    PackageKind, "kind" {
        Npm => "npm",
        Git => "git",
        Local => "local",
        Unknown => "unknown",
    }
}

wire_enum! {
    /// Which Pi settings configure a package.
    PackageScope, "scope" {
        User => "user",
        Project => "project",
    }
}

/// Gentle Shell's own update. `latest` is the latest stable release (for
/// `main`, the first 12 characters of the latest commit); `command` is `None`
/// for an `unknown` channel. `checked_at` is ISO 8601, `None` when no lookup
/// backs the answer.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellUpdate {
    pub current: Option<String>,
    pub latest: Option<String>,
    pub channel: ShellChannel,
    pub state: UpdateState,
    pub command: Option<String>,
    pub checked_at: Option<String>,
}

/// Pi's update. Only a bundled install can name the latest Pi.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PiUpdate {
    pub current: Option<String>,
    pub latest: Option<String>,
    pub state: UpdateState,
    pub command: String,
    pub checked_at: Option<String>,
}

/// One configured Pi package, in Pi's order. `latest` is always `None` today
/// (Pi's check reports only that a newer version exists); a git package has
/// no `current`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PackageUpdate {
    pub name: String,
    pub kind: PackageKind,
    pub scope: PackageScope,
    pub current: Option<String>,
    pub latest: Option<String>,
    pub pinned: Option<bool>,
    pub state: PackageUpdateState,
}

/// Pi's packages. `available` is `false` when Pi's listing could not be read
/// (then `items` is empty); at most 100 items, with `truncated`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackagesUpdate {
    pub available: bool,
    pub items: Vec<PackageUpdate>,
    pub truncated: bool,
    pub command: String,
    pub checked_at: Option<String>,
}

/// The decoded `get_update_status` data. `refreshed` is `true` only when this
/// answer ran the lookups; `offline` reports `PI_OFFLINE`, under which
/// nothing is looked up.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    pub refreshed: bool,
    pub offline: bool,
    pub gentle_shell: ShellUpdate,
    pub pi: PiUpdate,
    pub packages: PackagesUpdate,
}

impl UpdateStatus {
    pub(super) fn from_data(data: &Value) -> Result<Self, DesktopRpcError> {
        decode_data(data)
    }
}
