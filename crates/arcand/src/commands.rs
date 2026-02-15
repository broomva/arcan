use arcan_core::runtime::{Orchestrator, Provider};
use arcan_provider::anthropic::{AnthropicConfig, AnthropicProvider};
use arcan_provider::openai::{OpenAiCompatibleProvider, OpenAiConfig};
use std::sync::Arc;

/// Result of processing a `/` command.
#[derive(Debug)]
pub enum CommandResult {
    /// Command produced a text response (don't send to LLM).
    Response(String),
    /// Not a command — pass through to the agent loop.
    NotACommand,
}

/// Process a `/` command from user input.
///
/// Supported commands:
/// - `/model` — show current provider/model
/// - `/model <name>` — switch provider (anthropic, openai, ollama, mock)
/// - `/help` — list available commands
pub fn handle_command(message: &str, orchestrator: &Arc<Orchestrator>) -> CommandResult {
    let trimmed = message.trim();

    if !trimmed.starts_with('/') {
        return CommandResult::NotACommand;
    }

    let parts: Vec<&str> = trimmed.splitn(2, ' ').collect();
    let cmd = parts[0];
    let arg = parts.get(1).map(|s| s.trim());

    match cmd {
        "/model" => handle_model_command(arg, orchestrator),
        "/help" => CommandResult::Response(help_text()),
        _ => CommandResult::Response(format!("Unknown command: {cmd}\n\n{}", help_text())),
    }
}

fn handle_model_command(arg: Option<&str>, orchestrator: &Arc<Orchestrator>) -> CommandResult {
    match arg {
        None | Some("") => {
            // Show current model
            let name = orchestrator.provider_name();
            CommandResult::Response(format!("Current model: {name}"))
        }
        Some(provider_spec) => {
            // Parse "provider" or "provider:model"
            let (provider_name, model_override) =
                if let Some((p, m)) = provider_spec.split_once(':') {
                    (p, Some(m))
                } else {
                    (provider_spec, None)
                };

            match create_provider(provider_name, model_override) {
                Ok(new_provider) => {
                    let name = orchestrator.swap_provider(new_provider);
                    CommandResult::Response(format!("Switched to model: {name}"))
                }
                Err(err) => CommandResult::Response(format!("Failed to switch model: {err}")),
            }
        }
    }
}

/// Create a provider from a name string, with optional model override.
///
/// Supported names:
/// - `anthropic` — Claude (requires ANTHROPIC_API_KEY)
/// - `openai` — GPT-4o (requires OPENAI_API_KEY)
/// - `ollama` — local Ollama (no key needed)
/// - `ollama:qwen2.5` — Ollama with specific model
/// - `openai:gpt-4-turbo` — OpenAI with specific model
/// - `mock` — MockProvider for testing
pub fn create_provider(
    name: &str,
    model_override: Option<&str>,
) -> Result<Arc<dyn Provider>, String> {
    match name {
        "anthropic" => {
            let mut config = AnthropicConfig::from_env().map_err(|e| e.to_string())?;
            if let Some(model) = model_override {
                config.model = model.to_string();
            }
            Ok(Arc::new(AnthropicProvider::new(config)))
        }
        "openai" => {
            let mut config = OpenAiConfig::openai_from_env().map_err(|e| e.to_string())?;
            if let Some(model) = model_override {
                config.model = model.to_string();
            }
            Ok(Arc::new(OpenAiCompatibleProvider::new(config)))
        }
        "ollama" => {
            let mut config = OpenAiConfig::ollama_from_env().map_err(|e| e.to_string())?;
            if let Some(model) = model_override {
                config.model = model.to_string();
            }
            Ok(Arc::new(OpenAiCompatibleProvider::new(config)))
        }
        "mock" => Ok(Arc::new(crate::mock::MockProvider)),
        other => Err(format!(
            "Unknown provider '{other}'. Available: anthropic, openai, ollama, mock"
        )),
    }
}

fn help_text() -> String {
    r#"Available commands:
  /model              — Show current provider and model
  /model <provider>   — Switch provider (anthropic, openai, ollama, mock)
  /model <provider>:<model> — Switch provider with specific model
                        Examples: /model ollama:qwen2.5
                                  /model openai:gpt-4-turbo
                                  /model anthropic:claude-sonnet-4-5-20250929
  /help               — Show this help"#
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use arcan_core::runtime::{OrchestratorConfig, ToolRegistry};

    fn test_orchestrator() -> Arc<Orchestrator> {
        Arc::new(Orchestrator::new(
            Arc::new(crate::mock::MockProvider),
            ToolRegistry::default(),
            Vec::new(),
            OrchestratorConfig {
                max_iterations: 10,
                context: None,
                context_compiler: None,
            },
        ))
    }

    #[test]
    fn not_a_command_passes_through() {
        let orch = test_orchestrator();
        assert!(matches!(
            handle_command("Hello world", &orch),
            CommandResult::NotACommand
        ));
    }

    #[test]
    fn model_without_arg_shows_current() {
        let orch = test_orchestrator();
        match handle_command("/model", &orch) {
            CommandResult::Response(text) => {
                assert!(
                    text.contains("mock-provider"),
                    "Should show mock provider, got: {text}"
                );
            }
            other => panic!("Expected Response, got: {other:?}"),
        }
    }

    #[test]
    fn model_switch_to_mock() {
        let orch = test_orchestrator();
        match handle_command("/model mock", &orch) {
            CommandResult::Response(text) => {
                assert!(text.contains("Switched"), "Should confirm switch: {text}");
            }
            other => panic!("Expected Response, got: {other:?}"),
        }
    }

    #[test]
    fn model_unknown_provider_returns_error() {
        let orch = test_orchestrator();
        match handle_command("/model unknown", &orch) {
            CommandResult::Response(text) => {
                assert!(
                    text.contains("Failed") || text.contains("Unknown"),
                    "Should error: {text}"
                );
            }
            other => panic!("Expected Response, got: {other:?}"),
        }
    }

    #[test]
    fn help_command() {
        let orch = test_orchestrator();
        match handle_command("/help", &orch) {
            CommandResult::Response(text) => {
                assert!(text.contains("/model"));
                assert!(text.contains("/help"));
            }
            other => panic!("Expected Response, got: {other:?}"),
        }
    }

    #[test]
    fn unknown_command_shows_help() {
        let orch = test_orchestrator();
        match handle_command("/foo", &orch) {
            CommandResult::Response(text) => {
                assert!(text.contains("Unknown command"));
                assert!(text.contains("/help"));
            }
            other => panic!("Expected Response, got: {other:?}"),
        }
    }

    #[test]
    fn model_with_colon_syntax() {
        let orch = test_orchestrator();
        // Can't test real provider switch without API keys, but can test mock
        match handle_command("/model mock", &orch) {
            CommandResult::Response(text) => assert!(text.contains("Switched")),
            other => panic!("Expected Response, got: {other:?}"),
        }
    }

    #[test]
    fn create_provider_mock_works() {
        let provider = create_provider("mock", None).unwrap();
        assert_eq!(provider.name(), "mock-provider");
    }

    #[test]
    fn create_provider_unknown_errors() {
        let result = create_provider("foobar", None);
        assert!(result.is_err());
        let err = result.err().unwrap();
        assert!(err.contains("Unknown provider"), "Got: {err}");
    }
}
