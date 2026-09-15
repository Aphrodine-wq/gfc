use chrono::{Duration, Utc};
use gfc_cache::Cache;
use gfc_schema::{
    present_freshness, unknown_remote, DivergenceEvidence, DivergenceSignal, DivergenceState,
    FetchAgeSignal, FreshnessPresentedAs, HealthThresholds, Identity, LocalHealth, ProviderKind,
    RepositoryHealth, StalenessEvidence, StalenessSignal, StalenessState, WorktreeEvidence,
    WorktreeSignal, WorktreeState, SCHEMA_VERSION,
};
use tempfile::TempDir;

fn repo(id: &str, local_age_hours: i64) -> RepositoryHealth {
    let now = Utc::now() - Duration::hours(local_age_hours);
    RepositoryHealth {
        schema_version: SCHEMA_VERSION.into(),
        identity: Identity {
            id: id.into(),
            name: id.into(),
            path: format!("/tmp/{id}"),
            provider: ProviderKind::Git,
            remote_url: None,
            group: None,
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
fn load_never_marks_stale_cache_current() {
    let dir = TempDir::new().unwrap();
    let cache = Cache::open(&dir.path().join("cache.sqlite")).unwrap();
    cache.put(&repo("old", 5)).unwrap();
    cache.put(&repo("new", 0)).unwrap();
    let inv = cache
        .load_inventory(&HealthThresholds::default(), Utc::now())
        .unwrap();
    for r in &inv.repositories {
        if r.identity.id == "old" {
            assert_eq!(r.freshness.presented_as, FreshnessPresentedAs::Cached);
        }
    }
}
