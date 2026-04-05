use arcan_core::error::CoreError;
use std::fmt;

/// A credential that can produce HTTP authorization headers.
///
/// Implementations handle API keys, OAuth tokens with refresh, etc.
pub trait Credential: Send + Sync + fmt::Debug {
    /// Returns the authorization header value (e.g. `"Bearer <token>"`).
    fn auth_header(&self) -> Result<String, CoreError>;

    /// Returns the credential kind for display/logging.
    fn kind(&self) -> &str;

    /// Whether this credential needs periodic refresh (OAuth tokens do, API keys don't).
    fn needs_refresh(&self) -> bool {
        false
    }

    /// Refresh the credential if needed. No-op for static credentials.
    fn refresh(&self) -> Result<(), CoreError> {
        Ok(())
    }
}

/// A static API key credential that produces `Bearer <key>` headers.
///
/// Used for OpenAI, Ollama, and other Bearer-token APIs.
#[derive(Clone)]
pub struct ApiKeyCredential {
    api_key: String,
}

impl fmt::Debug for ApiKeyCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ApiKeyCredential")
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}

impl ApiKeyCredential {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }

    /// Returns the raw API key (for providers that need non-Bearer auth).
    pub fn raw_key(&self) -> &str {
        &self.api_key
    }

    /// Whether the underlying key is empty (e.g. Ollama local servers).
    pub fn is_empty(&self) -> bool {
        self.api_key.is_empty()
    }
}

impl Credential for ApiKeyCredential {
    fn auth_header(&self) -> Result<String, CoreError> {
        if self.api_key.is_empty() {
            return Err(CoreError::Auth("API key is empty".to_string()));
        }
        Ok(format!("Bearer {}", self.api_key))
    }

    fn kind(&self) -> &str {
        "api_key"
    }
}

/// A static API key credential that produces `x-api-key` style headers.
///
/// Used specifically for Anthropic which uses a custom header instead of Bearer.
#[derive(Clone)]
pub struct AnthropicApiKeyCredential {
    api_key: String,
}

impl fmt::Debug for AnthropicApiKeyCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnthropicApiKeyCredential")
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}

impl AnthropicApiKeyCredential {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }

    /// Returns the raw API key for direct use in `x-api-key` header.
    pub fn raw_key(&self) -> &str {
        &self.api_key
    }
}

impl Credential for AnthropicApiKeyCredential {
    fn auth_header(&self) -> Result<String, CoreError> {
        if self.api_key.is_empty() {
            return Err(CoreError::Auth("Anthropic API key is empty".to_string()));
        }
        // Anthropic uses `x-api-key` header directly, but we return the raw value
        // so the provider can set it on the appropriate header.
        Ok(self.api_key.clone())
    }

    fn kind(&self) -> &str {
        "anthropic_api_key"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_key_credential_bearer_header() {
        let cred = ApiKeyCredential::new("sk-test-123".to_string());
        assert_eq!(cred.auth_header().unwrap(), "Bearer sk-test-123");
        assert_eq!(cred.kind(), "api_key");
        assert!(!cred.needs_refresh());
        assert!(!cred.is_empty());
    }

    #[test]
    fn api_key_credential_empty_returns_error() {
        let cred = ApiKeyCredential::new(String::new());
        assert!(cred.auth_header().is_err());
        assert!(cred.is_empty());
    }

    #[test]
    fn anthropic_credential_raw_key() {
        let cred = AnthropicApiKeyCredential::new("sk-ant-test".to_string());
        assert_eq!(cred.auth_header().unwrap(), "sk-ant-test");
        assert_eq!(cred.kind(), "anthropic_api_key");
        assert_eq!(cred.raw_key(), "sk-ant-test");
    }

    #[test]
    fn anthropic_credential_empty_returns_error() {
        let cred = AnthropicApiKeyCredential::new(String::new());
        assert!(cred.auth_header().is_err());
    }

    #[test]
    fn api_key_debug_redacts_key() {
        let cred = ApiKeyCredential::new("secret-key".to_string());
        let debug_output = format!("{cred:?}");
        assert!(!debug_output.contains("secret-key"));
        assert!(debug_output.contains("REDACTED"));
    }

    #[test]
    fn refresh_is_noop_for_static_credentials() {
        let cred = ApiKeyCredential::new("test".to_string());
        assert!(cred.refresh().is_ok());
    }
}
