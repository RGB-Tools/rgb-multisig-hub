use super::*;

const TEST_DIR_BASE: &str = "tmp/post_operation/";

const PATH: &str = "postoperation";

#[serial_test::serial]
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
#[traced_test]
async fn success() {
    // with PSBT and not auto-approved operation
    let app_dir = format!("{TEST_DIR_BASE}success_with_psbt");
    let before_post = Utc::now().timestamp();
    let ctx = setup_daemon(&app_dir).await;
    let operation_type_part =
        multipart::Part::bytes((OperationType::SendRgb as u8).to_le_bytes().to_vec());
    let psbt_part = multipart::Part::bytes(b"psbt".to_vec());
    let consignment_part_1 = multipart::Part::bytes(b"consignment_data".to_vec());
    let consignment_part_2 = multipart::Part::bytes(b"consignment_data".to_vec());
    let form = multipart::Form::new()
        .part("operation_type", operation_type_part)
        .part("seed", multipart::Part::text(TEST_OPERATION_SEED_HEX))
        .part("file_psbt", psbt_part)
        .part("file_consignment", consignment_part_1)
        .part("file_consignment", consignment_part_2);
    let operation_idx = post_operation_with_multipart_form(&ctx, form, 0)
        .await
        .operation_idx;
    assert_eq!(operation_idx, 1);
    let after_post = Utc::now().timestamp();
    let res = get_operation_by_idx(&ctx, operation_idx, Some(0))
        .await
        .unwrap();
    assert_eq!(res.operation_idx, operation_idx);
    assert_eq!(res.operation_type, OperationType::SendRgb);
    assert_eq!(res.initiator_xpub, "xpub0");
    assert_eq!(res.seed.as_deref(), Some(TEST_OPERATION_SEED_HEX));
    assert!(res.created_at >= before_post && res.created_at <= after_post);
    assert_eq!(res.status, OperationStatus::Pending);
    assert!(!res.files.is_empty());
    let psbt_files: Vec<_> = res
        .files
        .iter()
        .filter(|f| f.r#type == FileType::OperationPsbt)
        .collect();
    assert_eq!(psbt_files.len(), 1);
    let consignment_files: Vec<_> = res
        .files
        .iter()
        .filter(|f| f.r#type == FileType::Consignment)
        .collect();
    assert_eq!(consignment_files.len(), 2);
    let files_dir = Path::new(&app_dir).join(FILES_DIR);
    for file in &res.files {
        let file_path = files_dir.join(&file.file_id);
        assert!(file_path.exists());
        let metadata = tokio::fs::metadata(&file_path).await.unwrap();
        assert_eq!(metadata.len(), file.size_bytes);
    }
    assert_eq!(res.my_response, None);
    assert_eq!(res.acked_by.len(), 0);
    assert_eq!(res.nacked_by.len(), 0);
    assert_eq!(res.threshold, Some(3));
    assert!(res.processed_at.is_none());

    // without PSBT and auto-approved operation
    let app_dir = format!("{TEST_DIR_BASE}success_without_psbt");
    let ctx = setup_daemon(&app_dir).await;
    let operation_type_part =
        multipart::Part::bytes((OperationType::Issuance as u8).to_le_bytes().to_vec());
    let consignment_part = multipart::Part::bytes(b"consignment".to_vec());
    let form = multipart::Form::new()
        .part("operation_type", operation_type_part)
        .part("file_consignment", consignment_part);
    let operation_idx = post_operation_with_multipart_form(&ctx, form, 0)
        .await
        .operation_idx;
    assert_eq!(operation_idx, 1);
    let res = get_operation_by_idx(&ctx, operation_idx, Some(0))
        .await
        .unwrap();
    assert_eq!(res.operation_idx, operation_idx);
    assert_eq!(res.operation_type, OperationType::Issuance);
    assert_eq!(res.initiator_xpub, "xpub0");
    assert_eq!(res.seed, None);
    assert_eq!(res.status, OperationStatus::Approved);
    assert_eq!(res.files.len(), 1);
    assert_eq!(res.files[0].r#type, FileType::Consignment);
    assert!(res.threshold.is_none());
    assert_eq!(res.my_response, None);

    // test all operation types
    let test_cases = vec![
        (
            OperationType::CreateUtxos,
            "create_utxos",
            false,
            vec!["file_psbt"],
        ),
        (
            OperationType::Issuance,
            "issuance",
            true,
            vec!["file_consignment", "file_media"],
        ),
        (
            OperationType::SendRgb,
            "send_rgb",
            false,
            vec!["file_psbt", "file_consignment", "file_operation_data"],
        ),
        (OperationType::SendBtc, "send_btc", false, vec!["file_psbt"]),
        (
            OperationType::Inflation,
            "inflation",
            false,
            vec!["file_psbt", "file_consignment", "file_operation_data"],
        ),
        (
            OperationType::BlindReceive,
            "blind_receive",
            true,
            vec!["file_operation_data"],
        ),
        (
            OperationType::WitnessReceive,
            "witness_receive",
            true,
            vec!["file_operation_data"],
        ),
    ];
    for (op_type, test_name, is_auto_approved, file_types) in test_cases {
        let app_dir = format!("{TEST_DIR_BASE}all_types_{}", test_name);
        let ctx = setup_daemon(&app_dir).await;
        let operation_type_part = multipart::Part::bytes((op_type as u8).to_le_bytes().to_vec());
        let mut form = multipart::Form::new().part("operation_type", operation_type_part);
        if OPERATION_TYPES_THAT_REQUIRE_SEED.contains(&op_type) {
            form = form.part("seed", multipart::Part::text(TEST_OPERATION_SEED_HEX));
        }
        for file_type in &file_types {
            let file_part = multipart::Part::bytes(format!("{}_content", file_type).into_bytes());
            form = form.part(*file_type, file_part);
        }
        let operation_idx = post_operation_with_multipart_form(&ctx, form, 0)
            .await
            .operation_idx;
        assert_eq!(operation_idx, 1);
        let res = get_operation_by_idx(&ctx, operation_idx, Some(0))
            .await
            .unwrap();
        assert_eq!(res.operation_idx, operation_idx);
        assert_eq!(res.operation_type, op_type);
        assert_eq!(
            res.seed.as_deref(),
            if OPERATION_TYPES_THAT_REQUIRE_SEED.contains(&op_type) {
                Some(TEST_OPERATION_SEED_HEX)
            } else {
                None
            }
        );
        assert_eq!(
            res.status,
            if is_auto_approved {
                OperationStatus::Approved
            } else {
                OperationStatus::Pending
            }
        );
        assert_eq!(res.threshold, if is_auto_approved { None } else { Some(3) });
        assert_eq!(res.my_response, None);
        assert_eq!(res.files.len(), file_types.len());
        let res = get_operation_by_idx(&ctx, operation_idx, Some(0))
            .await
            .unwrap();
        assert_eq!(res.operation_type, op_type);
        assert!(res.files.len() >= file_types.len());
        assert_eq!(
            res.status,
            if is_auto_approved {
                OperationStatus::Approved
            } else {
                OperationStatus::Pending
            }
        );
        assert_eq!(res.threshold, if is_auto_approved { None } else { Some(3) });
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

    // multipart form checks
    multipart_form_checks(
        &ctx,
        MultipartFormChecks {
            api_info: api_info.clone(),
            field_name: s!("operation_type"),
        },
    )
    .await;

    // invalid operation type
    let operation_type_part = multipart::Part::bytes(100u8.to_le_bytes().to_vec());
    let form = multipart::Form::new().part("operation_type", operation_type_part);
    let res = reqwest::Client::new()
        .post(format!("http://{}/{}", ctx.node_address, PATH))
        .bearer_auth(ctx.get_cosigner_token(0))
        .multipart(form)
        .send()
        .await
        .unwrap();
    check_response_is_nok(
        res,
        reqwest::StatusCode::BAD_REQUEST,
        "Invalid operation type: 100",
        "InvalidOperationType",
    )
    .await;

    // invalid file type
    let operation_type_part =
        multipart::Part::bytes((OperationType::SendRgb as u8).to_le_bytes().to_vec());
    let file_part = multipart::Part::bytes(b"file".to_vec());
    let form = multipart::Form::new()
        .part("operation_type", operation_type_part)
        .part("file_invalid", file_part);
    let res = reqwest::Client::new()
        .post(format!("http://{}/{}", ctx.node_address, PATH))
        .bearer_auth(ctx.get_cosigner_token(0))
        .multipart(form)
        .send()
        .await
        .unwrap();
    check_response_is_nok(
        res,
        reqwest::StatusCode::BAD_REQUEST,
        "invalid file type 'file_invalid'",
        "InvalidRequest",
    )
    .await;

    // more than one PSBT provided
    let operation_type_part =
        multipart::Part::bytes((OperationType::SendRgb as u8).to_le_bytes().to_vec());
    let psbt_part_1 = multipart::Part::bytes(b"psbt".to_vec());
    let psbt_part_2 = multipart::Part::bytes(b"psbt".to_vec());
    let form = multipart::Form::new()
        .part("operation_type", operation_type_part)
        .part("file_psbt", psbt_part_1)
        .part("file_psbt", psbt_part_2);
    let res = reqwest::Client::new()
        .post(format!("http://{}/{}", ctx.node_address, PATH))
        .bearer_auth(ctx.get_cosigner_token(0))
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

    // no files nor PSBT provided
    let operation_type_part =
        multipart::Part::bytes((OperationType::SendRgb as u8).to_le_bytes().to_vec());
    let form = multipart::Form::new().part("operation_type", operation_type_part);
    let res = reqwest::Client::new()
        .post(format!("http://{}/{}", ctx.node_address, PATH))
        .bearer_auth(ctx.get_cosigner_token(0))
        .multipart(form)
        .send()
        .await
        .unwrap();
    check_response_is_nok(
        res,
        reqwest::StatusCode::BAD_REQUEST,
        "no files nor PSBT provided",
        "InvalidRequest",
    )
    .await;

    // operation type not provided
    let psbt_part = multipart::Part::bytes(b"psbt".to_vec());
    let form = multipart::Form::new().part("file_psbt", psbt_part);
    let res = reqwest::Client::new()
        .post(format!("http://{}/{}", ctx.node_address, PATH))
        .bearer_auth(ctx.get_cosigner_token(0))
        .multipart(form)
        .send()
        .await
        .unwrap();
    check_response_is_nok(
        res,
        reqwest::StatusCode::BAD_REQUEST,
        "operation type not provided",
        "InvalidRequest",
    )
    .await;

    // seed not provided
    let operation_type_part =
        multipart::Part::bytes((OperationType::SendRgb as u8).to_le_bytes().to_vec());
    let psbt_part = multipart::Part::bytes(b"psbt".to_vec());
    let form = multipart::Form::new()
        .part("operation_type", operation_type_part)
        .part("file_psbt", psbt_part);
    let res = reqwest::Client::new()
        .post(format!("http://{}/{}", ctx.node_address, PATH))
        .bearer_auth(ctx.get_cosigner_token(0))
        .multipart(form)
        .send()
        .await
        .unwrap();
    check_response_is_nok(
        res,
        reqwest::StatusCode::BAD_REQUEST,
        "seed not provided",
        "InvalidRequest",
    )
    .await;

    // invalid seed length
    let operation_type_part =
        multipart::Part::bytes((OperationType::SendRgb as u8).to_le_bytes().to_vec());
    let seed_part = multipart::Part::text("00".repeat(31));
    let psbt_part = multipart::Part::bytes(b"psbt".to_vec());
    let form = multipart::Form::new()
        .part("operation_type", operation_type_part)
        .part("seed", seed_part)
        .part("file_psbt", psbt_part);
    let res = reqwest::Client::new()
        .post(format!("http://{}/{}", ctx.node_address, PATH))
        .bearer_auth(ctx.get_cosigner_token(0))
        .multipart(form)
        .send()
        .await
        .unwrap();
    check_response_is_nok(
        res,
        reqwest::StatusCode::BAD_REQUEST,
        "seed must be 64 hex chars",
        "InvalidRequest",
    )
    .await;

    // invalid seed hex
    let operation_type_part =
        multipart::Part::bytes((OperationType::SendRgb as u8).to_le_bytes().to_vec());
    let seed_part = multipart::Part::text("zz".repeat(32));
    let psbt_part = multipart::Part::bytes(b"psbt".to_vec());
    let form = multipart::Form::new()
        .part("operation_type", operation_type_part)
        .part("seed", seed_part)
        .part("file_psbt", psbt_part);
    let res = reqwest::Client::new()
        .post(format!("http://{}/{}", ctx.node_address, PATH))
        .bearer_auth(ctx.get_cosigner_token(0))
        .multipart(form)
        .send()
        .await
        .unwrap();
    check_response_is_nok(
        res,
        reqwest::StatusCode::BAD_REQUEST,
        "seed must be valid hex",
        "InvalidRequest",
    )
    .await;

    // failed to read seed field
    let boundary = "BOUNDARY123";
    let mut body_bytes = Vec::new();
    body_bytes.extend_from_slice(
        format!(
            "--{boundary}\r\n\
             Content-Disposition: form-data; name=\"operation_type\"\r\n\
             \r\n"
        )
        .as_bytes(),
    );
    body_bytes.push(OperationType::SendRgb as u8);
    body_bytes.extend_from_slice(
        format!(
            "\r\n--{boundary}\r\n\
             Content-Disposition: form-data; name=\"seed\"\r\n\
             \r\n"
        )
        .as_bytes(),
    );
    let chunks = vec![
        Ok::<Bytes, std::io::Error>(Bytes::from(body_bytes)),
        Ok(Bytes::from_static(b"some bytes but not complete...")),
        // stream ends here => server hits EOF while reading seed text()
    ];
    let body = Body::wrap_stream(stream::iter(chunks));
    let res = reqwest::Client::new()
        .post(format!("http://{}/{}", ctx.node_address, PATH))
        .bearer_auth(ctx.get_cosigner_token(0))
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

    // seed provided for operation type that doesn't use it
    let operation_type_part =
        multipart::Part::bytes((OperationType::Issuance as u8).to_le_bytes().to_vec());
    let seed_part = multipart::Part::text(TEST_OPERATION_SEED_HEX);
    let consignment_part = multipart::Part::bytes(b"consignment".to_vec());
    let form = multipart::Form::new()
        .part("operation_type", operation_type_part)
        .part("seed", seed_part)
        .part("file_consignment", consignment_part);
    let res = reqwest::Client::new()
        .post(format!("http://{}/{}", ctx.node_address, PATH))
        .bearer_auth(ctx.get_cosigner_token(0))
        .multipart(form)
        .send()
        .await
        .unwrap();
    check_response_is_nok(
        res,
        reqwest::StatusCode::BAD_REQUEST,
        "seed not allowed for operation type",
        "InvalidRequest",
    )
    .await;

    // cannot post operation while there's already a pending one
    // - first operation succeeds
    let operation_type_part =
        multipart::Part::bytes((OperationType::SendRgb as u8).to_le_bytes().to_vec());
    let psbt_part = multipart::Part::bytes(b"psbt".to_vec());
    let form = multipart::Form::new()
        .part("operation_type", operation_type_part)
        .part("seed", multipart::Part::text(TEST_OPERATION_SEED_HEX))
        .part("file_psbt", psbt_part);
    let res = reqwest::Client::new()
        .post(format!("http://{}/{}", ctx.node_address, PATH))
        .bearer_auth(ctx.get_cosigner_token(0))
        .multipart(form)
        .send()
        .await
        .unwrap();
    let _ = check_response_is_ok(res).await;
    // - second operation while the first is still pending fails
    let operation_type_part =
        multipart::Part::bytes((OperationType::SendRgb as u8).to_le_bytes().to_vec());
    let psbt_part = multipart::Part::bytes(b"psbt".to_vec());
    let form = multipart::Form::new()
        .part("operation_type", operation_type_part)
        .part("seed", multipart::Part::text(TEST_OPERATION_SEED_HEX))
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
        "Cannot post new operation: another operation is still pending",
        "CannotPostNewOperation",
    )
    .await;

    // cannot post operation with unprocessed operations
    let app_dir_2 = format!("{TEST_DIR_BASE}fail_unprocessed");
    let (ctx, operation_idx) = setup_with_approved_operation(&app_dir_2).await;
    // - mark operation as processed by other cosigners but NOT by the initiator (cosigner 0)
    for cosigner_idx in 1..=3 {
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
        let _ = check_response_is_ok(res).await;
    }
    // - try to post a new operation by the same initiator (cosigner 0) who hasn't processed the previous one
    let operation_type_part =
        multipart::Part::bytes((OperationType::SendRgb as u8).to_le_bytes().to_vec());
    let psbt_part = multipart::Part::bytes(b"psbt".to_vec());
    let form = multipart::Form::new()
        .part("operation_type", operation_type_part)
        .part("seed", multipart::Part::text(TEST_OPERATION_SEED_HEX))
        .part("file_psbt", psbt_part);
    let res = reqwest::Client::new()
        .post(format!("http://{}/{}", ctx.node_address, PATH))
        .bearer_auth(ctx.get_cosigner_token(0))
        .multipart(form)
        .send()
        .await
        .unwrap();
    check_response_is_nok(
        res,
        reqwest::StatusCode::FORBIDDEN,
        "Cannot post new operation: initiator has unprocessed operations",
        "CannotPostNewOperation",
    )
    .await;
}
