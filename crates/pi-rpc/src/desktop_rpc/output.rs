//! `get_job_output` and `get_subagent_thread`: the bounded, pollable reads of
//! what a background job printed and what a subagent did. Both are the
//! owner's own data — bounded, never redacted — so a consumer shows them to
//! the session owner like terminal output and never logs or forwards them.

use super::{
    DesktopJob, DesktopMethod, DesktopRpcError, bool_field, decode_data, field_object, string_field,
};
use serde::Deserialize;
use serde_json::{Value, json};

/// The `get_subagent_thread` params: exactly `{ id }`, plus `sinceVersion`
/// when given.
pub(super) fn subagent_thread_params(
    id: &str,
    since_version: Option<u64>,
) -> Result<Value, DesktopRpcError> {
    if id.is_empty() {
        return Err(DesktopRpcError::InvalidId(DesktopMethod::GetSubagentThread));
    }
    Ok(match since_version {
        None => json!({ "id": id }),
        Some(version) => json!({ "id": id, "sinceVersion": version }),
    })
}

/// The decoded `get_job_output` data: the job's list projection (so a poller
/// sees it end), its tail as plain text (at most 20 lines, terminal control
/// sequences already stripped; empty when nothing was printed yet), and
/// whether earlier lines were dropped or a long line was cut.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobOutput {
    pub job: DesktopJob,
    pub output: String,
    pub truncated: bool,
}

impl JobOutput {
    pub(super) fn from_data(data: &Value) -> Result<Self, DesktopRpcError> {
        Ok(Self {
            job: DesktopJob::from_data(field_object(data, "job")?)?,
            output: string_field(data, "output")?,
            truncated: bool_field(data, "truncated")?,
        })
    }
}

/// One thread item, oldest first. Text fields are cut at 2 000 characters and
/// tool `args` (JSON-encoded) and `output` at 500; a cut field ends with `…`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ThreadItem {
    Text {
        text: String,
    },
    Thinking {
        text: String,
    },
    /// A notice such as `retrying (2/3)`; it may carry an error message.
    Note {
        text: String,
    },
    Tool {
        name: String,
        args: String,
        running: bool,
        #[serde(rename = "isError")]
        is_error: bool,
        output: String,
    },
}

/// The decoded `get_subagent_thread` data. `version` grows with every change
/// to the thread; resending it as `sinceVersion` answers no items when nothing
/// changed. Otherwise `items` is the whole bounded window (the last 40), and
/// the client replaces its view with it. `dropped` counts items the store
/// itself evicted; `truncated` means the store still holds more than `items`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SubagentThread {
    pub id: String,
    pub version: u64,
    pub items: Vec<ThreadItem>,
    pub dropped: u64,
    pub truncated: bool,
}

impl SubagentThread {
    pub(super) fn from_data(data: &Value) -> Result<Self, DesktopRpcError> {
        decode_data(data)
    }
}
