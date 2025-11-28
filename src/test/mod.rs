use std::{
    net::SocketAddr,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

use amplify::s;
use biscuit_auth::{KeyPair, builder::date, macros::*};
use bytes::Bytes;
use chrono::{DateTime, Utc};
use futures_util::stream;
use reqwest::{Body, Response, header, multipart};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tracing_test::traced_test;

use crate::routes::{
    BumpAddressIndicesRequest, BumpAddressIndicesResponse, EmptyResponse, FileType,
    GetCurrentAddressIndicesResponse, GetFileRequest, GetLastProcessedOpIdxResponse,
    GetOperationByIdxRequest, InfoResponse, MarkOperationProcessedRequest, OperationResponse,
    OperationStatus, OperationType, PostOperationResponse, RespondToOperationRequest, UserRole,
};
use crate::startup::{FILES_DIR, MAX_RGB_LIB_VERSION, MIN_RGB_LIB_VERSION};

use super::*;

const JSON: &str = "application/json";
const OCTET_STREAM: &str = "application/octet-stream";

static COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Deserialize, Serialize)]
struct APIErrorBody {
    error: String,
    code: u16,
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum APIResponse<T> {
    Success(T),
    Error(APIErrorBody),
}

struct TestContext {
    node_address: SocketAddr,
    watch_only_token: String,
    cosigners: Vec<(String, String)>,
    root_keypair: KeyPair,
    rgb_lib_version: String,
}

impl TestContext {
    fn get_cosigner_xpub(&self, cosigner_idx: i32) -> String {
        self.cosigners.get(cosigner_idx as usize).unwrap().0.clone()
    }

    fn get_cosigner_token(&self, cosigner_idx: i32) -> String {
        self.cosigners.get(cosigner_idx as usize).unwrap().1.clone()
    }

    fn num_cosigners(&self) -> i32 {
        self.cosigners.len() as i32
    }
}

fn create_token(root: &KeyPair, role: UserRole, expiration_date: Option<DateTime<Utc>>) -> String {
    let mut authority = biscuit!("");
    match role {
        UserRole::Cosigner(xpub) => {
            authority = biscuit_merge!(authority, r#"role("cosigner"); xpub({xpub});"#);
        }
        UserRole::WatchOnly => {
            authority = biscuit_merge!(authority, r#"role("watch-only");"#);
        }
    }
    if let Some(expiration_date) = expiration_date {
        let exp = date(&expiration_date.into());
        authority = biscuit_merge!(authority, r#"check if time($t), $t < {exp};"#);
    }
    authority.build(root).unwrap().to_base64().unwrap()
}

async fn check_response_is_ok(res: Response) -> Response {
    if res.status() != reqwest::StatusCode::OK {
        panic!("reqwest response is not OK: {:?}", res.text().await);
    }
    res
}

async fn check_response_is_nok(
    res: Response,
    expected_status: reqwest::StatusCode,
    expected_message: &str,
    expected_name: &str,
) {
    assert_eq!(res.status(), expected_status);
    let api_error_response = res.json::<APIErrorBody>().await.unwrap();
    assert_eq!(api_error_response.code, expected_status.as_u16());
    if !api_error_response.error.contains(expected_message) {
        panic!(
            "unexpected error message: {} (expecting: {})",
            api_error_response.error, expected_message
        );
    }
    assert_eq!(api_error_response.name, expected_name);
}

async fn check_response_passed_auth(res: Response) {
    let status = res.status();
    if [
        reqwest::StatusCode::UNAUTHORIZED,
        reqwest::StatusCode::FORBIDDEN,
    ]
    .contains(&status)
    {
        panic!(
            "failed auth check: status={}, body={:?}",
            status,
            res.text().await
        );
    }
}

pub(crate) fn unique_bytes() -> Vec<u8> {
    COUNTER
        .fetch_add(1, Ordering::SeqCst)
        .to_be_bytes()
        .to_vec()
}

async fn start_daemon(
    app_dir: &str,
    root_public_key: biscuit_auth::PublicKey,
    cosigners: Vec<(String, String)>,
    threshold_colored: u8,
    threshold_vanilla: u8,
    rgb_lib_version: String,
) -> SocketAddr {
    let listener = TcpListener::bind("0.0.0.0:0").await.unwrap();
    let node_address = listener.local_addr().unwrap();
    let _ = std::fs::remove_dir_all(app_dir);
    std::fs::create_dir_all(app_dir).unwrap();
    let app_params = AppParams {
        app_dir: app_dir.into(),
        daemon_listening_port: 3001,
        root_public_key,
        cosigner_xpubs: cosigners.iter().map(|(xpub, _)| xpub.clone()).collect(),
        threshold_colored,
        threshold_vanilla,
        rgb_lib_version,
    };
    tokio::spawn(async move {
        let (router, app_state) = app(app_params).await.unwrap();
        axum::serve(listener, router)
            .with_graceful_shutdown(shutdown_signal(app_state))
            .await
            .unwrap();
    });
    node_address
}

async fn setup_daemon(app_dir: &str) -> TestContext {
    let root_keypair = KeyPair::new();
    let mut cosigners = Vec::new();
    for i in 0..4 {
        let xpub = format!("xpub{i}");
        cosigners.push((
            xpub.clone(),
            create_token(&root_keypair, UserRole::Cosigner(xpub), None),
        ));
    }
    let watch_only_token = create_token(&root_keypair, UserRole::WatchOnly, None);
    let rgb_lib_version = "0.3".to_string();
    let node_address = start_daemon(
        app_dir,
        root_keypair.public(),
        cosigners.clone(),
        3,
        3,
        rgb_lib_version.clone(),
    )
    .await;
    TestContext {
        node_address,
        watch_only_token,
        cosigners,
        root_keypair,
        rgb_lib_version,
    }
}

async fn setup_with_pending_operation(app_dir: &str) -> (TestContext, i32) {
    let ctx = setup_daemon(app_dir).await;
    let res = post_operation(&ctx, OperationType::SendRgb).await;
    (ctx, res.operation_idx)
}

async fn setup_with_approved_operation(app_dir: &str) -> (TestContext, i32) {
    let (ctx, operation_idx) = setup_with_pending_operation(app_dir).await;
    for cosigner_idx in 0..=2 {
        let req = RespondToOperationRequest {
            operation_idx,
            ack: true,
        };
        let json_payload = serde_json::to_string(&req).unwrap();
        let json_part = multipart::Part::text(json_payload).mime_str(JSON).unwrap();
        let psbt_part = multipart::Part::bytes(b"psbt".to_vec())
            .mime_str(OCTET_STREAM)
            .unwrap();
        let form = multipart::Form::new()
            .part("request", json_part)
            .part("file_psbt", psbt_part);
        respond_to_operation(&ctx, form, cosigner_idx).await;
    }
    (ctx, operation_idx)
}

// API helpers

async fn bump_address_indices(
    ctx: &TestContext,
    count: u8,
    internal: bool,
) -> BumpAddressIndicesResponse {
    let req = BumpAddressIndicesRequest { count, internal };
    let res = reqwest::Client::new()
        .post(format!("http://{}/bumpaddressindices", ctx.node_address))
        .bearer_auth(ctx.get_cosigner_token(0))
        .json(&req)
        .send()
        .await
        .unwrap()
        .json::<APIResponse<BumpAddressIndicesResponse>>()
        .await
        .unwrap();
    match res {
        APIResponse::Success(res) => res,
        APIResponse::Error(error) => {
            panic!("failed to bump address indices: {error:?}");
        }
    }
}

async fn get_current_address_indices(ctx: &TestContext) -> GetCurrentAddressIndicesResponse {
    let res = reqwest::Client::new()
        .get(format!(
            "http://{}/getcurrentaddressindices",
            ctx.node_address
        ))
        .bearer_auth(ctx.get_cosigner_token(0))
        .send()
        .await
        .unwrap();
    let res = check_response_is_ok(res)
        .await
        .json::<APIResponse<GetCurrentAddressIndicesResponse>>()
        .await
        .unwrap();
    match res {
        APIResponse::Success(res) => res,
        APIResponse::Error(error) => {
            panic!("failed to get current address indices: {error:?}");
        }
    }
}

async fn get_file(
    ctx: &TestContext,
    file_id: String,
    cosigner_idx: Option<i32>,
) -> reqwest::Response {
    let req = GetFileRequest { file_id };
    let token = match cosigner_idx {
        Some(cosigner_idx) => ctx.get_cosigner_token(cosigner_idx),
        None => ctx.watch_only_token.clone(),
    };
    let res = reqwest::Client::new()
        .post(format!("http://{}/getfile", ctx.node_address))
        .bearer_auth(token)
        .json(&req)
        .send()
        .await
        .unwrap();
    check_response_is_ok(res).await
}

async fn get_last_processed_op_idx(
    ctx: &TestContext,
    cosigner_idx: i32,
) -> GetLastProcessedOpIdxResponse {
    let res = reqwest::Client::new()
        .get(format!("http://{}/getlastprocessedopidx", ctx.node_address))
        .bearer_auth(ctx.get_cosigner_token(cosigner_idx))
        .send()
        .await
        .unwrap();
    let res = check_response_is_ok(res)
        .await
        .json::<APIResponse<GetLastProcessedOpIdxResponse>>()
        .await
        .unwrap();
    match res {
        APIResponse::Success(res) => res,
        APIResponse::Error(error) => {
            panic!("failed to get last processed op idx: {error:?}");
        }
    }
}

async fn get_operation_by_idx(
    ctx: &TestContext,
    operation_idx: i32,
    cosigner_idx: Option<i32>,
) -> Option<OperationResponse> {
    let req = GetOperationByIdxRequest { operation_idx };
    let token = match cosigner_idx {
        Some(cosigner_idx) => ctx.get_cosigner_token(cosigner_idx),
        None => ctx.watch_only_token.clone(),
    };
    let res = reqwest::Client::new()
        .post(format!("http://{}/getoperationbyidx", ctx.node_address))
        .bearer_auth(token)
        .json(&req)
        .send()
        .await
        .unwrap();
    let res = check_response_is_ok(res)
        .await
        .json::<APIResponse<Option<OperationResponse>>>()
        .await
        .unwrap();
    match res {
        APIResponse::Success(Some(res)) => Some(res),
        APIResponse::Success(None) => None,
        APIResponse::Error(error) => {
            panic!("failed to get operation: {error:?}");
        }
    }
}

async fn info(ctx: &TestContext, cosigner_idx: Option<i32>) -> InfoResponse {
    let token = match cosigner_idx {
        Some(cosigner_idx) => ctx.get_cosigner_token(cosigner_idx),
        None => ctx.watch_only_token.clone(),
    };
    let res = reqwest::Client::new()
        .get(format!("http://{}/info", ctx.node_address))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    let res = check_response_is_ok(res)
        .await
        .json::<APIResponse<InfoResponse>>()
        .await
        .unwrap();
    match res {
        APIResponse::Success(res) => res,
        APIResponse::Error(error) => {
            panic!("failed to get info: {error:?}");
        }
    }
}

async fn mark_operation_processed(ctx: &TestContext, operation_idx: i32, cosigner_idx: i32) {
    let req = MarkOperationProcessedRequest { operation_idx };
    let res = reqwest::Client::new()
        .post(format!(
            "http://{}/markoperationprocessed",
            ctx.node_address
        ))
        .bearer_auth(ctx.get_cosigner_token(cosigner_idx))
        .json(&req)
        .send()
        .await
        .unwrap();
    let res = check_response_is_ok(res)
        .await
        .json::<APIResponse<EmptyResponse>>()
        .await
        .unwrap();
    match res {
        APIResponse::Success(_) => {}
        APIResponse::Error(error) => {
            panic!("failed to mark operation as processed: {error:?}");
        }
    }
}

async fn post_operation(ctx: &TestContext, operation_type: OperationType) -> PostOperationResponse {
    let operation_type_part = multipart::Part::bytes((operation_type as u8).to_le_bytes().to_vec());
    let psbt_part = multipart::Part::bytes(b"psbt".to_vec());
    let form = multipart::Form::new()
        .part("operation_type", operation_type_part)
        .part("file_psbt", psbt_part);
    post_operation_with_multipart_form(ctx, form, 0).await
}

async fn post_operation_with_multipart_form(
    ctx: &TestContext,
    form: multipart::Form,
    cosigner_idx: i32,
) -> PostOperationResponse {
    let res = reqwest::Client::new()
        .post(format!("http://{}/postoperation", ctx.node_address))
        .bearer_auth(ctx.get_cosigner_token(cosigner_idx))
        .multipart(form)
        .send()
        .await
        .unwrap()
        .json::<APIResponse<PostOperationResponse>>()
        .await
        .unwrap();
    match res {
        APIResponse::Success(res) => res,
        APIResponse::Error(error) => {
            panic!("failed to post operation: {error:?}");
        }
    }
}

fn respond_to_operation_form(operation_idx: i32, ack: bool, with_psbt: bool) -> multipart::Form {
    let req = RespondToOperationRequest { operation_idx, ack };
    let json_payload = serde_json::to_string(&req).unwrap();
    let json_part = multipart::Part::text(json_payload).mime_str(JSON).unwrap();
    let mut form = multipart::Form::new().part("request", json_part);
    if with_psbt {
        let psbt_part = multipart::Part::bytes(unique_bytes())
            .mime_str(OCTET_STREAM)
            .unwrap();
        form = form.part("file_psbt", psbt_part);
    }
    form
}

async fn respond_to_operation(
    ctx: &TestContext,
    form: multipart::Form,
    cosigner_idx: i32,
) -> OperationResponse {
    let res = reqwest::Client::new()
        .post(format!("http://{}/respondtooperation", ctx.node_address))
        .bearer_auth(ctx.get_cosigner_token(cosigner_idx))
        .multipart(form)
        .send()
        .await
        .unwrap()
        .json::<APIResponse<OperationResponse>>()
        .await
        .unwrap();
    match res {
        APIResponse::Success(res) => res,
        APIResponse::Error(error) => {
            panic!("failed to respond to operation: {error:?}");
        }
    }
}

// common test checks

#[derive(Clone, Debug)]
pub(crate) struct APIInfo {
    pub(crate) method: reqwest::Method,
    pub(crate) path: String,
}

pub(crate) struct TokenChecks {
    pub(crate) api_info: APIInfo,
    pub(crate) allows_watch_only: bool,
}

async fn token_checks(ctx: &TestContext, token_checks: TokenChecks) {
    // no token
    let res = reqwest::Client::new()
        .request(
            token_checks.api_info.method.clone(),
            format!("http://{}/{}", ctx.node_address, token_checks.api_info.path),
        )
        .send()
        .await
        .unwrap();
    check_response_is_nok(
        res,
        reqwest::StatusCode::UNAUTHORIZED,
        "Missing or invalid credentials",
        "Unauthorized",
    )
    .await;

    // invalid token
    let invalid_token = "invalid";
    let res = reqwest::Client::new()
        .request(
            token_checks.api_info.method.clone(),
            format!("http://{}/{}", ctx.node_address, token_checks.api_info.path),
        )
        .bearer_auth(invalid_token)
        .send()
        .await
        .unwrap();
    check_response_is_nok(
        res,
        reqwest::StatusCode::UNAUTHORIZED,
        "Missing or invalid credentials",
        "Unauthorized",
    )
    .await;

    // expired token
    let expired_token = create_token(
        &ctx.root_keypair,
        UserRole::Cosigner(ctx.cosigners[0].0.clone()),
        Some(Utc::now() - Duration::from_secs(1)),
    );
    let res = reqwest::Client::new()
        .request(
            token_checks.api_info.method.clone(),
            format!("http://{}/{}", ctx.node_address, token_checks.api_info.path),
        )
        .bearer_auth(&expired_token)
        .send()
        .await
        .unwrap();
    check_response_is_nok(
        res,
        reqwest::StatusCode::UNAUTHORIZED,
        "Missing or invalid credentials",
        "Unauthorized",
    )
    .await;

    // watch-only
    if token_checks.allows_watch_only {
        let res = reqwest::Client::new()
            .request(
                token_checks.api_info.method.clone(),
                format!("http://{}/{}", ctx.node_address, token_checks.api_info.path),
            )
            .bearer_auth(&ctx.watch_only_token)
            .send()
            .await
            .unwrap();
        check_response_passed_auth(res).await;
    } else {
        let res = reqwest::Client::new()
            .request(
                token_checks.api_info.method.clone(),
                format!("http://{}/{}", ctx.node_address, token_checks.api_info.path),
            )
            .bearer_auth(&ctx.watch_only_token)
            .send()
            .await
            .unwrap();
        check_response_is_nok(
            res,
            reqwest::StatusCode::FORBIDDEN,
            "You don't have access to this resource",
            "Forbidden",
        )
        .await;
    }

    // unsupported token
    let mut unsupported_tokens = vec![];
    // - role cosigner but no xpub
    let mut authority = biscuit!("");
    authority = biscuit_merge!(authority, r#"role("cosigner");"#);
    let unsupported_token = authority
        .build(&ctx.root_keypair)
        .unwrap()
        .to_base64()
        .unwrap();
    unsupported_tokens.push(unsupported_token);
    // - role watch-only but has xpub
    let mut authority = biscuit!("");
    authority = biscuit_merge!(authority, r#"role("watch_only");"#);
    authority = biscuit_merge!(
        authority,
        r#"xpub({xpub});"#,
        xpub = ctx.cosigners[0].0.clone()
    );
    let unsupported_token = authority
        .build(&ctx.root_keypair)
        .unwrap()
        .to_base64()
        .unwrap();
    unsupported_tokens.push(unsupported_token);
    // - unknown role
    let mut authority = biscuit!("");
    authority = biscuit_merge!(authority, r#"role("unknown");"#);
    let unsupported_token = authority
        .build(&ctx.root_keypair)
        .unwrap()
        .to_base64()
        .unwrap();
    unsupported_tokens.push(unsupported_token);
    // - checks
    for unsupported_token in unsupported_tokens {
        let res = reqwest::Client::new()
            .request(
                token_checks.api_info.method.clone(),
                format!("http://{}/{}", ctx.node_address, token_checks.api_info.path),
            )
            .bearer_auth(&unsupported_token)
            .send()
            .await
            .unwrap();
        check_response_is_nok(
            res,
            reqwest::StatusCode::UNAUTHORIZED,
            "Missing or invalid credentials",
            "Unauthorized",
        )
        .await;
    }
}

async fn json_body_checks(ctx: &TestContext, api_info: APIInfo) {
    // missing JSON body
    let res = reqwest::Client::new()
        .request(
            api_info.method.clone(),
            format!("http://{}/{}", ctx.node_address, api_info.path),
        )
        .bearer_auth(ctx.get_cosigner_token(0))
        .send()
        .await
        .unwrap();
    check_response_is_nok(
        res,
        reqwest::StatusCode::BAD_REQUEST,
        "Invalid request",
        "InvalidRequest",
    )
    .await;

    // invalid JSON
    let res = reqwest::Client::new()
        .request(
            api_info.method.clone(),
            format!("http://{}/{}", ctx.node_address, api_info.path),
        )
        .bearer_auth(ctx.get_cosigner_token(0))
        .header(header::CONTENT_TYPE, "application/json")
        .body("invalid json")
        .send()
        .await
        .unwrap();
    check_response_is_nok(
        res,
        reqwest::StatusCode::BAD_REQUEST,
        "Failed to parse the request body as JSON",
        "InvalidRequest",
    )
    .await;

    // malformed JSON
    let res = reqwest::Client::new()
        .request(
            api_info.method.clone(),
            format!("http://{}/{}", ctx.node_address, api_info.path),
        )
        .bearer_auth(ctx.get_cosigner_token(0))
        .header(header::CONTENT_TYPE, "application/json")
        .body("{}")
        .send()
        .await
        .unwrap();
    check_response_is_nok(
        res,
        reqwest::StatusCode::BAD_REQUEST,
        "Failed to deserialize the JSON body",
        "InvalidRequest",
    )
    .await;
}

pub(crate) struct MultipartFormChecks {
    pub(crate) api_info: APIInfo,
    pub(crate) field_name: String,
}

async fn multipart_form_checks(ctx: &TestContext, multipart_form_checks: MultipartFormChecks) {
    // missing multipart form
    let res = reqwest::Client::new()
        .request(
            multipart_form_checks.api_info.method.clone(),
            format!(
                "http://{}/{}",
                ctx.node_address, multipart_form_checks.api_info.path
            ),
        )
        .bearer_auth(ctx.get_cosigner_token(1))
        .send()
        .await
        .unwrap();
    check_response_is_nok(
        res,
        reqwest::StatusCode::BAD_REQUEST,
        "Invalid request",
        "InvalidRequest",
    )
    .await;

    // empty multipart form
    let res = reqwest::Client::new()
        .request(
            multipart_form_checks.api_info.method.clone(),
            format!(
                "http://{}/{}",
                ctx.node_address, multipart_form_checks.api_info.path
            ),
        )
        .bearer_auth(ctx.get_cosigner_token(1))
        .multipart(multipart::Form::new())
        .send()
        .await
        .unwrap();
    check_response_is_nok(
        res,
        reqwest::StatusCode::BAD_REQUEST,
        "failed to parse multipart",
        "InvalidRequest",
    )
    .await;

    // invalid json field
    let boundary = "BOUNDARY123";
    let prefix = format!(
        "--{boundary}\r\n\
         Content-Disposition: form-data; name=\"{field_name}\"\r\n\
         \r\n",
        field_name = multipart_form_checks.field_name
    );
    let chunks = vec![
        Ok::<Bytes, std::io::Error>(Bytes::from(prefix)),
        Ok(Bytes::from_static(b"some bytes but not complete...")),
        // stream ends here => server hits EOF while reading bytes()
    ];
    let body = Body::wrap_stream(stream::iter(chunks));
    let res = reqwest::Client::new()
        .request(
            multipart_form_checks.api_info.method.clone(),
            format!(
                "http://{}/{}",
                ctx.node_address, multipart_form_checks.api_info.path
            ),
        )
        .bearer_auth(ctx.get_cosigner_token(1))
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(body)
        .send()
        .await
        .unwrap();
    check_response_is_nok(
        res,
        reqwest::StatusCode::BAD_REQUEST,
        "failed to read field",
        "InvalidRequest",
    )
    .await;

    // unexpected multipart form field
    let json_part = multipart::Part::text("text");
    let form = multipart::Form::new().part("invalid", json_part);
    let res = reqwest::Client::new()
        .request(
            multipart_form_checks.api_info.method.clone(),
            format!(
                "http://{}/{}",
                ctx.node_address, multipart_form_checks.api_info.path
            ),
        )
        .bearer_auth(ctx.get_cosigner_token(1))
        .multipart(form)
        .send()
        .await
        .unwrap();
    check_response_is_nok(
        res,
        reqwest::StatusCode::BAD_REQUEST,
        "unexpected field 'invalid'",
        "InvalidRequest",
    )
    .await;

    // empty file
    let empty_psbt_part = multipart::Part::bytes(vec![]);
    let form = multipart::Form::new().part("file_psbt", empty_psbt_part);
    let res = reqwest::Client::new()
        .request(
            multipart_form_checks.api_info.method.clone(),
            format!(
                "http://{}/{}",
                ctx.node_address, multipart_form_checks.api_info.path
            ),
        )
        .bearer_auth(ctx.get_cosigner_token(0))
        .multipart(form)
        .send()
        .await
        .unwrap();
    check_response_is_nok(
        res,
        reqwest::StatusCode::BAD_REQUEST,
        "empty file",
        "InvalidRequest",
    )
    .await;
}

// test modules

mod bump_address_indices;
mod get_current_address_indices;
mod get_file;
mod get_last_processed_op_idx;
mod get_operation_by_idx;
mod info;
mod mark_operation_processed;
mod post_operation;
mod respond_to_operation;
