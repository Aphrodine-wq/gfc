//! XDG configuration shared by the TUI and daemon.

use std::fs;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use gfc_schema::{HealthThresholds, ProviderKind};
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("could not resolve XDG directories")]
    NoProjectDirs,
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid TOML in {path}: {source}")]
    Toml {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("invalid configuration: {0}")]
    Validation(String),
}

#[derive(Debug, Clone)]
pub struct Paths {
    pub config_file: PathBuf,
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub cache_file: PathBuf,
    pub avatar_dir: PathBuf,
    pub runtime_dir: PathBuf,
    pub socket_file: PathBuf,
}

impl Paths {
    pub fn resolve() -> Result<Self, ConfigError> {
        let dirs =
            ProjectDirs::from("dev", "GitForgeCockpit", "gfc").ok_or(ConfigError::NoProjectDirs)?;
        let config_dir = dirs.config_dir().to_path_buf();
        let data_dir = dirs.data_dir().to_path_buf();
        let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("gfc"))
            .join("gfc");
        Ok(Self {
            config_file: config_dir.join("config.toml"),
            config_dir,
            cache_file: data_dir.join("cache.sqlite"),
            avatar_dir: dirs.cache_dir().join("avatars"),
            data_dir,
            socket_file: runtime_dir.join("daemon.sock"),
            runtime_dir,
        })
    }

    pub fn ensure_dirs(&self) -> Result<(), ConfigError> {
        for dir in [
            &self.config_dir,
            &self.data_dir,
            &self.runtime_dir,
            &self.avatar_dir,
        ] {
            fs::create_dir_all(dir).map_err(|source| ConfigError::Io {
                path: dir.clone(),
                source,
            })?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub roots: Vec<PathBuf>,
    #[serde(default)]
    pub repos: Vec<RegisteredRepo>,
    #[serde(default)]
    pub workspaces: Vec<Workspace>,
    #[serde(default)]
    pub tags: Vec<TagAssignment>,
    #[serde(default)]
    pub providers: ProvidersConfig,
    #[serde(default)]
    pub poll: PollConfig,
    #[serde(default)]
    pub webhook: WebhookConfig,
    #[serde(default)]
    pub health: HealthThresholds,
    #[serde(default)]
    pub launch: LaunchConfig,
    #[serde(default)]
    pub plugins: PluginsConfig,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub scan: ScanConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegisteredRepo {
    pub path: PathBuf,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Workspace {
    pub name: String,
    #[serde(default)]
    pub match_tags: Vec<String>,
    #[serde(default)]
    pub match_roots: Vec<PathBuf>,
    #[serde(default)]
    pub match_providers: Vec<ProviderKind>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TagAssignment {
    pub path: PathBuf,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ProvidersConfig {
    #[serde(default)]
    pub github: GithubConfig,
    #[serde(default)]
    pub gitlab: GitlabConfig,
    #[serde(default)]
    pub origin: OriginConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GithubConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_github_api")]
    pub api_url: String,
}

impl Default for GithubConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            api_url: default_github_api(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GitlabConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_gitlab_api")]
    pub api_url: String,
}

impl Default for GitlabConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            api_url: default_gitlab_api(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OriginConfig {
    /// Generic Git adapter is always on. Full CI requires app credentials.
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_origin_api")]
    pub api_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installation_id: Option<String>,
    #[serde(default)]
    pub app_credentials_configured: bool,
}

impl Default for OriginConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            api_url: default_origin_api(),
            installation_id: None,
            app_credentials_configured: false,
        }
    }
}

fn default_true() -> bool {
    true
}
fn default_github_api() -> String {
    "https://api.github.com".into()
}
fn default_gitlab_api() -> String {
    "https://gitlab.com/api/v4".into()
}
fn default_origin_api() -> String {
    "https://api.cursor.com/v1/origin".into()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PollConfig {
    #[serde(default = "default_local_ms")]
    pub local_interval_ms: u64,
    #[serde(default = "default_remote_secs")]
    pub remote_interval_secs: u64,
}

impl Default for PollConfig {
    fn default() -> Self {
        Self {
            local_interval_ms: default_local_ms(),
            remote_interval_secs: default_remote_secs(),
        }
    }
}

fn default_local_ms() -> u64 {
    1500
}
fn default_remote_secs() -> u64 {
    120
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WebhookConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_webhook_bind")]
    pub bind: String,
    /// Name of the secret-service / env / command slot; never a raw secret.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github_secret_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gitlab_token_ref: Option<String>,
}

impl Default for WebhookConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bind: default_webhook_bind(),
            github_secret_ref: None,
            gitlab_token_ref: None,
        }
    }
}

fn default_webhook_bind() -> String {
    "127.0.0.1:9847".into()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LaunchConfig {
    #[serde(default = "default_editor")]
    pub editor: String,
    #[serde(default = "default_terminal")]
    pub terminal: String,
    #[serde(default = "default_browser")]
    pub browser: String,
    #[serde(default = "default_lazygit")]
    pub lazygit: String,
    #[serde(default)]
    pub custom: Vec<CustomLaunch>,
    #[serde(default = "default_launch_target")]
    pub default_target: LaunchTarget,
}

impl Default for LaunchConfig {
    fn default() -> Self {
        Self {
            editor: default_editor(),
            terminal: default_terminal(),
            browser: default_browser(),
            lazygit: default_lazygit(),
            custom: vec![],
            default_target: LaunchTarget::Editor,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CustomLaunch {
    pub name: String,
    pub command: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchTarget {
    Editor,
    Terminal,
    Browser,
    Lazygit,
    Custom,
}

fn default_editor() -> String {
    "${EDITOR:-xdg-open} {path}".into()
}
fn default_terminal() -> String {
    "xterm -e bash -lc 'cd {path} && exec $SHELL'".into()
}
fn default_browser() -> String {
    "xdg-open {url}".into()
}
fn default_lazygit() -> String {
    "lazygit -p {path}".into()
}
fn default_launch_target() -> LaunchTarget {
    LaunchTarget::Editor
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PluginsConfig {
    #[serde(default)]
    pub enabled: Vec<PluginEntry>,
    /// Local-model summarization is disabled by default and is not required.
    #[serde(default)]
    pub summarizer_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginEntry {
    pub path: PathBuf,
    pub world: String,
    #[serde(default)]
    pub capabilities: PluginCapabilities,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PluginCapabilities {
    #[serde(default)]
    pub network: Vec<String>,
    #[serde(default)]
    pub filesystem: Vec<PathBuf>,
    #[serde(default)]
    pub process: bool,
    #[serde(default)]
    pub credentials: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuthConfig {
    /// Ordered credential backends.
    #[serde(default = "default_auth_order")]
    pub order: Vec<AuthBackend>,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            order: default_auth_order(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthBackend {
    ProviderCli,
    SecretService,
    Env,
    Command,
}

fn default_auth_order() -> Vec<AuthBackend> {
    vec![
        AuthBackend::ProviderCli,
        AuthBackend::SecretService,
        AuthBackend::Env,
        AuthBackend::Command,
    ]
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScanConfig {
    #[serde(default = "default_concurrency")]
    pub concurrency: usize,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            concurrency: default_concurrency(),
        }
    }
}

fn default_concurrency() -> usize {
    12
}

impl Config {
    pub fn load_or_default(path: &Path) -> Result<Self, ConfigError> {
        if !path.exists() {
            return Ok(Self::default());
        }
        Self::load(path)
    }

    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = fs::read_to_string(path).map_err(|source| ConfigError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let cfg: Config = toml::from_str(&text).map_err(|source| ConfigError::Toml {
            path: path.to_path_buf(),
            source,
        })?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        self.validate()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| ConfigError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let text = toml::to_string_pretty(self)
            .map_err(|err| ConfigError::Validation(format!("serialize failed: {err}")))?;
        fs::write(path, text).map_err(|source| ConfigError::Io {
            path: path.to_path_buf(),
            source,
        })
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.scan.concurrency == 0 {
            return Err(ConfigError::Validation(
                "scan.concurrency must be at least 1".into(),
            ));
        }
        if self.webhook.enabled && self.webhook.bind.is_empty() {
            return Err(ConfigError::Validation(
                "webhook.bind is required when webhooks are enabled".into(),
            ));
        }
        for ws in &self.workspaces {
            if ws.name.trim().is_empty() {
                return Err(ConfigError::Validation(
                    "workspace name must not be empty".into(),
                ));
            }
        }
        Ok(())
    }

    pub fn tags_for(&self, path: &Path) -> Vec<String> {
        let mut tags = Vec::new();
        for assignment in &self.tags {
            if path.starts_with(&assignment.path) || path == assignment.path {
                tags.extend(assignment.tags.iter().cloned());
            }
        }
        for repo in &self.repos {
            if repo.path == path {
                tags.extend(repo.tags.iter().cloned());
            }
        }
        tags.sort();
        tags.dedup();
        tags
    }

    pub fn expanded_roots(&self) -> Vec<PathBuf> {
        self.roots.iter().map(|p| expand_path(p)).collect()
    }

    pub fn expanded_repos(&self) -> Vec<RegisteredRepo> {
        self.repos
            .iter()
            .map(|r| RegisteredRepo {
                path: expand_path(&r.path),
                tags: r.tags.clone(),
                name: r.name.clone(),
            })
            .collect()
    }

    pub fn workspaces_for(
        &self,
        path: &Path,
        provider: ProviderKind,
        tags: &[String],
    ) -> Vec<String> {
        self.workspaces
            .iter()
            .filter(|ws| {
                let tag_ok =
                    ws.match_tags.is_empty() || ws.match_tags.iter().any(|t| tags.contains(t));
                let root_ok =
                    ws.match_roots.is_empty() || ws.match_roots.iter().any(|r| path.starts_with(r));
                let provider_ok =
                    ws.match_providers.is_empty() || ws.match_providers.contains(&provider);
                tag_ok && root_ok && provider_ok
            })
            .map(|ws| ws.name.clone())
            .collect()
    }
}

pub fn expand_path(path: &Path) -> PathBuf {
    let raw = path.to_string_lossy();
    if let Some(stripped) = raw.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(stripped);
        }
    }
    if raw == "~"
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home);
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_round_trips() {
        let cfg = Config::default();
        let text = toml::to_string_pretty(&cfg).unwrap();
        let parsed: Config = toml::from_str(&text).unwrap();
        assert_eq!(cfg, parsed);
    }

    #[test]
    fn rejects_zero_concurrency() {
        let cfg = Config {
            scan: ScanConfig { concurrency: 0 },
            ..Config::default()
        };
        assert!(cfg.validate().is_err());
    }
}
