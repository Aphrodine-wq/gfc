//! Normalized repository-health schema (v1).
//!
//! This crate is the source of truth for inventory records. The TUI compact
//! columns are derived views; every signal carries structured evidence and a
//! last-updated timestamp.

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: &str = "1.0.0";
pub const SCHEMA_ID: &str = "https://gfc.dev/schema/repository-health.v1.json";

/// Thresholds used to evaluate staleness and attention. Owned by config;
/// passed into schema evaluation so the core stays deterministic.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct HealthThresholds {
    /// Dirty worktrees count as needing attention.
    pub dirty_needs_attention: bool,
    /// Fetch older than this many seconds is stale.
    pub fetch_stale_after_secs: u64,
    /// Unfinished dirty/diverged work older than this is stale.
    pub unfinished_work_stale_after_secs: u64,
    /// Local scan younger than this may be presented as current.
    pub local_current_within_secs: u64,
    /// Remote/CI data younger than this may be presented as current.
    pub remote_current_within_secs: u64,
}

impl Default for HealthThresholds {
    fn default() -> Self {
        Self {
            dirty_needs_attention: true,
            fetch_stale_after_secs: 24 * 60 * 60,
            unfinished_work_stale_after_secs: 72 * 60 * 60,
            local_current_within_secs: 30,
            remote_current_within_secs: 300,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Github,
    Gitlab,
    Origin,
    Git,
}

impl ProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Github => "github",
            Self::Gitlab => "gitlab",
            Self::Origin => "origin",
            Self::Git => "git",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Identity {
    pub id: String,
    pub name: String,
    pub path: String,
    pub provider: ProviderKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub workspaces: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeState {
    Clean,
    Dirty,
    Conflicted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WorktreeEvidence {
    pub modified: u32,
    pub staged: u32,
    pub untracked: u32,
    #[serde(default)]
    pub conflicted_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WorktreeSignal {
    pub state: WorktreeState,
    pub evidence: WorktreeEvidence,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DivergenceState {
    Synced,
    Ahead,
    Behind,
    Diverged,
    NoUpstream,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DivergenceEvidence {
    pub branch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DivergenceSignal {
    pub state: DivergenceState,
    pub evidence: DivergenceEvidence,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FetchAgeSignal {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_fetch_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub age_secs: Option<u64>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LocalHealth {
    pub worktree: WorktreeSignal,
    pub divergence: DivergenceSignal,
    pub fetch_age: FetchAgeSignal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    Ok,
    Unavailable,
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConnectionEvidence {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConnectionSignal {
    pub state: ConnectionState,
    pub evidence: ConnectionEvidence,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CiState {
    Passing,
    Failing,
    Pending,
    Unknown,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CiEvidence {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conclusion: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CiSignal {
    pub state: CiState,
    pub evidence: CiEvidence,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RemoteHealth {
    pub connection: ConnectionSignal,
    pub ci: CiSignal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StalenessState {
    Fresh,
    Stale,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StalenessEvidence {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fetch_age_secs: Option<u64>,
    pub unfinished_work: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activity_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StalenessSignal {
    pub state: StalenessState,
    pub evidence: StalenessEvidence,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FreshnessPresentedAs {
    Current,
    Cached,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Freshness {
    pub local_updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_updated_at: Option<DateTime<Utc>>,
    pub presented_as: FreshnessPresentedAs,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub age_secs: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ScanError {
    pub path: String,
    pub message: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RepositoryHealth {
    pub schema_version: String,
    pub identity: Identity,
    pub local: LocalHealth,
    pub remote: RemoteHealth,
    pub staleness: StalenessSignal,
    pub freshness: Freshness,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ScanError>,
}

impl RepositoryHealth {
    pub fn needs_attention(&self, thresholds: &HealthThresholds) -> bool {
        if self.error.is_some() {
            return true;
        }
        if matches!(self.local.worktree.state, WorktreeState::Conflicted) {
            return true;
        }
        if thresholds.dirty_needs_attention
            && matches!(self.local.worktree.state, WorktreeState::Dirty)
        {
            return true;
        }
        match self.local.divergence.state {
            DivergenceState::Ahead | DivergenceState::Behind | DivergenceState::Diverged => {
                return true;
            }
            DivergenceState::Synced | DivergenceState::NoUpstream => {}
        }
        match self.remote.ci.state {
            CiState::Failing | CiState::Pending => return true,
            CiState::Passing | CiState::Unknown | CiState::Unsupported => {}
        }
        match self.remote.connection.state {
            ConnectionState::Unavailable | ConnectionState::Invalid => return true,
            ConnectionState::Ok => {}
        }
        matches!(self.staleness.state, StalenessState::Stale)
    }

    pub fn compact_local(&self) -> CompactLocal {
        match self.local.worktree.state {
            WorktreeState::Conflicted => CompactLocal::Conflicted,
            WorktreeState::Dirty => CompactLocal::Dirty,
            WorktreeState::Clean => match self.local.divergence.state {
                DivergenceState::Diverged => CompactLocal::Diverged,
                DivergenceState::Ahead => CompactLocal::Ahead,
                DivergenceState::Behind => CompactLocal::Behind,
                DivergenceState::Synced | DivergenceState::NoUpstream => CompactLocal::Clean,
            },
        }
    }

    pub fn compact_remote(&self) -> CompactRemote {
        match self.remote.connection.state {
            ConnectionState::Invalid => CompactRemote::Invalid,
            ConnectionState::Unavailable => CompactRemote::Unavailable,
            ConnectionState::Ok => match self.remote.ci.state {
                CiState::Failing => CompactRemote::CiFailing,
                CiState::Pending => CompactRemote::CiPending,
                CiState::Passing => CompactRemote::CiPassing,
                CiState::Unknown => CompactRemote::Unknown,
                CiState::Unsupported => CompactRemote::Unsupported,
            },
        }
    }

    pub fn recompute_staleness(&mut self, thresholds: &HealthThresholds, now: DateTime<Utc>) {
        let mut reasons = Vec::new();
        let unfinished = !matches!(self.local.worktree.state, WorktreeState::Clean)
            || matches!(
                self.local.divergence.state,
                DivergenceState::Ahead | DivergenceState::Behind | DivergenceState::Diverged
            );
        if let Some(age) = self.local.fetch_age.age_secs
            && age > thresholds.fetch_stale_after_secs
        {
            reasons.push(format!("fetch age {age}s exceeds threshold"));
        }
        if unfinished
            && let Some(last) = self.staleness.evidence.last_activity_at
        {
            let age = (now - last).num_seconds().max(0) as u64;
            if age > thresholds.unfinished_work_stale_after_secs {
                reasons.push(format!("unfinished work idle for {age}s"));
            }
        }
        self.staleness.evidence.fetch_age_secs = self.local.fetch_age.age_secs;
        self.staleness.evidence.unfinished_work = unfinished;
        self.staleness.evidence.reasons = reasons.clone();
        self.staleness.state = if reasons.is_empty() {
            StalenessState::Fresh
        } else {
            StalenessState::Stale
        };
        self.staleness.updated_at = now;
        let remote_ts = match self.remote.ci.state {
            CiState::Unknown | CiState::Unsupported => {
                match self.remote.connection.state {
                    ConnectionState::Ok => None,
                    ConnectionState::Unavailable | ConnectionState::Invalid => {
                        Some(self.remote.connection.updated_at)
                    }
                }
            }
            CiState::Passing | CiState::Failing | CiState::Pending => {
                Some(self.remote.ci.updated_at)
            }
        };
        self.freshness = present_freshness(
            self.local.worktree.updated_at,
            remote_ts,
            thresholds,
            now,
        );
    }
}

/// Present freshness strictly from timestamps vs thresholds. Stale cache
/// entries are never presented as current.
pub fn present_freshness(
    local_updated_at: DateTime<Utc>,
    remote_updated_at: Option<DateTime<Utc>>,
    thresholds: &HealthThresholds,
    now: DateTime<Utc>,
) -> Freshness {
    let local_age = (now - local_updated_at).num_seconds().max(0) as u64;
    let remote_age = remote_updated_at
        .map(|ts| (now - ts).num_seconds().max(0) as u64);
    let local_current = local_age <= thresholds.local_current_within_secs;
    let remote_current = match remote_age {
        Some(age) => age <= thresholds.remote_current_within_secs,
        None => true,
    };
    Freshness {
        local_updated_at,
        remote_updated_at,
        presented_as: if local_current && remote_current {
            FreshnessPresentedAs::Current
        } else {
            FreshnessPresentedAs::Cached
        },
        age_secs: Some(local_age.max(remote_age.unwrap_or(0))),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CompactLocal {
    Clean,
    Dirty,
    Conflicted,
    Ahead,
    Behind,
    Diverged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CompactRemote {
    CiPassing,
    CiFailing,
    CiPending,
    Unknown,
    Unsupported,
    Unavailable,
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Inventory {
    pub schema_version: String,
    pub generated_at: DateTime<Utc>,
    pub repositories: Vec<RepositoryHealth>,
    #[serde(default)]
    pub errors: Vec<ScanError>,
}

impl Inventory {
    pub fn new(repositories: Vec<RepositoryHealth>, errors: Vec<ScanError>, now: DateTime<Utc>) -> Self {
        Self {
            schema_version: SCHEMA_VERSION.to_string(),
            generated_at: now,
            repositories,
            errors,
        }
    }

    pub fn attention_count(&self, thresholds: &HealthThresholds) -> usize {
        self.repositories
            .iter()
            .filter(|r| r.needs_attention(thresholds))
            .count()
    }
}

/// Placeholder remote health used until a provider refresh completes.
pub fn unknown_remote(now: DateTime<Utc>) -> RemoteHealth {
    RemoteHealth {
        connection: ConnectionSignal {
            state: ConnectionState::Ok,
            evidence: ConnectionEvidence {
                message: Some("remote not refreshed".into()),
                http_status: None,
            },
            updated_at: now,
        },
        ci: CiSignal {
            state: CiState::Unknown,
            evidence: CiEvidence {
                provider: None,
                check_name: None,
                sha: None,
                url: None,
                conclusion: None,
            },
            updated_at: now,
        },
    }
}

pub fn json_schema() -> schemars::schema::RootSchema {
    schemars::schema_for!(Inventory)
}

#[cfg(test)]
mod eval_tests {
    use super::*;

    fn sample(now: DateTime<Utc>) -> RepositoryHealth {
        RepositoryHealth {
            schema_version: SCHEMA_VERSION.into(),
            identity: Identity {
                id: "abc".into(),
                name: "demo".into(),
                path: "/tmp/demo".into(),
                provider: ProviderKind::Git,
                remote_url: None,
                group: Some("tmp".into()),
                tags: vec![],
                workspaces: vec![],
            },
            local: LocalHealth {
                worktree: WorktreeSignal {
                    state: WorktreeState::Clean,
                    evidence: WorktreeEvidence {
                        modified: 0,
                        staged: 0,
                        untracked: 0,
                        conflicted_paths: vec![],
                    },
                    updated_at: now,
                },
                divergence: DivergenceSignal {
                    state: DivergenceState::Synced,
                    evidence: DivergenceEvidence {
                        branch: "main".into(),
                        upstream: Some("origin/main".into()),
                        ahead: 0,
                        behind: 0,
                    },
                    updated_at: now,
                },
                fetch_age: FetchAgeSignal {
                    last_fetch_at: Some(now),
                    age_secs: Some(0),
                    updated_at: now,
                },
            },
            remote: unknown_remote(now),
            staleness: StalenessSignal {
                state: StalenessState::Fresh,
                evidence: StalenessEvidence {
                    fetch_age_secs: Some(0),
                    unfinished_work: false,
                    last_activity_at: Some(now),
                    reasons: vec![],
                },
                updated_at: now,
            },
            freshness: present_freshness(now, Some(now), &HealthThresholds::default(), now),
            error: None,
        }
    }

    #[test]
    fn clean_synced_does_not_need_attention() {
        let now = Utc::now();
        let repo = sample(now);
        assert!(!repo.needs_attention(&HealthThresholds::default()));
    }

    #[test]
    fn conflicted_needs_attention() {
        let now = Utc::now();
        let mut repo = sample(now);
        repo.local.worktree.state = WorktreeState::Conflicted;
        assert!(repo.needs_attention(&HealthThresholds::default()));
    }

    #[test]
    fn stale_cache_never_presented_as_current() {
        let now = Utc::now();
        let old = now - chrono::Duration::seconds(10_000);
        let freshness = present_freshness(old, Some(old), &HealthThresholds::default(), now);
        assert_eq!(freshness.presented_as, FreshnessPresentedAs::Cached);
    }
}
