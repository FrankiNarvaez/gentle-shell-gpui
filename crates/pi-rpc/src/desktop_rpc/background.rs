//! `get_background_subagents` and `set_background_subagents`: the
//! background-subagents policy, read and written through the
//! `/gentle:background-subagents` owner. A change takes two calls, like
//! `set_review_mode`: the first answers a confirmation and writes nothing;
//! only a resend with its token writes the machine-wide file.

use super::{
    DesktopRpcError, MAX_PROFILE_APPLY_UNITS, bool_field, closed_field, decode_data, field_object,
    string_field,
};
use serde::Deserialize;
use serde_json::{Value, json};

wire_enum! {
    /// The policy: whether background subagents may be launched.
    BackgroundPolicy, "policy" {
        On => "on",
        Off => "off",
    }
}

wire_enum! {
    /// What decided the policy, in the owner's order.
    BackgroundSource, "source" {
        Project => "project",
        Global => "global",
        Environment => "environment",
        Default => "default",
    }
}

wire_enum! {
    /// Whether a policy file exists. No path crosses the wire.
    PolicyFile, "file" {
        Present => "present",
        Absent => "absent",
    }
}

wire_enum! {
    /// Whether this session can launch background subagents at all.
    BackgroundCapability, "capability" {
        Ready => "ready",
        Absent => "absent",
    }
}

/// The decoded `get_background_subagents` data, and the `state` every
/// `set_background_subagents` answer carries. `malformed` means the deciding
/// file exists but is not a valid policy file (the policy is then `off`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundSubagents {
    pub effective: BackgroundPolicy,
    pub source: BackgroundSource,
    pub malformed: bool,
    pub project_file: PolicyFile,
    pub global_file: PolicyFile,
    pub capability: BackgroundCapability,
}

impl BackgroundSubagents {
    pub(super) fn from_data(data: &Value) -> Result<Self, DesktopRpcError> {
        decode_data(data)
    }
}

/// The `set_background_subagents` params: exactly `{ policy }`, plus
/// `confirmationToken` when resending an approved confirmation. There is no
/// scope: only the machine-wide file is ever written.
pub(super) fn set_background_subagents_params(
    policy: BackgroundPolicy,
    confirmation_token: Option<&str>,
) -> Result<Value, DesktopRpcError> {
    Ok(match confirmation_token {
        None => json!({ "policy": policy.as_str() }),
        Some(token) => {
            if !(1..=MAX_PROFILE_APPLY_UNITS).contains(&token.encode_utf16().count()) {
                return Err(DesktopRpcError::InvalidConfirmationToken);
            }
            json!({ "policy": policy.as_str(), "confirmationToken": token })
        }
    })
}

/// The dialog Desktop must show before a change. The token is bound to this
/// exact text and to the state it describes; a stale token is answered with
/// a fresh confirmation, and nothing is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundConfirmation {
    pub title: String,
    pub message: String,
    pub token: String,
}

/// How a `set_background_subagents` request ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetBackgroundSubagentsOutcome {
    /// Nothing was written: show this dialog, and resend with its token only
    /// when the human approves.
    ConfirmationRequired(BackgroundConfirmation),
    /// The machine-wide file was written and the policy re-read. `changed`
    /// is whether the re-read state differs from the one shown; `effective`
    /// can stay the same when this project's file decides.
    Applied { changed: bool },
}

/// The decoded `set_background_subagents` data: the policy asked for, the
/// current (or re-read) state, and the outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetBackgroundSubagentsResult {
    pub policy: BackgroundPolicy,
    pub state: BackgroundSubagents,
    pub outcome: SetBackgroundSubagentsOutcome,
}

impl SetBackgroundSubagentsResult {
    pub(super) fn from_data(data: &Value) -> Result<Self, DesktopRpcError> {
        let status = string_field(data, "status")?;
        let outcome = match status.as_str() {
            "confirmation_required" => {
                let confirmation = field_object(data, "confirmation")?;
                let token = string_field(confirmation, "token")?;
                if token.is_empty() {
                    return Err(DesktopRpcError::MalformedPayload(
                        "confirmation token must not be empty".into(),
                    ));
                }
                SetBackgroundSubagentsOutcome::ConfirmationRequired(BackgroundConfirmation {
                    title: string_field(confirmation, "title")?,
                    message: string_field(confirmation, "message")?,
                    token,
                })
            }
            "applied" => SetBackgroundSubagentsOutcome::Applied {
                changed: bool_field(data, "changed")?,
            },
            _ => {
                return Err(DesktopRpcError::InvalidValue {
                    field: "status",
                    value: status,
                });
            }
        };
        Ok(Self {
            policy: closed_field(data, "policy")?,
            state: BackgroundSubagents::from_data(field_object(data, "state")?)?,
            outcome,
        })
    }
}
