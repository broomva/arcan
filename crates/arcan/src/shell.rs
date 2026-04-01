//! Single-process interactive REPL — no daemon needed.
//!
//! `arcan shell` builds the provider, tool registry, and command registry
//! in-process, then loops: read line, dispatch slash commands or send to the
//! LLM provider, execute tools inline, print response.

use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use aios_protocol::sandbox::NetworkPolicy;
use arcan_commands::{
    CommandContext, CommandRegistry, CommandResult, PermissionMode, is_tool_auto_approved,
    prompt_tool_permission,
};
use arcan_core::hooks::{HookContext, HookEvent, HookRegistry};
use arcan_core::protocol::{
    ChatMessage, ModelDirective, ModelStopReason, ToolCall, ToolDefinition,
};
use arcan_core::runtime::{Provider, ProviderRequest, ToolContext, ToolRegistry};
use arcan_core::state::AppState;
use arcan_harness::bridge::PraxisToolBridge;
use arcan_harness::{FsPolicy, LocalFs, SandboxPolicy};
use praxis_tools::edit::EditFileTool;
use praxis_tools::fs::{GlobTool, GrepTool, ListDirTool, ReadFileTool, WriteFileTool};
use praxis_tools::memory::{ReadMemoryTool, WriteMemoryTool};
use praxis_tools::shell::BashTool;
use std::collections::BTreeSet;

use crate::config::ResolvedConfig;

/// Token threshold above which auto-compaction triggers after each agent turn.
const COMPACT_THRESHOLD: usize = 100_000;

/// Target token count after compaction.
const COMPACT_TARGET: usize = 50_000;

/// Estimate total token count for a message list using a character-based heuristic.
///
/// Uses ~4 characters per token as a rough approximation.
fn estimate_tokens(messages: &[ChatMessage]) -> usize {
    messages.iter().map(|m| m.content.len() / 4).sum()
}

/// Compact a conversation to fit within a target token budget.
///
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

/// Run the interactive shell REPL.
#[allow(clippy::print_stderr, clippy::print_stdout)]
pub fn run_shell(
    data_dir: &Path,
    resolved: &ResolvedConfig,
    _session: Option<String>,
    yes: bool,
) -> anyhow::Result<()> {
    let workspace_root = std::env::current_dir()?;

    // --- Provider ---
    let provider = crate::build_provider(resolved)?;

    // --- Tools (same set as run_serve) ---
    let mut registry = ToolRegistry::default();

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

    let memory_dir = data_dir.join("memory");
    std::fs::create_dir_all(&memory_dir)?;
    registry.register(PraxisToolBridge::new(ReadMemoryTool::new(
        memory_dir.clone(),
    )));
    registry.register(PraxisToolBridge::new(WriteMemoryTool::new(memory_dir)));

    // --- Command registry ---
    let commands = CommandRegistry::with_builtins();
    let tool_defs = registry.definitions();

    // --- Hook registry ---
    let hook_registry = &resolved.hook_registry;
    let session_id = format!("shell-{}", uuid::Uuid::new_v4());
    let hook_ctx = HookContext {
        session_id: session_id.clone(),
        tool_name: None,
        tool_input: None,
        workspace: workspace_root.display().to_string(),
    };

    // --- Fire SessionStart hooks ---
    if !hook_registry.is_empty() {
        hook_registry.fire(&HookEvent::SessionStart, &hook_ctx);
    }

    // --- Session state ---
    let mut messages: Vec<ChatMessage> = Vec::new();
    let permission_mode = if yes {
        PermissionMode::Yes
    } else {
        PermissionMode::Default
    };
    let mut cmd_ctx = CommandContext {
        workspace: workspace_root,
        permission_mode,
        ..Default::default()
    };

    // --- Welcome banner ---
    eprintln!("arcan shell v{}", env!("CARGO_PKG_VERSION"));
    eprintln!(
        "Provider: {} | Tools: {} | Hooks: {} | Type /help for commands",
        provider.name(),
        tool_defs.len(),
        hook_registry.len(),
    );
    eprintln!();

    // --- REPL loop ---
    let stdin = std::io::stdin();
    loop {
        eprint!("arcan> ");
        std::io::stderr().flush().ok();

        let mut line = String::new();
        match stdin.read_line(&mut line) {
            Ok(0) => break, // EOF
            Ok(_) => {}
            Err(e) => {
                eprintln!("read error: {e}");
                break;
            }
        }

        let input = line.trim();
        if input.is_empty() {
            continue;
        }

        // --- Slash command dispatch ---
        if input.starts_with('/') {
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
                    cmd_ctx.session_approved_tools.clear();
                    eprintln!("Session cleared.");
                }
                Some(CommandResult::CompactRequested) => {
                    let before = estimate_tokens(&messages);
                    compact_conversation(&mut messages, COMPACT_TARGET);
                    let after = estimate_tokens(&messages);
                    eprintln!("[compact] {before} tokens -> {after} tokens");
                }
                Some(CommandResult::Quit) => {
                    eprintln!("Goodbye.");
                    break;
                }
                Some(CommandResult::Error(err)) => {
                    eprintln!("Error: {err}");
                }
                None => {
                    eprintln!("Unknown command: {input}. Type /help for available commands.");
                }
            }
            continue;
        }

        // --- Send to provider ---
        messages.push(ChatMessage::user(input));
        cmd_ctx.session_turns += 1;

        let response_text = run_agent_loop(
            &provider,
            &registry,
            &tool_defs,
            &mut messages,
            &mut cmd_ctx,
            hook_registry,
            &hook_ctx,
        );

        match response_text {
            Ok(text) => {
                if !text.is_empty() {
                    messages.push(ChatMessage::assistant(&text));
                }
            }
            Err(e) => {
                eprintln!("Error: {e}");
            }
        }

        // --- Auto-compact if conversation exceeds threshold ---
        let tokens = estimate_tokens(&messages);
        if tokens > COMPACT_THRESHOLD {
            eprintln!("[compact] {tokens} tokens -> compacting to ~{COMPACT_TARGET}");
            compact_conversation(&mut messages, COMPACT_TARGET);
            eprintln!("[compact] now ~{} tokens", estimate_tokens(&messages));
        }
    }

    // --- Fire SessionEnd hooks ---
    if !hook_registry.is_empty() {
        hook_registry.fire(&HookEvent::SessionEnd, &hook_ctx);
    }

    Ok(())
}

/// Execute the agent loop: call provider, execute tools, repeat until done.
/// Returns the accumulated text response.
#[allow(clippy::print_stdout, clippy::print_stderr)]
fn run_agent_loop(
    provider: &Arc<dyn Provider>,
    registry: &ToolRegistry,
    tool_defs: &[ToolDefinition],
    messages: &mut Vec<ChatMessage>,
    cmd_ctx: &mut CommandContext,
    hook_registry: &HookRegistry,
    base_hook_ctx: &HookContext,
) -> anyhow::Result<String> {
    let run_id = format!("shell-{}", uuid::Uuid::new_v4());
    let session_id = "shell";
    let state = AppState::default();
    let mut accumulated_text = String::new();
    let max_iterations = 24;

    // Fire RunStart hooks
    hook_registry.fire(&HookEvent::RunStart, base_hook_ctx);

    for iteration in 1..=max_iterations {
        let request = ProviderRequest {
            run_id: run_id.clone(),
            session_id: session_id.to_string(),
            iteration,
            messages: messages.clone(),
            tools: tool_defs.to_vec(),
            state: state.clone(),
        };

        let turn = provider.complete_streaming(&request, &|delta| {
            let mut out = std::io::stdout().lock();
            let _ = write!(out, "{delta}");
            let _ = out.flush();
        })?;

        // Track token usage
        if let Some(usage) = &turn.usage {
            cmd_ctx.session_input_tokens += usage.input_tokens;
            cmd_ctx.session_output_tokens += usage.output_tokens;
            // Rough cost estimate (Claude pricing: $3/MTok input, $15/MTok output)
            cmd_ctx.session_cost_usd +=
                (usage.input_tokens as f64 * 3.0 + usage.output_tokens as f64 * 15.0) / 1_000_000.0;
        }

        // Process directives — accumulate text and collect tool calls.
        // Text is already printed by the streaming callback if the provider
        // supports streaming; only print here for non-streaming providers.
        let is_streaming = provider.supports_streaming();
        let mut tool_calls: Vec<ToolCall> = Vec::new();
        for directive in &turn.directives {
            match directive {
                ModelDirective::Text { delta } => {
                    accumulated_text.push_str(delta);
                    if !is_streaming {
                        print!("{delta}");
                        let _ = std::io::stdout().flush();
                    }
                }
                ModelDirective::FinalAnswer { text } => {
                    accumulated_text.push_str(text);
                    if !is_streaming {
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
            }
            break;
        }

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
                    let (content, is_error) = execute_tool(registry, call, &ctx);
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
                            let (content, is_error) = execute_tool(registry, call, ctx_ref);
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

        // --- Phase 3: sequential post-tool hooks + assemble result blocks ---
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
    match registry.get(&call.tool_name) {
        Some(tool) => match tool.execute(call, ctx) {
            Ok(result) => {
                let output_str = match &result.output {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                (output_str, false)
            }
            Err(e) => (format!("Error: {e}"), true),
        },
        None => (format!("Error: tool '{}' not found", call.tool_name), true),
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
}
