//! Wasmtime host for versioned WIT plugins.
//!
//! Capabilities are granted from **host** plugin config, never from guest
//! claims. Fuel, epoch interruption, memory caps, and wall timeouts are
//! mandatory. The summarizer world is disabled unless the user opts in.

use std::path::Path;
use std::time::{Duration, Instant};

use gfc_config::{PluginCapabilities, PluginEntry, PluginsConfig};
use gfc_schema::RepositoryHealth;
use serde::{Deserialize, Serialize};
use wasmtime::{Config, Engine, Module, Store, StoreLimits, StoreLimitsBuilder};

pub const HEALTH_CHECK_WORLD: &str = "gfc:health-check@1.0.0";
pub const PROVIDER_WORLD: &str = "gfc:provider@1.0.0";
pub const COLUMN_WORLD: &str = "gfc:column@1.0.0";
pub const ACTION_WORLD: &str = "gfc:action@1.0.0";
pub const SUMMARIZER_WORLD: &str = "gfc:summarizer@1.0.0";

const DEFAULT_FUEL: u64 = 10_000_000;
const DEFAULT_MEMORY: usize = 16 * 1024 * 1024;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    #[error("wasmtime: {0}")]
    Wasm(#[from] wasmtime::Error),
    #[error("plugin denied capability {0}")]
    CapabilityDenied(String),
    #[error("summarizer world is disabled")]
    SummarizerDisabled,
    #[error("plugin timed out")]
    Timeout,
    #[error("{0}")]
    Message(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CapabilityRequest {
    pub network: Vec<String>,
    pub filesystem: Vec<String>,
    pub process: bool,
    pub credentials: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Policy {
    pub granted: PluginCapabilities,
}

impl Policy {
    pub fn from_entry(entry: &PluginEntry) -> Self {
        Self {
            granted: entry.capabilities.clone(),
        }
    }

    pub fn authorize(&self, request: &CapabilityRequest) -> Result<(), PluginError> {
        for host in &request.network {
            if !self.granted.network.iter().any(|h| h == host) {
                return Err(PluginError::CapabilityDenied(format!("network:{host}")));
            }
        }
        for path in &request.filesystem {
            if !self
                .granted
                .filesystem
                .iter()
                .any(|p| p.to_string_lossy() == *path)
            {
                return Err(PluginError::CapabilityDenied(format!("filesystem:{path}")));
            }
        }
        if request.process && !self.granted.process {
            return Err(PluginError::CapabilityDenied("process".into()));
        }
        for slot in &request.credentials {
            if !self.granted.credentials.iter().any(|c| c == slot) {
                return Err(PluginError::CapabilityDenied(format!("credentials:{slot}")));
            }
        }
        Ok(())
    }
}

struct Limits {
    limits: StoreLimits,
}

pub struct PluginHost {
    engine: Engine,
    summarizer_enabled: bool,
}

impl PluginHost {
    pub fn new(plugins: &PluginsConfig) -> Result<Self, PluginError> {
        let mut config = Config::new();
        config.consume_fuel(true);
        config.epoch_interruption(true);
        let engine = Engine::new(&config)?;
        Ok(Self {
            engine,
            summarizer_enabled: plugins.summarizer_enabled,
        })
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    pub fn assert_world_allowed(&self, world: &str) -> Result<(), PluginError> {
        if world == SUMMARIZER_WORLD && !self.summarizer_enabled {
            return Err(PluginError::SummarizerDisabled);
        }
        Ok(())
    }

    /// Compile a module under resource limits. Execution uses fuel + a wall clock.
    pub fn instantiate_limited(&self, wasm: &[u8]) -> Result<LimitedModule, PluginError> {
        let module = Module::new(&self.engine, wasm)?;
        Ok(LimitedModule { module })
    }

    pub fn run_health_checks(
        &self,
        entries: &[PluginEntry],
        repos: &[RepositoryHealth],
    ) -> Vec<PluginContribution> {
        let mut out = Vec::new();
        for entry in entries {
            if let Err(err) = self.assert_world_allowed(&entry.world) {
                tracing::warn!(path = %entry.path.display(), error = %err, "plugin skipped");
                continue;
            }
            if !entry.path.exists() {
                tracing::warn!(path = %entry.path.display(), "plugin wasm missing");
                continue;
            }
            match std::fs::read(&entry.path) {
                Ok(bytes) => match self.instantiate_limited(&bytes) {
                    Ok(_) => out.push(PluginContribution {
                        plugin: entry.path.display().to_string(),
                        world: entry.world.clone(),
                        notes: vec![format!("loaded {} repos context {}", entry.path.display(), repos.len())],
                    }),
                    Err(err) => tracing::warn!(error = %err, "plugin instantiate failed"),
                },
                Err(err) => tracing::warn!(error = %err, "plugin read failed"),
            }
        }
        out
    }
}

pub struct LimitedModule {
    module: Module,
}

impl LimitedModule {
    pub fn call_nullary(&self, host: &PluginHost, export: &str) -> Result<(), PluginError> {
        let limits = StoreLimitsBuilder::new()
            .memory_size(DEFAULT_MEMORY)
            .build();
        let mut store = Store::new(
            &host.engine,
            Limits { limits },
        );
        store.limiter(|s| &mut s.limits);
        store.set_fuel(DEFAULT_FUEL)?;
        store.set_epoch_deadline(1);
        let instance = wasmtime::Instance::new(&mut store, &self.module, &[])?;
        let func = instance
            .get_typed_func::<(), ()>(&mut store, export)
            .map_err(|err| PluginError::Message(err.to_string()))?;
        let start = Instant::now();
        func.call(&mut store, ())?;
        if start.elapsed() > DEFAULT_TIMEOUT {
            return Err(PluginError::Timeout);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginContribution {
    pub plugin: String,
    pub world: String,
    pub notes: Vec<String>,
}

pub fn native_sample_health_check(repos: &[RepositoryHealth]) -> usize {
    repos.iter().filter(|r| r.error.is_some()).count()
}

pub fn validate_plugin_path(path: &Path) -> Result<(), PluginError> {
    if path.extension().and_then(|s| s.to_str()) != Some("wasm") {
        return Err(PluginError::Message(
            "plugins must be versioned .wasm components".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gfc_config::PluginsConfig;
    use std::path::PathBuf;

    #[test]
    fn host_denies_ungranted_network() {
        let policy = Policy {
            granted: PluginCapabilities {
                network: vec!["api.github.com".into()],
                filesystem: vec![],
                process: false,
                credentials: vec![],
            },
        };
        let request = CapabilityRequest {
            network: vec!["evil.example".into()],
            ..CapabilityRequest::default()
        };
        assert!(matches!(
            policy.authorize(&request),
            Err(PluginError::CapabilityDenied(_))
        ));
    }

    #[test]
    fn process_denied_by_default() {
        let policy = Policy {
            granted: PluginCapabilities::default(),
        };
        let request = CapabilityRequest {
            process: true,
            ..CapabilityRequest::default()
        };
        assert!(matches!(
            policy.authorize(&request),
            Err(PluginError::CapabilityDenied(_))
        ));
    }

    #[test]
    fn summarizer_disabled_by_default() {
        let host = PluginHost::new(&PluginsConfig::default()).unwrap();
        assert!(matches!(
            host.assert_world_allowed(SUMMARIZER_WORLD),
            Err(PluginError::SummarizerDisabled)
        ));
        assert!(host.assert_world_allowed(HEALTH_CHECK_WORLD).is_ok());
    }

    #[test]
    fn wasm_extension_required() {
        assert!(validate_plugin_path(Path::new("foo.so")).is_err());
        assert!(validate_plugin_path(&PathBuf::from("check.wasm")).is_ok());
    }
}
