//! Single-process interactive REPL — no daemon needed.
//!
//! `arcan shell` builds the provider, tool registry, and command registry
//! in-process, then loops: read line, dispatch slash commands or send to the
//! LLM provider, execute tools inline, print response.
//!
//! Since BRO-356, the shell also opens a Lago `RedbJournal` for persistent
//! session history.  Events are appended best-effort (errors logged, never
//! fatal) so an on-disk failure cannot break the interactive experience.
//!
//! Since BRO-385, a shared **workspace journal** (`LanceJournal`) is opened at
//! `.arcan/workspace.lance/` for cross-session knowledge.  Governed memory
//! tools target this shared journal so insights persist across sessions.  The
//! per-session `RedbJournal` remains for conversation detail.

use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use aios_protocol::sandbox::NetworkPolicy;
use arcan_commands::{
    CommandContext, CommandRegistry, CommandResult, PermissionMode, is_tool_auto_approved,
    prompt_tool_permission,
};
use arcan_core::hooks::{HookContext, HookEvent, HookRegistry};
use arcan_core::protocol::{
    ChatMessage, ModelDirective, ModelStopReason, ToolCall, ToolDefinition,
};
use arcan_core::runtime::{Provider, ProviderRequest, StreamEvent, ToolContext, ToolRegistry};
use arcan_core::state::AppState;
use arcan_harness::bridge::PraxisToolBridge;
use arcan_harness::{FsPolicy, LocalFs, SandboxPolicy};
use lago_core::Journal;
use lago_core::event::{EventEnvelope, EventPayload};
use lago_core::id::{BranchId, EventId, SessionId as LagoSessionId};
use lago_core::session::{Session as LagoSession, SessionConfig};
use lago_journal::RedbJournal;
use lago_lance::LanceJournal;
use nous_core::{EvalContext, EvalHook, EvaluatorRegistry};
use praxis_tools::edit::EditFileTool;
use praxis_tools::fs::{GlobTool, GrepTool, ListDirTool, ReadFileTool, WriteFileTool};
use praxis_tools::memory::{ReadMemoryTool, WriteMemoryTool};
use praxis_tools::shell::BashTool;
use std::collections::BTreeSet;
use std::sync::RwLock;

// Phase 2: Governed memory tools (BRO-360, BRO-361)
use arcan_lago::{MemoryCommitTool, MemoryProjection, MemoryProposeTool, MemoryQueryTool};

use life_vigil::VigConfig;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;

use autonomic_controller::ContextPressureRule;
use autonomic_core::context::ContextRuling;
use autonomic_core::gating::HomeostaticState;

use crate::config::ResolvedConfig;

// ---------------------------------------------------------------------------
// Phase 7: Identity (BRO-370, BRO-371)
// ---------------------------------------------------------------------------

/// Supported identity tiers for Arcan users.
#[derive(Debug, Clone, PartialEq, Eq)]
enum IdentityTier {
    Anonymous,
    Free,
    Pro,
    Enterprise,
}

impl std::fmt::Display for IdentityTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Anonymous => write!(f, "anonymous"),
            Self::Free => write!(f, "free"),
            Self::Pro => write!(f, "pro"),
            Self::Enterprise => write!(f, "enterprise"),
        }
    }
}

/// Parsed identity information.
#[derive(Debug, Clone)]
struct IdentityInfo {
    tier: IdentityTier,
    subject: Option<String>,
}

/// Resolve identity from `ARCAN_IDENTITY_TOKEN` env var or `~/.arcan/identity.json`.
///
/// The identity file/token is expected to be a JSON object with at least a `tier` field.
/// Example: `{"tier": "pro", "sub": "user@example.com"}`
///
/// Returns `None` if no identity source is found.
fn resolve_identity() -> Option<IdentityInfo> {
    // Try env var first
    if let Ok(token) = std::env::var("ARCAN_IDENTITY_TOKEN") {
        if let Some(info) = parse_identity_json(&token) {
            return Some(info);
        }
    }

    // Fall back to ~/.arcan/identity.json
    if let Some(home) = dirs::home_dir() {
        let path = home.join(".arcan/identity.json");
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Some(info) = parse_identity_json(&content) {
                return Some(info);
            }
        }
    }

    None
}

/// Parse identity JSON (from env var or file).
fn parse_identity_json(json_str: &str) -> Option<IdentityInfo> {
    let value: serde_json::Value = serde_json::from_str(json_str).ok()?;
    let tier_str = value
        .get("tier")
        .and_then(|v| v.as_str())
        .unwrap_or("anonymous");
    let tier = match tier_str {
        "free" => IdentityTier::Free,
        "pro" => IdentityTier::Pro,
        "enterprise" => IdentityTier::Enterprise,
        _ => IdentityTier::Anonymous,
    };
    let subject = value.get("sub").and_then(|v| v.as_str()).map(String::from);
    Some(IdentityInfo { tier, subject })
}

/// Maximum number of lines to include in the session summary.
const SUMMARY_MAX_LINES: usize = 50;

/// Maximum number of characters from a single message to consider for extraction.
const EXTRACT_MAX_CHARS: usize = 2000;

/// Fallback context window (tokens) when the provider doesn't report one.
const DEFAULT_CONTEXT_WINDOW: usize = 200_000;

/// Estimate cost in USD for a model call based on token usage and model name.
///
/// Uses published Claude pricing (per million tokens):
/// - Opus: $15 input, $75 output
/// - Sonnet: $3 input, $15 output
/// - Haiku: $0.25 input, $1.25 output
fn estimate_cost(input_tokens: u64, output_tokens: u64, model: &str) -> f64 {
    let (input_rate, output_rate) = match model {
        m if m.contains("opus") => (15.0, 75.0),
        m if m.contains("sonnet") => (3.0, 15.0),
        m if m.contains("haiku") => (0.25, 1.25),
        _ => (3.0, 15.0), // default to sonnet pricing
    };
    (input_tokens as f64 * input_rate + output_tokens as f64 * output_rate) / 1_000_000.0
}

/// Query the Autonomic daemon for the current economic mode.
///
/// Best-effort: returns `None` on any failure (timeout, unreachable, etc.).
fn query_autonomic_mode(autonomic_url: &str) -> Option<String> {
    // Use a short timeout — this is advisory and must not block the REPL.
    let url = format!("{}/gating/shell", autonomic_url.trim_end_matches('/'));
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .ok()?;
    let resp = client.get(&url).send().ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body: serde_json::Value = resp.json().ok()?;
    // The gating response contains economic_mode or economic.mode depending on version.
    body.get("economic_mode")
        .or_else(|| body.pointer("/economic/mode"))
        .and_then(|v| v.as_str())
        .map(|s| {
            // Capitalize first letter for display
            let mut c = s.chars();
            match c.next() {
                Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
                None => s.to_string(),
            }
        })
}

/// Estimate total token count for a message list using a character-based heuristic.
///
/// Uses ~4 characters per token as a rough approximation.
fn estimate_tokens(messages: &[ChatMessage]) -> usize {
    messages.iter().map(|m| m.content.len() / 4).sum()
}

/// Compact a conversation to fit within a target token budget.
///
/// Smart compaction: extract memories THEN compress conversation.
/// This is the Claude Code pattern — compression and crystallization happen simultaneously.
#[allow(clippy::print_stderr)]
fn compact_with_extraction(messages: &mut Vec<ChatMessage>, memory_dir: &Path, target: usize) {
    // 1. Extract insights BEFORE compacting (last chance to see full conversation)
    extract_and_save_memories(messages, memory_dir);

    // 2. Update MEMORY.md index
    crate::prompt::write_memory_index(memory_dir);

    // 3. Compress the conversation
    compact_conversation(messages, target);

    // 4. EGRI signal: compaction = proactive management failed
    eprintln!(
        "[compact] ⚠ Emergency compaction triggered. Use memory_offload proactively to avoid this."
    );
}

/// Preserves the system context (first message if system role) and the most
/// recent messages. Inserts a compaction marker so the agent knows earlier
/// context was dropped.
fn compact_conversation(messages: &mut Vec<ChatMessage>, target: usize) {
    if messages.len() <= 4 {
        return;
    }
    let current = estimate_tokens(messages);
    if current <= target {
        return;
    }

    // Keep system context (first msg if system) + compaction marker
    let mut kept = Vec::new();
    let start_idx = if messages
        .first()
        .is_some_and(|m| m.role == arcan_core::protocol::Role::System)
    {
        kept.push(messages[0].clone());
        1
    } else {
        0
    };

    // Add compaction marker
    kept.push(ChatMessage::system(
        "[Earlier conversation compacted to stay within context limits]",
    ));

    // Budget remaining after kept prefix
    let budget = target.saturating_sub(estimate_tokens(&kept));

    // Walk backwards from the end, keeping recent messages that fit
    let mut tail = Vec::new();
    let mut used = 0;
    for msg in messages[start_idx..].iter().rev() {
        let cost = msg.content.len() / 4;
        if used + cost > budget {
            break;
        }
        used += cost;
        tail.push(msg.clone());
    }
    tail.reverse();
    kept.extend(tail);

    *messages = kept;
}

// ---------------------------------------------------------------------------
// Terminal echo suppression (prevents raw keystrokes during agent thinking)
// ---------------------------------------------------------------------------

/// Suppress terminal echo. Returns the saved termios state to restore later.
#[cfg(unix)]
fn suppress_echo() -> Option<nix::sys::termios::Termios> {
    let stdin = std::io::stdin();
    let saved = nix::sys::termios::tcgetattr(&stdin).ok()?;
    let mut modified = saved.clone();
    modified
        .local_flags
        .remove(nix::sys::termios::LocalFlags::ECHO);
    nix::sys::termios::tcsetattr(&stdin, nix::sys::termios::SetArg::TCSANOW, &modified).ok()?;
    Some(saved)
}

/// Restore terminal echo from saved termios state.
#[cfg(unix)]
fn restore_echo(saved: Option<nix::sys::termios::Termios>) {
    if let Some(original) = saved {
        let stdin = std::io::stdin();
        let _ =
            nix::sys::termios::tcsetattr(&stdin, nix::sys::termios::SetArg::TCSANOW, &original);
    }
}

// ---------------------------------------------------------------------------
// Lago journal helpers (BRO-356 / BRO-357)
// ---------------------------------------------------------------------------

/// Sequence counter shared across the shell session.
struct SeqCounter(AtomicU64);

impl SeqCounter {
    fn new(start: u64) -> Self {
        Self(AtomicU64::new(start))
    }

    fn next(&self) -> u64 {
        self.0.fetch_add(1, Ordering::Relaxed) + 1
    }
}

/// Append a single event to the journal, **best-effort**.
fn append_event_sync(journal: &dyn Journal, event: EventEnvelope) {
    let fut = journal.append(event);
    let result = match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle.block_on(fut),
        Err(_) => match tokio::runtime::Runtime::new() {
            Ok(rt) => rt.block_on(fut),
            Err(e) => {
                tracing::warn!("lago: cannot create fallback runtime: {e}");
                return;
            }
        },
    };
    if let Err(e) = result {
        tracing::warn!("lago: failed to append event: {e}");
    }
}

/// Build an `EventEnvelope` for a chat message.
fn make_message_event(
    session_id: &LagoSessionId,
    branch_id: &BranchId,
    seq: &SeqCounter,
    role: &str,
    content: &str,
    model: Option<&str>,
) -> EventEnvelope {
    EventEnvelope {
        event_id: EventId::new(),
        session_id: session_id.clone(),
        branch_id: branch_id.clone(),
        run_id: None,
        seq: seq.next(),
        timestamp: EventEnvelope::now_micros(),
        parent_id: None,
        payload: EventPayload::Message {
            role: role.to_string(),
            content: content.to_string(),
            model: model.map(str::to_string),
            token_usage: None,
        },
        metadata: HashMap::new(),
        schema_version: 1,
    }
}

/// Register (or update) a Lago session record.
fn put_session_sync(journal: &dyn Journal, session: LagoSession) {
    let fut = journal.put_session(session);
    let result = match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle.block_on(fut),
        Err(_) => match tokio::runtime::Runtime::new() {
            Ok(rt) => rt.block_on(fut),
            Err(_) => return,
        },
    };
    if let Err(e) = result {
        tracing::warn!("lago: failed to put session: {e}");
    }
}

/// List all sessions from the journal (sync wrapper).
fn list_sessions_sync(journal: &dyn Journal) -> Vec<LagoSession> {
    let fut = journal.list_sessions();
    let result = match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle.block_on(fut),
        Err(_) => match tokio::runtime::Runtime::new() {
            Ok(rt) => rt.block_on(fut),
            Err(_) => return Vec::new(),
        },
    };
    result.unwrap_or_default()
}

/// Replay persisted messages from a Lago session into a `Vec<ChatMessage>`.
///
/// Returns the messages and the highest sequence number seen.
fn replay_session_messages(
    journal: &dyn Journal,
    session_id: &LagoSessionId,
    branch_id: &BranchId,
) -> (Vec<ChatMessage>, u64) {
    let events = read_events_sync(
        journal,
        lago_core::EventQuery::new()
            .session(session_id.clone())
            .branch(branch_id.clone()),
    );

    let mut messages = Vec::new();
    let mut max_seq: u64 = 0;

    for env in &events {
        if env.seq > max_seq {
            max_seq = env.seq;
        }
        if let EventPayload::Message { role, content, .. } = &env.payload {
            let msg = match role.as_str() {
                "system" => ChatMessage::system(content),
                "user" => ChatMessage::user(content),
                "assistant" => ChatMessage::assistant(content),
                "tool" => ChatMessage::tool(content),
                _ => ChatMessage::user(content),
            };
            messages.push(msg);
        }
    }

    (messages, max_seq)
}

/// Read events from a journal synchronously.
fn read_events_sync(journal: &dyn Journal, query: lago_core::EventQuery) -> Vec<EventEnvelope> {
    let fut = journal.read(query);
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle.block_on(fut).unwrap_or_default(),
        Err(_) => match tokio::runtime::Runtime::new() {
            Ok(rt) => rt.block_on(fut).unwrap_or_default(),
            Err(_) => Vec::new(),
        },
    }
}

/// Build a small workspace-context section from recent shared journal summaries.
fn load_workspace_context(
    journal: &dyn Journal,
    workspace_session_id: &LagoSessionId,
    branch_id: &BranchId,
    current_session_id: &LagoSessionId,
) -> Option<String> {
    let events = read_events_sync(
        journal,
        lago_core::EventQuery::new()
            .session(workspace_session_id.clone())
            .branch(branch_id.clone()),
    );

    let current_session_marker = format!("Session {}", current_session_id);
    let mut summaries = Vec::new();
    for env in events.iter().rev() {
        let EventPayload::Message { role, content, .. } = &env.payload else {
            continue;
        };
        if role != "system" || content.is_empty() {
            continue;
        }
        if content.contains(&current_session_marker) {
            continue;
        }
        summaries.push(content.trim().to_string());
        if summaries.len() >= 5 {
            break;
        }
    }

    if summaries.is_empty() {
        None
    } else {
        summaries.reverse();
        Some(
            summaries
                .into_iter()
                .map(|summary| format!("- {summary}"))
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }
}

/// Load memory context from `.arcan/memory/*.md` files.
///
/// Reads all markdown files from the memory directory and returns a formatted
/// string suitable for injection into the system prompt. Returns `None` if the
/// directory doesn't exist or contains no memory files.
///
/// Note: The system prompt now uses `crate::prompt::build_memory_section` instead.
/// This function is retained for backward compatibility and tests.
#[cfg(test)]
fn load_memory_context(memory_dir: &Path) -> Option<String> {
    if !memory_dir.exists() {
        return None;
    }

    let entries = std::fs::read_dir(memory_dir).ok()?;
    let mut sections = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let key = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();
        if let Ok(content) = std::fs::read_to_string(&path) {
            if !content.trim().is_empty() {
                sections.push(format!("## {key}\n{content}"));
            }
        }
    }

    if sections.is_empty() {
        return None;
    }

    sections.sort();
    Some(format!(
        "# Agent Memory (cross-session)\n\n{}",
        sections.join("\n\n")
    ))
}

/// Check whether a message body is a structured content-block array
/// (the JSON encoding used for tool_use/tool_result exchanges).
fn is_structured_content(content: &str) -> bool {
    let trimmed = content.trim_start();
    if !trimmed.starts_with('[') {
        return false;
    }
    // Quick parse: check if first element has a "type" field.
    serde_json::from_str::<Vec<serde_json::Value>>(content)
        .map(|arr| arr.first().is_some_and(|v| v.get("type").is_some()))
        .unwrap_or(false)
}

/// Extract the text content from structured content blocks, skipping tool_use/tool_result.
fn extract_text_from_structured(content: &str) -> String {
    let Ok(blocks) = serde_json::from_str::<Vec<serde_json::Value>>(content) else {
        return String::new();
    };
    blocks
        .iter()
        .filter_map(|b| {
            if b.get("type").and_then(|t| t.as_str()) == Some("text") {
                b.get("text").and_then(|t| t.as_str()).map(String::from)
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Extract key facts from the latest agent turn and save to `.arcan/memory/session_summary.md`.
///
/// Uses a heuristic approach (no API calls): scans the conversation for patterns
/// that indicate decisions, file paths, errors, TODOs, and key findings.
#[allow(clippy::print_stderr)]
fn extract_and_save_memories(messages: &[ChatMessage], memory_dir: &Path) {
    if messages.is_empty() {
        return;
    }

    // Collect lines from assistant messages that look like key facts.
    let mut facts = Vec::new();

    for msg in messages.iter().rev().take(10) {
        if msg.role != arcan_core::protocol::Role::Assistant {
            continue;
        }

        // Skip structured content blocks (tool_use / tool_result JSON arrays).
        // These are encoded as JSON arrays with {"type":"tool_use",...} elements
        // by the agent loop — not natural language worth extracting.
        let content = if is_structured_content(&msg.content) {
            // Extract only the "text" blocks from structured content.
            extract_text_from_structured(&msg.content)
        } else {
            msg.content.clone()
        };

        let content = if content.len() > EXTRACT_MAX_CHARS {
            &content[..EXTRACT_MAX_CHARS]
        } else {
            &content
        };

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.len() < 10 {
                continue;
            }

            // Heuristic: keep lines that look like decisions, findings, or actions.
            let dominated_by_signal = is_memory_signal(trimmed);
            if dominated_by_signal {
                facts.push(format!("- {trimmed}"));
            }

            if facts.len() >= SUMMARY_MAX_LINES {
                break;
            }
        }
    }

    if facts.is_empty() {
        return;
    }

    // Build the session summary markdown.
    let timestamp = chrono::Utc::now().format("%Y-%m-%d %H:%M UTC");
    let summary = format!(
        "# Session Summary\n\n**Updated**: {timestamp}\n\n{}\n",
        facts.join("\n")
    );

    // Write to memory directory, creating it if needed.
    if let Err(e) = std::fs::create_dir_all(memory_dir) {
        eprintln!("[memory] Failed to create memory dir: {e}");
        return;
    }

    let path = memory_dir.join("session_summary.md");
    if let Err(e) = std::fs::write(&path, summary) {
        eprintln!("[memory] Failed to write session summary: {e}");
    }
}

/// Determine whether a line looks like a key fact worth remembering.
///
/// Returns `true` for lines containing decision markers, file paths,
/// error descriptions, TODOs, or other notable patterns.
fn is_memory_signal(line: &str) -> bool {
    let lower = line.to_lowercase();

    // --- Blocklist: filter out noise that matches bullet patterns ---
    // Tool descriptions from bare mode system prompt
    let tool_noise = [
        "read_file",
        "write_file",
        "edit_file",
        "bash:",
        "glob:",
        "grep:",
        "list_dir",
        "memory_query",
        "memory_propose",
        "memory_commit",
        "read_memory",
        "write_memory",
        "memory_search",
        "memory_similar",
        "memory_browse",
        "memory_recent",
        "memory_offload",
        "memory_forget",
        "run shell commands",
        "find files by pattern",
        "search file contents",
        "read file contents",
        "create or overwrite",
        "make targeted edits",
    ];
    // Strip markdown bold markers before matching (catches **read_file:** etc.)
    let stripped = lower.replace("**", "").replace('`', "");
    if tool_noise.iter().any(|t| stripped.contains(t)) {
        return false;
    }

    // Generic filler patterns (model boilerplate)
    let filler_noise = [
        "let me know if you",
        "feel free to ask",
        "i can help with",
        "here's how you can",
        "would you like to",
        "shall i proceed",
        "i hope this helps",
        "is there anything else",
    ];
    if filler_noise.iter().any(|f| lower.contains(f)) {
        return false;
    }

    // Decision / conclusion markers
    if lower.starts_with("- ") || lower.starts_with("* ") {
        // Bullet points are often summaries
        return true;
    }

    // Headings (markdown)
    if lower.starts_with("## ") || lower.starts_with("### ") {
        return true;
    }

    // Explicit signal words
    let signal_words = [
        "decision:",
        "decided",
        "chose",
        "created",
        "implemented",
        "fixed",
        "error:",
        "warning:",
        "bug:",
        "todo:",
        "fixme:",
        "note:",
        "important:",
        "key finding",
        "conclusion",
        "summary",
        "architecture",
        "pattern:",
        "learned",
        "discovered",
        "the issue was",
        "root cause",
        "solution:",
        "workaround:",
    ];
    if signal_words.iter().any(|w| lower.contains(w)) {
        return true;
    }

    // File paths (likely references to code)
    if line.contains('/')
        && (lower.contains(".rs")
            || lower.contains(".toml")
            || lower.contains(".ts")
            || lower.contains(".md"))
    {
        return true;
    }

    false
}

/// Guard that keeps the OTel telemetry pipeline alive for the shell session.
///
/// When dropped, flushes and shuts down the tracer and meter providers.
/// If no OTLP endpoint was configured, this is a no-op wrapper.
pub struct ShellTelemetryGuard {
    tracer_provider: Option<opentelemetry_sdk::trace::SdkTracerProvider>,
    meter_provider: Option<opentelemetry_sdk::metrics::SdkMeterProvider>,
}

impl Drop for ShellTelemetryGuard {
    fn drop(&mut self) {
        if let Some(ref tp) = self.tracer_provider {
            let _ = tp.shutdown();
        }
        if let Some(ref mp) = self.meter_provider {
            let _ = mp.shutdown();
        }
    }
}

/// Initialise the telemetry pipeline for shell mode (BRO-372).
///
/// Shell mode needs two capabilities simultaneously:
/// 1. **File-based structured logging** — tracing fmt output goes to
///    `<log_dir>/shell.log` so it never clobbers the interactive terminal.
/// 2. **OTLP export** — when `OTEL_EXPORTER_OTLP_ENDPOINT` is set, all
///    `tracing` spans are forwarded to an OTel collector for distributed
///    tracing (Langfuse, LangSmith, Jaeger, etc.).
///
/// When no OTLP endpoint is configured, Vigil degrades gracefully to
/// structured logging only (identical to the previous behaviour).
///
/// **Requires**: A Tokio runtime must be entered (but not blocked-on) before
/// calling this function, because tonic's OTLP exporter calls
/// `Handle::current()` during construction.
#[allow(clippy::print_stderr)]
pub fn init_shell_telemetry(log_dir: &Path, service_name: &str) -> ShellTelemetryGuard {
    let file_appender = tracing_appender::rolling::never(log_dir, "shell.log");
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let vig_config = VigConfig::for_service(service_name).with_env_overrides();

    if vig_config.otlp_endpoint.is_some() {
        // Full pipeline: file-appender fmt + OTel tracing layer.
        match build_shell_otel_subscriber(file_appender, env_filter, &vig_config) {
            Ok(guard) => return guard,
            Err(e) => {
                // Fall back to file-only logging if OTel setup fails.
                eprintln!("[vigil] OTel init failed ({e}), falling back to file logging");
                // Re-create the file appender and filter since they were moved.
                let file_appender = tracing_appender::rolling::never(log_dir, "shell.log");
                let env_filter =
                    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
                tracing_subscriber::fmt()
                    .with_writer(file_appender)
                    .with_env_filter(env_filter)
                    .init();
                return ShellTelemetryGuard {
                    tracer_provider: None,
                    meter_provider: None,
                };
            }
        }
    }

    // Logging-only: file appender with env filter, no OTel export.
    tracing_subscriber::fmt()
        .with_writer(file_appender)
        .with_env_filter(env_filter)
        .init();

    ShellTelemetryGuard {
        tracer_provider: None,
        meter_provider: None,
    }
}

/// Build a combined subscriber: file-appender fmt + OTel tracing layer.
///
/// Sets up the OTel tracer and meter providers (for OTLP export) and
/// composes them with a file-appender fmt layer (for shell.log output).
fn build_shell_otel_subscriber(
    file_appender: tracing_appender::rolling::RollingFileAppender,
    env_filter: EnvFilter,
    vig_config: &VigConfig,
) -> Result<ShellTelemetryGuard, String> {
    use opentelemetry::global;
    use opentelemetry::trace::TracerProvider as _;
    use opentelemetry_otlp::WithExportConfig as _;

    let endpoint = vig_config
        .otlp_endpoint
        .as_deref()
        .unwrap_or("http://localhost:4317");

    let resource = opentelemetry_sdk::Resource::builder()
        .with_service_name(vig_config.service_name.clone())
        .build();

    // Build OTLP span exporter (gRPC)
    let span_exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .build()
        .map_err(|e| format!("span exporter: {e}"))?;

    let tracer_provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
        .with_batch_exporter(span_exporter)
        .with_resource(resource.clone())
        .build();

    global::set_tracer_provider(tracer_provider.clone());

    // Build OTLP metric exporter
    let metric_exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .build()
        .map_err(|e| format!("metric exporter: {e}"))?;

    let meter_provider = opentelemetry_sdk::metrics::SdkMeterProvider::builder()
        .with_periodic_exporter(metric_exporter)
        .with_resource(resource)
        .build();

    global::set_meter_provider(meter_provider.clone());

    // Build OTel tracing layer bridged to the tracer provider
    let tracer = tracer_provider.tracer(vig_config.service_name.clone());
    let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);

    // Compose: file-appender fmt layer + OTel layer
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_writer(file_appender)
        .with_filter(env_filter);

    tracing_subscriber::registry()
        .with(otel_layer)
        .with(fmt_layer)
        .try_init()
        .map_err(|e| format!("subscriber init: {e}"))?;

    Ok(ShellTelemetryGuard {
        tracer_provider: Some(tracer_provider),
        meter_provider: Some(meter_provider),
    })
}

/// Run the interactive shell REPL.
#[allow(clippy::print_stderr, clippy::print_stdout)]
pub fn run_shell(
    data_dir: &Path,
    resolved: &ResolvedConfig,
    session: Option<&str>,
    yes: bool,
    resume: bool,
    budget: Option<f64>,
    show_reasoning: bool,
) -> anyhow::Result<()> {
    let workspace_root = std::env::current_dir()?;

    // --- Lago journal (BRO-356) ---
    // Use per-session journal files to support parallel shell instances.
    // Each session gets its own .redb file, avoiding lock contention.
    // The main journal.redb is reserved for the daemon (arcan serve).
    let journals_dir = data_dir.join("shell-journals");
    std::fs::create_dir_all(&journals_dir).ok();

    // Determine session ID early so we can name the journal file
    let lago_session_id_for_journal = session.map(LagoSessionId::from_string).unwrap_or_default();

    let journal_path = journals_dir.join(format!("{}.redb", lago_session_id_for_journal));
    let (journal, journal_ephemeral): (Arc<dyn Journal>, bool) =
        match RedbJournal::open(&journal_path) {
            Ok(j) => (Arc::new(j), false),
            Err(e) => {
                eprintln!(
                    "[lago] Warning: could not open journal ({}). Using ephemeral session.",
                    e
                );
                (
                    Arc::new(crate::ephemeral_journal::EphemeralJournal::new()),
                    true,
                )
            }
        };
    let branch_id = BranchId::from_string("main");

    // --- Workspace journal (BRO-385) ---
    // Shared Lance-backed journal for cross-session knowledge.
    // Governed memory tools target this so insights persist across sessions.
    let workspace_path = data_dir.join("workspace.lance");
    let workspace_lance: Option<Arc<LanceJournal>> = {
        let open_fut = LanceJournal::open(&workspace_path);
        let result = match tokio::runtime::Handle::try_current() {
            Ok(handle) => Some(handle.block_on(open_fut)),
            Err(_) => match tokio::runtime::Runtime::new() {
                Ok(rt) => Some(rt.block_on(open_fut)),
                Err(e) => {
                    tracing::warn!("lago-lance: cannot create runtime: {e}");
                    None
                }
            },
        };
        match result {
            Some(Ok(j)) => {
                eprintln!("[lago] Workspace journal: {}", workspace_path.display());
                Some(Arc::new(j))
            }
            Some(Err(e)) => {
                eprintln!(
                    "[lago] Warning: could not open workspace Lance journal ({}). Governed memory will use session journal.",
                    e
                );
                None
            }
            None => None,
        }
    };
    let workspace_journal: Option<Arc<dyn Journal>> = workspace_lance
        .as_ref()
        .map(|journal| journal.clone() as Arc<dyn Journal>);
    // Sequence counter for workspace journal events (separate from session seq)
    let workspace_seq = SeqCounter::new(0);
    // Fixed session ID for workspace-level events
    let workspace_session_id = LagoSessionId::from_string("workspace");

    // Session ID was determined above (for journal file naming).
    // If resuming, check if the journal has existing events.
    let lago_session_id = lago_session_id_for_journal;
    let resuming = resume;

    // Seed sequence counter from journal head
    let head_seq = {
        let fut = journal.head_seq(&lago_session_id, &branch_id);
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => handle.block_on(fut).unwrap_or(0),
            Err(_) => match tokio::runtime::Runtime::new() {
                Ok(rt) => rt.block_on(fut).unwrap_or(0),
                Err(_) => 0,
            },
        }
    };
    let seq_counter = SeqCounter::new(head_seq);

    // Register (or update) the session record
    put_session_sync(
        journal.as_ref(),
        LagoSession {
            session_id: lago_session_id.clone(),
            config: SessionConfig {
                name: format!("shell-{}", lago_session_id),
                model: resolved.model.clone().unwrap_or_default(),
                params: HashMap::new(),
            },
            created_at: if resuming {
                list_sessions_sync(journal.as_ref())
                    .iter()
                    .find(|s| s.session_id == lago_session_id)
                    .map(|s| s.created_at)
                    .unwrap_or_else(EventEnvelope::now_micros)
            } else {
                EventEnvelope::now_micros()
            },
            branches: vec![branch_id.clone()],
        },
    );

    // --- Provider ---
    let provider = crate::build_provider(resolved)?;

    // --- Autonomic context regulation ---
    let context_rule = ContextPressureRule::default();
    let context_window = provider
        .context_window()
        .unwrap_or(DEFAULT_CONTEXT_WINDOW as u32) as usize;
    let mut homeostatic_state = HomeostaticState::for_agent("shell");
    homeostatic_state.cognitive.tokens_remaining = context_window as u64;

    // --- Tools (same set as run_serve) ---
    let mut registry = ToolRegistry::default();

    if resolved.bare {
        // Bare mode: NO tool schemas sent to model (described in system prompt instead).
        // Small models (≤4K context) hallucinate function calls when tools are present.
        eprintln!("[bare] No tools registered — capabilities described in system prompt");
    } else {
        let fs_policy = FsPolicy::new(workspace_root.clone());
        let local_fs = Arc::new(LocalFs::new(fs_policy));

        registry.register(PraxisToolBridge::new(ReadFileTool::new(local_fs.clone())));
        registry.register(PraxisToolBridge::new(WriteFileTool::new(local_fs.clone())));
        registry.register(PraxisToolBridge::new(ListDirTool::new(local_fs.clone())));
        registry.register(PraxisToolBridge::new(EditFileTool::new(local_fs.clone())));
        registry.register(PraxisToolBridge::new(GlobTool::new(local_fs.clone())));
        registry.register(PraxisToolBridge::new(GrepTool::new(local_fs)));

        let sandbox_policy = SandboxPolicy {
            workspace_root: workspace_root.clone(),
            shell_enabled: true,
            network: NetworkPolicy::AllowAll,
            allowed_env: BTreeSet::new(),
            max_execution_ms: 30_000,
            max_stdout_bytes: 1024 * 1024,
            max_stderr_bytes: 1024 * 1024,
        };

        let sandbox_provider = crate::sandbox_router::build_sandbox_provider_with_fallback();
        let runner: Box<dyn praxis_core::sandbox::CommandRunner> =
            Box::new(arcan_praxis::SandboxCommandRunner::new(sandbox_provider));
        registry.register(PraxisToolBridge::new(BashTool::new(sandbox_policy, runner)));
    } // else (not bare) — tool registration

    // --- Embedding provider (BRO-388) ---
    let embedding_provider: Option<Arc<dyn crate::embedding::EmbeddingProvider>> =
        crate::embedding::HttpEmbeddingProvider::from_env().map(|p| Arc::new(p) as _);
    if embedding_provider.is_some() {
        eprintln!("[embedding] Provider configured (embed-on-write active)");
    }

    let memory_dir = data_dir.join("memory");
    std::fs::create_dir_all(&memory_dir)?;

    // Extended tools — skipped in bare mode
    if !resolved.bare {
        registry.register(PraxisToolBridge::new(ReadMemoryTool::new(
            memory_dir.clone(),
        )));
        registry.register(PraxisToolBridge::new(WriteMemoryTool::new(
            memory_dir.clone(),
        )));

        // --- BRO-417: Agent-driven memory retrieval tools ---
        registry.register(PraxisToolBridge::new(
            crate::memory_tools::MemorySearchTool::new(&memory_dir),
        ));
        registry.register(PraxisToolBridge::new(
            crate::memory_tools::MemorySimilarTool::new(
                &memory_dir,
                embedding_provider.clone(),
                workspace_lance.clone(),
            ),
        ));
        registry.register(PraxisToolBridge::new(
            crate::memory_tools::MemoryBrowseTool::new(&memory_dir),
        ));
        registry.register(PraxisToolBridge::new(
            crate::memory_tools::MemoryRecentTool::new(&memory_dir),
        ));
        registry.register(PraxisToolBridge::new(
            crate::memory_tools::MemoryOffloadTool::new(&memory_dir),
        ));
        registry.register(PraxisToolBridge::new(
            crate::memory_tools::MemoryForgetTool::new(&memory_dir),
        ));

        // --- Phase 2: Governed memory tools (BRO-360, BRO-361, BRO-385) ---
        let memory_journal: Arc<dyn Journal> =
            workspace_journal.clone().unwrap_or_else(|| journal.clone());
        let memory_projection = Arc::new(RwLock::new(MemoryProjection::new()));
        registry.register(MemoryQueryTool::new(memory_projection));
        registry.register(MemoryProposeTool::new(memory_journal.clone()));
        registry.register(MemoryCommitTool::new(memory_journal));
    } // if !resolved.bare — extended tools

    // --- Phase 6: Spaces networking (BRO-368, BRO-369) ---
    #[cfg(feature = "spaces")]
    let spaces_connected = {
        let mut connected = false;
        if !resolved.bare && resolved.spaces_backend != "none" {
            match resolved.spaces_backend.as_str() {
                "spacetimedb" | "mainnet" => {
                    match arcan_spaces::SpacetimeDbConfig::resolve(
                        resolved.spaces_host.as_deref(),
                        resolved.spaces_database_id.as_deref(),
                        resolved.spaces_token.as_deref(),
                    ) {
                        Ok(stdb_config) => {
                            let spaces_port: Arc<dyn arcan_spaces::SpacesPort> =
                                Arc::new(arcan_spaces::SpacetimeDbClient::new(stdb_config));
                            arcan_spaces::register_spaces_tools(&mut registry, spaces_port);
                            connected = true;
                            eprintln!("[spaces] SpacetimeDB backend connected");
                        }
                        Err(e) => {
                            eprintln!("[spaces] SpacetimeDB config failed (non-fatal): {e}");
                        }
                    }
                }
                "mock" => {
                    let spaces_port: Arc<dyn arcan_spaces::SpacesPort> =
                        Arc::new(arcan_spaces::MockSpacesClient::default_hub());
                    arcan_spaces::register_spaces_tools(&mut registry, spaces_port);
                    connected = true;
                    eprintln!("[spaces] Mock backend connected");
                }
                _ => {
                    // Unknown backend — skip silently
                }
            }
        }
        connected
    };
    #[cfg(not(feature = "spaces"))]
    let spaces_connected = false;

    // --- Phase 7: Identity resolution (BRO-370, BRO-371) ---
    let identity = resolve_identity();
    if let Some(ref id) = identity {
        let subject_display = id
            .subject
            .as_deref()
            .map(|s| format!(" ({s})"))
            .unwrap_or_default();
        eprintln!("[identity] Tier: {}{}", id.tier, subject_display);
    }

    // --- Skill discovery ---
    let skill_registry = if resolved.skills_enabled {
        match crate::skills::discover_skills(
            &resolved.skill_dirs,
            data_dir,
            resolved.skills_write_registry,
        ) {
            Ok(reg) => Some(reg),
            Err(e) => {
                eprintln!("[skills] Discovery failed (non-fatal): {e}");
                None
            }
        }
    } else {
        None
    };

    let skill_names: Vec<String> = skill_registry
        .as_ref()
        .map(praxis_skills::registry::SkillRegistry::skill_names)
        .unwrap_or_default();

    // --- Command registry ---
    let commands = CommandRegistry::with_builtins();
    let tool_defs = registry.definitions();

    // --- Nous eval registry (BRO-362) ---
    let nous_registry: Option<EvaluatorRegistry> = match nous_heuristics::default_registry() {
        Ok(reg) => {
            eprintln!("[nous] {} evaluators active", reg.len());
            Some(reg)
        }
        Err(e) => {
            tracing::warn!(error = %e, "Nous eval init failed (non-fatal)");
            None
        }
    };

    // --- Hook registry ---
    let hook_registry = &resolved.hook_registry;
    let session_id_str = lago_session_id.to_string();
    let hook_ctx = HookContext {
        session_id: session_id_str.clone(),
        tool_name: None,
        tool_input: None,
        workspace: workspace_root.display().to_string(),
    };

    // --- Fire SessionStart hooks ---
    if !hook_registry.is_empty() {
        hook_registry.fire(&HookEvent::SessionStart, &hook_ctx);
    }

    // --- Session state ---
    let permission_mode = if yes {
        PermissionMode::Yes
    } else {
        PermissionMode::Default
    };
    let provider_name = provider.name().to_string();
    let model_name = resolved
        .model
        .clone()
        .unwrap_or_else(|| "default".to_string());
    // --- Autonomic economic mode (BRO-365) ---
    let economic_mode = resolved
        .autonomic_url
        .as_deref()
        .and_then(query_autonomic_mode);

    let mut cmd_ctx = CommandContext {
        workspace: workspace_root,
        permission_mode,
        memory_dir: memory_dir.clone(),
        provider_name: provider_name.clone(),
        model_name: model_name.clone(),
        data_dir: data_dir.to_path_buf(),
        tools_count: tool_defs.len(),
        hooks_count: hook_registry.len(),
        skill_names: skill_names.clone(),
        budget_usd: budget,
        economic_mode,
        show_reasoning,
        workspace_journal_status: workspace_journal
            .as_ref()
            .map(|_| format!("{} (shared)", workspace_path.display())),
        context_window: Some(context_window),
        ..Default::default()
    };

    // --- Generate MEMORY.md index (BRO-419) ---
    crate::prompt::write_memory_index(&memory_dir);

    // --- Build unified system prompt (liquid prompt, BRO-420 cache split) ---
    let system_prompt = if resolved.bare {
        // Bare mode: minimal prompt for small-context models (≤4K tokens).
        eprintln!("[bare] Using minimal system prompt");
        crate::prompt::build_bare_prompt(&cmd_ctx.workspace, &provider_name, &model_name)
    } else {
        let claude_md = crate::prompt::load_claude_md(&cmd_ctx.workspace);
        let skill_catalog_text = skill_registry
            .as_ref()
            .map(crate::skills::build_system_prompt);
        let workspace_context = workspace_journal.as_ref().and_then(|journal| {
            load_workspace_context(
                journal.as_ref(),
                &workspace_session_id,
                &branch_id,
                &lago_session_id,
            )
        });

        let system_prompt_struct = crate::prompt::build_system_prompt(
            &cmd_ctx.workspace,
            &provider_name,
            &model_name,
            &memory_dir,
            workspace_context.as_deref(),
            skill_catalog_text.as_deref(),
            claude_md.as_deref(),
        );
        // Populate context token estimates for /context command
        cmd_ctx.project_instructions_tokens = claude_md.as_deref().map_or(0, |s| s.len() / 4);
        cmd_ctx.skills_catalog_tokens = skill_catalog_text.as_deref().map_or(0, |s| s.len() / 4);
        cmd_ctx.git_context_tokens =
            crate::prompt::build_git_section(&cmd_ctx.workspace).map_or(0, |s| s.len() / 4);
        cmd_ctx.memory_index_tokens =
            crate::prompt::build_memory_section(&memory_dir).map_or(0, |s| s.len() / 4);
        cmd_ctx.workspace_context_tokens = workspace_context.as_deref().map_or(0, |s| s.len() / 4);

        system_prompt_struct.combined()
    };

    // --- Replay or initialize message history (BRO-358) ---
    let mut messages: Vec<ChatMessage> = Vec::new();
    if resuming {
        let (replayed, _max_seq) =
            replay_session_messages(journal.as_ref(), &lago_session_id, &branch_id);
        if replayed.is_empty() {
            messages.push(ChatMessage::system(&system_prompt));
        } else {
            messages = replayed;
            eprintln!(
                "[lago] Restored {} messages from session {}",
                messages.len(),
                lago_session_id
            );
        }
    } else {
        messages.push(ChatMessage::system(&system_prompt));
        // Persist the system prompt as the first event
        append_event_sync(
            journal.as_ref(),
            make_message_event(
                &lago_session_id,
                &branch_id,
                &seq_counter,
                "system",
                &system_prompt,
                None,
            ),
        );
    }

    // --- Welcome banner ---
    eprintln!("arcan shell v{}", env!("CARGO_PKG_VERSION"));
    eprintln!(
        "Provider: {} | Model: {} | Tools: {} | Hooks: {} | Skills: {}",
        provider_name,
        model_name,
        tool_defs.len(),
        hook_registry.len(),
        skill_names.len(),
    );
    if let Some(b) = budget {
        eprintln!("Budget: ${b:.2}");
    }
    // Phase 7: Identity in banner
    if let Some(ref id) = identity {
        let subject_display = id
            .subject
            .as_deref()
            .map(|s| format!(" ({s})"))
            .unwrap_or_default();
        eprintln!("Identity: {}{}", id.tier, subject_display);
    }
    // Phase 6: Spaces status in banner
    if spaces_connected {
        eprintln!("Spaces: connected ({})", resolved.spaces_backend);
    }
    if journal_ephemeral {
        eprintln!(
            "Session: {} | Journal: ephemeral (in-memory) | Type /help for commands",
            lago_session_id,
        );
    } else {
        eprintln!(
            "Session: {} | Journal: {} | Type /help for commands",
            lago_session_id,
            journal_path.display()
        );
    }
    // BRO-385: Workspace journal banner line
    if workspace_journal.is_some() {
        eprintln!("Workspace: {} (shared)", workspace_path.display());
    }
    eprintln!();

    // --- Phase 8: Vigil session span (BRO-372, BRO-373) ---
    // Wrap the entire REPL session in a tracing span for observability.
    // If no OTLP endpoint is configured, these are no-op structured log entries.
    let _session_span = tracing::info_span!(
        "arcan_shell_session",
        session_id = %lago_session_id,
        provider = %provider_name,
        model = %model_name,
        identity_tier = identity.as_ref().map(|id| id.tier.to_string()).unwrap_or_else(|| "none".to_string()),
    )
    .entered();

    // --- Message queue: stdin reader thread ---
    let (input_tx, input_rx) = std::sync::mpsc::channel::<Option<String>>();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        loop {
            let mut line = String::new();
            match stdin.read_line(&mut line) {
                Ok(0) => {
                    let _ = input_tx.send(None); // EOF
                    break;
                }
                Ok(_) => {
                    let _ = input_tx.send(Some(line));
                }
                Err(_) => {
                    let _ = input_tx.send(None);
                    break;
                }
            }
        }
    });

    // --- Dedicated writer thread for non-blocking persistence ---
    // All journal writes, memory extraction, and MEMORY.md regeneration
    // go through this channel. Sequential, ordered, non-blocking to the REPL.
    enum WriteTask {
        Event {
            journal: Arc<dyn Journal>,
            event: EventEnvelope,
        },
        ExtractMemories {
            messages: Vec<ChatMessage>,
            memory_dir: std::path::PathBuf,
        },
    }
    let (writer_tx, writer_rx) = std::sync::mpsc::channel::<WriteTask>();
    std::thread::spawn(move || {
        for task in writer_rx {
            match task {
                WriteTask::Event { journal, event } => {
                    append_event_sync(journal.as_ref(), event);
                }
                WriteTask::ExtractMemories {
                    messages,
                    memory_dir,
                } => {
                    extract_and_save_memories(&messages, &memory_dir);
                    crate::prompt::write_memory_index(&memory_dir);
                }
            }
        }
    });

    // --- REPL loop ---
    loop {
        eprint!("\narcan> ");
        std::io::stderr().flush().ok();

        let Ok(Some(line)) = input_rx.recv() else {
            break; // EOF or channel closed
        };

        let input = line.trim();
        if input.is_empty() {
            continue;
        }

        // --- Slash command dispatch ---
        if input.starts_with('/') {
            // Update message_count before command dispatch
            cmd_ctx.message_count = messages.len();

            // Bare "/" — show all available commands as hints.
            if input == "/" {
                let all = commands.matching_commands("");
                eprintln!("Available commands:");
                for (name, desc) in &all {
                    eprintln!("  {name} — {desc}");
                }
                continue;
            }

            // --- BRO-359: /sessions and /session commands (need journal access) ---
            if input == "/sessions" || input == "/sess" {
                // List sessions from journal files in shell-journals/ directory
                let sessions = list_sessions_sync(journal.as_ref());
                // Also scan directory for other session journal files
                let mut extra_sessions: Vec<String> = Vec::new();
                if let Ok(entries) = std::fs::read_dir(&journals_dir) {
                    for entry in entries.flatten() {
                        if let Some(name) = entry.path().file_stem().and_then(|s| s.to_str()) {
                            if name != lago_session_id.to_string() {
                                extra_sessions.push(name.to_string());
                            }
                        }
                    }
                }
                if sessions.is_empty() && extra_sessions.is_empty() {
                    println!("No sessions found.");
                } else if sessions.is_empty() {
                    println!("Other session journals (use --session <id> --resume):");
                    for sid in &extra_sessions {
                        println!("  {sid}");
                    }
                } else {
                    println!(
                        "{:<28} {:<20} {:>6}  NAME",
                        "SESSION ID", "CREATED", "EVENTS"
                    );
                    println!("{}", "-".repeat(80));
                    let mut sorted = sessions;
                    sorted.sort_by(|a, b| b.created_at.cmp(&a.created_at));
                    for sess in &sorted {
                        let ts_secs = sess.created_at / 1_000_000;
                        let dt = chrono::DateTime::from_timestamp(ts_secs as i64, 0)
                            .map(|d| d.format("%Y-%m-%d %H:%M UTC").to_string())
                            .unwrap_or_else(|| "unknown".to_string());
                        let head = {
                            let fut = journal.head_seq(&sess.session_id, &branch_id);
                            match tokio::runtime::Handle::try_current() {
                                Ok(h) => h.block_on(fut).unwrap_or(0),
                                Err(_) => match tokio::runtime::Runtime::new() {
                                    Ok(rt) => rt.block_on(fut).unwrap_or(0),
                                    Err(_) => 0,
                                },
                            }
                        };
                        let marker = if sess.session_id == lago_session_id {
                            " *"
                        } else {
                            ""
                        };
                        println!(
                            "{:<28} {:<20} {:>6}  {}{}",
                            sess.session_id, dt, head, sess.config.name, marker
                        );
                    }
                }
                continue;
            }

            // /session <id> — info about switching sessions
            if input.starts_with("/session ") {
                let target = input.strip_prefix("/session ").unwrap_or("").trim();
                if target.is_empty() {
                    eprintln!("Usage: /session <session-id>");
                } else {
                    let sessions = list_sessions_sync(journal.as_ref());
                    if sessions.iter().any(|s| s.session_id.as_str() == target) {
                        eprintln!("[lago] Note: /session switch is not yet supported mid-REPL.");
                        eprintln!(
                            "       Use `arcan shell --session {target} --resume` to resume."
                        );
                    } else {
                        eprintln!("Session '{target}' not found. Use /sessions to list available.");
                    }
                }
                continue;
            }

            match commands.execute(input, &mut cmd_ctx) {
                Some(CommandResult::Output(text)) => {
                    println!("{text}");
                }
                Some(CommandResult::ClearSession) => {
                    messages.clear();
                    cmd_ctx.session_turns = 0;
                    cmd_ctx.session_input_tokens = 0;
                    cmd_ctx.session_output_tokens = 0;
                    cmd_ctx.session_cost_usd = 0.0;
                    cmd_ctx.message_count = 0;
                    cmd_ctx.tool_call_count = 0;
                    cmd_ctx.session_approved_tools.clear();
                    eprintln!("Session cleared.");
                }
                Some(CommandResult::CompactRequested) => {
                    let before = estimate_tokens(&messages);
                    let target = context_window * 35 / 100;
                    compact_with_extraction(&mut messages, &memory_dir, target);
                    homeostatic_state.cognitive.turns_since_compact = 0;
                    let after = estimate_tokens(&messages);
                    eprintln!("[compact] {before} tokens -> {after} tokens (target: {target})");
                }
                Some(CommandResult::ConsolidateRequested) => {
                    eprintln!("[consolidate] Running memory consolidation...");
                    crate::consolidator::consolidate(&memory_dir);
                    crate::prompt::write_memory_index(&memory_dir);
                    eprintln!("[consolidate] Done.");
                }
                Some(CommandResult::Quit) => {
                    eprintln!("Goodbye.");
                    break;
                }
                Some(CommandResult::Error(err)) => {
                    eprintln!("Error: {err}");
                }
                None => {
                    // Not a builtin command — try to activate it as a skill.
                    if let Some(ref sr) = skill_registry {
                        match crate::skills::try_activate_skill(sr, input) {
                            Ok(Some((skill_state, _remaining))) => {
                                eprintln!("[skill] Activated: {}", skill_state.name);
                                let instructions = crate::skills::active_skill_prompt(&skill_state);
                                messages.push(ChatMessage::system(&instructions));
                                continue;
                            }
                            Ok(None) => { /* not a skill prefix */ }
                            Err(e) => {
                                eprintln!("[skill] {e}");
                            }
                        }
                    }

                    // Show matching commands as hints.
                    let prefix = input.strip_prefix('/').unwrap_or(input);
                    let matches = commands.matching_commands(prefix);
                    if matches.is_empty() {
                        eprintln!("Unknown command: {input}. Type /help for available commands.");
                    } else if matches.len() == 1 {
                        eprintln!("Unknown command: {input}. Did you mean?");
                        for (name, desc) in &matches {
                            eprintln!("  {name} — {desc}");
                        }
                    } else {
                        eprintln!("Matching commands:");
                        for (name, desc) in &matches {
                            eprintln!("  {name} — {desc}");
                        }
                    }
                }
            }
            continue;
        }

        // --- Send to provider ---
        messages.push(ChatMessage::user(input));
        cmd_ctx.session_turns += 1;
        cmd_ctx.message_count = messages.len();

        // Persist user message (non-blocking via writer thread)
        let _ = writer_tx.send(WriteTask::Event {
            journal: journal.clone(),
            event: make_message_event(
                &lago_session_id,
                &branch_id,
                &seq_counter,
                "user",
                input,
                None,
            ),
        });

        let emb_ctx = match (&embedding_provider, &workspace_journal) {
            (Some(ep), Some(wj)) => Some(EmbeddingContext {
                provider: ep.as_ref(),
                workspace_journal: wj.as_ref(),
                workspace_session_id: &workspace_session_id,
                branch_id: &branch_id,
                workspace_seq: &workspace_seq,
            }),
            _ => None,
        };
        // Suppress terminal echo while agent is working so queued
        // keystrokes don't visually leak onto the streaming output.
        #[cfg(unix)]
        let saved_termios = suppress_echo();

        let response_text = run_agent_loop(
            &provider,
            &registry,
            &tool_defs,
            &mut messages,
            &mut cmd_ctx,
            hook_registry,
            &hook_ctx,
            nous_registry.as_ref(),
            emb_ctx.as_ref(),
        );

        // Restore terminal echo.
        #[cfg(unix)]
        restore_echo(saved_termios);

        match response_text {
            Ok(text) => {
                if !text.is_empty() {
                    messages.push(ChatMessage::assistant(&text));
                }
                // Update in-memory state immediately (instant).
                cmd_ctx.message_count = messages.len();
                homeostatic_state.operational.error_streak = 0;
                if !text.is_empty() {
                    let prev = homeostatic_state.eval.aggregate_quality_score;
                    let signal = if text.len() > 20 { 0.85 } else { 0.5 };
                    homeostatic_state.eval.aggregate_quality_score = prev * 0.7 + signal * 0.3;
                    homeostatic_state.eval.quality_trend =
                        homeostatic_state.eval.aggregate_quality_score - prev;
                    homeostatic_state.eval.inline_eval_count += 1;
                }

                // --- Persistence via writer thread (non-blocking) ---
                if !text.is_empty() {
                    let _ = writer_tx.send(WriteTask::Event {
                        journal: journal.clone(),
                        event: make_message_event(
                            &lago_session_id,
                            &branch_id,
                            &seq_counter,
                            "assistant",
                            &text,
                            resolved.model.as_deref(),
                        ),
                    });
                }
                if let Some(ref wj) = workspace_journal {
                    let truncated: String = text.chars().take(200).collect();
                    let summary = if truncated.len() < text.len() {
                        format!(
                            "Session {} turn {}: {truncated}...",
                            lago_session_id, cmd_ctx.session_turns
                        )
                    } else {
                        format!(
                            "Session {} turn {}: {}",
                            lago_session_id, cmd_ctx.session_turns, text
                        )
                    };
                    let _ = writer_tx.send(WriteTask::Event {
                        journal: wj.clone(),
                        event: make_message_event(
                            &workspace_session_id,
                            &branch_id,
                            &workspace_seq,
                            "system",
                            &summary,
                            None,
                        ),
                    });
                }
                let _ = writer_tx.send(WriteTask::ExtractMemories {
                    messages: messages.clone(),
                    memory_dir: memory_dir.clone(),
                });
            }
            Err(e) => {
                eprintln!("Error: {e}");
                homeostatic_state.operational.error_streak += 1;
            }
        }

        // --- Autonomic-regulated context compression ---
        let tokens = estimate_tokens(&messages);
        let pressure = if context_window > 0 {
            tokens as f32 / context_window as f32
        } else {
            0.0
        };

        // Update cognitive state for Autonomic evaluation
        homeostatic_state.cognitive.context_pressure = pressure;
        homeostatic_state.cognitive.total_tokens_used =
            cmd_ctx.session_input_tokens + cmd_ctx.session_output_tokens;
        homeostatic_state.cognitive.tokens_remaining = context_window.saturating_sub(tokens) as u64;
        homeostatic_state.cognitive.turns_completed += 1;
        homeostatic_state.cognitive.turns_since_compact += 1;
        homeostatic_state.cognitive.tool_density =
            if homeostatic_state.cognitive.turns_completed > 0 {
                cmd_ctx.tool_call_count as f64 / homeostatic_state.cognitive.turns_completed as f64
            } else {
                0.0
            };

        let advice = context_rule.evaluate_compression(&homeostatic_state);

        // Store ruling for /context command display
        cmd_ctx.context_ruling = Some(format!(
            "{:?} \u{2014} pressure {:.0}%, quality {:.2}, tool_density {:.1}, turns_since_compact {}",
            advice.ruling,
            advice.pressure * 100.0,
            homeostatic_state.eval.aggregate_quality_score,
            homeostatic_state.cognitive.tool_density,
            homeostatic_state.cognitive.turns_since_compact,
        ));

        match advice.ruling {
            ContextRuling::Breathe => {}
            ContextRuling::Dilate => {
                eprintln!(
                    "[context] {:.0}% \u{2014} dilating ({})",
                    pressure * 100.0,
                    advice.rationale
                );
            }
            ContextRuling::Compress => {
                let target = advice.target_tokens.unwrap_or(context_window * 35 / 100);
                eprintln!(
                    "[context] {:.0}% \u{2014} compressing to ~{} ({})",
                    pressure * 100.0,
                    target,
                    advice.rationale
                );
                compact_with_extraction(&mut messages, &memory_dir, target);
                homeostatic_state.cognitive.turns_since_compact = 0;
                let after = estimate_tokens(&messages);
                eprintln!("[context] now ~{after} tokens (memories extracted)");
            }
            ContextRuling::Emergency => {
                let target = advice.target_tokens.unwrap_or(context_window * 25 / 100);
                eprintln!(
                    "[context] EMERGENCY {:.0}% \u{2014} compacting to ~{}",
                    pressure * 100.0,
                    target
                );
                compact_with_extraction(&mut messages, &memory_dir, target);
                homeostatic_state.cognitive.turns_since_compact = 0;
                let after = estimate_tokens(&messages);
                eprintln!("[context] now ~{after} tokens");
            }
        }
    }

    // --- Fire SessionEnd hooks ---
    if !hook_registry.is_empty() {
        hook_registry.fire(&HookEvent::SessionEnd, &hook_ctx);
    }

    Ok(())
}

/// Optional embedding context for dual-write (BRO-388).
struct EmbeddingContext<'a> {
    provider: &'a dyn crate::embedding::EmbeddingProvider,
    workspace_journal: &'a dyn Journal,
    workspace_session_id: &'a LagoSessionId,
    branch_id: &'a BranchId,
    workspace_seq: &'a SeqCounter,
}

/// Execute the agent loop: call provider, execute tools, repeat until done.
/// Returns the accumulated text response.
#[allow(clippy::print_stdout, clippy::print_stderr, clippy::too_many_arguments)]
fn run_agent_loop(
    provider: &Arc<dyn Provider>,
    registry: &ToolRegistry,
    tool_defs: &[ToolDefinition],
    messages: &mut Vec<ChatMessage>,
    cmd_ctx: &mut CommandContext,
    hook_registry: &HookRegistry,
    base_hook_ctx: &HookContext,
    nous_registry: Option<&EvaluatorRegistry>,
    embedding_ctx: Option<&EmbeddingContext<'_>>,
) -> anyhow::Result<String> {
    let run_id = format!("shell-{}", uuid::Uuid::new_v4());
    let session_id = "shell";
    let state = AppState::default();

    // Phase 8: Vigil span for the entire agent loop run (BRO-372)
    let _run_span = tracing::info_span!(
        "agent_loop_run",
        run_id = %run_id,
        session_id = session_id,
    )
    .entered();
    let mut accumulated_text = String::new();
    let max_iterations = 24;

    // Fire RunStart hooks
    hook_registry.fire(&HookEvent::RunStart, base_hook_ctx);

    for iteration in 1..=max_iterations {
        // Phase 8: Vigil span per iteration (BRO-372)
        let _iter_span = tracing::info_span!(
            "agent_loop_iteration",
            iteration = iteration,
            run_id = %run_id,
        )
        .entered();

        // --- Budget enforcement (BRO-364) ---
        if let Some(budget) = cmd_ctx.budget_usd {
            if cmd_ctx.session_cost_usd >= budget {
                eprintln!(
                    "\n[budget] Session cost ${:.4} has reached the ${:.2} budget limit. Stopping.",
                    cmd_ctx.session_cost_usd, budget
                );
                break;
            }
            let pct = cmd_ctx.session_cost_usd / budget;
            if pct >= 0.8 {
                eprintln!(
                    "\n[budget] Warning: {:.0}% of ${:.2} budget consumed (${:.4} spent)",
                    pct * 100.0,
                    budget,
                    cmd_ctx.session_cost_usd,
                );
            }
        }

        let request = ProviderRequest {
            run_id: run_id.clone(),
            session_id: session_id.to_string(),
            iteration,
            messages: messages.clone(),
            tools: tool_defs.to_vec(),
            state: state.clone(),
        };

        // Phase 8: Vigil span for provider call (BRO-372)
        let _provider_span = tracing::info_span!(
            "provider_call",
            provider = %cmd_ctx.provider_name,
            model = %cmd_ctx.model_name,
            iteration = iteration,
        )
        .entered();

        let spinner = crate::spinner::ShellSpinner::start();
        let spinner_state = Arc::clone(&spinner.state);
        let first_delta = std::sync::atomic::AtomicBool::new(true);
        let display_reasoning = cmd_ctx.show_reasoning;
        let md_renderer = std::cell::RefCell::new(crate::markdown::StreamingMarkdown::new());

        let turn = provider.complete_streaming(&request, &|delta| {
            match delta {
                StreamEvent::Reasoning(text) => {
                    // Switch spinner to reasoning phase so it shows token progress.
                    if first_delta.swap(false, std::sync::atomic::Ordering::Relaxed) {
                        let mut ft = spinner_state.first_token_at.lock().unwrap();
                        if ft.is_none() {
                            *ft = Some(std::time::Instant::now());
                        }
                        if display_reasoning {
                            // Stop spinner and start printing reasoning tokens.
                            spinner_state
                                .stop
                                .store(true, std::sync::atomic::Ordering::Relaxed);
                            std::thread::sleep(std::time::Duration::from_millis(60));
                            eprint!("\r\x1b[2K");
                            // Print header in dim style.
                            let mut out = std::io::stdout().lock();
                            let _ = write!(out, "\x1b[2m");
                            let _ = out.flush();
                        }
                    }
                    spinner_state.phase.store(
                        crate::spinner::PHASE_REASONING,
                        std::sync::atomic::Ordering::Relaxed,
                    );
                    // Count reasoning tokens.
                    let estimated = (text.len() as u64).div_ceil(4);
                    spinner_state
                        .tokens
                        .fetch_add(estimated, std::sync::atomic::Ordering::Relaxed);
                    // Optionally display reasoning tokens in dimmed style.
                    if display_reasoning {
                        let mut out = std::io::stdout().lock();
                        let _ = write!(out, "{text}");
                        let _ = out.flush();
                    }
                }
                StreamEvent::Text(text) => {
                    // On first text delta, stop the spinner render loop to prevent
                    // stderr spinner lines from interleaving with stdout text.
                    if first_delta.swap(false, std::sync::atomic::Ordering::Relaxed)
                        || spinner_state
                            .phase
                            .load(std::sync::atomic::Ordering::Relaxed)
                            != crate::spinner::PHASE_STREAMING
                    {
                        // Reset dim style if reasoning was displayed.
                        if display_reasoning {
                            let mut out = std::io::stdout().lock();
                            let _ = write!(out, "\x1b[0m\n\n");
                            let _ = out.flush();
                        }
                        // Stop the render thread — flowing text is its own feedback.
                        spinner_state
                            .stop
                            .store(true, std::sync::atomic::Ordering::Relaxed);
                        // Brief pause to let the render thread exit and clear its line.
                        std::thread::sleep(std::time::Duration::from_millis(60));
                        spinner_state.phase.store(
                            crate::spinner::PHASE_STREAMING,
                            std::sync::atomic::Ordering::Relaxed,
                        );
                        let mut ft = spinner_state.first_token_at.lock().unwrap();
                        if ft.is_none() {
                            *ft = Some(std::time::Instant::now());
                        }
                        // Clear any residual spinner line.
                        eprint!("\r\x1b[2K");
                    }
                    // Estimate tokens: ~4 chars per token
                    let estimated = (text.len() as u64).div_ceil(4);
                    spinner_state
                        .tokens
                        .fetch_add(estimated, std::sync::atomic::Ordering::Relaxed);
                    // Stream through markdown renderer for ANSI-styled output.
                    md_renderer.borrow_mut().push(text);
                }
            }
        })?;

        // Flush any remaining buffered markdown (partial final line).
        md_renderer.borrow_mut().flush();

        drop(_provider_span);

        // Finish spinner with cost info
        let turn_cost = if let Some(usage) = &turn.usage {
            estimate_cost(usage.input_tokens, usage.output_tokens, &cmd_ctx.model_name)
        } else {
            0.0
        };
        spinner.finish(turn_cost);

        // Track token usage with per-model cost estimation (BRO-364)
        if let Some(usage) = &turn.usage {
            cmd_ctx.session_input_tokens += usage.input_tokens;
            cmd_ctx.session_output_tokens += usage.output_tokens;
            cmd_ctx.session_cost_usd +=
                estimate_cost(usage.input_tokens, usage.output_tokens, &cmd_ctx.model_name);
        }

        // Process directives — accumulate text and collect tool calls.
        // Text is already printed by the streaming callback if any deltas
        // were received; only print here if nothing was streamed.
        let did_stream = !first_delta.load(std::sync::atomic::Ordering::Relaxed);
        let mut tool_calls: Vec<ToolCall> = Vec::new();
        for directive in &turn.directives {
            match directive {
                ModelDirective::Text { delta } => {
                    accumulated_text.push_str(delta);
                    if !did_stream {
                        print!("{delta}");
                        let _ = std::io::stdout().flush();
                    }
                }
                ModelDirective::FinalAnswer { text } => {
                    accumulated_text.push_str(text);
                    if !did_stream {
                        print!("{text}");
                        let _ = std::io::stdout().flush();
                    }
                }
                ModelDirective::ToolCall { call } => {
                    tool_calls.push(call.clone());
                }
                ModelDirective::StatePatch { .. } => {
                    // State patches are not used in shell mode
                }
            }
        }

        // If no tool calls, we're done
        if tool_calls.is_empty() || turn.stop_reason != ModelStopReason::ToolUse {
            if !accumulated_text.is_empty() {
                println!();
            } else if tool_calls.is_empty() {
                // Model returned nothing — likely context overflow or guardrail block
                eprintln!("(no response — model may have hit context limit or content filter)");
            }
            break;
        }

        // Track tool call count
        cmd_ctx.tool_call_count += tool_calls.len();

        // Build assistant message with tool_use content blocks.
        // The Anthropic API requires: assistant msg (with tool_use) → user msg (with tool_result).
        // Since ChatMessage.content is a String, we encode the tool_use blocks as JSON
        // that the provider can detect and parse as structured content.
        {
            let mut content_blocks = Vec::new();
            if !accumulated_text.is_empty() {
                content_blocks.push(serde_json::json!({
                    "type": "text",
                    "text": accumulated_text,
                }));
                accumulated_text.clear();
            }
            for call in &tool_calls {
                content_blocks.push(serde_json::json!({
                    "type": "tool_use",
                    "id": call.call_id,
                    "name": call.tool_name,
                    "input": call.input,
                }));
            }
            // Store as JSON array — the provider's build_messages will
            // detect this and use it as structured content blocks.
            messages.push(ChatMessage {
                role: arcan_core::protocol::Role::Assistant,
                content: serde_json::to_string(&content_blocks).unwrap_or_default(),
                tool_call_id: None,
            });
        }

        // Execute tool calls and collect results as a single user message
        // with tool_result content blocks (Anthropic API format).
        //
        // Phase 1 (sequential): permission checks and pre-tool hooks.
        // Phase 2 (parallel):   execute approved tools concurrently.
        // Phase 3 (sequential): fire post-tool hooks in order.
        let ctx = ToolContext {
            run_id: run_id.clone(),
            session_id: session_id.to_string(),
            iteration,
        };

        // result_blocks[i] corresponds to tool_calls[i]. Pre-fill with None;
        // denied/blocked calls are resolved in Phase 1, approved calls in Phase 2.
        let mut result_blocks: Vec<Option<serde_json::Value>> = vec![None; tool_calls.len()];

        // Indices of tool calls approved for execution after permission + hook gates.
        let mut approved_indices: Vec<usize> = Vec::new();
        // Hook contexts built per-call (needed in Phase 3 for post-tool hooks).
        let mut hook_contexts: Vec<Option<HookContext>> = vec![None; tool_calls.len()];

        // --- Phase 1: sequential permission checks and pre-tool hooks ---
        for (i, call) in tool_calls.iter().enumerate() {
            eprintln!("\n[tool: {}]", call.tool_name);

            // --- Permission check ---
            let is_read_only_annotation = tool_defs
                .iter()
                .find(|d| d.name == call.tool_name)
                .and_then(|d| d.annotations.as_ref())
                .is_some_and(|a| a.read_only);

            let auto_approved = is_tool_auto_approved(
                &call.tool_name,
                cmd_ctx.permission_mode,
                &cmd_ctx.session_approved_tools,
                is_read_only_annotation,
            );

            if !auto_approved {
                let choice = prompt_tool_permission(&call.tool_name);
                match choice {
                    'y' => { /* execute once */ }
                    'a' => {
                        cmd_ctx
                            .session_approved_tools
                            .insert(call.tool_name.clone());
                    }
                    _ => {
                        eprintln!("[tool: {}] DENIED by user", call.tool_name);
                        result_blocks[i] = Some(serde_json::json!({
                            "type": "tool_result",
                            "tool_use_id": call.call_id,
                            "content": format!("Permission denied: user declined to run '{}'", call.tool_name),
                            "is_error": true,
                        }));
                        continue;
                    }
                }
            }

            // Build hook context for this tool call
            let tool_hook_ctx = HookContext {
                session_id: base_hook_ctx.session_id.clone(),
                tool_name: Some(call.tool_name.clone()),
                tool_input: Some(call.input.clone()),
                workspace: base_hook_ctx.workspace.clone(),
            };

            // Fire PreToolUse blocking hooks — deny if any returns non-zero
            if let Err(denied) =
                hook_registry.check_blocking(&HookEvent::PreToolUse, &tool_hook_ctx)
            {
                eprintln!("[hook] blocked {}: {}", call.tool_name, denied.message);
                result_blocks[i] = Some(serde_json::json!({
                    "type": "tool_result",
                    "tool_use_id": call.call_id,
                    "content": format!("Blocked by hook: {}", denied.message),
                    "is_error": true,
                }));
                continue;
            }
            // Fire non-blocking PreToolUse hooks
            hook_registry.fire(&HookEvent::PreToolUse, &tool_hook_ctx);

            hook_contexts[i] = Some(tool_hook_ctx);
            approved_indices.push(i);
        }

        // --- Phase 2: parallel tool execution via std::thread::scope ---
        // Each approved tool runs in its own scoped thread. Results are
        // collected into a Vec indexed by position in approved_indices.
        let parallel_results: Vec<(usize, String, bool)> = if approved_indices.len() <= 1 {
            // Single tool — no threading overhead needed.
            approved_indices
                .iter()
                .map(|&i| {
                    let call = &tool_calls[i];
                    let tool_spinner = crate::spinner::ShellSpinner::start_tool(&call.tool_name);
                    let (content, is_error) = execute_tool(registry, call, &ctx);
                    tool_spinner.finish_tool(!is_error);
                    (i, content, is_error)
                })
                .collect()
        } else {
            std::thread::scope(|s| {
                let handles: Vec<_> = approved_indices
                    .iter()
                    .map(|&i| {
                        let call = &tool_calls[i];
                        let ctx_ref = &ctx;
                        s.spawn(move || {
                            let tool_spinner =
                                crate::spinner::ShellSpinner::start_tool(&call.tool_name);
                            let (content, is_error) = execute_tool(registry, call, ctx_ref);
                            tool_spinner.finish_tool(!is_error);
                            (i, content, is_error)
                        })
                    })
                    .collect();

                handles
                    .into_iter()
                    .map(|h| h.join().expect("tool thread panicked"))
                    .collect()
            })
        };

        // --- Phase 3: sequential post-tool hooks + Nous eval + assemble result blocks ---
        for (i, content, is_error) in parallel_results {
            let call = &tool_calls[i];
            let display = if content.len() > 200 {
                format!("{}... ({} bytes)", &content[..200], content.len())
            } else {
                content.clone()
            };
            if is_error {
                eprintln!("[tool: {}] ERROR: {display}", call.tool_name);
            } else {
                eprintln!("[tool: {}] OK: {display}", call.tool_name);
            }

            // Fire post-tool hooks (must be sequential — may have side effects)
            if let Some(ref tool_hook_ctx) = hook_contexts[i] {
                if is_error {
                    hook_registry.fire(&HookEvent::PostToolUseFailure, tool_hook_ctx);
                } else {
                    hook_registry.fire(&HookEvent::PostToolUse, tool_hook_ctx);
                }
            }

            // --- Nous post-tool evaluation (BRO-362) ---
            if let Some(registry) = nous_registry {
                let mut eval_ctx = EvalContext::new(session_id);
                eval_ctx.run_id = Some(run_id.clone());
                eval_ctx.iteration = Some(iteration);
                eval_ctx.tool_name = Some(call.tool_name.clone());
                eval_ctx.tool_errored = Some(is_error);
                eval_ctx.tool_call_count = Some(cmd_ctx.tool_call_count as u32);

                for evaluator in registry.evaluators_for(EvalHook::PostToolCall) {
                    match evaluator.evaluate(&eval_ctx) {
                        Ok(scores) => {
                            for score in &scores {
                                // Update running scores in cmd_ctx (BRO-363)
                                // Replace existing score for this evaluator or add new.
                                if let Some(existing) = cmd_ctx
                                    .nous_scores
                                    .iter_mut()
                                    .find(|(name, _)| name == &score.evaluator)
                                {
                                    existing.1 = score.value;
                                } else {
                                    cmd_ctx
                                        .nous_scores
                                        .push((score.evaluator.clone(), score.value));
                                }

                                if score.value < 0.5 {
                                    eprintln!(
                                        "[nous] Warning: {}: {:.2} — {}",
                                        score.evaluator,
                                        score.value,
                                        score
                                            .explanation
                                            .as_deref()
                                            .unwrap_or("score below threshold"),
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            tracing::debug!(
                                evaluator = evaluator.name(),
                                error = %e,
                                "nous eval error (non-fatal)"
                            );
                        }
                    }
                }
            }

            // --- BRO-388: Embed-on-write for memory_offload ---
            // When memory_offload succeeds and an embedding provider is configured,
            // embed the saved content and write to the workspace Lance journal.
            // This is the dual-write: filesystem (.md) + Lance (with vector).
            if call.tool_name == "memory_offload" && !is_error {
                if let Some(ectx) = embedding_ctx {
                    if let Some(content_text) = call.input.get("content").and_then(|v| v.as_str()) {
                        match ectx.provider.embed(content_text) {
                            Ok(embedding) => {
                                let title = call
                                    .input
                                    .get("title")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("memory");
                                let mem_event = EventEnvelope {
                                    event_id: lago_core::id::EventId::new(),
                                    session_id: ectx.workspace_session_id.clone(),
                                    branch_id: ectx.branch_id.clone(),
                                    run_id: None,
                                    seq: ectx.workspace_seq.next(),
                                    timestamp: EventEnvelope::now_micros(),
                                    parent_id: None,
                                    payload: lago_core::event::EventPayload::Message {
                                        role: "memory".to_string(),
                                        content: content_text.to_string(),
                                        model: None,
                                        token_usage: None,
                                    },
                                    metadata: {
                                        let mut m = std::collections::HashMap::new();
                                        m.insert("title".to_string(), title.to_string());
                                        m.insert(
                                            lago_lance::EMBEDDING_META_KEY.to_string(),
                                            serde_json::to_string(&embedding).unwrap_or_default(),
                                        );
                                        m
                                    },
                                    schema_version: 1,
                                };
                                append_event_sync(ectx.workspace_journal, mem_event);
                                tracing::debug!(
                                    title = title,
                                    dim = embedding.len(),
                                    "embedded memory offload to workspace lance"
                                );
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "embedding failed (non-fatal)");
                            }
                        }
                    }
                }
            }

            result_blocks[i] = Some(serde_json::json!({
                "type": "tool_result",
                "tool_use_id": call.call_id,
                "content": content,
                "is_error": is_error,
            }));
        }

        // Flatten Option<Value> → Value (all slots should be filled by now).
        let result_blocks: Vec<serde_json::Value> = result_blocks
            .into_iter()
            .map(|opt| opt.expect("tool result slot was not filled"))
            .collect();

        // Push tool results as a single user message with structured blocks.
        messages.push(ChatMessage {
            role: arcan_core::protocol::Role::User,
            content: serde_json::to_string(&result_blocks).unwrap_or_default(),
            tool_call_id: None,
        });
    }

    // Fire RunEnd hooks
    hook_registry.fire(&HookEvent::RunEnd, base_hook_ctx);

    Ok(accumulated_text)
}

/// Execute a single tool call against the registry, returning (content, is_error).
///
/// This is a pure helper extracted so it can be called from scoped threads
/// during parallel tool execution.
fn execute_tool(registry: &ToolRegistry, call: &ToolCall, ctx: &ToolContext) -> (String, bool) {
    // Phase 8: Vigil span per tool execution (BRO-372)
    let _tool_span = tracing::info_span!(
        "execute_tool",
        tool_name = %call.tool_name,
        tool_call_id = %call.call_id,
    )
    .entered();

    match registry.get(&call.tool_name) {
        Some(tool) => match tool.execute(call, ctx) {
            Ok(result) => {
                let output_str = match &result.output {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                tracing::debug!(tool_name = %call.tool_name, status = "ok", "tool completed");
                (output_str, false)
            }
            Err(e) => {
                tracing::warn!(tool_name = %call.tool_name, error = %e, "tool failed");
                (format!("Error: {e}"), true)
            }
        },
        None => {
            tracing::warn!(tool_name = %call.tool_name, "tool not found");
            (format!("Error: tool '{}' not found", call.tool_name), true)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arcan_core::protocol::Role;

    fn make_msg(role: Role, content: &str) -> ChatMessage {
        match role {
            Role::System => ChatMessage::system(content),
            Role::User => ChatMessage::user(content),
            Role::Assistant => ChatMessage::assistant(content),
            Role::Tool => ChatMessage::tool(content),
        }
    }

    #[test]
    fn test_estimate_tokens() {
        // Empty conversation
        assert_eq!(estimate_tokens(&[]), 0);

        // "hello" = 5 chars / 4 = 1 token
        let msgs = vec![make_msg(Role::User, "hello")];
        assert_eq!(estimate_tokens(&msgs), 1);

        // 400 chars / 4 = 100 tokens
        let msgs = vec![make_msg(Role::User, &"a".repeat(400))];
        assert_eq!(estimate_tokens(&msgs), 100);

        // Multiple messages
        let msgs = vec![
            make_msg(Role::System, &"s".repeat(40)),     // 10
            make_msg(Role::User, &"u".repeat(80)),       // 20
            make_msg(Role::Assistant, &"a".repeat(120)), // 30
        ];
        assert_eq!(estimate_tokens(&msgs), 60);
    }

    #[test]
    fn test_compact_preserves_recent() {
        let mut messages = vec![
            make_msg(Role::System, "You are a helpful assistant."),
            make_msg(Role::User, &"old question ".repeat(1000)),
            make_msg(Role::Assistant, &"old answer ".repeat(1000)),
            make_msg(Role::User, &"another old question ".repeat(1000)),
            make_msg(Role::Assistant, &"another old answer ".repeat(1000)),
            make_msg(Role::User, "recent question"),
            make_msg(Role::Assistant, "recent answer"),
        ];

        let before_len = messages.len();
        compact_conversation(&mut messages, 200);

        // Should have fewer messages than before
        assert!(
            messages.len() < before_len,
            "Should have compacted: {} < {}",
            messages.len(),
            before_len
        );

        // System message should be preserved as first
        assert_eq!(messages[0].role, Role::System);
        assert_eq!(messages[0].content, "You are a helpful assistant.");

        // Compaction marker should be second
        assert_eq!(messages[1].role, Role::System);
        assert!(messages[1].content.contains("compacted"));

        // Recent messages should be present
        assert!(messages.iter().any(|m| m.content == "recent answer"));
        assert!(messages.iter().any(|m| m.content == "recent question"));
    }

    #[test]
    fn test_compact_noop_under_threshold() {
        let mut messages = vec![
            make_msg(Role::System, "sys"),
            make_msg(Role::User, "hello"),
            make_msg(Role::Assistant, "hi"),
        ];

        let original = messages.clone();
        // Target much larger than current tokens — should be a no-op
        compact_conversation(&mut messages, 100_000);
        assert_eq!(messages.len(), original.len());
        for (a, b) in messages.iter().zip(original.iter()) {
            assert_eq!(a.content, b.content);
        }
    }

    #[test]
    fn test_compact_noop_few_messages() {
        // With 4 or fewer messages, compaction should be a no-op
        let mut messages = vec![
            make_msg(Role::System, &"s".repeat(100_000)),
            make_msg(Role::User, &"u".repeat(100_000)),
            make_msg(Role::Assistant, &"a".repeat(100_000)),
            make_msg(Role::User, &"u2".repeat(100_000)),
        ];

        let original_len = messages.len();
        compact_conversation(&mut messages, 100);
        assert_eq!(messages.len(), original_len);
    }

    #[test]
    fn test_compact_without_system_message() {
        let mut messages = vec![
            make_msg(Role::User, &"old ".repeat(5000)),
            make_msg(Role::Assistant, &"old reply ".repeat(5000)),
            make_msg(Role::User, &"old 2 ".repeat(5000)),
            make_msg(Role::Assistant, &"old reply 2 ".repeat(5000)),
            make_msg(Role::User, "recent"),
            make_msg(Role::Assistant, "recent reply"),
        ];

        compact_conversation(&mut messages, 200);

        // First message should be the compaction marker (no system msg to preserve)
        assert_eq!(messages[0].role, Role::System);
        assert!(messages[0].content.contains("compacted"));

        // Recent messages should be preserved
        assert!(messages.iter().any(|m| m.content == "recent"));
        assert!(messages.iter().any(|m| m.content == "recent reply"));
    }

    #[test]
    fn test_compact_command_in_registry() {
        let registry = arcan_commands::CommandRegistry::with_builtins();
        let mut ctx = arcan_commands::CommandContext::default();
        let result = registry.execute("/compact", &mut ctx);
        assert!(result.is_some());
        assert!(matches!(
            result.unwrap(),
            arcan_commands::CommandResult::CompactRequested
        ));
    }

    #[test]
    fn test_memory_command_in_registry() {
        let registry = arcan_commands::CommandRegistry::with_builtins();
        let mut ctx = arcan_commands::CommandContext::default();
        // /memory and /mem alias should both resolve
        let result = registry.execute("/memory", &mut ctx);
        assert!(result.is_some());

        let result = registry.execute("/mem", &mut ctx);
        assert!(result.is_some());
    }

    #[test]
    fn test_is_memory_signal() {
        // Bullet points
        assert!(is_memory_signal("- This is a key decision we made"));
        assert!(is_memory_signal("* Another bullet point summary"));

        // Headings
        assert!(is_memory_signal("## Architecture Overview"));
        assert!(is_memory_signal("### Key Findings"));

        // Signal words
        assert!(is_memory_signal("Decision: use redb for persistence"));
        assert!(is_memory_signal("Fixed the timeout bug in the agent loop"));
        assert!(is_memory_signal("TODO: wire up the approval workflow"));
        assert!(is_memory_signal("The root cause was a missing await"));

        // File paths
        assert!(is_memory_signal(
            "Modified crates/arcan/src/shell.rs to add memory"
        ));
        assert!(is_memory_signal(
            "Updated crates/arcan/Cargo.toml with new dependency"
        ));

        // Non-signals
        assert!(!is_memory_signal(""));
        assert!(!is_memory_signal("short"));
        assert!(!is_memory_signal("Hello, how can I help you today?"));
        assert!(!is_memory_signal("The weather is nice today and I like it"));
    }

    #[test]
    fn test_load_memory_context_no_dir() {
        let result = load_memory_context(std::path::Path::new("/nonexistent/dir/memory"));
        assert!(result.is_none());
    }

    #[test]
    fn test_load_memory_context_empty_dir() {
        let dir = std::env::temp_dir().join(format!(
            "arcan-mem-load-empty-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let result = load_memory_context(&dir);
        assert!(result.is_none());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn test_load_memory_context_with_files() {
        let dir = std::env::temp_dir().join(format!(
            "arcan-mem-load-files-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("session_summary.md"), "# Summary\nKey fact here").unwrap();
        std::fs::write(dir.join("global.md"), "# Global\nPersistent note").unwrap();
        // Non-md file should be ignored
        std::fs::write(dir.join("notes.txt"), "ignored").unwrap();

        let result = load_memory_context(&dir);
        assert!(result.is_some());
        let ctx = result.unwrap();
        assert!(ctx.contains("# Agent Memory (cross-session)"));
        assert!(ctx.contains("## global"));
        assert!(ctx.contains("Persistent note"));
        assert!(ctx.contains("## session_summary"));
        assert!(ctx.contains("Key fact here"));
        // .txt file should NOT appear
        assert!(!ctx.contains("ignored"));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn test_extract_and_save_memories() {
        let dir = std::env::temp_dir().join(format!(
            "arcan-mem-extract-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));

        // Messages with extractable content
        let messages = vec![
            make_msg(Role::User, "Fix the timeout bug"),
            make_msg(
                Role::Assistant,
                "- Found the root cause in the agent loop\n\
                 - Fixed the timeout by adding a retry\n\
                 Modified crates/arcan/src/shell.rs to handle edge case\n\
                 The weather is nice",
            ),
        ];

        extract_and_save_memories(&messages, &dir);

        let summary_path = dir.join("session_summary.md");
        assert!(summary_path.exists());
        let content = std::fs::read_to_string(&summary_path).unwrap();
        assert!(content.contains("# Session Summary"));
        assert!(content.contains("Found the root cause"));
        assert!(content.contains("Fixed the timeout"));
        assert!(content.contains("shell.rs"));
        // Non-signal line should not appear
        assert!(!content.contains("weather is nice"));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn test_extract_empty_messages() {
        let dir = std::env::temp_dir().join(format!(
            "arcan-mem-extract-empty-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));

        extract_and_save_memories(&[], &dir);
        // No file should be created for empty messages
        assert!(!dir.join("session_summary.md").exists());
    }

    // --- BRO-364: estimate_cost tests ---

    #[test]
    fn test_estimate_cost_opus() {
        let cost = estimate_cost(1_000_000, 1_000_000, "claude-opus-4-20250514");
        // $15 input + $75 output = $90
        assert!((cost - 90.0).abs() < 0.001);
    }

    #[test]
    fn test_estimate_cost_sonnet() {
        let cost = estimate_cost(1_000_000, 1_000_000, "claude-sonnet-4-20250514");
        // $3 input + $15 output = $18
        assert!((cost - 18.0).abs() < 0.001);
    }

    #[test]
    fn test_estimate_cost_haiku() {
        let cost = estimate_cost(1_000_000, 1_000_000, "claude-haiku-4-20250514");
        // $0.25 input + $1.25 output = $1.50
        assert!((cost - 1.50).abs() < 0.001);
    }

    #[test]
    fn test_estimate_cost_unknown_defaults_to_sonnet() {
        let cost = estimate_cost(1_000_000, 1_000_000, "some-unknown-model");
        // Should use sonnet pricing: $3 + $15 = $18
        assert!((cost - 18.0).abs() < 0.001);
    }

    #[test]
    fn test_estimate_cost_zero_tokens() {
        let cost = estimate_cost(0, 0, "claude-sonnet-4-20250514");
        assert!((cost).abs() < f64::EPSILON);
    }

    #[test]
    fn test_estimate_cost_typical_turn() {
        // A typical turn: 5000 input, 1000 output, sonnet
        let cost = estimate_cost(5000, 1000, "claude-sonnet-4-20250514");
        // (5000 * 3 + 1000 * 15) / 1_000_000 = (15000 + 15000) / 1_000_000 = 0.03
        assert!((cost - 0.03).abs() < 0.001);
    }

    // --- Phase 7: Identity tests (BRO-370) ---

    #[test]
    fn test_parse_identity_json_pro() {
        let json = r#"{"tier": "pro", "sub": "user@example.com"}"#;
        let info = parse_identity_json(json).unwrap();
        assert_eq!(info.tier, IdentityTier::Pro);
        assert_eq!(info.subject.as_deref(), Some("user@example.com"));
    }

    #[test]
    fn test_parse_identity_json_free() {
        let json = r#"{"tier": "free"}"#;
        let info = parse_identity_json(json).unwrap();
        assert_eq!(info.tier, IdentityTier::Free);
        assert!(info.subject.is_none());
    }

    #[test]
    fn test_parse_identity_json_enterprise() {
        let json = r#"{"tier": "enterprise", "sub": "admin@corp.com"}"#;
        let info = parse_identity_json(json).unwrap();
        assert_eq!(info.tier, IdentityTier::Enterprise);
        assert_eq!(info.subject.as_deref(), Some("admin@corp.com"));
    }

    #[test]
    fn test_parse_identity_json_unknown_tier_defaults_to_anonymous() {
        let json = r#"{"tier": "unknown_value"}"#;
        let info = parse_identity_json(json).unwrap();
        assert_eq!(info.tier, IdentityTier::Anonymous);
    }

    #[test]
    fn test_parse_identity_json_no_tier_defaults_to_anonymous() {
        let json = r#"{"sub": "user@example.com"}"#;
        let info = parse_identity_json(json).unwrap();
        assert_eq!(info.tier, IdentityTier::Anonymous);
    }

    #[test]
    fn test_parse_identity_json_invalid_json() {
        let json = "not valid json";
        assert!(parse_identity_json(json).is_none());
    }

    #[test]
    fn test_parse_identity_json_empty() {
        let json = "{}";
        let info = parse_identity_json(json).unwrap();
        assert_eq!(info.tier, IdentityTier::Anonymous);
        assert!(info.subject.is_none());
    }

    #[test]
    fn test_identity_tier_display() {
        assert_eq!(IdentityTier::Anonymous.to_string(), "anonymous");
        assert_eq!(IdentityTier::Free.to_string(), "free");
        assert_eq!(IdentityTier::Pro.to_string(), "pro");
        assert_eq!(IdentityTier::Enterprise.to_string(), "enterprise");
    }
}
