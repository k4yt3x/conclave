use prost::Message;
use reqwest::Client;

use crate::error::{Error, Result};

/// HTTP client wrapper for the Conclave server API.
pub struct ApiClient {
    client: Client,
    base_url: String,
    token: Option<String>,
}

impl ApiClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            token: None,
        }
    }

    pub fn set_token(&mut self, token: String) {
        self.token = Some(token);
    }

    /// Send a POST request with a protobuf body, returning raw response bytes.
    async fn post(&self, path: &str, body: &impl Message) -> Result<Vec<u8>> {
        let url = format!("{}{path}", self.base_url);
        let mut req = self
            .client
            .post(&url)
            .header("Content-Type", "application/x-protobuf");

        if let Some(token) = &self.token {
            req = req.header("Authorization", format!("Bearer {token}"));
        }

        let mut buf = Vec::new();
        body.encode(&mut buf).unwrap();

        let resp = req.body(buf).send().await?;
        Self::handle_response(resp).await
    }

    /// Send a GET request, returning raw response bytes.
    async fn get(&self, path: &str) -> Result<Vec<u8>> {
        let url = format!("{}{path}", self.base_url);
        let mut req = self.client.get(&url);

        if let Some(token) = &self.token {
            req = req.header("Authorization", format!("Bearer {token}"));
        }

        let resp = req.send().await?;
        Self::handle_response(resp).await
    }

    async fn handle_response(resp: reqwest::Response) -> Result<Vec<u8>> {
        let status = resp.status();
        let body = resp.bytes().await?;

        if !status.is_success() {
            let error_msg = if let Ok(err) = conclave_proto::ErrorResponse::decode(body.as_ref()) {
                err.message
            } else {
                String::from_utf8_lossy(&body).to_string()
            };
            return Err(Error::Server {
                status: status.as_u16(),
                message: error_msg,
            });
        }

        Ok(body.to_vec())
    }

    // ── Auth ──────────────────────────────────────────────────────

    pub async fn register(
        &self,
        username: &str,
        password: &str,
    ) -> Result<conclave_proto::RegisterResponse> {
        let req = conclave_proto::RegisterRequest {
            username: username.to_string(),
            password: password.to_string(),
        };
        let bytes = self.post("/api/v1/register", &req).await?;
        Ok(conclave_proto::RegisterResponse::decode(bytes.as_slice())?)
    }

    pub async fn login(
        &self,
        username: &str,
        password: &str,
    ) -> Result<conclave_proto::LoginResponse> {
        let req = conclave_proto::LoginRequest {
            username: username.to_string(),
            password: password.to_string(),
        };
        let bytes = self.post("/api/v1/login", &req).await?;
        Ok(conclave_proto::LoginResponse::decode(bytes.as_slice())?)
    }

    pub async fn me(&self) -> Result<conclave_proto::UserInfoResponse> {
        let bytes = self.get("/api/v1/me").await?;
        Ok(conclave_proto::UserInfoResponse::decode(bytes.as_slice())?)
    }

    // ── Key Packages ──────────────────────────────────────────────

    pub async fn upload_key_package(&self, key_package_data: Vec<u8>) -> Result<()> {
        let req = conclave_proto::UploadKeyPackageRequest { key_package_data };
        self.post("/api/v1/key-packages", &req).await?;
        Ok(())
    }

    // ── Groups ────────────────────────────────────────────────────

    pub async fn create_group(
        &self,
        name: &str,
        member_usernames: Vec<String>,
    ) -> Result<conclave_proto::CreateGroupResponse> {
        let req = conclave_proto::CreateGroupRequest {
            name: name.to_string(),
            member_usernames,
        };
        let bytes = self.post("/api/v1/groups", &req).await?;
        Ok(conclave_proto::CreateGroupResponse::decode(
            bytes.as_slice(),
        )?)
    }

    pub async fn list_groups(&self) -> Result<conclave_proto::ListGroupsResponse> {
        let bytes = self.get("/api/v1/groups").await?;
        Ok(conclave_proto::ListGroupsResponse::decode(
            bytes.as_slice(),
        )?)
    }

    pub async fn invite_to_group(
        &self,
        group_id: &str,
        usernames: Vec<String>,
    ) -> Result<conclave_proto::InviteToGroupResponse> {
        let req = conclave_proto::InviteToGroupRequest { usernames };
        let bytes = self
            .post(&format!("/api/v1/groups/{group_id}/invite"), &req)
            .await?;
        Ok(conclave_proto::InviteToGroupResponse::decode(
            bytes.as_slice(),
        )?)
    }

    pub async fn upload_commit(
        &self,
        group_id: &str,
        commit_message: Vec<u8>,
        welcome_messages: std::collections::HashMap<String, Vec<u8>>,
        group_info: Vec<u8>,
    ) -> Result<()> {
        let req = conclave_proto::UploadCommitRequest {
            commit_message,
            welcome_messages,
            group_info,
        };
        self.post(&format!("/api/v1/groups/{group_id}/commit"), &req)
            .await?;
        Ok(())
    }

    // ── Messages ──────────────────────────────────────────────────

    pub async fn send_message(
        &self,
        group_id: &str,
        mls_message: Vec<u8>,
    ) -> Result<conclave_proto::SendMessageResponse> {
        let req = conclave_proto::SendMessageRequest { mls_message };
        let bytes = self
            .post(&format!("/api/v1/groups/{group_id}/messages"), &req)
            .await?;
        Ok(conclave_proto::SendMessageResponse::decode(
            bytes.as_slice(),
        )?)
    }

    pub async fn get_messages(
        &self,
        group_id: &str,
        after: i64,
    ) -> Result<conclave_proto::GetMessagesResponse> {
        let bytes = self
            .get(&format!("/api/v1/groups/{group_id}/messages?after={after}"))
            .await?;
        Ok(conclave_proto::GetMessagesResponse::decode(
            bytes.as_slice(),
        )?)
    }

    // ── Welcomes ──────────────────────────────────────────────────

    pub async fn list_pending_welcomes(
        &self,
    ) -> Result<conclave_proto::ListPendingWelcomesResponse> {
        let bytes = self.get("/api/v1/welcomes").await?;
        Ok(conclave_proto::ListPendingWelcomesResponse::decode(
            bytes.as_slice(),
        )?)
    }
}
