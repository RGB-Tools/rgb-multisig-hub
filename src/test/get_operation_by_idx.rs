use super::*;

const TEST_DIR_BASE: &str = "tmp/get_operation/";

const PATH: &str = "getoperationbyidx";

#[serial_test::serial]
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
#[traced_test]
async fn success() {
    let app_dir = format!("{TEST_DIR_BASE}success");

    let (ctx, operation_idx) = setup_with_pending_operation(&app_dir).await;

    // get operation as initiator
    let res = get_operation_by_idx(&ctx, operation_idx, Some(0))
        .await
        .unwrap();
    assert_eq!(res.operation_idx, operation_idx);
    assert_eq!(res.initiator_xpub, "xpub0");
    assert_eq!(res.operation_type, OperationType::SendRgb);
    assert_eq!(res.my_response, None);

    // get operation as different cosigner
    let res = get_operation_by_idx(&ctx, operation_idx, Some(1))
        .await
        .unwrap();
    assert_eq!(res.operation_idx, operation_idx);
    assert_eq!(res.initiator_xpub, "xpub0");
    assert_eq!(res.operation_type, OperationType::SendRgb);
    assert!(res.my_response.is_none());

    // get operation as watch-only
    let res = get_operation_by_idx(&ctx, operation_idx, None)
        .await
        .unwrap();
    assert_eq!(res.operation_idx, operation_idx);
    assert_eq!(res.initiator_xpub, "xpub0");
    assert!(res.my_response.is_none());

    // get non-existent operation
    let res = get_operation_by_idx(&ctx, 9999, Some(0)).await;
    assert!(res.is_none());
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
}
