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

// ─── Identity tier types ─────────────────────────────────────────────────────

/// Subscription / capability tier for a session.
///
/// Determines the `PolicySet` that arcand enforces for this session
/// (BRO-221). The tier is embedded in the Anima identity token and
/// verified server-side — the client cannot forge a higher tier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    /// Unauthenticated / guest — no persistent memory, read-only shell.
    Anonymous,
    /// Authenticated free plan — sandboxed shell, 7-day Lago retention.
    Free,
    /// Paid pro plan — full wildcard capability grant, 90-day retention.
    Pro,
    /// Enterprise — dedicated Life instance, RBAC, custom skill registry.
    Enterprise,
}

/// Role within an enterprise tenant.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TenantRole {
    Admin,
    Member,
    Viewer,
    Agent,
}

/// Identity claims embedded in the Anima identity token (BRO-221).
///
/// These claims are issued by broomva.tech (Anima layer) and verified
/// by arcand. The `PolicySet` for the session is derived entirely from
/// these claims — the client-supplied policy in `CreateSessionRequest`
/// is treated as a hint only when no identity token is present.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityClaims {
    /// Subject (user ID or anonymous ID).
    pub sub: String,
    /// Capability tier for this session.
    pub tier: Tier,
    /// Enterprise organization ID (present for Enterprise tier only).
    #[serde(default)]
    pub org_id: Option<String>,
    /// Tenant RBAC roles within the organization.
    #[serde(default)]
    pub roles: Vec<TenantRole>,
    /// Enterprise capability overrides — replaces the default wildcard
    /// when non-empty, enabling restricted enterprise policies.
    #[serde(default)]
    pub custom_capabilities: Option<Vec<String>>,
    /// Issued-at timestamp (unix seconds).
    #[serde(default)]
    pub iat: u64,
    /// Expiry timestamp (unix seconds).
    pub exp: u64,
}

/// Validate a short-lived Anima identity token.
///
/// Uses HS256 with `secret`. Returns `Err(JwtError::Expired)` if the token
/// is expired, or `Err(JwtError::Invalid(...))` for any other validation
/// failure.
pub fn validate_identity_token(token: &str, secret: &str) -> Result<IdentityClaims, JwtError> {
    let key = DecodingKey::from_secret(secret.as_bytes());
    let mut validation = Validation::new(jsonwebtoken::Algorithm::HS256);
    validation.validate_exp = true;
    validation.required_spec_claims.clear();
    validation.required_spec_claims.insert("exp".to_string());
    validation.required_spec_claims.insert("sub".to_string());

    let token_data =
        decode::<IdentityClaims>(token, &key, &validation).map_err(|e| match e.kind() {
            jsonwebtoken::errors::ErrorKind::ExpiredSignature => JwtError::Expired,
            _ => JwtError::Invalid(e.to_string()),
        })?;

    let claims = token_data.claims;

    // Cross-field invariants that the JWT library cannot enforce:
    //   • Enterprise tier requires `org_id` (tenant is mandatory for RBAC routing).
    //   • Non-enterprise tiers must not carry enterprise-only claims — a forged
    //     token that embeds org/role/capability fields while claiming a lower tier
    //     would otherwise smuggle escalation data past the policy gate.
    match &claims.tier {
        Tier::Enterprise => {
            if claims.org_id.is_none() {
                return Err(JwtError::Invalid(
                    "enterprise tier requires org_id".to_string(),
                ));
            }
        }
        _ => {
            if claims.org_id.is_some() {
                return Err(JwtError::Invalid(
                    "org_id is only valid for enterprise tier".to_string(),
                ));
            }
            if !claims.roles.is_empty() {
                return Err(JwtError::Invalid(
                    "roles are only valid for enterprise tier".to_string(),
                ));
            }
            if claims.custom_capabilities.is_some() {
                return Err(JwtError::Invalid(
                    "custom_capabilities are only valid for enterprise tier".to_string(),
                ));
            }
        }
    }

    Ok(claims)
}

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

    // ─── Identity token tests ─────────────────────────────────────────────────

    fn make_identity_token(sub: &str, tier: Tier, exp: u64) -> String {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let claims = IdentityClaims {
            sub: sub.to_string(),
            tier,
            org_id: None,
            roles: vec![],
            custom_capabilities: None,
            iat: now,
            exp,
        };
        let key = EncodingKey::from_secret(TEST_SECRET.as_bytes());
        encode(&Header::default(), &claims, &key).unwrap()
    }

    fn future_exp() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600
    }

    #[test]
    fn identity_token_valid_free_tier() {
        let token = make_identity_token("user-abc", Tier::Free, future_exp());
        let claims = validate_identity_token(&token, TEST_SECRET).unwrap();
        assert_eq!(claims.sub, "user-abc");
        assert_eq!(claims.tier, Tier::Free);
    }

    #[test]
    fn identity_token_valid_enterprise_tier() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let claims_in = IdentityClaims {
            sub: "org-user-1".to_string(),
            tier: Tier::Enterprise,
            org_id: Some("org-acme".to_string()),
            roles: vec![TenantRole::Admin],
            custom_capabilities: None,
            iat: now,
            exp: now + 3600,
        };
        let key = EncodingKey::from_secret(TEST_SECRET.as_bytes());
        let token = encode(&Header::default(), &claims_in, &key).unwrap();
        let claims = validate_identity_token(&token, TEST_SECRET).unwrap();
        assert_eq!(claims.tier, Tier::Enterprise);
        assert_eq!(claims.org_id.as_deref(), Some("org-acme"));
    }

    #[test]
    fn identity_token_expired_returns_error() {
        let token = make_identity_token("user-abc", Tier::Pro, 1000); // long past
        let result = validate_identity_token(&token, TEST_SECRET);
        assert!(matches!(result, Err(JwtError::Expired)));
    }

    #[test]
    fn identity_token_wrong_secret_returns_error() {
        let token = make_identity_token("user-abc", Tier::Free, future_exp());
        let result = validate_identity_token(&token, "wrong-secret");
        assert!(matches!(result, Err(JwtError::Invalid(_))));
    }

    #[test]
    fn identity_token_anonymous_tier() {
        let token = make_identity_token("anon-xyz", Tier::Anonymous, future_exp());
        let claims = validate_identity_token(&token, TEST_SECRET).unwrap();
        assert_eq!(claims.tier, Tier::Anonymous);
    }

    #[test]
    fn identity_token_with_custom_capabilities() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let claims_in = IdentityClaims {
            sub: "ent-user".to_string(),
            tier: Tier::Enterprise,
            org_id: Some("org-corp".to_string()),
            roles: vec![TenantRole::Member],
            custom_capabilities: Some(vec!["fs:read:**".to_string(), "exec:cmd:ls".to_string()]),
            iat: now,
            exp: now + 3600,
        };
        let key = EncodingKey::from_secret(TEST_SECRET.as_bytes());
        let token = encode(&Header::default(), &claims_in, &key).unwrap();
        let claims = validate_identity_token(&token, TEST_SECRET).unwrap();
        let caps = claims.custom_capabilities.unwrap();
        assert_eq!(caps.len(), 2);
        assert!(caps.contains(&"fs:read:**".to_string()));
    }

    // ── Cross-field invariant tests ───────────────────────────────────────────

    fn encode_identity(claims: &IdentityClaims) -> String {
        let key = EncodingKey::from_secret(TEST_SECRET.as_bytes());
        encode(&Header::default(), claims, &key).unwrap()
    }

    fn base_claims(tier: Tier) -> IdentityClaims {
        IdentityClaims {
            sub: "user-x".to_string(),
            tier,
            org_id: None,
            roles: vec![],
            custom_capabilities: None,
            iat: 0,
            exp: future_exp(),
        }
    }

    #[test]
    fn enterprise_without_org_id_is_rejected() {
        let claims = base_claims(Tier::Enterprise); // org_id is None
        let token = encode_identity(&claims);
        let result = validate_identity_token(&token, TEST_SECRET);
        assert!(
            matches!(result, Err(JwtError::Invalid(ref msg)) if msg.contains("org_id")),
            "expected Invalid error mentioning org_id, got: {result:?}"
        );
    }

    #[test]
    fn non_enterprise_with_org_id_is_rejected() {
        let mut claims = base_claims(Tier::Pro);
        claims.org_id = Some("org-sneaky".to_string());
        let token = encode_identity(&claims);
        let result = validate_identity_token(&token, TEST_SECRET);
        assert!(
            matches!(result, Err(JwtError::Invalid(ref msg)) if msg.contains("org_id")),
            "expected Invalid error mentioning org_id, got: {result:?}"
        );
    }

    #[test]
    fn non_enterprise_with_roles_is_rejected() {
        let mut claims = base_claims(Tier::Free);
        claims.roles = vec![TenantRole::Admin];
        let token = encode_identity(&claims);
        let result = validate_identity_token(&token, TEST_SECRET);
        assert!(
            matches!(result, Err(JwtError::Invalid(ref msg)) if msg.contains("roles")),
            "expected Invalid error mentioning roles, got: {result:?}"
        );
    }

    #[test]
    fn non_enterprise_with_custom_capabilities_is_rejected() {
        let mut claims = base_claims(Tier::Anonymous);
        claims.custom_capabilities = Some(vec!["fs:read:**".to_string()]);
        let token = encode_identity(&claims);
        let result = validate_identity_token(&token, TEST_SECRET);
        assert!(
            matches!(result, Err(JwtError::Invalid(ref msg)) if msg.contains("custom_capabilities")),
            "expected Invalid error mentioning custom_capabilities, got: {result:?}"
        );
    }

    #[test]
    fn enterprise_with_org_id_and_roles_is_accepted() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let claims = IdentityClaims {
            sub: "ent-admin".to_string(),
            tier: Tier::Enterprise,
            org_id: Some("org-valid".to_string()),
            roles: vec![TenantRole::Admin],
            custom_capabilities: None,
            iat: now,
            exp: now + 3600,
        };
        let token = encode_identity(&claims);
        let result = validate_identity_token(&token, TEST_SECRET);
        assert!(result.is_ok(), "valid enterprise token should be accepted: {result:?}");
    }
}
