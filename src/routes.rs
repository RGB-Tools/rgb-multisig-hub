use std::{collections::HashSet, sync::Arc};

use amplify::s;
use axum::{
    Json,
    body::Body,
    extract::{Multipart, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use axum_extra::extract::WithRejection;
use sea_orm::{ActiveValue, DeriveActiveEnum, EnumIter};
use serde::{Deserialize, Serialize};
use tokio::{fs::File, io::AsyncWriteExt};
use tokio_util::io::ReaderStream;

use crate::{
    auth::{AuthenticatedCosigner, AuthenticatedUser},
    database::entities::{cosigner_op_status, next_address_index, op_file, operation},
    error::APIError,
    startup::{AppState, MAX_RGB_LIB_VERSION, MIN_RGB_LIB_VERSION},
    utils::{compute_file_id, get_threshold_for_operation, no_cancel, now, persist_temp_file},
};

pub(crate) const AUTO_APPROVED_OPS: [OperationType; 3] = [
    OperationType::Issuance,
    OperationType::BlindReceive,
    OperationType::WitnessReceive,
];

impl AppState {
    pub(crate) async fn get_operation_by_idx_with_files(
        &self,
        operation_idx: i32,
        cosigner_idx: Option<i32>,
    ) -> Result<Option<OperationResponse>, APIError> {
        // get operation from DB
        let Some(op) = self.database.get_operation_by_idx(operation_idx).await? else {
            return Ok(None);
        };

        // get initiator cosigner from DB
        let initiator = self
            .database
            .get_cosigner_by_idx(op.initiator_idx)
            .await?
            .expect("initiator should be set");

        // get cosigner op status entries with cosigners from DB
        let status_entries_with_cosigner = self
            .database
            .get_cosigner_op_status_with_cosigners_by_operation_idx(op.idx)
            .await?;

        // extract my response and processed_at if cosigner_idx is provided
        let mut my_response = None;
        let mut processed_at = None;
        if let Some(cosigner_idx) = cosigner_idx {
            for (status, _) in &status_entries_with_cosigner {
                if status.cosigner_idx == cosigner_idx {
                    my_response = status.ack;
                    processed_at = status.processed_at;
                    break;
                }
            }
        }

        // get operation files from DB and read their metadata from filesystem
        let op_files = self.database.get_op_files_by_operation_idx(op.idx).await?;
        let mut files = Vec::new();
        for file in op_files {
            let file_path = self.files_dir.join(&file.file_id);
            let metadata = tokio::fs::metadata(&file_path).await?;
            files.push(FileMetadata {
                file_id: file.file_id,
                r#type: file.r#type,
                posted_by_xpub: initiator.xpub.clone(),
                size_bytes: metadata.len(),
            });
        }

        // get other cosigner's PSBT files from DB and read their metadata from filesystem
        for (status, cosigner) in &status_entries_with_cosigner {
            if cosigner.xpub == initiator.xpub {
                continue;
            }
            if let Some(psbt_op_file_idx) = &status.psbt_op_file_idx {
                let psbt_file = self
                    .database
                    .get_op_file_by_idx(*psbt_op_file_idx)
                    .await?
                    .expect("PSBT op file should exist");
                let file_path = self.files_dir.join(&psbt_file.file_id);
                let metadata = tokio::fs::metadata(&file_path).await?;
                files.push(FileMetadata {
                    file_id: psbt_file.file_id,
                    r#type: FileType::ResponsePsbt,
                    posted_by_xpub: cosigner.xpub.clone(),
                    size_bytes: metadata.len(),
                });
            }
        }

        // calculate acked_by and nacked_by sets
        let mut acked_by = HashSet::new();
        let mut nacked_by = HashSet::new();
        for (status_entry, cosigner) in &status_entries_with_cosigner {
            match status_entry.ack {
                Some(true) => acked_by.insert(cosigner.xpub.clone()),
                Some(false) => nacked_by.insert(cosigner.xpub.clone()),
                None => continue,
            };
        }

        // get threshold for operation type
        let threshold =
            get_threshold_for_operation(&op.r#type, self.threshold_vanilla, self.threshold_colored);

        Ok(Some(OperationResponse {
            operation_idx: op.idx,
            initiator_xpub: initiator.xpub,
            created_at: op.created_at,
            operation_type: op.r#type,
            status: op.status,
            acked_by,
            nacked_by,
            threshold,
            my_response,
            processed_at,
            files,
        }))
    }
}

#[derive(Deserialize, Serialize)]
pub(crate) struct BumpAddressIndicesRequest {
    pub(crate) count: u8,
    pub(crate) internal: bool,
}

#[derive(Deserialize, Serialize)]
pub(crate) struct BumpAddressIndicesResponse {
    pub(crate) first: u32,
}

#[derive(Deserialize, Serialize)]
pub(crate) struct EmptyResponse {}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct FileMetadata {
    pub(crate) file_id: String,
    pub(crate) r#type: FileType,
    pub(crate) posted_by_xpub: String,
    pub(crate) size_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Deserialize, Serialize)]
#[sea_orm(rs_type = "u8", db_type = "TinyUnsigned")]
pub(crate) enum FileType {
    #[sea_orm(num_value = 1)]
    Consignment = 1,
    #[sea_orm(num_value = 2)]
    Media = 2,
    #[sea_orm(num_value = 3)]
    OperationData = 3,
    #[sea_orm(num_value = 4)]
    OperationPsbt = 4,
    #[sea_orm(num_value = 5)]
    ResponsePsbt = 5,
    #[sea_orm(num_value = 6)]
    Fascia = 6,
}

#[derive(Deserialize, Serialize)]
pub(crate) struct GetCurrentAddressIndicesResponse {
    pub(crate) internal: Option<u32>,
    pub(crate) external: Option<u32>,
}

#[derive(Deserialize, Serialize)]
pub(crate) struct GetFileRequest {
    pub(crate) file_id: String,
}

#[derive(Deserialize, Serialize)]
pub(crate) struct GetLastProcessedOpIdxResponse {
    pub(crate) operation_idx: i32,
}

#[derive(Deserialize, Serialize)]
pub(crate) struct GetOperationByIdxRequest {
    pub(crate) operation_idx: i32,
}

#[derive(Debug, PartialEq, Eq, Deserialize, Serialize)]
pub(crate) enum UserRole {
    Cosigner(String),
    WatchOnly,
}

#[derive(Deserialize, Serialize)]
pub(crate) struct InfoResponse {
    pub(crate) min_rgb_lib_version: String,
    pub(crate) max_rgb_lib_version: String,
    pub(crate) rgb_lib_version: String,
    pub(crate) last_operation_idx: Option<i32>,
    pub(crate) user_role: UserRole,
}

#[derive(Deserialize, Serialize)]
pub(crate) struct MarkOperationProcessedRequest {
    pub(crate) operation_idx: i32,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct OperationResponse {
    pub(crate) operation_idx: i32,
    pub(crate) initiator_xpub: String,
    pub(crate) created_at: i64,
    pub(crate) operation_type: OperationType,
    pub(crate) status: OperationStatus,
    pub(crate) acked_by: HashSet<String>,
    pub(crate) nacked_by: HashSet<String>,
    pub(crate) threshold: Option<u8>,
    pub(crate) my_response: Option<bool>,
    pub(crate) processed_at: Option<i64>,
    pub(crate) files: Vec<FileMetadata>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Deserialize, Serialize)]
#[sea_orm(rs_type = "u8", db_type = "TinyUnsigned")]
pub(crate) enum OperationStatus {
    #[sea_orm(num_value = 1)]
    Pending = 1,
    #[sea_orm(num_value = 2)]
    Approved = 2,
    #[sea_orm(num_value = 3)]
    Discarded = 3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Deserialize, Serialize)]
#[sea_orm(rs_type = "u8", db_type = "TinyUnsigned")]
pub(crate) enum OperationType {
    #[sea_orm(num_value = 1)]
    CreateUtxos = 1,
    #[sea_orm(num_value = 2)]
    Issuance = 2,
    #[sea_orm(num_value = 3)]
    SendRgb = 3,
    #[sea_orm(num_value = 4)]
    SendBtc = 4,
    #[sea_orm(num_value = 5)]
    Inflation = 5,
    #[sea_orm(num_value = 6)]
    BlindReceive = 6,
    #[sea_orm(num_value = 7)]
    WitnessReceive = 7,
    #[sea_orm(num_value = 8)]
    Burn = 8,
}

impl TryFrom<u8> for OperationType {
    type Error = APIError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(OperationType::CreateUtxos),
            2 => Ok(OperationType::Issuance),
            3 => Ok(OperationType::SendRgb),
            4 => Ok(OperationType::SendBtc),
            5 => Ok(OperationType::Inflation),
            6 => Ok(OperationType::BlindReceive),
            7 => Ok(OperationType::WitnessReceive),
            8 => Ok(OperationType::Burn),
            _ => Err(APIError::InvalidOperationType(value)),
        }
    }
}

#[derive(Deserialize, Serialize)]
pub(crate) struct PostOperationResponse {
    pub(crate) operation_idx: i32,
}

#[derive(Deserialize, Serialize)]
pub(crate) struct RespondToOperationRequest {
    pub(crate) operation_idx: i32,
    pub(crate) ack: bool,
}

pub(crate) async fn bump_address_indices(
    State(state): State<Arc<AppState>>,
    WithRejection(Json(req), _): WithRejection<Json<BumpAddressIndicesRequest>, APIError>,
) -> Result<Json<BumpAddressIndicesResponse>, APIError> {
    // acquire write lock to prevent concurrent write operations
    let _lock = state.write_lock.lock().await;

    // check if request is valid
    if req.count == 0 {
        return Err(APIError::InvalidCount);
    }

    // increment address indices and return the first of the new range
    let txn = state.database.begin_transaction().await?;
    let index = state.database.get_next_address_index(Some(&txn)).await?;
    let first = if req.internal {
        index.internal
    } else {
        index.external
    };
    let new_next_index = first
        .checked_add(req.count.into())
        .ok_or(APIError::Unexpected(s!("address index overflow")))?;
    let mut index: next_address_index::ActiveModel = index.into();
    if req.internal {
        index.internal = ActiveValue::Set(new_next_index);
    } else {
        index.external = ActiveValue::Set(new_next_index);
    }
    state
        .database
        .update_next_address_index(index, &txn)
        .await?;
    txn.commit().await?;

    Ok(Json(BumpAddressIndicesResponse { first }))
}

pub(crate) async fn get_current_address_indices(
    State(state): State<Arc<AppState>>,
) -> Result<Json<GetCurrentAddressIndicesResponse>, APIError> {
    // get next address indices
    let index = state.database.get_next_address_index(None).await?;

    // calculate current address indices
    let internal = if index.internal == 0 {
        None
    } else {
        Some(index.internal - 1)
    };
    let external = if index.external == 0 {
        None
    } else {
        Some(index.external - 1)
    };

    Ok(Json(GetCurrentAddressIndicesResponse {
        internal,
        external,
    }))
}

pub(crate) async fn get_file(
    State(state): State<Arc<AppState>>,
    WithRejection(Json(req), _): WithRejection<Json<GetFileRequest>, APIError>,
) -> Result<Response, APIError> {
    // check if request is valid
    let file_path = state.files_dir.join(&req.file_id);
    if !file_path.exists() {
        return Err(APIError::FileNotFound);
    }

    // read file metadata and stream file
    let file = File::open(&file_path).await?;
    let metadata = file.metadata().await?;
    let len = metadata.len();
    let stream = ReaderStream::new(file);
    let body = Body::from_stream(stream);
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&len.to_string()).expect("cannot be invalid"),
    );
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );

    Ok((StatusCode::OK, headers, body).into_response())
}

pub(crate) async fn get_last_processed_op_idx(
    State(state): State<Arc<AppState>>,
    AuthenticatedCosigner {
        idx: cosigner_idx, ..
    }: AuthenticatedCosigner,
) -> Result<Json<GetLastProcessedOpIdxResponse>, APIError> {
    // get last processed operation index for the requesting cosigner
    let operation_idx = state
        .database
        .get_last_cosigner_processed_op_idx(cosigner_idx)
        .await?;

    Ok(Json(GetLastProcessedOpIdxResponse { operation_idx }))
}

pub(crate) async fn get_operation_by_idx(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    WithRejection(Json(req), _): WithRejection<Json<GetOperationByIdxRequest>, APIError>,
) -> Result<Json<Option<OperationResponse>>, APIError> {
    // get cosigner index, if any
    let cosigner_idx = match user {
        AuthenticatedUser::Cosigner(AuthenticatedCosigner { idx, .. }) => Some(idx),
        AuthenticatedUser::WatchOnly => None,
    };

    // get operation response
    let operation_response = state
        .get_operation_by_idx_with_files(req.operation_idx, cosigner_idx)
        .await?;

    Ok(Json(operation_response))
}

pub(crate) async fn info(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
) -> Result<Json<InfoResponse>, APIError> {
    // get last operation index
    let last_operation_idx = state.database.get_last_operation_idx().await?;

    // get user role
    let user_role = match user {
        AuthenticatedUser::Cosigner(c) => UserRole::Cosigner(c.xpub),
        AuthenticatedUser::WatchOnly => UserRole::WatchOnly,
    };

    Ok(Json(InfoResponse {
        min_rgb_lib_version: MIN_RGB_LIB_VERSION.to_string(),
        max_rgb_lib_version: MAX_RGB_LIB_VERSION.to_string(),
        rgb_lib_version: state.rgb_lib_version.clone(),
        last_operation_idx,
        user_role,
    }))
}

pub(crate) async fn mark_operation_processed(
    State(state): State<Arc<AppState>>,
    AuthenticatedCosigner {
        idx: cosigner_idx, ..
    }: AuthenticatedCosigner,
    WithRejection(Json(req), _): WithRejection<Json<MarkOperationProcessedRequest>, APIError>,
) -> Result<Json<EmptyResponse>, APIError> {
    no_cancel(async move {
        // acquire write lock to prevent concurrent write operations
        let _lock = state.write_lock.lock().await;

        // check if request is allowed
        let op = state
            .database
            .get_operation_by_idx(req.operation_idx)
            .await?
            .ok_or(APIError::OperationNotFound)?;
        if op.status == OperationStatus::Pending {
            return Err(APIError::CannotMarkOperationProcessed(s!(
                "a pending operation cannot be marked as processed"
            )));
        }
        let status = state
            .database
            .get_cosigner_op_status_entry(cosigner_idx, req.operation_idx)
            .await?
            .ok_or(APIError::OperationNotFound)?;
        if status.processed_at.is_some() {
            return Err(APIError::CannotMarkOperationProcessed(s!(
                "already marked this operation as processed"
            )));
        }

        // set processed_at for cosigner op status entry
        let mut status: cosigner_op_status::ActiveModel = status.into();
        status.processed_at = ActiveValue::Set(Some(now().unix_timestamp()));
        state
            .database
            .update_cosigner_op_status(status, None)
            .await?;

        Ok(Json(EmptyResponse {}))
    })
    .await
}

pub(crate) async fn post_operation(
    State(state): State<Arc<AppState>>,
    AuthenticatedCosigner {
        idx: cosigner_idx, ..
    }: AuthenticatedCosigner,
    WithRejection(mut multipart, _): WithRejection<Multipart, APIError>,
) -> Result<Json<PostOperationResponse>, APIError> {
    no_cancel(async move {
        // acquire write lock to prevent concurrent write operations
        let _lock = state.write_lock.lock().await;

        // check if request is allowed
        if state.database.has_pending_operation().await? {
            return Err(APIError::CannotPostNewOperation(s!(
                "another operation is still pending"
            )));
        }
        let has_unprocessed = state
            .database
            .has_unprocessed_operation(cosigner_idx)
            .await?;
        if has_unprocessed {
            return Err(APIError::CannotPostNewOperation(s!(
                "initiator has unprocessed operations"
            )));
        }

        // parse multipart form
        let mut operation_type = None;
        let mut files = Vec::new();
        let mut psbt_file = None;
        while let Some(mut field) = multipart
            .next_field()
            .await
            .map_err(|_| APIError::InvalidRequest(s!("failed to parse multipart")))?
        {
            let name = field.name().unwrap_or("").to_string();
            match name.as_str() {
                "operation_type" => {
                    let op_type = field.bytes().await.map_err(|e| {
                        APIError::InvalidRequest(format!("failed to read field: {e}"))
                    })?;
                    let op_type = OperationType::try_from(op_type[0])?;
                    operation_type = Some(op_type);
                }
                field_name if field_name.starts_with("file_") => {
                    let file_type = match &field_name[5..] {
                        "psbt" => FileType::OperationPsbt,
                        "operation_data" => FileType::OperationData,
                        "fascia" => FileType::Fascia,
                        "media" => FileType::Media,
                        "consignment" => FileType::Consignment,
                        _ => {
                            return Err(APIError::InvalidRequest(format!(
                                "invalid file type '{field_name}'"
                            )));
                        }
                    };
                    let temp_file = tempfile::Builder::new()
                        .prefix("tmp_")
                        .tempfile_in(&state.files_dir)?;
                    let mut async_file = File::from_std(temp_file.reopen()?);
                    let mut file_size = 0;
                    while let Some(chunk) = field.chunk().await.map_err(|e| {
                        APIError::InvalidRequest(format!("failed to read chunk: {e}"))
                    })? {
                        file_size += chunk.len();
                        async_file.write_all(&chunk).await?;
                    }
                    async_file.flush().await?;
                    if file_size == 0 {
                        return Err(APIError::InvalidRequest(format!(
                            "empty file {}",
                            field_name
                        )));
                    }
                    if file_type == FileType::OperationPsbt {
                        if psbt_file.is_some() {
                            return Err(APIError::InvalidRequest(s!(
                                "more than one PSBT provided"
                            )));
                        }
                        psbt_file = Some(temp_file);
                    } else {
                        files.push((file_type, temp_file));
                    }
                }
                _ => {
                    return Err(APIError::InvalidRequest(format!(
                        "unexpected field '{name}'"
                    )));
                }
            }
        }

        // check if request is valid
        if files.is_empty() && psbt_file.is_none() {
            return Err(APIError::InvalidRequest(s!("no files nor PSBT provided")));
        }
        let operation_type =
            operation_type.ok_or(APIError::InvalidRequest(s!("operation type not provided")))?;

        // get current timestamp
        let now = now().unix_timestamp();

        // request is valid and allowed, start transaction
        let txn = state.database.begin_transaction().await?;

        // save operation
        let initial_status = if AUTO_APPROVED_OPS.contains(&operation_type) {
            OperationStatus::Approved
        } else {
            OperationStatus::Pending
        };
        let db_operation = operation::ActiveModel {
            r#type: ActiveValue::Set(operation_type),
            status: ActiveValue::Set(initial_status),
            initiator_idx: ActiveValue::Set(cosigner_idx),
            created_at: ActiveValue::Set(now),
            ..Default::default()
        };
        let operation_idx = state.database.set_operation(db_operation, &txn).await?;

        // save operation files
        for (file_type, temp_file) in files.into_iter() {
            let file_id = compute_file_id(temp_file.path()).await?;
            let file_path = state.files_dir.join(&file_id);
            if !file_path.exists() {
                persist_temp_file(temp_file, &file_path).await?;
            }
            let db_file = op_file::ActiveModel {
                file_id: ActiveValue::Set(file_id),
                r#type: ActiveValue::Set(file_type),
                operation_idx: ActiveValue::Set(operation_idx),
                ..Default::default()
            };
            state.database.set_op_file(db_file, &txn).await?;
        }

        // save PSBT file if provided
        if let Some(psbt_temp) = psbt_file {
            let file_id = compute_file_id(psbt_temp.path()).await?;
            let file_path = state.files_dir.join(&file_id);
            if !file_path.exists() {
                persist_temp_file(psbt_temp, &file_path).await?;
            }
            let db_file = op_file::ActiveModel {
                file_id: ActiveValue::Set(file_id),
                r#type: ActiveValue::Set(FileType::OperationPsbt),
                operation_idx: ActiveValue::Set(operation_idx),
                ..Default::default()
            };
            state.database.set_op_file(db_file, &txn).await?;
        }

        // create cosigner op status for all cosigners
        for idx in state.cosigners_by_idx.keys() {
            let cosigner_op_status = cosigner_op_status::ActiveModel {
                operation_idx: ActiveValue::Set(operation_idx),
                cosigner_idx: ActiveValue::Set(*idx),
                ..Default::default()
            };
            state
                .database
                .set_cosigner_op_status(&cosigner_op_status, &txn)
                .await?;
        }

        // commit transaction
        txn.commit().await?;

        Ok(Json(PostOperationResponse { operation_idx }))
    })
    .await
}

pub(crate) async fn respond_to_operation(
    State(state): State<Arc<AppState>>,
    AuthenticatedCosigner {
        idx: cosigner_idx, ..
    }: AuthenticatedCosigner,
    WithRejection(mut multipart, _): WithRejection<Multipart, APIError>,
) -> Result<Json<OperationResponse>, APIError> {
    no_cancel(async move {
        // acquire write lock to prevent concurrent write operations
        let _lock = state.write_lock.lock().await;

        // parse multipart form
        let mut req = None;
        let mut psbt_file = None;
        while let Some(mut field) = multipart
            .next_field()
            .await
            .map_err(|_| APIError::InvalidRequest(s!("failed to parse multipart")))?
        {
            let name = field.name().unwrap_or("").to_string();
            match name.as_str() {
                "request" => {
                    let json_str = field.text().await.map_err(|e| {
                        APIError::InvalidRequest(format!("failed to read field: {e}"))
                    })?;
                    let payload: RespondToOperationRequest = serde_json::from_str(&json_str)
                        .map_err(|e| {
                            APIError::InvalidRequest(format!("failed to parse JSON: {e}"))
                        })?;
                    req = Some(payload);
                }
                "file_psbt" => {
                    if psbt_file.is_some() {
                        return Err(APIError::InvalidRequest(s!("more than one PSBT provided")));
                    }
                    let temp_file = tempfile::Builder::new()
                        .prefix("tmp_")
                        .tempfile_in(&state.files_dir)?;
                    let mut async_file = File::from_std(temp_file.reopen()?);
                    let mut file_size = 0;
                    while let Some(chunk) = field.chunk().await.map_err(|e| {
                        APIError::InvalidRequest(format!("failed to read chunk: {e}"))
                    })? {
                        file_size += chunk.len();
                        async_file.write_all(&chunk).await?;
                    }
                    async_file.flush().await?;
                    if file_size == 0 {
                        return Err(APIError::InvalidRequest(s!("empty file")));
                    }
                    psbt_file = Some(temp_file);
                }
                _ => {
                    return Err(APIError::InvalidRequest(format!(
                        "unexpected field '{name}'"
                    )));
                }
            }
        }

        // check if request is valid and allowed
        let req = req.ok_or(APIError::InvalidRequest(s!("missing request body")))?;
        if req.ack && psbt_file.is_none() {
            return Err(APIError::InvalidRequest(s!("ACK requires PSBT file")));
        }
        let op = state
            .database
            .get_operation_by_idx(req.operation_idx)
            .await?
            .ok_or(APIError::OperationNotFound)?;
        if op.status != OperationStatus::Pending {
            return Err(APIError::CannotRespondToOperation(s!(
                "operation is not pending"
            )));
        }
        let status_entry = state
            .database
            .get_cosigner_op_status_entry(cosigner_idx, req.operation_idx)
            .await?
            .expect("CosignerOpStatus entry should exist");
        if status_entry.ack.is_some() {
            return Err(APIError::CannotRespondToOperation(s!(
                "already responded to this operation"
            )));
        }
        let last_processed_op_idx = state
            .database
            .get_last_cosigner_processed_op_idx(cosigner_idx)
            .await?;
        if req.operation_idx != last_processed_op_idx + 1 {
            return Err(APIError::CannotRespondToOperation(s!(
                "operation is not the next one to be processed"
            )));
        }

        // request is valid and allowed, start transaction
        let txn = state.database.begin_transaction().await?;

        // save PSBT file if provided
        let psbt_op_file_idx = if let Some(psbt_temp) = psbt_file {
            let file_id = compute_file_id(psbt_temp.path()).await?;
            let file_path = state.files_dir.join(&file_id);
            if !file_path.exists() {
                persist_temp_file(psbt_temp, &file_path).await?;
            }
            let db_file = op_file::ActiveModel {
                file_id: ActiveValue::Set(file_id),
                r#type: ActiveValue::Set(FileType::ResponsePsbt),
                operation_idx: ActiveValue::Set(op.idx),
                ..Default::default()
            };
            Some(state.database.set_op_file(db_file, &txn).await?)
        } else {
            None
        };

        // update cosigner op status
        let mut status: cosigner_op_status::ActiveModel = status_entry.into();
        status.ack = ActiveValue::Set(Some(req.ack));
        status.responded_at = ActiveValue::Set(Some(now().unix_timestamp()));
        if let Some(psbt_op_file_idx) = psbt_op_file_idx {
            status.psbt_op_file_idx = ActiveValue::Set(Some(psbt_op_file_idx));
        }
        state
            .database
            .update_cosigner_op_status(status, Some(&txn))
            .await?;

        // count ACKs and NACKs to determine new status
        let status_entries = state
            .database
            .iter_cosigner_op_status_by_operation_idx(op.idx, &txn)
            .await?;
        let (ack_count, nack_count) =
            status_entries
                .iter()
                .fold((0u8, 0u8), |(ack, nack), s| match s.ack {
                    Some(true) => (ack + 1, nack),
                    Some(false) => (ack, nack + 1),
                    None => (ack, nack),
                });
        let threshold = get_threshold_for_operation(
            &op.r#type,
            state.threshold_vanilla,
            state.threshold_colored,
        )
        .expect("threshold should be set for non-auto-approved operations");
        let total_cosigners = state.cosigners_by_xpub.len() as u8;
        let new_status = if ack_count >= threshold {
            Some(OperationStatus::Approved)
        } else if nack_count > (total_cosigners - threshold) {
            Some(OperationStatus::Discarded)
        } else {
            None
        };

        // update operation status if the operation is now approved or discarded
        if let Some(status) = new_status {
            let mut operation: operation::ActiveModel = op.into();
            operation.status = ActiveValue::Set(status);
            state.database.update_operation(operation, &txn).await?;
            tracing::debug!("Operation new status: {:?}", new_status);
        }

        // commit transaction
        txn.commit().await?;

        // get updated operation response
        let operation_response = state
            .get_operation_by_idx_with_files(req.operation_idx, Some(cosigner_idx))
            .await?
            .expect("operation should exist after response");

        Ok(Json(operation_response))
    })
    .await
}

#[derive(Deserialize, Serialize)]
pub(crate) struct TransferStatusRequest {
    pub(crate) batch_transfer_idx: i32,
    pub(crate) accept: Option<bool>,
}

#[derive(Deserialize, Serialize)]
pub(crate) struct TransferStatusInfo {
    pub(crate) cosigner_xpub: String,
    pub(crate) accepted: bool,
    pub(crate) registered_at: i64,
}

#[derive(Deserialize, Serialize)]
pub(crate) struct TransferStatusResponse {
    pub(crate) status: Option<TransferStatusInfo>,
}

pub(crate) async fn transfer_status(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    WithRejection(Json(req), _): WithRejection<Json<TransferStatusRequest>, APIError>,
) -> Result<Json<TransferStatusResponse>, APIError> {
    if let Some(accepted) = req.accept {
        // only cosigners may set the transfer status
        let AuthenticatedUser::Cosigner(cosigner) = &user else {
            return Err(APIError::Forbidden);
        };
        let cosigner_idx = cosigner.idx;

        // acquire write lock to prevent concurrent write operations
        let _lock = state.write_lock.lock().await;

        let registered_at = now().unix_timestamp();
        let txn = state.database.begin_transaction().await?;
        let row = state
            .database
            .set_transfer_status(
                req.batch_transfer_idx,
                cosigner_idx,
                registered_at,
                accepted,
                &txn,
            )
            .await?;
        txn.commit().await?;

        let cosigner_xpub = state
            .cosigners_by_idx
            .get(&row.cosigner_idx)
            .cloned()
            .unwrap_or_default();
        Ok(Json(TransferStatusResponse {
            status: Some(TransferStatusInfo {
                cosigner_xpub,
                accepted: row.accepted,
                registered_at: row.registered_at,
            }),
        }))
    } else {
        let row = state
            .database
            .get_transfer_status(req.batch_transfer_idx)
            .await?;
        let status = row.map(|r| {
            let cosigner_xpub = state
                .cosigners_by_idx
                .get(&r.cosigner_idx)
                .cloned()
                .unwrap_or_default();
            TransferStatusInfo {
                cosigner_xpub,
                accepted: r.accepted,
                registered_at: r.registered_at,
            }
        });
        Ok(Json(TransferStatusResponse { status }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_try_from_operation_type() {
        assert!(matches!(
            OperationType::try_from(1),
            Ok(OperationType::CreateUtxos)
        ));
        assert!(matches!(
            OperationType::try_from(2),
            Ok(OperationType::Issuance)
        ));
        assert!(matches!(
            OperationType::try_from(3),
            Ok(OperationType::SendRgb)
        ));
        assert!(matches!(
            OperationType::try_from(4),
            Ok(OperationType::SendBtc)
        ));
        assert!(matches!(
            OperationType::try_from(5),
            Ok(OperationType::Inflation)
        ));
        assert!(matches!(
            OperationType::try_from(6),
            Ok(OperationType::BlindReceive)
        ));
        assert!(matches!(
            OperationType::try_from(7),
            Ok(OperationType::WitnessReceive)
        ));
        assert!(matches!(
            OperationType::try_from(8),
            Ok(OperationType::Burn)
        ));
        assert!(matches!(
            OperationType::try_from(9),
            Err(APIError::InvalidOperationType(9))
        ));
    }
}
