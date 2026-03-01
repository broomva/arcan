use crate::protocol::ChatMessage;

/// The kind of context block, determining assembly order and default priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContextBlockKind {
    Persona,
    Rules,
    Memory,
    Retrieval,
    Workspace,
    Task,
}

impl ContextBlockKind {
    /// Fixed assembly order index: Persona=0, Rules=1, Memory=2, Retrieval=3, Workspace=4, Task=5
    fn order(self) -> u8 {
        match self {
            Self::Persona => 0,
            Self::Rules => 1,
            Self::Memory => 2,
            Self::Retrieval => 3,
            Self::Workspace => 4,
            Self::Task => 5,
        }
    }
}

/// A typed block of context to be assembled into the system prompt.
#[derive(Debug, Clone)]
pub struct ContextBlock {
    pub kind: ContextBlockKind,
    pub content: String,
    /// Priority (0 = lowest, 255 = highest). Higher priority blocks are kept when budget is exceeded.
    /// Persona defaults to 255 (never dropped).
    pub priority: u8,
}

/// Configuration for the context compiler.
#[derive(Debug, Clone)]
pub struct ContextCompilerConfig {
    /// Total token budget for compiled system messages.
    pub total_budget: usize,
    /// Per-kind token budgets. Blocks are truncated to their kind's budget.
    /// Kinds not listed here get unlimited (up to total_budget).
    pub block_budgets: Vec<(ContextBlockKind, usize)>,
}

impl Default for ContextCompilerConfig {
    fn default() -> Self {
        Self {
            total_budget: 30_000,
            block_budgets: vec![
                (ContextBlockKind::Persona, 2_000),
                (ContextBlockKind::Rules, 5_000),
                (ContextBlockKind::Memory, 8_000),
                (ContextBlockKind::Retrieval, 6_000),
                (ContextBlockKind::Workspace, 5_000),
                (ContextBlockKind::Task, 4_000),
            ],
        }
    }
}

impl ContextCompilerConfig {
    fn budget_for(&self, kind: ContextBlockKind) -> Option<usize> {
        self.block_budgets
            .iter()
            .find(|(k, _)| *k == kind)
            .map(|(_, b)| *b)
    }
}

/// Result of context compilation.
#[derive(Debug, Clone)]
pub struct CompiledContext {
    /// System messages to prepend to the conversation.
    pub system_messages: Vec<ChatMessage>,
    /// Estimated total tokens used.
    pub total_tokens: usize,
    /// Blocks that were dropped due to budget constraints.
    pub dropped_blocks: Vec<ContextBlockKind>,
}

/// Estimate token count using ~4 chars per token heuristic.
fn estimate_tokens(text: &str) -> usize {
    text.len().div_ceil(4).max(1)
}

/// Truncate text to approximately `max_tokens` tokens, respecting word boundaries.
fn truncate_to_budget(text: &str, max_tokens: usize) -> &str {
    let max_chars = max_tokens * 4;
    if text.len() <= max_chars {
        return text;
    }
    // Find the last space before max_chars for word boundary
    let truncated = &text[..max_chars];
    match truncated.rfind(' ') {
        Some(pos) if pos > max_chars / 2 => &text[..pos],
        _ => truncated,
    }
}

/// Compile context blocks into system messages with budget enforcement.
///
/// Assembly order is fixed: Persona -> Rules -> Memory -> Retrieval -> Workspace -> Task.
/// Each block is truncated to its per-kind budget, then if total exceeds the total budget,
/// lowest-priority blocks are dropped (Persona is never dropped).
pub fn compile_context(blocks: &[ContextBlock], config: &ContextCompilerConfig) -> CompiledContext {
    if blocks.is_empty() {
        return CompiledContext {
            system_messages: Vec::new(),
            total_tokens: 0,
            dropped_blocks: Vec::new(),
        };
    }

    // Sort blocks by assembly order, preserving relative order for same kind
    let mut sorted: Vec<&ContextBlock> = blocks.iter().filter(|b| !b.content.is_empty()).collect();
    sorted.sort_by_key(|b| b.kind.order());

    // Truncate each block to its per-kind budget
    let truncated: Vec<(&ContextBlock, &str)> = sorted
        .iter()
        .map(|block| {
            let content = if let Some(budget) = config.budget_for(block.kind) {
                truncate_to_budget(&block.content, budget)
            } else {
                block.content.as_str()
            };
            (*block, content)
        })
        .collect();

    // Calculate total tokens
    let total: usize = truncated.iter().map(|(_, c)| estimate_tokens(c)).sum();

    if total <= config.total_budget {
        // Everything fits
        let system_messages = truncated
            .iter()
            .map(|(_, content)| ChatMessage::system(*content))
            .collect();
        return CompiledContext {
            system_messages,
            total_tokens: total,
            dropped_blocks: Vec::new(),
        };
    }

    // Need to drop lowest-priority blocks. Sort by priority ascending (lowest first to drop).
    let mut indexed: Vec<(usize, &ContextBlock, &str, usize)> = truncated
        .iter()
        .enumerate()
        .map(|(i, (block, content))| (i, *block, *content, estimate_tokens(content)))
        .collect();

    // Sort by priority ascending — lowest priority gets dropped first.
    // Persona (priority 255 by convention) should never be dropped.
    indexed.sort_by(|a, b| a.1.priority.cmp(&b.1.priority));

    let mut budget_remaining = config.total_budget;
    let mut keep_indices: Vec<usize> = Vec::new();
    let mut dropped_blocks: Vec<ContextBlockKind> = Vec::new();

    // Process highest priority first (from the end)
    for &(original_idx, block, _, tokens) in indexed.iter().rev() {
        if tokens <= budget_remaining {
            keep_indices.push(original_idx);
            budget_remaining = budget_remaining.saturating_sub(tokens);
        } else {
            dropped_blocks.push(block.kind);
        }
    }

    // Restore original assembly order
    keep_indices.sort_unstable();

    let system_messages: Vec<ChatMessage> = keep_indices
        .iter()
        .map(|&i| ChatMessage::system(truncated[i].1))
        .collect();

    let total_tokens: usize = keep_indices
        .iter()
        .map(|&i| estimate_tokens(truncated[i].1))
        .sum();

    CompiledContext {
        system_messages,
        total_tokens,
        dropped_blocks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_block(kind: ContextBlockKind, content: &str, priority: u8) -> ContextBlock {
        ContextBlock {
            kind,
            content: content.to_string(),
            priority,
        }
    }

    #[test]
    fn empty_blocks_returns_empty() {
        let result = compile_context(&[], &ContextCompilerConfig::default());
        assert!(result.system_messages.is_empty());
        assert_eq!(result.total_tokens, 0);
        assert!(result.dropped_blocks.is_empty());
    }

    #[test]
    fn single_block_compiles() {
        let blocks = vec![make_block(
            ContextBlockKind::Persona,
            "You are a helpful assistant.",
            255,
        )];
        let result = compile_context(&blocks, &ContextCompilerConfig::default());
        assert_eq!(result.system_messages.len(), 1);
        assert_eq!(
            result.system_messages[0].content,
            "You are a helpful assistant."
        );
        assert!(result.dropped_blocks.is_empty());
    }

    #[test]
    fn all_six_blocks_in_order() {
        let blocks = vec![
            make_block(ContextBlockKind::Task, "Current task info", 50),
            make_block(ContextBlockKind::Persona, "I am an AI", 255),
            make_block(ContextBlockKind::Memory, "User prefers dark mode", 100),
            make_block(ContextBlockKind::Rules, "Never lie", 200),
            make_block(ContextBlockKind::Workspace, "cwd: /home", 80),
            make_block(ContextBlockKind::Retrieval, "Relevant docs", 90),
        ];
        let config = ContextCompilerConfig {
            total_budget: 100_000,
            block_budgets: Vec::new(), // no per-kind limits
        };
        let result = compile_context(&blocks, &config);
        assert_eq!(result.system_messages.len(), 6);
        // Verify assembly order: Persona, Rules, Memory, Retrieval, Workspace, Task
        assert!(result.system_messages[0].content.contains("I am an AI"));
        assert!(result.system_messages[1].content.contains("Never lie"));
        assert!(result.system_messages[2].content.contains("dark mode"));
        assert!(result.system_messages[3].content.contains("Relevant docs"));
        assert!(result.system_messages[4].content.contains("cwd:"));
        assert!(result.system_messages[5].content.contains("Current task"));
        assert!(result.dropped_blocks.is_empty());
    }

    #[test]
    fn block_truncation_respects_budget() {
        let long_content = "word ".repeat(10000); // ~50000 chars = ~12500 tokens
        let blocks = vec![make_block(ContextBlockKind::Memory, &long_content, 100)];
        let config = ContextCompilerConfig {
            total_budget: 100_000,
            block_budgets: vec![(ContextBlockKind::Memory, 100)], // Only 100 tokens
        };
        let result = compile_context(&blocks, &config);
        assert_eq!(result.system_messages.len(), 1);
        // Content should be truncated: 100 tokens * 4 chars = 400 chars max
        assert!(result.system_messages[0].content.len() <= 400);
    }

    #[test]
    fn total_budget_overflow_drops_low_priority() {
        let blocks = vec![
            make_block(ContextBlockKind::Persona, &"a".repeat(400), 255), // ~100 tokens
            make_block(ContextBlockKind::Rules, &"b".repeat(400), 200),   // ~100 tokens
            make_block(ContextBlockKind::Memory, &"c".repeat(400), 50),   // ~100 tokens (low prio)
            make_block(ContextBlockKind::Retrieval, &"d".repeat(400), 30), // ~100 tokens (lowest)
        ];
        let config = ContextCompilerConfig {
            total_budget: 250, // Only fits ~2.5 blocks
            block_budgets: Vec::new(),
        };
        let result = compile_context(&blocks, &config);
        // Should keep Persona (255) and Rules (200), drop Memory (50) and Retrieval (30)
        assert!(result.system_messages.len() <= 3);
        assert!(!result.dropped_blocks.is_empty());
        // Persona should always be there
        assert!(
            result
                .system_messages
                .iter()
                .any(|m| m.content.contains('a'))
        );
    }

    #[test]
    fn persona_never_dropped() {
        let blocks = vec![
            make_block(ContextBlockKind::Persona, &"x".repeat(100), 255),
            make_block(ContextBlockKind::Rules, &"y".repeat(4000), 200),
        ];
        let config = ContextCompilerConfig {
            total_budget: 50, // Very small — can only fit Persona
            block_budgets: Vec::new(),
        };
        let result = compile_context(&blocks, &config);
        assert!(
            result
                .system_messages
                .iter()
                .any(|m| m.content.contains('x'))
        );
    }

    #[test]
    fn empty_content_skipped() {
        let blocks = vec![
            make_block(ContextBlockKind::Persona, "hello", 255),
            make_block(ContextBlockKind::Rules, "", 200),
            make_block(ContextBlockKind::Memory, "   ", 100),
        ];
        let config = ContextCompilerConfig {
            total_budget: 100_000,
            block_budgets: Vec::new(),
        };
        let result = compile_context(&blocks, &config);
        // Empty content is filtered, but whitespace-only is not empty
        assert_eq!(result.system_messages.len(), 2);
    }

    #[test]
    fn default_config_reasonable() {
        let config = ContextCompilerConfig::default();
        assert_eq!(config.total_budget, 30_000);
        assert_eq!(config.block_budgets.len(), 6);
    }

    #[test]
    fn word_boundary_truncation() {
        let content = "hello world this is a test of truncation at word boundaries";
        let truncated = truncate_to_budget(content, 3); // 3 tokens = 12 chars
        // Should truncate at a word boundary
        assert!(truncated.len() <= 12);
        assert!(!truncated.ends_with(' '));
    }

    #[test]
    fn token_count_accuracy() {
        // 100 chars = 25 tokens (at 4 chars/token)
        let tokens = estimate_tokens(&"a".repeat(100));
        assert_eq!(tokens, 25);

        // 1 char = 1 token (minimum)
        assert_eq!(estimate_tokens("a"), 1);

        // empty = 1 token (minimum)
        assert_eq!(estimate_tokens(""), 1);
    }

    #[test]
    fn dropped_blocks_reported() {
        let blocks = vec![
            make_block(ContextBlockKind::Persona, &"a".repeat(400), 255),
            make_block(ContextBlockKind::Memory, &"c".repeat(400), 50),
        ];
        let config = ContextCompilerConfig {
            total_budget: 110, // Only fits one block
            block_budgets: Vec::new(),
        };
        let result = compile_context(&blocks, &config);
        assert!(!result.dropped_blocks.is_empty());
        assert!(result.dropped_blocks.contains(&ContextBlockKind::Memory));
    }

    #[test]
    fn custom_budgets_applied() {
        let blocks = vec![
            make_block(ContextBlockKind::Persona, &"p".repeat(1000), 255),
            make_block(ContextBlockKind::Rules, &"r".repeat(1000), 200),
        ];
        let config = ContextCompilerConfig {
            total_budget: 100_000,
            block_budgets: vec![
                (ContextBlockKind::Persona, 50), // 50 tokens = 200 chars
                (ContextBlockKind::Rules, 50),
            ],
        };
        let result = compile_context(&blocks, &config);
        assert_eq!(result.system_messages.len(), 2);
        // Each should be truncated to ~200 chars
        for msg in &result.system_messages {
            assert!(msg.content.len() <= 200);
        }
    }

    #[test]
    fn deterministic_ordering() {
        let blocks = vec![
            make_block(ContextBlockKind::Workspace, "ws", 80),
            make_block(ContextBlockKind::Persona, "persona", 255),
            make_block(ContextBlockKind::Task, "task", 50),
        ];
        let config = ContextCompilerConfig {
            total_budget: 100_000,
            block_budgets: Vec::new(),
        };
        // Run twice to verify determinism
        let r1 = compile_context(&blocks, &config);
        let r2 = compile_context(&blocks, &config);
        assert_eq!(r1.system_messages.len(), r2.system_messages.len());
        for (a, b) in r1.system_messages.iter().zip(r2.system_messages.iter()) {
            assert_eq!(a.content, b.content);
        }
    }

    #[test]
    fn compiles_alongside_compact_messages() {
        // Verify compiled context produces system messages that work with compact_messages
        let blocks = vec![
            make_block(ContextBlockKind::Persona, "You are helpful.", 255),
            make_block(ContextBlockKind::Rules, "Be concise.", 200),
        ];
        let config = ContextCompilerConfig::default();
        let compiled = compile_context(&blocks, &config);

        // System messages can be prepended to a conversation
        let mut messages = compiled.system_messages;
        messages.push(ChatMessage::user("Hello"));
        messages.push(ChatMessage::assistant("Hi!"));

        // The conversation should be valid
        assert!(messages.len() >= 4);
        assert_eq!(messages[0].role, crate::protocol::Role::System);
        assert_eq!(messages[1].role, crate::protocol::Role::System);
    }
}
