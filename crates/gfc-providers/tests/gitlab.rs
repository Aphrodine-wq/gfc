use gfc_providers::ForgeClient;
use gfc_schema::CiState;
use serde_json::json;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn gitlab_failed_pipeline() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/projects/group%2Fproj/pipelines/latest"))
        .and(header("PRIVATE-TOKEN", "glpat-test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 1,
            "status": "failed",
            "sha": "fff",
            "web_url": "https://gitlab.example/ci"
        })))
        .mount(&server)
        .await;
    let client = ForgeClient::with_urls(
        "http://github.test".into(),
        server.uri(),
        "http://origin.test".into(),
    );
    let health = client
        .gitlab_remote_health("glpat-test", "group/proj", "main")
        .await
        .unwrap();
    assert_eq!(health.ci.state, CiState::Failing);
    assert_eq!(health.ci.evidence.provider.as_deref(), Some("gitlab"));
}

#[tokio::test]
async fn gitlab_401_is_invalid_not_blocking() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    let client = ForgeClient::with_urls(
        "http://github.test".into(),
        server.uri(),
        "http://origin.test".into(),
    );
    let health = client
        .gitlab_remote_health("bad", "group/proj", "main")
        .await
        .unwrap();
    assert_eq!(
        health.connection.state,
        gfc_schema::ConnectionState::Invalid
    );
}
