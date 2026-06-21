use super::*;

const TEST_DIR_BASE: &str = "tmp/transfer_status/";

const PATH: &str = "transferstatus";

#[serial_test::serial]
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
#[traced_test]
async fn success() {
    let app_dir = format!("{TEST_DIR_BASE}success");
    let ctx = setup_daemon(&app_dir).await;

    let batch_transfer_idx = 42;

    // reading an unset transfer status returns None for both cosigners and watch-only users
    let res = transfer_status(&ctx, batch_transfer_idx, None, Some(0)).await;
    assert!(res.status.is_none());
    let res = transfer_status(&ctx, batch_transfer_idx, None, None).await;
    assert!(res.status.is_none());

    // a cosigner can register acceptance
    let before_set = Utc::now().timestamp();
    let res = transfer_status(&ctx, batch_transfer_idx, Some(true), Some(1)).await;
    let status = res.status.expect("status should be set");
    assert_eq!(status.cosigner_xpub, "xpub1");
    assert!(status.accepted);
    assert!(status.registered_at >= before_set);
    assert!(status.registered_at <= Utc::now().timestamp());

    // subsequent reads return the stored status
    let res = transfer_status(&ctx, batch_transfer_idx, None, Some(2)).await;
    let read_status = res.status.expect("status should be readable");
    assert_eq!(read_status.cosigner_xpub, status.cosigner_xpub);
    assert_eq!(read_status.accepted, status.accepted);
    assert_eq!(read_status.registered_at, status.registered_at);
    let res = transfer_status(&ctx, batch_transfer_idx, None, None).await;
    let watch_only_status = res
        .status
        .expect("watch-only should be able to read status");
    assert_eq!(watch_only_status.cosigner_xpub, status.cosigner_xpub);
    assert_eq!(watch_only_status.accepted, status.accepted);
    assert_eq!(watch_only_status.registered_at, status.registered_at);

    // setting the same status is idempotent and keeps the original cosigner/timestamp
    let res = transfer_status(&ctx, batch_transfer_idx, Some(true), Some(3)).await;
    let idempotent_status = res.status.expect("status should still be set");
    assert_eq!(idempotent_status.cosigner_xpub, status.cosigner_xpub);
    assert_eq!(idempotent_status.accepted, status.accepted);
    assert_eq!(idempotent_status.registered_at, status.registered_at);

    // a different batch can store a rejection independently
    let rejected_batch_transfer_idx = 43;
    let res = transfer_status(&ctx, rejected_batch_transfer_idx, Some(false), Some(2)).await;
    let rejected_status = res.status.expect("rejection should be set");
    assert_eq!(rejected_status.cosigner_xpub, "xpub2");
    assert!(!rejected_status.accepted);
    let res = transfer_status(&ctx, rejected_batch_transfer_idx, None, None).await;
    let read_rejected_status = res.status.expect("rejection should be readable");
    assert_eq!(
        read_rejected_status.cosigner_xpub,
        rejected_status.cosigner_xpub
    );
    assert_eq!(read_rejected_status.accepted, rejected_status.accepted);
    assert_eq!(
        read_rejected_status.registered_at,
        rejected_status.registered_at
    );
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

    // watch-only users can read but cannot set transfer status
    let req = TransferStatusRequest {
        batch_transfer_idx: 1,
        accept: Some(true),
    };
    let res = reqwest::Client::new()
        .post(format!("http://{}/{}", ctx.node_address, PATH))
        .bearer_auth(&ctx.watch_only_token)
        .json(&req)
        .send()
        .await
        .unwrap();
    check_response_is_nok(
        res,
        reqwest::StatusCode::FORBIDDEN,
        "Forbidden",
        "Forbidden",
    )
    .await;

    // once a status is set, it cannot be changed to the opposite value
    let batch_transfer_idx = 2;
    let res = transfer_status(&ctx, batch_transfer_idx, Some(true), Some(0)).await;
    assert!(res.status.expect("status should be set").accepted);
    let req = TransferStatusRequest {
        batch_transfer_idx,
        accept: Some(false),
    };
    let res = reqwest::Client::new()
        .post(format!("http://{}/{}", ctx.node_address, PATH))
        .bearer_auth(ctx.get_cosigner_token(1))
        .json(&req)
        .send()
        .await
        .unwrap();
    check_response_is_nok(
        res,
        reqwest::StatusCode::BAD_REQUEST,
        "Transfer status already set to a different value",
        "TransferStatusMismatch",
    )
    .await;
}
