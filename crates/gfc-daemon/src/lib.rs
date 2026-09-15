pub mod rpc;
pub mod systemd;
pub mod webhooks;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use gfc_auth::{AuthObservation, observe, resolve};
use gfc_cache::Cache;
use gfc_config::{Config, LaunchTarget, Paths};
use gfc_git::{head_sha, scan_inventory};
use gfc_plugin::PluginHost;
use gfc_providers::{ForgeClient, GithubUser, parse_owner_repo};
use gfc_schema::{
    ConnectionEvidence, ConnectionSignal, ConnectionState, Inventory, ProviderKind, RemoteHealth,
    SCHEMA_VERSION, present_freshness,
};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Mutex, Notify};
use tracing::{info, warn};

use crate::rpc::{RpcRequest, err, ok};

#[derive(Clone)]
pub struct Daemon {
    inner: Arc<Mutex<Inner>>,
    scan_now: Arc<Notify>,
}

struct Inner {
    paths: Paths,
    config: Config,
    cache: Cache,
    inventory: Inventory,
    last_auth: Vec<AuthObservation>,
    last_network: Option<String>,
    last_scan_at: Option<chrono::DateTime<Utc>>,
    github_identity: Option<GithubIdentity>,
    github_identity_retry_at: Option<chrono::DateTime<Utc>>,
}

struct GithubIdentity {
    login: String,
    display_name: String,
    avatar_path: Option<PathBuf>,
}

impl GithubIdentity {
    fn from_user(user: GithubUser, avatar_path: Option<PathBuf>) -> Self {
        Self {
            login: user.login.clone(),
            display_name: user.display_name().to_string(),
            avatar_path,
        }
    }
}

impl Daemon {
    pub fn new(paths: Paths, config: Config, cache: Cache) -> Self {
        let now = Utc::now();
        Self {
            inner: Arc::new(Mutex::new(Inner {
                paths,
                config,
                cache,
                inventory: Inventory::new(vec![], vec![], now),
                last_auth: vec![],
                last_network: None,
                last_scan_at: None,
                github_identity: None,
                github_identity_retry_at: None,
            })),
            scan_now: Arc::new(Notify::new()),
        }
    }

    pub fn trigger_scan(&self) {
        self.scan_now.notify_waiters();
    }

    pub async fn snapshot(&self) -> Inventory {
        self.inner.lock().await.inventory.clone()
    }

    pub async fn run(self, socket: PathBuf) -> anyhow::Result<()> {
        if let Some(parent) = socket.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let _ = std::fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket)?;
        info!(path = %socket.display(), "daemon listening");

        let daemon = self.clone();
        tokio::spawn(async move { daemon.scan_loop().await });
        let daemon = self.clone();
        tokio::spawn(async move { daemon.config_watch().await });
        let daemon = self.clone();
        tokio::spawn(async move { daemon.maybe_webhooks().await });

        self.trigger_scan();

        loop {
            let (stream, _) = listener.accept().await?;
            let daemon = self.clone();
            tokio::spawn(async move {
                if let Err(err) = daemon.handle_client(stream).await {
                    warn!(error = %err, "client disconnected");
                }
            });
        }
    }

    async fn scan_loop(&self) {
        loop {
            if let Err(err) = self.refresh().await {
                warn!(error = %err, "scan failed");
            }
            let interval = {
                let inner = self.inner.lock().await;
                Duration::from_millis(inner.config.poll.local_interval_ms.max(250))
            };
            tokio::select! {
                _ = tokio::time::sleep(interval) => {}
                _ = self.scan_now.notified() => {}
            }
        }
    }

    async fn config_watch(&self) {
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let path = self.inner.lock().await.paths.config_file.clone();
            if !path.exists() {
                continue;
            }
            match Config::load(&path) {
                Ok(cfg) => {
                    let mut inner = self.inner.lock().await;
                    if inner.config != cfg {
                        info!("config hot-reloaded");
                        inner.config = cfg;
                        self.scan_now.notify_waiters();
                    }
                }
                Err(err) => warn!(error = %err, "invalid config ignored; last-good retained"),
            }
        }
    }

    async fn maybe_webhooks(&self) {
        let (enabled, bind, github, gitlab) = {
            let inner = self.inner.lock().await;
            let cfg = &inner.config.webhook;
            let (g, l) = webhooks::secrets_from_config(cfg);
            (cfg.enabled, cfg.bind.clone(), g, l)
        };
        if !enabled {
            return;
        }
        let (tx, mut rx) = tokio::sync::mpsc::channel(32);
        let state = webhooks::HookState {
            github_secret: github,
            gitlab_token: gitlab,
            origin_key: None,
            events: tx,
            seen: Arc::new(std::sync::Mutex::new(Default::default())),
        };
        let daemon = self.clone();
        tokio::spawn(async move {
            while rx.recv().await.is_some() {
                daemon.trigger_scan();
            }
        });
        webhooks::serve(bind, state).await;
    }

    async fn refresh(&self) -> anyhow::Result<()> {
        let (config, extra) = {
            let inner = self.inner.lock().await;
            (inner.config.clone(), Vec::<PathBuf>::new())
        };
        self.refresh_github_identity(&config).await;
        let mut inventory = scan_inventory(&config, &extra).await;
        self.enrich_remote(&config, &mut inventory).await;
        let now = Utc::now();
        for repo in &mut inventory.repositories {
            repo.recompute_staleness(&config.health, now);
        }
        let mut inner = self.inner.lock().await;
        let _ = inner.cache.put_inventory(&inventory);
        let _ = inner.cache.set_metric(
            "attention_count",
            &inventory.attention_count(&config.health).to_string(),
        );
        inner.inventory = inventory;
        inner.last_scan_at = Some(now);
        Ok(())
    }

    async fn refresh_github_identity(&self, config: &Config) {
        if !config.providers.github.enabled {
            let mut inner = self.inner.lock().await;
            inner.github_identity = None;
            inner.github_identity_retry_at = None;
            return;
        }
        {
            let inner = self.inner.lock().await;
            if inner.github_identity.is_some() {
                return;
            }
            if inner
                .github_identity_retry_at
                .is_some_and(|at| Utc::now() < at)
            {
                return;
            }
        }
        let cred = match resolve(ProviderKind::Github, &config.auth) {
            Ok(cred) => cred,
            Err(_) => return,
        };
        self.record_auth(observe(ProviderKind::Github, &cred)).await;
        self.observe_network("api.github.com").await;
        let client = ForgeClient::from_config(config);
        let user = match client.github_user(&cred.token).await {
            Ok(user) => user,
            Err(err) => {
                warn!(error = %err, "github identity fetch failed");
                self.inner.lock().await.github_identity_retry_at =
                    Some(Utc::now() + chrono::Duration::minutes(5));
                return;
            }
        };
        let avatar_dir = self.inner.lock().await.paths.avatar_dir.clone();
        let avatar_path = avatar_dir.join(GithubUser::avatar_filename(&user.login));
        let mut stored = None;
        if let Ok(bytes) = client.fetch_bytes(&user.avatar_url).await {
            let _ = std::fs::create_dir_all(&avatar_dir);
            if std::fs::write(&avatar_path, bytes).is_ok() {
                stored = Some(avatar_path);
            }
        }
        let mut inner = self.inner.lock().await;
        inner.github_identity = Some(GithubIdentity::from_user(user, stored));
        inner.github_identity_retry_at = None;
    }

    async fn enrich_remote(&self, config: &Config, inventory: &mut Inventory) {
        let client = ForgeClient::from_config(config);
        for repo in &mut inventory.repositories {
            let path = PathBuf::from(&repo.identity.path);
            let sha = match head_sha(&path) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let parsed = repo
                .identity
                .remote_url
                .as_deref()
                .and_then(parse_owner_repo);
            let result = match repo.identity.provider {
                ProviderKind::Github if config.providers.github.enabled => {
                    self.observe_network("api.github.com").await;
                    match resolve(ProviderKind::Github, &config.auth) {
                        Ok(cred) => {
                            self.record_auth(observe(ProviderKind::Github, &cred)).await;
                            let (owner, name) = match &parsed {
                                Some(p) => p,
                                None => continue,
                            };
                            client
                                .github_remote_health(&cred.token, owner, name, &sha)
                                .await
                        }
                        Err(err) => Ok(invalid(&err.to_string())),
                    }
                }
                ProviderKind::Gitlab if config.providers.gitlab.enabled => {
                    self.observe_network("gitlab").await;
                    match resolve(ProviderKind::Gitlab, &config.auth) {
                        Ok(cred) => {
                            self.record_auth(observe(ProviderKind::Gitlab, &cred)).await;
                            let (owner, name) = match &parsed {
                                Some(p) => p,
                                None => continue,
                            };
                            let project = format!("{owner}/{name}");
                            client
                                .gitlab_remote_health(
                                    &cred.token,
                                    &project,
                                    &repo.local.divergence.evidence.branch,
                                )
                                .await
                        }
                        Err(err) => Ok(invalid(&err.to_string())),
                    }
                }
                ProviderKind::Origin if config.providers.origin.enabled => {
                    self.observe_network("api.cursor.com").await;
                    let token = resolve(ProviderKind::Origin, &config.auth).ok();
                    if let Some(cred) = &token {
                        self.record_auth(observe(ProviderKind::Origin, cred)).await;
                    }
                    let (owner, name) = match &parsed {
                        Some(p) => (p.0.as_str(), p.1.as_str()),
                        None => ("unknown", "unknown"),
                    };
                    client
                        .origin_remote_health(
                            token.as_ref().map(|c| c.token.as_str()),
                            owner,
                            name,
                            &sha,
                            config.providers.origin.app_credentials_configured,
                        )
                        .await
                }
                ProviderKind::Git
                | ProviderKind::Github
                | ProviderKind::Gitlab
                | ProviderKind::Origin => Ok(repo.remote.clone()),
            };
            match result {
                Ok(remote) => {
                    repo.remote = remote;
                    repo.freshness = present_freshness(
                        repo.local.worktree.updated_at,
                        Some(repo.remote.ci.updated_at),
                        &config.health,
                        Utc::now(),
                    );
                }
                Err(err) => {
                    warn!(path = %repo.identity.path, error = %err, "provider failed");
                    repo.remote.connection = ConnectionSignal {
                        state: ConnectionState::Unavailable,
                        evidence: ConnectionEvidence {
                            message: Some(err.to_string()),
                            http_status: None,
                        },
                        updated_at: Utc::now(),
                    };
                }
            }
        }
        if !config.plugins.enabled.is_empty() {
            if let Ok(host) = PluginHost::new(&config.plugins) {
                let entries: Vec<_> = if config.plugins.summarizer_enabled {
                    config.plugins.enabled.clone()
                } else {
                    config
                        .plugins
                        .enabled
                        .iter()
                        .filter(|e| e.world != gfc_plugin::SUMMARIZER_WORLD)
                        .cloned()
                        .collect()
                };
                let _ = host.run_health_checks(&entries, &inventory.repositories);
            }
        }
    }

    async fn observe_network(&self, host: &str) {
        self.inner.lock().await.last_network = Some(host.into());
        tracing::info!(host, "network request");
    }

    async fn record_auth(&self, obs: AuthObservation) {
        let mut inner = self.inner.lock().await;
        inner.last_auth.retain(|a| a.provider != obs.provider);
        inner.last_auth.push(obs);
    }

    async fn handle_client(&self, mut stream: UnixStream) -> anyhow::Result<()> {
        loop {
            let mut len_buf = [0u8; 4];
            if stream.read_exact(&mut len_buf).await.is_err() {
                break;
            }
            let len = u32::from_be_bytes(len_buf) as usize;
            let mut buf = vec![0u8; len];
            stream.read_exact(&mut buf).await?;
            let req: RpcRequest = serde_json::from_slice(&buf)?;
            let resp = self.dispatch(req).await;
            let bytes = serde_json::to_vec(&resp)?;
            stream
                .write_all(&(bytes.len() as u32).to_be_bytes())
                .await?;
            stream.write_all(&bytes).await?;
        }
        Ok(())
    }

    async fn dispatch(&self, req: RpcRequest) -> crate::rpc::RpcResponse {
        match req.method.as_str() {
            "inventory.snapshot" => {
                let inv = self.snapshot().await;
                ok(req.id, serde_json::to_value(inv).unwrap_or(Value::Null))
            }
            "scan.trigger" => {
                self.trigger_scan();
                ok(req.id, json!({"ok": true}))
            }
            "config.get" => {
                let inner = self.inner.lock().await;
                match toml::to_string_pretty(&inner.config) {
                    Ok(text) => ok(
                        req.id,
                        json!({"toml": text, "path": inner.paths.config_file}),
                    ),
                    Err(e) => err(req.id, -32000, e.to_string()),
                }
            }
            "config.set" => {
                let toml_text = req.params.get("toml").and_then(Value::as_str).unwrap_or("");
                match toml::from_str::<Config>(toml_text) {
                    Ok(cfg) => match cfg.validate() {
                        Ok(()) => {
                            let mut inner = self.inner.lock().await;
                            if let Err(e) = cfg.save(&inner.paths.config_file) {
                                return err(req.id, -32000, e.to_string());
                            }
                            inner.config = cfg;
                            self.scan_now.notify_waiters();
                            ok(req.id, json!({"ok": true}))
                        }
                        Err(e) => err(req.id, -32602, e.to_string()),
                    },
                    Err(e) => err(req.id, -32602, e.to_string()),
                }
            }
            "auth.status" => {
                let inner = self.inner.lock().await;
                let items: Vec<Value> = inner
                    .last_auth
                    .iter()
                    .map(|a| json!({"provider": a.provider, "source": a.source, "label": a.label}))
                    .collect();
                ok(
                    req.id,
                    json!({
                        "credentials": items,
                        "last_network": inner.last_network,
                        "github": inner.github_identity.as_ref().map(|id| json!({
                            "login": id.login,
                            "name": id.display_name,
                            "avatar_path": id.avatar_path,
                        })),
                    }),
                )
            }
            "metrics.local" => {
                let inner = self.inner.lock().await;
                let attention = inner.inventory.attention_count(&inner.config.health);
                let current = inner
                    .inventory
                    .repositories
                    .iter()
                    .filter(|r| {
                        r.freshness.presented_as == gfc_schema::FreshnessPresentedAs::Current
                    })
                    .count();
                let total = inner.inventory.repositories.len();
                ok(
                    req.id,
                    json!({
                        "attention_count": attention,
                        "repo_count": total,
                        "current_count": current,
                        "schema_version": SCHEMA_VERSION,
                        "last_scan_at": inner.last_scan_at,
                        "current_local_remote_pct": if total == 0 {
                            0.0
                        } else {
                            (current as f64 / total as f64) * 100.0
                        },
                    }),
                )
            }
            "repo.launch" => {
                let path = req.params.get("path").and_then(Value::as_str).unwrap_or("");
                let target = req
                    .params
                    .get("target")
                    .and_then(Value::as_str)
                    .unwrap_or("editor");
                let inner = self.inner.lock().await;
                let repo = inner
                    .inventory
                    .repositories
                    .iter()
                    .find(|r| r.identity.path == path)
                    .cloned();
                let launch = inner.config.launch.clone();
                drop(inner);
                match repo {
                    Some(repo) => {
                        if let Err(e) = launch_repo(&launch, target, &repo) {
                            err(req.id, -32000, e.to_string())
                        } else {
                            ok(req.id, json!({"ok": true}))
                        }
                    }
                    None => err(req.id, -32602, "unknown repository"),
                }
            }
            "inventory.subscribe" => ok(req.id, json!({"ok": true})),
            other => err(req.id, -32601, format!("unknown method {other}")),
        }
    }
}

fn invalid(msg: &str) -> RemoteHealth {
    let now = Utc::now();
    RemoteHealth {
        connection: ConnectionSignal {
            state: ConnectionState::Invalid,
            evidence: ConnectionEvidence {
                message: Some(msg.into()),
                http_status: None,
            },
            updated_at: now,
        },
        ci: gfc_schema::unknown_remote(now).ci,
    }
}

pub fn launch_repo(
    launch: &gfc_config::LaunchConfig,
    target: &str,
    repo: &gfc_schema::RepositoryHealth,
) -> std::io::Result<()> {
    let url = repo
        .identity
        .remote_url
        .clone()
        .unwrap_or_else(|| repo.identity.path.clone());
    let parsed = match target {
        "editor" => LaunchTarget::Editor,
        "terminal" => LaunchTarget::Terminal,
        "browser" => LaunchTarget::Browser,
        "lazygit" => LaunchTarget::Lazygit,
        "custom" => LaunchTarget::Custom,
        _ => launch.default_target,
    };
    let template = match parsed {
        LaunchTarget::Editor => launch.editor.as_str(),
        LaunchTarget::Terminal => launch.terminal.as_str(),
        LaunchTarget::Browser => launch.browser.as_str(),
        LaunchTarget::Lazygit => launch.lazygit.as_str(),
        LaunchTarget::Custom => launch
            .custom
            .first()
            .map(|c| c.command.as_str())
            .unwrap_or(launch.editor.as_str()),
    };
    let cmd = template
        .replace("{path}", &repo.identity.path)
        .replace("{url}", &url);
    std::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .spawn()?;
    Ok(())
}

pub fn default_socket(paths: &Paths) -> &Path {
    &paths.socket_file
}

pub fn is_running(socket: &Path) -> bool {
    socket.exists() && std::os::unix::net::UnixStream::connect(socket).is_ok()
}
