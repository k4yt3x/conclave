use std::collections::HashMap;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use prost::Message;
use tower::ServiceExt;

use conclave_server::{api, config, db, state};

// ── Helpers ────────────────────────────────────────────────────────

fn fake_key_package(label: &[u8]) -> Vec<u8> {
    // MLS 1.0 version (0x0001) + mls_key_package wire format (0x0003) + arbitrary payload.
    let mut data = vec![0x00, 0x01, 0x00, 0x03];
    data.extend_from_slice(label);
    data
}

fn setup() -> Router {
    let database = db::Database::open_in_memory().unwrap();
    let config = config::ServerConfig::default();
    let app_state = Arc::new(state::AppState::new(database, config));
    api::router().with_state(app_state)
}

async fn register_user(app: &Router, username: &str, password: &str) -> u64 {
    let req_body = conclave_proto::RegisterRequest {
        username: username.to_string(),
        password: password.to_string(),
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

async fn create_group_for(app: &Router, token: &str, name: &str, members: Vec<String>) -> String {
    let req_body = conclave_proto::CreateGroupRequest {
        name: name.to_string(),
        member_usernames: members,
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

    let group_id = create_group_for(&app, &token, "test-group", vec![]).await;
    assert!(!group_id.is_empty());
}

#[tokio::test]
async fn test_create_group_empty_name() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    let req_body = conclave_proto::CreateGroupRequest {
        name: "".to_string(),
        member_usernames: vec![],
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
async fn test_create_group_long_name() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;

    let long_name = "g".repeat(129);
    let req_body = conclave_proto::CreateGroupRequest {
        name: long_name,
        member_usernames: vec![],
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

    let group_id = create_group_for(&app, &token, "my-group", vec![]).await;

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
    assert_eq!(resp.groups[0].name, "my-group");
}

// ── Message Tests (25-29) ─────────────────────────────────────────

#[tokio::test]
async fn test_send_message_success() {
    let app = setup();
    register_user(&app, "alice", "password123").await;
    let token = login_user(&app, "alice", "password123").await;
    let group_id = create_group_for(&app, &token, "msg-group", vec![]).await;

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
    let group_id = create_group_for(&app, &alice_token, "alice-group", vec![]).await;

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
    let group_id = create_group_for(&app, &token, "msg-group", vec![]).await;

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
    let group_id = create_group_for(&app, &alice_token, "alice-group", vec![]).await;

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
    let group_id = create_group_for(&app, &token, "msg-group", vec![]).await;

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
    let group_id = create_group_for(&app, &token, "inv-group", vec![]).await;

    let req_body = conclave_proto::InviteToGroupRequest { usernames: vec![] };
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
    let group_id = create_group_for(&app, &alice_token, "alice-group", vec![]).await;

    // Bob is not a member and tries to invite
    register_user(&app, "bob", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;

    // Charlie exists to be invited
    register_user(&app, "charlie", "password123").await;

    let req_body = conclave_proto::InviteToGroupRequest {
        usernames: vec!["charlie".to_string()],
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

    // Register bob and upload a key package for him
    register_user(&app, "bob", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;
    upload_key_package_for(&app, &bob_token, &fake_key_package(b"bob_key_package")).await;

    // Create group with bob as a member
    let group_id = create_group_for(&app, &alice_token, "rm-group", vec!["bob".to_string()]).await;

    // Alice needs to upload a commit with welcome for bob so bob gets added
    let mut welcomes = HashMap::new();
    welcomes.insert("bob".to_string(), b"welcome_for_bob".to_vec());
    let req_body = conclave_proto::UploadCommitRequest {
        commit_message: b"add_bob_commit".to_vec(),
        welcome_messages: welcomes,
        group_info: b"info".to_vec(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/commit"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {alice_token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Now alice removes bob
    let req_body = conclave_proto::RemoveMemberRequest {
        username: "bob".to_string(),
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
    register_user(&app, "alice", "password123").await;
    let alice_token = login_user(&app, "alice", "password123").await;
    let group_id = create_group_for(&app, &alice_token, "rm-group", vec![]).await;

    // Bob is not a group member
    register_user(&app, "bob", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;

    let req_body = conclave_proto::RemoveMemberRequest {
        username: "alice".to_string(),
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
    let group_id = create_group_for(&app, &alice_token, "rm-group", vec![]).await;

    // Bob exists but is not in the group
    register_user(&app, "bob", "password123").await;

    let req_body = conclave_proto::RemoveMemberRequest {
        username: "bob".to_string(),
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
    let group_id = create_group_for(&app, &alice_token, "rm-group", vec![]).await;

    let req_body = conclave_proto::RemoveMemberRequest {
        username: "nonexistent_user".to_string(),
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

    // Register bob and upload a key package
    register_user(&app, "bob", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;
    upload_key_package_for(&app, &bob_token, &fake_key_package(b"bob_kp")).await;

    // Create group with bob
    let group_id =
        create_group_for(&app, &alice_token, "leave-group", vec!["bob".to_string()]).await;

    // Upload commit with welcome to add bob as member
    let mut welcomes = HashMap::new();
    welcomes.insert("bob".to_string(), b"welcome_bob".to_vec());
    let req_body = conclave_proto::UploadCommitRequest {
        commit_message: b"add_bob".to_vec(),
        welcome_messages: welcomes,
        group_info: b"info".to_vec(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/commit"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {alice_token}"))
        .body(Body::from(body))
        .unwrap();

    app.clone().oneshot(request).await.unwrap();

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
    let group_id = create_group_for(&app, &alice_token, "leave-group", vec![]).await;

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
    let group_id = create_group_for(&app, &token, "info-group", vec![]).await;

    // Store group info via upload_commit
    let req_body = conclave_proto::UploadCommitRequest {
        commit_message: b"commit".to_vec(),
        welcome_messages: HashMap::new(),
        group_info: b"group_info_data".to_vec(),
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
    let group_id = create_group_for(&app, &token, "info-group", vec![]).await;

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
    let group_id = create_group_for(&app, &alice_token, "info-group", vec![]).await;

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
    let group_id = create_group_for(&app, &alice_token, "ext-group", vec![]).await;

    // Alice uploads a commit with group_info so external join is possible.
    let commit_body = conclave_proto::UploadCommitRequest {
        commit_message: vec![],
        welcome_messages: std::collections::HashMap::new(),
        group_info: b"fake_group_info".to_vec(),
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

    // Bob performs an external join
    register_user(&app, "bob", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;

    let req_body = conclave_proto::ExternalJoinRequest {
        commit_message: b"ext_commit".to_vec(),
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

    // Verify bob is now a member by sending a message
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
    let group_id = create_group_for(&app, &token, "commit-group", vec![]).await;

    // Upload commit with group_info
    let req_body = conclave_proto::UploadCommitRequest {
        commit_message: b"commit_msg".to_vec(),
        welcome_messages: HashMap::new(),
        group_info: b"stored_group_info".to_vec(),
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
    let group_id = create_group_for(&app, &alice_token, "commit-group", vec![]).await;

    register_user(&app, "bob", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;

    let req_body = conclave_proto::UploadCommitRequest {
        commit_message: b"commit".to_vec(),
        welcome_messages: HashMap::new(),
        group_info: b"info".to_vec(),
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

    register_user(&app, "bob", "password123").await;
    let bob_token = login_user(&app, "bob", "password123").await;
    upload_key_package_for(&app, &bob_token, &fake_key_package(b"bob_kp")).await;

    // Create group with bob as member
    let group_id =
        create_group_for(&app, &alice_token, "welcome-group", vec!["bob".to_string()]).await;

    // Upload commit with welcome for bob
    let mut welcomes = HashMap::new();
    welcomes.insert("bob".to_string(), b"welcome_data_for_bob".to_vec());
    let req_body = conclave_proto::UploadCommitRequest {
        commit_message: b"add_bob".to_vec(),
        welcome_messages: welcomes,
        group_info: b"info".to_vec(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{group_id}/commit"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {alice_token}"))
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Bob lists pending welcomes to get the welcome_id
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
    assert_eq!(resp.welcomes[0].welcome_message, b"welcome_data_for_bob");

    // We need the DB-level welcome ID. The proto PendingWelcome doesn't expose it directly,
    // but the accept endpoint uses it from the path. The DB assigns sequential IDs starting
    // from 1, so the first welcome will have id=1.
    let welcome_id = 1;

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
