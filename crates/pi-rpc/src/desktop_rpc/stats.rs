//! `get_usage_stats`: cross-session usage, the same history and aggregation
//! `/gentle:stats` shows. Counts, token sums, costs, model ids, day keys and
//! project display names only.

use super::{DesktopRpcError, decode_data};
use serde::Deserialize;
use serde_json::{Map, Value, json};

wire_enum! {
    /// The local calendar days counted: the last 7 (the server default) or
    /// 30 days including today, or everything.
    StatsRange, "range" {
        Week => "7d",
        Month => "30d",
        All => "all",
    }
}

wire_enum! {
    /// Which sessions count: those of this session's working directory (the
    /// server default) or every session.
    StatsScope, "scope" {
        Project => "project",
        All => "all",
    }
}

/// The `get_usage_stats` params: only the given keys, so the server defaults
/// apply to the rest.
pub(super) fn usage_stats_params(range: Option<StatsRange>, scope: Option<StatsScope>) -> Value {
    let mut params = Map::new();
    if let Some(range) = range {
        params.insert("range".into(), json!(range.as_str()));
    }
    if let Some(scope) = scope {
        params.insert("scope".into(), json!(scope.as_str()));
    }
    Value::Object(params)
}

/// Totals over the whole range, even when an array is truncated.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageStatsTotals {
    /// Sessions with at least one assistant usage record in range.
    pub sessions: u64,
    /// Assistant usage records in range.
    pub messages: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub total_tokens: u64,
    /// In the provider's currency units.
    pub cost: f64,
}

/// One model, largest `tokens` first; `share` is its fraction of
/// `totals.total_tokens`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct UsageStatsModel {
    pub model: String,
    pub messages: u64,
    pub tokens: u64,
    pub cost: f64,
    pub share: f64,
}

/// Tokens on one local calendar day (`YYYY-MM-DD`), oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct UsageStatsDay {
    pub day: String,
    pub tokens: u64,
}

/// One working directory, by display name (never the path), largest
/// `tokens` first. Two directories with the same name stay two rows.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct UsageStatsProject {
    pub name: String,
    pub sessions: u64,
    pub messages: u64,
    pub tokens: u64,
    pub cost: f64,
}

/// Which arrays had more rows than their bound (models 50, days 400,
/// projects 50). `by_project` is present only with `by_project`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageStatsTruncated {
    pub by_model: bool,
    pub by_day: bool,
    pub by_project: Option<bool>,
}

/// The decoded `get_usage_stats` data. `by_project` is present only for
/// `scope: all`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageStats {
    pub range: StatsRange,
    pub scope: StatsScope,
    pub totals: UsageStatsTotals,
    pub by_model: Vec<UsageStatsModel>,
    pub by_day: Vec<UsageStatsDay>,
    pub by_project: Option<Vec<UsageStatsProject>>,
    pub truncated: UsageStatsTruncated,
}

impl UsageStats {
    pub(super) fn from_data(data: &Value) -> Result<Self, DesktopRpcError> {
        let stats: Self = decode_data(data)?;
        if stats.by_project.is_some() != (stats.scope == StatsScope::All) {
            return Err(DesktopRpcError::MalformedPayload(
                "byProject must be present exactly for scope all".into(),
            ));
        }
        Ok(stats)
    }
}
