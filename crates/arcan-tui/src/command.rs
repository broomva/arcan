/// Metadata for a slash command, used by autocomplete and `/help`.
#[derive(Debug, Clone)]
pub struct CommandInfo {
    pub name: &'static str,
    pub description: &'static str,
    pub usage: &'static str,
}

/// All available slash commands with descriptions (single source of truth).
pub const COMMANDS: &[CommandInfo] = &[
    CommandInfo {
        name: "/approve",
        description: "Submit approval decision",
        usage: "/approve <id> <yes|no> [reason]",
    },
    CommandInfo {
        name: "/clear",
        description: "Clear conversation",
        usage: "/clear",
    },
    CommandInfo {
        name: "/help",
        description: "Show available commands",
        usage: "/help",
    },
    CommandInfo {
        name: "/login",
        description: "Login to provider (device code)",
        usage: "/login [openai] [--browser]",
    },
    CommandInfo {
        name: "/logout",
        description: "Logout from provider",
        usage: "/logout [openai]",
    },
    CommandInfo {
        name: "/model",
        description: "Show or set model",
        usage: "/model [provider[:model]]",
    },
    CommandInfo {
        name: "/provider",
        description: "Show or set provider",
        usage: "/provider [name]",
    },
    CommandInfo {
        name: "/sessions",
        description: "Browse sessions",
        usage: "/sessions",
    },
    CommandInfo {
        name: "/state",
        description: "Inspect agent state",
        usage: "/state",
    },
];

/// Filter commands whose name starts with the given prefix.
pub fn filter_commands(prefix: &str) -> Vec<&'static CommandInfo> {
    COMMANDS
        .iter()
        .filter(|cmd| cmd.name.starts_with(prefix))
        .collect()
}

/// Parsed TUI command from user input.
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    /// Clear the conversation log.
    Clear,
    /// Show available commands.
    Help,
    /// Model inspection or switching.
    Model(ModelSubcommand),
    /// Submit an approval decision.
    Approve {
        approval_id: String,
        decision: String,
        reason: Option<String>,
    },
    /// Authenticate with a provider via OAuth.
    Login { provider: String, device: bool },
    /// Remove stored credentials for a provider.
    Logout { provider: String },
    /// Show or set the active provider.
    Provider { name: Option<String> },
    /// Toggle or browse sessions.
    Sessions,
    /// Fetch and display agent state.
    State,
    /// Send a plain message to the agent.
    SendMessage(String),
}

/// Model-related subcommands.
#[derive(Debug, PartialEq, Eq)]
pub enum ModelSubcommand {
    /// Show the current model.
    ShowCurrent,
    /// Set the provider (and optionally the model).
    Set {
        provider: String,
        model: Option<String>,
    },
}

/// Parse a user input string into a `Command`.
///
/// Slash-prefixed inputs are parsed as commands; everything else is a message.
pub fn parse(input: &str) -> Result<Command, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("Empty input".to_string());
    }

    if !trimmed.starts_with('/') {
        return Ok(Command::SendMessage(trimmed.to_string()));
    }

    let (cmd, args) = trimmed
        .split_once(' ')
        .map(|(c, a)| (c, a.trim()))
        .unwrap_or((trimmed, ""));

    match cmd {
        "/clear" => Ok(Command::Clear),
        "/help" => Ok(Command::Help),
        "/model" => parse_model(args),
        "/approve" => parse_approve(args),
        "/login" => parse_login(args),
        "/logout" => parse_logout(args),
        "/provider" => {
            let name = args.split_whitespace().next().map(|s| s.to_string());
            Ok(Command::Provider { name })
        }
        "/sessions" => Ok(Command::Sessions),
        "/state" => Ok(Command::State),
        unknown => Err(format!(
            "Unknown command: {unknown}. Type /help for available commands."
        )),
    }
}

fn parse_model(args: &str) -> Result<Command, String> {
    if args.is_empty() {
        return Ok(Command::Model(ModelSubcommand::ShowCurrent));
    }

    if args.contains(char::is_whitespace) {
        return Err("Usage: /model | /model <provider> | /model <provider>:<model>".to_string());
    }

    if let Some((provider, model)) = args.split_once(':') {
        if provider.is_empty() || model.is_empty() {
            return Err("Usage: /model <provider>:<model> (both values are required)".to_string());
        }
        return Ok(Command::Model(ModelSubcommand::Set {
            provider: provider.to_string(),
            model: Some(model.to_string()),
        }));
    }

    Ok(Command::Model(ModelSubcommand::Set {
        provider: args.to_string(),
        model: None,
    }))
}

fn parse_approve(args: &str) -> Result<Command, String> {
    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.len() < 2 {
        return Err("Usage: /approve <id> <yes|no> [reason]".to_string());
    }

    let approval_id = parts[0].to_string();
    let decision = match parts[1].to_ascii_lowercase().as_str() {
        "yes" | "y" | "approved" | "approve" => "approved".to_string(),
        "no" | "n" | "denied" | "deny" => "denied".to_string(),
        invalid => {
            return Err(format!(
                "Invalid approval decision '{invalid}'. Use yes/no."
            ));
        }
    };

    let reason = if parts.len() > 2 {
        Some(parts[2..].join(" "))
    } else {
        None
    };

    Ok(Command::Approve {
        approval_id,
        decision,
        reason,
    })
}

fn parse_login(args: &str) -> Result<Command, String> {
    let parts: Vec<&str> = args.split_whitespace().collect();
    let provider = parts
        .iter()
        .find(|p| !p.starts_with("--"))
        .copied()
        .unwrap_or("openai")
        .to_string();
    // Default to device code flow in TUI (no browser needed).
    // Use --browser to opt into PKCE browser flow.
    let device = !parts.contains(&"--browser");
    Ok(Command::Login { provider, device })
}

fn parse_logout(args: &str) -> Result<Command, String> {
    let provider = args
        .split_whitespace()
        .next()
        .unwrap_or("openai")
        .to_string();
    Ok(Command::Logout { provider })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_message() {
        assert_eq!(
            parse("hello world").unwrap(),
            Command::SendMessage("hello world".to_string())
        );
    }

    #[test]
    fn clear_command() {
        assert_eq!(parse("/clear").unwrap(), Command::Clear);
    }

    #[test]
    fn help_command() {
        assert_eq!(parse("/help").unwrap(), Command::Help);
    }

    #[test]
    fn model_show() {
        assert_eq!(
            parse("/model").unwrap(),
            Command::Model(ModelSubcommand::ShowCurrent)
        );
    }

    #[test]
    fn model_set_provider() {
        assert_eq!(
            parse("/model mock").unwrap(),
            Command::Model(ModelSubcommand::Set {
                provider: "mock".to_string(),
                model: None,
            })
        );
    }

    #[test]
    fn model_set_provider_with_model() {
        assert_eq!(
            parse("/model ollama:qwen2.5").unwrap(),
            Command::Model(ModelSubcommand::Set {
                provider: "ollama".to_string(),
                model: Some("qwen2.5".to_string()),
            })
        );
    }

    #[test]
    fn model_rejects_incomplete() {
        let err = parse("/model ollama:").unwrap_err();
        assert!(err.contains("required"), "got: {err}");
    }

    #[test]
    fn model_rejects_spaces() {
        let err = parse("/model ollama qwen").unwrap_err();
        assert!(err.contains("Usage"), "got: {err}");
    }

    #[test]
    fn approve_yes() {
        assert_eq!(
            parse("/approve ap-1 yes because").unwrap(),
            Command::Approve {
                approval_id: "ap-1".to_string(),
                decision: "approved".to_string(),
                reason: Some("because".to_string()),
            }
        );
    }

    #[test]
    fn approve_no_reason() {
        assert_eq!(
            parse("/approve ap-2 no").unwrap(),
            Command::Approve {
                approval_id: "ap-2".to_string(),
                decision: "denied".to_string(),
                reason: None,
            }
        );
    }

    #[test]
    fn approve_missing_args() {
        let err = parse("/approve ap-1").unwrap_err();
        assert!(err.contains("Usage"), "got: {err}");
    }

    #[test]
    fn approve_invalid_decision() {
        let err = parse("/approve ap-1 maybe").unwrap_err();
        assert!(err.contains("Invalid"), "got: {err}");
    }

    #[test]
    fn unknown_command() {
        let err = parse("/foobar").unwrap_err();
        assert!(err.contains("Unknown"), "got: {err}");
    }

    #[test]
    fn empty_input() {
        assert!(parse("").is_err());
        assert!(parse("   ").is_err());
    }

    #[test]
    fn sessions_command() {
        assert_eq!(parse("/sessions").unwrap(), Command::Sessions);
    }

    #[test]
    fn state_command() {
        assert_eq!(parse("/state").unwrap(), Command::State);
    }

    #[test]
    fn login_default_uses_device_flow() {
        assert_eq!(
            parse("/login").unwrap(),
            Command::Login {
                provider: "openai".to_string(),
                device: true,
            }
        );
    }

    #[test]
    fn login_explicit_provider() {
        assert_eq!(
            parse("/login openai").unwrap(),
            Command::Login {
                provider: "openai".to_string(),
                device: true,
            }
        );
    }

    #[test]
    fn login_browser_flag() {
        assert_eq!(
            parse("/login openai --browser").unwrap(),
            Command::Login {
                provider: "openai".to_string(),
                device: false,
            }
        );
    }

    #[test]
    fn logout_default_provider() {
        assert_eq!(
            parse("/logout").unwrap(),
            Command::Logout {
                provider: "openai".to_string(),
            }
        );
    }

    #[test]
    fn logout_explicit_provider() {
        assert_eq!(
            parse("/logout openai").unwrap(),
            Command::Logout {
                provider: "openai".to_string(),
            }
        );
    }

    #[test]
    fn provider_command() {
        assert_eq!(
            parse("/provider").unwrap(),
            Command::Provider { name: None }
        );
    }

    #[test]
    fn provider_set() {
        assert_eq!(
            parse("/provider ollama").unwrap(),
            Command::Provider {
                name: Some("ollama".to_string()),
            }
        );
    }

    #[test]
    fn filter_commands_all() {
        let all = filter_commands("/");
        assert_eq!(all.len(), COMMANDS.len());
    }

    #[test]
    fn filter_commands_prefix() {
        let filtered = filter_commands("/cl");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "/clear");
    }

    #[test]
    fn filter_commands_multiple_matches() {
        let filtered = filter_commands("/s");
        assert_eq!(filtered.len(), 2);
        let names: Vec<&str> = filtered.iter().map(|c| c.name).collect();
        assert!(names.contains(&"/sessions"));
        assert!(names.contains(&"/state"));
    }

    #[test]
    fn filter_commands_no_match() {
        let filtered = filter_commands("/xyz");
        assert!(filtered.is_empty());
    }
}
