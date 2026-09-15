//! Credential resolution. Tokens are never written to the metadata cache.

use std::process::Command;

use gfc_config::{AuthBackend, AuthConfig};
use gfc_schema::ProviderKind;

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("no credential found for {provider:?}")]
    NotFound { provider: ProviderKind },
    #[error("credential command failed: {0}")]
    Command(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialSource {
    ProviderCli,
    SecretService,
    Env,
    Command,
}

impl CredentialSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ProviderCli => "provider_cli",
            Self::SecretService => "secret_service",
            Self::Env => "env",
            Self::Command => "command",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Credential {
    pub token: String,
    pub source: CredentialSource,
    pub label: String,
}

pub fn resolve(provider: ProviderKind, auth: &AuthConfig) -> Result<Credential, AuthError> {
    for backend in &auth.order {
        if let Some(cred) = resolve_backend(provider, *backend, auth) {
            tracing::info!(
                provider = provider.as_str(),
                source = cred.source.as_str(),
                label = %cred.label,
                "resolved credential"
            );
            return Ok(cred);
        }
    }
    Err(AuthError::NotFound { provider })
}

fn resolve_backend(
    provider: ProviderKind,
    backend: AuthBackend,
    auth: &AuthConfig,
) -> Option<Credential> {
    match backend {
        AuthBackend::ProviderCli => from_cli(provider),
        AuthBackend::SecretService => from_secret_tool(provider),
        AuthBackend::Env => from_env(provider),
        AuthBackend::Command => from_command(provider, auth),
    }
}

fn from_cli(provider: ProviderKind) -> Option<Credential> {
    let (bin, args): (&str, &[&str]) = match provider {
        ProviderKind::Github => ("gh", &["auth", "token"]),
        ProviderKind::Gitlab => ("glab", &["auth", "token"]),
        ProviderKind::Origin => ("origin", &["auth", "token"]),
        ProviderKind::Git => return None,
    };
    let output = Command::new(bin).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if token.is_empty() {
        return None;
    }
    Some(Credential {
        token,
        source: CredentialSource::ProviderCli,
        label: format!("{bin} auth token"),
    })
}

fn from_secret_tool(provider: ProviderKind) -> Option<Credential> {
    let key = provider_key(provider)?;
    let output = Command::new("secret-tool")
        .args(["lookup", "service", "gfc", "provider", key])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if token.is_empty() {
        return None;
    }
    Some(Credential {
        token,
        source: CredentialSource::SecretService,
        label: format!("secret-service:{key}"),
    })
}

fn from_env(provider: ProviderKind) -> Option<Credential> {
    let names = env_names(provider);
    for name in names {
        if let Ok(token) = std::env::var(name) {
            let token = token.trim().to_string();
            if !token.is_empty() {
                return Some(Credential {
                    token,
                    source: CredentialSource::Env,
                    label: format!("env:{name}"),
                });
            }
        }
    }
    None
}

fn from_command(provider: ProviderKind, _auth: &AuthConfig) -> Option<Credential> {
    let var = match provider {
        ProviderKind::Github => "GFC_GITHUB_CREDENTIAL_COMMAND",
        ProviderKind::Gitlab => "GFC_GITLAB_CREDENTIAL_COMMAND",
        ProviderKind::Origin => "GFC_ORIGIN_CREDENTIAL_COMMAND",
        ProviderKind::Git => return None,
    };
    let cmdline = std::env::var(var).ok()?;
    let output = Command::new("sh")
        .arg("-c")
        .arg(&cmdline)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if token.is_empty() {
        return None;
    }
    Some(Credential {
        token,
        source: CredentialSource::Command,
        label: format!("command:{var}"),
    })
}

fn provider_key(provider: ProviderKind) -> Option<&'static str> {
    match provider {
        ProviderKind::Github => Some("github"),
        ProviderKind::Gitlab => Some("gitlab"),
        ProviderKind::Origin => Some("origin"),
        ProviderKind::Git => None,
    }
}

fn env_names(provider: ProviderKind) -> &'static [&'static str] {
    match provider {
        ProviderKind::Github => &["GFC_GITHUB_TOKEN", "GITHUB_TOKEN", "GH_TOKEN"],
        ProviderKind::Gitlab => &["GFC_GITLAB_TOKEN", "GITLAB_TOKEN", "GLAB_TOKEN"],
        ProviderKind::Origin => &["GFC_ORIGIN_TOKEN", "CURSOR_ORIGIN_TOKEN"],
        ProviderKind::Git => &[],
    }
}

/// Observability record: source label only, never the secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthObservation {
    pub provider: String,
    pub source: String,
    pub label: String,
}

pub fn observe(provider: ProviderKind, cred: &Credential) -> AuthObservation {
    AuthObservation {
        provider: provider.as_str().into(),
        source: cred.source.as_str().into(),
        label: cred.label.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gfc_config::AuthConfig;

    #[test]
    fn env_backend_reads_token() {
        unsafe {
            std::env::set_var("GFC_GITHUB_TOKEN", "ghs_test_not_a_real_token");
        }
        let cred = from_env(ProviderKind::Github).expect("env cred");
        assert_eq!(cred.source, CredentialSource::Env);
        assert_eq!(cred.label, "env:GFC_GITHUB_TOKEN");
        unsafe {
            std::env::remove_var("GFC_GITHUB_TOKEN");
        }
        let _cfg = AuthConfig::default();
    }
}
