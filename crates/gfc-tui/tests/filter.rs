use chrono::Utc;
use gfc_schema::*;
use gfc_tui::{SortKey, visible_repos};

fn repo(name: &str, provider: ProviderKind) -> RepositoryHealth {
    let now = Utc::now();
    RepositoryHealth {
        schema_version: SCHEMA_VERSION.into(),
        identity: Identity {
            id: name.into(),
            name: name.into(),
            path: format!("/src/{name}"),
            provider,
            remote_url: None,
            group: Some("src".into()),
            tags: vec![],
            workspaces: vec!["work".into()],
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
        freshness: present_freshness(now, None, &HealthThresholds::default(), now),
        error: None,
    }
}

#[test]
fn sorts_by_name_and_filters_provider() {
    let now = Utc::now();
    let inv = Inventory::new(
        vec![
            repo("zeta", ProviderKind::Gitlab),
            repo("alpha", ProviderKind::Github),
        ],
        vec![],
        now,
    );
    let t = HealthThresholds::default();
    let by_name = visible_repos(&inv, &t, "", false, SortKey::Name);
    assert_eq!(by_name[0].identity.name, "alpha");
    let gh = visible_repos(&inv, &t, "github", false, SortKey::Provider);
    assert_eq!(gh.len(), 1);
    assert_eq!(gh[0].identity.name, "alpha");
}
