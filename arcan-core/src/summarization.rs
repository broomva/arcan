//! Automatic context compression for long agent sessions (BRO-425).
//!
//! Long tool-driven sessions accumulate large tool results and many turns,
//! eventually crowding the model's context window and pushing valuable
//! early-session decisions out. This module adds a [`SummarizationMiddleware`]
//! to the turn middleware chain that, when the estimated context exceeds a
//! configurable token threshold, **compresses older turns into a summary while
//! preserving the most recent N turns at full fidelity**.
//!
//! Design mirrors DeerFlow's `SummarizationMiddleware` (summarize completed
//! sub-tasks) and Hermes' `context_compressor.py`, adapted to Life's typed
//! context compiler ([`crate::context_compiler::ContextBlockKind::Compressed`]).
//!
//! ## Nothing is ever lost
//!
//! Compression happens on the **per-call [`ProviderRequest`]**, whose
//! `messages` are a *clone* of the orchestrator's durable history. The
//! canonical message log — and therefore the Lago event journal, which is
//! upstream of everything here — is never mutated. The full history stays
//! replayable via `lago log` / `lago replay`; only the copy handed to the
//! model for a single call is compressed. This is the seam that makes
//! "full history always retrievable from Lago" hold by construction.
//!
//! ## What gets compressed
//!
//! * **System messages** are always kept verbatim (persona / rules / prior
//!   summaries live here).
//! * The **most recent `recent_turns` turns** are kept verbatim. A "turn" is a
//!   `User` message and everything following it up to the next `User` message,
//!   matching how a human reads a conversation.
//! * **Older turns** are folded into a single compressed summary system
//!   message ([`COMPRESSED_SUMMARY_HEADER`]), produced by a pluggable
//!   [`Summarizer`]. The default [`HeuristicSummarizer`] is deterministic and
//!   LLM-free; an LLM-backed summarizer can be dropped in behind the same
//!   trait (it should fall back to a heuristic on error so the middleware
//!   stays non-blocking / fail-open).

use crate::context::estimate_total_tokens;
use crate::context_compiler::{ContextBlock, ContextBlockKind};
use crate::error::CoreError;
use crate::protocol::{ChatMessage, Role};
use crate::runtime::{ProviderRequest, TurnMiddleware};
use std::sync::Arc;

/// Header prefixing a compressed conversation-history summary message.
///
/// Callers (and tests) can identify a synthesized summary by this marker; it
/// distinguishes middleware-produced summaries from genuine dialogue.
pub const COMPRESSED_SUMMARY_HEADER: &str = "[compressed-summary]";

/// Configuration for [`SummarizationMiddleware`].
///
/// Every threshold called out in the ticket's acceptance criteria is
/// configurable here: the token limit that triggers compression, the number of
/// recent turns preserved, and the per-message size above which a single
/// message (typically a large tool result) is truncated inside the summary.
#[derive(Debug, Clone)]
pub struct SummarizationConfig {
    /// Total estimated context tokens above which older turns are compressed.
    /// Below this, the middleware is a no-op.
    pub token_threshold: usize,
    /// Number of most-recent turns kept at full fidelity (never compressed).
    pub recent_turns: usize,
    /// A single message longer than this many characters is truncated to this
    /// length inside the compressed summary. Bounds the summary size when older
    /// turns contain very large tool results.
    pub result_char_threshold: usize,
}

impl Default for SummarizationConfig {
    fn default() -> Self {
        Self {
            // ~120k of a 200k window: leave generous headroom for the recent
            // window plus the model's output reservation.
            token_threshold: 120_000,
            recent_turns: 10,
            result_char_threshold: 8_000,
        }
    }
}

impl SummarizationConfig {
    /// Validate/normalize the config, clamping nonsensical values to safe ones.
    fn normalized(&self) -> Self {
        Self {
            token_threshold: self.token_threshold,
            // Always keep at least the current turn verbatim.
            recent_turns: self.recent_turns.max(1),
            // Keep enough of each message to preserve a leading gist.
            result_char_threshold: self.result_char_threshold.max(64),
        }
    }
}

/// Produces a compressed textual summary of a slice of older conversation
/// messages.
///
/// Implementations may call an LLM (separate from the main agent loop) or apply
/// a deterministic heuristic — the middleware is agnostic. `summarize` is
/// infallible by design: an LLM-backed implementation must catch its own errors
/// and fall back to a heuristic so the middleware never blocks a model call.
pub trait Summarizer: Send + Sync {
    /// Summarize `messages` (older turns) into a compact string capturing the
    /// gist and any key decisions. `result_char_threshold` bounds how much of
    /// any single message the summary should retain.
    fn summarize(&self, messages: &[ChatMessage], result_char_threshold: usize) -> String;
}

/// Deterministic, LLM-free summarizer.
///
/// Retains a truncated leading slice of every compressed message (so concisely
/// stated early decisions survive), tags each with its role, and notes tool
/// activity. Being deterministic makes it cheap and unit-testable; it is the
/// default so compression never depends on an external service being reachable.
#[derive(Debug, Clone, Default)]
pub struct HeuristicSummarizer;

impl HeuristicSummarizer {
    /// Truncate `text` to at most `max_chars`, on a char boundary, collapsing
    /// interior newlines so each message stays on one summary line.
    fn gist(text: &str, max_chars: usize) -> String {
        let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if one_line.chars().count() <= max_chars {
            return one_line;
        }
        let truncated: String = one_line.chars().take(max_chars).collect();
        format!("{truncated}…")
    }
}

impl Summarizer for HeuristicSummarizer {
    fn summarize(&self, messages: &[ChatMessage], result_char_threshold: usize) -> String {
        // Per-message gist cap: bounded by the result threshold but kept modest
        // so the summary itself stays small even for many messages.
        let gist_cap = result_char_threshold.min(240);

        let mut lines: Vec<String> = Vec::with_capacity(messages.len() + 1);
        let mut tool_calls = 0usize;
        for msg in messages {
            let (label, body) = match msg.role {
                Role::System => ("system", msg.content.as_str()),
                Role::User => ("user", msg.content.as_str()),
                Role::Assistant => ("assistant", msg.content.as_str()),
                Role::Tool => {
                    tool_calls += 1;
                    ("tool-result", msg.content.as_str())
                }
            };
            if body.trim().is_empty() {
                continue;
            }
            lines.push(format!("- {label}: {}", Self::gist(body, gist_cap)));
        }

        let header = format!(
            "Summary of {} earlier message(s) ({} tool result(s)), compressed to \
             preserve key context. Full detail remains in the event journal.",
            messages.len(),
            tool_calls,
        );

        let mut out =
            String::with_capacity(header.len() + lines.iter().map(|l| l.len() + 1).sum::<usize>());
        out.push_str(&header);
        for line in lines {
            out.push('\n');
            out.push_str(&line);
        }
        out
    }
}

/// A turn: a `User` message and everything that follows it up to (but not
/// including) the next `User` message. Messages before the first `User` message
/// that are *not* system messages form a leading turn of their own.
fn split_into_turns(messages: &[ChatMessage]) -> Vec<Vec<ChatMessage>> {
    let mut turns: Vec<Vec<ChatMessage>> = Vec::new();
    let mut current: Vec<ChatMessage> = Vec::new();
    for msg in messages {
        if msg.role == Role::User && !current.is_empty() {
            turns.push(std::mem::take(&mut current));
        }
        current.push(msg.clone());
    }
    if !current.is_empty() {
        turns.push(current);
    }
    turns
}

/// Build the compressed conversation-history [`ContextBlock`] from a summary
/// string, for callers assembling the system prompt through
/// [`crate::context_compiler::compile_context`].
///
/// This is the context-compiler counterpart to the message-window compression
/// [`SummarizationMiddleware`] performs: both express the same "compressed
/// older history" concept, one as a typed block, the other as an inline
/// summary message.
pub fn compressed_block(summary: impl Into<String>, priority: u8) -> ContextBlock {
    ContextBlock {
        kind: ContextBlockKind::Compressed,
        content: format!("{COMPRESSED_SUMMARY_HEADER}\n{}", summary.into()),
        priority,
    }
}

/// Turn middleware that compresses older conversation turns when the estimated
/// context exceeds [`SummarizationConfig::token_threshold`].
///
/// Wired into any `Vec<Arc<dyn TurnMiddleware>>` (e.g. via
/// [`crate::runtime::Orchestrator::with_turn_middlewares`]). Runs in
/// `before_model_call`, mutating only the per-call request.
pub struct SummarizationMiddleware {
    config: SummarizationConfig,
    summarizer: Arc<dyn Summarizer>,
}

impl SummarizationMiddleware {
    /// Construct with an explicit config and the default heuristic summarizer.
    pub fn new(config: SummarizationConfig) -> Self {
        Self {
            config: config.normalized(),
            summarizer: Arc::new(HeuristicSummarizer),
        }
    }

    /// Construct with default config and the default heuristic summarizer.
    pub fn with_defaults() -> Self {
        Self::new(SummarizationConfig::default())
    }

    /// Construct with an explicit config and a custom (e.g. LLM-backed)
    /// summarizer.
    pub fn with_summarizer(config: SummarizationConfig, summarizer: Arc<dyn Summarizer>) -> Self {
        Self {
            config: config.normalized(),
            summarizer,
        }
    }

    /// Compress a message list per this middleware's config. Returns `None`
    /// when no compression is warranted (under threshold, or too few turns to
    /// compress). Pure and side-effect free — the core of the middleware,
    /// exposed for testing and for callers that want compression without the
    /// middleware plumbing.
    pub fn compress(&self, messages: &[ChatMessage]) -> Option<CompressionOutcome> {
        let tokens_before = estimate_total_tokens(messages);
        if tokens_before <= self.config.token_threshold {
            return None;
        }

        // System messages are always preserved verbatim, in order, at the front.
        let system_msgs: Vec<ChatMessage> = messages
            .iter()
            .filter(|m| m.role == Role::System)
            .cloned()
            .collect();
        let conversation: Vec<ChatMessage> = messages
            .iter()
            .filter(|m| m.role != Role::System)
            .cloned()
            .collect();

        let turns = split_into_turns(&conversation);
        // Need at least one older turn beyond the preserved recent window to
        // have anything to compress.
        if turns.len() <= self.config.recent_turns {
            return None;
        }

        let split_at = turns.len() - self.config.recent_turns;
        let older: Vec<ChatMessage> = turns[..split_at].iter().flatten().cloned().collect();
        let recent: Vec<ChatMessage> = turns[split_at..].iter().flatten().cloned().collect();

        if older.is_empty() {
            return None;
        }

        let summary_text = self
            .summarizer
            .summarize(&older, self.config.result_char_threshold);
        let summary_msg =
            ChatMessage::system(format!("{COMPRESSED_SUMMARY_HEADER}\n{summary_text}"));

        // Rebuild: [system messages...] + [compressed summary] + [recent turns].
        // Keeping the summary in the system block (which the downstream
        // compactor also always preserves) means the recent window still opens
        // with a User message → valid role alternation for providers.
        let mut rebuilt = system_msgs;
        rebuilt.push(summary_msg);
        rebuilt.extend(recent);

        let tokens_after = estimate_total_tokens(&rebuilt);
        // If summarization somehow didn't shrink the context (e.g. tiny
        // history, pathological config), don't bother rewriting.
        if tokens_after >= tokens_before {
            return None;
        }

        Some(CompressionOutcome {
            messages: rebuilt,
            compressed_turns: split_at,
            tokens_before,
            tokens_after,
        })
    }
}

/// Result of a compression pass.
#[derive(Debug, Clone)]
pub struct CompressionOutcome {
    /// Messages after compression (system + summary + recent window).
    pub messages: Vec<ChatMessage>,
    /// Number of older turns folded into the summary.
    pub compressed_turns: usize,
    /// Estimated tokens before compression.
    pub tokens_before: usize,
    /// Estimated tokens after compression.
    pub tokens_after: usize,
}

impl TurnMiddleware for SummarizationMiddleware {
    fn before_model_call(&self, request: &mut ProviderRequest) -> Result<(), CoreError> {
        if let Some(outcome) = self.compress(&request.messages) {
            tracing::debug!(
                run_id = %request.run_id,
                session_id = %request.session_id,
                iteration = request.iteration,
                compressed_turns = outcome.compressed_turns,
                tokens_before = outcome.tokens_before,
                tokens_after = outcome.tokens_after,
                "SummarizationMiddleware compressed older turns",
            );
            request.messages = outcome.messages;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn big(text: &str, times: usize) -> String {
        format!("{text} {}", "x".repeat(times))
    }

    #[test]
    fn no_compression_under_threshold() {
        let mw = SummarizationMiddleware::new(SummarizationConfig {
            token_threshold: 1_000_000,
            recent_turns: 2,
            result_char_threshold: 500,
        });
        let messages = vec![
            ChatMessage::system("persona"),
            ChatMessage::user("hello"),
            ChatMessage::assistant("hi"),
        ];
        assert!(mw.compress(&messages).is_none());
    }

    #[test]
    fn no_compression_when_few_turns() {
        // Over the token threshold, but only 2 turns and recent_turns=10 → nothing older to compress.
        let mw = SummarizationMiddleware::new(SummarizationConfig {
            token_threshold: 10,
            recent_turns: 10,
            result_char_threshold: 500,
        });
        let messages = vec![
            ChatMessage::user(big("q1", 400)),
            ChatMessage::assistant(big("a1", 400)),
        ];
        assert!(mw.compress(&messages).is_none());
    }

    #[test]
    fn split_into_turns_groups_by_user() {
        let messages = vec![
            ChatMessage::user("q1"),
            ChatMessage::assistant("a1"),
            ChatMessage::tool_result("c1", "r1"),
            ChatMessage::user("q2"),
            ChatMessage::assistant("a2"),
        ];
        let turns = split_into_turns(&messages);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].len(), 3);
        assert_eq!(turns[1].len(), 2);
    }

    #[test]
    fn compression_preserves_recent_turns_verbatim() {
        let mut messages = vec![ChatMessage::system("persona")];
        for i in 0..20 {
            messages.push(ChatMessage::user(big(&format!("question {i}"), 300)));
            messages.push(ChatMessage::assistant(big(&format!("answer {i}"), 300)));
        }
        let mw = SummarizationMiddleware::new(SummarizationConfig {
            token_threshold: 1_000,
            recent_turns: 3,
            result_char_threshold: 200,
        });
        let outcome = mw.compress(&messages).expect("should compress");

        // Compressed 20 - 3 = 17 older turns.
        assert_eq!(outcome.compressed_turns, 17);
        assert!(outcome.tokens_after < outcome.tokens_before);

        // System message preserved at the front.
        assert_eq!(outcome.messages[0].role, Role::System);
        assert_eq!(outcome.messages[0].content, "persona");
        // Summary present.
        assert!(
            outcome
                .messages
                .iter()
                .any(|m| m.content.contains(COMPRESSED_SUMMARY_HEADER))
        );
        // Most recent 3 turns preserved verbatim, in full.
        for i in 17..20 {
            let q = big(&format!("question {i}"), 300);
            let a = big(&format!("answer {i}"), 300);
            assert!(
                outcome.messages.iter().any(|m| m.content == q),
                "recent question {i} must survive verbatim"
            );
            assert!(
                outcome.messages.iter().any(|m| m.content == a),
                "recent answer {i} must survive verbatim"
            );
        }
    }

    /// Acceptance test: a long session with many tool calls compresses without
    /// losing key information.
    #[test]
    fn long_session_compresses_without_losing_key_information() {
        let mut messages = vec![ChatMessage::system("You are a build agent.")];

        // Early, load-bearing decisions the agent must not forget.
        messages.push(ChatMessage::user(
            "IMPORTANT: deploy target is port 8080 and the database is PostgreSQL.",
        ));
        messages.push(ChatMessage::assistant(
            "Decision noted: bind port 8080, use PostgreSQL for persistence.",
        ));

        // Many noisy tool calls with large results (the bulk of the context).
        for i in 0..40 {
            messages.push(ChatMessage::user(format!("run step {i}")));
            messages.push(ChatMessage::assistant(format!("calling tool for step {i}")));
            messages.push(ChatMessage::tool_result(
                format!("call-{i}"),
                big(&format!("verbose log output for step {i}"), 2_000),
            ));
        }

        // Current request.
        messages.push(ChatMessage::user("Now summarize the deployment config."));

        let tokens_before = estimate_total_tokens(&messages);

        let mw = SummarizationMiddleware::new(SummarizationConfig {
            token_threshold: 10_000,
            recent_turns: 5,
            result_char_threshold: 300,
        });
        let outcome = mw
            .compress(&messages)
            .expect("long session should compress");

        // Meaningfully smaller context.
        assert!(
            outcome.tokens_after < tokens_before / 2,
            "expected >2x reduction: {} -> {}",
            tokens_before,
            outcome.tokens_after
        );

        // Key early decisions survive in the compressed summary (they were
        // concise and near the start of their messages).
        let joined: String = outcome.messages.iter().map(|m| m.content.clone()).collect();
        assert!(joined.contains("8080"), "port decision must survive");
        assert!(
            joined.contains("PostgreSQL"),
            "database decision must survive"
        );

        // The current request is preserved verbatim (it is in the recent window).
        assert!(
            outcome
                .messages
                .iter()
                .any(|m| m.content == "Now summarize the deployment config."),
            "current request must survive"
        );

        // System prompt preserved.
        assert_eq!(outcome.messages[0].content, "You are a build agent.");
    }

    #[test]
    fn middleware_rewrites_request_messages() {
        let mut messages = vec![ChatMessage::system("sys")];
        for i in 0..30 {
            messages.push(ChatMessage::user(big(&format!("q{i}"), 200)));
            messages.push(ChatMessage::assistant(big(&format!("a{i}"), 200)));
        }
        let mw = SummarizationMiddleware::new(SummarizationConfig {
            token_threshold: 1_000,
            recent_turns: 4,
            result_char_threshold: 200,
        });

        let before = messages.len();
        let mut request = ProviderRequest {
            run_id: "r1".into(),
            session_id: "s1".into(),
            iteration: 3,
            messages,
            tools: Vec::new(),
            max_tokens: None,
            state: crate::state::AppState::default(),
        };
        mw.before_model_call(&mut request).expect("no error");
        assert!(
            request.messages.len() < before,
            "middleware should shrink the message list"
        );
        assert!(
            request
                .messages
                .iter()
                .any(|m| m.content.contains(COMPRESSED_SUMMARY_HEADER))
        );
    }

    #[test]
    fn custom_summarizer_is_used() {
        struct TaggingSummarizer;
        impl Summarizer for TaggingSummarizer {
            fn summarize(&self, _messages: &[ChatMessage], _threshold: usize) -> String {
                "CUSTOM-SUMMARY-MARKER".to_string()
            }
        }

        let mut messages = vec![ChatMessage::system("sys")];
        for i in 0..12 {
            messages.push(ChatMessage::user(big(&format!("q{i}"), 200)));
            messages.push(ChatMessage::assistant(big(&format!("a{i}"), 200)));
        }
        let mw = SummarizationMiddleware::with_summarizer(
            SummarizationConfig {
                token_threshold: 500,
                recent_turns: 2,
                result_char_threshold: 200,
            },
            Arc::new(TaggingSummarizer),
        );
        let outcome = mw.compress(&messages).expect("should compress");
        assert!(
            outcome
                .messages
                .iter()
                .any(|m| m.content.contains("CUSTOM-SUMMARY-MARKER"))
        );
    }

    #[test]
    fn compressed_block_has_right_kind_and_header() {
        let block = compressed_block("some summary", 120);
        assert_eq!(block.kind, ContextBlockKind::Compressed);
        assert_eq!(block.priority, 120);
        assert!(block.content.starts_with(COMPRESSED_SUMMARY_HEADER));
        assert!(block.content.contains("some summary"));
    }

    #[test]
    fn heuristic_gist_truncates_long_text() {
        let long = "a".repeat(1000);
        let g = HeuristicSummarizer::gist(&long, 100);
        // 100 chars + ellipsis.
        assert!(g.chars().count() <= 101);
        assert!(g.ends_with('…'));
    }

    #[test]
    fn heuristic_gist_collapses_whitespace() {
        let g = HeuristicSummarizer::gist("hello\n\n   world\ttab", 100);
        assert_eq!(g, "hello world tab");
    }

    #[test]
    fn recent_turns_clamped_to_at_least_one() {
        let mw = SummarizationMiddleware::new(SummarizationConfig {
            token_threshold: 100,
            recent_turns: 0,
            // Small cap vs large messages so the summary genuinely shrinks.
            result_char_threshold: 100,
        });
        // recent_turns normalized to 1, so with >1 turn there is something to compress.
        let mut messages = Vec::new();
        for i in 0..5 {
            messages.push(ChatMessage::user(big(&format!("q{i}"), 2_000)));
            messages.push(ChatMessage::assistant(big(&format!("a{i}"), 2_000)));
        }
        let outcome = mw.compress(&messages).expect("should compress");
        // Only the last turn preserved verbatim.
        assert_eq!(outcome.compressed_turns, 4);
    }

    #[test]
    fn no_rewrite_when_summary_would_not_shrink_context() {
        // Messages are small relative to the gist cap, so summarizing them
        // (with per-line labels + header) would not reduce tokens → no rewrite.
        let mw = SummarizationMiddleware::new(SummarizationConfig {
            token_threshold: 10,
            recent_turns: 1,
            result_char_threshold: 4_000,
        });
        let mut messages = Vec::new();
        for i in 0..4 {
            messages.push(ChatMessage::user(format!("q{i}")));
            messages.push(ChatMessage::assistant(format!("a{i}")));
        }
        assert!(mw.compress(&messages).is_none());
    }
}
