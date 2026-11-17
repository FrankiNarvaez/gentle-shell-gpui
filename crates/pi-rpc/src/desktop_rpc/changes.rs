//! `get_changes` and `get_change_diff`: the session's uncommitted changes
//! from the `/gentle:changes` owner, each against its `HEAD`. Roots cross
//! only as opaque, session-scoped ids plus a directory-name label.

use super::{DesktopRpcError, decode_data};
use serde::Deserialize;
use serde_json::Value;

wire_enum! {
    /// How Git reports a changed file.
    ChangeStatus, "status" {
        Modified => "modified",
        Added => "added",
        Deleted => "deleted",
        Renamed => "renamed",
        Untracked => "untracked",
    }
}

/// A root with changes: its opaque id (`root-1`, …) names it for
/// `get_change_diff`; `label` is its directory name, never a path. Two roots
/// may share a label.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ChangeRoot {
    pub id: String,
    pub label: String,
}

/// One changed file: its root id, its path relative to that root (as Git
/// reports it, `/` separators), and its line counts.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ChangedFile {
    pub root: String,
    pub path: String,
    pub status: ChangeStatus,
    pub added: u64,
    pub deleted: u64,
}

/// The decoded `get_changes` data. Only roots with changes are listed;
/// `truncated` means more than 200 files changed and `files` holds the first
/// 200.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Changes {
    pub roots: Vec<ChangeRoot>,
    pub files: Vec<ChangedFile>,
    pub truncated: bool,
}

impl Changes {
    pub(super) fn from_data(data: &Value) -> Result<Self, DesktopRpcError> {
        let changes: Self = decode_data(data)?;
        if let Some(file) = changes
            .files
            .iter()
            .find(|file| !changes.roots.iter().any(|root| root.id == file.root))
        {
            return Err(DesktopRpcError::MalformedPayload(format!(
                "changed file {} names an unlisted root {}",
                file.path, file.root
            )));
        }
        Ok(changes)
    }
}

/// The decoded `get_change_diff` data: one file's unified diff (empty when
/// Git reported no difference), capped at 256 KiB with `truncated`. The text
/// is file content and can contain anything the file does.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ChangeDiff {
    pub root: String,
    pub path: String,
    pub diff: String,
    pub truncated: bool,
}

impl ChangeDiff {
    pub(super) fn from_data(data: &Value) -> Result<Self, DesktopRpcError> {
        decode_data(data)
    }
}
