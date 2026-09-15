use gfc_schema::{CompactLocal, CompactRemote, HealthThresholds, RepositoryHealth};

use crate::filter::{compact_local_label, compact_remote_label, freshness_label};

/// Compact selected-repo header: three lines, no timestamp dump.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthBar {
    pub name: String,
    pub path: String,
    pub attention: bool,
    pub branch: String,
    pub local: CompactLocal,
    pub ahead: u32,
    pub behind: u32,
    pub remote: CompactRemote,
    pub freshness: String,
    pub provider: String,
}

impl HealthBar {
    pub fn from_repo(repo: &RepositoryHealth, thresholds: &HealthThresholds) -> Self {
        let div = &repo.local.divergence.evidence;
        Self {
            name: repo.identity.name.clone(),
            path: repo.identity.path.clone(),
            attention: repo.needs_attention(thresholds),
            branch: div.branch.clone(),
            local: repo.compact_local(),
            ahead: div.ahead,
            behind: div.behind,
            remote: repo.compact_remote(),
            freshness: freshness_label(repo),
            provider: repo.identity.provider.as_str().to_string(),
        }
    }

    pub fn attention_label(&self) -> &'static str {
        if self.attention {
            "needs attention"
        } else {
            "ok"
        }
    }

    /// Three plain lines so the header cannot grow back into an evidence dump.
    pub fn plain_lines(&self) -> [String; 3] {
        [
            format!("{}  {}  {}", self.name, self.attention_label(), self.path),
            format!(
                "{}  {}  ahead {}  behind {}",
                self.branch,
                compact_local_label(self.local),
                self.ahead,
                self.behind
            ),
            format!(
                "{}  {}  {}",
                compact_remote_label(self.remote),
                self.freshness,
                self.provider
            ),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use gfc_schema::*;

    fn repo(name: &str, dirty: bool) -> RepositoryHealth {
        let now = Utc::now();
        let mut r = RepositoryHealth {
            schema_version: SCHEMA_VERSION.into(),
            identity: Identity {
                id: name.into(),
                name: name.into(),
                path: format!("/tmp/{name}"),
                provider: ProviderKind::Git,
                remote_url: None,
                group: Some("tmp".into()),
                tags: vec![],
                workspaces: vec![],
            },
            local: LocalHealth {
                worktree: WorktreeSignal {
                    state: if dirty {
                        WorktreeState::Dirty
                    } else {
                        WorktreeState::Clean
                    },
                    evidence: WorktreeEvidence {
                        modified: u32::from(dirty),
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
                        upstream: None,
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
                    unfinished_work: dirty,
                    last_activity_at: Some(now),
                    reasons: vec![],
                },
                updated_at: now,
            },
            freshness: present_freshness(now, None, &HealthThresholds::default(), now),
            error: None,
        };
        r.recompute_staleness(&HealthThresholds::default(), now);
        r
    }

    fn joined(bar: &HealthBar) -> String {
        bar.plain_lines().join("\n")
    }

    fn looks_like_dump(text: &str) -> bool {
        text.contains("worktree")
            || text.contains("updated_at")
            || text.contains("last_fetch")
            || (text.contains('T') && text.contains('Z'))
    }

    #[test]
    fn dirty_repo_needs_attention_not_a_dump() {
        let t = HealthThresholds::default();
        let bar = HealthBar::from_repo(&repo("alpha", true), &t);
        assert!(bar.attention);
        let text = joined(&bar);
        assert_eq!(bar.plain_lines().len(), 3);
        assert!(text.contains("alpha"));
        assert!(text.contains("needs attention"));
        assert!(text.contains("dirty"));
        assert!(text.contains("/tmp/alpha"));
        assert!(!looks_like_dump(&text));
    }

    #[test]
    fn clean_repo_with_missing_remote_is_ok() {
        let t = HealthThresholds::default();
        let bar = HealthBar::from_repo(&repo("zeta", false), &t);
        assert!(!bar.attention);
        let text = joined(&bar);
        assert!(text.contains("ok"));
        assert!(text.contains("clean"));
        assert!(text.contains("ci:?"));
        assert!(text.contains("git"));
        assert!(!text.contains("https://"));
        assert!(!looks_like_dump(&text));
    }
}
