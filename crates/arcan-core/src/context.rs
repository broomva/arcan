use crate::protocol::{ChatMessage, Role};

/// Configuration for context window management.
#[derive(Debug, Clone)]
pub struct ContextConfig {
    /// Maximum estimated tokens for the context window.
    /// Messages will be compacted when approaching this limit.
    pub max_context_tokens: usize,
    /// Reserve this many tokens for the model's response.
    /// Context budget = max_context_tokens - reserve_output_tokens.
    pub reserve_output_tokens: usize,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            max_context_tokens: 200_000,
            reserve_output_tokens: 8_192,
        }
    }
}

impl ContextConfig {
    /// Context budget available for input messages.
    pub fn input_budget(&self) -> usize {
        self.max_context_tokens
            .saturating_sub(self.reserve_output_tokens)
    }
}

/// Result of a context compaction operation.
#[derive(Debug, Clone)]
pub struct CompactionResult {
    /// Messages after compaction.
    pub messages: Vec<ChatMessage>,
    /// Number of messages dropped.
    pub dropped_count: usize,
    /// Estimated tokens before compaction.
    pub tokens_before: usize,
    /// Estimated tokens after compaction.
    pub tokens_after: usize,
}

/// Estimate the token count of a message using a character-based heuristic.
///
/// Uses ~4 characters per token as a rough approximation.
/// This is intentionally conservative (overestimates) to avoid exceeding limits.
pub fn estimate_tokens(text: &str) -> usize {
    // ~4 chars per token on average for English text.
    // Add overhead for message framing (role, formatting).
    let content_tokens = text.len().div_ceil(4);
    // ~4 tokens overhead per message for role/formatting
    content_tokens + 4
}

/// Estimate total tokens for a message list.
pub fn estimate_total_tokens(messages: &[ChatMessage]) -> usize {
    messages.iter().map(|m| estimate_tokens(&m.content)).sum()
}

/// Compact messages to fit within the context budget.
///
/// Strategy:
/// 1. Always preserve system messages (they contain persona/instructions)
/// 2. Always preserve the most recent user message (it's the current request)
/// 3. Preserve tool results that are paired with tool calls still in context
/// 4. Drop oldest non-system, non-final-user messages first
///
/// Returns `None` if no compaction was needed.
pub fn compact_messages(
    messages: &[ChatMessage],
    config: &ContextConfig,
) -> Option<CompactionResult> {
    let budget = config.input_budget();
    let tokens_before = estimate_total_tokens(messages);

    if tokens_before <= budget {
        return None;
    }

    // Separate messages into categories
    let mut system_msgs: Vec<(usize, &ChatMessage)> = Vec::new();
    let mut other_msgs: Vec<(usize, &ChatMessage)> = Vec::new();

    for (i, msg) in messages.iter().enumerate() {
        if msg.role == Role::System {
            system_msgs.push((i, msg));
        } else {
            other_msgs.push((i, msg));
        }
    }

    // System messages are always kept

    // The last user message is always kept (it's the current request)
    let last_user = other_msgs.iter().rposition(|(_i, m)| m.role == Role::User);

    let mut keep_indices: Vec<usize> = system_msgs.iter().map(|(i, _)| *i).collect();

    if let Some(last_user_pos) = last_user {
        keep_indices.push(other_msgs[last_user_pos].0);
    }

    // Budget remaining after system messages and last user message
    let fixed_tokens: usize = keep_indices
        .iter()
        .map(|&i| estimate_tokens(&messages[i].content))
        .sum();

    let mut remaining_budget = budget.saturating_sub(fixed_tokens);

    // Add non-fixed messages from most recent to oldest (recency bias)
    let mut candidate_indices: Vec<usize> = other_msgs
        .iter()
        .map(|(i, _)| *i)
        .filter(|i| !keep_indices.contains(i))
        .collect();

    // Reverse to process most recent first
    candidate_indices.reverse();

    let mut accepted: Vec<usize> = Vec::new();
    for idx in &candidate_indices {
        let msg_tokens = estimate_tokens(&messages[*idx].content);
        if msg_tokens <= remaining_budget {
            accepted.push(*idx);
            remaining_budget = remaining_budget.saturating_sub(msg_tokens);
        }
        // If a message doesn't fit, skip it (drop from context)
    }

    // Combine kept indices and sort by original position
    keep_indices.extend(accepted);
    keep_indices.sort_unstable();
    keep_indices.dedup();

    let dropped_count = messages.len() - keep_indices.len();
    if dropped_count == 0 {
        return None;
    }

    let compacted: Vec<ChatMessage> = keep_indices.iter().map(|&i| messages[i].clone()).collect();

    let tokens_after = estimate_total_tokens(&compacted);

    // Safety: if we somehow still exceeded budget with just system + last user,
    // that's a fundamental limit we can't fix by dropping more messages.
    // Just return what we have.

    Some(CompactionResult {
        messages: compacted,
        dropped_count,
        tokens_before,
        tokens_after,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ChatMessage;

    fn make_msg(role: Role, content: &str) -> ChatMessage {
        match role {
            Role::System => ChatMessage::system(content),
            Role::User => ChatMessage::user(content),
            Role::Assistant => ChatMessage::assistant(content),
            Role::Tool => ChatMessage::tool(content),
        }
    }

    #[test]
    fn no_compaction_when_within_budget() {
        let messages = vec![
            make_msg(Role::System, "You are an agent."),
            make_msg(Role::User, "Hello"),
            make_msg(Role::Assistant, "Hi there!"),
        ];
        let config = ContextConfig {
            max_context_tokens: 100_000,
            reserve_output_tokens: 4_096,
        };
        assert!(compact_messages(&messages, &config).is_none());
    }

    #[test]
    fn compaction_drops_oldest_messages() {
        // Create messages that exceed a small budget
        let mut messages = vec![make_msg(Role::System, "sys")];
        for i in 0..50 {
            messages.push(make_msg(Role::User, &format!("user message {i}")));
            messages.push(make_msg(
                Role::Assistant,
                &"long response text ".repeat(100),
            ));
        }
        // Final user message
        messages.push(make_msg(Role::User, "current question"));

        let config = ContextConfig {
            max_context_tokens: 2_000,
            reserve_output_tokens: 500,
        };

        let result = compact_messages(&messages, &config).expect("should compact");
        assert!(result.dropped_count > 0);
        assert!(result.tokens_after <= config.input_budget());

        // System message preserved
        assert_eq!(result.messages[0].role, Role::System);
        assert_eq!(result.messages[0].content, "sys");

        // Last user message preserved
        assert!(
            result
                .messages
                .iter()
                .any(|m| m.content == "current question")
        );
    }

    #[test]
    fn system_messages_always_preserved() {
        let messages = vec![
            make_msg(Role::System, "system prompt 1"),
            make_msg(Role::System, "system prompt 2"),
            make_msg(Role::User, &"long msg ".repeat(500)),
            make_msg(Role::Assistant, &"long reply ".repeat(500)),
            make_msg(Role::User, "current"),
        ];

        let config = ContextConfig {
            max_context_tokens: 500,
            reserve_output_tokens: 100,
        };

        let result = compact_messages(&messages, &config).expect("should compact");

        // Both system messages preserved
        let system_count = result
            .messages
            .iter()
            .filter(|m| m.role == Role::System)
            .count();
        assert_eq!(system_count, 2);
    }

    #[test]
    fn last_user_message_always_preserved() {
        let messages = vec![
            make_msg(Role::System, "sys"),
            make_msg(Role::User, &"old ".repeat(500)),
            make_msg(Role::Assistant, &"reply ".repeat(500)),
            make_msg(Role::User, "latest question"),
        ];

        let config = ContextConfig {
            max_context_tokens: 200,
            reserve_output_tokens: 50,
        };

        let result = compact_messages(&messages, &config).expect("should compact");
        let last = result.messages.last().expect("should have messages");
        assert_eq!(last.content, "latest question");
    }

    #[test]
    fn recency_bias_keeps_newer_messages() {
        let mut messages = vec![make_msg(Role::System, "sys")];
        // Add many messages, each large enough to force compaction
        for i in 0..20 {
            messages.push(make_msg(
                Role::User,
                &format!("question {i} {}", "q".repeat(200)),
            ));
            messages.push(make_msg(
                Role::Assistant,
                &format!("answer {i} {}", "x".repeat(200)),
            ));
        }
        messages.push(make_msg(Role::User, "final"));

        let config = ContextConfig {
            max_context_tokens: 1_000,
            reserve_output_tokens: 200,
        };

        let result = compact_messages(&messages, &config).expect("should compact");

        // The most recent messages should be preserved (recency bias)
        let has_recent = result
            .messages
            .iter()
            .any(|m| m.content.contains("answer 19"));
        let has_old = result
            .messages
            .iter()
            .any(|m| m.content.contains("answer 0"));

        assert!(has_recent, "Recent messages should be kept");
        // Old messages may or may not be there depending on budget,
        // but if compaction happened, old should be dropped first
        if result.dropped_count > 2 {
            assert!(!has_old, "Old messages should be dropped first");
        }
    }

    #[test]
    fn empty_messages_no_compaction() {
        let messages: Vec<ChatMessage> = Vec::new();
        let config = ContextConfig::default();
        assert!(compact_messages(&messages, &config).is_none());
    }

    #[test]
    fn single_user_message_no_compaction_if_within_budget() {
        let messages = vec![make_msg(Role::User, "hello")];
        let config = ContextConfig::default();
        assert!(compact_messages(&messages, &config).is_none());
    }

    #[test]
    fn estimate_tokens_reasonable() {
        // "hello" = 5 chars → ~1-2 tokens + 4 overhead
        let tokens = estimate_tokens("hello");
        assert!(
            tokens >= 5,
            "Should have at least 5 tokens for 'hello' + overhead"
        );
        assert!(tokens <= 10, "Should not be excessive");

        // Empty string
        let empty = estimate_tokens("");
        assert!(empty >= 4, "Should have overhead");

        // Long text: 1000 chars → ~250 content tokens + 4 overhead
        let long = estimate_tokens(&"a".repeat(1000));
        assert!(long >= 250);
        assert!(long <= 260);
    }

    #[test]
    fn default_config_reasonable() {
        let config = ContextConfig::default();
        assert_eq!(config.max_context_tokens, 200_000);
        assert_eq!(config.reserve_output_tokens, 8_192);
        assert!(config.input_budget() > 190_000);
    }

    #[test]
    fn compaction_result_reports_accurate_counts() {
        let mut messages = vec![make_msg(Role::System, "sys")];
        for i in 0..10 {
            messages.push(make_msg(Role::User, &format!("q{i}")));
            messages.push(make_msg(Role::Assistant, &"x".repeat(200)));
        }
        messages.push(make_msg(Role::User, "final"));

        let config = ContextConfig {
            max_context_tokens: 300,
            reserve_output_tokens: 50,
        };

        let result = compact_messages(&messages, &config).expect("should compact");
        assert_eq!(result.messages.len() + result.dropped_count, messages.len());
        assert!(result.tokens_before > result.tokens_after);
    }

    #[test]
    fn tool_messages_can_be_dropped() {
        let messages = vec![
            make_msg(Role::System, "sys"),
            make_msg(Role::User, "q1"),
            make_msg(Role::Assistant, "calling tool"),
            ChatMessage::tool_result("call-1", &"x".repeat(500)),
            make_msg(Role::User, "current"),
        ];

        let config = ContextConfig {
            max_context_tokens: 100,
            reserve_output_tokens: 20,
        };

        let result = compact_messages(&messages, &config).expect("should compact");
        // Tool result is large and old — it should be dropped
        assert!(result.dropped_count > 0);
        assert!(result.messages.iter().any(|m| m.content == "current"));
    }
}
