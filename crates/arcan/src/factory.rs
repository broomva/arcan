use arcan_core::error::CoreError;
use arcan_core::runtime::{Provider, ProviderFactory};
use arcan_provider::anthropic::{AnthropicConfig, AnthropicProvider};
use arcan_provider::openai::{OpenAiCompatibleProvider, OpenAiConfig};
use arcan_provider::{apfel, ollama};
use arcand::mock::MockProvider;
use std::sync::Arc;

/// Default provider factory for the daemon binary.
///
/// Delegates to existing provider configs. Parses specs like
/// `"anthropic"`, `"ollama:llama3.2"`, `"openai:gpt-4o"`, `"mock"`.
pub struct ArcanProviderFactory;

impl ProviderFactory for ArcanProviderFactory {
    fn build(&self, spec: &str) -> Result<Arc<dyn Provider>, CoreError> {
        let (provider_name, model_override) = match spec.split_once(':') {
            Some((name, model)) => (name, Some(model)),
            None => (spec, None),
        };

        match provider_name {
            "mock" => {
                tracing::info!("Provider switched to: mock");
                Ok(Arc::new(MockProvider))
            }
            "anthropic" => {
                let config = AnthropicConfig::from_resolved(model_override, None, None)
                    .map_err(|e| CoreError::Provider(e.to_string()))?;
                tracing::info!(model = %config.model, "Provider switched to: anthropic");
                Ok(Arc::new(AnthropicProvider::new(config)))
            }
            "openai" | "codex" | "openai-codex" => {
                let config = OpenAiConfig::openai_from_resolved(model_override, None, None)
                    .map_err(|e| CoreError::Provider(e.to_string()))?;
                tracing::info!(model = %config.model, "Provider switched to: openai");
                Ok(Arc::new(OpenAiCompatibleProvider::new(config)))
            }
            "ollama" => {
                let base_url = ollama::resolve_base_url();
                ollama::ensure_ollama_running(&base_url)?;
                let config =
                    OpenAiConfig::ollama_from_resolved(model_override, Some(&base_url), None, None)
                        .map_err(|e| CoreError::Provider(e.to_string()))?;
                tracing::info!(model = %config.model, base_url = %config.base_url, "Provider switched to: ollama");
                Ok(Arc::new(OpenAiCompatibleProvider::new(config)))
            }
            "apfel" | "apple" => {
                let base_url = apfel::resolve_base_url();
                apfel::ensure_apfel_running(&base_url)?;
                let config = OpenAiConfig::apfel_from_resolved(Some(&base_url), None)
                    .map_err(|e| CoreError::Provider(e.to_string()))?;
                tracing::info!(base_url = %config.base_url, "Provider switched to: apfel (Apple on-device)");
                Ok(Arc::new(OpenAiCompatibleProvider::new(config)))
            }
            _ => Err(CoreError::Provider(format!(
                "unknown provider: \"{provider_name}\". Available: anthropic, openai, ollama, apfel, mock"
            ))),
        }
    }

    fn available_providers(&self) -> Vec<String> {
        let mut providers = vec![
            "anthropic".to_string(),
            "openai".to_string(),
            "mock".to_string(),
        ];

        // Check apfel (Apple on-device model)
        let apfel_url = apfel::resolve_base_url();
        if apfel::is_apfel_running(&apfel_url) {
            providers.push("apfel".to_string());
        }

        let base_url = ollama::resolve_base_url();
        if ollama::is_ollama_running(&base_url) {
            match ollama::list_ollama_models(&base_url) {
                Ok(models) if !models.is_empty() => {
                    for model in models {
                        providers.push(format!("ollama:{model}"));
                    }
                }
                _ => {
                    providers.push("ollama".to_string());
                }
            }
        } else {
            providers.push("ollama".to_string());
        }

        providers
    }
}
