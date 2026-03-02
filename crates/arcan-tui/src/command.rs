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
}
