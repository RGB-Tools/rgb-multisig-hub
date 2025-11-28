use super::*;

const TEST_DIR_BASE: &str = "tmp/info/";

const PATH: &str = "info";

#[serial_test::serial]
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
#[traced_test]
async fn success() {
    let app_dir = format!("{TEST_DIR_BASE}success");

    let ctx = setup_daemon(&app_dir).await;

    for cosigner_idx in 0..ctx.num_cosigners() {
        let res = info(&ctx, Some(cosigner_idx)).await;
        assert_eq!(res.min_rgb_lib_version, MIN_RGB_LIB_VERSION);
        assert_eq!(res.max_rgb_lib_version, MAX_RGB_LIB_VERSION);
        assert_eq!(res.rgb_lib_version, ctx.rgb_lib_version);
        assert_eq!(res.last_operation_idx, None);
        assert_eq!(
            res.user_role,
            UserRole::Cosigner(ctx.get_cosigner_xpub(cosigner_idx))
        );
    }

    // watch-only
    let res = info(&ctx, None).await;
    assert_eq!(res.min_rgb_lib_version, MIN_RGB_LIB_VERSION);
    assert_eq!(res.max_rgb_lib_version, MAX_RGB_LIB_VERSION);
    assert_eq!(res.rgb_lib_version, ctx.rgb_lib_version);
    assert_eq!(res.last_operation_idx, None);
    assert_eq!(res.user_role, UserRole::WatchOnly);
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
