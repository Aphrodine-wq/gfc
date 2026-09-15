use gfc_schema::{
    CompactLocal, CompactRemote, FreshnessPresentedAs, HealthThresholds, Inventory,
    RepositoryHealth,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Attention,
    Name,
    Provider,
    Freshness,
}

impl SortKey {
    pub fn next(self) -> Self {
        match self {
            Self::Attention => Self::Name,
            Self::Name => Self::Provider,
            Self::Provider => Self::Freshness,
            Self::Freshness => Self::Attention,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Attention => "attention",
            Self::Name => "name",
            Self::Provider => "provider",
            Self::Freshness => "freshness",
        }
    }
}

pub fn visible_repos<'a>(
    inventory: &'a Inventory,
    thresholds: &HealthThresholds,
    filter: &str,
    attention_only: bool,
    sort: SortKey,
) -> Vec<&'a RepositoryHealth> {
    let q = filter.to_ascii_lowercase();
    let mut rows: Vec<&RepositoryHealth> = inventory
        .repositories
        .iter()
        .filter(|r| {
            if attention_only && !r.needs_attention(thresholds) {
                return false;
            }
            if q.is_empty() {
                return true;
            }
            r.identity.name.to_ascii_lowercase().contains(&q)
                || r.identity.path.to_ascii_lowercase().contains(&q)
                || r.identity.provider.as_str().contains(&q)
        })
        .collect();
    rows.sort_by(|a, b| match sort {
        SortKey::Attention => b
            .needs_attention(thresholds)
            .cmp(&a.needs_attention(thresholds))
            .then(a.identity.name.cmp(&b.identity.name)),
        SortKey::Name => a.identity.name.cmp(&b.identity.name),
        SortKey::Provider => a
            .identity
            .provider
            .as_str()
            .cmp(b.identity.provider.as_str())
            .then(a.identity.name.cmp(&b.identity.name)),
        SortKey::Freshness => a
            .freshness
            .age_secs
            .unwrap_or(0)
            .cmp(&b.freshness.age_secs.unwrap_or(0)),
    });
    rows
}

pub fn compact_local_label(v: CompactLocal) -> &'static str {
    match v {
        CompactLocal::Clean => "clean",
        CompactLocal::Dirty => "dirty",
        CompactLocal::Conflicted => "conflict",
        CompactLocal::Ahead => "ahead",
        CompactLocal::Behind => "behind",
        CompactLocal::Diverged => "diverged",
    }
}

pub fn compact_remote_label(v: CompactRemote) -> &'static str {
    match v {
        CompactRemote::CiPassing => "ci:ok",
        CompactRemote::CiFailing => "ci:fail",
        CompactRemote::CiPending => "ci:pend",
        CompactRemote::Unknown => "ci:?",
        CompactRemote::Unsupported => "ci:n/a",
        CompactRemote::Unavailable => "down",
        CompactRemote::Invalid => "invalid",
    }
}

pub fn freshness_label(repo: &RepositoryHealth) -> String {
    match repo.freshness.presented_as {
        FreshnessPresentedAs::Current => "current".into(),
        FreshnessPresentedAs::Cached => format!("{}s cache", repo.freshness.age_secs.unwrap_or(0)),
    }
}

pub fn evidence_text(repo: &RepositoryHealth) -> String {
    let wt = &repo.local.worktree;
    let div = &repo.local.divergence;
    let ci = &repo.remote.ci;
    format!(
        "path: {}\n\
         provider: {}\n\
         worktree: {:?}  modified={} staged={} untracked={}  @ {}\n\
         conflicts: {}\n\
         branch: {} upstream={} ahead={} behind={:?}  @ {}\n\
         fetch_age: {:?}s last={:?}\n\
         remote: {:?} {}\n\
         ci: {:?} {} {} {}\n\
         stale: {:?} {}\n\
         freshness: {:?} local={} remote={:?}\n",
        repo.identity.path,
        repo.identity.provider.as_str(),
        wt.state,
        wt.evidence.modified,
        wt.evidence.staged,
        wt.evidence.untracked,
        wt.updated_at,
        wt.evidence.conflicted_paths.join(", "),
        div.evidence.branch,
        div.evidence.upstream.clone().unwrap_or_default(),
        div.evidence.ahead,
        div.state,
        div.updated_at,
        repo.local.fetch_age.age_secs,
        repo.local.fetch_age.last_fetch_at,
        repo.remote.connection.state,
        repo.remote
            .connection
            .evidence
            .message
            .clone()
            .unwrap_or_default(),
        ci.state,
        ci.evidence.check_name.clone().unwrap_or_default(),
        ci.evidence.conclusion.clone().unwrap_or_default(),
        ci.updated_at,
        repo.staleness.state,
        repo.staleness.evidence.reasons.join("; "),
        repo.freshness.presented_as,
        repo.freshness.local_updated_at,
        repo.freshness.remote_updated_at,
    )
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

    #[test]
    fn attention_filter_and_sort() {
        let now = Utc::now();
        let inv = Inventory::new(vec![repo("zeta", false), repo("alpha", true)], vec![], now);
        let t = HealthThresholds::default();
        let vis = visible_repos(&inv, &t, "", true, SortKey::Attention);
        assert_eq!(vis.len(), 1);
        assert_eq!(vis[0].identity.name, "alpha");
        let vis = visible_repos(&inv, &t, "zet", false, SortKey::Name);
        assert_eq!(vis[0].identity.name, "zeta");
    }
}
