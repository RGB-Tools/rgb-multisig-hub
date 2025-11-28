mod auth;
mod database;
mod error;
mod routes;
mod startup;
mod utils;

#[cfg(test)]
mod test;

use anyhow::Result;
use axum::{
    Router,
    extract::DefaultBodyLimit,
    http::Request,
    middleware,
    response::Response,
    routing::{get, post},
};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::signal;
use tower_http::trace::TraceLayer;
use tower_http::{cors::CorsLayer, limit::RequestBodyLimitLayer};
use tracing::Span;
use tracing_subscriber::{
    filter,
    fmt::{
        FormatFields,
        format::{DefaultFields, Writer},
    },
    prelude::*,
};

use crate::{
    auth::conditional_auth_middleware,
    error::AppError,
    routes::{
        bump_address_indices, get_current_address_indices, get_file, get_last_processed_op_idx,
        get_operation_by_idx, info, mark_operation_processed, post_operation, respond_to_operation,
        transfer_status,
    },
    startup::{AppParams, AppState, LOGS_DIR, parse_startup_args_and_config, start_daemon},
};

#[tokio::main]
async fn main() -> Result<()> {
    let app_params = parse_startup_args_and_config()?;

    // stdout logger
    let stdout_log = tracing_subscriber::fmt::layer().fmt_fields(TypedFields::default());

    // file logger
    let log_dir = app_params.app_dir.join(LOGS_DIR);
    let file_appender = tracing_appender::rolling::daily(&log_dir, "rgb-multisig-hub.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    let file_log = tracing_subscriber::fmt::layer()
        .with_file(true)
        .with_line_number(true)
        .with_target(true)
        .with_thread_ids(true)
        .with_thread_names(true)
        .with_writer(non_blocking);

    tracing_subscriber::registry()
        .with(stdout_log.with_filter(filter::LevelFilter::INFO))
        .with(file_log.with_filter(filter::LevelFilter::DEBUG))
        .init();

    let addr = SocketAddr::from(([0, 0, 0, 0], app_params.daemon_listening_port));

    let (router, app_state) = app(app_params).await?;

    tracing::info!("Listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal(app_state))
        .await
        .unwrap();

    Ok(())
}

pub(crate) async fn app(app_params: AppParams) -> Result<(Router, Arc<AppState>), AppError> {
    let app_state = start_daemon(&app_params).await?;

    let router = Router::new()
        .route(
            "/postoperation",
            post(post_operation).layer(RequestBodyLimitLayer::new(100 * 1024 * 1024)),
        )
        // all routes before this will have the default body limit disabled
        .layer(DefaultBodyLimit::disable())
        .route("/bumpaddressindices", post(bump_address_indices))
        .route("/transferstatus", post(transfer_status))
        .route(
            "/getcurrentaddressindices",
            get(get_current_address_indices),
        )
        .route("/getfile", post(get_file))
        .route("/getlastprocessedopidx", get(get_last_processed_op_idx))
        .route("/getoperationbyidx", post(get_operation_by_idx))
        .route("/info", get(info))
        .route("/markoperationprocessed", post(mark_operation_processed))
        .route("/respondtooperation", post(respond_to_operation))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &Request<_>| {
                    tracing::info_span!(
                        "request",
                        status_code = tracing::field::Empty,
                        uri = tracing::field::display(request.uri()),
                        request_id = tracing::field::display(uuid::Uuid::new_v4()),
                    )
                })
                .on_request(|_request: &Request<_>, _span: &Span| {
                    tracing::info!("STARTED");
                })
                .on_response(|response: &Response, latency: Duration, span: &Span| {
                    span.record("status_code", tracing::field::display(response.status()));
                    tracing::info!("ENDED in {:?}", latency);
                }),
        )
        .layer(middleware::from_fn_with_state(
            app_state.clone(),
            conditional_auth_middleware,
        ))
        .layer(CorsLayer::permissive())
        .with_state(app_state.clone());

    Ok((router, app_state))
}

/// Tokio signal handler that will wait for a user to press CTRL+C.
async fn shutdown_signal(app_state: Arc<AppState>) {
    let cancel_token = app_state.cancel_token.clone();

    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
        _ = cancel_token.cancelled() => {},
    }

    tracing::info!("Received a shutdown signal");
}

// workaround for https://github.com/tokio-rs/tracing/issues/1372
#[derive(Default)]
struct TypedFields(DefaultFields);

impl<'writer> FormatFields<'writer> for TypedFields {
    fn format_fields<R: tracing_subscriber::field::RecordFields>(
        &self,
        writer: Writer<'writer>,
        fields: R,
    ) -> std::fmt::Result {
        self.0.format_fields(writer, fields)
    }
}
