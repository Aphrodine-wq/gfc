use std::path::{Path, PathBuf};

use gfc_auth::resolve;
use gfc_config::{Config, Paths};
use gfc_providers::{ForgeClient, GithubUser};
use gfc_schema::ProviderKind;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubIdentity {
    pub login: String,
    pub display_name: String,
    pub avatar_path: Option<PathBuf>,
}

impl GithubIdentity {
    pub fn from_auth_status(value: &Value) -> Option<Self> {
        let gh = value.get("github")?;
        if gh.is_null() {
            return None;
        }
        let login = gh.get("login")?.as_str()?.trim().to_string();
        if login.is_empty() {
            return None;
        }
        let display_name = gh
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(login.as_str())
            .to_string();
        let avatar_path = gh
            .get("avatar_path")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(PathBuf::from);
        Some(Self {
            login,
            display_name,
            avatar_path,
        })
    }
}

pub async fn fetch_github_identity(config: &Config, paths: &Paths) -> Option<GithubIdentity> {
    if !config.providers.github.enabled {
        return None;
    }
    let cred = resolve(ProviderKind::Github, &config.auth).ok()?;
    let client = ForgeClient::from_config(config);
    let user = client.github_user(&cred.token).await.ok()?;
    let _ = std::fs::create_dir_all(&paths.avatar_dir);
    let avatar_path = paths
        .avatar_dir
        .join(GithubUser::avatar_filename(&user.login));
    let stored = if let Ok(bytes) = client.fetch_bytes(&user.avatar_url).await {
        std::fs::write(&avatar_path, bytes)
            .ok()
            .map(|_| avatar_path)
    } else {
        None
    };
    Some(GithubIdentity {
        login: user.login.clone(),
        display_name: user.display_name().to_string(),
        avatar_path: stored,
    })
}

pub fn load_avatar_image(path: &Path) -> Option<image::DynamicImage> {
    image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_github_identity_from_auth_status() {
        let value = json!({
            "github": {
                "login": "octocat",
                "name": "The Octocat",
                "avatar_path": "/tmp/octocat.img"
            }
        });
        let id = GithubIdentity::from_auth_status(&value).unwrap();
        assert_eq!(id.login, "octocat");
        assert_eq!(id.display_name, "The Octocat");
        assert_eq!(
            id.avatar_path.as_deref(),
            Some(Path::new("/tmp/octocat.img"))
        );
    }

    #[test]
    fn missing_or_null_github_is_unsigned() {
        assert!(GithubIdentity::from_auth_status(&json!({})).is_none());
        assert!(GithubIdentity::from_auth_status(&json!({"github": null})).is_none());
        assert!(GithubIdentity::from_auth_status(&json!({"github": {"login": ""}})).is_none());
    }

    #[test]
    fn empty_name_falls_back_to_login() {
        let value = json!({"github": {"login": "octocat", "name": "  "}});
        let id = GithubIdentity::from_auth_status(&value).unwrap();
        assert_eq!(id.display_name, "octocat");
        assert!(id.avatar_path.is_none());
    }
}
