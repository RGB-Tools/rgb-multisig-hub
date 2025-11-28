use super::*;

const TEST_DIR_BASE: &str = "tmp/mark_operation_processed/";

const PATH: &str = "markoperationprocessed";

#[serial_test::serial]
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
#[traced_test]
async fn success() {
    let app_dir = format!("{TEST_DIR_BASE}success");

    let (ctx, operation_idx) = setup_with_approved_operation(&app_dir).await;

    // no processed operations
    for cosigner_idx in 0..ctx.num_cosigners() {
        let res = get_last_processed_op_idx(&ctx, cosigner_idx).await;
        assert_eq!(res.operation_idx, 0);
    }

    // mark as processed by cosigner 0
    let res = get_operation_by_idx(&ctx, operation_idx, Some(0))
        .await
        .unwrap();
    assert!(res.processed_at.is_none());
    mark_operation_processed(&ctx, operation_idx, 0).await;
    let res = get_operation_by_idx(&ctx, operation_idx, Some(0))
        .await
        .unwrap();
    assert!(res.processed_at.is_some());

    // mark as processed by other cosigners
    for cosigner_idx in 1..ctx.num_cosigners() {
        let res = get_operation_by_idx(&ctx, operation_idx, Some(cosigner_idx))
            .await
            .unwrap();
        assert!(res.processed_at.is_none());
        let res = get_last_processed_op_idx(&ctx, cosigner_idx).await;
        assert_eq!(res.operation_idx, 0);
        mark_operation_processed(&ctx, operation_idx, cosigner_idx).await;
        let res = get_last_processed_op_idx(&ctx, cosigner_idx).await;
        assert_eq!(res.operation_idx, operation_idx);
    }
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
            allows_watch_only: false,
        },
    )
    .await;

    // JSON body checks
    json_body_checks(&ctx, api_info.clone()).await;

    // non-existent operation
    let req = MarkOperationProcessedRequest {
        operation_idx: 9999,
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
        "Operation not found",
        "OperationNotFound",
    )
    .await;

    // already marked as processed
    let operation_idx = post_operation(&ctx, OperationType::Issuance)
        .await
        .operation_idx;
    mark_operation_processed(&ctx, operation_idx, 0).await;
    let req = MarkOperationProcessedRequest { operation_idx };
    let res = reqwest::Client::new()
        .post(format!("http://{}/{}", ctx.node_address, PATH))
        .bearer_auth(ctx.get_cosigner_token(0))
        .json(&req)
        .send()
        .await
        .unwrap();
    check_response_is_nok(
        res,
        reqwest::StatusCode::FORBIDDEN,
        "Cannot mark operation as processed: already marked this operation as processed",
        "CannotMarkOperationProcessed",
    )
    .await;

    // pending operation
    let operation_idx = post_operation(&ctx, OperationType::SendRgb)
        .await
        .operation_idx;
    let req = MarkOperationProcessedRequest { operation_idx };
    let res = reqwest::Client::new()
        .post(format!("http://{}/{}", ctx.node_address, PATH))
        .bearer_auth(ctx.get_cosigner_token(0))
        .json(&req)
        .send()
        .await
        .unwrap();
    check_response_is_nok(
        res,
        reqwest::StatusCode::FORBIDDEN,
        "Cannot mark operation as processed: a pending operation cannot be marked as processed",
        "CannotMarkOperationProcessed",
    )
    .await;
}
