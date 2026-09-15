use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

use criterion::{criterion_group, criterion_main, Criterion};
use gfc_config::Config;
use gfc_git::scan_inventory;
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
        .unwrap();
    assert!(status.success());
}

fn make_repos(root: &Path, n: usize) {
    for i in 0..n {
        let path = root.join(format!("repo-{i:02}"));
        fs::create_dir_all(&path).unwrap();
        git(&path, &["init", "-b", "main"]);
        git(&path, &["config", "user.email", "gfc@example.test"]);
        git(&path, &["config", "user.name", "gfc"]);
        fs::write(path.join("README.md"), format!("repo {i}\n")).unwrap();
        git(&path, &["add", "README.md"]);
        git(&path, &["commit", "-m", "init"]);
        if i % 5 == 0 {
            fs::write(path.join("README.md"), format!("dirty {i}\n")).unwrap();
        }
    }
}

fn sequential_git_status(root: &Path) {
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.join(".git").exists() {
            let _ = Command::new("git")
                .args(["-C"])
                .arg(&path)
                .args(["--no-optional-locks", "status", "--porcelain=v2", "-b"])
                .output();
        }
    }
}

fn bench_local_scan(c: &mut Criterion) {
    let tmp = TempDir::new().unwrap();
    make_repos(tmp.path(), 50);
    let cfg = Config {
        roots: vec![tmp.path().to_path_buf()],
        scan: gfc_config::ScanConfig { concurrency: 12 },
        ..Config::default()
    };
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    let start = Instant::now();
    let inv = rt.block_on(scan_inventory(&cfg, &[]));
    eprintln!(
        "prototype: 50-repo gfc scan {:?} ({} repos, {} errors)",
        start.elapsed(),
        inv.repositories.len(),
        inv.errors.len()
    );

    c.bench_function("gfc_scan_50", |b| {
        b.iter(|| rt.block_on(scan_inventory(&cfg, &[])));
    });
    c.bench_function("sequential_git_status_50", |b| {
        b.iter(|| sequential_git_status(tmp.path()));
    });
}

criterion_group!(benches, bench_local_scan);
criterion_main!(benches);
