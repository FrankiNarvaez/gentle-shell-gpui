//! The session state `get_snapshot { sections }` and the `session_snapshot`
//! event carry: the sections of the feed owner's live projection
//! (`DesktopFeedSnapshot` in Gentle Shell's `lib/desktop-feed.ts`), made
//! path-free by the server. Each section keeps the feed's own shape and caps.
//!
//! A section the feed reports as `null` (no owner, no worktree, no pending
//! question) is `Some(None)`; a section the answer does not carry (not
//! requested, or unchanged since the last event) is `None`. Free text
//! (titles, labels, questions) is the person's or the agent's own wording and
//! is kept as written.

use super::{DesktopRpcError, decode_data};
use serde::Deserialize;
use serde_json::Value;

wire_enum! {
    /// The section names `get_snapshot` accepts, in canonical wire order:
    /// the server answers them in this order whatever order was requested.
    SessionSection, "sections" {
        Project => "project",
        Model => "model",
        Context => "context",
        Cost => "cost",
        Usage => "usage",
        Profiles => "profiles",
        Permission => "permission",
        Todo => "todo",
        Changes => "changes",
        Agents => "agents",
        Jobs => "jobs",
        Review => "review",
        Stats => "stats",
        Activity => "activity",
        PendingQuestion => "pendingQuestion",
        Commands => "commands",
    }
}

wire_enum! {
    /// What the session is doing: `blocked` (a question or consent waits) beats
    /// `working` (a turn runs), which beats `waiting` (only background work
    /// runs), which beats `idle`.
    ActivityState, "activity.state" {
        Working => "working",
        Waiting => "waiting",
        Blocked => "blocked",
        Idle => "idle",
    }
}

wire_enum! {
    /// What raised a pending question: an ask tool's single choice or
    /// questionnaire, an extension dialog, or a consent blocker.
    PendingQuestionKind, "pendingQuestion.kind" {
        Choice => "choice",
        Questionnaire => "questionnaire",
        Prompt => "prompt",
        Consent => "consent",
    }
}

wire_enum! {
    /// Where a slash command comes from.
    CommandSource, "commands.source" {
        Builtin => "builtin",
        Extension => "extension",
        Prompt => "prompt",
        Skill => "skill",
    }
}

wire_enum! {
    /// An orchestrator service tier, as the status bar shows it.
    ServiceTier, "tier" {
        Standard => "standard",
        Fast => "fast",
    }
}

/// A nullable section that may also be absent: absent stays `None`, a
/// present `null` becomes `Some(None)`.
fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// The sections of one answer or event. Only the sections it carries are
/// `Some`.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSections {
    pub project: Option<SessionProject>,
    pub model: Option<SessionModel>,
    #[serde(default, deserialize_with = "present")]
    pub context: Option<Option<SessionContext>>,
    pub cost: Option<SessionCost>,
    #[serde(default, deserialize_with = "present")]
    pub usage: Option<Option<SessionUsage>>,
    #[serde(default, deserialize_with = "present")]
    pub profiles: Option<Option<SessionProfiles>>,
    #[serde(default, deserialize_with = "present")]
    pub permission: Option<Option<SessionPermission>>,
    pub todo: Option<SessionTodo>,
    #[serde(default, deserialize_with = "present")]
    pub changes: Option<Option<SessionChanges>>,
    #[serde(default, deserialize_with = "present")]
    pub agents: Option<Option<SessionAgents>>,
    #[serde(default, deserialize_with = "present")]
    pub jobs: Option<Option<SessionJobs>>,
    #[serde(default, deserialize_with = "present")]
    pub review: Option<Option<SessionReview>>,
    pub stats: Option<SessionStats>,
    pub activity: Option<SessionActivity>,
    #[serde(default, deserialize_with = "present")]
    pub pending_question: Option<Option<SessionPendingQuestion>>,
    pub commands: Option<SessionCommands>,
}

impl SessionSections {
    pub(super) fn from_data(data: &Value) -> Result<Self, DesktopRpcError> {
        if !data.is_object() {
            return Err(DesktopRpcError::MalformedPayload(
                "session sections must be an object".into(),
            ));
        }
        decode_data(data)
    }
}

/// The working directory's basename (`null` for a filesystem or drive root),
/// its branch, and the dirty file count. Never the path.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SessionProject {
    pub name: Option<String>,
    pub branch: Option<String>,
    pub dirty: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SessionModel {
    pub id: Option<String>,
    pub provider: Option<String>,
    pub effort: Option<String>,
    pub profile: SessionModelProfile,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SessionModelProfile {
    pub name: Option<String>,
    pub tier: Option<ServiceTier>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SessionContext {
    pub percent: Option<f64>,
    pub tokens: Option<u64>,
    pub window: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SessionCost {
    pub total: f64,
    pub subscription: bool,
}

/// The usage panel: the session's own provider at the top level, and every
/// provider the panel lists in `providers`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SessionUsage {
    #[serde(flatten)]
    pub current: SessionUsageEntry,
    pub providers: Vec<SessionUsageEntry>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionUsageEntry {
    pub provider: String,
    pub status: String,
    pub plan: Option<String>,
    pub fetched_at: Option<f64>,
    /// What the panel says for a provider with no windows.
    pub note: Option<String>,
    pub limits: Vec<SessionUsageLimit>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionUsageLimit {
    pub name: String,
    pub limit_reached: bool,
    pub windows: Vec<SessionUsageWindow>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionUsageWindow {
    pub label: String,
    pub used_percent: f64,
    pub window_seconds: f64,
    pub reset_at: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SessionProfiles {
    /// The profile this session routes with.
    pub active: Option<String>,
    /// The store's default for new sessions.
    pub default: Option<String>,
    pub items: Vec<SessionProfileItem>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SessionProfileItem {
    pub name: String,
    pub tier: Option<ServiceTier>,
    pub mode: String,
    pub orchestrator: Option<SessionOrchestratorRoute>,
    /// One row per agent role, alphabetical.
    pub roles: Vec<SessionRoleRoute>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SessionOrchestratorRoute {
    pub model: String,
    pub thinking: String,
    pub tier: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SessionRoleRoute {
    pub agent: String,
    pub model: String,
    pub thinking: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPermission {
    pub mode: String,
    pub default_mode: Option<String>,
    pub yolo: bool,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SessionTodo {
    pub items: Vec<SessionTodoItem>,
    pub progress: SessionTodoProgress,
    pub current: Option<String>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SessionTodoItem {
    pub id: u64,
    pub title: String,
    pub state: String,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct SessionTodoProgress {
    pub done: u64,
    pub total: u64,
    pub open: u64,
}

/// Uncommitted changes, path-free: every worktree root is an opaque
/// `root-<12 hex>` label, and every file path is relative to its worktree.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SessionChanges {
    pub files: Vec<SessionChangedFile>,
    pub summary: SessionChangesSummary,
    pub worktrees: Vec<SessionWorktree>,
    pub truncated: bool,
}

/// One changed file. `root` is `None` for a path no worktree holds (then
/// `path` is the bare file name). `added` / `deleted` are `None` for an
/// untracked file whose lines were not counted (binary, too large).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SessionChangedFile {
    pub path: String,
    pub root: Option<String>,
    pub status: String,
    pub added: Option<u64>,
    pub deleted: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct SessionChangesSummary {
    pub files: u64,
    pub added: u64,
    pub deleted: u64,
}

/// A worktree with changes; `root` is `None` for `/` or a bare drive.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SessionWorktree {
    pub root: Option<String>,
    pub branch: Option<String>,
    pub files: u64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SessionAgents {
    pub items: Vec<SessionAgentItem>,
    pub active: u64,
    pub spend: SessionAgentRuns,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SessionAgentItem {
    pub id: String,
    pub name: String,
    pub role: Option<String>,
    pub state: String,
    pub step: String,
    pub model: String,
    pub thinking: Option<String>,
    pub spend: SessionSpend,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct SessionSpend {
    pub tokens: u64,
    pub cost: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct SessionAgentRuns {
    pub runs: u64,
    pub tokens: u64,
    pub cost: f64,
}

/// Background jobs. `elapsed_ms` moves on every flush and is not a change
/// for `session_snapshot`; derive it from `started_at`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SessionJobs {
    pub running: Vec<SessionRunningJob>,
    pub recent: Vec<SessionRecentJob>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRunningJob {
    pub id: String,
    pub label: String,
    pub started_at: u64,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRecentJob {
    pub id: String,
    pub label: String,
    pub status: String,
    pub started_at: u64,
    pub ended_at: Option<u64>,
    pub exit_code: Option<i64>,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SessionReview {
    pub state: String,
    pub label: String,
    pub scope: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStats {
    pub turns: u64,
    pub tool_calls: u64,
    pub tool_errors: u64,
    pub cost: f64,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub cache_hit_rate: Option<f64>,
    pub duration_ms: u64,
    pub models: Vec<SessionStatsModel>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SessionStatsModel {
    pub model: String,
    pub turns: u64,
    pub tokens: u64,
    pub cost: f64,
}

/// The activity every desktop region shows: its state, the turn phase, the
/// live background work, and the words for it (`label`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionActivity {
    pub state: ActivityState,
    pub streaming: bool,
    pub phase: Option<String>,
    pub phase_label: Option<String>,
    pub blocked_label: Option<String>,
    pub agents: u64,
    pub jobs: u64,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SessionQuestionOption {
    pub label: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SessionQuestion {
    pub question: String,
    pub header: Option<String>,
    pub options: Vec<SessionQuestionOption>,
    pub multi: bool,
    pub custom: bool,
}

/// The question the session waits on. The first (or only) question is
/// flattened at the top level; `questions` has every one. `terminal_only`
/// questions can only be answered in the terminal.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPendingQuestion {
    pub kind: PendingQuestionKind,
    pub tool: Option<String>,
    pub tool_call_id: Option<String>,
    pub question: String,
    pub header: Option<String>,
    pub options: Vec<SessionQuestionOption>,
    pub multi: bool,
    pub custom: bool,
    pub questions: Vec<SessionQuestion>,
    /// Pi's dialog kind for a `prompt` (select, confirm, input, editor, custom).
    pub prompt_kind: Option<String>,
    pub terminal_only: bool,
    pub opened_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SessionCommands {
    pub items: Vec<SessionCommand>,
    pub truncated: bool,
}

/// One slash command. `terminal` commands show their result in the TUI.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SessionCommand {
    pub name: String,
    pub description: Option<String>,
    pub args: Option<String>,
    pub terminal: bool,
    pub source: CommandSource,
}

/// The data of a `session_snapshot` event: only the sections whose content
/// changed since the previous one.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionSnapshotEvent {
    pub sections: SessionSections,
}

impl SessionSnapshotEvent {
    pub(super) fn from_data(data: &Value) -> Result<Self, DesktopRpcError> {
        let sections = data.get("sections").ok_or_else(|| {
            DesktopRpcError::MalformedPayload("session_snapshot has no sections".into())
        })?;
        Ok(Self {
            sections: SessionSections::from_data(sections)?,
        })
    }
}
