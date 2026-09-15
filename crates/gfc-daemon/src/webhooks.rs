use std::collections::HashMap;
use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use gfc_config::WebhookConfig;
use gfc_providers::{verify_github_hmac, verify_gitlab_token, verify_origin_ed25519};
use tokio::sync::mpsc::Sender;
use tracing::{info, warn};

#[derive(Clone)]
pub struct HookState {
    pub github_secret: Option<String>,
    pub gitlab_token: Option<String>,
    pub origin_key: Option<[u8; 32]>,
    pub events: Sender<String>,
    pub seen: Arc<std::sync::Mutex<HashMap<String, ()>>>,
}

pub fn router(state: HookState) -> Router {
    Router::new()
        .route("/hooks/github", post(github))
        .route("/hooks/gitlab", post(gitlab))
        .route("/hooks/origin", post(origin))
        .with_state(state)
}

fn resolve_ref(spec: &Option<String>) -> Option<String> {
    let spec = spec.as_ref()?;
    if let Some(name) = spec.strip_prefix("env:") {
        return std::env::var(name).ok();
    }
    None
}

pub fn secrets_from_config(cfg: &WebhookConfig) -> (Option<String>, Option<String>) {
    (
        resolve_ref(&cfg.github_secret_ref),
        resolve_ref(&cfg.gitlab_token_ref),
    )
}

async fn github(State(state): State<HookState>, headers: HeaderMap, body: Bytes) -> StatusCode {
    let sig = headers
        .get("x-hub-signature-256")
        .and_then(|v| v.to_str().ok());
    let Some(secret) = state.github_secret.as_deref() else {
        return StatusCode::UNAUTHORIZED;
    };
    if verify_github_hmac(secret, &body, sig).is_err() {
        return StatusCode::UNAUTHORIZED;
    }
    dedup_and_fire(&state, headers.get("x-github-delivery"), "github").await
}

async fn gitlab(State(state): State<HookState>, headers: HeaderMap, body: Bytes) -> StatusCode {
    let _ = body;
    let token = headers.get("x-gitlab-token").and_then(|v| v.to_str().ok());
    let Some(expected) = state.gitlab_token.as_deref() else {
        return StatusCode::UNAUTHORIZED;
    };
    if verify_gitlab_token(expected, token).is_err() {
        return StatusCode::UNAUTHORIZED;
    }
    dedup_and_fire(&state, headers.get("x-gitlab-event-uuid"), "gitlab").await
}

async fn origin(State(state): State<HookState>, headers: HeaderMap, body: Bytes) -> StatusCode {
    let Some(key) = state.origin_key else {
        return StatusCode::UNAUTHORIZED;
    };
    let id = headers.get("webhook-id").and_then(|v| v.to_str().ok());
    let ts = headers
        .get("webhook-timestamp")
        .and_then(|v| v.to_str().ok());
    let sig = headers
        .get("webhook-signature")
        .and_then(|v| v.to_str().ok());
    let now = chrono::Utc::now().timestamp();
    if verify_origin_ed25519(&body, id, ts, sig, &key, now).is_err() {
        return StatusCode::UNAUTHORIZED;
    }
    dedup_and_fire(&state, headers.get("webhook-id"), "origin").await
}

async fn dedup_and_fire(
    state: &HookState,
    id: Option<&axum::http::HeaderValue>,
    source: &str,
) -> StatusCode {
    if let Some(id) = id.and_then(|v| v.to_str().ok()) {
        let mut seen = state.seen.lock().expect("seen");
        if seen.contains_key(id) {
            return StatusCode::ACCEPTED;
        }
        seen.insert(id.to_string(), ());
    }
    info!(source, "webhook accepted");
    let _ = state.events.try_send(source.into());
    StatusCode::ACCEPTED
}

pub async fn serve(bind: String, state: HookState) {
    let listener = match tokio::net::TcpListener::bind(&bind).await {
        Ok(l) => l,
        Err(err) => {
            warn!(error = %err, bind, "webhook listener failed");
            return;
        }
    };
    info!(bind, "webhook listener started");
    if let Err(err) = axum::serve(listener, router(state)).await {
        warn!(error = %err, "webhook server stopped");
    }
}
