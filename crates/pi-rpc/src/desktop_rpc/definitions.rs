//! `list_agent_definitions`: the agent definitions this session can launch,
//! the `subagents.json` defaults and Pi's default orchestrator, read fresh
//! by the owners `subagent_list_agents` and `/gentle:profiles` use. No file
//! path, prompt body or problem text crosses; only counts of problems.

use super::{DesktopRpcError, decode_data};
use serde::Deserialize;
use serde_json::Value;

wire_enum! {
    /// Where a definition was found: the agent home or the project's `.pi`.
    AgentDefinitionScope, "scope" {
        Global => "global",
        Project => "project",
    }
}

wire_enum! {
    /// Pi's `settings.json` default orchestrator: named (`set`), a file
    /// without `defaultProvider`/`defaultModel` (`unset`), no file
    /// (`missing`), or a file that cannot be parsed (`invalid`).
    OrchestratorStatus, "defaultOrchestrator.status" {
        Set => "set",
        Unset => "unset",
        Missing => "missing",
        Invalid => "invalid",
    }
}

/// One discovered definition. `model`, `thinking` and `mode` are what it
/// declares (`None` when it inherits); routing can still override `model`
/// at launch. `tools` is the list after overlays, at most 64 names.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDefinition {
    pub name: String,
    pub description: Option<String>,
    pub scope: AgentDefinitionScope,
    pub model: Option<String>,
    pub thinking: Option<String>,
    pub mode: Option<String>,
    pub tools: Vec<String>,
    pub tools_truncated: bool,
}

/// Counts of definitions that failed to parse and of discovery notices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct AgentDefinitionProblems {
    pub invalid: u64,
    pub notices: u64,
}

/// `default_model`, `default_thinking` and `default_mode` from
/// `subagents.json` (project over global); `mode` defaults to `task`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SubagentDefaults {
    pub model: Option<String>,
    pub thinking: Option<String>,
    pub mode: String,
}

/// Pi's default orchestrator. `model` (`provider/model`) and `thinking`
/// (`defaultThinkingLevel`) are present only when `status` is `set`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DefaultOrchestrator {
    pub status: OrchestratorStatus,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub thinking: Option<String>,
}

impl DefaultOrchestrator {
    /// The orchestrator's model and thinking, only when Pi names one.
    pub fn route(&self) -> Option<(&str, Option<&str>)> {
        if self.status != OrchestratorStatus::Set {
            return None;
        }
        let model = self.model.as_deref().filter(|model| !model.is_empty())?;
        Some((model, self.thinking.as_deref()))
    }
}

/// The decoded `list_agent_definitions` data: definitions sorted by name, at
/// most 200 (`truncated` when there were more).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDefinitions {
    pub agents: Vec<AgentDefinition>,
    pub truncated: bool,
    pub problems: AgentDefinitionProblems,
    pub subagent_defaults: SubagentDefaults,
    pub default_orchestrator: DefaultOrchestrator,
}

impl AgentDefinitions {
    pub(super) fn from_data(data: &Value) -> Result<Self, DesktopRpcError> {
        decode_data(data)
    }
}
