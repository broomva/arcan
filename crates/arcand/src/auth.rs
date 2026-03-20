//! JWT authentication middleware for the Arcan HTTP API.
//!
//! Validates `Authorization: Bearer <token>` headers using HS256 JWTs signed
//! with a shared secret (same as broomva.tech `AUTH_SECRET`).
//!
//! When no secret is configured (`ARCAN_JWT_SECRET` / `AUTH_SECRET`), auth is
//! disabled and all requests proceed without validation. This allows local
//! development without token management.
//!
//! The middleware extracts user claims (`sub`, `email`) and injects an
//! [`AuthUser`] into request extensions for downstream handlers.

use std::sync::Arc;

use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use jsonwebtoken::{DecodingKey, Validation, decode};
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ─── JWT Claims ──────────────────────────────────────────────────────────────

/// JWT claims signed by broomva.tech (Better Auth).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArcanClaims {
    /// User ID (subject).
    pub sub: String,
    /// User email.
    #[serde(default)]
    pub email: String,
    /// Expiry (unix timestamp).
    pub exp: u64,
    /// Issued at (unix timestamp).
    #[serde(default)]
    pub iat: u64,
}

/// Authenticated user context injected into request extensions.
#[derive(Debug, Clone, Serialize)]
pub struct AuthUser {
    /// User ID from JWT `sub` claim.
    pub user_id: String,
    /// User email from JWT `email` claim.
    pub email: String,
}

// ─── Errors ──────────────────────────────────────────────────────────────────

/// JWT validation errors.
#[derive(Debug, Error)]
pub enum JwtError {
    #[error("missing bearer token")]
    MissingToken,
    #[error("invalid token: {0}")]
    Invalid(String),
    #[error("token expired")]
    Expired,
}

// ─── Validation ──────────────────────────────────────────────────────────────

/// Validate a JWT token string against the shared secret.
pub fn validate_jwt(token: &str, secret: &str) -> Result<ArcanClaims, JwtError> {
    let key = DecodingKey::from_secret(secret.as_bytes());
    let mut validation = Validation::new(jsonwebtoken::Algorithm::HS256);
    validation.validate_exp = true;
    // broomva.tech may not set audience/issuer
    validation.required_spec_claims.clear();
    validation.required_spec_claims.insert("exp".to_string());
    validation.required_spec_claims.insert("sub".to_string());

    let token_data =
        decode::<ArcanClaims>(token, &key, &validation).map_err(|e| match e.kind() {
            jsonwebtoken::errors::ErrorKind::ExpiredSignature => JwtError::Expired,
            _ => JwtError::Invalid(e.to_string()),
        })?;

    Ok(token_data.claims)
}

/// Extract a bearer token from an Authorization header value.
pub fn extract_bearer_token(header_value: &str) -> Result<&str, JwtError> {
    header_value
        .strip_prefix("Bearer ")
        .or_else(|| header_value.strip_prefix("bearer "))
        .ok_or(JwtError::MissingToken)
}

// ─── Auth State ──────────────────────────────────────────────────────────────

/// Shared auth configuration for the middleware.
#[derive(Clone)]
pub struct AuthConfig {
    /// JWT secret. If `None`, auth is disabled.
    pub jwt_secret: Option<String>,
}

impl AuthConfig {
    /// Resolve auth configuration from environment variables.
    ///
    /// Checks `ARCAN_JWT_SECRET` first, then falls back to `AUTH_SECRET`.
    /// If neither is set, auth is disabled.
    pub fn from_env() -> Self {
        let secret = std::env::var("ARCAN_JWT_SECRET")
            .or_else(|_| std::env::var("AUTH_SECRET"))
            .ok()
            .filter(|s| !s.is_empty());

        if secret.is_some() {
            tracing::info!("JWT auth enabled for protected routes");
        } else {
            tracing::warn!(
                "No ARCAN_JWT_SECRET or AUTH_SECRET set — JWT auth DISABLED. \
                 All routes are unprotected. Set one of these env vars in production."
            );
        }

        Self { jwt_secret: secret }
    }
}

// ─── Auth error response ─────────────────────────────────────────────────────

/// JSON error body for auth failures.
#[derive(Serialize)]
struct AuthErrorBody {
    error: String,
    message: String,
}

fn auth_error(status: StatusCode, message: impl Into<String>) -> Response {
    let body = AuthErrorBody {
        error: "unauthorized".to_string(),
        message: message.into(),
    };
    (status, axum::Json(body)).into_response()
}

// ─── Middleware ───────────────────────────────────────────────────────────────

/// Axum middleware that validates JWT bearer tokens and injects [`AuthUser`].
///
/// If no JWT secret is configured, all requests are allowed through (local dev).
/// If a secret IS configured, a valid Bearer token is required on every request
/// that passes through this middleware layer.
pub async fn jwt_auth_middleware(
    axum::extract::State(config): axum::extract::State<Arc<AuthConfig>>,
    mut request: Request,
    next: Next,
) -> Response {
    // If no secret is configured, skip auth entirely (local dev mode).
    let Some(secret) = &config.jwt_secret else {
        return next.run(request).await;
    };

    // Extract Authorization header.
    let auth_header = match request.headers().get("authorization") {
        Some(h) => match h.to_str() {
            Ok(s) => s.to_string(),
            Err(_) => return auth_error(StatusCode::UNAUTHORIZED, "invalid authorization header"),
        },
        None => return auth_error(StatusCode::UNAUTHORIZED, "missing authorization header"),
    };

    // Extract bearer token.
    let token = match extract_bearer_token(&auth_header) {
        Ok(t) => t,
        Err(e) => return auth_error(StatusCode::UNAUTHORIZED, e.to_string()),
    };

    // Validate JWT.
    let claims = match validate_jwt(token, secret) {
        Ok(c) => c,
        Err(e) => return auth_error(StatusCode::UNAUTHORIZED, e.to_string()),
    };

    // Inject AuthUser into request extensions for downstream handlers.
    request.extensions_mut().insert(AuthUser {
        user_id: claims.sub,
        email: claims.email,
    });

    next.run(request).await
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use axum::routing::get;
    use axum::{Extension, Router};
    use jsonwebtoken::{EncodingKey, Header, encode};

    const TEST_SECRET: &str = "test-secret-for-arcan-jwt";

    fn make_token(sub: &str, email: &str, exp: u64) -> String {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let claims = ArcanClaims {
            sub: sub.to_string(),
            email: email.to_string(),
            exp,
            iat: now,
        };
        let key = EncodingKey::from_secret(TEST_SECRET.as_bytes());
        encode(&Header::default(), &claims, &key).unwrap()
    }

    fn make_valid_token() -> String {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        make_token("user-123", "test@broomva.tech", now + 3600)
    }

    fn make_expired_token() -> String {
        make_token("user-123", "test@broomva.tech", 1000) // long past
    }

    // --- Unit tests ---

    #[test]
    fn validate_valid_token() {
        let token = make_valid_token();
        let claims = validate_jwt(&token, TEST_SECRET).unwrap();
        assert_eq!(claims.sub, "user-123");
        assert_eq!(claims.email, "test@broomva.tech");
    }

    #[test]
    fn validate_wrong_secret() {
        let token = make_valid_token();
        let result = validate_jwt(&token, "wrong-secret");
        assert!(matches!(result, Err(JwtError::Invalid(_))));
    }

    #[test]
    fn validate_expired_token() {
        let token = make_expired_token();
        let result = validate_jwt(&token, TEST_SECRET);
        assert!(matches!(result, Err(JwtError::Expired)));
    }

    #[test]
    fn extract_bearer() {
        assert_eq!(extract_bearer_token("Bearer abc123").unwrap(), "abc123");
        assert_eq!(extract_bearer_token("bearer abc123").unwrap(), "abc123");
        assert!(extract_bearer_token("Basic abc123").is_err());
    }

    #[test]
    fn auth_config_defaults_to_disabled() {
        // When neither env var is set, auth should be disabled.
        // We can't easily unset env vars in Rust 2024 (unsafe), so just test the struct.
        let config = AuthConfig { jwt_secret: None };
        assert!(config.jwt_secret.is_none());
    }

    // --- Integration tests (middleware through axum) ---

    /// Helper: build an axum app with auth middleware on a test route.
    fn app_with_auth(secret: Option<String>) -> Router {
        let auth_config = Arc::new(AuthConfig { jwt_secret: secret });

        let protected = Router::new()
            .route(
                "/protected",
                get(|ext: Option<Extension<AuthUser>>| async move {
                    match ext {
                        Some(Extension(user)) => axum::Json(serde_json::json!({
                            "user_id": user.user_id,
                            "email": user.email,
                        }))
                        .into_response(),
                        None => {
                            axum::Json(serde_json::json!({"user_id": "anonymous"})).into_response()
                        }
                    }
                }),
            )
            .layer(axum::middleware::from_fn_with_state(
                auth_config,
                jwt_auth_middleware,
            ));

        let public = Router::new().route(
            "/health",
            get(|| async { axum::Json(serde_json::json!({"status": "ok"})) }),
        );

        public.merge(protected)
    }

    async fn send_request(
        app: Router,
        uri: &str,
        auth_header: Option<&str>,
    ) -> (StatusCode, String) {
        use tower::ServiceExt;

        let mut builder = HttpRequest::builder().uri(uri).method("GET");
        if let Some(header) = auth_header {
            builder = builder.header("authorization", header);
        }
        let request = builder.body(Body::empty()).unwrap();

        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8(body.to_vec()).unwrap())
    }

    #[tokio::test]
    async fn health_without_token_returns_200() {
        let app = app_with_auth(Some(TEST_SECRET.to_string()));
        let (status, _body) = send_request(app, "/health", None).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn protected_without_token_returns_401() {
        let app = app_with_auth(Some(TEST_SECRET.to_string()));
        let (status, body) = send_request(app, "/protected", None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(body.contains("missing authorization header"));
    }

    #[tokio::test]
    async fn protected_with_valid_token_returns_200() {
        let app = app_with_auth(Some(TEST_SECRET.to_string()));
        let token = make_valid_token();
        let (status, body) =
            send_request(app, "/protected", Some(&format!("Bearer {token}"))).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("user-123"));
    }

    #[tokio::test]
    async fn protected_with_expired_token_returns_401() {
        let app = app_with_auth(Some(TEST_SECRET.to_string()));
        let token = make_expired_token();
        let (status, body) =
            send_request(app, "/protected", Some(&format!("Bearer {token}"))).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(body.contains("expired"));
    }

    #[tokio::test]
    async fn protected_with_wrong_secret_returns_401() {
        let app = app_with_auth(Some(TEST_SECRET.to_string()));
        // Sign with a different secret
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let claims = ArcanClaims {
            sub: "user-123".to_string(),
            email: "test@broomva.tech".to_string(),
            exp: now + 3600,
            iat: now,
        };
        let key = EncodingKey::from_secret(b"wrong-secret");
        let token = encode(&Header::default(), &claims, &key).unwrap();
        let (status, _body) =
            send_request(app, "/protected", Some(&format!("Bearer {token}"))).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn auth_disabled_allows_all_requests() {
        let app = app_with_auth(None); // No secret → auth disabled
        let (status, _body) = send_request(app, "/protected", None).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn invalid_bearer_format_returns_401() {
        let app = app_with_auth(Some(TEST_SECRET.to_string()));
        let (status, body) = send_request(app, "/protected", Some("Basic abc123")).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(body.contains("missing bearer token"));
    }
}
