//! `list_sessions` and `switch_session`: Pi's own saved sessions, by id. No
//! session file path, working directory or full message text crosses the
//! wire, in either direction.

use super::{DesktopRpcError, decode_data};
use serde::Deserialize;
use serde_json::{Map, Value, json};

/// Longest `switch_session` id, in UTF-16 code units.
pub const MAX_SESSION_ID_UNITS: usize = 256;
/// Largest `list_sessions` page.
pub const MAX_SESSIONS_LIMIT: u32 = 50;

wire_enum! {
    /// Which sessions `list_sessions` lists, and where `switch_session`
    /// resolves an id: the active project's (the server default) or every
    /// project's.
    SessionsScope, "scope" {
        Project => "project",
        All => "all",
    }
}

/// The optional `list_sessions` params. `None` fields are omitted, so the
/// server defaults (`project`, 20 entries, first page) apply.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListSessionsQuery {
    pub scope: Option<SessionsScope>,
    /// Page size, 1 to [`MAX_SESSIONS_LIMIT`].
    pub limit: Option<u32>,
    /// The opaque `next_cursor` of the previous page.
    pub cursor: Option<String>,
}

impl ListSessionsQuery {
    pub(super) fn params(&self) -> Result<Value, DesktopRpcError> {
        let mut params = Map::new();
        if let Some(scope) = self.scope {
            params.insert("scope".into(), json!(scope.as_str()));
        }
        if let Some(limit) = self.limit {
            if !(1..=MAX_SESSIONS_LIMIT).contains(&limit) {
                return Err(DesktopRpcError::InvalidSessionsLimit(limit));
            }
            params.insert("limit".into(), json!(limit));
        }
        if let Some(cursor) = &self.cursor {
            if cursor.is_empty() {
                return Err(DesktopRpcError::InvalidSessionsCursor);
            }
            params.insert("cursor".into(), json!(cursor));
        }
        Ok(Value::Object(params))
    }
}

/// The `switch_session` params: exactly `{ id }`, plus `scope` when given.
pub(super) fn switch_session_params(
    id: &str,
    scope: Option<SessionsScope>,
) -> Result<Value, DesktopRpcError> {
    if !(1..=MAX_SESSION_ID_UNITS).contains(&id.encode_utf16().count()) {
        return Err(DesktopRpcError::InvalidId(
            super::DesktopMethod::SwitchSession,
        ));
    }
    Ok(match scope {
        None => json!({ "id": id }),
        Some(scope) => json!({ "id": id, "scope": scope.as_str() }),
    })
}

/// One saved session. `name` is present only when the user named it;
/// `message_count` only when Pi counted one. `first_message` is collapsed and
/// at most 200 characters (Pi says `(no messages)` for an empty session);
/// `project` is the working directory's last segment or `unknown`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionEntry {
    pub id: String,
    pub name: Option<String>,
    /// Last activity, in milliseconds.
    pub modified: u64,
    pub message_count: Option<u64>,
    pub first_message: String,
    pub project: String,
    /// The session this Pi process runs.
    pub current: bool,
}

/// The decoded `list_sessions` data: one page, newest first, and the cursor
/// of the next page when more sessions follow.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionList {
    pub sessions: Vec<SessionEntry>,
    pub next_cursor: Option<String>,
}

impl SessionList {
    pub(super) fn from_data(data: &Value) -> Result<Self, DesktopRpcError> {
        decode_data(data)
    }
}

/// The decoded `switch_session` data. `changed: false` is a switch to the
/// current session (a no-op) or, with `cancelled`, a switch an extension
/// refused. After a change the session was replaced: its event sequence
/// restarts at 1 and the client re-reads `get_snapshot`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SwitchSessionResult {
    pub id: String,
    pub changed: bool,
    #[serde(default)]
    pub cancelled: bool,
}

impl SwitchSessionResult {
    pub(super) fn from_data(data: &Value) -> Result<Self, DesktopRpcError> {
        decode_data(data)
    }
}
