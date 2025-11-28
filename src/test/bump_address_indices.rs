use super::*;

const TEST_DIR_BASE: &str = "tmp/bump_address_indices/";

const PATH: &str = "bumpaddressindices";

#[serial_test::serial]
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
#[traced_test]
async fn success() {
    let app_dir = format!("{TEST_DIR_BASE}success");

    let ctx = setup_daemon(&app_dir).await;

    // bump external index by 5
    let ext_count_1 = 5;
    let res = bump_address_indices(&ctx, ext_count_1, false).await;
    assert_eq!(res.first, 0);
    let res = get_current_address_indices(&ctx).await;
    assert_eq!(res.internal, None);
    assert_eq!(res.external, Some(ext_count_1 as u32 - 1));

    // bump internal index by 3
    let int_count_1 = 3;
    let res = bump_address_indices(&ctx, int_count_1, true).await;
    assert_eq!(res.first, 0);
    let res = get_current_address_indices(&ctx).await;
    assert_eq!(res.internal, Some(int_count_1 as u32 - 1));
    assert_eq!(res.external, Some(ext_count_1 as u32 - 1));

    // bump external index again by 10
    let ext_count_2 = 10;
    let res = bump_address_indices(&ctx, ext_count_2, false).await;
    assert_eq!(res.first, ext_count_1 as u32);
    let res = get_current_address_indices(&ctx).await;
    assert_eq!(res.internal, Some(int_count_1 as u32 - 1));
    assert_eq!(
        res.external,
        Some(ext_count_1 as u32 + ext_count_2 as u32 - 1)
    );

    // bump internal index again by 7
    let int_count_2 = 7;
    let res = bump_address_indices(&ctx, int_count_2, true).await;
    assert_eq!(res.first, int_count_1 as u32);
    let res = get_current_address_indices(&ctx).await;
    assert_eq!(
        res.internal,
        Some(int_count_1 as u32 + int_count_2 as u32 - 1)
    );
    assert_eq!(
        res.external,
        Some(ext_count_1 as u32 + ext_count_2 as u32 - 1)
    );

    // bump external index again by 1
    let ext_count_3 = 1;
    let res = bump_address_indices(&ctx, ext_count_3, false).await;
    assert_eq!(res.first, ext_count_1 as u32 + ext_count_2 as u32);
    let res = get_current_address_indices(&ctx).await;
    assert_eq!(
        res.internal,
        Some(int_count_1 as u32 + int_count_2 as u32 - 1)
    );
    assert_eq!(
        res.external,
        Some(ext_count_1 as u32 + ext_count_2 as u32 + ext_count_3 as u32 - 1)
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
            allows_watch_only: false,
        },
    )
    .await;

    // JSON body checks
    json_body_checks(&ctx, api_info.clone()).await;

    // invalid count
    let req = BumpAddressIndicesRequest {
        count: 0,
        internal: false,
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
        "Invalid count: must be greater than 0",
        "InvalidCount",
    )
    .await;
}
