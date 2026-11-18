//! `get_review_mode`, `set_review_mode` and `review_mode_changed`: the
//! receipt-driven development (RDD) switch, read and written through the
//! `/gentle:review-mode` owner. A change takes two calls: the first answers a
//! confirmation and writes nothing; only a resend with its token writes.

use super::{
    DesktopRpcError, MAX_PROFILE_APPLY_UNITS, bool_field, closed_field, decode_data, field_object,
    string_field,
};
use serde::Deserialize;
use serde_json::{Value, json};

wire_enum! {
    /// Whether reviews run in this workspace. `unknown` is a failed,
    /// timed-out or unrecognized read: the server fails closed.
    ReviewEffective, "effective" {
        Enabled => "enabled",
        Disabled => "disabled",
        Unknown => "unknown",
    }
}

wire_enum! {
    /// Which setting decided the effective mode: none (on by default), the
    /// machine-wide setting, or this repository's.
    ReviewSource, "source" {
        Default => "default",
        Global => "global",
        Clone => "clone",
        Unknown => "unknown",
    }
}

wire_enum! {
    /// One stored setting.
    ReviewSetting, "scopes" {
        Enabled => "enabled",
        Disabled => "disabled",
        Unset => "unset",
        Unknown => "unknown",
    }
}

wire_enum! {
    /// Which setting `set_review_mode` changes: this repository's or the
    /// machine-wide one.
    ReviewScope, "scope" {
        Clone => "clone",
        Global => "global",
    }
}

/// The stored machine-wide and repository settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct ReviewScopes {
    pub global: ReviewSetting,
    pub clone: ReviewSetting,
}

/// The decoded `get_review_mode` data, and the `mode` every
/// `set_review_mode` answer and `review_mode_changed` event carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct ReviewMode {
    pub effective: ReviewEffective,
    pub source: ReviewSource,
    pub scopes: ReviewScopes,
}

impl ReviewMode {
    pub(super) fn from_data(data: &Value) -> Result<Self, DesktopRpcError> {
        decode_data(data)
    }
}

/// The `set_review_mode` params: exactly `{ enabled, scope }`, plus
/// `confirmationToken` when resending an approved confirmation.
pub(super) fn set_review_mode_params(
    enabled: bool,
    scope: ReviewScope,
    confirmation_token: Option<&str>,
) -> Result<Value, DesktopRpcError> {
    Ok(match confirmation_token {
        None => json!({ "enabled": enabled, "scope": scope.as_str() }),
        Some(token) => {
            if !(1..=MAX_PROFILE_APPLY_UNITS).contains(&token.encode_utf16().count()) {
                return Err(DesktopRpcError::InvalidConfirmationToken);
            }
            json!({ "enabled": enabled, "scope": scope.as_str(), "confirmationToken": token })
        }
    })
}

/// The dialog Desktop must show before a change. The token is bound to this
/// exact text and to the state it was computed from; a stale token is
/// answered with a fresh confirmation, and nothing is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewConfirmation {
    pub title: String,
    pub message: String,
    pub token: String,
}

/// How a `set_review_mode` request ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetReviewModeOutcome {
    /// Nothing was written: show this dialog, and resend with its token only
    /// when the human approves.
    ConfirmationRequired(ReviewConfirmation),
    /// The setting was written and re-read. `changed` is whether the re-read
    /// state differs from the one the confirmation was computed from; a
    /// `review_mode_changed` event follows only when it is `true`.
    Applied { changed: bool },
}

/// The decoded `set_review_mode` data: the request it answers, the current
/// (or re-read) mode, and the outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetReviewModeResult {
    pub enabled: bool,
    pub scope: ReviewScope,
    pub mode: ReviewMode,
    pub outcome: SetReviewModeOutcome,
}

impl SetReviewModeResult {
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
                SetReviewModeOutcome::ConfirmationRequired(ReviewConfirmation {
                    title: string_field(confirmation, "title")?,
                    message: string_field(confirmation, "message")?,
                    token,
                })
            }
            "applied" => SetReviewModeOutcome::Applied {
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
            enabled: bool_field(data, "enabled")?,
            scope: closed_field(data, "scope")?,
            mode: ReviewMode::from_data(field_object(data, "mode")?)?,
            outcome,
        })
    }
}

/// The data of a `review_mode_changed` event: the mode a confirmed
/// `set_review_mode` produced (identical to its response's `mode`) and the
/// mode its confirmation was computed from. A change made in the TUI, Herdr or
/// another process emits nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewModeChanged {
    pub mode: ReviewMode,
    pub previous_mode: ReviewMode,
}

impl ReviewModeChanged {
    pub(super) fn from_data(data: &Value) -> Result<Self, DesktopRpcError> {
        decode_data(data)
    }
}
