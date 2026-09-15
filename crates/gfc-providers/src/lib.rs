mod ci;
mod webhook;

pub use ci::{RawCheck, map_gitlab_pipeline, normalize_checks};
pub use webhook::{WebhookError, verify_github_hmac, verify_gitlab_token, verify_origin_ed25519};

use chrono::Utc;
use gfc_config::Config;
use gfc_schema::{
    CiEvidence, CiSignal, CiState, ConnectionEvidence, ConnectionSignal, ConnectionState,
    ProviderKind, RemoteHealth,
};
use serde::Deserialize;

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("{0}")]
    Message(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteRepo {
    pub full_name: String,
    pub clone_url: String,
    pub web_url: String,
    pub provider: ProviderKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubUser {
    pub login: String,
    pub name: Option<String>,
    pub avatar_url: String,
}

impl GithubUser {
    pub fn display_name(&self) -> &str {
        match self.name.as_deref() {
            Some(name) if !name.trim().is_empty() => name,
            _ => self.login.as_str(),
        }
    }

    pub fn avatar_filename(login: &str) -> String {
        let safe: String = login
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        format!("{safe}.img")
    }
}

#[derive(Debug, Clone)]
pub struct ForgeClient {
    http: reqwest::Client,
    github_api: String,
    gitlab_api: String,
    origin_api: String,
}

impl ForgeClient {
    pub fn from_config(config: &Config) -> Self {
        Self {
            http: reqwest::Client::builder()
                .user_agent("gfc/0.1")
                .build()
                .expect("reqwest client"),
            github_api: config.providers.github.api_url.clone(),
            gitlab_api: config.providers.gitlab.api_url.clone(),
            origin_api: config.providers.origin.api_url.clone(),
        }
    }

    pub fn with_urls(github_api: String, gitlab_api: String, origin_api: String) -> Self {
        Self {
            http: reqwest::Client::builder()
                .user_agent("gfc/0.1")
                .build()
                .expect("reqwest client"),
            github_api,
            gitlab_api,
            origin_api,
        }
    }

    pub async fn github_remote_health(
        &self,
        token: &str,
        owner: &str,
        repo: &str,
        sha: &str,
    ) -> Result<RemoteHealth, ProviderError> {
        let now = Utc::now();
        let checks_url = format!(
            "{}/repos/{owner}/{repo}/commits/{sha}/check-runs",
            self.github_api
        );
        let status_url = format!(
            "{}/repos/{owner}/{repo}/commits/{sha}/status",
            self.github_api
        );
        let checks_resp = self
            .http
            .get(&checks_url)
            .bearer_auth(token)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await?;
        let status_resp = self
            .http
            .get(&status_url)
            .bearer_auth(token)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await?;
        if checks_resp.status().as_u16() == 401 || status_resp.status().as_u16() == 401 {
            return Ok(invalid_remote(
                now,
                Some(401),
                "github credentials rejected",
            ));
        }
        if !checks_resp.status().is_success() {
            return Ok(unavailable(
                now,
                Some(checks_resp.status().as_u16()),
                "github check-runs request failed",
            ));
        }
        let checks_body: GitHubCheckRuns = checks_resp.json().await?;
        let mut raw: Vec<RawCheck> = checks_body
            .check_runs
            .into_iter()
            .map(|c| RawCheck {
                name: c.name,
                status: c.status,
                conclusion: c.conclusion,
            })
            .collect();
        if status_resp.status().is_success() {
            let combined: GitHubCombinedStatus = status_resp
                .json()
                .await
                .unwrap_or(GitHubCombinedStatus { statuses: vec![] });
            for st in combined.statuses {
                raw.push(RawCheck {
                    name: st.context,
                    status: st.state.clone(),
                    conclusion: Some(st.state),
                });
            }
        }
        let (state, ev) = normalize_checks(&raw);
        Ok(RemoteHealth {
            connection: ConnectionSignal {
                state: ConnectionState::Ok,
                evidence: ConnectionEvidence {
                    message: Some("github ok".into()),
                    http_status: Some(200),
                },
                updated_at: now,
            },
            ci: CiSignal {
                state,
                evidence: CiEvidence {
                    provider: Some("github".into()),
                    check_name: ev.as_ref().map(|e| e.name.clone()),
                    sha: Some(sha.into()),
                    url: Some(format!("https://github.com/{owner}/{repo}/commit/{sha}")),
                    conclusion: ev.and_then(|e| e.conclusion),
                },
                updated_at: now,
            },
        })
    }

    pub async fn gitlab_remote_health(
        &self,
        token: &str,
        project: &str,
        r#ref: &str,
    ) -> Result<RemoteHealth, ProviderError> {
        let now = Utc::now();
        let encoded = urlencoding_minimal(project);
        let url = format!(
            "{}/projects/{encoded}/pipelines/latest?ref={ref}",
            self.gitlab_api
        );
        let resp = self
            .http
            .get(&url)
            .header("PRIVATE-TOKEN", token)
            .send()
            .await?;
        let code = resp.status().as_u16();
        if code == 401 {
            return Ok(invalid_remote(
                now,
                Some(401),
                "gitlab credentials rejected",
            ));
        }
        if code == 404 {
            return Ok(RemoteHealth {
                connection: ConnectionSignal {
                    state: ConnectionState::Ok,
                    evidence: ConnectionEvidence {
                        message: Some("no pipeline".into()),
                        http_status: Some(404),
                    },
                    updated_at: now,
                },
                ci: CiSignal {
                    state: CiState::Unknown,
                    evidence: CiEvidence {
                        provider: Some("gitlab".into()),
                        check_name: None,
                        sha: None,
                        url: None,
                        conclusion: None,
                    },
                    updated_at: now,
                },
            });
        }
        if !resp.status().is_success() {
            return Ok(unavailable(
                now,
                Some(code),
                "gitlab pipeline request failed",
            ));
        }
        let pipeline: GitLabPipeline = resp.json().await?;
        let state = map_gitlab_pipeline(&pipeline.status);
        Ok(RemoteHealth {
            connection: ConnectionSignal {
                state: ConnectionState::Ok,
                evidence: ConnectionEvidence {
                    message: Some("gitlab ok".into()),
                    http_status: Some(200),
                },
                updated_at: now,
            },
            ci: CiSignal {
                state,
                evidence: CiEvidence {
                    provider: Some("gitlab".into()),
                    check_name: Some(format!("pipeline #{}", pipeline.id)),
                    sha: pipeline.sha,
                    url: pipeline.web_url,
                    conclusion: Some(pipeline.status),
                },
                updated_at: now,
            },
        })
    }

    pub async fn origin_remote_health(
        &self,
        token: Option<&str>,
        owner: &str,
        repo: &str,
        sha: &str,
        app_configured: bool,
    ) -> Result<RemoteHealth, ProviderError> {
        let now = Utc::now();
        if !app_configured || token.is_none() {
            return Ok(RemoteHealth {
                connection: ConnectionSignal {
                    state: ConnectionState::Ok,
                    evidence: ConnectionEvidence {
                        message: Some(
                            "origin generic git adapter; CI unsupported without app credentials"
                                .into(),
                        ),
                        http_status: None,
                    },
                    updated_at: now,
                },
                ci: CiSignal {
                    state: CiState::Unsupported,
                    evidence: CiEvidence {
                        provider: Some("origin".into()),
                        check_name: None,
                        sha: Some(sha.into()),
                        url: Some(format!("https://cursor.com/codebase/{owner}/{repo}")),
                        conclusion: None,
                    },
                    updated_at: now,
                },
            });
        }
        let token = token.unwrap();
        let url = format!(
            "{}/repos/{owner}/{repo}/commits/{sha}/check-runs",
            self.origin_api
        );
        let resp = self.http.get(&url).bearer_auth(token).send().await?;
        let code = resp.status().as_u16();
        if code == 401 || code == 403 {
            return Ok(invalid_remote(now, Some(code), "origin app token rejected"));
        }
        if !resp.status().is_success() {
            return Ok(unavailable(
                now,
                Some(code),
                "origin check-runs request failed",
            ));
        }
        let body: OriginCheckRuns = resp.json().await?;
        let raw: Vec<RawCheck> = body
            .check_runs
            .into_iter()
            .map(|c| RawCheck {
                name: c.name.unwrap_or_else(|| "check".into()),
                status: c.status.unwrap_or_else(|| "completed".into()),
                conclusion: c.conclusion,
            })
            .collect();
        let (state, ev) = normalize_checks(&raw);
        Ok(RemoteHealth {
            connection: ConnectionSignal {
                state: ConnectionState::Ok,
                evidence: ConnectionEvidence {
                    message: Some("origin app ok".into()),
                    http_status: Some(200),
                },
                updated_at: now,
            },
            ci: CiSignal {
                state,
                evidence: CiEvidence {
                    provider: Some("origin".into()),
                    check_name: ev.as_ref().map(|e| e.name.clone()),
                    sha: Some(sha.into()),
                    url: Some(format!("https://cursor.com/codebase/{owner}/{repo}")),
                    conclusion: ev.and_then(|e| e.conclusion),
                },
                updated_at: now,
            },
        })
    }

    pub async fn github_list_repos(&self, token: &str) -> Result<Vec<RemoteRepo>, ProviderError> {
        let url = format!("{}/user/repos?per_page=100", self.github_api);
        let resp = self
            .http
            .get(url)
            .bearer_auth(token)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(ProviderError::Message(format!(
                "github list failed: {}",
                resp.status()
            )));
        }
        let rows: Vec<GitHubRepo> = resp.json().await?;
        Ok(rows
            .into_iter()
            .map(|r| RemoteRepo {
                full_name: r.full_name,
                clone_url: r.clone_url.unwrap_or_default(),
                web_url: r.html_url.unwrap_or_default(),
                provider: ProviderKind::Github,
            })
            .collect())
    }

    pub async fn github_user(&self, token: &str) -> Result<GithubUser, ProviderError> {
        let url = format!("{}/user", self.github_api);
        let resp = self
            .http
            .get(url)
            .bearer_auth(token)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(ProviderError::Message(format!(
                "github user failed: {}",
                resp.status()
            )));
        }
        let raw: GitHubUserRaw = resp.json().await?;
        Ok(GithubUser {
            login: raw.login,
            name: raw.name.filter(|n| !n.trim().is_empty()),
            avatar_url: sized_avatar_url(&raw.avatar_url),
        })
    }

    pub async fn fetch_bytes(&self, url: &str) -> Result<Vec<u8>, ProviderError> {
        let resp = self.http.get(url).send().await?;
        if !resp.status().is_success() {
            return Err(ProviderError::Message(format!(
                "download failed: {}",
                resp.status()
            )));
        }
        Ok(resp.bytes().await?.to_vec())
    }

    pub async fn gitlab_list_repos(&self, token: &str) -> Result<Vec<RemoteRepo>, ProviderError> {
        let url = format!(
            "{}/projects?membership=true&simple=true&per_page=100",
            self.gitlab_api
        );
        let resp = self
            .http
            .get(url)
            .header("PRIVATE-TOKEN", token)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(ProviderError::Message(format!(
                "gitlab list failed: {}",
                resp.status()
            )));
        }
        let rows: Vec<GitLabProject> = resp.json().await?;
        Ok(rows
            .into_iter()
            .map(|r| RemoteRepo {
                full_name: r.path_with_namespace,
                clone_url: r.http_url_to_repo.unwrap_or_default(),
                web_url: r.web_url.unwrap_or_default(),
                provider: ProviderKind::Gitlab,
            })
            .collect())
    }
}

fn invalid_remote(now: chrono::DateTime<Utc>, status: Option<u16>, msg: &str) -> RemoteHealth {
    RemoteHealth {
        connection: ConnectionSignal {
            state: ConnectionState::Invalid,
            evidence: ConnectionEvidence {
                message: Some(msg.into()),
                http_status: status,
            },
            updated_at: now,
        },
        ci: CiSignal {
            state: CiState::Unknown,
            evidence: CiEvidence {
                provider: None,
                check_name: None,
                sha: None,
                url: None,
                conclusion: None,
            },
            updated_at: now,
        },
    }
}

fn unavailable(now: chrono::DateTime<Utc>, status: Option<u16>, msg: &str) -> RemoteHealth {
    RemoteHealth {
        connection: ConnectionSignal {
            state: ConnectionState::Unavailable,
            evidence: ConnectionEvidence {
                message: Some(msg.into()),
                http_status: status,
            },
            updated_at: now,
        },
        ci: CiSignal {
            state: CiState::Unknown,
            evidence: CiEvidence {
                provider: None,
                check_name: None,
                sha: None,
                url: None,
                conclusion: None,
            },
            updated_at: now,
        },
    }
}

fn urlencoding_minimal(s: &str) -> String {
    s.replace('/', "%2F")
}

fn sized_avatar_url(url: &str) -> String {
    if url.contains("s=") {
        url.to_string()
    } else if url.contains('?') {
        format!("{url}&s=80")
    } else {
        format!("{url}?s=80")
    }
}

pub fn parse_owner_repo(url: &str) -> Option<(String, String)> {
    let trimmed = url.trim().trim_end_matches(".git");
    let rest = if let Some(r) = trimmed.strip_prefix("git@") {
        r.split_once(':')?.1
    } else if let Some(idx) = trimmed.find("://") {
        let after = &trimmed[idx + 3..];
        after.split_once('/')?.1
    } else {
        trimmed
    };
    let mut parts = rest.split('/');
    let owner = parts.next()?.to_string();
    let repo = parts.next()?.to_string();
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some((owner, repo))
}

#[derive(Debug, Deserialize)]
struct GitHubCheckRuns {
    #[serde(default)]
    check_runs: Vec<GitHubCheckRun>,
}

#[derive(Debug, Deserialize)]
struct GitHubCheckRun {
    name: String,
    status: String,
    conclusion: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitHubCombinedStatus {
    #[serde(default)]
    statuses: Vec<GitHubStatus>,
}

#[derive(Debug, Deserialize)]
struct GitHubStatus {
    state: String,
    context: String,
}

#[derive(Debug, Deserialize)]
struct GitHubRepo {
    full_name: String,
    clone_url: Option<String>,
    html_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitHubUserRaw {
    login: String,
    name: Option<String>,
    avatar_url: String,
}

#[derive(Debug, Deserialize)]
struct GitLabPipeline {
    id: u64,
    status: String,
    sha: Option<String>,
    web_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitLabProject {
    path_with_namespace: String,
    http_url_to_repo: Option<String>,
    web_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OriginCheckRuns {
    #[serde(default)]
    check_runs: Vec<OriginCheckRun>,
}

#[derive(Debug, Deserialize)]
struct OriginCheckRun {
    name: Option<String>,
    status: Option<String>,
    conclusion: Option<String>,
}

#[cfg(test)]
mod parse_tests {
    use super::*;

    #[test]
    fn parses_ssh_and_https() {
        assert_eq!(
            parse_owner_repo("git@github.com:acme/app.git").unwrap(),
            ("acme".into(), "app".into())
        );
        assert_eq!(
            parse_owner_repo("https://gitlab.example.com/acme/app.git").unwrap(),
            ("acme".into(), "app".into())
        );
    }
}
