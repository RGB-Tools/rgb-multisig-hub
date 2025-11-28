use super::*;

const TEST_DIR_BASE: &str = "tmp/get_current_address_indices/";

const PATH: &str = "getcurrentaddressindices";

#[serial_test::serial]
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
#[traced_test]
async fn success() {
    let app_dir = format!("{TEST_DIR_BASE}success");

    let ctx = setup_daemon(&app_dir).await;

    // get current address indices with cosigner 0
    let res = get_current_address_indices(&ctx).await;
    assert_eq!(res.internal, None);
    assert_eq!(res.external, None);

    // get current address indices with a different cosiger
    let res = reqwest::Client::new()
        .get(format!("http://{}/{}", ctx.node_address, PATH))
        .bearer_auth(ctx.get_cosigner_token(1))
        .send()
        .await
        .unwrap();
    let res = check_response_is_ok(res)
        .await
        .json::<APIResponse<GetCurrentAddressIndicesResponse>>()
        .await
        .unwrap();
    match res {
        APIResponse::Success(res) => {
            assert_eq!(res.internal, None);
            assert_eq!(res.external, None);
        }
        APIResponse::Error(error) => {
            panic!("Failed to get current address indices: {error:?}");
        }
    }

    // get current address indices as watch-only
    let res = reqwest::Client::new()
        .get(format!("http://{}/{}", ctx.node_address, PATH))
        .bearer_auth(&ctx.watch_only_token)
        .send()
        .await
        .unwrap();
    let res = check_response_is_ok(res)
        .await
        .json::<APIResponse<GetCurrentAddressIndicesResponse>>()
        .await
        .unwrap();
    match res {
        APIResponse::Success(res) => {
            assert_eq!(res.internal, None);
            assert_eq!(res.external, None);
        }
        APIResponse::Error(error) => {
            panic!("Failed to get current address indices: {error:?}");
        }
    }

    // bump external indices by 5
    let ext_count_1 = 5;
    let bump_response = bump_address_indices(&ctx, ext_count_1, false).await;
    assert_eq!(bump_response.first, 0);
    let res = get_current_address_indices(&ctx).await;
    assert_eq!(res.internal, None);
    assert_eq!(res.external, Some(ext_count_1 as u32 - 1));

    // bump internal indices by 3
    let int_count = 3;
    let bump_response = bump_address_indices(&ctx, int_count, true).await;
    assert_eq!(bump_response.first, 0);
    let res = get_current_address_indices(&ctx).await;
    assert_eq!(res.internal, Some(int_count as u32 - 1));
    assert_eq!(res.external, Some(ext_count_1 as u32 - 1));

    // bump external indices again by 10
    let ext_count_2 = 10;
    let bump_response = bump_address_indices(&ctx, ext_count_2, false).await;
    assert_eq!(bump_response.first, ext_count_1 as u32);
    let res = get_current_address_indices(&ctx).await;
    assert_eq!(res.internal, Some(int_count as u32 - 1));
    assert_eq!(
        res.external,
        Some(ext_count_1 as u32 + ext_count_2 as u32 - 1)
    );
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
            allows_watch_only: true,
        },
    )
    .await;
}
