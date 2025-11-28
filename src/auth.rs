use axum::{
    async_trait,
    body::Body,
    extract::{FromRequestParts, State},
    http::{Request, StatusCode, request::Parts},
    middleware::Next,
    response::Response,
};
use biscuit_auth::{Biscuit, PublicKey, macros::authorizer};
use std::sync::Arc;

use crate::{
    error::{AppError, AuthError},
    startup::AppState,
    utils::hex_str_to_vec,
};

const WATCH_ONLY_ALLOWED_ROUTES: &[&str] = &[
    "/transferstatus",
    "/info",
    "/getoperationbyidx",
    "/getcurrentaddressindices",
    "/getfile",
];

fn is_watch_only_allowed(path: &str) -> bool {
    WATCH_ONLY_ALLOWED_ROUTES.contains(&path)
}

fn is_token_expired(token: &Biscuit) -> bool {
    authorizer!(r#"allow if true;"#)
        .time()
        .build(token)
        .and_then(|mut authorizer| authorizer.authorize())
        .is_err()
}

#[derive(Debug, Clone)]
pub(crate) struct AuthenticatedCosigner {
    pub(crate) xpub: String,
    pub(crate) idx: i32,
}

#[derive(Debug, Clone)]
pub(crate) enum AuthenticatedUser {
    Cosigner(AuthenticatedCosigner),
    WatchOnly,
}

#[async_trait]
impl<S> FromRequestParts<S> for AuthenticatedUser
where
    S: Send + Sync,
{
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<AuthenticatedUser>()
            .cloned()
            .ok_or(StatusCode::UNAUTHORIZED)
    }
}

#[async_trait]
impl<S> FromRequestParts<S> for AuthenticatedCosigner
where
    S: Send + Sync,
{
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        match parts.extensions.get::<AuthenticatedUser>() {
            Some(AuthenticatedUser::Cosigner(cosigner)) => Ok(cosigner.clone()),
            _ => Err(StatusCode::FORBIDDEN),
        }
    }
}

pub(crate) fn check_auth_args(root_public_key: &str) -> Result<PublicKey, AppError> {
    let key_bytes = hex_str_to_vec(root_public_key).ok_or(AppError::InvalidRootKey)?;
    if key_bytes.len() != 32 {
        return Err(AppError::InvalidRootKey);
    }
    let mut key_array = [0u8; 32];
    key_array.copy_from_slice(&key_bytes);
    PublicKey::from_bytes(&key_array, biscuit_auth::Algorithm::Ed25519)
        .map_err(|_| AppError::InvalidRootKey)
}

pub(crate) async fn conditional_auth_middleware(
    State(app_state): State<Arc<AppState>>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, AuthError> {
    let auth_header = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));
    let auth_token = match auth_header {
        Some(token) => token,
        None => return Err(AuthError::Unauthorized),
    };

    // verify the token
    let token = Biscuit::from_base64(auth_token, app_state.root_public_key)
        .map_err(|_| AuthError::Unauthorized)?;

    // check if the token is expired
    if is_token_expired(&token) {
        return Err(AuthError::Unauthorized);
    }

    // determine the authenticated user based on the role and the xPub
    let mut authorizer = token.authorizer().map_err(|_| AuthError::Unauthorized)?;
    let role = authorizer
        .query("data($r) <- role($r)")
        .ok()
        .and_then(|v: Vec<(String,)>| v.first().map(|r| r.0.clone()))
        .ok_or(AuthError::Unauthorized)?;
    let xpub = authorizer
        .query("data($x) <- xpub($x)")
        .ok()
        .and_then(|v: Vec<(String,)>| v.first().map(|x| x.0.clone()));
    let user = match (role.as_str(), xpub) {
        ("cosigner", Some(xpub)) => {
            let idx = app_state
                .cosigners_by_xpub
                .get(&xpub)
                .ok_or(AuthError::Unauthorized)?;
            AuthenticatedUser::Cosigner(AuthenticatedCosigner { xpub, idx: *idx })
        }
        ("watch-only", None) => AuthenticatedUser::WatchOnly,
        _ => return Err(AuthError::Unauthorized),
    };

    let api_path = request.uri().path();
    match &user {
        AuthenticatedUser::Cosigner(cosigner) => {
            tracing::info!(
                "authenticated cosigner {} (xpub {}) for path {}",
                cosigner.idx,
                cosigner.xpub,
                api_path
            );
        }
        AuthenticatedUser::WatchOnly => {
            tracing::info!("authenticated watch-only user for path {}", api_path);
        }
    }

    // if the user is a watch-only user, check if the path is allowed
    if matches!(user, AuthenticatedUser::WatchOnly) && !is_watch_only_allowed(api_path) {
        tracing::warn!(
            "watch-only user attempted to access forbidden path {}",
            api_path
        );
        return Err(AuthError::Forbidden);
    }

    // insert the authenticated user into the request extensions
    let mut request = request;
    request.extensions_mut().insert(user);

    Ok(next.run(request).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_auth_args() {
        // success
        let root_public_key = "0606bc5f1e32cb636c96911fc3e97174609d51ee5304a319610f451e8b1112ca";
        let public_key = check_auth_args(root_public_key).unwrap();
        assert_eq!(public_key.to_bytes_hex(), root_public_key);

        // fail: not a valid hex string
        let root_public_key = "invalid";
        let result = check_auth_args(root_public_key);
        assert!(matches!(result.unwrap_err(), AppError::InvalidRootKey));

        // fail: not a valid public key
        let root_public_key = "0606bc5f1e32cb636c96911fc3e97174609d51ee5304a319610f451e8b1112";
        let result = check_auth_args(root_public_key);
        assert!(matches!(result.unwrap_err(), AppError::InvalidRootKey));
    }
}
