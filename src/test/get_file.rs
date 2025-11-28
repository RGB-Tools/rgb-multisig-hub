use super::*;

const TEST_DIR_BASE: &str = "tmp/get_file/";

const PATH: &str = "getfile";

#[serial_test::serial]
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
#[traced_test]
async fn success() {
    let app_dir = format!("{TEST_DIR_BASE}success");

    let (ctx, operation_idx) = setup_with_pending_operation(&app_dir).await;

    let res = get_operation_by_idx(&ctx, operation_idx, Some(0))
        .await
        .unwrap();
    assert!(!res.files.is_empty());
    let file_id = res.files[0].file_id.clone();

    // get file with cosigner 0
    let res = get_file(&ctx, file_id.clone(), Some(0)).await;
    assert_eq!(
        res.headers()
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap(),
        "application/octet-stream"
    );
    let body = res.bytes().await.unwrap();
    assert!(!body.is_empty());
    assert_eq!(&body[..], b"psbt");

    // get file with a different cosigner
    let res = get_file(&ctx, file_id.clone(), Some(1)).await;
    let body = res.bytes().await.unwrap();
    assert_eq!(&body[..], b"psbt");

    // get file with a watch-only wallet
    let res = get_file(&ctx, file_id, None).await;
    let body = res.bytes().await.unwrap();
    assert_eq!(&body[..], b"psbt");
}

#[serial_test::serial]
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
#[traced_test]
async fn fail() {
    let app_dir = format!("{TEST_DIR_BASE}fail");

    let ctx = setup_daemon(&app_dir).await;

    let api_info = APIInfo {
        method: reqwest::Method::POST,
        path: PATH.to_string(),
    };

    // token checks
    token_checks(
        &ctx,
        TokenChecks {
            api_info: api_info.clone(),
            allows_watch_only: true,
        },
    )
    .await;

    // JSON body checks
    json_body_checks(&ctx, api_info.clone()).await;

    // non-existent file
    let req = GetFileRequest {
        file_id: "non_existent_file_id".to_string(),
    };
    let res = reqwest::Client::new()
        .post(format!("http://{}/{}", ctx.node_address, PATH))
        .bearer_auth(ctx.get_cosigner_token(0))
        .json(&req)
        .send()
        .await
        .unwrap();
    check_response_is_nok(
        res,
        reqwest::StatusCode::BAD_REQUEST,
        "File not found",
        "FileNotFound",
    )
    .await;
}
