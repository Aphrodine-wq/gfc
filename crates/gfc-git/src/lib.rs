//! Read-only local Git inventory.
//!
//! Status is gathered with `git --no-optional-locks` so the index is never
//! written. One failed repository cannot block the rest of the scan.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::SystemTime;

use chrono::{DateTime, Utc};
use gfc_config::{expand_path, Config};
use gfc_schema::{
    present_freshness, unknown_remote, DivergenceEvidence, DivergenceSignal, DivergenceState,
    FetchAgeSignal, Identity, Inventory, LocalHealth, ProviderKind, RepositoryHealth, ScanError,
    StalenessEvidence, StalenessSignal, StalenessState, WorktreeEvidence, WorktreeSignal,
    WorktreeState, SCHEMA_VERSION,
};
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use walkdir::WalkDir;

const SKIP_DIR_NAMES: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    ".direnv",
    "vendor",
    ".cache",
];

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("{0}")]
    Message(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone)]
pub struct DiscoveredRepo {
    pub path: PathBuf,
    pub name: String,
    pub group: Option<String>,
}

pub fn discover(config: &Config, extra_roots: &[PathBuf]) -> Vec<DiscoveredRepo> {
    let mut found = Vec::new();
    let mut roots: Vec<PathBuf> = config.expanded_roots();
    roots.extend(extra_roots.iter().map(|p| expand_path(p)));
    for repo in config.expanded_repos() {
        if is_git_repo(&repo.path) {
            found.push(to_discovered(&repo.path, repo.name.clone()));
        }
    }
    for root in roots {
        if !root.exists() {
            continue;
        }
        for entry in WalkDir::new(&root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                let name = e.file_name().to_string_lossy();
                !SKIP_DIR_NAMES.contains(&name.as_ref())
            })
            .flatten()
        {
            if entry.file_type().is_dir() && is_git_repo(entry.path()) {
                found.push(to_discovered(entry.path(), None));
            }
        }
        if is_git_repo(&root) {
            found.push(to_discovered(&root, None));
        }
    }
    found.sort_by(|a, b| a.path.cmp(&b.path));
    found.dedup_by(|a, b| a.path == b.path);
    found
}

fn to_discovered(path: &Path, name: Option<String>) -> DiscoveredRepo {
    let name = name.unwrap_or_else(|| {
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string())
    });
    let group = path
        .parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned());
    DiscoveredRepo {
        path: path.to_path_buf(),
        name,
        group,
    }
}

pub fn is_git_repo(path: &Path) -> bool {
    let git = path.join(".git");
    git.is_dir() || git.is_file()
}

pub async fn scan_inventory(
    config: &Config,
    extra_roots: &[PathBuf],
) -> Inventory {
    let discovered = discover(config, extra_roots);
    scan_discovered(config, discovered).await
}

pub async fn scan_discovered(config: &Config, discovered: Vec<DiscoveredRepo>) -> Inventory {
    let now = Utc::now();
    let concurrency = config.scan.concurrency.max(1);
    let sem = Arc::new(Semaphore::new(concurrency));
    let mut set = JoinSet::new();
    let thresholds = config.health.clone();

    for repo in discovered {
        let sem = sem.clone();
        let cfg = config.clone();
        set.spawn(async move {
            let _permit = sem.acquire_owned().await;
            tokio::task::spawn_blocking(move || scan_one(&cfg, &repo, Utc::now())).await
        });
    }

    let mut repositories = Vec::new();
    let mut errors = Vec::new();
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok(Ok(Ok(health))) => repositories.push(health),
            Ok(Ok(Err(err))) => errors.push(err),
            Ok(Err(join_err)) => errors.push(ScanError {
                path: "<unknown>".into(),
                message: format!("scan task failed: {join_err}"),
                updated_at: now,
            }),
            Err(join_err) => errors.push(ScanError {
                path: "<unknown>".into(),
                message: format!("isolated scan panic: {join_err}"),
                updated_at: now,
            }),
        }
    }
    repositories.sort_by(|a, b| a.identity.path.cmp(&b.identity.path));
    let mut inventory = Inventory::new(repositories, errors, now);
    for repo in &mut inventory.repositories {
        repo.recompute_staleness(&thresholds, Utc::now());
    }
    inventory
}

pub fn scan_one(
    config: &Config,
    repo: &DiscoveredRepo,
    now: DateTime<Utc>,
) -> Result<RepositoryHealth, ScanError> {
    match scan_one_inner(config, repo, now) {
        Ok(health) => Ok(health),
        Err(err) => Err(ScanError {
            path: repo.path.display().to_string(),
            message: err.to_string(),
            updated_at: now,
        }),
    }
}

fn scan_one_inner(
    config: &Config,
    repo: &DiscoveredRepo,
    now: DateTime<Utc>,
) -> Result<RepositoryHealth, GitError> {
    let porcelain = git_output(
        &repo.path,
        &[
            "--no-optional-locks",
            "status",
            "--porcelain=v2",
            "-b",
            "--untracked-files=all",
        ],
    )?;
    let parsed = parse_porcelain_v2(&porcelain);
    let remote_url = git_output(
        &repo.path,
        &["--no-optional-locks", "remote", "get-url", "origin"],
    )
    .ok()
    .map(|s| s.trim().to_string())
    .filter(|s| !s.is_empty());
    let provider = classify_provider(remote_url.as_deref());
    let fetch_age = fetch_age_signal(&repo.path, now);
    let tags = config.tags_for(&repo.path);
    let workspaces = config.workspaces_for(&repo.path, provider, &tags);
    let id = repo_id(&repo.path);

    let worktree_state = if !parsed.conflicted_paths.is_empty() {
        WorktreeState::Conflicted
    } else if parsed.modified + parsed.staged + parsed.untracked > 0 {
        WorktreeState::Dirty
    } else {
        WorktreeState::Clean
    };
    let divergence_state = if parsed.upstream.is_none() {
        DivergenceState::NoUpstream
    } else if parsed.ahead > 0 && parsed.behind > 0 {
        DivergenceState::Diverged
    } else if parsed.ahead > 0 {
        DivergenceState::Ahead
    } else if parsed.behind > 0 {
        DivergenceState::Behind
    } else {
        DivergenceState::Synced
    };

    let health = RepositoryHealth {
        schema_version: SCHEMA_VERSION.into(),
        identity: Identity {
            id,
            name: repo.name.clone(),
            path: repo.path.display().to_string(),
            provider,
            remote_url,
            group: repo.group.clone(),
            tags,
            workspaces,
        },
        local: LocalHealth {
            worktree: WorktreeSignal {
                state: worktree_state,
                evidence: WorktreeEvidence {
                    modified: parsed.modified,
                    staged: parsed.staged,
                    untracked: parsed.untracked,
                    conflicted_paths: parsed.conflicted_paths,
                },
                updated_at: now,
            },
            divergence: DivergenceSignal {
                state: divergence_state,
                evidence: DivergenceEvidence {
                    branch: parsed.branch,
                    upstream: parsed.upstream,
                    ahead: parsed.ahead,
                    behind: parsed.behind,
                },
                updated_at: now,
            },
            fetch_age,
        },
        remote: unknown_remote(now),
        staleness: StalenessSignal {
            state: StalenessState::Fresh,
            evidence: StalenessEvidence {
                fetch_age_secs: None,
                unfinished_work: false,
                last_activity_at: last_activity(&repo.path),
                reasons: vec![],
            },
            updated_at: now,
        },
        freshness: present_freshness(now, None, &config.health, now),
        error: None,
    };
    Ok(health)
}

#[derive(Default)]
struct Porcelain {
    branch: String,
    upstream: Option<String>,
    ahead: u32,
    behind: u32,
    modified: u32,
    staged: u32,
    untracked: u32,
    conflicted_paths: Vec<String>,
}

fn parse_porcelain_v2(text: &str) -> Porcelain {
    let mut out = Porcelain {
        branch: "HEAD".into(),
        ..Porcelain::default()
    };
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("# branch.head ") {
            out.branch = rest.trim().to_string();
            continue;
        }
        if let Some(rest) = line.strip_prefix("# branch.upstream ") {
            out.upstream = Some(rest.trim().to_string());
            continue;
        }
        if let Some(rest) = line.strip_prefix("# branch.ab ") {
            // +ahead -behind
            for part in rest.split_whitespace() {
                if let Some(n) = part.strip_prefix('+') {
                    out.ahead = n.parse().unwrap_or(0);
                } else if let Some(n) = part.strip_prefix('-') {
                    out.behind = n.parse().unwrap_or(0);
                }
            }
            continue;
        }
        if line.starts_with('?') {
            out.untracked += 1;
            continue;
        }
        if line.starts_with('u') {
            out.modified += 1;
            if let Some(path) = line.split_whitespace().last() {
                out.conflicted_paths.push(path.to_string());
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("1 ") {
            let xy = rest.get(..2).unwrap_or("..");
            let x = xy.chars().next().unwrap_or('.');
            let y = xy.chars().nth(1).unwrap_or('.');
            if x != '.' {
                out.staged += 1;
            }
            if y != '.' {
                out.modified += 1;
            }
            continue;
        }
        if line.starts_with("2 ") {
            out.modified += 1;
        }
    }
    out
}

pub fn classify_provider(remote_url: Option<&str>) -> ProviderKind {
    let Some(url) = remote_url else {
        return ProviderKind::Git;
    };
    let lower = url.to_ascii_lowercase();
    if lower.contains("github.com") {
        ProviderKind::Github
    } else if lower.contains("origin.cursor.com") || lower.contains("cursor.com/codebase") {
        ProviderKind::Origin
    } else if lower.contains("gitlab") {
        ProviderKind::Gitlab
    } else {
        ProviderKind::Git
    }
}

fn git_output(repo: &Path, args: &[&str]) -> Result<String, GitError> {
    let output = Command::new("git")
        .args(["-C"])
        .arg(repo)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GitError::Message(format!(
            "git {} failed: {}",
            args.join(" "),
            stderr.trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn fetch_age_signal(repo: &Path, now: DateTime<Utc>) -> FetchAgeSignal {
    let fetch_head = repo.join(".git/FETCH_HEAD");
    let last = mtime_utc(&fetch_head).or_else(|| {
        // .git may be a file for worktrees / submodules.
        gitdir(repo).and_then(|dir| mtime_utc(&dir.join("FETCH_HEAD")))
    });
    FetchAgeSignal {
        last_fetch_at: last,
        age_secs: last.map(|ts| (now - ts).num_seconds().max(0) as u64),
        updated_at: now,
    }
}

fn last_activity(repo: &Path) -> Option<DateTime<Utc>> {
    mtime_utc(&repo.join(".git"))
        .or_else(|| gitdir(repo).and_then(|dir| mtime_utc(&dir)))
}

fn gitdir(repo: &Path) -> Option<PathBuf> {
    let git = repo.join(".git");
    if git.is_dir() {
        return Some(git);
    }
    if git.is_file() {
        let text = std::fs::read_to_string(&git).ok()?;
        let path = text.strip_prefix("gitdir:")?.trim();
        let dir = if Path::new(path).is_absolute() {
            PathBuf::from(path)
        } else {
            repo.join(path)
        };
        return Some(dir);
    }
    None
}

fn mtime_utc(path: &Path) -> Option<DateTime<Utc>> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    Some(system_time_to_utc(modified))
}

fn system_time_to_utc(time: SystemTime) -> DateTime<Utc> {
    DateTime::<Utc>::from(time)
}

fn repo_id(path: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(path.to_string_lossy().as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Head SHA for remote CI lookups. Read-only.
pub fn head_sha(repo: &Path) -> Result<String, GitError> {
    let sha = git_output(repo, &["--no-optional-locks", "rev-parse", "HEAD"])?;
    Ok(sha.trim().to_string())
}

/// Read-only connectivity probe. Does not fetch or update the index.
pub fn ls_remote_head(repo: &Path) -> Result<String, GitError> {
    git_output(
        repo,
        &[
            "--no-optional-locks",
            "ls-remote",
            "--heads",
            "origin",
            "HEAD",
        ],
    )
}

pub fn index_mtime(repo: &Path) -> Option<SystemTime> {
    let index = repo.join(".git/index");
    std::fs::metadata(index)
        .and_then(|m| m.modified())
        .ok()
        .or_else(|| {
            gitdir(repo).and_then(|dir| {
                std::fs::metadata(dir.join("index"))
                    .and_then(|m| m.modified())
                    .ok()
            })
        })
}

#[cfg(test)]
mod porcelain_tests {
    use super::*;

    #[test]
    fn parses_ahead_behind_and_dirty() {
        let text = "\
# branch.oid abc
# branch.head main
# branch.upstream origin/main
# branch.ab +2 -1
1 .M N... 100644 100644 100644 abc abc file.txt
? extra
";
        let parsed = parse_porcelain_v2(text);
        assert_eq!(parsed.ahead, 2);
        assert_eq!(parsed.behind, 1);
        assert_eq!(parsed.modified, 1);
        assert_eq!(parsed.untracked, 1);
        assert_eq!(parsed.branch, "main");
    }

    #[test]
    fn classifies_providers() {
        assert_eq!(
            classify_provider(Some("git@github.com:acme/app.git")),
            ProviderKind::Github
        );
        assert_eq!(
            classify_provider(Some("https://origin.cursor.com/acme/app.git")),
            ProviderKind::Origin
        );
        assert_eq!(
            classify_provider(Some("https://gitlab.example.com/acme/app.git")),
            ProviderKind::Gitlab
        );
        assert_eq!(classify_provider(None), ProviderKind::Git);
    }
}
