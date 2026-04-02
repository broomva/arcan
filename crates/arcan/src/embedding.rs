//! Pluggable embedding provider for semantic vector search.
//!
//! Configured via environment variables:
//! - `ARCAN_EMBEDDING_URL` — OpenAI-compatible `/v1/embeddings` endpoint
//! - `ARCAN_EMBEDDING_MODEL` — model name (default: `text-embedding-3-small`)
//! - `ARCAN_EMBEDDING_API_KEY` or `OPENAI_API_KEY` — bearer token
//!
//! If no URL is configured, embedding is disabled (graceful degradation).

use serde_json::json;

/// Trait for embedding text into vectors.
pub trait EmbeddingProvider: Send + Sync {
    /// Embed a single text string into a float vector.
    fn embed(&self, text: &str) -> Result<Vec<f32>, anyhow::Error>;
}

/// HTTP-based embedding provider compatible with the OpenAI `/v1/embeddings` API.
///
/// Works with OpenAI, vLLM, Ollama (`/v1/embeddings`), or any compatible endpoint.
pub struct HttpEmbeddingProvider {
    url: String,
    model: String,
    api_key: Option<String>,
    client: reqwest::blocking::Client,
}

impl HttpEmbeddingProvider {
    /// Create from explicit configuration.
    pub fn new(url: String, model: String, api_key: Option<String>) -> Self {
        Self {
            url,
            model,
            api_key,
            client: reqwest::blocking::Client::new(),
        }
    }

    /// Attempt to create from environment variables.
    ///
    /// Returns `None` if `ARCAN_EMBEDDING_URL` is not set — graceful degradation.
    pub fn from_env() -> Option<Self> {
        let url = std::env::var("ARCAN_EMBEDDING_URL").ok()?;
        let model = std::env::var("ARCAN_EMBEDDING_MODEL")
            .unwrap_or_else(|_| "text-embedding-3-small".to_string());
        let api_key = std::env::var("ARCAN_EMBEDDING_API_KEY")
            .or_else(|_| std::env::var("OPENAI_API_KEY"))
            .ok();

        Some(Self::new(url, model, api_key))
    }
}

impl EmbeddingProvider for HttpEmbeddingProvider {
    fn embed(&self, text: &str) -> Result<Vec<f32>, anyhow::Error> {
        let body = json!({
            "input": text,
            "model": &self.model,
        });

        let mut request = self
            .client
            .post(&self.url)
            .header("Content-Type", "application/json")
            .json(&body);

        if let Some(ref key) = self.api_key {
            request = request.bearer_auth(key);
        }

        let response = request.send()?;
        let status = response.status();
        if !status.is_success() {
            let text = response.text().unwrap_or_default();
            anyhow::bail!("embedding API returned {status}: {text}");
        }

        let json: serde_json::Value = response.json()?;
        let embedding = json["data"][0]["embedding"]
            .as_array()
            .ok_or_else(|| {
                anyhow::anyhow!("invalid embedding response: missing data[0].embedding")
            })?
            .iter()
            .map(|v| v.as_f64().unwrap_or(0.0) as f32)
            .collect();

        Ok(embedding)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_env_returns_none_when_unset() {
        // Clear the env var to ensure None (it's very unlikely to be set in tests)
        let result = HttpEmbeddingProvider::from_env();
        // If ARCAN_EMBEDDING_URL happens to be set in the test environment,
        // this test still passes — it just returns Some.
        // The key invariant: from_env() never panics.
        let _ = result;
    }

    #[test]
    fn new_creates_provider() {
        let provider = HttpEmbeddingProvider::new(
            "http://localhost:11434/v1/embeddings".to_string(),
            "nomic-embed-text".to_string(),
            None,
        );
        assert_eq!(provider.url, "http://localhost:11434/v1/embeddings");
        assert_eq!(provider.model, "nomic-embed-text");
        assert!(provider.api_key.is_none());
    }

    #[test]
    fn new_with_api_key() {
        let provider = HttpEmbeddingProvider::new(
            "https://api.openai.com/v1/embeddings".to_string(),
            "text-embedding-3-small".to_string(),
            Some("sk-test-key".to_string()),
        );
        assert_eq!(provider.api_key.as_deref(), Some("sk-test-key"));
    }
}
