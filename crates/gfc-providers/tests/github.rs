use gfc_providers::{ForgeClient, parse_owner_repo};
use gfc_schema::{CiState, ConnectionState};
use serde_json::json;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn github_failure_and_pending_normalize() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/app/commits/abc/check-runs"))
        .and(header("Authorization", "Bearer tok"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "check_runs": [
                {"name": "build", "status": "completed", "conclusion": "failure"}
            ]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/app/commits/abc/status"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "statuses": []
        })))
        .mount(&server)
        .await;

    let client = ForgeClient::with_urls(
        server.uri(),
        "http://gitlab.test".into(),
        "http://origin.test".into(),
    );
    let health = client
        .github_remote_health("tok", "acme", "app", "abc")
        .await
        .unwrap();
    assert_eq!(health.connection.state, ConnectionState::Ok);
    assert_eq!(health.ci.state, CiState::Failing);
}

#[tokio::test]
async fn github_provider_failure_is_unavailable_not_panic() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    let client = ForgeClient::with_urls(
        server.uri(),
        "http://gitlab.test".into(),
        "http://origin.test".into(),
    );
    let health = client
        .github_remote_health("tok", "acme", "app", "abc")
        .await
        .unwrap();
    assert_eq!(health.connection.state, ConnectionState::Unavailable);
}

#[tokio::test]
async fn gitlab_latest_pipeline() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/projects/acme%2Fapp/pipelines/latest"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 9,
            "status": "running",
            "sha": "abc",
            "web_url": "https://gitlab.example/p"
        })))
        .mount(&server)
        .await;
    let client = ForgeClient::with_urls(
        "http://github.test".into(),
        server.uri(),
        "http://origin.test".into(),
    );
    let health = client
        .gitlab_remote_health("tok", "acme/app", "main")
        .await
        .unwrap();
    assert_eq!(health.ci.state, CiState::Pending);
}

#[test]
fn parse_origin_remote() {
    let (o, r) = parse_owner_repo("https://origin.cursor.com/acme/app.git").unwrap();
    assert_eq!((o, r), ("acme".into(), "app".into()));
}

#[tokio::test]
async fn github_user_prefers_profile_name() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .and(header("Authorization", "Bearer tok"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "login": "octocat",
            "name": "The Octocat",
            "avatar_url": "http://avatars.test/octocat"
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/octocat"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"png"))
        .mount(&server)
        .await;

    let client = ForgeClient::with_urls(
        server.uri(),
        "http://gitlab.test".into(),
        "http://origin.test".into(),
    );
    let user = client.github_user("tok").await.unwrap();
    assert_eq!(user.login, "octocat");
    assert_eq!(user.display_name(), "The Octocat");
    assert!(user.avatar_url.ends_with("s=80") || user.avatar_url.contains("s=80"));
    let bytes = client
        .fetch_bytes(&format!("{}/octocat", server.uri()))
        .await
        .unwrap();
    assert_eq!(bytes, b"png");
}

#[tokio::test]
async fn github_user_falls_back_to_login() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "login": "octocat",
            "name": "  ",
            "avatar_url": "http://avatars.test/octocat?v=4"
        })))
        .mount(&server)
        .await;
    let client = ForgeClient::with_urls(
        server.uri(),
        "http://gitlab.test".into(),
        "http://origin.test".into(),
    );
    let user = client.github_user("tok").await.unwrap();
    assert_eq!(user.display_name(), "octocat");
}
