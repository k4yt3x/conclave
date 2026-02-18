use prost::Message;
use reqwest::Client;
use reqwest_eventsource::EventSource;

use crate::error::{Error, Result};

/// HTTP client wrapper for the Conclave server API.
pub struct ApiClient {
    client: Client,
    base_url: String,
    token: Option<String>,
}

impl ApiClient {
    pub fn new(base_url: &str, accept_invalid_certs: bool) -> Self {
        let client = Client::builder()
            .danger_accept_invalid_certs(accept_invalid_certs)
            .build()
            .expect("failed to build HTTP client");

        // Auto-prepend https:// if no scheme is present
        let base_url = if !base_url.is_empty()
            && !base_url.starts_with("http://")
            && !base_url.starts_with("https://")
        {
            format!("https://{base_url}")
        } else {
            base_url.to_string()
        };

        Self {
            client,
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

    pub async fn logout(&self) -> Result<()> {
        let url = format!("{}/api/v1/logout", self.base_url);
        let mut req = self
            .client
            .post(&url)
            .header("Content-Type", "application/x-protobuf");

        if let Some(token) = &self.token {
            req = req.header("Authorization", format!("Bearer {token}"));
        }

        let resp = req.send().await?;
        if !resp.status().is_success() {
            let body = resp.bytes().await?;
            let error_msg = if let Ok(err) = conclave_proto::ErrorResponse::decode(body.as_ref()) {
                err.message
            } else {
                String::from_utf8_lossy(&body).to_string()
            };
            return Err(Error::Server {
                status: 500,
                message: error_msg,
            });
        }
        Ok(())
    }

    pub async fn me(&self) -> Result<conclave_proto::UserInfoResponse> {
        let bytes = self.get("/api/v1/me").await?;
        Ok(conclave_proto::UserInfoResponse::decode(bytes.as_slice())?)
    }

    // ── Key Packages ──────────────────────────────────────────────

    pub async fn upload_key_package(&self, key_package_data: Vec<u8>) -> Result<()> {
        let req = conclave_proto::UploadKeyPackageRequest {
            key_package_data,
            entries: vec![],
        };
        self.post("/api/v1/key-packages", &req).await?;
        Ok(())
    }

    /// Upload multiple key packages in a single request, with last-resort flag support.
    pub async fn upload_key_packages(&self, entries: Vec<(Vec<u8>, bool)>) -> Result<()> {
        let proto_entries = entries
            .into_iter()
            .map(|(data, is_last_resort)| conclave_proto::KeyPackageEntry {
                data,
                is_last_resort,
            })
            .collect();
        let req = conclave_proto::UploadKeyPackageRequest {
            key_package_data: vec![],
            entries: proto_entries,
        };
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

    /// Accept (delete) a pending welcome by its server-assigned ID.
    pub async fn accept_welcome(&self, welcome_id: i64) -> Result<()> {
        let url = format!("{}/api/v1/welcomes/{welcome_id}/accept", self.base_url);
        let mut req = self
            .client
            .post(&url)
            .header("Content-Type", "application/x-protobuf");

        if let Some(token) = &self.token {
            req = req.header("Authorization", format!("Bearer {token}"));
        }

        let resp = req.send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.bytes().await?;
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
        Ok(())
    }

    // ── Member Management ──────────────────────────────────────────

    pub async fn remove_member(
        &self,
        group_id: &str,
        username: &str,
        commit_message: Vec<u8>,
        group_info: Vec<u8>,
    ) -> Result<()> {
        let req = conclave_proto::RemoveMemberRequest {
            username: username.to_string(),
            commit_message,
            group_info,
        };
        self.post(&format!("/api/v1/groups/{group_id}/remove"), &req)
            .await?;
        Ok(())
    }

    pub async fn leave_group(&self, group_id: &str) -> Result<()> {
        let req = conclave_proto::LeaveGroupRequest {};
        self.post(&format!("/api/v1/groups/{group_id}/leave"), &req)
            .await?;
        Ok(())
    }

    pub async fn get_group_info(
        &self,
        group_id: &str,
    ) -> Result<conclave_proto::GetGroupInfoResponse> {
        let bytes = self
            .get(&format!("/api/v1/groups/{group_id}/group-info"))
            .await?;
        Ok(conclave_proto::GetGroupInfoResponse::decode(
            bytes.as_slice(),
        )?)
    }

    pub async fn external_join(&self, group_id: &str, commit_message: Vec<u8>) -> Result<()> {
        let req = conclave_proto::ExternalJoinRequest { commit_message };
        self.post(&format!("/api/v1/groups/{group_id}/external-join"), &req)
            .await?;
        Ok(())
    }

    pub async fn reset_account(&self) -> Result<()> {
        let url = format!("{}/api/v1/reset-account", self.base_url);
        let mut req = self
            .client
            .post(&url)
            .header("Content-Type", "application/x-protobuf");

        if let Some(token) = &self.token {
            req = req.header("Authorization", format!("Bearer {token}"));
        }

        let resp = req.send().await?;
        if !resp.status().is_success() {
            let body = resp.bytes().await?;
            let error_msg = if let Ok(err) = conclave_proto::ErrorResponse::decode(body.as_ref()) {
                err.message
            } else {
                String::from_utf8_lossy(&body).to_string()
            };
            return Err(Error::Server {
                status: 500,
                message: error_msg,
            });
        }
        Ok(())
    }

    // ── SSE ───────────────────────────────────────────────────────

    /// Create an SSE EventSource connected to the server's event stream.
    pub fn connect_sse(&self) -> Result<EventSource> {
        let token = self
            .token
            .as_ref()
            .ok_or_else(|| Error::Other("not logged in".into()))?;
        let url = format!("{}/api/v1/events", self.base_url);
        let builder = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {token}"));
        Ok(EventSource::new(builder)
            .map_err(|e| Error::Other(format!("SSE connection failed: {e}")))?)
    }
}
