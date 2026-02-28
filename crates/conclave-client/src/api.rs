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

/// Normalize a server URL by prepending `https://` if no scheme is present
/// and stripping any trailing slash.
pub fn normalize_server_url(url: &str) -> String {
    let url = if !url.is_empty() && !url.starts_with("http://") && !url.starts_with("https://") {
        format!("https://{url}")
    } else {
        url.to_string()
    };
    url.trim_end_matches('/').to_string()
}

impl ApiClient {
    pub fn new(base_url: &str, accept_invalid_certs: bool) -> Self {
        let client = Client::builder()
            .danger_accept_invalid_certs(accept_invalid_certs)
            .build()
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "HTTP client build failed, using default");
                Client::new()
            });

        Self {
            client,
            base_url: normalize_server_url(base_url),
            token: None,
        }
    }

    pub fn set_token(&mut self, token: String) {
        self.token = Some(token);
    }

    /// Send a POST request with a protobuf body, returning raw response bytes.
    async fn post(&self, path: &str, body: &impl Message) -> Result<Vec<u8>> {
        let url = format!("{}{path}", self.base_url);
        let mut request = self
            .client
            .post(&url)
            .header("Content-Type", "application/x-protobuf");

        if let Some(token) = &self.token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }

        let mut buf = Vec::new();
        body.encode(&mut buf)
            .map_err(|e| Error::Other(format!("protobuf encode failed: {e}")))?;

        let response = request.body(buf).send().await?;
        Self::handle_response(response).await
    }

    /// Send a PATCH request with a protobuf body, returning raw response bytes.
    async fn patch(&self, path: &str, body: &impl Message) -> Result<Vec<u8>> {
        let url = format!("{}{path}", self.base_url);
        let mut request = self
            .client
            .patch(&url)
            .header("Content-Type", "application/x-protobuf");

        if let Some(token) = &self.token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }

        let mut buf = Vec::new();
        body.encode(&mut buf)
            .map_err(|e| Error::Other(format!("protobuf encode failed: {e}")))?;

        let response = request.body(buf).send().await?;
        Self::handle_response(response).await
    }

    /// Send a GET request, returning raw response bytes.
    async fn get(&self, path: &str) -> Result<Vec<u8>> {
        let url = format!("{}{path}", self.base_url);
        let mut request = self.client.get(&url);

        if let Some(token) = &self.token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }

        let response = request.send().await?;
        Self::handle_response(response).await
    }

    async fn handle_response(response: reqwest::Response) -> Result<Vec<u8>> {
        let status = response.status();
        let body = response.bytes().await?;

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
        alias: Option<&str>,
        registration_token: Option<&str>,
    ) -> Result<conclave_proto::RegisterResponse> {
        let request = conclave_proto::RegisterRequest {
            username: username.to_string(),
            password: password.to_string(),
            alias: alias.unwrap_or_default().to_string(),
            registration_token: registration_token.unwrap_or_default().to_string(),
        };
        let bytes = self.post("/api/v1/register", &request).await?;
        Ok(conclave_proto::RegisterResponse::decode(bytes.as_slice())?)
    }

    pub async fn login(
        &self,
        username: &str,
        password: &str,
    ) -> Result<conclave_proto::LoginResponse> {
        let request = conclave_proto::LoginRequest {
            username: username.to_string(),
            password: password.to_string(),
        };
        let bytes = self.post("/api/v1/login", &request).await?;
        Ok(conclave_proto::LoginResponse::decode(bytes.as_slice())?)
    }

    pub async fn logout(&self) -> Result<()> {
        let url = format!("{}/api/v1/logout", self.base_url);
        let mut request = self
            .client
            .post(&url)
            .header("Content-Type", "application/x-protobuf");

        if let Some(token) = &self.token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }

        let response = request.send().await?;
        if !response.status().is_success() {
            let body = response.bytes().await?;
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

    pub async fn update_profile(&self, alias: &str) -> Result<()> {
        let request = conclave_proto::UpdateProfileRequest {
            alias: alias.to_string(),
        };
        self.patch("/api/v1/me", &request).await?;
        Ok(())
    }

    pub async fn change_password(&self, new_password: &str) -> Result<()> {
        let request = conclave_proto::ChangePasswordRequest {
            new_password: new_password.to_string(),
        };
        self.post("/api/v1/change-password", &request).await?;
        Ok(())
    }

    pub async fn update_group(&self, group_id: i64, alias: Option<&str>) -> Result<()> {
        let request = conclave_proto::UpdateGroupRequest {
            alias: alias.unwrap_or_default().to_string(),
            group_name: String::new(),
            message_expiry_seconds: 0,
            update_message_expiry: false,
        };
        self.patch(&format!("/api/v1/groups/{group_id}"), &request)
            .await?;
        Ok(())
    }

    pub async fn set_group_expiry(&self, group_id: i64, seconds: i64) -> Result<()> {
        let request = conclave_proto::UpdateGroupRequest {
            alias: String::new(),
            group_name: String::new(),
            message_expiry_seconds: seconds,
            update_message_expiry: true,
        };
        self.patch(&format!("/api/v1/groups/{group_id}"), &request)
            .await?;
        Ok(())
    }

    pub async fn get_retention_policy(
        &self,
        group_id: i64,
    ) -> Result<conclave_proto::GetRetentionPolicyResponse> {
        let bytes = self
            .get(&format!("/api/v1/groups/{group_id}/retention"))
            .await?;
        Ok(conclave_proto::GetRetentionPolicyResponse::decode(
            bytes.as_slice(),
        )?)
    }

    // ── Key Packages ──────────────────────────────────────────────

    pub async fn upload_key_package(&self, key_package_data: Vec<u8>) -> Result<()> {
        let request = conclave_proto::UploadKeyPackageRequest {
            key_package_data,
            entries: vec![],
        };
        self.post("/api/v1/key-packages", &request).await?;
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
        let request = conclave_proto::UploadKeyPackageRequest {
            key_package_data: vec![],
            entries: proto_entries,
        };
        self.post("/api/v1/key-packages", &request).await?;
        Ok(())
    }

    // ── Groups ────────────────────────────────────────────────────

    pub async fn create_group(
        &self,
        alias: Option<&str>,
        group_name: &str,
    ) -> Result<conclave_proto::CreateGroupResponse> {
        let request = conclave_proto::CreateGroupRequest {
            alias: alias.unwrap_or_default().to_string(),
            group_name: group_name.to_string(),
        };
        let bytes = self.post("/api/v1/groups", &request).await?;
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
        group_id: i64,
        user_ids: Vec<i64>,
    ) -> Result<conclave_proto::InviteToGroupResponse> {
        let request = conclave_proto::InviteToGroupRequest { user_ids };
        let bytes = self
            .post(&format!("/api/v1/groups/{group_id}/invite"), &request)
            .await?;
        Ok(conclave_proto::InviteToGroupResponse::decode(
            bytes.as_slice(),
        )?)
    }

    pub async fn upload_commit(
        &self,
        group_id: i64,
        commit_message: Vec<u8>,
        group_info: Vec<u8>,
        mls_group_id: Option<&str>,
    ) -> Result<()> {
        let request = conclave_proto::UploadCommitRequest {
            commit_message,
            group_info,
            mls_group_id: mls_group_id.unwrap_or_default().to_string(),
        };
        self.post(&format!("/api/v1/groups/{group_id}/commit"), &request)
            .await?;
        Ok(())
    }

    // ── Messages ──────────────────────────────────────────────────

    pub async fn send_message(
        &self,
        group_id: i64,
        mls_message: Vec<u8>,
    ) -> Result<conclave_proto::SendMessageResponse> {
        let request = conclave_proto::SendMessageRequest { mls_message };
        let bytes = self
            .post(&format!("/api/v1/groups/{group_id}/messages"), &request)
            .await?;
        Ok(conclave_proto::SendMessageResponse::decode(
            bytes.as_slice(),
        )?)
    }

    pub async fn get_messages(
        &self,
        group_id: i64,
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
        let mut request = self
            .client
            .post(&url)
            .header("Content-Type", "application/x-protobuf");

        if let Some(token) = &self.token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }

        let response = request.send().await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.bytes().await?;
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

    // ── Invite Escrow ──────────────────────────────────────────────

    pub async fn escrow_invite(
        &self,
        group_id: i64,
        invitee_id: i64,
        commit_message: Vec<u8>,
        welcome_message: Vec<u8>,
        group_info: Vec<u8>,
    ) -> Result<()> {
        let request = conclave_proto::EscrowInviteRequest {
            invitee_id,
            commit_message,
            welcome_message,
            group_info,
        };
        self.post(
            &format!("/api/v1/groups/{group_id}/escrow-invite"),
            &request,
        )
        .await?;
        Ok(())
    }

    pub async fn list_pending_invites(&self) -> Result<conclave_proto::ListPendingInvitesResponse> {
        let bytes = self.get("/api/v1/invites").await?;
        Ok(conclave_proto::ListPendingInvitesResponse::decode(
            bytes.as_slice(),
        )?)
    }

    pub async fn accept_pending_invite(&self, invite_id: i64) -> Result<()> {
        let empty = conclave_proto::AcceptInviteResponse {};
        self.post(&format!("/api/v1/invites/{invite_id}/accept"), &empty)
            .await?;
        Ok(())
    }

    pub async fn decline_pending_invite(&self, invite_id: i64) -> Result<()> {
        let empty = conclave_proto::DeclineInviteResponse {};
        self.post(&format!("/api/v1/invites/{invite_id}/decline"), &empty)
            .await?;
        Ok(())
    }

    pub async fn list_group_pending_invites(
        &self,
        group_id: i64,
    ) -> Result<conclave_proto::ListGroupPendingInvitesResponse> {
        let bytes = self
            .get(&format!("/api/v1/groups/{group_id}/invites"))
            .await?;
        Ok(conclave_proto::ListGroupPendingInvitesResponse::decode(
            bytes.as_slice(),
        )?)
    }

    pub async fn cancel_invite(&self, group_id: i64, invitee_id: i64) -> Result<()> {
        let request = conclave_proto::CancelInviteRequest { invitee_id };
        self.post(
            &format!("/api/v1/groups/{group_id}/cancel-invite"),
            &request,
        )
        .await?;
        Ok(())
    }

    pub async fn get_user_by_username(
        &self,
        username: &str,
    ) -> Result<conclave_proto::UserInfoResponse> {
        let bytes = self.get(&format!("/api/v1/users/{username}")).await?;
        Ok(conclave_proto::UserInfoResponse::decode(bytes.as_slice())?)
    }

    pub async fn get_user_by_id(&self, user_id: i64) -> Result<conclave_proto::UserInfoResponse> {
        let bytes = self.get(&format!("/api/v1/users/by-id/{user_id}")).await?;
        Ok(conclave_proto::UserInfoResponse::decode(bytes.as_slice())?)
    }

    // ── Admin Management ──────────────────────────────────────────

    pub async fn promote_member(&self, group_id: i64, user_id: i64) -> Result<()> {
        let request = conclave_proto::PromoteMemberRequest { user_id };
        self.post(&format!("/api/v1/groups/{group_id}/promote"), &request)
            .await?;
        Ok(())
    }

    pub async fn demote_member(&self, group_id: i64, user_id: i64) -> Result<()> {
        let request = conclave_proto::DemoteMemberRequest { user_id };
        self.post(&format!("/api/v1/groups/{group_id}/demote"), &request)
            .await?;
        Ok(())
    }

    pub async fn list_admins(&self, group_id: i64) -> Result<conclave_proto::ListAdminsResponse> {
        let bytes = self
            .get(&format!("/api/v1/groups/{group_id}/admins"))
            .await?;
        Ok(conclave_proto::ListAdminsResponse::decode(
            bytes.as_slice(),
        )?)
    }

    // ── Member Management ──────────────────────────────────────────

    pub async fn remove_member(
        &self,
        group_id: i64,
        user_id: i64,
        commit_message: Vec<u8>,
        group_info: Vec<u8>,
    ) -> Result<()> {
        let request = conclave_proto::RemoveMemberRequest {
            user_id,
            commit_message,
            group_info,
        };
        self.post(&format!("/api/v1/groups/{group_id}/remove"), &request)
            .await?;
        Ok(())
    }

    pub async fn leave_group(
        &self,
        group_id: i64,
        commit_message: Vec<u8>,
        group_info: Vec<u8>,
    ) -> Result<()> {
        let request = conclave_proto::LeaveGroupRequest {
            commit_message,
            group_info,
        };
        self.post(&format!("/api/v1/groups/{group_id}/leave"), &request)
            .await?;
        Ok(())
    }

    pub async fn get_group_info(
        &self,
        group_id: i64,
    ) -> Result<conclave_proto::GetGroupInfoResponse> {
        let bytes = self
            .get(&format!("/api/v1/groups/{group_id}/group-info"))
            .await?;
        Ok(conclave_proto::GetGroupInfoResponse::decode(
            bytes.as_slice(),
        )?)
    }

    pub async fn external_join(
        &self,
        group_id: i64,
        commit_message: Vec<u8>,
        mls_group_id: &str,
    ) -> Result<()> {
        let request = conclave_proto::ExternalJoinRequest {
            commit_message,
            mls_group_id: mls_group_id.to_string(),
        };
        self.post(
            &format!("/api/v1/groups/{group_id}/external-join"),
            &request,
        )
        .await?;
        Ok(())
    }

    pub async fn reset_account(&self) -> Result<()> {
        let url = format!("{}/api/v1/reset-account", self.base_url);
        let mut request = self
            .client
            .post(&url)
            .header("Content-Type", "application/x-protobuf");

        if let Some(token) = &self.token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }

        let response = request.send().await?;
        if !response.status().is_success() {
            let body = response.bytes().await?;
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
        EventSource::new(builder).map_err(|e| Error::Other(format!("SSE connection failed: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_no_scheme() {
        assert_eq!(normalize_server_url("example.com"), "https://example.com");
    }

    #[test]
    fn test_normalize_with_port() {
        assert_eq!(
            normalize_server_url("example.com:8443"),
            "https://example.com:8443"
        );
    }

    #[test]
    fn test_normalize_https_preserved() {
        assert_eq!(
            normalize_server_url("https://example.com"),
            "https://example.com"
        );
    }

    #[test]
    fn test_normalize_http_preserved() {
        assert_eq!(
            normalize_server_url("http://example.com"),
            "http://example.com"
        );
    }

    #[test]
    fn test_normalize_trailing_slash_stripped() {
        assert_eq!(
            normalize_server_url("https://example.com/"),
            "https://example.com"
        );
    }

    #[test]
    fn test_normalize_multiple_trailing_slashes() {
        assert_eq!(
            normalize_server_url("https://example.com///"),
            "https://example.com"
        );
    }

    #[test]
    fn test_normalize_empty_string() {
        assert_eq!(normalize_server_url(""), "");
    }

    #[test]
    fn test_normalize_with_path() {
        assert_eq!(
            normalize_server_url("example.com/api"),
            "https://example.com/api"
        );
    }
}
