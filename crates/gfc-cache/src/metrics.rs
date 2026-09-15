//! Local-only success metrics. Never uploaded.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalMetrics {
    pub opened_at: DateTime<Utc>,
    pub triage_complete_ms: Option<u64>,
    pub attention_count: usize,
    pub repo_count: usize,
    pub oldest_dirty_or_diverged_secs: Option<u64>,
    pub ci_fail_to_open_ms: Option<u64>,
    pub current_local_remote_pct: f64,
}

impl LocalMetrics {
    pub fn snapshot(
        opened_at: DateTime<Utc>,
        attention_count: usize,
        repo_count: usize,
        oldest_dirty_or_diverged_secs: Option<u64>,
        current_local_remote_pct: f64,
    ) -> Self {
        Self {
            opened_at,
            triage_complete_ms: None,
            attention_count,
            repo_count,
            oldest_dirty_or_diverged_secs,
            ci_fail_to_open_ms: None,
            current_local_remote_pct,
        }
    }
}
