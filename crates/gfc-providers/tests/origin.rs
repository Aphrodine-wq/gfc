use ed25519_dalek::{Signer, SigningKey};
use gfc_providers::{ForgeClient, WebhookError, verify_github_hmac, verify_origin_ed25519};
use gfc_schema::CiState;
use sha2::{Digest, Sha256};

#[tokio::test]
async fn origin_generic_adapter_marks_ci_unsupported() {
    let client = ForgeClient::with_urls(
        "http://github.test".into(),
        "http://gitlab.test".into(),
        "http://origin.test".into(),
    );
    let health = client
        .origin_remote_health(None, "acme", "app", "abc", false)
        .await
        .unwrap();
    assert_eq!(health.ci.state, CiState::Unsupported);
    assert!(
        health
            .connection
            .evidence
            .message
            .as_deref()
            .unwrap()
            .contains("generic git")
    );
}

#[tokio::test]
async fn origin_app_path_reads_check_runs() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path(
            "/repos/acme/app/commits/abc/check-runs",
        ))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "check_runs": [
                    {"name": "depot", "status": "completed", "conclusion": "success"}
                ]
            })),
        )
        .mount(&server)
        .await;
    let client = ForgeClient::with_urls(
        "http://github.test".into(),
        "http://gitlab.test".into(),
        server.uri(),
    );
    let health = client
        .origin_remote_health(Some("oit_test"), "acme", "app", "abc", true)
        .await
        .unwrap();
    assert_eq!(health.ci.state, CiState::Passing);
}

#[test]
fn origin_webhook_rejects_bad_signature() {
    let signing = SigningKey::from_bytes(&[3u8; 32]);
    let vk = signing.verifying_key();
    let err = verify_origin_ed25519(
        b"{}",
        Some("id"),
        Some("1700000000"),
        Some("v1ed,AAAA"),
        vk.as_bytes(),
        1_700_000_000,
    )
    .unwrap_err();
    assert_eq!(err, WebhookError::InvalidSignature);
}

#[test]
fn origin_webhook_accepts_signed_body() {
    let signing = SigningKey::from_bytes(&[3u8; 32]);
    let vk = signing.verifying_key();
    let body = b"{\"event\":\"repository.pushed\"}";
    let id = "wh_1";
    let ts = "1700000000";
    let mut hasher = Sha256::new();
    hasher.update(id.as_bytes());
    hasher.update(b".");
    hasher.update(ts.as_bytes());
    hasher.update(b".");
    hasher.update(body);
    let digest_hex = hex::encode(hasher.finalize());
    let sig = signing.sign(digest_hex.as_bytes());
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, sig.to_bytes());
    let header = format!("v1ed,{b64}");
    assert!(
        verify_origin_ed25519(
            body,
            Some(id),
            Some(ts),
            Some(&header),
            vk.as_bytes(),
            1_700_000_000,
        )
        .is_ok()
    );
}

#[test]
fn github_hmac_still_required() {
    assert!(verify_github_hmac("s", b"{}", None).is_err());
}
