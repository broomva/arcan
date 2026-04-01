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

/// Maximum number of lines to include in the session summary.
const SUMMARY_MAX_LINES: usize = 50;

/// Maximum number of characters from a single message to consider for extraction.
const EXTRACT_MAX_CHARS: usize = 2000;

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

/// Load memory context from `.arcan/memory/*.md` files.
///
/// Reads all markdown files from the memory directory and returns a formatted
/// string suitable for injection into the system prompt. Returns `None` if the
/// directory doesn't exist or contains no memory files.
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

        let content = if msg.content.len() > EXTRACT_MAX_CHARS {
            &msg.content[..EXTRACT_MAX_CHARS]
        } else {
            &msg.content
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
    registry.register(PraxisToolBridge::new(WriteMemoryTool::new(
        memory_dir.clone(),
    )));

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
    let provider_name = provider.name().to_string();
    let model_name = resolved
        .model
        .clone()
        .unwrap_or_else(|| "default".to_string());
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
        ..Default::default()
    };

    // --- Load cross-session memory into system prompt ---
    if let Some(memory_context) = load_memory_context(&memory_dir) {
        messages.push(ChatMessage::system(&memory_context));
    }

    // --- Inject skill catalog into system prompt ---
    if let Some(ref sr) = skill_registry {
        let catalog = crate::skills::build_system_prompt(sr);
        if !catalog.is_empty() {
            messages.push(ChatMessage::system(&catalog));
        }
    }

    // --- Welcome banner ---
    eprintln!("arcan shell v{}", env!("CARGO_PKG_VERSION"));
    eprintln!(
        "Provider: {} | Model: {} | Tools: {} | Hooks: {} | Skills: {} | Type /help for commands",
        provider_name,
        model_name,
        tool_defs.len(),
        hook_registry.len(),
        skill_names.len(),
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
            // Update message_count before command dispatch
            cmd_ctx.message_count = messages.len();

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

                    eprintln!("Unknown command: {input}. Type /help for available commands.");
                }
            }
            continue;
        }

        // --- Send to provider ---
        messages.push(ChatMessage::user(input));
        cmd_ctx.session_turns += 1;
        cmd_ctx.message_count = messages.len();

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
                // Update message count after loop completes
                cmd_ctx.message_count = messages.len();
                // Extract and save key facts from this turn to persistent memory.
                extract_and_save_memories(&messages, &memory_dir);
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
}
