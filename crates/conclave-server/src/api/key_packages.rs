use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;

use crate::auth::AuthUser;
use crate::error::{Error, Result};
use crate::state::AppState;

use super::{decode_proto, proto_response, validate_key_package_wire_format};

pub async fn upload_key_package(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let request = decode_proto::<conclave_proto::UploadKeyPackageRequest>(&body)?;

    if !request.entries.is_empty() {
        // Batch upload path.
        for entry in &request.entries {
            if entry.data.is_empty() {
                return Err(Error::BadRequest("key package data is required".into()));
            }
            if entry.data.len() > 16 * 1024 {
                return Err(Error::BadRequest(
                    "key_package_data must be 16 KiB or smaller".into(),
                ));
            }
            validate_key_package_wire_format(&entry.data)?;
            state
                .db
                .store_key_package(auth.user_id, &entry.data, entry.is_last_resort)?;
        }
    } else if !request.key_package_data.is_empty() {
        // Legacy single-upload path (regular key package).
        if request.key_package_data.len() > 16 * 1024 {
            return Err(Error::BadRequest(
                "key_package_data must be 16 KiB or smaller".into(),
            ));
        }
        validate_key_package_wire_format(&request.key_package_data)?;
        state
            .db
            .store_key_package(auth.user_id, &request.key_package_data, false)?;
    } else {
        return Err(Error::BadRequest("key_package_data is required".into()));
    }

    if !request.signing_key_fingerprint.is_empty() {
        state
            .db
            .update_signing_key_fingerprint(auth.user_id, &request.signing_key_fingerprint)?;
    }

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::UploadKeyPackageResponse {},
    ))
}

pub async fn get_key_package(
    State(state): State<Arc<AppState>>,
    _auth: AuthUser,
    Path(user_id): Path<i64>,
) -> Result<impl IntoResponse> {
    let data = state
        .db
        .consume_key_package(user_id)?
        .ok_or_else(|| Error::NotFound("no key package available for this user".into()))?;

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::GetKeyPackageResponse {
            key_package_data: data,
        },
    ))
}
