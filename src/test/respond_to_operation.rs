use super::*;

const TEST_DIR_BASE: &str = "tmp/respond_to_operation/";

const PATH: &str = "respondtooperation";

#[serial_test::serial]
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
#[traced_test]
async fn success() {
    let app_dir = format!("{TEST_DIR_BASE}success");

    let (ctx, operation_idx) = setup_with_pending_operation(&app_dir).await;

    let num_cosigners = ctx.num_cosigners();

    // operation discarded
    let form = respond_to_operation_form(operation_idx, true, true);
    let res = respond_to_operation(&ctx, form, 0).await;
    assert_eq!(res.acked_by.len(), 1);
    let form = respond_to_operation_form(operation_idx, true, true);
    let res = respond_to_operation(&ctx, form, 1).await;
    assert_eq!(res.operation_idx, operation_idx);
    assert_eq!(res.status, OperationStatus::Pending);
    assert_eq!(res.acked_by.len(), 2);
    assert!(res.acked_by.contains("xpub0"));
    assert!(res.acked_by.contains("xpub1"));
    assert_eq!(res.nacked_by.len(), 0);
    assert_eq!(res.my_response, Some(true));
    let psbt_files: Vec<_> = res
        .files
        .iter()
        .filter(|f| f.r#type == FileType::ResponsePsbt)
        .collect();
    assert!(psbt_files.len() >= 2);
    let files_dir = Path::new(&app_dir).join(FILES_DIR);
    for file in &res.files {
        let file_path = files_dir.join(&file.file_id);
        assert!(file_path.exists());
        let metadata = tokio::fs::metadata(&file_path).await.unwrap();
        assert_eq!(metadata.len(), file.size_bytes);
    }
    let form = respond_to_operation_form(operation_idx, false, false);
    let res = respond_to_operation(&ctx, form, 2).await;
    assert_eq!(res.operation_idx, operation_idx);
    assert_eq!(res.status, OperationStatus::Pending);
    let form = respond_to_operation_form(operation_idx, false, false);
    let res = respond_to_operation(&ctx, form, 3).await;
    assert_eq!(res.operation_idx, operation_idx);
    assert_eq!(res.status, OperationStatus::Discarded);
    for cosigner_idx in 0..num_cosigners {
        mark_operation_processed(&ctx, operation_idx, cosigner_idx).await;
    }

    // operation approved
    let operation_idx = post_operation(&ctx, OperationType::SendRgb)
        .await
        .operation_idx;
    let form = respond_to_operation_form(operation_idx, true, true);
    let res = respond_to_operation(&ctx, form, 0).await;
    assert_eq!(res.acked_by.len(), 1);
    let form = respond_to_operation_form(operation_idx, true, true);
    let res = respond_to_operation(&ctx, form, 1).await;
    assert_eq!(res.operation_idx, operation_idx);
    assert_eq!(res.status, OperationStatus::Pending);
    assert_eq!(res.acked_by.len(), 2);
    let form = respond_to_operation_form(operation_idx, true, true);
    let res = respond_to_operation(&ctx, form, 2).await;
    assert_eq!(res.operation_idx, operation_idx);
    assert_eq!(res.status, OperationStatus::Approved);
    assert_eq!(res.acked_by.len(), 3);
    assert!(res.acked_by.contains("xpub0"));
    assert!(res.acked_by.contains("xpub1"));
    assert!(res.acked_by.contains("xpub2"));
}

#[serial_test::serial]
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
#[traced_test]
async fn fail() {
    let app_dir = format!("{TEST_DIR_BASE}fail");

    let (ctx, operation_idx) = setup_with_pending_operation(&app_dir).await;

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

    // multipart form checks
    multipart_form_checks(
        &ctx,
        MultipartFormChecks {
            api_info: api_info.clone(),
            field_name: s!("request"),
        },
    )
    .await;

    // invalid json
    let json_part = multipart::Part::text("invalid json")
        .mime_str(JSON)
        .unwrap();
    let form = multipart::Form::new().part("request", json_part);
    let res = reqwest::Client::new()
        .post(format!("http://{}/{}", ctx.node_address, PATH))
        .bearer_auth(ctx.get_cosigner_token(1))
        .multipart(form)
        .send()
        .await
        .unwrap();
    check_response_is_nok(
        res,
        reqwest::StatusCode::BAD_REQUEST,
        "failed to parse JSON",
        "InvalidRequest",
    )
    .await;

    // more than one PSBT provided
    let req = RespondToOperationRequest {
        operation_idx,
        ack: true,
    };
    let json_payload = serde_json::to_string(&req).unwrap();
    let json_part = multipart::Part::text(json_payload).mime_str(JSON).unwrap();
    let psbt_part_1 = multipart::Part::bytes(unique_bytes())
        .mime_str(OCTET_STREAM)
        .unwrap();
    let psbt_part_2 = multipart::Part::bytes(unique_bytes())
        .mime_str(OCTET_STREAM)
        .unwrap();
    let form = multipart::Form::new()
        .part("request", json_part)
        .part("file_psbt", psbt_part_1)
        .part("file_psbt", psbt_part_2);
    let res = reqwest::Client::new()
        .post(format!("http://{}/{}", ctx.node_address, PATH))
        .bearer_auth(ctx.get_cosigner_token(1))
        .multipart(form)
        .send()
        .await
        .unwrap();
    check_response_is_nok(
        res,
        reqwest::StatusCode::BAD_REQUEST,
        "more than one PSBT provided",
        "InvalidRequest",
    )
    .await;

    // missing request body
    let psbt_part = multipart::Part::bytes(unique_bytes())
        .mime_str(OCTET_STREAM)
        .unwrap();
    let form = multipart::Form::new().part("file_psbt", psbt_part);
    let res = reqwest::Client::new()
        .post(format!("http://{}/{}", ctx.node_address, PATH))
        .bearer_auth(ctx.get_cosigner_token(2))
        .multipart(form)
        .send()
        .await
        .unwrap();
    check_response_is_nok(
        res,
        reqwest::StatusCode::BAD_REQUEST,
        "missing request body",
        "InvalidRequest",
    )
    .await;

    // ACK without PSBT
    let json_payload = serde_json::to_string(&req).unwrap();
    let json_part = multipart::Part::text(json_payload).mime_str(JSON).unwrap();
    let form = multipart::Form::new().part("request", json_part);
    let res = reqwest::Client::new()
        .post(format!("http://{}/{}", ctx.node_address, PATH))
        .bearer_auth(ctx.get_cosigner_token(1))
        .multipart(form)
        .send()
        .await
        .unwrap();
    check_response_is_nok(
        res,
        reqwest::StatusCode::BAD_REQUEST,
        "ACK requires PSBT file",
        "InvalidRequest",
    )
    .await;

    // already responded to this operation
    let req = RespondToOperationRequest {
        operation_idx,
        ack: true,
    };
    let json_payload = serde_json::to_string(&req).unwrap();
    let json_part = multipart::Part::text(json_payload.clone())
        .mime_str(JSON)
        .unwrap();
    let psbt_part = multipart::Part::bytes(unique_bytes())
        .mime_str(OCTET_STREAM)
        .unwrap();
    let form = multipart::Form::new()
        .part("request", json_part)
        .part("file_psbt", psbt_part);
    reqwest::Client::new()
        .post(format!("http://{}/{}", ctx.node_address, PATH))
        .bearer_auth(ctx.get_cosigner_token(1))
        .multipart(form)
        .send()
        .await
        .unwrap();
    let json_part = multipart::Part::text(json_payload.clone())
        .mime_str(JSON)
        .unwrap();
    let psbt_part = multipart::Part::bytes(unique_bytes())
        .mime_str(OCTET_STREAM)
        .unwrap();
    let form = multipart::Form::new()
        .part("request", json_part)
        .part("file_psbt", psbt_part);
    let res = reqwest::Client::new()
        .post(format!("http://{}/{}", ctx.node_address, PATH))
        .bearer_auth(ctx.get_cosigner_token(1))
        .multipart(form)
        .send()
        .await
        .unwrap();
    check_response_is_nok(
        res,
        reqwest::StatusCode::FORBIDDEN,
        "already responded to this operation",
        "CannotRespondToOperation",
    )
    .await;

    // respond to operation that is not pending
    let form = respond_to_operation_form(operation_idx, true, true);
    respond_to_operation(&ctx, form, 0).await;
    let json_part = multipart::Part::text(json_payload.clone())
        .mime_str(JSON)
        .unwrap();
    let psbt_part = multipart::Part::bytes(unique_bytes())
        .mime_str(OCTET_STREAM)
        .unwrap();
    let form = multipart::Form::new()
        .part("request", json_part)
        .part("file_psbt", psbt_part);
    reqwest::Client::new()
        .post(format!("http://{}/{}", ctx.node_address, PATH))
        .bearer_auth(ctx.get_cosigner_token(2))
        .multipart(form)
        .send()
        .await
        .unwrap();
    let json_part = multipart::Part::text(json_payload).mime_str(JSON).unwrap();
    let psbt_part = multipart::Part::bytes(unique_bytes())
        .mime_str(OCTET_STREAM)
        .unwrap();
    let form = multipart::Form::new()
        .part("request", json_part)
        .part("file_psbt", psbt_part);
    let res = reqwest::Client::new()
        .post(format!("http://{}/{}", ctx.node_address, PATH))
        .bearer_auth(ctx.get_cosigner_token(3))
        .multipart(form)
        .send()
        .await
        .unwrap();
    check_response_is_nok(
        res,
        reqwest::StatusCode::FORBIDDEN,
        "operation is not pending",
        "CannotRespondToOperation",
    )
    .await;

    // test that operations must be processed sequentially
    let app_dir_2 = format!("{TEST_DIR_BASE}fail_sequential");
    let (ctx2, operation_idx_1) = setup_with_approved_operation(&app_dir_2).await;
    for cosigner_idx in [0, 1] {
        let req = MarkOperationProcessedRequest {
            operation_idx: operation_idx_1,
        };
        let res = reqwest::Client::new()
            .post(format!(
                "http://{}/markoperationprocessed",
                ctx2.node_address
            ))
            .bearer_auth(ctx2.get_cosigner_token(cosigner_idx))
            .json(&req)
            .send()
            .await
            .unwrap();
        let _ = check_response_is_ok(res).await;
    }
    let operation_type_part =
        multipart::Part::bytes((OperationType::SendRgb as u8).to_le_bytes().to_vec());
    let psbt_part = multipart::Part::bytes(b"psbt2".to_vec());
    let form = multipart::Form::new()
        .part("operation_type", operation_type_part)
        .part("file_psbt", psbt_part);
    let res = reqwest::Client::new()
        .post(format!("http://{}/postoperation", ctx2.node_address))
        .bearer_auth(ctx2.get_cosigner_token(0))
        .multipart(form)
        .send()
        .await
        .unwrap()
        .json::<APIResponse<PostOperationResponse>>()
        .await
        .unwrap();
    let operation_idx_2 = match res {
        APIResponse::Success(res) => {
            assert_eq!(res.operation_idx, 2);
            res.operation_idx
        }
        APIResponse::Error(error) => {
            panic!("Failed to post operation 2: {error:?}");
        }
    };
    let req = RespondToOperationRequest {
        operation_idx: operation_idx_2,
        ack: true,
    };
    let json_payload = serde_json::to_string(&req).unwrap();
    let json_part = multipart::Part::text(json_payload).mime_str(JSON).unwrap();
    let psbt_part = multipart::Part::bytes(unique_bytes())
        .mime_str(OCTET_STREAM)
        .unwrap();
    let form = multipart::Form::new()
        .part("request", json_part)
        .part("file_psbt", psbt_part);
    let res = reqwest::Client::new()
        .post(format!("http://{}/{}", ctx2.node_address, PATH))
        .bearer_auth(ctx2.get_cosigner_token(2))
        .multipart(form)
        .send()
        .await
        .unwrap();
    check_response_is_nok(
        res,
        reqwest::StatusCode::FORBIDDEN,
        "operation is not the next one to be processed",
        "CannotRespondToOperation",
    )
    .await;
    let req = MarkOperationProcessedRequest {
        operation_idx: operation_idx_1,
    };
    let res = reqwest::Client::new()
        .post(format!(
            "http://{}/markoperationprocessed",
            ctx2.node_address
        ))
        .bearer_auth(ctx2.get_cosigner_token(2))
        .json(&req)
        .send()
        .await
        .unwrap();
    let _ = check_response_is_ok(res).await;
    let req = RespondToOperationRequest {
        operation_idx: operation_idx_2,
        ack: true,
    };
    let json_payload = serde_json::to_string(&req).unwrap();
    let json_part = multipart::Part::text(json_payload).mime_str(JSON).unwrap();
    let psbt_part = multipart::Part::bytes(unique_bytes())
        .mime_str(OCTET_STREAM)
        .unwrap();
    let form = multipart::Form::new()
        .part("request", json_part)
        .part("file_psbt", psbt_part);
    let res = reqwest::Client::new()
        .post(format!("http://{}/{}", ctx2.node_address, PATH))
        .bearer_auth(ctx2.get_cosigner_token(2))
        .multipart(form)
        .send()
        .await
        .unwrap();
    let _ = check_response_is_ok(res).await;
}
