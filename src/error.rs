use amplify::s;
use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct APIErrorResponse {
    pub(crate) error: String,
    pub(crate) code: u16,
    pub(crate) name: String,
}

/// The error variants returned by APIs
#[derive(Debug, thiserror::Error)]
pub enum APIError {
    #[error("Cannot mark operation as processed: {0}")]
    CannotMarkOperationProcessed(String),

    #[error("Cannot post new operation: {0}")]
    CannotPostNewOperation(String),

    #[error("Cannot respond to operation: {0}")]
    CannotRespondToOperation(String),

    #[error("Database error: {0}")]
    Database(#[from] sea_orm::DbErr),

    #[error("File not found")]
    FileNotFound,

    #[error("Forbidden")]
    Forbidden,

    #[error("Invalid count: must be greater than 0")]
    InvalidCount,

    #[error("Invalid operation type: {0}")]
    InvalidOperationType(u8),

    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    #[error("IO error: {0}")]
    IO(#[from] std::io::Error),

    #[error("Operation not found")]
    OperationNotFound,

    #[error("Transfer status already set to a different value")]
    TransferStatusMismatch,

    #[error("Unexpected error: {0}")]
    Unexpected(String),
}

impl APIError {
    fn name(&self) -> String {
        format!("{self:?}")
            .split('(')
            .next()
            .unwrap()
            .split(" {")
            .next()
            .unwrap()
            .to_string()
    }
}

impl From<axum::extract::rejection::JsonRejection> for APIError {
    fn from(err: axum::extract::rejection::JsonRejection) -> Self {
        APIError::InvalidRequest(err.to_string())
    }
}

impl From<axum::extract::multipart::MultipartRejection> for APIError {
    fn from(err: axum::extract::multipart::MultipartRejection) -> Self {
        APIError::InvalidRequest(err.to_string())
    }
}

impl IntoResponse for APIError {
    fn into_response(self) -> Response {
        let (status, error, name) = match self {
            APIError::Database(_) | APIError::IO(_) | APIError::Unexpected(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                self.to_string(),
                self.name(),
            ),
            APIError::FileNotFound
            | APIError::InvalidCount
            | APIError::InvalidOperationType(_)
            | APIError::InvalidRequest(_)
            | APIError::OperationNotFound
            | APIError::TransferStatusMismatch => {
                (StatusCode::BAD_REQUEST, self.to_string(), self.name())
            }
            APIError::CannotMarkOperationProcessed(_)
            | APIError::CannotPostNewOperation(_)
            | APIError::CannotRespondToOperation(_)
            | APIError::Forbidden => (StatusCode::FORBIDDEN, self.to_string(), self.name()),
        };

        let error = error.replace("\n", " ");

        tracing::error!("APIError: {error}");

        let body = Json(
            serde_json::to_value(APIErrorResponse {
                error,
                code: status.as_u16(),
                name,
            })
            .unwrap(),
        );

        (status, body).into_response()
    }
}

/// The error variants returned during app startup
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Cannot change cosigners")]
    CannotChangeCosigners,

    #[error("Config error: {0}")]
    Config(#[from] confy::ConfyError),

    #[error("DB error: {0}")]
    Database(#[from] migration::DbErr),

    #[error("Inconsistent state: {0}")]
    InconsistentState(String),

    #[error("Invalid cosigner number: {0}")]
    InvalidCosignerNumber(usize),

    #[error("Invalid rgb-lib version: {0}")]
    InvalidRgbLibVersion(String),

    #[error("The provided root public key is invalid")]
    InvalidRootKey,

    #[error("Invalid threshold: {0}")]
    InvalidThreshold(String),

    #[error("IO error: {0}")]
    IO(#[from] std::io::Error),

    #[error("Configuration file is missing, expected in '{0}'")]
    MissingConfigFile(String),

    #[error("Port {0} is unavailable")]
    UnavailablePort(u16),
}

/// The error variants returned by the authentication checks
#[derive(Debug)]
pub enum AuthError {
    Unauthorized,
    Forbidden,
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        match self {
            AuthError::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                Json(APIErrorResponse {
                    code: StatusCode::UNAUTHORIZED.as_u16(),
                    error: s!("Missing or invalid credentials"),
                    name: s!("Unauthorized"),
                }),
            )
                .into_response(),
            AuthError::Forbidden => (
                StatusCode::FORBIDDEN,
                Json(APIErrorResponse {
                    code: StatusCode::FORBIDDEN.as_u16(),
                    error: s!("You don't have access to this resource"),
                    name: s!("Forbidden"),
                }),
            )
                .into_response(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    async fn extract_response_body(response: Response) -> (StatusCode, APIErrorResponse) {
        let status = response.status();
        let body = response.into_body();
        let bytes = to_bytes(body, usize::MAX).await.unwrap();
        let error_response: APIErrorResponse = serde_json::from_slice(&bytes).unwrap();
        (status, error_response)
    }

    #[tokio::test]
    async fn test_api_error_into_response_internal_server_error() {
        // Database error
        let db_err = APIError::Database(sea_orm::DbErr::Custom(s!("db error")));
        let response = db_err.into_response();
        let (status, body) = extract_response_body(response).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body.code, 500);
        assert_eq!(body.name, "Database");
        assert!(body.error.contains("db error"));

        // IO error
        let io_err = APIError::IO(std::io::Error::other("io error"));
        let response = io_err.into_response();
        let (status, body) = extract_response_body(response).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body.code, 500);
        assert_eq!(body.name, "IO");
        assert!(body.error.contains("io error"));

        // Unexpected error
        let unexpected_err = APIError::Unexpected(s!("unexpected error"));
        let response = unexpected_err.into_response();
        let (status, body) = extract_response_body(response).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body.code, 500);
        assert_eq!(body.name, "Unexpected");
        assert!(body.error.contains("unexpected error"));
    }

    #[tokio::test]
    async fn test_api_error_into_response_bad_request() {
        // FileNotFound
        let err = APIError::FileNotFound;
        let response = err.into_response();
        let (status, body) = extract_response_body(response).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body.code, 400);
        assert_eq!(body.name, "FileNotFound");
        assert_eq!(body.error, "File not found");

        // InvalidCount
        let err = APIError::InvalidCount;
        let response = err.into_response();
        let (status, body) = extract_response_body(response).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body.code, 400);
        assert_eq!(body.name, "InvalidCount");
        assert_eq!(body.error, "Invalid count: must be greater than 0");

        // InvalidOperationType
        let err = APIError::InvalidOperationType(99);
        let response = err.into_response();
        let (status, body) = extract_response_body(response).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body.code, 400);
        assert_eq!(body.name, "InvalidOperationType");
        assert_eq!(body.error, "Invalid operation type: 99");

        // InvalidRequest
        let err = APIError::InvalidRequest(s!("invalid json"));
        let response = err.into_response();
        let (status, body) = extract_response_body(response).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body.code, 400);
        assert_eq!(body.name, "InvalidRequest");
        assert!(body.error.contains("invalid json"));

        // OperationNotFound
        let err = APIError::OperationNotFound;
        let response = err.into_response();
        let (status, body) = extract_response_body(response).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body.code, 400);
        assert_eq!(body.name, "OperationNotFound");
        assert_eq!(body.error, "Operation not found");
    }

    #[tokio::test]
    async fn test_api_error_into_response_forbidden() {
        // CannotMarkOperationProcessed
        let err = APIError::CannotMarkOperationProcessed(s!("not allowed"));
        let response = err.into_response();
        let (status, body) = extract_response_body(response).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body.code, 403);
        assert_eq!(body.name, "CannotMarkOperationProcessed");
        assert!(body.error.contains("not allowed"));

        // CannotPostNewOperation
        let err = APIError::CannotPostNewOperation(s!("pending operation"));
        let response = err.into_response();
        let (status, body) = extract_response_body(response).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body.code, 403);
        assert_eq!(body.name, "CannotPostNewOperation");
        assert!(body.error.contains("pending operation"));

        // CannotRespondToOperation
        let err = APIError::CannotRespondToOperation(s!("already responded"));
        let response = err.into_response();
        let (status, body) = extract_response_body(response).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body.code, 403);
        assert_eq!(body.name, "CannotRespondToOperation");
        assert!(body.error.contains("already responded"));
    }

    #[tokio::test]
    async fn test_api_error_newline_replacement() {
        // Test that newlines in error messages are replaced with spaces
        let err = APIError::InvalidRequest(s!("error with\nnewline\ncharacters"));
        let response = err.into_response();
        let (_, body) = extract_response_body(response).await;
        assert!(!body.error.contains('\n'));
        assert!(body.error.contains("error with newline characters"));
    }

    #[test]
    fn test_api_error_name() {
        assert_eq!(APIError::FileNotFound.name(), "FileNotFound");
        assert_eq!(APIError::InvalidCount.name(), "InvalidCount");
        assert_eq!(
            APIError::InvalidOperationType(1).name(),
            "InvalidOperationType"
        );
        assert_eq!(
            APIError::InvalidRequest(s!("test")).name(),
            "InvalidRequest"
        );
        assert_eq!(APIError::OperationNotFound.name(), "OperationNotFound");
        assert_eq!(
            APIError::CannotMarkOperationProcessed(s!("test")).name(),
            "CannotMarkOperationProcessed"
        );
        assert_eq!(
            APIError::CannotPostNewOperation(s!("test")).name(),
            "CannotPostNewOperation"
        );
        assert_eq!(
            APIError::CannotRespondToOperation(s!("test")).name(),
            "CannotRespondToOperation"
        );
        assert_eq!(APIError::Unexpected(s!("test")).name(), "Unexpected");
    }
}
