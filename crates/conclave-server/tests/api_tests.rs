use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use prost::Message;
use tower::ServiceExt;

use conclave_server::{api, config, db, state};

// ── Helpers ────────────────────────────────────────────────────────

fn fake_key_package(label: &[u8]) -> Vec<u8> {
    // MLS 1.0 version (0x0001) + mls_key_package wire format (0x0005) + arbitrary payload.
    let mut data = vec![0x00, 0x01, 0x00, 0x05];
    data.extend_from_slice(label);
    data
}

fn setup() -> Router {
    let database = db::Database::open_in_memory().unwrap();
    let config = config::ServerConfig::default();
    let app_state = Arc::new(state::AppState::new(database, config));
    api::router().with_state(app_state)
}

fn setup_with_state() -> (Router, Arc<state::AppState>) {
    let database = db::Database::open_in_memory().unwrap();
    let config = config::ServerConfig::default();
    let app_state = Arc::new(state::AppState::new(database, config));
    let router = api::router().with_state(app_state.clone());
    (router, app_state)
}

async fn register_user(app: &Router, username: &str, password: &str) -> i64 {
    let req_body = conclave_proto::RegisterRequest {
        username: username.to_string(),
        password: password.to_string(),
        alias: String::new(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/register")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let resp = conclave_proto::RegisterResponse::decode(body_bytes).unwrap();
    resp.user_id
}

async fn login_user(app: &Router, username: &str, password: &str) -> String {
    let req_body = conclave_proto::LoginRequest {
        username: username.to_string(),
        password: password.to_string(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/login")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let resp = conclave_proto::LoginResponse::decode(body_bytes).unwrap();
    resp.token
}

async fn create_group_for(app: &Router, token: &str, name: &str) -> i64 {
    let req_body = conclave_proto::CreateGroupRequest {
        alias: name.to_string(),
        group_name: name.to_string(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/groups")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let resp = conclave_proto::CreateGroupResponse::decode(body_bytes).unwrap();
    resp.group_id
}

async fn upload_key_package_for(app: &Router, token: &str, data: &[u8]) {
    let req_body = conclave_proto::UploadKeyPackageRequest {
        key_package_data: data.to_vec(),
        entries: vec![],
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/key-packages")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

async fn upload_key_packages_batch(
    app: &Router,
    token: &str,
    entries: Vec<conclave_proto::KeyPackageEntry>,
) {
    let req_body = conclave_proto::UploadKeyPackageRequest {
        key_package_data: vec![],
        entries,
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/key-packages")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

// ── Registration Tests (1-5) ──────────────────────────────────────

#[tokio::test]
async fn test_register_success() {
    let app = setup();
    let user_id = register_user(&app, "alice", "password123").await;
    assert!(user_id > 0);
}

#[tokio::test]
async fn test_register_duplicate_username() {
    let app = setup();
    register_user(&app, "alice", "password123").await;

    let req_body = conclave_proto::RegisterRequest {
        username: "alice".to_string(),
        password: "password456".to_string(),
        alias: String::new(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/register")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn test_register_empty_username() {
    let app = setup();

    let req_body = conclave_proto::RegisterRequest {
        username: "".to_string(),
        password: "password123".to_string(),
        alias: String::new(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/register")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_register_short_password() {
    let app = setup();

    let req_body = conclave_proto::RegisterRequest {
        username: "alice".to_string(),
        password: "short".to_string(),
        alias: String::new(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/register")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_register_long_username() {
    let app = setup();

    let long_name = "a".repeat(65);
    let req_body = conclave_proto::RegisterRequest {
        username: long_name,
        password: "password123".to_string(),
        alias: String::new(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/register")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

// ── Login Tests (6-8) ─────────────────────────────────────────────

#[tokio::test]
async fn test_login_success() {
    let app = setup();
    register_user(&app, "alice", "password123").await;

    let token = login_user(&app, "alice", "password123").await;
    assert!(!token.is_empty());
}

#[tokio::test]
async fn test_login_wrong_password() {
    let app = setup();
    register_user(&app, "alice", "password123").await;

    let req_body = conclave_proto::LoginRequest {
        username: "alice".to_string(),
        password: "wrongpassword".to_string(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/login")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_login_nonexistent_user() {
    let app = setup();

    let req_body = conclave_proto::LoginRequest {
        username: "nobody".to_string(),
        password: "password123".to_string(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/login")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

// ── Logout Test (9) ───────────────────────────────────────────────

#[tokio::test]
async fn test_logout_success() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    // Logout
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/logout")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    // Attempt to use /me with the same token should return 401
    let request = Request::builder()
        .method("GET")
        .uri("/api/v1/me")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

// ── Me Tests (10-12) ──────────────────────────────────────────────

#[tokio::test]
async fn test_me_authenticated() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    let request = Request::builder()
        .method("GET")
        .uri("/api/v1/me")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let resp = conclave_proto::UserInfoResponse::decode(body_bytes).unwrap();
    assert_eq!(resp.username, "alice");
    assert!(resp.user_id > 0);
}

#[tokio::test]
async fn test_me_no_auth_header() {
    let app = setup();

    let request = Request::builder()
        .method("GET")
        .uri("/api/v1/me")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_me_invalid_token() {
    let app = setup();

    let request = Request::builder()
        .method("GET")
        .uri("/api/v1/me")
        .header(header::AUTHORIZATION, "Bearer totally_bogus_token")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

// ── Change Password Tests ─────────────────────────────────────────

#[tokio::test]
async fn test_change_password_success() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    let req_body = conclave_proto::ChangePasswordRequest {
        current_password: "password123".to_string(),
        new_password: "newpass456".to_string(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/change-password")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Login with new password should succeed
    let _new_token = login_user(&app, "alice", "newpass456").await;
}

#[tokio::test]
async fn test_change_password_old_password_no_longer_works() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    let req_body = conclave_proto::ChangePasswordRequest {
        current_password: "password123".to_string(),
        new_password: "newpass456".to_string(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/change-password")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Login with old password should fail
    let req_body = conclave_proto::LoginRequest {
        username: "alice".to_string(),
        password: "password123".to_string(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/login")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_change_password_wrong_current() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    let req_body = conclave_proto::ChangePasswordRequest {
        current_password: "wrong_password".to_string(),
        new_password: "newpass456".to_string(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/change-password")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_change_password_short_new_password() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    let req_body = conclave_proto::ChangePasswordRequest {
        current_password: "password123".to_string(),
        new_password: "short".to_string(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/change-password")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_change_password_unauthenticated() {
    let app = setup();

    let req_body = conclave_proto::ChangePasswordRequest {
        current_password: "password123".to_string(),
        new_password: "newpass456".to_string(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/change-password")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_change_password_empty_new_password() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    let req_body = conclave_proto::ChangePasswordRequest {
        current_password: "password123".to_string(),
        new_password: String::new(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/change-password")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_change_password_session_stays_valid() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    let req_body = conclave_proto::ChangePasswordRequest {
        current_password: "password123".to_string(),
        new_password: "newpass456".to_string(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/change-password")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // The same token should still work for authenticated requests
    let request = Request::builder()
        .method("GET")
        .uri("/api/v1/me")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

// ── User Lookup Tests (13-14) ─────────────────────────────────────

#[tokio::test]
async fn test_get_user_by_username_success() {
    let app = setup();
    let user_id = register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    let request = Request::builder()
        .method("GET")
        .uri("/api/v1/users/alice")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let resp = conclave_proto::UserInfoResponse::decode(body_bytes).unwrap();
    assert_eq!(resp.username, "alice");
    assert_eq!(resp.user_id, user_id);
}

#[tokio::test]
async fn test_get_user_by_username_not_found() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    let request = Request::builder()
        .method("GET")
        .uri("/api/v1/users/unknown_user")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_get_user_by_id_success() {
    let app = setup();
    let user_id = register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    let request = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/users/by-id/{user_id}"))
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let resp = conclave_proto::UserInfoResponse::decode(body_bytes).unwrap();
    assert_eq!(resp.user_id, user_id);
    assert_eq!(resp.username, "alice");
}

#[tokio::test]
async fn test_get_user_by_id_not_found() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    let request = Request::builder()
        .method("GET")
        .uri("/api/v1/users/by-id/99999")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// ── Key Package Tests (15-19) ─────────────────────────────────────

#[tokio::test]
async fn test_upload_key_package_success() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    upload_key_package_for(&app, &token, &fake_key_package(b"dummy_key_package")).await;
}

#[tokio::test]
async fn test_upload_key_package_empty() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    let req_body = conclave_proto::UploadKeyPackageRequest {
        key_package_data: vec![],
        entries: vec![],
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/key-packages")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_upload_key_package_too_large() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    let large_data = vec![0u8; 16 * 1024 + 1]; // 16 KiB + 1 byte
    let req_body = conclave_proto::UploadKeyPackageRequest {
        key_package_data: large_data,
        entries: vec![],
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/key-packages")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_get_key_package_success() {
    let app = setup();
    let user_id = register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    upload_key_package_for(&app, &token, &fake_key_package(b"my_key_package_data")).await;

    let request = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/key-packages/{user_id}"))
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let resp = conclave_proto::GetKeyPackageResponse::decode(body_bytes).unwrap();
    assert_eq!(
        resp.key_package_data,
        fake_key_package(b"my_key_package_data")
    );
}

#[tokio::test]
async fn test_get_key_package_none_available() {
    let app = setup();
    let user_id = register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    let request = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/key-packages/{user_id}"))
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// ── Group Tests (20-24) ───────────────────────────────────────────

#[tokio::test]
async fn test_create_group_success() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    let group_id = create_group_for(&app, &token, "test_group").await;
    assert!(group_id > 0);
}

#[tokio::test]
async fn test_create_group_empty_alias_and_name() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    let req_body = conclave_proto::CreateGroupRequest {
        alias: String::new(),
        group_name: "empty_alias_test_group".to_string(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/groups")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn test_create_group_long_alias() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    let long_alias = "g".repeat(65);
    let req_body = conclave_proto::CreateGroupRequest {
        alias: long_alias,
        group_name: "long_alias_test_group".to_string(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/groups")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_list_groups_empty() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    let request = Request::builder()
        .method("GET")
        .uri("/api/v1/groups")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let resp = conclave_proto::ListGroupsResponse::decode(body_bytes).unwrap();
    assert!(resp.groups.is_empty());
}

#[tokio::test]
async fn test_list_groups_with_groups() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    let group_id = create_group_for(&app, &token, "my_group").await;

    let request = Request::builder()
        .method("GET")
        .uri("/api/v1/groups")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let resp = conclave_proto::ListGroupsResponse::decode(body_bytes).unwrap();
    assert_eq!(resp.groups.len(), 1);
    assert_eq!(resp.groups[0].group_id, group_id);
    assert_eq!(resp.groups[0].alias, "my_group");
}

// ── Message Tests (25-29) ─────────────────────────────────────────

#[tokio::test]
async fn test_send_message_success() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;
    let group_id = create_group_for(&app, &token, "msg_group").await;

    let req_body = conclave_proto::SendMessageRequest {
        mls_message: b"test_message".to_vec(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/messages"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let resp = conclave_proto::SendMessageResponse::decode(body_bytes).unwrap();
    assert!(resp.sequence_num > 0);
}

#[tokio::test]
async fn test_send_message_not_member() {
    let app = setup();
    // Alice creates a group
    register_user(&app, "alice", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let group_id = create_group_for(&app, &alice_token, "alice_group").await;

    // Bob is not a member
    register_user(&app, "bob", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;

    let req_body = conclave_proto::SendMessageRequest {
        mls_message: b"test_message".to_vec(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/messages"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {bob_token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_get_messages_success() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;
    let group_id = create_group_for(&app, &token, "msg_group").await;

    // Send a message
    let req_body = conclave_proto::SendMessageRequest {
        mls_message: b"hello_world".to_vec(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/messages"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();

    app.clone().oneshot(request).await.unwrap();

    // Get messages
    let request = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/groups/{group_id}/messages"))
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let resp = conclave_proto::GetMessagesResponse::decode(body_bytes).unwrap();
    assert_eq!(resp.messages.len(), 1);
    assert_eq!(resp.messages[0].mls_message, b"hello_world");
}

#[tokio::test]
async fn test_get_messages_not_member() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let group_id = create_group_for(&app, &alice_token, "alice_group").await;

    register_user(&app, "bob", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;

    let request = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/groups/{group_id}/messages"))
        .header(header::AUTHORIZATION, format!("Bearer {bob_token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_get_messages_after_seq() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;
    let group_id = create_group_for(&app, &token, "msg_group").await;

    // Send two messages
    for msg in &[b"msg_one".as_slice(), b"msg_two".as_slice()] {
        let req_body = conclave_proto::SendMessageRequest {
            mls_message: msg.to_vec(),
        };
        let mut body = Vec::new();
        req_body.encode(&mut body).unwrap();

        let request = Request::builder()
            .method("POST")
            .uri(format!("/api/v1/groups/{group_id}/messages"))
            .header(header::CONTENT_TYPE, "application/x-protobuf")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::from(body))
            .unwrap();

        app.clone().oneshot(request).await.unwrap();
    }

    // Get messages after seq=1
    let request = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/groups/{group_id}/messages?after=1"))
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let resp = conclave_proto::GetMessagesResponse::decode(body_bytes).unwrap();
    assert_eq!(resp.messages.len(), 1);
    assert_eq!(resp.messages[0].mls_message, b"msg_two");
    assert_eq!(resp.messages[0].sequence_num, 2);
}

// ── Invite Tests (30-31) ──────────────────────────────────────────

#[tokio::test]
async fn test_invite_no_usernames() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;
    let group_id = create_group_for(&app, &token, "inv_group").await;

    let req_body = conclave_proto::InviteToGroupRequest { user_ids: vec![] };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/invite"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_invite_not_member() {
    let app = setup();
    // Alice creates a group
    register_user(&app, "alice", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let group_id = create_group_for(&app, &alice_token, "alice_group").await;

    // Bob is not a member and tries to invite
    register_user(&app, "bob", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;

    // Charlie exists to be invited
    let charlie_id = register_user(&app, "charlie", "password123").await;

    let req_body = conclave_proto::InviteToGroupRequest {
        user_ids: vec![charlie_id],
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/invite"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {bob_token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

// ── Remove Member Tests (32-35) ───────────────────────────────────

#[tokio::test]
async fn test_remove_member_success() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;

    let bob_id = register_user(&app, "bob", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;

    let group_id = create_group_for(&app, &alice_token, "rm_group").await;
    add_member_via_escrow(&app, &alice_token, &bob_token, group_id, bob_id).await;

    // Alice removes bob
    let req_body = conclave_proto::RemoveMemberRequest {
        user_id: bob_id,
        commit_message: b"remove_commit".to_vec(),
        group_info: b"updated_info".to_vec(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/remove"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {alice_token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Verify bob is no longer a member by having bob try to send a message
    let req_body = conclave_proto::SendMessageRequest {
        mls_message: b"should_fail".to_vec(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/messages"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {bob_token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_remove_member_not_group_member() {
    let app = setup();
    let alice_id = register_user(&app, "alice", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let group_id = create_group_for(&app, &alice_token, "rm_group").await;

    // Bob is not a group member
    register_user(&app, "bob", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;

    let req_body = conclave_proto::RemoveMemberRequest {
        user_id: alice_id,
        commit_message: b"commit".to_vec(),
        group_info: b"info".to_vec(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/remove"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {bob_token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_remove_member_target_not_member() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let group_id = create_group_for(&app, &alice_token, "rm_group").await;

    // Bob exists but is not in the group
    let bob_id = register_user(&app, "bob", "password123").await;

    let req_body = conclave_proto::RemoveMemberRequest {
        user_id: bob_id,
        commit_message: b"commit".to_vec(),
        group_info: b"info".to_vec(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/remove"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {alice_token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_remove_member_target_not_found() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let group_id = create_group_for(&app, &alice_token, "rm_group").await;

    let req_body = conclave_proto::RemoveMemberRequest {
        user_id: 999999,
        commit_message: b"commit".to_vec(),
        group_info: b"info".to_vec(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/remove"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {alice_token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// ── Leave Group Tests (36-37) ─────────────────────────────────────

#[tokio::test]
async fn test_leave_group_success() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;

    let bob_id = register_user(&app, "bob", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;

    let group_id = create_group_for(&app, &alice_token, "leave_group").await;
    add_member_via_escrow(&app, &alice_token, &bob_token, group_id, bob_id).await;

    // Bob leaves the group
    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/leave"))
        .header(header::AUTHORIZATION, format!("Bearer {bob_token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Verify bob is no longer a member
    let req_body = conclave_proto::SendMessageRequest {
        mls_message: b"should_fail".to_vec(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/messages"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {bob_token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_leave_group_not_member() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let group_id = create_group_for(&app, &alice_token, "leave_group").await;

    register_user(&app, "bob", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;

    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/leave"))
        .header(header::AUTHORIZATION, format!("Bearer {bob_token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

// ── Group Info Tests (38-40) ──────────────────────────────────────

#[tokio::test]
async fn test_get_group_info_success() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;
    let group_id = create_group_for(&app, &token, "info_group").await;

    // Store group info via upload_commit
    let req_body = conclave_proto::UploadCommitRequest {
        commit_message: b"commit".to_vec(),

        group_info: b"group_info_data".to_vec(),
        mls_group_id: String::new(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/commit"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Get group info
    let request = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/groups/{group_id}/group-info"))
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let resp = conclave_proto::GetGroupInfoResponse::decode(body_bytes).unwrap();
    assert_eq!(resp.group_info, b"group_info_data");
}

#[tokio::test]
async fn test_get_group_info_not_found() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;
    let group_id = create_group_for(&app, &token, "info_group").await;

    // No group info has been stored yet
    let request = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/groups/{group_id}/group-info"))
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_get_group_info_not_member() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let group_id = create_group_for(&app, &alice_token, "info_group").await;

    register_user(&app, "bob", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;

    let request = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/groups/{group_id}/group-info"))
        .header(header::AUTHORIZATION, format!("Bearer {bob_token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

// ── External Join Test (41) ───────────────────────────────────────

#[tokio::test]
async fn test_external_join_success() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let group_id = create_group_for(&app, &alice_token, "ext_group").await;

    // Register Bob and add him to the group via escrow invite.
    let bob_id = register_user(&app, "bob", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;
    add_member_via_escrow(&app, &alice_token, &bob_token, group_id, bob_id).await;

    // Upload group info so external join can proceed.
    let commit_body = conclave_proto::UploadCommitRequest {
        commit_message: b"update".to_vec(),
        group_info: b"fake_group_info".to_vec(),
        mls_group_id: String::new(),
    };
    let mut cbody = Vec::new();
    commit_body.encode(&mut cbody).unwrap();
    let commit_request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/commit"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {alice_token}"))
        .body(Body::from(cbody))
        .unwrap();
    let response = app.clone().oneshot(commit_request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Bob (an existing member) performs an external join (e.g., after account reset).
    let req_body = conclave_proto::ExternalJoinRequest {
        commit_message: b"ext_commit".to_vec(),
        mls_group_id: String::new(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/external-join"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {bob_token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Verify bob is still a member by sending a message.
    let req_body = conclave_proto::SendMessageRequest {
        mls_message: b"bob_message".to_vec(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/messages"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {bob_token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

// ── Reset Account Test (42) ───────────────────────────────────────

#[tokio::test]
async fn test_reset_account_success() {
    let app = setup();
    let user_id = register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    // Upload a key package
    upload_key_package_for(&app, &token, &fake_key_package(b"my_key_package")).await;

    // Reset account
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/reset-account")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Verify key package is gone
    let request = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/key-packages/{user_id}"))
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// ── Upload Commit Tests (43-44) ───────────────────────────────────

#[tokio::test]
async fn test_upload_commit_stores_group_info() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;
    let group_id = create_group_for(&app, &token, "commit_group").await;

    // Upload commit with group_info
    let req_body = conclave_proto::UploadCommitRequest {
        commit_message: b"commit_msg".to_vec(),

        group_info: b"stored_group_info".to_vec(),
        mls_group_id: String::new(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/commit"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Retrieve group info and verify
    let request = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/groups/{group_id}/group-info"))
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let resp = conclave_proto::GetGroupInfoResponse::decode(body_bytes).unwrap();
    assert_eq!(resp.group_info, b"stored_group_info");
}

#[tokio::test]
async fn test_upload_commit_not_member() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let group_id = create_group_for(&app, &alice_token, "commit_group").await;

    register_user(&app, "bob", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;

    let req_body = conclave_proto::UploadCommitRequest {
        commit_message: b"commit".to_vec(),

        group_info: b"info".to_vec(),
        mls_group_id: String::new(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/commit"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {bob_token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

// ── Welcome Tests (45-46) ─────────────────────────────────────────

#[tokio::test]
async fn test_accept_welcome_success() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;

    let bob_id = register_user(&app, "bob", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;

    let group_id = create_group_for(&app, &alice_token, "welcome_group").await;

    // Alice escrow-invites bob (this creates a pending invite with welcome data).
    create_escrow_invite(&app, &alice_token, group_id, bob_id).await;

    // Bob accepts the invite (this creates a pending welcome and adds bob to group).
    let invites = list_pending_invites(&app, &bob_token).await;
    assert_eq!(invites.len(), 1);
    let invite_id = invites[0].invite_id;

    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/invites/{invite_id}/accept"))
        .header(header::AUTHORIZATION, format!("Bearer {bob_token}"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Bob lists pending welcomes
    let request = Request::builder()
        .method("GET")
        .uri("/api/v1/welcomes")
        .header(header::AUTHORIZATION, format!("Bearer {bob_token}"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let resp = conclave_proto::ListPendingWelcomesResponse::decode(body_bytes).unwrap();
    assert_eq!(resp.welcomes.len(), 1);
    assert_eq!(resp.welcomes[0].welcome_message, b"welcome_data");

    let welcome_id = resp.welcomes[0].welcome_id;

    // Bob accepts the welcome
    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/welcomes/{welcome_id}/accept"))
        .header(header::AUTHORIZATION, format!("Bearer {bob_token}"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    // Verify the welcome is gone
    let request = Request::builder()
        .method("GET")
        .uri("/api/v1/welcomes")
        .header(header::AUTHORIZATION, format!("Bearer {bob_token}"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let resp = conclave_proto::ListPendingWelcomesResponse::decode(body_bytes).unwrap();
    assert!(resp.welcomes.is_empty());
}

#[tokio::test]
async fn test_accept_welcome_not_found() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/welcomes/99999/accept")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// ── Batch Key Package Tests (47-49) ───────────────────────────────

#[tokio::test]
async fn test_batch_upload_and_ordered_consumption() {
    let app = setup();
    let user_id = register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    // Upload 3 regular key packages in a batch.
    upload_key_packages_batch(
        &app,
        &token,
        vec![
            conclave_proto::KeyPackageEntry {
                data: fake_key_package(b"kp1"),
                is_last_resort: false,
            },
            conclave_proto::KeyPackageEntry {
                data: fake_key_package(b"kp2"),
                is_last_resort: false,
            },
            conclave_proto::KeyPackageEntry {
                data: fake_key_package(b"kp3"),
                is_last_resort: false,
            },
        ],
    )
    .await;

    // Consume them in FIFO order.
    let expected_packages = [
        fake_key_package(b"kp1"),
        fake_key_package(b"kp2"),
        fake_key_package(b"kp3"),
    ];
    for expected in expected_packages {
        let request = Request::builder()
            .method("GET")
            .uri(format!("/api/v1/key-packages/{user_id}"))
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();

        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
        let resp = conclave_proto::GetKeyPackageResponse::decode(body_bytes).unwrap();
        assert_eq!(resp.key_package_data, expected);
    }

    // All consumed.
    let request = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/key-packages/{user_id}"))
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_last_resort_not_deleted_on_consumption() {
    let app = setup();
    let user_id = register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    // Upload 1 last-resort + 1 regular.
    upload_key_packages_batch(
        &app,
        &token,
        vec![
            conclave_proto::KeyPackageEntry {
                data: fake_key_package(b"last_resort"),
                is_last_resort: true,
            },
            conclave_proto::KeyPackageEntry {
                data: fake_key_package(b"regular"),
                is_last_resort: false,
            },
        ],
    )
    .await;

    // First consume should return the regular one (deleted).
    let request = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/key-packages/{user_id}"))
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let resp = conclave_proto::GetKeyPackageResponse::decode(body_bytes).unwrap();
    assert_eq!(resp.key_package_data, fake_key_package(b"regular"));

    // Second consume should return last-resort (NOT deleted).
    let request = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/key-packages/{user_id}"))
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let resp = conclave_proto::GetKeyPackageResponse::decode(body_bytes).unwrap();
    assert_eq!(resp.key_package_data, fake_key_package(b"last_resort"));

    // Third consume should STILL return last-resort.
    let request = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/key-packages/{user_id}"))
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let resp = conclave_proto::GetKeyPackageResponse::decode(body_bytes).unwrap();
    assert_eq!(resp.key_package_data, fake_key_package(b"last_resort"));
}

// ── Username Validation Tests ─────────────────────────────────────

#[tokio::test]
async fn test_register_unicode_username_rejected() {
    let app = setup();
    let req_body = conclave_proto::RegisterRequest {
        username: "héllo_wörld".to_string(),
        password: "password123".to_string(),
        alias: String::new(),
    };
    let body = req_body.encode_to_vec();
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/register")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .body(Body::from(body))
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_register_username_with_spaces() {
    let app = setup();
    let req_body = conclave_proto::RegisterRequest {
        username: "has spaces".to_string(),
        password: "password123".to_string(),
        alias: String::new(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/register")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .body(Body::from(body))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_register_username_with_control_chars() {
    let app = setup();
    let req_body = conclave_proto::RegisterRequest {
        username: "user\x00name".to_string(),
        password: "password123".to_string(),
        alias: String::new(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/register")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .body(Body::from(body))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

// ── Key Package Wire Format Validation Tests ──────────────────────

#[tokio::test]
async fn test_upload_key_package_invalid_mls_version() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;
    let bad_kp = vec![0x00, 0x02, 0x00, 0x05, 0xAA, 0xBB];
    let req_body = conclave_proto::UploadKeyPackageRequest {
        key_package_data: bad_kp,
        entries: vec![],
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/key-packages")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_upload_key_package_wrong_wire_format() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;
    let bad_kp = vec![0x00, 0x01, 0x00, 0x01, 0xAA, 0xBB];
    let req_body = conclave_proto::UploadKeyPackageRequest {
        key_package_data: bad_kp,
        entries: vec![],
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/key-packages")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_upload_key_package_too_short_for_header() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;
    let bad_kp = vec![0x00, 0x01, 0x00];
    let req_body = conclave_proto::UploadKeyPackageRequest {
        key_package_data: bad_kp,
        entries: vec![],
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/key-packages")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_batch_upload_validates_wire_format() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;
    let entries = vec![
        conclave_proto::KeyPackageEntry {
            data: fake_key_package(b"good"),
            is_last_resort: false,
        },
        conclave_proto::KeyPackageEntry {
            data: vec![0x00, 0x02, 0x00, 0x05, 0xAA],
            is_last_resort: false,
        },
    ];
    let req_body = conclave_proto::UploadKeyPackageRequest {
        key_package_data: vec![],
        entries,
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/key-packages")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

// ── Invite Flow Tests ─────────────────────────────────────────────

#[tokio::test]
async fn test_invite_consumes_key_package() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let bob_id = register_user(&app, "bob", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;
    upload_key_package_for(&app, &bob_token, &fake_key_package(b"bob_kp_1")).await;

    let group_id = create_group_for(&app, &alice_token, "invite_consume").await;

    // Invite bob (this should consume his key package).
    let req_body = conclave_proto::InviteToGroupRequest {
        user_ids: vec![bob_id],
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();
    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/invite"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {alice_token}"))
        .body(Body::from(body))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Bob's key package should be consumed.
    let request = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/key-packages/{bob_id}"))
        .header(header::AUTHORIZATION, format!("Bearer {alice_token}"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_invite_existing_member_conflict() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let bob_id = register_user(&app, "bob", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;

    let group_id = create_group_for(&app, &alice_token, "dup_invite").await;
    add_member_via_escrow(&app, &alice_token, &bob_token, group_id, bob_id).await;

    upload_key_package_for(&app, &bob_token, &fake_key_package(b"bob_kp2")).await;

    let req_body = conclave_proto::InviteToGroupRequest {
        user_ids: vec![bob_id],
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();
    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/invite"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {alice_token}"))
        .body(Body::from(body))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn test_invite_nonexistent_user() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let group_id = create_group_for(&app, &alice_token, "ghost_invite").await;

    let req_body = conclave_proto::InviteToGroupRequest {
        user_ids: vec![999999],
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();
    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/invite"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {alice_token}"))
        .body(Body::from(body))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// ── Upload Commit With Welcomes Test ──────────────────────────────

#[tokio::test]
async fn test_escrow_invite_creates_pending_welcome_on_accept() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let bob_id = register_user(&app, "bob", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;

    let group_id = create_group_for(&app, &alice_token, "welcome_test").await;

    // Alice escrow-invites bob.
    create_escrow_invite(&app, &alice_token, group_id, bob_id).await;

    // Bob accepts the invite.
    let invites = list_pending_invites(&app, &bob_token).await;
    assert_eq!(invites.len(), 1);
    let invite_id = invites[0].invite_id;

    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/invites/{invite_id}/accept"))
        .header(header::AUTHORIZATION, format!("Bearer {bob_token}"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Bob should now have a pending welcome.
    let request = Request::builder()
        .method("GET")
        .uri("/api/v1/welcomes")
        .header(header::AUTHORIZATION, format!("Bearer {bob_token}"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let resp = conclave_proto::ListPendingWelcomesResponse::decode(body_bytes).unwrap();
    assert_eq!(resp.welcomes.len(), 1);
    assert_eq!(resp.welcomes[0].group_id, group_id);
    assert_eq!(resp.welcomes[0].welcome_message, b"welcome_data");
}

// ── Leave Group With Commit Storage ───────────────────────────────

#[tokio::test]
async fn test_leave_group_stores_commit_message() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let bob_id = register_user(&app, "bob", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;

    let group_id = create_group_for(&app, &alice_token, "leave_commit").await;
    add_member_via_escrow(&app, &alice_token, &bob_token, group_id, bob_id).await;

    let req_body = conclave_proto::LeaveGroupRequest {
        commit_message: b"leave_commit_data".to_vec(),
        group_info: b"leave_gi".to_vec(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();
    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/leave"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {bob_token}"))
        .body(Body::from(body))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let request = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/groups/{group_id}/messages?after=0"))
        .header(header::AUTHORIZATION, format!("Bearer {alice_token}"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let resp = conclave_proto::GetMessagesResponse::decode(body_bytes).unwrap();
    let has_leave_commit = resp
        .messages
        .iter()
        .any(|m| m.mls_message == b"leave_commit_data");
    assert!(has_leave_commit, "leave commit message should be stored");
}

// ── Remove Member Stores Group Info ───────────────────────────────

#[tokio::test]
async fn test_remove_member_stores_group_info() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let bob_id = register_user(&app, "bob", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;

    let group_id = create_group_for(&app, &alice_token, "rm_gi_group").await;
    add_member_via_escrow(&app, &alice_token, &bob_token, group_id, bob_id).await;

    let req_body = conclave_proto::RemoveMemberRequest {
        user_id: bob_id,
        commit_message: b"removal_commit".to_vec(),
        group_info: b"removal_gi_data".to_vec(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();
    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/remove"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {alice_token}"))
        .body(Body::from(body))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let request = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/groups/{group_id}/group-info"))
        .header(header::AUTHORIZATION, format!("Bearer {alice_token}"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let resp = conclave_proto::GetGroupInfoResponse::decode(body_bytes).unwrap();
    assert_eq!(resp.group_info, b"removal_gi_data");
}

// ── Last-Resort Replacement ───────────────────────────────────────

#[tokio::test]
async fn test_last_resort_replacement_via_batch() {
    let app = setup();
    let user_id = register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    upload_key_packages_batch(
        &app,
        &token,
        vec![conclave_proto::KeyPackageEntry {
            data: fake_key_package(b"lr_old"),
            is_last_resort: true,
        }],
    )
    .await;

    upload_key_packages_batch(
        &app,
        &token,
        vec![conclave_proto::KeyPackageEntry {
            data: fake_key_package(b"lr_new"),
            is_last_resort: true,
        }],
    )
    .await;

    let request = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/key-packages/{user_id}"))
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let resp = conclave_proto::GetKeyPackageResponse::decode(body_bytes).unwrap();
    assert_eq!(resp.key_package_data, fake_key_package(b"lr_new"));
}

// ── Protobuf Error Response Format ────────────────────────────────

#[tokio::test]
async fn test_error_response_is_protobuf() {
    let app = setup();
    let request = Request::builder()
        .method("GET")
        .uri("/api/v1/me")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(content_type, "application/x-protobuf");

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let error_resp = conclave_proto::ErrorResponse::decode(body_bytes).unwrap();
    assert!(!error_resp.message.is_empty());
}

// ── Multiple Groups ───────────────────────────────────────────────

#[tokio::test]
async fn test_list_multiple_groups() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    create_group_for(&app, &token, "group_1").await;
    create_group_for(&app, &token, "group_2").await;
    create_group_for(&app, &token, "group_3").await;

    let request = Request::builder()
        .method("GET")
        .uri("/api/v1/groups")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let resp = conclave_proto::ListGroupsResponse::decode(body_bytes).unwrap();
    assert_eq!(resp.groups.len(), 3);
}

// ── Group Members in ListGroups ───────────────────────────────────

#[tokio::test]
async fn test_list_groups_includes_members() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let bob_id = register_user(&app, "bob", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;
    let group_id = create_group_for(&app, &alice_token, "member_group").await;
    add_member_via_escrow(&app, &alice_token, &bob_token, group_id, bob_id).await;

    let request = Request::builder()
        .method("GET")
        .uri("/api/v1/groups")
        .header(header::AUTHORIZATION, format!("Bearer {alice_token}"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let resp = conclave_proto::ListGroupsResponse::decode(body_bytes).unwrap();
    assert_eq!(resp.groups.len(), 1);
    let member_names: Vec<&str> = resp.groups[0]
        .members
        .iter()
        .map(|m| m.username.as_str())
        .collect();
    assert!(member_names.contains(&"alice"));
    assert!(member_names.contains(&"bob"));
}

// ── Message Sequence Numbers ──────────────────────────────────────

#[tokio::test]
async fn test_message_sequence_numbers_increment() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;
    let group_id = create_group_for(&app, &token, "seq_group").await;

    for expected_seq in 1u64..=3 {
        let req_body = conclave_proto::SendMessageRequest {
            mls_message: format!("msg_{expected_seq}").into_bytes(),
        };
        let mut body = Vec::new();
        req_body.encode(&mut body).unwrap();
        let request = Request::builder()
            .method("POST")
            .uri(format!("/api/v1/groups/{group_id}/messages"))
            .header(header::CONTENT_TYPE, "application/x-protobuf")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::from(body))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
        let resp = conclave_proto::SendMessageResponse::decode(body_bytes).unwrap();
        assert_eq!(resp.sequence_num, expected_seq);
    }
}

// ── Multiple Sessions ─────────────────────────────────────────────

#[tokio::test]
async fn test_multiple_login_sessions() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token1 = login_user(&app, "alice", "password123").await;
    let token2 = login_user(&app, "alice", "password123").await;
    assert_ne!(token1, token2);

    for token in [&token1, &token2] {
        let request = Request::builder()
            .method("GET")
            .uri("/api/v1/me")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}

// ── Logout Isolation ──────────────────────────────────────────────

#[tokio::test]
async fn test_logout_invalidates_only_one_token() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token1 = login_user(&app, "alice", "password123").await;
    let token2 = login_user(&app, "alice", "password123").await;

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/logout")
        .header(header::AUTHORIZATION, format!("Bearer {token1}"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let request = Request::builder()
        .method("GET")
        .uri("/api/v1/me")
        .header(header::AUTHORIZATION, format!("Bearer {token1}"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let request = Request::builder()
        .method("GET")
        .uri("/api/v1/me")
        .header(header::AUTHORIZATION, format!("Bearer {token2}"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

// ── Reset Keeps Session ───────────────────────────────────────────

#[tokio::test]
async fn test_reset_account_keeps_session() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;
    upload_key_package_for(&app, &token, &fake_key_package(b"kp")).await;

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/reset-account")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let request = Request::builder()
        .method("GET")
        .uri("/api/v1/me")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

// ── Nonexistent Group ─────────────────────────────────────────────

#[tokio::test]
async fn test_send_message_to_nonexistent_group() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    let req_body = conclave_proto::SendMessageRequest {
        mls_message: b"msg".to_vec(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/groups/999999/messages")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

// ── Empty Batch Entry ─────────────────────────────────────────────

#[tokio::test]
async fn test_batch_upload_empty_entry_data() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    let req_body = conclave_proto::UploadKeyPackageRequest {
        key_package_data: vec![],
        entries: vec![conclave_proto::KeyPackageEntry {
            data: vec![],
            is_last_resort: false,
        }],
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/key-packages")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

// ── Malformed Protobuf ────────────────────────────────────────────

#[tokio::test]
async fn test_malformed_protobuf_body() {
    let app = setup();
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/register")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .body(Body::from(vec![0xFF, 0xFF, 0xFF, 0xFF]))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

// ── Self-Invite Skipped ───────────────────────────────────────────

#[tokio::test]
async fn test_invite_self_is_skipped() {
    let app = setup();
    let alice_id = register_user(&app, "alice", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    upload_key_package_for(&app, &alice_token, &fake_key_package(b"alice_kp")).await;

    let group_id = create_group_for(&app, &alice_token, "self_invite").await;

    let req_body = conclave_proto::InviteToGroupRequest {
        user_ids: vec![alice_id],
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();
    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/invite"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {alice_token}"))
        .body(Body::from(body))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let resp = conclave_proto::InviteToGroupResponse::decode(body_bytes).unwrap();
    assert!(
        resp.member_key_packages.is_empty(),
        "self-invite should produce no key packages"
    );
}

// ── External Join Nonexistent Group ───────────────────────────────

#[tokio::test]
async fn test_external_join_nonexistent_group() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    let req_body = conclave_proto::ExternalJoinRequest {
        commit_message: b"ext_commit".to_vec(),
        mls_group_id: String::new(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/groups/999999/external-join")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert!(
        response.status() == StatusCode::NOT_FOUND || response.status() == StatusCode::UNAUTHORIZED
    );
}

// ── Get Messages After Parameter ──────────────────────────────────

#[tokio::test]
async fn test_get_messages_respects_after_parameter() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;
    let group_id = create_group_for(&app, &token, "paging_group").await;

    for i in 1..=5 {
        let req_body = conclave_proto::SendMessageRequest {
            mls_message: format!("msg_{i}").into_bytes(),
        };
        let mut body = Vec::new();
        req_body.encode(&mut body).unwrap();
        let request = Request::builder()
            .method("POST")
            .uri(format!("/api/v1/groups/{group_id}/messages"))
            .header(header::CONTENT_TYPE, "application/x-protobuf")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::from(body))
            .unwrap();
        app.clone().oneshot(request).await.unwrap();
    }

    let request = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/groups/{group_id}/messages?after=3"))
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let resp = conclave_proto::GetMessagesResponse::decode(body_bytes).unwrap();
    assert_eq!(resp.messages.len(), 2);
    assert_eq!(resp.messages[0].sequence_num, 4);
    assert_eq!(resp.messages[1].sequence_num, 5);
}

// ── Username Boundary Validation Tests ────────────────────────────

#[tokio::test]
async fn test_register_username_exactly_64_chars() {
    let app = setup();
    let username = "a".repeat(64);
    let user_id = register_user(&app, &username, "password123").await;
    assert!(user_id > 0);
}

#[tokio::test]
async fn test_register_username_starting_with_underscore() {
    let app = setup();
    let req_body = conclave_proto::RegisterRequest {
        username: "_underscored".to_string(),
        password: "password123".to_string(),
        alias: String::new(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/register")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_register_username_starting_with_dot() {
    let app = setup();
    let req_body = conclave_proto::RegisterRequest {
        username: ".dotted".to_string(),
        password: "password123".to_string(),
        alias: String::new(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/register")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_register_username_starting_with_hyphen() {
    let app = setup();
    let req_body = conclave_proto::RegisterRequest {
        username: "-hyphenated".to_string(),
        password: "password123".to_string(),
        alias: String::new(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/register")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_register_username_valid_with_underscores() {
    let app = setup();
    let user_id = register_user(&app, "user_name_with_all123", "password123").await;
    assert!(user_id > 0);
}

#[tokio::test]
async fn test_register_username_with_dot_rejected() {
    let app = setup();
    let req_body = conclave_proto::RegisterRequest {
        username: "user.name".to_string(),
        password: "password123".to_string(),
        alias: String::new(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/register")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_register_username_with_hyphen_rejected() {
    let app = setup();
    let req_body = conclave_proto::RegisterRequest {
        username: "user-name".to_string(),
        password: "password123".to_string(),
        alias: String::new(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/register")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_register_empty_password() {
    let app = setup();
    let req_body = conclave_proto::RegisterRequest {
        username: "validuser".to_string(),
        password: String::new(),
        alias: String::new(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/register")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

// ── Key Package Edge Cases ────────────────────────────────────────

#[tokio::test]
async fn test_key_package_exactly_16kib() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    // Build a key package that is exactly 16 KiB total (including 4-byte MLS header).
    let mut data = vec![0x00, 0x01, 0x00, 0x05];
    data.resize(16 * 1024, 0xAB);
    upload_key_package_for(&app, &token, &data).await;
}

#[tokio::test]
async fn test_batch_upload_oversized_entry() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    // Create one entry that exceeds 16 KiB.
    let mut oversized_data = vec![0x00, 0x01, 0x00, 0x05];
    oversized_data.resize(16 * 1024 + 1, 0xCC);

    let entries = vec![conclave_proto::KeyPackageEntry {
        data: oversized_data,
        is_last_resort: false,
    }];

    let req_body = conclave_proto::UploadKeyPackageRequest {
        key_package_data: vec![],
        entries,
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/key-packages")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

// ── External Join Edge Cases ──────────────────────────────────────

#[tokio::test]
async fn test_external_join_no_group_info_stored() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let group_id = create_group_for(&app, &alice_token, "no_info_group").await;

    // Alice (a member) attempts external join but no group_info has been uploaded.
    let req_body = conclave_proto::ExternalJoinRequest {
        commit_message: b"ext_commit".to_vec(),
        mls_group_id: String::new(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/external-join"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {alice_token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

// ── Message Pagination ────────────────────────────────────────────

#[tokio::test]
async fn test_get_messages_limit_capped_at_500() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;
    let group_id = create_group_for(&app, &token, "big_group").await;

    // Send 600 messages.
    for i in 1..=600 {
        let req_body = conclave_proto::SendMessageRequest {
            mls_message: format!("msg_{i}").into_bytes(),
        };
        let mut body = Vec::new();
        req_body.encode(&mut body).unwrap();
        let request = Request::builder()
            .method("POST")
            .uri(format!("/api/v1/groups/{group_id}/messages"))
            .header(header::CONTENT_TYPE, "application/x-protobuf")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::from(body))
            .unwrap();
        app.clone().oneshot(request).await.unwrap();
    }

    // Request with limit=1000, which should be capped to 500.
    let request = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/v1/groups/{group_id}/messages?after=0&limit=1000"
        ))
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let resp = conclave_proto::GetMessagesResponse::decode(body_bytes).unwrap();
    assert_eq!(resp.messages.len(), 500);
}

// ── Group Name Validation ─────────────────────────────────────────

#[tokio::test]
async fn test_create_group_alias_exactly_64_chars() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    let group_alias = "g".repeat(64);
    let group_id = create_group_for(&app, &token, &group_alias).await;
    assert!(group_id > 0);
}

// ── Auth Header Format ────────────────────────────────────────────

#[tokio::test]
async fn test_auth_header_without_bearer_prefix() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    let request = Request::builder()
        .method("GET")
        .uri("/api/v1/me")
        .header(header::AUTHORIZATION, format!("Token {token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_auth_header_empty_bearer() {
    let app = setup();

    let request = Request::builder()
        .method("GET")
        .uri("/api/v1/me")
        .header(header::AUTHORIZATION, "Bearer ")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

// ── Process Commit Atomicity ──────────────────────────────────────

#[tokio::test]
async fn test_escrow_invite_multiple_members() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;

    let bob_id = register_user(&app, "bob", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;

    let charlie_id = register_user(&app, "charlie", "password123").await;
    let charlie_token = login_user(&app, "charlie", "password123").await;

    let group_id = create_group_for(&app, &alice_token, "multi_welcome").await;

    // Add both bob and charlie via escrow invites.
    add_member_via_escrow(&app, &alice_token, &bob_token, group_id, bob_id).await;
    add_member_via_escrow(&app, &alice_token, &charlie_token, group_id, charlie_id).await;

    // Verify both bob and charlie are now group members (can send messages).
    for (token, sender_name) in [(&bob_token, "bob"), (&charlie_token, "charlie")] {
        let req_body = conclave_proto::SendMessageRequest {
            mls_message: format!("hello from {sender_name}").into_bytes(),
        };
        let mut body = Vec::new();
        req_body.encode(&mut body).unwrap();
        let request = Request::builder()
            .method("POST")
            .uri(format!("/api/v1/groups/{group_id}/messages"))
            .header(header::CONTENT_TYPE, "application/x-protobuf")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::from(body))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "{sender_name} should be a group member after escrow invite"
        );
    }
}

// ── Leave Group Stores Group Info ─────────────────────────────────

#[tokio::test]
async fn test_leave_group_stores_group_info() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let bob_id = register_user(&app, "bob", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;
    let group_id = create_group_for(&app, &alice_token, "leave_gi").await;
    add_member_via_escrow(&app, &alice_token, &bob_token, group_id, bob_id).await;

    // Bob leaves with group_info attached.
    let req_body = conclave_proto::LeaveGroupRequest {
        commit_message: b"bob_leave_commit".to_vec(),
        group_info: b"leave_group_info_data".to_vec(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();
    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/leave"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {bob_token}"))
        .body(Body::from(body))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Verify the group info was stored and is retrievable by alice.
    let request = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/groups/{group_id}/group-info"))
        .header(header::AUTHORIZATION, format!("Bearer {alice_token}"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let resp = conclave_proto::GetGroupInfoResponse::decode(body_bytes).unwrap();
    assert_eq!(resp.group_info, b"leave_group_info_data");
}

// ── External Join Commit Stored as Message ────────────────────────

#[tokio::test]
async fn test_external_join_commit_stored_as_message() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let group_id = create_group_for(&app, &alice_token, "ext_msg_group").await;

    // Register Bob and add him to the group via escrow invite.
    let bob_id = register_user(&app, "bob", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;
    upload_key_package_for(&app, &bob_token, &fake_key_package(b"bob_kp")).await;
    add_member_via_escrow(&app, &alice_token, &bob_token, group_id, bob_id).await;

    // Bob (existing member) performs an external join with a commit_message.
    let req_body = conclave_proto::ExternalJoinRequest {
        commit_message: b"external_join_commit_data".to_vec(),
        mls_group_id: String::new(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();
    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/external-join"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {bob_token}"))
        .body(Body::from(body))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Alice retrieves messages and verifies the external join commit is present.
    let request = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/v1/groups/{group_id}/messages?after=0&limit=500"
        ))
        .header(header::AUTHORIZATION, format!("Bearer {alice_token}"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let resp = conclave_proto::GetMessagesResponse::decode(body_bytes).unwrap();
    let has_external_join_commit = resp
        .messages
        .iter()
        .any(|m| m.mls_message == b"external_join_commit_data");
    assert!(
        has_external_join_commit,
        "external join commit message should be stored and retrievable"
    );
}

// ── Security: External Join Requires Membership ──────────────────

#[tokio::test]
async fn test_external_join_requires_membership() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let group_id = create_group_for(&app, &alice_token, "private_group").await;

    // Alice uploads commit with group_info.
    let commit_body = conclave_proto::UploadCommitRequest {
        commit_message: vec![],

        group_info: b"group_info_data".to_vec(),
        mls_group_id: String::new(),
    };
    let mut cbody = Vec::new();
    commit_body.encode(&mut cbody).unwrap();
    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/commit"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {alice_token}"))
        .body(Body::from(cbody))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Eve (not a group member) attempts an external join.
    register_user(&app, "eve", "password123").await;
    let eve_token = login_user(&app, "eve", "password123").await;

    let req_body = conclave_proto::ExternalJoinRequest {
        commit_message: b"eve_commit".to_vec(),
        mls_group_id: String::new(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();
    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/external-join"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {eve_token}"))
        .body(Body::from(body))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "non-member should be rejected from external join"
    );
}

// ── PATCH /api/v1/me Tests ───────────────────────────────────────

#[tokio::test]
async fn test_update_profile_alias() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    let request_body = conclave_proto::UpdateProfileRequest {
        alias: "Alice W.".to_string(),
    };
    let mut body = Vec::new();
    request_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("PATCH")
        .uri("/api/v1/me")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let request = Request::builder()
        .method("GET")
        .uri("/api/v1/me")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let profile = conclave_proto::UserInfoResponse::decode(body_bytes).unwrap();
    assert_eq!(profile.alias, "Alice W.");
}

#[tokio::test]
async fn test_update_profile_clear_alias() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    let request_body = conclave_proto::UpdateProfileRequest {
        alias: "Alice W.".to_string(),
    };
    let mut body = Vec::new();
    request_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("PATCH")
        .uri("/api/v1/me")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let request_body = conclave_proto::UpdateProfileRequest {
        alias: String::new(),
    };
    let mut body = Vec::new();
    request_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("PATCH")
        .uri("/api/v1/me")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let request = Request::builder()
        .method("GET")
        .uri("/api/v1/me")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let profile = conclave_proto::UserInfoResponse::decode(body_bytes).unwrap();
    assert_eq!(profile.alias, "");
}

#[tokio::test]
async fn test_update_profile_invalid_alias() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    let request_body = conclave_proto::UpdateProfileRequest {
        alias: "bad\x00alias".to_string(),
    };
    let mut body = Vec::new();
    request_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("PATCH")
        .uri("/api/v1/me")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_update_profile_unauthenticated() {
    let app = setup();

    let request_body = conclave_proto::UpdateProfileRequest {
        alias: "Alice W.".to_string(),
    };
    let mut body = Vec::new();
    request_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("PATCH")
        .uri("/api/v1/me")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

// ── PATCH /api/v1/groups/{id} Tests ──────────────────────────────

#[tokio::test]
async fn test_update_group_alias_by_creator() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;
    let group_id = create_group_for(&app, &token, "original_alias").await;

    let request_body = conclave_proto::UpdateGroupRequest {
        alias: "updated_alias".to_string(),
        group_name: "alias_update_group".to_string(),
    };
    let mut body = Vec::new();
    request_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("PATCH")
        .uri(format!("/api/v1/groups/{group_id}"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let request = Request::builder()
        .method("GET")
        .uri("/api/v1/groups")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let groups_response = conclave_proto::ListGroupsResponse::decode(body_bytes).unwrap();
    assert_eq!(groups_response.groups.len(), 1);
    assert_eq!(groups_response.groups[0].alias, "updated_alias");
}

#[tokio::test]
async fn test_update_group_name_by_creator() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;
    let group_id = create_group_for(&app, &token, "my_group").await;

    let request_body = conclave_proto::UpdateGroupRequest {
        alias: String::new(),
        group_name: "new_group_name".to_string(),
    };
    let mut body = Vec::new();
    request_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("PATCH")
        .uri(format!("/api/v1/groups/{group_id}"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let request = Request::builder()
        .method("GET")
        .uri("/api/v1/groups")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let groups_response = conclave_proto::ListGroupsResponse::decode(body_bytes).unwrap();
    assert_eq!(groups_response.groups.len(), 1);
    assert_eq!(groups_response.groups[0].group_name, "new_group_name");
}

#[tokio::test]
async fn test_update_group_non_creator_rejected() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    register_user(&app, "bob", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;
    upload_key_package_for(&app, &bob_token, &fake_key_package(b"bob_kp")).await;

    let group_id = create_group_for(&app, &alice_token, "alice_group").await;

    let request_body = conclave_proto::UpdateGroupRequest {
        alias: "hijacked".to_string(),
        group_name: "non_creator_update_group".to_string(),
    };
    let mut body = Vec::new();
    request_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("PATCH")
        .uri(format!("/api/v1/groups/{group_id}"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {bob_token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_update_group_not_found() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    let request_body = conclave_proto::UpdateGroupRequest {
        alias: "phantom".to_string(),
        group_name: "not_found_update_group".to_string(),
    };
    let mut body = Vec::new();
    request_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("PATCH")
        .uri("/api/v1/groups/99999")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();

    // Non-existent group returns 401 since the user is not an admin.
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_update_group_duplicate_group_name() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    let first_group_body = conclave_proto::CreateGroupRequest {
        alias: "first".to_string(),

        group_name: "unique_name".to_string(),
    };
    let mut body = Vec::new();
    first_group_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/groups")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    let second_group_id = create_group_for(&app, &token, "second").await;

    let request_body = conclave_proto::UpdateGroupRequest {
        alias: String::new(),
        group_name: "unique_name".to_string(),
    };
    let mut body = Vec::new();
    request_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("PATCH")
        .uri(format!("/api/v1/groups/{second_group_id}"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn test_update_group_invalid_alias() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;
    let group_id = create_group_for(&app, &token, "my_group").await;

    let request_body = conclave_proto::UpdateGroupRequest {
        alias: "a".repeat(65),
        group_name: "invalid_alias_group".to_string(),
    };
    let mut body = Vec::new();
    request_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("PATCH")
        .uri(format!("/api/v1/groups/{group_id}"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_update_group_removed_admin_rejected() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;

    let group_id = create_group_for(&app, &alice_token, "admin_rm_test").await;

    // Alice leaves the group.
    let leave_body = conclave_proto::LeaveGroupRequest {
        commit_message: b"leave_commit".to_vec(),
        group_info: b"gi".to_vec(),
    };
    let mut body = Vec::new();
    leave_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/leave"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {alice_token}"))
        .body(Body::from(body))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Alice (removed admin) tries to update the group — should be rejected.
    let update_body = conclave_proto::UpdateGroupRequest {
        alias: "hijacked".to_string(),
        group_name: String::new(),
    };
    let mut body = Vec::new();
    update_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("PATCH")
        .uri(format!("/api/v1/groups/{group_id}"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {alice_token}"))
        .body(Body::from(body))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

// ── Group Name Validation ────────────────────────────────────────

#[tokio::test]
async fn test_create_group_name_with_dot_rejected() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    let req_body = conclave_proto::CreateGroupRequest {
        alias: String::new(),

        group_name: "my.group".to_string(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/groups")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_create_group_name_with_hyphen_rejected() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    let req_body = conclave_proto::CreateGroupRequest {
        alias: String::new(),

        group_name: "my-group".to_string(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/groups")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_create_group_name_with_space_rejected() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    let req_body = conclave_proto::CreateGroupRequest {
        alias: String::new(),

        group_name: "my group".to_string(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/groups")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_create_group_name_with_unicode_rejected() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    let req_body = conclave_proto::CreateGroupRequest {
        alias: String::new(),

        group_name: "gr\u{00fc}ppe".to_string(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/groups")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_create_group_name_starting_with_underscore_rejected() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    let req_body = conclave_proto::CreateGroupRequest {
        alias: String::new(),

        group_name: "_mygroup".to_string(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/groups")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_create_group_name_empty_rejected() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    let req_body = conclave_proto::CreateGroupRequest {
        alias: String::new(),

        group_name: String::new(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/groups")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_create_group_name_too_long_rejected() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    let req_body = conclave_proto::CreateGroupRequest {
        alias: String::new(),

        group_name: "a".repeat(65),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/groups")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_update_group_name_with_dot_rejected() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;
    let group_id = create_group_for(&app, &token, "my_group").await;

    let request_body = conclave_proto::UpdateGroupRequest {
        alias: String::new(),
        group_name: "new.name".to_string(),
    };
    let mut body = Vec::new();
    request_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("PATCH")
        .uri(format!("/api/v1/groups/{group_id}"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_update_group_name_with_hyphen_rejected() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;
    let group_id = create_group_for(&app, &token, "my_group2").await;

    let request_body = conclave_proto::UpdateGroupRequest {
        alias: String::new(),
        group_name: "new-name".to_string(),
    };
    let mut body = Vec::new();
    request_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("PATCH")
        .uri(format!("/api/v1/groups/{group_id}"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_update_profile_broadcasts_to_group_members() {
    let (app, app_state) = setup_with_state();
    let mut rx = app_state.sse_tx.subscribe();

    register_user(&app, "alice", "password123").await;
    let bob_id = register_user(&app, "bob", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;

    upload_key_package_for(&app, &bob_token, &fake_key_package(b"bob1")).await;
    let group_id = create_group_for(&app, &alice_token, "test").await;

    // Add bob as a group member via escrow invite.
    add_member_via_escrow(&app, &alice_token, &bob_token, group_id, bob_id).await;

    // Drain any events from group creation and escrow invite.
    while rx.try_recv().is_ok() {}

    // Alice updates her profile.
    let request_body = conclave_proto::UpdateProfileRequest {
        alias: "Alice W.".to_string(),
    };
    let mut body = Vec::new();
    request_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("PATCH")
        .uri("/api/v1/me")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {alice_token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Bob should receive a GroupUpdateEvent with update_type "member_profile".
    let event = rx.try_recv().expect("expected SSE event for bob");
    let server_event = conclave_proto::ServerEvent::decode(event.data.as_slice()).unwrap();
    match server_event.event {
        Some(conclave_proto::server_event::Event::GroupUpdate(update)) => {
            assert_eq!(update.update_type, "member_profile");
        }
        other => panic!("expected GroupUpdate event, got {other:?}"),
    }
}

#[tokio::test]
async fn test_update_profile_no_broadcast_without_groups() {
    let (app, app_state) = setup_with_state();
    let mut rx = app_state.sse_tx.subscribe();

    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    let request_body = conclave_proto::UpdateProfileRequest {
        alias: "Alice W.".to_string(),
    };
    let mut body = Vec::new();
    request_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("PATCH")
        .uri("/api/v1/me")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // No groups → no broadcast.
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn test_update_group_broadcasts_to_members() {
    let (app, app_state) = setup_with_state();
    let mut rx = app_state.sse_tx.subscribe();

    register_user(&app, "alice", "password123").await;
    let bob_id = register_user(&app, "bob", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;

    upload_key_package_for(&app, &bob_token, &fake_key_package(b"bob1")).await;
    let group_id = create_group_for(&app, &alice_token, "test").await;

    // Add bob as a group member via escrow invite.
    add_member_via_escrow(&app, &alice_token, &bob_token, group_id, bob_id).await;

    // Drain any events from group creation and escrow invite.
    while rx.try_recv().is_ok() {}

    // Alice updates the group alias.
    let request_body = conclave_proto::UpdateGroupRequest {
        alias: "new_topic".to_string(),
        group_name: "broadcast_update_group".to_string(),
    };
    let mut body = Vec::new();
    request_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("PATCH")
        .uri(format!("/api/v1/groups/{group_id}"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {alice_token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Bob should receive a GroupUpdateEvent with update_type "group_settings".
    let event = rx.try_recv().expect("expected SSE event for bob");
    let server_event = conclave_proto::ServerEvent::decode(event.data.as_slice()).unwrap();
    match server_event.event {
        Some(conclave_proto::server_event::Event::GroupUpdate(update)) => {
            assert_eq!(update.group_id, group_id);
            assert_eq!(update.update_type, "group_settings");
        }
        other => panic!("expected GroupUpdate event, got {other:?}"),
    }
}

// ── Helper: add a user to a group via escrow invite flow ──────────

async fn add_member_via_escrow(
    app: &Router,
    admin_token: &str,
    member_token: &str,
    group_id: i64,
    member_id: i64,
) {
    // Step 1: Admin creates escrow invite
    let req_body = conclave_proto::EscrowInviteRequest {
        invitee_id: member_id,
        commit_message: b"add_member".to_vec(),
        welcome_message: b"welcome".to_vec(),
        group_info: b"gi".to_vec(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();
    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/escrow-invite"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {admin_token}"))
        .body(Body::from(body))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Step 2: Get the invite ID
    let request = Request::builder()
        .method("GET")
        .uri("/api/v1/invites")
        .header(header::AUTHORIZATION, format!("Bearer {member_token}"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let resp = conclave_proto::ListPendingInvitesResponse::decode(body_bytes).unwrap();
    let invite = resp
        .invites
        .iter()
        .find(|i| i.group_id == group_id)
        .expect("expected pending invite for group");

    // Step 3: Member accepts the invite
    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/invites/{}/accept", invite.invite_id))
        .header(header::AUTHORIZATION, format!("Bearer {member_token}"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

// ── Admin role tests ──────────────────────────────────────────────

#[tokio::test]
async fn test_promote_member_success() {
    let app = setup();

    register_user(&app, "alice", "password123").await;
    let bob_id = register_user(&app, "bob", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;

    upload_key_package_for(&app, &bob_token, &fake_key_package(b"bob1")).await;
    let group_id = create_group_for(&app, &alice_token, "test_promote").await;
    add_member_via_escrow(&app, &alice_token, &bob_token, group_id, bob_id).await;

    // Alice promotes Bob.
    let request_body = conclave_proto::PromoteMemberRequest { user_id: bob_id };
    let mut body = Vec::new();
    request_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/promote"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {alice_token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_promote_member_not_admin_rejected() {
    let app = setup();

    register_user(&app, "alice", "password123").await;
    let bob_id = register_user(&app, "bob", "password123").await;
    let charlie_id = register_user(&app, "charlie", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;
    let charlie_token = login_user(&app, "charlie", "password123").await;

    let group_id = create_group_for(&app, &alice_token, "test_promote_nonadmin").await;
    add_member_via_escrow(&app, &alice_token, &bob_token, group_id, bob_id).await;
    add_member_via_escrow(&app, &alice_token, &charlie_token, group_id, charlie_id).await;

    // Bob (regular member) tries to promote Charlie.
    let request_body = conclave_proto::PromoteMemberRequest {
        user_id: charlie_id,
    };
    let mut body = Vec::new();
    request_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/promote"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {bob_token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_promote_member_already_admin() {
    let app = setup();

    register_user(&app, "alice", "password123").await;
    let bob_id = register_user(&app, "bob", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;

    upload_key_package_for(&app, &bob_token, &fake_key_package(b"bob1")).await;
    let group_id = create_group_for(&app, &alice_token, "test_promote_dup").await;
    add_member_via_escrow(&app, &alice_token, &bob_token, group_id, bob_id).await;

    // Promote Bob first.
    let request_body = conclave_proto::PromoteMemberRequest { user_id: bob_id };
    let mut body = Vec::new();
    request_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/promote"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {alice_token}"))
        .body(Body::from(body))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Promote Bob again — should conflict (409).
    let request_body = conclave_proto::PromoteMemberRequest { user_id: bob_id };
    let mut body = Vec::new();
    request_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/promote"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {alice_token}"))
        .body(Body::from(body))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn test_promote_member_not_member_rejected() {
    let app = setup();

    register_user(&app, "alice", "password123").await;
    let bob_id = register_user(&app, "bob", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;

    let group_id = create_group_for(&app, &alice_token, "test_promote_nonmember").await;

    // Try to promote Bob who is not a member.
    let request_body = conclave_proto::PromoteMemberRequest { user_id: bob_id };
    let mut body = Vec::new();
    request_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/promote"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {alice_token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_demote_member_success() {
    let app = setup();

    register_user(&app, "alice", "password123").await;
    let bob_id = register_user(&app, "bob", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;

    upload_key_package_for(&app, &bob_token, &fake_key_package(b"bob1")).await;
    let group_id = create_group_for(&app, &alice_token, "test_demote").await;
    add_member_via_escrow(&app, &alice_token, &bob_token, group_id, bob_id).await;

    // Promote Bob first.
    let request_body = conclave_proto::PromoteMemberRequest { user_id: bob_id };
    let mut body = Vec::new();
    request_body.encode(&mut body).unwrap();
    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/promote"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {alice_token}"))
        .body(Body::from(body))
        .unwrap();
    app.clone().oneshot(request).await.unwrap();

    // Alice demotes Bob.
    let request_body = conclave_proto::DemoteMemberRequest { user_id: bob_id };
    let mut body = Vec::new();
    request_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/demote"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {alice_token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_demote_member_not_admin_rejected() {
    let app = setup();

    let alice_id = register_user(&app, "alice", "password123").await;
    let bob_id = register_user(&app, "bob", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;

    upload_key_package_for(&app, &bob_token, &fake_key_package(b"bob1")).await;
    let group_id = create_group_for(&app, &alice_token, "test_demote_nonadmin").await;
    add_member_via_escrow(&app, &alice_token, &bob_token, group_id, bob_id).await;

    // Bob (regular member) tries to demote Alice.
    let request_body = conclave_proto::DemoteMemberRequest { user_id: alice_id };
    let mut body = Vec::new();
    request_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/demote"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {bob_token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_demote_last_admin_rejected() {
    let app = setup();

    let alice_id = register_user(&app, "alice", "password123").await;
    let bob_id = register_user(&app, "bob", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;

    upload_key_package_for(&app, &bob_token, &fake_key_package(b"bob1")).await;
    let group_id = create_group_for(&app, &alice_token, "test_demote_last").await;
    add_member_via_escrow(&app, &alice_token, &bob_token, group_id, bob_id).await;

    // Alice is the only admin — try to demote herself.
    let request_body = conclave_proto::DemoteMemberRequest { user_id: alice_id };
    let mut body = Vec::new();
    request_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/demote"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {alice_token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_demote_regular_member_rejected() {
    let app = setup();

    register_user(&app, "alice", "password123").await;
    let bob_id = register_user(&app, "bob", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;

    upload_key_package_for(&app, &bob_token, &fake_key_package(b"bob1")).await;
    let group_id = create_group_for(&app, &alice_token, "test_demote_regular").await;
    add_member_via_escrow(&app, &alice_token, &bob_token, group_id, bob_id).await;

    // Alice tries to demote Bob who is not an admin.
    let request_body = conclave_proto::DemoteMemberRequest { user_id: bob_id };
    let mut body = Vec::new();
    request_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/demote"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {alice_token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_list_admins_success() {
    let app = setup();

    register_user(&app, "alice", "password123").await;
    let bob_id = register_user(&app, "bob", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;

    upload_key_package_for(&app, &bob_token, &fake_key_package(b"bob1")).await;
    let group_id = create_group_for(&app, &alice_token, "test_admins").await;
    add_member_via_escrow(&app, &alice_token, &bob_token, group_id, bob_id).await;

    let request = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/groups/{group_id}/admins"))
        .header(header::AUTHORIZATION, format!("Bearer {alice_token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let resp = conclave_proto::ListAdminsResponse::decode(body_bytes).unwrap();
    assert_eq!(resp.admins.len(), 1);
    assert_eq!(resp.admins[0].username, "alice");
    assert_eq!(resp.admins[0].role, "admin");
}

#[tokio::test]
async fn test_list_admins_not_member_rejected() {
    let app = setup();

    register_user(&app, "alice", "password123").await;
    register_user(&app, "bob", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;

    let group_id = create_group_for(&app, &alice_token, "test_admins_nonmember").await;

    // Bob is not a member.
    let request = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/groups/{group_id}/admins"))
        .header(header::AUTHORIZATION, format!("Bearer {bob_token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_invite_requires_admin() {
    let app = setup();

    register_user(&app, "alice", "password123").await;
    let bob_id = register_user(&app, "bob", "password123").await;
    let charlie_id = register_user(&app, "charlie", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;

    upload_key_package_for(&app, &bob_token, &fake_key_package(b"bob1")).await;
    upload_key_package_for(
        &app,
        &login_user(&app, "charlie", "password123").await.as_str(),
        &fake_key_package(b"charlie1"),
    )
    .await;
    let group_id = create_group_for(&app, &alice_token, "test_invite_admin").await;
    add_member_via_escrow(&app, &alice_token, &bob_token, group_id, bob_id).await;

    // Bob (regular member) tries to invite Charlie.
    let request_body = conclave_proto::InviteToGroupRequest {
        user_ids: vec![charlie_id],
    };
    let mut body = Vec::new();
    request_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/invite"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {bob_token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_remove_member_requires_admin() {
    let app = setup();

    register_user(&app, "alice", "password123").await;
    let bob_id = register_user(&app, "bob", "password123").await;
    let charlie_id = register_user(&app, "charlie", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;
    let charlie_token = login_user(&app, "charlie", "password123").await;

    let group_id = create_group_for(&app, &alice_token, "test_remove_admin").await;
    add_member_via_escrow(&app, &alice_token, &bob_token, group_id, bob_id).await;
    add_member_via_escrow(&app, &alice_token, &charlie_token, group_id, charlie_id).await;

    // Bob (regular member) tries to remove Charlie.
    let request_body = conclave_proto::RemoveMemberRequest {
        user_id: charlie_id,
        commit_message: b"remove".to_vec(),
        group_info: b"gi".to_vec(),
    };
    let mut body = Vec::new();
    request_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/remove"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {bob_token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_update_group_requires_admin() {
    let app = setup();

    register_user(&app, "alice", "password123").await;
    let bob_id = register_user(&app, "bob", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;

    upload_key_package_for(&app, &bob_token, &fake_key_package(b"bob1")).await;
    let group_id = create_group_for(&app, &alice_token, "test_update_admin").await;
    add_member_via_escrow(&app, &alice_token, &bob_token, group_id, bob_id).await;

    // Bob (regular member) tries to update the group.
    let request_body = conclave_proto::UpdateGroupRequest {
        alias: "new_alias".to_string(),
        group_name: "new_name".to_string(),
    };
    let mut body = Vec::new();
    request_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("PATCH")
        .uri(format!("/api/v1/groups/{group_id}"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {bob_token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_group_member_role_in_list_groups() {
    let app = setup();

    register_user(&app, "alice", "password123").await;
    let bob_id = register_user(&app, "bob", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;

    upload_key_package_for(&app, &bob_token, &fake_key_package(b"bob1")).await;
    let group_id = create_group_for(&app, &alice_token, "test_roles_list").await;
    add_member_via_escrow(&app, &alice_token, &bob_token, group_id, bob_id).await;

    // List groups and check member roles.
    let request = Request::builder()
        .method("GET")
        .uri("/api/v1/groups")
        .header(header::AUTHORIZATION, format!("Bearer {alice_token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let resp = conclave_proto::ListGroupsResponse::decode(body_bytes).unwrap();
    assert_eq!(resp.groups.len(), 1);

    let group = &resp.groups[0];
    let alice_member = group
        .members
        .iter()
        .find(|m| m.username == "alice")
        .unwrap();
    assert_eq!(alice_member.role, "admin");

    let bob_member = group.members.iter().find(|m| m.username == "bob").unwrap();
    assert_eq!(bob_member.role, "member");
}

// ── Escrow Invite Tests ──────────────────────────────────────────

async fn create_escrow_invite(
    app: &Router,
    admin_token: &str,
    group_id: i64,
    invitee_id: i64,
) -> StatusCode {
    let req_body = conclave_proto::EscrowInviteRequest {
        invitee_id,
        commit_message: b"commit_data".to_vec(),
        welcome_message: b"welcome_data".to_vec(),
        group_info: b"group_info_data".to_vec(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/escrow-invite"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {admin_token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    response.status()
}

async fn list_pending_invites(app: &Router, token: &str) -> Vec<conclave_proto::PendingInvite> {
    let request = Request::builder()
        .method("GET")
        .uri("/api/v1/invites")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let resp = conclave_proto::ListPendingInvitesResponse::decode(body_bytes).unwrap();
    resp.invites
}

#[tokio::test]
async fn test_escrow_invite_success() {
    let app = setup();

    register_user(&app, "alice", "password123").await;
    let bob_id = register_user(&app, "bob", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;

    let group_id = create_group_for(&app, &alice_token, "escrow_test").await;

    let status = create_escrow_invite(&app, &alice_token, group_id, bob_id).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn test_escrow_invite_not_admin() {
    let app = setup();

    register_user(&app, "alice", "password123").await;
    let bob_id = register_user(&app, "bob", "password123").await;
    let charlie_id = register_user(&app, "charlie", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;

    upload_key_package_for(&app, &bob_token, &fake_key_package(b"bob1")).await;
    let group_id = create_group_for(&app, &alice_token, "escrow_not_admin").await;
    add_member_via_escrow(&app, &alice_token, &bob_token, group_id, bob_id).await;

    // Bob is a regular member, not an admin.
    let status = create_escrow_invite(&app, &bob_token, group_id, charlie_id).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_escrow_invite_nonexistent_user() {
    let app = setup();

    register_user(&app, "alice", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;

    let group_id = create_group_for(&app, &alice_token, "escrow_nonexistent").await;

    let status = create_escrow_invite(&app, &alice_token, group_id, 999999).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_escrow_invite_already_member() {
    let app = setup();

    register_user(&app, "alice", "password123").await;
    let bob_id = register_user(&app, "bob", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;

    let group_id = create_group_for(&app, &alice_token, "escrow_already_member").await;
    add_member_via_escrow(&app, &alice_token, &bob_token, group_id, bob_id).await;

    // Bob is already a member.
    let status = create_escrow_invite(&app, &alice_token, group_id, bob_id).await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn test_escrow_invite_missing_fields() {
    let app = setup();

    register_user(&app, "alice", "password123").await;
    let bob_id = register_user(&app, "bob", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;

    let group_id = create_group_for(&app, &alice_token, "escrow_missing").await;

    // Zero invitee_id.
    let req_body = conclave_proto::EscrowInviteRequest {
        invitee_id: 0,
        commit_message: b"commit".to_vec(),
        welcome_message: b"welcome".to_vec(),
        group_info: b"ginfo".to_vec(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/escrow-invite"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {alice_token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // Empty commit_message.
    let req_body = conclave_proto::EscrowInviteRequest {
        invitee_id: bob_id,
        commit_message: vec![],
        welcome_message: b"welcome".to_vec(),
        group_info: b"ginfo".to_vec(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/escrow-invite"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {alice_token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // Empty welcome_message.
    let req_body = conclave_proto::EscrowInviteRequest {
        invitee_id: bob_id,
        commit_message: b"commit".to_vec(),
        welcome_message: vec![],
        group_info: b"ginfo".to_vec(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/escrow-invite"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {alice_token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // Empty group_info.
    let req_body = conclave_proto::EscrowInviteRequest {
        invitee_id: bob_id,
        commit_message: b"commit".to_vec(),
        welcome_message: b"welcome".to_vec(),
        group_info: vec![],
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/escrow-invite"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {alice_token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_list_pending_invites_empty() {
    let app = setup();

    register_user(&app, "alice", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;

    let invites = list_pending_invites(&app, &alice_token).await;
    assert!(invites.is_empty());
}

#[tokio::test]
async fn test_list_pending_invites_success() {
    let app = setup();

    let alice_id = register_user(&app, "alice", "password123").await;
    let bob_id = register_user(&app, "bob", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;

    let group_id = create_group_for(&app, &alice_token, "escrow_list").await;

    create_escrow_invite(&app, &alice_token, group_id, bob_id).await;

    // Bob should see the invite.
    let invites = list_pending_invites(&app, &bob_token).await;
    assert_eq!(invites.len(), 1);
    assert_eq!(invites[0].group_id, group_id);
    assert_eq!(invites[0].group_name, "escrow_list");
    assert_eq!(invites[0].inviter_username, "alice");
    assert_eq!(invites[0].inviter_id, alice_id);
    assert!(invites[0].invite_id > 0);

    // Alice should not see any invites (she is the inviter, not invitee).
    let alice_invites = list_pending_invites(&app, &alice_token).await;
    assert!(alice_invites.is_empty());
}

#[tokio::test]
async fn test_accept_invite_success() {
    let app = setup();

    register_user(&app, "alice", "password123").await;
    let bob_id = register_user(&app, "bob", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;

    let group_id = create_group_for(&app, &alice_token, "escrow_accept").await;

    create_escrow_invite(&app, &alice_token, group_id, bob_id).await;

    let invites = list_pending_invites(&app, &bob_token).await;
    assert_eq!(invites.len(), 1);
    let invite_id = invites[0].invite_id;

    // Bob accepts the invite.
    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/invites/{invite_id}/accept"))
        .header(header::AUTHORIZATION, format!("Bearer {bob_token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Verify Bob's groups now include the group.
    let request = Request::builder()
        .method("GET")
        .uri("/api/v1/groups")
        .header(header::AUTHORIZATION, format!("Bearer {bob_token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let resp = conclave_proto::ListGroupsResponse::decode(body_bytes).unwrap();
    assert_eq!(resp.groups.len(), 1);
    assert_eq!(resp.groups[0].group_id, group_id);

    // Verify the invite is gone.
    let invites = list_pending_invites(&app, &bob_token).await;
    assert!(invites.is_empty());
}

#[tokio::test]
async fn test_accept_invite_not_invitee() {
    let app = setup();

    register_user(&app, "alice", "password123").await;
    let bob_id = register_user(&app, "bob", "password123").await;
    register_user(&app, "charlie", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;
    let charlie_token = login_user(&app, "charlie", "password123").await;

    let group_id = create_group_for(&app, &alice_token, "escrow_not_invitee").await;

    create_escrow_invite(&app, &alice_token, group_id, bob_id).await;

    let invites = list_pending_invites(&app, &bob_token).await;
    assert_eq!(invites.len(), 1);
    let invite_id = invites[0].invite_id;

    // Charlie tries to accept Bob's invite.
    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/invites/{invite_id}/accept"))
        .header(header::AUTHORIZATION, format!("Bearer {charlie_token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_accept_invite_not_found() {
    let app = setup();

    register_user(&app, "alice", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/invites/99999/accept")
        .header(header::AUTHORIZATION, format!("Bearer {alice_token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_decline_invite_success() {
    let app = setup();

    register_user(&app, "alice", "password123").await;
    let bob_id = register_user(&app, "bob", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;

    let group_id = create_group_for(&app, &alice_token, "escrow_decline").await;

    create_escrow_invite(&app, &alice_token, group_id, bob_id).await;

    let invites = list_pending_invites(&app, &bob_token).await;
    assert_eq!(invites.len(), 1);
    let invite_id = invites[0].invite_id;

    // Bob declines the invite.
    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/invites/{invite_id}/decline"))
        .header(header::AUTHORIZATION, format!("Bearer {bob_token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Verify the invite is gone.
    let invites = list_pending_invites(&app, &bob_token).await;
    assert!(invites.is_empty());

    // Verify Bob is NOT a member of the group.
    let request = Request::builder()
        .method("GET")
        .uri("/api/v1/groups")
        .header(header::AUTHORIZATION, format!("Bearer {bob_token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let resp = conclave_proto::ListGroupsResponse::decode(body_bytes).unwrap();
    assert!(resp.groups.is_empty());
}

#[tokio::test]
async fn test_decline_invite_not_invitee() {
    let app = setup();

    register_user(&app, "alice", "password123").await;
    let bob_id = register_user(&app, "bob", "password123").await;
    register_user(&app, "charlie", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;
    let charlie_token = login_user(&app, "charlie", "password123").await;

    let group_id = create_group_for(&app, &alice_token, "escrow_decline_not_invitee").await;

    create_escrow_invite(&app, &alice_token, group_id, bob_id).await;

    let invites = list_pending_invites(&app, &bob_token).await;
    assert_eq!(invites.len(), 1);
    let invite_id = invites[0].invite_id;

    // Charlie tries to decline Bob's invite.
    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/invites/{invite_id}/decline"))
        .header(header::AUTHORIZATION, format!("Bearer {charlie_token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_escrow_invite_duplicate() {
    let app = setup();

    register_user(&app, "alice", "password123").await;
    let bob_id = register_user(&app, "bob", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;

    let group_id = create_group_for(&app, &alice_token, "escrow_duplicate").await;

    // First invite should succeed.
    let status = create_escrow_invite(&app, &alice_token, group_id, bob_id).await;
    assert_eq!(status, StatusCode::OK);

    // Second invite for the same group+user should fail due to UNIQUE constraint.
    let status = create_escrow_invite(&app, &alice_token, group_id, bob_id).await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn test_accept_invite_twice() {
    let app = setup();

    register_user(&app, "alice", "password123").await;
    let bob_id = register_user(&app, "bob", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;

    let group_id = create_group_for(&app, &alice_token, "escrow_double_accept").await;

    create_escrow_invite(&app, &alice_token, group_id, bob_id).await;

    let invites = list_pending_invites(&app, &bob_token).await;
    assert_eq!(invites.len(), 1);
    let invite_id = invites[0].invite_id;

    // First accept should succeed.
    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/invites/{invite_id}/accept"))
        .header(header::AUTHORIZATION, format!("Bearer {bob_token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Second accept should fail (invite no longer exists).
    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/invites/{invite_id}/accept"))
        .header(header::AUTHORIZATION, format!("Bearer {bob_token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_decline_invite_twice() {
    let app = setup();

    register_user(&app, "alice", "password123").await;
    let bob_id = register_user(&app, "bob", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;

    let group_id = create_group_for(&app, &alice_token, "escrow_double_decline").await;

    create_escrow_invite(&app, &alice_token, group_id, bob_id).await;

    let invites = list_pending_invites(&app, &bob_token).await;
    assert_eq!(invites.len(), 1);
    let invite_id = invites[0].invite_id;

    // First decline should succeed.
    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/invites/{invite_id}/decline"))
        .header(header::AUTHORIZATION, format!("Bearer {bob_token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Second decline should fail (invite no longer exists).
    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/invites/{invite_id}/decline"))
        .header(header::AUTHORIZATION, format!("Bearer {bob_token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_escrow_invite_already_member_via_creation() {
    let app = setup();

    register_user(&app, "alice", "password123").await;
    let bob_id = register_user(&app, "bob", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;

    // Create a group and add bob via escrow.
    let group_id = create_group_for(&app, &alice_token, "escrow_already_member").await;
    add_member_via_escrow(&app, &alice_token, &bob_token, group_id, bob_id).await;

    // Trying to escrow-invite bob should fail because he's already a member.
    let status = create_escrow_invite(&app, &alice_token, group_id, bob_id).await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn test_escrow_invite_unauthenticated() {
    let app = setup();

    // Attempt to create escrow invite without auth.
    let request_body = conclave_proto::EscrowInviteRequest {
        invitee_id: 999,
        commit_message: vec![1, 2, 3],
        welcome_message: vec![4, 5, 6],
        group_info: vec![7, 8, 9],
    };

    let body = prost::Message::encode_to_vec(&request_body);
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/groups/1/escrow-invite")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_list_invites_unauthenticated() {
    let app = setup();

    let request = Request::builder()
        .method("GET")
        .uri("/api/v1/invites")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_accept_invite_unauthenticated() {
    let app = setup();

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/invites/1/accept")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_decline_invite_unauthenticated() {
    let app = setup();

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/invites/1/decline")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

// ── List group pending invites ───────────────────────────────────

async fn list_group_pending_invites(
    app: &Router,
    token: &str,
    group_id: i64,
) -> (StatusCode, Vec<conclave_proto::PendingInvite>) {
    let request = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/groups/{group_id}/invites"))
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    if status != StatusCode::OK {
        return (status, vec![]);
    }

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let resp = conclave_proto::ListGroupPendingInvitesResponse::decode(body_bytes).unwrap();
    (status, resp.invites)
}

#[tokio::test]
async fn test_list_group_pending_invites_empty() {
    let app = setup();

    register_user(&app, "alice", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let group_id = create_group_for(&app, &alice_token, "empty_invites").await;

    let (status, invites) = list_group_pending_invites(&app, &alice_token, group_id).await;
    assert_eq!(status, StatusCode::OK);
    assert!(invites.is_empty());
}

#[tokio::test]
async fn test_list_group_pending_invites_success() {
    let app = setup();

    register_user(&app, "alice", "password123").await;
    let bob_id = register_user(&app, "bob", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let group_id = create_group_for(&app, &alice_token, "list_invites").await;

    create_escrow_invite(&app, &alice_token, group_id, bob_id).await;

    let (status, invites) = list_group_pending_invites(&app, &alice_token, group_id).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(invites.len(), 1);
    assert_eq!(invites[0].group_id, group_id);
    assert_eq!(invites[0].invitee_id, bob_id);
}

#[tokio::test]
async fn test_list_group_pending_invites_not_admin() {
    let app = setup();

    register_user(&app, "alice", "password123").await;
    let bob_id = register_user(&app, "bob", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;

    upload_key_package_for(&app, &bob_token, &fake_key_package(b"bob1")).await;
    let group_id = create_group_for(&app, &alice_token, "not_admin_list").await;
    add_member_via_escrow(&app, &alice_token, &bob_token, group_id, bob_id).await;

    let (status, _) = list_group_pending_invites(&app, &bob_token, group_id).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

// ── Cancel invite ────────────────────────────────────────────────

async fn cancel_invite(app: &Router, token: &str, group_id: i64, invitee_id: i64) -> StatusCode {
    let req_body = conclave_proto::CancelInviteRequest { invitee_id };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/cancel-invite"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    response.status()
}

#[tokio::test]
async fn test_cancel_invite_success() {
    let app = setup();

    register_user(&app, "alice", "password123").await;
    let bob_id = register_user(&app, "bob", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let group_id = create_group_for(&app, &alice_token, "cancel_test").await;

    create_escrow_invite(&app, &alice_token, group_id, bob_id).await;

    let status = cancel_invite(&app, &alice_token, group_id, bob_id).await;
    assert_eq!(status, StatusCode::OK);

    // Verify the invite is gone.
    let (_, invites) = list_group_pending_invites(&app, &alice_token, group_id).await;
    assert!(invites.is_empty());
}

#[tokio::test]
async fn test_cancel_invite_not_found() {
    let app = setup();

    register_user(&app, "alice", "password123").await;
    let bob_id = register_user(&app, "bob", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let group_id = create_group_for(&app, &alice_token, "cancel_not_found").await;

    let status = cancel_invite(&app, &alice_token, group_id, bob_id).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_cancel_invite_not_admin() {
    let app = setup();

    register_user(&app, "alice", "password123").await;
    let bob_id = register_user(&app, "bob", "password123").await;
    let charlie_id = register_user(&app, "charlie", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;

    upload_key_package_for(&app, &bob_token, &fake_key_package(b"bob1")).await;
    let group_id = create_group_for(&app, &alice_token, "cancel_not_admin").await;
    add_member_via_escrow(&app, &alice_token, &bob_token, group_id, bob_id).await;

    create_escrow_invite(&app, &alice_token, group_id, charlie_id).await;

    let charlie_id = {
        let request = Request::builder()
            .method("GET")
            .uri("/api/v1/users/charlie")
            .header(header::AUTHORIZATION, format!("Bearer {alice_token}"))
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
        conclave_proto::UserInfoResponse::decode(body_bytes)
            .unwrap()
            .user_id
    };

    // Bob is a regular member, not admin.
    let status = cancel_invite(&app, &bob_token, group_id, charlie_id).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_cancel_invite_nonexistent_user() {
    let app = setup();

    register_user(&app, "alice", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let group_id = create_group_for(&app, &alice_token, "cancel_nouser").await;

    let status = cancel_invite(&app, &alice_token, group_id, 99999).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
