use super::*;

const TEST_DIR_BASE: &str = "tmp/get_last_processed_op_idx/";

const PATH: &str = "getlastprocessedopidx";

#[serial_test::serial]
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
#[traced_test]
async fn success() {
    let app_dir = format!("{TEST_DIR_BASE}success");

    let ctx = setup_daemon(&app_dir).await;

    // no processed operations
    for cosigner_idx in 0..ctx.num_cosigners() {
        let res = get_last_processed_op_idx(&ctx, cosigner_idx).await;
        assert_eq!(res.operation_idx, 0);
    }

    // initiate and approve an operation (idx 1)
    let (ctx, operation_idx_1) = setup_with_approved_operation(&app_dir).await;
    assert_eq!(operation_idx_1, 1);

    // still 0 for all cosigners (not yet marked as processed)
    for cosigner_idx in 0..ctx.num_cosigners() {
        let res = get_last_processed_op_idx(&ctx, cosigner_idx).await;
        assert_eq!(res.operation_idx, 0);
    }

    // cosigner 0 marks operation 1 as processed
    mark_operation_processed(&ctx, operation_idx_1, 0).await;
    let operation_idx_1 = get_last_processed_op_idx(&ctx, 0).await.operation_idx;
    assert_eq!(operation_idx_1, 1);
    for cosigner_idx in 1..ctx.num_cosigners() {
        let res = get_last_processed_op_idx(&ctx, cosigner_idx).await;
        assert_eq!(res.operation_idx, 0);
    }

    // initiate an approved operation (idx 2)
    let operation_idx_2 = post_operation(&ctx, OperationType::Issuance)
        .await
        .operation_idx;
    assert_eq!(operation_idx_2, 2);

    // all cosigners (except 1) mark operation 1 as processed
    for cosigner_idx in 1..ctx.num_cosigners() - 1 {
        mark_operation_processed(&ctx, operation_idx_1, cosigner_idx).await;
        let res = get_last_processed_op_idx(&ctx, cosigner_idx).await;
        assert_eq!(res.operation_idx, 1);
    }

    // cosigner 1 marks operation 2 as processed
    mark_operation_processed(&ctx, operation_idx_2, 1).await;
    let res = get_last_processed_op_idx(&ctx, 1).await;
    assert_eq!(res.operation_idx, 2);

    // cosigner 0 should still receive 1 (hasn't processed operation 2 yet)
    let res = get_last_processed_op_idx(&ctx, 0).await;
    assert_eq!(res.operation_idx, 1);

    // cosigner 2 marks operation 2 as processed
    mark_operation_processed(&ctx, operation_idx_2, 2).await;
    let res = get_last_processed_op_idx(&ctx, 2).await;
    assert_eq!(res.operation_idx, 2);

    // cosigner 0 marks operation 2 as processed
    mark_operation_processed(&ctx, operation_idx_2, 0).await;
    let res = get_last_processed_op_idx(&ctx, 0).await;
    assert_eq!(res.operation_idx, 2);

    // initiate a third approved operation (idx 3)
    let operation_idx_3 = post_operation(&ctx, OperationType::BlindReceive)
        .await
        .operation_idx;
    assert_eq!(operation_idx_3, 3);

    // cosigner 2 marks operation 3 as processed
    mark_operation_processed(&ctx, operation_idx_3, 2).await;
    let res = get_last_processed_op_idx(&ctx, 2).await;
    assert_eq!(res.operation_idx, 3);

    // verify final state for all cosigners
    let expected = [2, 2, 3, 0];
    for cosigner_idx in 0..ctx.num_cosigners() {
        let res = get_last_processed_op_idx(&ctx, cosigner_idx).await;
        assert_eq!(res.operation_idx, expected[cosigner_idx as usize]);
    }
}

#[serial_test::serial]
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
#[traced_test]
async fn fail() {
    let app_dir = format!("{TEST_DIR_BASE}fail");

    let ctx = setup_daemon(&app_dir).await;

    let api_info = APIInfo {
        method: reqwest::Method::GET,
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
}
