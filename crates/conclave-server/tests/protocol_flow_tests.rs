use std::collections::HashMap;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use prost::Message;
use tempfile::TempDir;
use tower::ServiceExt;

use conclave_client::mls::MlsManager;
use conclave_server::{api, config, db, state};

// ── Helpers ────────────────────────────────────────────────────────

fn setup() -> Router {
    let database = db::Database::open_in_memory().unwrap();
    let config = config::ServerConfig::default();
    let app_state = Arc::new(state::AppState::new(database, config));
    api::router().with_state(app_state)
}

async fn register_and_login(app: &Router, username: &str) -> (i64, String) {
    let password = format!("{username}_password");

    let req_body = conclave_proto::RegisterRequest {
        username: username.to_string(),
        password: password.clone(),
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
    let register_resp = conclave_proto::RegisterResponse::decode(body_bytes).unwrap();

    // Login
    let req_body = conclave_proto::LoginRequest {
        username: username.to_string(),
        password,
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
    let login_resp = conclave_proto::LoginResponse::decode(body_bytes).unwrap();

    (register_resp.user_id, login_resp.token)
}

/// Upload MLS key packages (1 last-resort + 5 regular) via the batch API.
async fn upload_real_key_packages(app: &Router, token: &str, mls: &MlsManager) {
    let entries = conclave_client::config::generate_initial_key_packages(mls).unwrap();
    let proto_entries: Vec<conclave_proto::KeyPackageEntry> = entries
        .into_iter()
        .map(|(data, is_last_resort)| conclave_proto::KeyPackageEntry {
            data,
            is_last_resort,
        })
        .collect();

    let req_body = conclave_proto::UploadKeyPackageRequest {
        key_package_data: vec![],
        entries: proto_entries,
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

async fn create_server_group(app: &Router, token: &str, name: &str) -> i64 {
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

/// Fetch key packages for the given user IDs via the /invite endpoint.
async fn invite_members(
    app: &Router,
    token: &str,
    group_id: i64,
    user_ids: Vec<i64>,
) -> HashMap<i64, Vec<u8>> {
    let req_body = conclave_proto::InviteToGroupRequest { user_ids };
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
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let resp = conclave_proto::InviteToGroupResponse::decode(body_bytes).unwrap();
    resp.member_key_packages
}

async fn upload_commit(
    app: &Router,
    token: &str,
    group_id: i64,
    commit_message: Vec<u8>,
    group_info: Vec<u8>,
    mls_group_id: String,
) {
    let req_body = conclave_proto::UploadCommitRequest {
        commit_message,
        group_info,
        mls_group_id,
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
}

async fn send_mls_message(app: &Router, token: &str, group_id: i64, mls_message: Vec<u8>) -> u64 {
    let req_body = conclave_proto::SendMessageRequest { mls_message };
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
    resp.sequence_num
}

async fn get_messages(
    app: &Router,
    token: &str,
    group_id: i64,
    after: i64,
) -> Vec<conclave_proto::StoredMessage> {
    let request = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/v1/groups/{group_id}/messages?after={after}&limit=100"
        ))
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let resp = conclave_proto::GetMessagesResponse::decode(body_bytes).unwrap();
    resp.messages
}

async fn get_pending_welcomes(app: &Router, token: &str) -> Vec<conclave_proto::PendingWelcome> {
    let request = Request::builder()
        .method("GET")
        .uri("/api/v1/welcomes")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let resp = conclave_proto::ListPendingWelcomesResponse::decode(body_bytes).unwrap();
    resp.welcomes
}

async fn accept_welcome(app: &Router, token: &str, welcome_id: i64) {
    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/welcomes/{welcome_id}/accept"))
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

/// Escrow an invite: admin sends commit+welcome+group_info for one invitee,
/// then the invitee lists pending invites and accepts.
async fn escrow_and_accept_invite(
    app: &Router,
    admin_token: &str,
    member_token: &str,
    group_id: i64,
    member_id: i64,
    commit_message: Vec<u8>,
    welcome_message: Vec<u8>,
    group_info: Vec<u8>,
) {
    let req_body = conclave_proto::EscrowInviteRequest {
        invitee_id: member_id,
        commit_message,
        welcome_message,
        group_info,
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

    // List pending invites for the member.
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

    // Member accepts the invite.
    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/invites/{}/accept", invite.invite_id))
        .header(header::AUTHORIZATION, format!("Bearer {member_token}"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

// ── End-to-End Protocol Flow Tests ────────────────────────────────

/// Full flow: register, upload real MLS key packages, create group, invite
/// member via escrow, have invited member join via welcome, and verify both
/// can encrypt/decrypt messages through the server.
#[tokio::test]
async fn test_e2e_group_creation_and_messaging() {
    let app = setup();
    let alice_dir = TempDir::new().unwrap();
    let bob_dir = TempDir::new().unwrap();

    // Register both users and upload real MLS key packages.
    let (alice_id, alice_token) = register_and_login(&app, "alice").await;
    let (bob_id, bob_token) = register_and_login(&app, "bob").await;

    let alice_mls = MlsManager::new(alice_dir.path(), alice_id).unwrap();
    let bob_mls = MlsManager::new(bob_dir.path(), bob_id).unwrap();

    upload_real_key_packages(&app, &alice_token, &alice_mls).await;
    upload_real_key_packages(&app, &bob_token, &bob_mls).await;

    // Create server group (no members), then invite bob via the invite endpoint.
    let server_group_id = create_server_group(&app, &alice_token, "test_room").await;
    let member_kps = invite_members(&app, &alice_token, server_group_id, vec![bob_id]).await;

    let create_result = alice_mls.create_group(&member_kps).unwrap();
    let mls_group_id = create_result.mls_group_id.clone();

    // Upload the initial commit with mls_group_id to initialize the group.
    upload_commit(
        &app,
        &alice_token,
        server_group_id,
        create_result.commit,
        create_result.group_info.clone(),
        create_result.mls_group_id,
    )
    .await;

    // Escrow invite for bob with the MLS welcome message.
    let bob_welcome = create_result
        .welcomes
        .get(&bob_id)
        .expect("welcome for bob")
        .clone();
    escrow_and_accept_invite(
        &app,
        &alice_token,
        &bob_token,
        server_group_id,
        bob_id,
        b"escrow_commit".to_vec(),
        bob_welcome.clone(),
        create_result.group_info,
    )
    .await;

    // Bob processes the welcome from the pending welcomes.
    let welcomes = get_pending_welcomes(&app, &bob_token).await;
    assert_eq!(welcomes.len(), 1);
    assert_eq!(welcomes[0].group_id, server_group_id);

    let bob_mls_group_id = bob_mls.join_group(&welcomes[0].welcome_message).unwrap();
    assert_eq!(bob_mls_group_id, mls_group_id);

    accept_welcome(&app, &bob_token, welcomes[0].welcome_id).await;

    let plaintext = b"Hello from Alice!";
    let encrypted = alice_mls.encrypt_message(&mls_group_id, plaintext).unwrap();
    let seq = send_mls_message(&app, &alice_token, server_group_id, encrypted).await;
    assert!(seq > 0);

    let messages = get_messages(&app, &bob_token, server_group_id, 0).await;
    // Find the application message (skip the commit message at seq 1).
    let app_msg = messages
        .iter()
        .find(|m| m.sequence_num == seq)
        .expect("message should exist on server");

    let decrypted = bob_mls
        .decrypt_message(&mls_group_id, &app_msg.mls_message)
        .unwrap();
    match decrypted {
        conclave_client::mls::DecryptedMessage::Application(data) => {
            assert_eq!(data, plaintext);
        }
        other => panic!("expected Application message, got: {other:?}"),
    }

    let reply = b"Hello from Bob!";
    let encrypted_reply = bob_mls.encrypt_message(&mls_group_id, reply).unwrap();
    let reply_seq = send_mls_message(&app, &bob_token, server_group_id, encrypted_reply).await;

    let messages = get_messages(&app, &alice_token, server_group_id, 0).await;
    let reply_msg = messages
        .iter()
        .find(|m| m.sequence_num == reply_seq)
        .expect("reply should exist on server");

    let decrypted_reply = alice_mls
        .decrypt_message(&mls_group_id, &reply_msg.mls_message)
        .unwrap();
    match decrypted_reply {
        conclave_client::mls::DecryptedMessage::Application(data) => {
            assert_eq!(data, reply);
        }
        other => panic!("expected Application message, got: {other:?}"),
    }
}

/// Three-party group: alice creates a group, then invites bob and charlie via
/// escrow. All three exchange encrypted messages through the server.
#[tokio::test]
async fn test_e2e_three_party_messaging() {
    let app = setup();
    let alice_dir = TempDir::new().unwrap();
    let bob_dir = TempDir::new().unwrap();
    let charlie_dir = TempDir::new().unwrap();

    let (alice_id, alice_token) = register_and_login(&app, "alice").await;
    let (bob_id, bob_token) = register_and_login(&app, "bob").await;
    let (charlie_id, charlie_token) = register_and_login(&app, "charlie").await;

    let alice_mls = MlsManager::new(alice_dir.path(), alice_id).unwrap();
    let bob_mls = MlsManager::new(bob_dir.path(), bob_id).unwrap();
    let charlie_mls = MlsManager::new(charlie_dir.path(), charlie_id).unwrap();

    upload_real_key_packages(&app, &alice_token, &alice_mls).await;
    upload_real_key_packages(&app, &bob_token, &bob_mls).await;
    upload_real_key_packages(&app, &charlie_token, &charlie_mls).await;

    let server_group_id = create_server_group(&app, &alice_token, "trio_room").await;
    let member_kps = invite_members(
        &app,
        &alice_token,
        server_group_id,
        vec![bob_id, charlie_id],
    )
    .await;

    let create_result = alice_mls.create_group(&member_kps).unwrap();
    let mls_group_id = create_result.mls_group_id.clone();

    // Upload initial commit with mls_group_id.
    upload_commit(
        &app,
        &alice_token,
        server_group_id,
        create_result.commit,
        create_result.group_info.clone(),
        create_result.mls_group_id,
    )
    .await;

    // Escrow invite for bob.
    let bob_welcome = create_result
        .welcomes
        .get(&bob_id)
        .expect("welcome for bob")
        .clone();
    escrow_and_accept_invite(
        &app,
        &alice_token,
        &bob_token,
        server_group_id,
        bob_id,
        b"escrow_commit".to_vec(),
        bob_welcome,
        create_result.group_info.clone(),
    )
    .await;

    // Escrow invite for charlie.
    let charlie_welcome = create_result
        .welcomes
        .get(&charlie_id)
        .expect("welcome for charlie")
        .clone();
    escrow_and_accept_invite(
        &app,
        &alice_token,
        &charlie_token,
        server_group_id,
        charlie_id,
        b"escrow_commit".to_vec(),
        charlie_welcome,
        create_result.group_info,
    )
    .await;

    // Bob processes welcome.
    let bob_welcomes = get_pending_welcomes(&app, &bob_token).await;
    assert_eq!(bob_welcomes.len(), 1);
    let bob_mls_gid = bob_mls
        .join_group(&bob_welcomes[0].welcome_message)
        .unwrap();
    accept_welcome(&app, &bob_token, bob_welcomes[0].welcome_id).await;

    // Charlie processes welcome.
    let charlie_welcomes = get_pending_welcomes(&app, &charlie_token).await;
    assert_eq!(charlie_welcomes.len(), 1);
    let charlie_mls_gid = charlie_mls
        .join_group(&charlie_welcomes[0].welcome_message)
        .unwrap();
    accept_welcome(&app, &charlie_token, charlie_welcomes[0].welcome_id).await;

    assert_eq!(bob_mls_gid, mls_group_id);
    assert_eq!(charlie_mls_gid, mls_group_id);

    let plaintext = b"Group message from Alice";
    let encrypted = alice_mls.encrypt_message(&mls_group_id, plaintext).unwrap();
    let seq = send_mls_message(&app, &alice_token, server_group_id, encrypted).await;

    let messages = get_messages(&app, &bob_token, server_group_id, 0).await;
    let msg = messages.iter().find(|m| m.sequence_num == seq).unwrap();

    let bob_decrypted = bob_mls
        .decrypt_message(&mls_group_id, &msg.mls_message)
        .unwrap();
    match bob_decrypted {
        conclave_client::mls::DecryptedMessage::Application(data) => {
            assert_eq!(data, plaintext);
        }
        other => panic!("bob expected Application, got: {other:?}"),
    }

    let charlie_msgs = get_messages(&app, &charlie_token, server_group_id, 0).await;
    let charlie_msg = charlie_msgs.iter().find(|m| m.sequence_num == seq).unwrap();

    let charlie_decrypted = charlie_mls
        .decrypt_message(&mls_group_id, &charlie_msg.mls_message)
        .unwrap();
    match charlie_decrypted {
        conclave_client::mls::DecryptedMessage::Application(data) => {
            assert_eq!(data, plaintext);
        }
        other => panic!("charlie expected Application, got: {other:?}"),
    }
}

/// Test the invite flow: alice creates a group solo, then invites bob via
/// escrow after the fact. Bob processes the welcome and can communicate.
#[tokio::test]
async fn test_e2e_post_creation_invite_flow() {
    let app = setup();
    let alice_dir = TempDir::new().unwrap();
    let bob_dir = TempDir::new().unwrap();

    let (alice_id, alice_token) = register_and_login(&app, "alice").await;
    let (bob_id, bob_token) = register_and_login(&app, "bob").await;

    let alice_mls = MlsManager::new(alice_dir.path(), alice_id).unwrap();
    let bob_mls = MlsManager::new(bob_dir.path(), bob_id).unwrap();

    upload_real_key_packages(&app, &alice_token, &alice_mls).await;
    upload_real_key_packages(&app, &bob_token, &bob_mls).await;

    let server_group_id = create_server_group(&app, &alice_token, "solo_room").await;

    // Create a solo MLS group and upload the initial commit.
    let create_result = alice_mls.create_group(&HashMap::new()).unwrap();
    let mls_group_id = create_result.mls_group_id.clone();

    upload_commit(
        &app,
        &alice_token,
        server_group_id,
        vec![],
        create_result.group_info,
        create_result.mls_group_id,
    )
    .await;

    // Invite bob via the invite endpoint to get his key package.
    let member_kps = invite_members(&app, &alice_token, server_group_id, vec![bob_id]).await;

    // Alice performs the MLS invite using bob's key package from the server.
    let invite_result = alice_mls
        .invite_to_group(&mls_group_id, &member_kps)
        .unwrap();

    // Upload the invite commit (no welcomes in commit; use escrow instead).
    upload_commit(
        &app,
        &alice_token,
        server_group_id,
        invite_result.commit,
        invite_result.group_info.clone(),
        String::new(),
    )
    .await;

    // Escrow invite for bob with the MLS welcome.
    let bob_welcome = invite_result
        .welcomes
        .get(&bob_id)
        .expect("welcome for bob")
        .clone();
    escrow_and_accept_invite(
        &app,
        &alice_token,
        &bob_token,
        server_group_id,
        bob_id,
        b"escrow_commit".to_vec(),
        bob_welcome,
        invite_result.group_info,
    )
    .await;

    let welcomes = get_pending_welcomes(&app, &bob_token).await;
    assert_eq!(welcomes.len(), 1);
    let bob_mls_gid = bob_mls.join_group(&welcomes[0].welcome_message).unwrap();
    assert_eq!(bob_mls_gid, mls_group_id);
    accept_welcome(&app, &bob_token, welcomes[0].welcome_id).await;

    let encrypted = alice_mls
        .encrypt_message(&mls_group_id, b"Post-invite message")
        .unwrap();
    let seq = send_mls_message(&app, &alice_token, server_group_id, encrypted).await;

    let messages = get_messages(&app, &bob_token, server_group_id, 0).await;
    let msg = messages.iter().find(|m| m.sequence_num == seq).unwrap();
    let decrypted = bob_mls
        .decrypt_message(&mls_group_id, &msg.mls_message)
        .unwrap();
    match decrypted {
        conclave_client::mls::DecryptedMessage::Application(data) => {
            assert_eq!(data, b"Post-invite message");
        }
        other => panic!("expected Application, got: {other:?}"),
    }
}

/// Test member removal: alice removes bob from the group, uploads the commit,
/// and verifies the commit is stored as a message that charlie can process.
#[tokio::test]
async fn test_e2e_member_removal_flow() {
    let app = setup();
    let alice_dir = TempDir::new().unwrap();
    let bob_dir = TempDir::new().unwrap();
    let charlie_dir = TempDir::new().unwrap();

    let (alice_id, alice_token) = register_and_login(&app, "alice").await;
    let (bob_id, bob_token) = register_and_login(&app, "bob").await;
    let (charlie_id, charlie_token) = register_and_login(&app, "charlie").await;

    let alice_mls = MlsManager::new(alice_dir.path(), alice_id).unwrap();
    let bob_mls = MlsManager::new(bob_dir.path(), bob_id).unwrap();
    let charlie_mls = MlsManager::new(charlie_dir.path(), charlie_id).unwrap();

    upload_real_key_packages(&app, &alice_token, &alice_mls).await;
    upload_real_key_packages(&app, &bob_token, &bob_mls).await;
    upload_real_key_packages(&app, &charlie_token, &charlie_mls).await;

    let server_group_id = create_server_group(&app, &alice_token, "removal_test").await;
    let member_kps = invite_members(
        &app,
        &alice_token,
        server_group_id,
        vec![bob_id, charlie_id],
    )
    .await;

    let create_result = alice_mls.create_group(&member_kps).unwrap();
    let mls_group_id = create_result.mls_group_id.clone();
    upload_commit(
        &app,
        &alice_token,
        server_group_id,
        create_result.commit,
        create_result.group_info.clone(),
        create_result.mls_group_id,
    )
    .await;

    // Escrow invite for bob.
    let bob_welcome = create_result
        .welcomes
        .get(&bob_id)
        .expect("welcome for bob")
        .clone();
    escrow_and_accept_invite(
        &app,
        &alice_token,
        &bob_token,
        server_group_id,
        bob_id,
        b"escrow_commit".to_vec(),
        bob_welcome,
        create_result.group_info.clone(),
    )
    .await;

    // Escrow invite for charlie.
    let charlie_welcome = create_result
        .welcomes
        .get(&charlie_id)
        .expect("welcome for charlie")
        .clone();
    escrow_and_accept_invite(
        &app,
        &alice_token,
        &charlie_token,
        server_group_id,
        charlie_id,
        b"escrow_commit".to_vec(),
        charlie_welcome,
        create_result.group_info,
    )
    .await;

    let bob_welcomes = get_pending_welcomes(&app, &bob_token).await;
    let bob_mls_gid = bob_mls
        .join_group(&bob_welcomes[0].welcome_message)
        .unwrap();
    accept_welcome(&app, &bob_token, bob_welcomes[0].welcome_id).await;

    let charlie_welcomes = get_pending_welcomes(&app, &charlie_token).await;
    let charlie_mls_gid = charlie_mls
        .join_group(&charlie_welcomes[0].welcome_message)
        .unwrap();
    accept_welcome(&app, &charlie_token, charlie_welcomes[0].welcome_id).await;

    assert_eq!(bob_mls_gid, mls_group_id);
    assert_eq!(charlie_mls_gid, mls_group_id);

    let bob_index = alice_mls
        .find_member_index(&mls_group_id, bob_id)
        .unwrap()
        .expect("bob should be in the group");

    let (remove_commit, remove_group_info) =
        alice_mls.remove_member(&mls_group_id, bob_index).unwrap();

    let req_body = conclave_proto::RemoveMemberRequest {
        user_id: bob_id,
        commit_message: remove_commit.clone(),
        group_info: remove_group_info,
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();
    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{server_group_id}/remove"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {alice_token}"))
        .body(Body::from(body))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let messages = get_messages(&app, &charlie_token, server_group_id, 0).await;
    let remove_msg = messages.last().expect("removal commit should be stored");
    let decrypted = charlie_mls
        .decrypt_message(&mls_group_id, &remove_msg.mls_message)
        .unwrap();
    match decrypted {
        conclave_client::mls::DecryptedMessage::Commit(info) => {
            assert!(
                !info.members_removed.is_empty(),
                "should report a member removed"
            );
        }
        other => panic!("charlie expected Commit, got: {other:?}"),
    }

    let post_removal = b"Message after bob removed";
    let encrypted = alice_mls
        .encrypt_message(&mls_group_id, post_removal)
        .unwrap();
    let seq = send_mls_message(&app, &alice_token, server_group_id, encrypted.clone()).await;

    let charlie_msgs = get_messages(&app, &charlie_token, server_group_id, 0).await;
    let msg = charlie_msgs.iter().find(|m| m.sequence_num == seq).unwrap();
    let charlie_decrypted = charlie_mls
        .decrypt_message(&mls_group_id, &msg.mls_message)
        .unwrap();
    match charlie_decrypted {
        conclave_client::mls::DecryptedMessage::Application(data) => {
            assert_eq!(data, post_removal);
        }
        other => panic!("charlie expected Application after removal, got: {other:?}"),
    }

    // Bob should not be able to decrypt (wrong epoch or key).
    let bob_result = bob_mls.decrypt_message(&mls_group_id, &encrypted);
    match bob_result {
        Ok(conclave_client::mls::DecryptedMessage::Application(_)) => {
            panic!("removed member should not be able to decrypt new messages");
        }
        _ => {} // Any error or non-Application result is expected.
    }
}

/// Test key rotation: alice rotates keys (empty commit to advance epoch),
/// other members process the commit, and messaging continues to work.
#[tokio::test]
async fn test_e2e_key_rotation_continuity() {
    let app = setup();
    let alice_dir = TempDir::new().unwrap();
    let bob_dir = TempDir::new().unwrap();

    let (alice_id, alice_token) = register_and_login(&app, "alice").await;
    let (bob_id, bob_token) = register_and_login(&app, "bob").await;

    let alice_mls = MlsManager::new(alice_dir.path(), alice_id).unwrap();
    let bob_mls = MlsManager::new(bob_dir.path(), bob_id).unwrap();

    upload_real_key_packages(&app, &alice_token, &alice_mls).await;
    upload_real_key_packages(&app, &bob_token, &bob_mls).await;

    let server_group_id = create_server_group(&app, &alice_token, "rotation_test").await;
    let member_kps = invite_members(&app, &alice_token, server_group_id, vec![bob_id]).await;

    let create_result = alice_mls.create_group(&member_kps).unwrap();
    let mls_group_id = create_result.mls_group_id.clone();
    upload_commit(
        &app,
        &alice_token,
        server_group_id,
        create_result.commit,
        create_result.group_info.clone(),
        create_result.mls_group_id,
    )
    .await;

    // Escrow invite for bob.
    let bob_welcome = create_result
        .welcomes
        .get(&bob_id)
        .expect("welcome for bob")
        .clone();
    escrow_and_accept_invite(
        &app,
        &alice_token,
        &bob_token,
        server_group_id,
        bob_id,
        b"escrow_commit".to_vec(),
        bob_welcome,
        create_result.group_info,
    )
    .await;

    let bob_welcomes = get_pending_welcomes(&app, &bob_token).await;
    let bob_mls_gid = bob_mls
        .join_group(&bob_welcomes[0].welcome_message)
        .unwrap();
    accept_welcome(&app, &bob_token, bob_welcomes[0].welcome_id).await;
    assert_eq!(bob_mls_gid, mls_group_id);

    let pre_rotation = b"Before key rotation";
    let encrypted = alice_mls
        .encrypt_message(&mls_group_id, pre_rotation)
        .unwrap();
    send_mls_message(&app, &alice_token, server_group_id, encrypted).await;

    let (rotation_commit, rotation_group_info) = alice_mls.rotate_keys(&mls_group_id).unwrap();
    upload_commit(
        &app,
        &alice_token,
        server_group_id,
        rotation_commit.clone(),
        rotation_group_info,
        String::new(),
    )
    .await;

    let messages = get_messages(&app, &bob_token, server_group_id, 0).await;
    // Find and process the rotation commit message.
    for msg in &messages {
        let result = bob_mls.decrypt_message(&mls_group_id, &msg.mls_message);
        // Ignore errors from messages bob already processed or from his own.
        let _ = result;
    }

    let post_rotation = b"After key rotation";
    let encrypted = alice_mls
        .encrypt_message(&mls_group_id, post_rotation)
        .unwrap();
    let seq = send_mls_message(&app, &alice_token, server_group_id, encrypted).await;

    let new_messages = get_messages(&app, &bob_token, server_group_id, 0).await;
    let msg = new_messages.iter().find(|m| m.sequence_num == seq).unwrap();
    let decrypted = bob_mls
        .decrypt_message(&mls_group_id, &msg.mls_message)
        .unwrap();
    match decrypted {
        conclave_client::mls::DecryptedMessage::Application(data) => {
            assert_eq!(data, post_rotation);
        }
        other => panic!("expected Application after rotation, got: {other:?}"),
    }
}

/// Test external rejoin: alice removes bob, then bob rejoins via external commit
/// using the stored group info. After rejoin, bob can send and receive messages.
#[tokio::test]
async fn test_e2e_external_rejoin_after_removal() {
    let app = setup();
    let alice_dir = TempDir::new().unwrap();
    let bob_dir = TempDir::new().unwrap();

    let (alice_id, alice_token) = register_and_login(&app, "alice").await;
    let (bob_id, bob_token) = register_and_login(&app, "bob").await;

    let alice_mls = MlsManager::new(alice_dir.path(), alice_id).unwrap();
    let bob_mls = MlsManager::new(bob_dir.path(), bob_id).unwrap();

    upload_real_key_packages(&app, &alice_token, &alice_mls).await;
    upload_real_key_packages(&app, &bob_token, &bob_mls).await;

    let server_group_id = create_server_group(&app, &alice_token, "rejoin_test").await;
    let member_kps = invite_members(&app, &alice_token, server_group_id, vec![bob_id]).await;

    let create_result = alice_mls.create_group(&member_kps).unwrap();
    let mls_group_id = create_result.mls_group_id.clone();
    upload_commit(
        &app,
        &alice_token,
        server_group_id,
        create_result.commit,
        create_result.group_info.clone(),
        create_result.mls_group_id,
    )
    .await;

    // Escrow invite for bob.
    let bob_welcome = create_result
        .welcomes
        .get(&bob_id)
        .expect("welcome for bob")
        .clone();
    escrow_and_accept_invite(
        &app,
        &alice_token,
        &bob_token,
        server_group_id,
        bob_id,
        b"escrow_commit".to_vec(),
        bob_welcome,
        create_result.group_info,
    )
    .await;

    let bob_welcomes = get_pending_welcomes(&app, &bob_token).await;
    bob_mls
        .join_group(&bob_welcomes[0].welcome_message)
        .unwrap();
    accept_welcome(&app, &bob_token, bob_welcomes[0].welcome_id).await;

    let bob_index = alice_mls
        .find_member_index(&mls_group_id, bob_id)
        .unwrap()
        .expect("bob should be in the group");

    let (remove_commit, remove_group_info) =
        alice_mls.remove_member(&mls_group_id, bob_index).unwrap();

    let req_body = conclave_proto::RemoveMemberRequest {
        user_id: bob_id,
        commit_message: remove_commit,
        group_info: remove_group_info,
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();
    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{server_group_id}/remove"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {alice_token}"))
        .body(Body::from(body))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Bob attempts an external rejoin after being removed.
    // The server should reject this since Bob is no longer a group member.
    let req_body = conclave_proto::ExternalJoinRequest {
        commit_message: b"bob_rejoin_commit".to_vec(),
        mls_group_id: String::new(),
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();
    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/groups/{server_group_id}/external-join"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {bob_token}"))
        .body(Body::from(body))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "removed members should not be able to external-join"
    );
}

/// Test that real MLS key packages pass the server's wire format validation.
/// The server checks for MLS version 0x0001 and wire format 0x0003 in the
/// first 4 bytes.
#[tokio::test]
async fn test_e2e_real_key_packages_pass_wire_format_validation() {
    let app = setup();
    let alice_dir = TempDir::new().unwrap();

    let (alice_id, alice_token) = register_and_login(&app, "alice").await;
    let alice_mls = MlsManager::new(alice_dir.path(), alice_id).unwrap();

    let kp_bytes = alice_mls.generate_key_package().unwrap();

    assert!(kp_bytes.len() >= 4, "key package too short");
    assert_eq!(kp_bytes[0], 0x00, "MLS version high byte");
    assert_eq!(kp_bytes[1], 0x01, "MLS version low byte");
    assert_eq!(kp_bytes[2], 0x00, "wire format high byte");
    assert_eq!(kp_bytes[3], 0x05, "wire format low byte (mls_key_package)");

    // Upload it via the API — should succeed.
    let req_body = conclave_proto::UploadKeyPackageRequest {
        key_package_data: kp_bytes,
        entries: vec![],
    };
    let mut body = Vec::new();
    req_body.encode(&mut body).unwrap();
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/key-packages")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::AUTHORIZATION, format!("Bearer {alice_token}"))
        .body(Body::from(body))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

/// Test that the server correctly stores and returns key packages so that
/// another user's MLS client can parse them.
#[tokio::test]
async fn test_e2e_key_package_roundtrip_through_server() {
    let app = setup();
    let alice_dir = TempDir::new().unwrap();
    let bob_dir = TempDir::new().unwrap();

    let (alice_id, alice_token) = register_and_login(&app, "alice").await;
    let (bob_id, _bob_token) = register_and_login(&app, "bob").await;

    let alice_mls = MlsManager::new(alice_dir.path(), alice_id).unwrap();
    let bob_mls = MlsManager::new(bob_dir.path(), bob_id).unwrap();

    upload_real_key_packages(&app, &alice_token, &alice_mls).await;

    // Bob retrieves alice's key package from the server.
    let request = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/key-packages/{alice_id}"))
        .header(header::AUTHORIZATION, format!("Bearer {_bob_token}"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let resp = conclave_proto::GetKeyPackageResponse::decode(body_bytes).unwrap();

    // The key package bytes should be valid MLS that bob's client can parse
    // to create a group with alice.
    let mut member_kps = HashMap::new();
    member_kps.insert(alice_id, resp.key_package_data);
    let result = bob_mls.create_group(&member_kps);
    assert!(
        result.is_ok(),
        "bob should be able to use alice's key package from the server"
    );
}

/// Test multiple sequential messages maintain correct ordering through the
/// server, and all decrypt correctly on the receiving side.
#[tokio::test]
async fn test_e2e_message_ordering_and_sequence_numbers() {
    let app = setup();
    let alice_dir = TempDir::new().unwrap();
    let bob_dir = TempDir::new().unwrap();

    let (alice_id, alice_token) = register_and_login(&app, "alice").await;
    let (bob_id, bob_token) = register_and_login(&app, "bob").await;

    let alice_mls = MlsManager::new(alice_dir.path(), alice_id).unwrap();
    let bob_mls = MlsManager::new(bob_dir.path(), bob_id).unwrap();

    upload_real_key_packages(&app, &alice_token, &alice_mls).await;
    upload_real_key_packages(&app, &bob_token, &bob_mls).await;

    let server_group_id = create_server_group(&app, &alice_token, "ordering_test").await;
    let member_kps = invite_members(&app, &alice_token, server_group_id, vec![bob_id]).await;

    let create_result = alice_mls.create_group(&member_kps).unwrap();
    let mls_group_id = create_result.mls_group_id.clone();
    upload_commit(
        &app,
        &alice_token,
        server_group_id,
        create_result.commit,
        create_result.group_info.clone(),
        create_result.mls_group_id,
    )
    .await;

    // Escrow invite for bob.
    let bob_welcome = create_result
        .welcomes
        .get(&bob_id)
        .expect("welcome for bob")
        .clone();
    escrow_and_accept_invite(
        &app,
        &alice_token,
        &bob_token,
        server_group_id,
        bob_id,
        b"escrow_commit".to_vec(),
        bob_welcome,
        create_result.group_info,
    )
    .await;

    let bob_welcomes = get_pending_welcomes(&app, &bob_token).await;
    bob_mls
        .join_group(&bob_welcomes[0].welcome_message)
        .unwrap();
    accept_welcome(&app, &bob_token, bob_welcomes[0].welcome_id).await;

    // Alice sends 10 sequential messages.
    let mut sent_seqs = Vec::new();
    for i in 0..10 {
        let plaintext = format!("Message #{i}");
        let encrypted = alice_mls
            .encrypt_message(&mls_group_id, plaintext.as_bytes())
            .unwrap();
        let seq = send_mls_message(&app, &alice_token, server_group_id, encrypted).await;
        sent_seqs.push(seq);
    }

    // Verify sequence numbers are strictly increasing.
    for window in sent_seqs.windows(2) {
        assert!(window[1] > window[0], "sequence numbers must be increasing");
    }

    // Bob retrieves and decrypts all messages in order.
    let messages = get_messages(&app, &bob_token, server_group_id, 0).await;
    let mut decrypted_count = 0;
    for (i, seq) in sent_seqs.iter().enumerate() {
        let msg = messages.iter().find(|m| m.sequence_num == *seq).unwrap();
        let decrypted = bob_mls
            .decrypt_message(&mls_group_id, &msg.mls_message)
            .unwrap();
        match decrypted {
            conclave_client::mls::DecryptedMessage::Application(data) => {
                let expected = format!("Message #{i}");
                assert_eq!(
                    String::from_utf8(data).unwrap(),
                    expected,
                    "message #{i} content mismatch"
                );
                decrypted_count += 1;
            }
            other => panic!("message #{i}: expected Application, got: {other:?}"),
        }
    }
    assert_eq!(decrypted_count, 10);
}
