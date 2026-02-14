# The Harness Problem: How Arcan + Lago Address It

## Research Sources

- [Vercel: How to Build Agents with Filesystems and Bash](https://vercel.com/blog/how-to-build-agents-with-filesystems-and-bash)
- [can.ac: The Harness Problem](https://blog.can.ac/2026/02/12/the-harness-problem/)
- [OpenClaw: Agent Loop Concepts](https://docs.openclaw.ai/concepts/agent-loop)
- [Vercel AI SDK: Stream Protocol](https://ai-sdk.dev/docs/ai-sdk-ui/stream-protocol)

---

## 1. What Is the Harness Problem?

The harness is everything between "the model knows what to change" and "the change is applied." Tool schemas, error messages, edit formats, state management, sandboxing, streaming -- the infrastructure that translates model intent into workspace mutations.

The critical insight from can.ac's benchmarks: **edit format choice impacts success rates more than model quality**. Their data:

| Model | Patch Format | str_replace | Hashline |
|---|---|---|---|
| GPT-4 Turbo | 26% | ~40% | 59% |
| Grok Code Fast 1 | 6.7% | ~30% | 68.3% |
| Claude Sonnet 4 | ~45% | ~60% | ~65% |

A 10x improvement (6.7% to 68.3%) from harness design alone, with no model change.

Three edit strategies dominate the industry:

1. **Patch/diff** (OpenAI Codex): Unstructured diff strings. Assumes model was trained on patch format. Fails catastrophically on non-OpenAI models (46-50% failure rate).

2. **str_replace** (Claude Code): Exact old-text matching including whitespace. Fails when the target appears multiple times or when models can't reproduce exact whitespace. Generates frequent "String to replace not found" errors.

3. **Hashline** (arcan, can.ac benchmark winner): Content-hash-based line references. Models reference 8-char tags instead of reproducing content. Stale tags auto-reject. Output tokens decrease ~20% because models don't need to repeat existing code.

---

## 2. How Arcan Addresses Each Dimension

### 2.1 File Editing: Hashline Tags (Best in Class)

Arcan implements the hashline technique in `arcan-harness/src/edit.rs`:

- **Hash function**: BLAKE3 over `"{line_no}:{content}"`, truncated to 8 hex chars
- **Positional uniqueness**: Line number is part of the hash, so identical text at different lines produces different tags -- superior to content-only hashing
- **Operations**: `ReplaceLine`, `InsertAfterTag`, `DeleteLine` -- all reference tags, never raw content
- **Stale rejection**: If a tag doesn't match any current line, the edit fails before any mutation occurs
- **Sequential application**: Multiple edits in one request are applied in order; each re-hashes so later operations see the effect of earlier ones

**Arcan's hashline is the approach that won the can.ac benchmarks.** This is the single most impactful harness design choice.

**Current gaps to address:**
- No multiline block operations (replace N consecutive lines as a unit)
- No transactional rollback (if edit 3 of 5 fails, edits 1-2 are already applied)
- No diff preview before commit (show what would change without writing)

### 2.2 Filesystem Tools (Complete)

Arcan implements the full set that Vercel's blog identifies as essential:

| Tool | Arcan | Notes |
|---|---|---|
| `read_file` | ReadFileTool | With hashline tags for edit anchoring |
| `write_file` | WriteFileTool | Atomic full-file overwrite |
| `edit_file` | EditFileTool | Hashline-based safe editing |
| `list_dir` | ListDirTool | JSON output with file/dir types |
| `glob` | GlobTool | Pattern search across workspace |
| `grep` | GrepTool | Regex content search with glob filter |
| `bash` | BashTool | Sandboxed shell execution |
| `read_memory` | ReadMemoryTool | Persistent cross-session knowledge |
| `write_memory` | WriteMemoryTool | Durable key-value markdown store |

All filesystem tools enforce workspace boundaries through `FsPolicy` with path canonicalization (prevents symlink escape). This matches Vercel's recommendation of "security through isolation."

**Current gaps:**
- No `patch_file` tool for applying unified diffs (useful for large files >400 lines where Cursor's research shows diffs outperform full rewrites)
- No file watching / change notification (can't detect external modifications mid-session)

### 2.3 Agent Loop (Deterministic Orchestrator)

Arcan's `Orchestrator::run()` implements the core agent loop pattern that all sources describe:

```
load state -> call LLM with tools -> execute directives -> loop until stop
```

The orchestrator processes directives in strict order: Text, ToolCall, StatePatch, FinalAnswer. Each directive emits events immediately and updates state before the next iteration. Middleware hooks fire at 5 lifecycle points: before/after model call, before/after tool call, on run finished.

**Compared to OpenClaw's architecture:**

| Feature | OpenClaw | Arcan | Assessment |
|---|---|---|---|
| Session serialization | Per-session write locks | Single-threaded per run | Equivalent safety |
| Hook system | before/after agent, tool, compaction | 5 middleware hooks | Arcan is simpler but sufficient |
| Event streaming | lifecycle/assistant/tool channels | Single AgentEvent stream | Arcan is simpler |
| Compaction/retry | Auto-compaction with reset | Not implemented | Gap |
| Timeout enforcement | 600s default | Declared but not enforced | Gap |

**Current gaps:**
- No context window management (messages accumulate indefinitely until provider token limit)
- No compaction/summarization for long sessions
- No run cancellation (can't interrupt mid-stream)
- No per-iteration token tracking

### 2.4 Streaming Protocol (Multi-Format SSE)

Arcan supports streaming via SSE with format selection:

- **Native Arcan format**: All 10 `AgentEvent` variants serialized as JSON
- **AI SDK v5 format**: `AiSdkPart` mapping to Vercel's data stream protocol
- **Lago multi-format**: OpenAI, Anthropic, Vercel, Lago native via `SseBridge`

**Compared to Vercel AI SDK v1 UI Message Stream Protocol:**

| Protocol Part | Vercel SDK v1 | Arcan AiSdkPart | Status |
|---|---|---|---|
| `message-start` | Required | `Start` | Covered |
| `text-start` | Required | Missing | Gap |
| `text-delta` | Required | `TextDelta` | Covered |
| `text-end` | Required | Missing | Gap |
| `tool-input-start` | Required | `ToolCallBegin` | Covered |
| `tool-input-delta` | Required | `ToolCallDelta` | Covered |
| `tool-input-available` | Required | Missing | Gap |
| `tool-output-available` | Required | `ToolResult` | Covered |
| `start-step` | Required | Missing | Gap |
| `finish-step` | Required | Missing | Gap |
| `finish` | Required | `Finish` | Covered |
| `error` | Required | `Error` | Covered |
| `abort` | Optional | Missing | Gap |
| `reasoning-start/delta/end` | Optional | Missing | Gap |
| `source-url` | Optional | Missing | Minor |
| `file` | Optional | Missing | Minor |
| `data-*` | Extension | `ArcanStatePatch` | Arcan extension |

Arcan covers the core parts but misses boundary signals (`text-start`/`text-end`, `step` markers) and the newer reasoning chain support.

**Current gaps:**
- Missing text/step boundary signals (clients must infer boundaries)
- No incremental tool argument streaming (full args sent in one delta)
- No reasoning/extended thinking visualization
- No SSE event IDs for reconnection/dedup
- No `retry:` header for client retry strategy

### 2.5 Sandboxing (Process-Level Enforcement)

Arcan's `SandboxPolicy` declares comprehensive constraints:

```rust
SandboxPolicy {
    workspace_root,
    shell_enabled,
    network: NetworkPolicy,
    allowed_env: BTreeSet<String>,
    max_execution_ms,
    max_stdout_bytes, max_stderr_bytes,
    max_processes, max_memory_mb,
}
```

`LocalCommandRunner` enforces:
- Shell enable/disable gate
- **Environment variable filtering**: `env_clear()` + allow only listed vars. Empty `allowed_env` = allow none (fixed from previous bug where empty = allow all)
- **Working directory validation**: cwd canonicalized and checked against workspace_root. Rejects paths outside workspace with `PolicyViolation`
- **Execution timeout**: Uses `wait-timeout` crate. Kills process if `max_execution_ms` exceeded. Returns `SandboxError::Timeout`
- **Output size limits**: stdout/stderr truncated at policy limits with truncation marker

Not yet enforced (requires OS-level isolation):
- Network isolation (declared but no syscall/namespace blocking)
- Process count limits (declared but no `setrlimit`/cgroup)
- Memory limits (declared but no `setrlimit`/cgroup)

### 2.6 Persistence and Governance (Lago Integration)

This is where arcan + lago together provide capabilities that no single source above describes:

| Capability | Arcan + Lago | Industry Standard |
|---|---|---|
| ACID event journal | RedbJournal (embedded) | Most use SQLite or flat files |
| Content-addressed blobs | lago-store (SHA-256 + zstd) | Few implement this |
| Rule-based policy | PolicyEngine + Middleware | Most hardcode rules |
| Risk assessment | RiskLevel from ToolAnnotations | Not common |
| Session branching | parent_id + BranchId | Rare |
| State projection | Projection trait | Most replay naively |
| Multi-format SSE | OpenAI/Anthropic/Vercel/Lago | Most support one format |

The lago integration gives arcan a persistence and governance layer that exceeds what any of the four sources describe. The event-sourced architecture means every tool call, model output, and state mutation is durably recorded with ACID guarantees.

---

## 3. Scorecard

| Dimension | Industry Best | Arcan Status | Score |
|---|---|---|---|
| **Edit reliability** | Hashline (can.ac benchmark winner) | BLAKE3 hashline with positional uniqueness | 9/10 |
| **Filesystem tools** | read/write/edit/glob/grep/bash/memory | All implemented with workspace sandboxing | 9/10 |
| **Agent loop** | Deterministic, middleware, event-sourced | Correct orchestrator with 5 middleware hooks | 8/10 |
| **Streaming protocol** | Vercel AI SDK v1 full spec | Core parts covered, missing boundaries | 6/10 |
| **Sandbox enforcement** | Container/namespace isolation | Env filtering, cwd validation, timeout, output limits enforced | 7/10 |
| **Persistence** | ACID journal + policy governance | Lago integration (exceeds industry) | 10/10 |
| **Context management** | Sliding window, compaction | Not implemented | 2/10 |
| **MCP integration** | External tool bridge | Implemented via rmcp | 8/10 |
| **Skills/knowledge** | SKILL.md discovery + loading | Implemented with frontmatter + prompts | 8/10 |

**Overall: 7.4/10** -- Strong foundation with best-in-class editing and persistence. Sandbox enforcement improved from 4/10 to 7/10 (P0 fixes applied). Remaining gaps: container-level isolation and streaming protocol completeness.

---

## 4. Prioritized Plan

### P0: Security Fixes (Immediate) -- DONE

**4.1 Fix sandbox environment variable logic** -- DONE

Removed `|| policy.allowed_env.is_empty()` from env check. Empty `allowed_env` now correctly means "allow no request env vars." 3 tests verify behavior.

**4.2 Validate cwd against workspace_root** -- DONE

Added `validate_cwd()` that canonicalizes and checks against workspace_root. Rejects paths outside workspace with `PolicyViolation`. 3 tests verify boundary enforcement.

**4.3 Enforce execution timeout** -- DONE

Uses `wait-timeout` crate's `ChildExt::wait_timeout()`. Spawns child process, waits with `Duration::from_millis(max_execution_ms)`, kills on timeout. Returns `SandboxError::Timeout`. 1 test verifies kill behavior.

**4.4 Enforce output size limits** -- DONE

Added `truncate_output()` helper that caps output at policy limits and appends a truncation marker. Applied to both stdout and stderr. 4 tests verify truncation behavior.

### P1: Streaming Protocol Alignment (Short-term)

**4.5 Add boundary signals to AiSdkPart**

Add `TextStart`, `TextEnd`, `ToolInputAvailable`, `StartStep`, `FinishStep` variants to match the full Vercel AI SDK v1 spec. Update `to_aisdk_parts()` mapping.

**4.6 Add SSE event IDs**

Include monotonic event IDs in SSE frames for client-side reconnection and duplicate detection. The lago `SseBridge` already tracks sequence numbers -- surface them as SSE `id:` fields.

**4.7 Add reasoning chain support**

Add `ReasoningDelta` variant to `AgentEvent` for models that expose extended thinking. Map to `reasoning-start/delta/end` in AI SDK.

### P2: Context Management (Medium-term)

**4.8 Implement context window middleware**

Add a `ContextWindowMiddleware` that:
- Tracks cumulative token count across iterations
- Triggers summarization when approaching provider limits
- Optionally drops oldest messages (sliding window)
- Emits `ContextCompacted` events for audit

**4.9 Add token usage tracking to AgentEvent**

Extend `ModelOutput` event with `prompt_tokens` and `completion_tokens` fields. Populate from provider responses.

### P3: Edit System Enhancements (Medium-term)

**4.10 Multiline edit operations**

Add `ReplaceRange { start_tag, end_tag, new_text }` to `TaggedEditOp`. This covers the common case of replacing a block of lines (function body, config section) without needing one operation per line.

**4.11 Transactional edit batches**

Apply all edits to a copy of the content. Only write the file if all operations succeed. This prevents partial mutations when a batch has a stale tag in a later operation.

**4.12 Add diff-based editing for large files**

For files >400 lines, offer a `patch_file` tool that accepts unified diff format. Cursor's research shows diffs outperform full-file rewrites at scale.

### P4: Sandbox Hardening (Longer-term)

**4.13 Implement BubblewrapRunner**

Use `bwrap` (bubblewrap) for Linux namespace isolation:
- Mount workspace read-write, everything else read-only
- Drop network access per `NetworkPolicy`
- Set `RLIMIT_CPU`, `RLIMIT_AS` per policy
- Use PID namespace to enforce process limits

**4.14 Implement DockerRunner**

For maximum isolation, run tool commands in ephemeral containers:
- Mount workspace volume
- Apply resource limits via cgroup
- Network policy via docker network settings
- Clean up container after execution

### P5: Agent Loop Improvements (Longer-term)

**4.15 Run cancellation**

Add an `AbortSignal` (tokio `CancellationToken`) to the orchestrator. Check at the top of each iteration and after each tool call. Emit `RunCancelled` event on abort.

**4.16 Session compaction**

When message history exceeds a threshold, summarize older messages into a compact system message. Preserve tool results and state patches. Emit `SessionCompacted` event. This is what OpenClaw calls "auto-compaction with buffer reset."

**4.17 Parallel tool execution**

When the model requests multiple independent tool calls in one turn, execute them concurrently. Requires dependency analysis (tools that share state must be serialized).

---

## 5. What Arcan Gets Right That Others Don't

### 5.1 The Edit Format

Arcan's hashline implementation is the exact technique that won the can.ac benchmarks. The BLAKE3 hash with line-number positional encoding is superior to both OpenAI's patch format and Claude Code's str_replace. This is not incremental -- it's a fundamental architectural advantage.

### 5.2 Event-Sourced Persistence

Most agent systems use either in-memory state (lost on crash) or naive file logging. Arcan + lago provides ACID-guaranteed event sourcing with content-addressed blobs, sequence-numbered journals, and typed projections. This enables replay, branching, auditing, and time-travel debugging -- capabilities that don't exist in Vercel's approach or OpenClaw's architecture.

### 5.3 Policy as Data

The lago `PolicyEngine` evaluates rules against a `PolicyContext` that includes risk level, tool name, category, and session ID. Rules are composable (AND/OR/NOT conditions), prioritized, and logged. This is more sophisticated than hardcoded permission checks.

### 5.4 Multi-Format Streaming

Most agent systems output one wire format. Arcan supports native events, Vercel AI SDK, OpenAI Chat Completion chunks, Anthropic format, and Lago native -- all from the same event stream via `SseBridge`. This makes arcan compatible with any frontend framework.

### 5.5 Separation of Concerns

The bridge crate pattern (`arcan-lago`) keeps `arcan-core` free of persistence dependencies. Any backend can implement `SessionRepository`. Any provider can implement `Provider`. Any tool can implement `Tool`. This composability is rare in monolithic agent frameworks.
