use std::time::Duration;

use gfc_cache::Cache;
use gfc_config::{Config, Paths};
use gfc_daemon::Daemon;
use gfc_daemon::rpc::{RpcRequest, call, write_frame};
use serde_json::json;
use tempfile::TempDir;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn snapshot_round_trip() {
    let tmp = TempDir::new().unwrap();
    let socket = tmp.path().join("gfc.sock");
    let cache = Cache::open(&tmp.path().join("c.sqlite")).unwrap();
    let mut paths = Paths::resolve().unwrap();
    paths.socket_file = socket.clone();
    paths.cache_file = tmp.path().join("c.sqlite");
    paths.avatar_dir = tmp.path().join("avatars");
    paths.config_file = tmp.path().join("config.toml");
    let cfg = Config {
        roots: vec![tmp.path().to_path_buf()],
        ..Config::default()
    };
    let daemon = Daemon::new(paths, cfg, cache);
    let sock = socket.clone();
    let handle = tokio::spawn(async move { daemon.run(sock).await });
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if socket.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("daemon socket");

    let value = tokio::task::spawn_blocking({
        let socket = socket.clone();
        move || {
            let mut stream = std::os::unix::net::UnixStream::connect(&socket).expect("connect");
            call(&mut stream, "inventory.snapshot", json!({})).unwrap()
        }
    })
    .await
    .unwrap();
    assert_eq!(value["schema_version"], "1.0.0");
    assert!(value.get("repositories").is_some());

    let _ = tokio::task::spawn_blocking({
        let socket = socket.clone();
        move || {
            let mut stream = std::os::unix::net::UnixStream::connect(&socket).expect("connect");
            let req = RpcRequest {
                jsonrpc: "2.0".into(),
                id: json!(2),
                method: "scan.trigger".into(),
                params: json!({}),
            };
            write_frame(&mut stream, &serde_json::to_vec(&req).unwrap()).unwrap();
        }
    })
    .await;
    handle.abort();
}
