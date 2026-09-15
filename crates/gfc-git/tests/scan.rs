use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use gfc_config::Config;
use gfc_git::{discover, index_mtime, scan_discovered, scan_one, DiscoveredRepo};
use gfc_schema::{DivergenceState, WorktreeState};
use tempfile::TempDir;

fn git(cwd: &Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .env("GIT_AUTHOR_NAME", "gfc")
        .env("GIT_AUTHOR_EMAIL", "gfc@example.test")
        .env("GIT_COMMITTER_NAME", "gfc")
        .env("GIT_COMMITTER_EMAIL", "gfc@example.test")
        .status()
        .expect("git");
    assert!(status.success(), "git {args:?} failed");
}

fn init_repo(root: &Path, name: &str) -> std::path::PathBuf {
    let path = root.join(name);
    fs::create_dir_all(&path).unwrap();
    git(&path, &["init", "-b", "main"]);
    git(&path, &["config", "user.email", "gfc@example.test"]);
    git(&path, &["config", "user.name", "gfc"]);
    fs::write(path.join("README.md"), format!("{name}\n")).unwrap();
    git(&path, &["add", "README.md"]);
    git(&path, &["commit", "-m", "init"]);
    path
}

#[test]
fn discovers_nested_repos_and_skips_target() {
    let tmp = TempDir::new().unwrap();
    let a = init_repo(tmp.path(), "alpha");
    let _b = init_repo(tmp.path(), "beta");
    fs::create_dir_all(a.join("target/.git")).unwrap();
    let cfg = Config {
        roots: vec![tmp.path().to_path_buf()],
        ..Config::default()
    };
    let found = discover(&cfg, &[]);
    let names: Vec<_> = found.iter().map(|r| r.name.as_str()).collect();
    assert!(names.contains(&"alpha"));
    assert!(names.contains(&"beta"));
    assert!(!names.contains(&"target"));
}

#[test]
fn dirty_and_clean_worktrees() {
    let tmp = TempDir::new().unwrap();
    let clean = init_repo(tmp.path(), "clean");
    let dirty = init_repo(tmp.path(), "dirty");
    fs::write(dirty.join("README.md"), "changed\n").unwrap();
    let cfg = Config::default();
    let clean_h = scan_one(
        &cfg,
        &DiscoveredRepo {
            path: clean,
            name: "clean".into(),
            group: None,
        },
        chrono::Utc::now(),
    )
    .unwrap();
    let dirty_h = scan_one(
        &cfg,
        &DiscoveredRepo {
            path: dirty,
            name: "dirty".into(),
            group: None,
        },
        chrono::Utc::now(),
    )
    .unwrap();
    assert_eq!(clean_h.local.worktree.state, WorktreeState::Clean);
    assert_eq!(dirty_h.local.worktree.state, WorktreeState::Dirty);
    assert!(dirty_h.needs_attention(&cfg.health));
}

#[test]
fn status_does_not_write_index() {
    let tmp = TempDir::new().unwrap();
    let repo = init_repo(tmp.path(), "locked");
    let before = index_mtime(&repo).expect("index exists");
    std::thread::sleep(Duration::from_millis(20));
    let cfg = Config::default();
    scan_one(
        &cfg,
        &DiscoveredRepo {
            path: repo.clone(),
            name: "locked".into(),
            group: None,
        },
        chrono::Utc::now(),
    )
    .unwrap();
    let after = index_mtime(&repo).expect("index exists");
    assert_eq!(before, after, "scan must not write .git/index");
}

#[test]
fn failed_repo_does_not_block_inventory() {
    let tmp = TempDir::new().unwrap();
    let good = init_repo(tmp.path(), "good");
    let bad = tmp.path().join("not-a-repo");
    fs::create_dir_all(&bad).unwrap();
    let cfg = Config::default();
    let inventory = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(scan_discovered(
            &cfg,
            vec![
                DiscoveredRepo {
                    path: good,
                    name: "good".into(),
                    group: None,
                },
                DiscoveredRepo {
                    path: bad,
                    name: "bad".into(),
                    group: None,
                },
            ],
        ));
    assert_eq!(inventory.repositories.len(), 1);
    assert_eq!(inventory.errors.len(), 1);
    assert_eq!(inventory.repositories[0].identity.name, "good");
}

#[test]
fn ahead_behind_from_fake_upstream() {
    let tmp = TempDir::new().unwrap();
    let repo = init_repo(tmp.path(), "div");
    let bare = tmp.path().join("upstream.git");
    git(tmp.path(), &["clone", "--bare", repo.to_str().unwrap(), bare.to_str().unwrap()]);
    git(&repo, &["remote", "add", "origin", bare.to_str().unwrap()]);
    git(&repo, &["fetch", "origin"]);
    git(&repo, &["branch", "--set-upstream-to=origin/main", "main"]);
    fs::write(repo.join("extra.md"), "x\n").unwrap();
    git(&repo, &["add", "extra.md"]);
    git(&repo, &["commit", "-m", "ahead"]);
    let cfg = Config::default();
    let health = scan_one(
        &cfg,
        &DiscoveredRepo {
            path: repo,
            name: "div".into(),
            group: None,
        },
        chrono::Utc::now(),
    )
    .unwrap();
    assert_eq!(health.local.divergence.state, DivergenceState::Ahead);
    assert_eq!(health.local.divergence.evidence.ahead, 1);
}
