use arcan_core::error::CoreError;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::path::PathBuf;
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::credential::Credential;

// ─── OpenAI Codex OAuth constants ─────────────────────────────────

const OPENAI_AUTH_URL: &str = "https://auth.openai.com/authorize";
const OPENAI_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const OPENAI_DEVICE_AUTH_URL: &str = "https://auth.openai.com/oauth/device/code";
const OPENAI_CLIENT_ID: &str = "app_scp_BIqDzYAUWMiRFEih7bh0N";
const OPENAI_REDIRECT_URI: &str = "http://127.0.0.1:8769/callback";
const OPENAI_SCOPE: &str = "openai.public";

// ─── Token types ──────────────────────────────────────────────────

/// Persisted OAuth token set.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthTokenSet {
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// Absolute expiry time (seconds since UNIX epoch).
    pub expires_at: u64,
    pub provider: String,
}

impl OAuthTokenSet {
    pub fn is_expired(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        // Treat as expired 60s before actual expiry to avoid edge-case failures.
        now >= self.expires_at.saturating_sub(60)
    }
}

/// A credential backed by an OAuth token with automatic refresh.
pub struct OAuthCredential {
    tokens: RwLock<OAuthTokenSet>,
    client_id: String,
    token_url: String,
}

impl fmt::Debug for OAuthCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OAuthCredential")
            .field("provider", &self.provider_name())
            .field("token_url", &self.token_url)
            .finish()
    }
}

impl OAuthCredential {
    pub fn new(tokens: OAuthTokenSet, client_id: String, token_url: String) -> Self {
        Self {
            tokens: RwLock::new(tokens),
            client_id,
            token_url,
        }
    }

    /// Create from a stored token set using OpenAI defaults.
    pub fn openai(tokens: OAuthTokenSet) -> Self {
        Self::new(
            tokens,
            OPENAI_CLIENT_ID.to_string(),
            OPENAI_TOKEN_URL.to_string(),
        )
    }

    fn provider_name(&self) -> String {
        self.tokens
            .read()
            .map(|t| t.provider.clone())
            .unwrap_or_else(|_| "unknown".to_string())
    }
}

impl Credential for OAuthCredential {
    fn auth_header(&self) -> Result<String, CoreError> {
        // Auto-refresh if expired.
        if self.needs_refresh() {
            self.refresh()?;
        }
        let tokens = self
            .tokens
            .read()
            .map_err(|e| CoreError::Auth(format!("token lock poisoned: {e}")))?;
        Ok(format!("Bearer {}", tokens.access_token))
    }

    fn kind(&self) -> &str {
        "oauth"
    }

    fn needs_refresh(&self) -> bool {
        self.tokens.read().map(|t| t.is_expired()).unwrap_or(true)
    }

    fn refresh(&self) -> Result<(), CoreError> {
        let refresh_token = {
            let tokens = self
                .tokens
                .read()
                .map_err(|e| CoreError::Auth(format!("token lock poisoned: {e}")))?;
            match &tokens.refresh_token {
                Some(rt) => rt.clone(),
                None => {
                    return Err(CoreError::Auth(
                        "no refresh token available, re-login required".to_string(),
                    ));
                }
            }
        };

        let new_tokens = refresh_token_grant(&self.token_url, &self.client_id, &refresh_token)?;

        let mut tokens = self
            .tokens
            .write()
            .map_err(|e| CoreError::Auth(format!("token lock poisoned: {e}")))?;

        tokens.access_token = new_tokens.access_token;
        if let Some(rt) = new_tokens.refresh_token {
            tokens.refresh_token = Some(rt);
        }
        tokens.expires_at = new_tokens.expires_at;

        // Persist refreshed tokens.
        if let Err(e) = store_tokens(&tokens) {
            tracing::warn!(%e, "failed to persist refreshed tokens");
        }

        Ok(())
    }
}

// ─── Token storage ────────────────────────────────────────────────

/// Returns the credentials directory: `~/.arcan/credentials/`.
pub fn credentials_dir() -> Result<PathBuf, CoreError> {
    let home = dirs::home_dir()
        .ok_or_else(|| CoreError::Auth("could not determine home directory".to_string()))?;
    Ok(home.join(".arcan").join("credentials"))
}

/// Path to the credential file for a given provider.
fn credential_path(provider: &str) -> Result<PathBuf, CoreError> {
    Ok(credentials_dir()?.join(format!("{provider}.json")))
}

/// Store tokens to `~/.arcan/credentials/<provider>.json`.
pub fn store_tokens(tokens: &OAuthTokenSet) -> Result<(), CoreError> {
    let dir = credentials_dir()?;
    std::fs::create_dir_all(&dir)
        .map_err(|e| CoreError::Auth(format!("failed to create credentials dir: {e}")))?;

    let path = dir.join(format!("{}.json", tokens.provider));
    let json = serde_json::to_string_pretty(tokens)
        .map_err(|e| CoreError::Auth(format!("failed to serialize tokens: {e}")))?;

    std::fs::write(&path, &json)
        .map_err(|e| CoreError::Auth(format!("failed to write credentials: {e}")))?;

    // Set file permissions to 0600 (owner read/write only) on Unix.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(&path, perms)
            .map_err(|e| CoreError::Auth(format!("failed to set file permissions: {e}")))?;
    }

    Ok(())
}

/// Load stored tokens for a given provider.
pub fn load_tokens(provider: &str) -> Result<OAuthTokenSet, CoreError> {
    let path = credential_path(provider)?;
    let json = std::fs::read_to_string(&path)
        .map_err(|e| CoreError::Auth(format!("no stored credentials for {provider}: {e}")))?;
    serde_json::from_str(&json)
        .map_err(|e| CoreError::Auth(format!("invalid stored credentials for {provider}: {e}")))
}

/// Remove stored tokens for a given provider.
pub fn remove_tokens(provider: &str) -> Result<(), CoreError> {
    let path = credential_path(provider)?;
    if path.exists() {
        std::fs::remove_file(&path)
            .map_err(|e| CoreError::Auth(format!("failed to remove credentials: {e}")))?;
    }
    Ok(())
}

/// List providers that have stored credentials.
pub fn list_stored_providers() -> Vec<String> {
    let Ok(dir) = credentials_dir() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(std::result::Result::ok)
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.strip_suffix(".json").map(String::from)
        })
        .collect()
}

// ─── Token refresh ────────────────────────────────────────────────

/// Standard OAuth 2.0 refresh_token grant.
fn refresh_token_grant(
    token_url: &str,
    client_id: &str,
    refresh_token: &str,
) -> Result<OAuthTokenSet, CoreError> {
    let client = reqwest::blocking::Client::new();
    let resp = client
        .post(token_url)
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", client_id),
            ("refresh_token", refresh_token),
        ])
        .send()
        .map_err(|e| CoreError::Auth(format!("refresh request failed: {e}")))?;

    let status = resp.status();
    let body = resp
        .text()
        .map_err(|e| CoreError::Auth(format!("failed to read refresh response: {e}")))?;

    if !status.is_success() {
        return Err(CoreError::Auth(format!(
            "token refresh failed ({status}): {body}"
        )));
    }

    parse_token_response(&body, "openai")
}

// ─── PKCE helpers ─────────────────────────────────────────────────

/// Generate a random PKCE code verifier (43-128 URL-safe characters).
fn generate_code_verifier() -> String {
    let mut rng = rand::rng();
    let bytes: Vec<u8> = (0..32).map(|_| rng.random::<u8>()).collect();
    URL_SAFE_NO_PAD.encode(&bytes)
}

/// Compute the S256 code challenge from a verifier.
fn compute_code_challenge(verifier: &str) -> String {
    let hash = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(hash)
}

// ─── OAuth flows ──────────────────────────────────────────────────

/// Run the PKCE Authorization Code flow for OpenAI.
///
/// 1. Generate PKCE verifier/challenge
/// 2. Open browser to authorization URL
/// 3. Start local server on port 8769 to receive callback
/// 4. Exchange authorization code for tokens
/// 5. Store tokens to disk
#[allow(clippy::print_stderr)]
pub fn pkce_login_openai() -> Result<OAuthTokenSet, CoreError> {
    let code_verifier = generate_code_verifier();
    let code_challenge = compute_code_challenge(&code_verifier);

    // Build authorization URL.
    let auth_url = format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&code_challenge={}&code_challenge_method=S256",
        OPENAI_AUTH_URL,
        urlencoding::encode(OPENAI_CLIENT_ID),
        urlencoding::encode(OPENAI_REDIRECT_URI),
        urlencoding::encode(OPENAI_SCOPE),
        urlencoding::encode(&code_challenge),
    );

    eprintln!("Opening browser for OpenAI authentication...");
    eprintln!("If the browser doesn't open, visit:\n{auth_url}\n");

    // Try to open browser (best-effort).
    let _ = open::that(&auth_url);

    // Start local callback server.
    let listener = std::net::TcpListener::bind("127.0.0.1:8769")
        .map_err(|e| CoreError::Auth(format!("failed to bind callback server: {e}")))?;

    eprintln!("Waiting for authorization callback on http://127.0.0.1:8769/callback ...");

    let code = wait_for_callback(&listener)?;

    // Exchange code for tokens.
    let client = reqwest::blocking::Client::new();
    let resp = client
        .post(OPENAI_TOKEN_URL)
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", OPENAI_CLIENT_ID),
            ("code", code.as_str()),
            ("redirect_uri", OPENAI_REDIRECT_URI),
            ("code_verifier", code_verifier.as_str()),
        ])
        .send()
        .map_err(|e| CoreError::Auth(format!("token exchange failed: {e}")))?;

    let status = resp.status();
    let body = resp
        .text()
        .map_err(|e| CoreError::Auth(format!("failed to read token response: {e}")))?;

    if !status.is_success() {
        return Err(CoreError::Auth(format!(
            "token exchange failed ({status}): {body}"
        )));
    }

    let tokens = parse_token_response(&body, "openai")?;
    store_tokens(&tokens)?;

    eprintln!("Successfully authenticated with OpenAI!");
    Ok(tokens)
}

/// Run the Device Code flow for OpenAI (headless/SSH environments).
///
/// 1. Request device code
/// 2. Display verification URL and user code
/// 3. Poll token endpoint until authorized
/// 4. Store tokens to disk
#[allow(clippy::print_stderr)]
pub fn device_login_openai() -> Result<OAuthTokenSet, CoreError> {
    let client = reqwest::blocking::Client::new();

    // Step 1: Request device code.
    let resp = client
        .post(OPENAI_DEVICE_AUTH_URL)
        .form(&[("client_id", OPENAI_CLIENT_ID), ("scope", OPENAI_SCOPE)])
        .send()
        .map_err(|e| CoreError::Auth(format!("device auth request failed: {e}")))?;

    let status = resp.status();
    let body = resp
        .text()
        .map_err(|e| CoreError::Auth(format!("failed to read device auth response: {e}")))?;

    if !status.is_success() {
        return Err(CoreError::Auth(format!(
            "device authorization failed ({status}): {body}"
        )));
    }

    let device_resp: DeviceCodeResponse = serde_json::from_str(&body)
        .map_err(|e| CoreError::Auth(format!("invalid device auth response: {e}")))?;

    // Step 2: Display instructions.
    eprintln!("\nTo authenticate, visit: {}", device_resp.verification_uri);
    if let Some(ref complete_uri) = device_resp.verification_uri_complete {
        eprintln!("Or open: {complete_uri}");
    }
    eprintln!("Enter code: {}\n", device_resp.user_code);

    // Step 3: Poll for token.
    let interval = std::time::Duration::from_secs(device_resp.interval.max(5));
    let deadline =
        std::time::Instant::now() + std::time::Duration::from_secs(device_resp.expires_in.min(900));

    loop {
        std::thread::sleep(interval);

        if std::time::Instant::now() > deadline {
            return Err(CoreError::Auth(
                "device authorization timed out".to_string(),
            ));
        }

        let resp = client
            .post(OPENAI_TOKEN_URL)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("client_id", OPENAI_CLIENT_ID),
                ("device_code", &device_resp.device_code),
            ])
            .send()
            .map_err(|e| CoreError::Auth(format!("device token poll failed: {e}")))?;

        let status = resp.status();
        let body = resp
            .text()
            .map_err(|e| CoreError::Auth(format!("failed to read poll response: {e}")))?;

        if status.is_success() {
            let tokens = parse_token_response(&body, "openai")?;
            store_tokens(&tokens)?;
            eprintln!("Successfully authenticated with OpenAI!");
            return Ok(tokens);
        }

        // Check for specific OAuth error codes.
        if let Ok(err_resp) = serde_json::from_str::<OAuthErrorResponse>(&body) {
            match err_resp.error.as_str() {
                "authorization_pending" => continue,
                "slow_down" => {
                    // Back off a bit more.
                    std::thread::sleep(std::time::Duration::from_secs(5));
                    continue;
                }
                "expired_token" => {
                    return Err(CoreError::Auth(
                        "device code expired, please try again".to_string(),
                    ));
                }
                "access_denied" => {
                    return Err(CoreError::Auth("authorization denied by user".to_string()));
                }
                _ => {
                    return Err(CoreError::Auth(format!(
                        "device auth error: {}",
                        err_resp.error_description.unwrap_or(err_resp.error)
                    )));
                }
            }
        }

        return Err(CoreError::Auth(format!(
            "unexpected device token response ({status}): {body}"
        )));
    }
}

// ─── Internal helpers ─────────────────────────────────────────────

/// Wait for the OAuth callback on a local server, extract the `code` parameter.
fn wait_for_callback(listener: &std::net::TcpListener) -> Result<String, CoreError> {
    use std::io::{Read, Write};

    let (mut stream, _) = listener
        .accept()
        .map_err(|e| CoreError::Auth(format!("failed to accept callback: {e}")))?;

    let mut buf = [0u8; 4096];
    let n = stream
        .read(&mut buf)
        .map_err(|e| CoreError::Auth(format!("failed to read callback request: {e}")))?;
    let request = String::from_utf8_lossy(&buf[..n]);

    // Extract the request path from "GET /callback?code=...&state=... HTTP/1.1"
    let path = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .ok_or_else(|| CoreError::Auth("malformed callback request".to_string()))?;

    // Parse the URL to extract the code parameter.
    let full_url = format!("http://127.0.0.1:8769{path}");
    let parsed = url::Url::parse(&full_url)
        .map_err(|e| CoreError::Auth(format!("failed to parse callback URL: {e}")))?;

    // Check for error in callback.
    if let Some(error) = parsed.query_pairs().find(|(k, _)| k == "error") {
        let desc = parsed
            .query_pairs()
            .find(|(k, _)| k == "error_description")
            .map(|(_, v)| v.to_string())
            .unwrap_or_default();
        // Send error response to browser.
        let response = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n<html><body><h1>Authentication Failed</h1><p>You can close this window.</p></body></html>";
        let _ = stream.write_all(response.as_bytes());
        return Err(CoreError::Auth(format!(
            "OAuth error: {} — {desc}",
            error.1
        )));
    }

    let code = parsed
        .query_pairs()
        .find(|(k, _)| k == "code")
        .map(|(_, v)| v.to_string())
        .ok_or_else(|| CoreError::Auth("no authorization code in callback".to_string()))?;

    // Send success response to browser.
    let response = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n<html><body><h1>Authenticated!</h1><p>You can close this window and return to the terminal.</p></body></html>";
    let _ = stream.write_all(response.as_bytes());

    Ok(code)
}

/// Parse a standard OAuth token response into our `OAuthTokenSet`.
fn parse_token_response(body: &str, provider: &str) -> Result<OAuthTokenSet, CoreError> {
    let resp: TokenResponse = serde_json::from_str(body)
        .map_err(|e| CoreError::Auth(format!("invalid token response: {e}")))?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    Ok(OAuthTokenSet {
        access_token: resp.access_token,
        refresh_token: resp.refresh_token,
        expires_at: now + resp.expires_in.unwrap_or(3600),
        provider: provider.to_string(),
    })
}

// ─── Response types ───────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    #[allow(dead_code)]
    token_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    verification_uri_complete: Option<String>,
    expires_in: u64,
    interval: u64,
}

#[derive(Debug, Deserialize)]
struct OAuthErrorResponse {
    error: String,
    error_description: Option<String>,
}

// ─── URL encoding helper ──────────────────────────────────────────

mod urlencoding {
    pub fn encode(s: &str) -> String {
        let mut encoded = String::new();
        for byte in s.bytes() {
            match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    encoded.push(byte as char);
                }
                _ => {
                    encoded.push('%');
                    encoded.push_str(&format!("{byte:02X}"));
                }
            }
        }
        encoded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_set_not_expired() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let tokens = OAuthTokenSet {
            access_token: "test".to_string(),
            refresh_token: Some("refresh".to_string()),
            expires_at: now + 3600,
            provider: "test".to_string(),
        };
        assert!(!tokens.is_expired());
    }

    #[test]
    fn token_set_expired() {
        let tokens = OAuthTokenSet {
            access_token: "test".to_string(),
            refresh_token: None,
            expires_at: 1000, // Long past.
            provider: "test".to_string(),
        };
        assert!(tokens.is_expired());
    }

    #[test]
    fn token_set_expires_within_buffer() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let tokens = OAuthTokenSet {
            access_token: "test".to_string(),
            refresh_token: None,
            expires_at: now + 30, // 30s from now, within 60s buffer.
            provider: "test".to_string(),
        };
        assert!(tokens.is_expired());
    }

    #[test]
    fn pkce_code_verifier_length() {
        let verifier = generate_code_verifier();
        assert!(verifier.len() >= 43);
    }

    #[test]
    fn pkce_code_challenge_is_base64url() {
        let verifier = generate_code_verifier();
        let challenge = compute_code_challenge(&verifier);
        // Base64url should not contain +, /, or =.
        assert!(!challenge.contains('+'));
        assert!(!challenge.contains('/'));
        assert!(!challenge.contains('='));
        assert_eq!(challenge.len(), 43); // SHA-256 = 32 bytes → 43 base64url chars.
    }

    #[test]
    fn parse_token_response_full() {
        let body = r#"{
            "access_token": "at-123",
            "refresh_token": "rt-456",
            "expires_in": 7200,
            "token_type": "Bearer"
        }"#;
        let tokens = parse_token_response(body, "openai").unwrap();
        assert_eq!(tokens.access_token, "at-123");
        assert_eq!(tokens.refresh_token.as_deref(), Some("rt-456"));
        assert_eq!(tokens.provider, "openai");
        assert!(!tokens.is_expired());
    }

    #[test]
    fn parse_token_response_minimal() {
        let body = r#"{"access_token": "at-123"}"#;
        let tokens = parse_token_response(body, "test").unwrap();
        assert_eq!(tokens.access_token, "at-123");
        assert!(tokens.refresh_token.is_none());
    }

    #[test]
    fn token_storage_roundtrip() {
        let dir = std::env::temp_dir().join("arcan-oauth-test");
        let _ = std::fs::remove_dir_all(&dir);

        // Temporarily override home dir by using direct path functions.
        let tokens = OAuthTokenSet {
            access_token: "at-roundtrip".to_string(),
            refresh_token: Some("rt-roundtrip".to_string()),
            expires_at: 9999999999,
            provider: "test-roundtrip".to_string(),
        };

        // Write directly to a known path.
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test-roundtrip.json");
        let json = serde_json::to_string_pretty(&tokens).unwrap();
        std::fs::write(&path, &json).unwrap();

        // Read back.
        let loaded: OAuthTokenSet =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(loaded.access_token, "at-roundtrip");
        assert_eq!(loaded.refresh_token.as_deref(), Some("rt-roundtrip"));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn oauth_credential_kind() {
        let tokens = OAuthTokenSet {
            access_token: "at-test".to_string(),
            refresh_token: None,
            expires_at: 9999999999,
            provider: "openai".to_string(),
        };
        let cred = OAuthCredential::openai(tokens);
        assert_eq!(cred.kind(), "oauth");
    }

    #[test]
    fn oauth_credential_auth_header_with_valid_token() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let tokens = OAuthTokenSet {
            access_token: "at-valid".to_string(),
            refresh_token: None,
            expires_at: now + 3600,
            provider: "openai".to_string(),
        };
        let cred = OAuthCredential::openai(tokens);
        assert_eq!(cred.auth_header().unwrap(), "Bearer at-valid");
    }

    #[test]
    fn urlencoding_basic() {
        assert_eq!(urlencoding::encode("hello"), "hello");
        assert_eq!(urlencoding::encode("hello world"), "hello%20world");
        assert_eq!(urlencoding::encode("a+b"), "a%2Bb");
    }

    #[test]
    fn list_stored_providers_empty() {
        // This just tests that the function doesn't panic on empty/missing dirs.
        // In CI or fresh machines, there may be no stored providers.
        let _ = list_stored_providers();
    }
}
