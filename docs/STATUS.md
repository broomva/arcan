# Arcan Project Status

> **This is the single source of truth** for what's implemented, what's missing, and what's planned.
> Last updated: 2026-02-14

---

## 1. Project Overview

Arcan is a **Rust-based agent daemon** designed for reliability, streaming, and secure tool execution. It draws from Vercel AI SDK, Claude Code, and modern agentic systems.

**Design philosophy**: The agent's message history IS the application state. Every action produces immutable events; the system's state is a projection of its event log.

### Workspace Structure

```
arcan-rs/
  crates/
    arcan-core/       Traits, protocol types, orchestrator, AI SDK mapping
    arcan-harness/    Tools (9), sandbox, MCP bridge, skills
    arcan-store/      Session persistence (InMemory, JSONL)
    arcan-provider/   LLM adapters (Anthropic, Rig bridge)
    arcand/           Agent loop library, Axum HTTP/SSE server, mock provider
    arcan-lago/       Lago bridge (ACID journal, policy, SSE multi-format)
    arcan/            Production binary (unified daemon with Lago)
```

### Crate Summary

| Crate | Purpose | Tests | Key Exports |
|---|---|---|---|
| `arcan-core` | Foundation: traits, protocol, state, AI SDK | 35 | `Orchestrator`, `Provider`, `Tool`, `Middleware`, `AppState`, `AgentEvent`, `TokenUsage` |
| `arcan-harness` | Tools, sandbox, MCP bridge, skills | 39 | 9 tool impls, `FsPolicy`, `SandboxPolicy`, `SkillRegistry`, `McpTool` |
| `arcan-store` | Persistence backends | 7 | `SessionRepository`, `InMemorySessionRepository`, `JsonlSessionRepository` |
| `arcan-provider` | LLM adapters | 9 | `AnthropicProvider`, `RigProvider`, `anthropic_rig_provider()` |
| `arcand` | Server infrastructure | 0 | `AgentLoop`, `create_router()`, `MockProvider` |
| `arcan-lago` | Lago persistence bridge | 33 | `LagoSessionRepository`, `LagoPolicyMiddleware`, `AppStateProjection`, `SseBridge` |
| `arcan` | Production daemon binary | 0 | CLI entry point with Lago + policy |

**Total: 118 tests, all passing, clippy clean.**

### Dependency Graph

```
arcan (production binary)
  +-- arcand (server library)
  |     +-- arcan-core
  |     +-- arcan-harness  --> arcan-core
  |     +-- arcan-provider --> arcan-core
  |     +-- arcan-store    --> arcan-core
  +-- arcan-lago (bridge)
  |     +-- arcan-core
  |     +-- arcan-store
  |     +-- lago-{core, journal, store, api, policy}
  +-- lago-journal, lago-store, lago-policy (direct)
```

`arcan-core` has zero dependency on lago. Only `arcan-lago` and `arcan` depend on lago crates.

---

## 2. Implementation Status Matrix

### 2.1 Core Runtime

| Feature | Status | Location | Tests | Notes |
|---|---|---|---|---|
| Orchestrator loop | Done | `arcan-core/src/runtime.rs` | 8 | Deterministic: directives processed in order |
| Provider trait | Done | `arcan-core/src/runtime.rs` | - | Synchronous `complete()` |
| Tool trait | Done | `arcan-core/src/runtime.rs` | - | `definition()` + `execute()` |
| Middleware trait (5 hooks) | Done | `arcan-core/src/runtime.rs` | 1 | before/after model, pre/post tool, on_run_finished |
| ToolRegistry | Done | `arcan-core/src/runtime.rs` | - | BTreeMap-based, register/get/definitions |
| ToolAnnotations (MCP-aligned) | Done | `arcan-core/src/protocol.rs` | - | read_only, destructive, idempotent, open_world, requires_confirmation |
| Budget control (max_iterations) | Done | `arcan-core/src/runtime.rs` | 1 | Configurable, default 24 |
| RunStopReason (6 variants) | Done | `arcan-core/src/protocol.rs` | - | Completed, NeedsUser, BlockedByPolicy, BudgetExceeded, Cancelled, Error |
| Run cancellation (AtomicBool) | Done | `arcan-core/src/runtime.rs` | 1 | `run_cancellable()` checks flag at iteration boundaries |
| Token usage tracking | Done | `arcan-core/src/protocol.rs` | 1 | `TokenUsage` struct, accumulated in `RunOutput.total_usage` |
| Parallel tool execution | Not Done | - | - | Tools execute sequentially |

### 2.2 Protocol Types

| Feature | Status | Location | Notes |
|---|---|---|---|
| AgentEvent (10 variants) | Done | `arcan-core/src/protocol.rs` | RunStarted through RunFinished |
| ChatMessage with tool_call_id | Done | `arcan-core/src/protocol.rs` | Proper tool result attribution |
| ModelDirective (4 variants) | Done | `arcan-core/src/protocol.rs` | Text, ToolCall, StatePatch, FinalAnswer |
| ModelStopReason (6 variants) | Done | `arcan-core/src/protocol.rs` | EndTurn, ToolUse, NeedsUser, MaxTokens, Safety, Unknown |
| StatePatch (JSON Patch + Merge) | Done | `arcan-core/src/state.rs` | RFC 6902 + RFC 7396 |
| ToolContent (MCP-compatible) | Done | `arcan-core/src/protocol.rs` | Text, Image, Json |

### 2.3 Filesystem Tools

| Tool | Status | Location | Annotations |
|---|---|---|---|
| `read_file` | Done | `arcan-harness/src/fs.rs` | read_only, idempotent |
| `write_file` | Done | `arcan-harness/src/fs.rs` | destructive |
| `list_dir` | Done | `arcan-harness/src/fs.rs` | read_only, idempotent |
| `edit_file` | Done | `arcan-harness/src/edit.rs` | destructive |
| `glob` | Done | `arcan-harness/src/fs.rs` | read_only, idempotent |
| `grep` | Done | `arcan-harness/src/fs.rs` | read_only, idempotent |
| `bash` | Done | `arcan-harness/src/sandbox.rs` | destructive, open_world, requires_confirmation |
| `read_memory` | Done | `arcan-harness/src/memory.rs` | read_only |
| `write_memory` | Done | `arcan-harness/src/memory.rs` | - |

### 2.4 Edit System

| Feature | Status | Location | Notes |
|---|---|---|---|
| BLAKE3 hashline (positional) | Done | `arcan-harness/src/edit.rs` | 8-char tag from blake3(line_no:content) |
| ReplaceLine | Done | `arcan-harness/src/edit.rs` | Reference by tag |
| InsertAfterTag | Done | `arcan-harness/src/edit.rs` | Reference by tag |
| DeleteLine | Done | `arcan-harness/src/edit.rs` | Reference by tag |
| Stale tag rejection | Done | `arcan-harness/src/edit.rs` | Fails before any mutation |
| Sequential multi-edit | Done | `arcan-harness/src/edit.rs` | Later ops see effect of earlier ones |
| ReplaceRange (multiline) | Not Done | - | Replace N consecutive lines as a unit |
| Transactional batches | Not Done | - | If op 3/5 fails, ops 1-2 already applied |
| Diff-based editing (>400 lines) | Not Done | - | `patch_file` tool for large files |

### 2.5 Sandbox

| Feature | Status | Location | Notes |
|---|---|---|---|
| FsPolicy workspace boundary | Done | `arcan-harness/src/fs.rs` | Canonicalize + starts_with |
| SandboxPolicy (all fields) | Done | `arcan-harness/src/sandbox.rs` | 9 constraint fields |
| Shell enable/disable gate | Done | `arcan-harness/src/sandbox.rs` | - |
| Environment variable filtering | Done | `arcan-harness/src/sandbox.rs` | Empty allowed_env = deny all (fixed) |
| CWD workspace validation | Done | `arcan-harness/src/sandbox.rs` | Canonicalize + reject outside root |
| Execution timeout | Done | `arcan-harness/src/sandbox.rs` | wait-timeout crate, kills on expiry |
| Output size truncation | Done | `arcan-harness/src/sandbox.rs` | Truncation marker appended |
| Network isolation | Not Done | - | Declared but no enforcement |
| Process count limits | Not Done | - | Needs setrlimit/cgroup |
| Memory limits | Not Done | - | Needs setrlimit/cgroup |
| BubblewrapRunner | Not Done | - | Linux namespace isolation |
| DockerRunner | Not Done | - | Container-based isolation |

### 2.6 Providers

| Provider | Status | Location | Streaming | Tests |
|---|---|---|---|---|
| MockProvider | Done | `arcand/src/mock.rs` | No | 0 |
| AnthropicProvider | Done | `arcan-provider/src/anthropic.rs` | Full + tool use + usage tracking | 9 |
| RigProvider bridge | Done | `arcan-provider/src/rig_bridge.rs` | Via rig-core | 0 |
| OpenAI Provider | Not Done | - | - | - |

### 2.7 Streaming Protocol

| Feature | Status | Location | Notes |
|---|---|---|---|
| Native AgentEvent SSE | Done | `arcand/src/server.rs` | 10 variants as JSON, `/health` endpoint |
| AiSdkPart mapping (v5) | Done | `arcan-core/src/aisdk.rs` | 8 part types |
| Lago multi-format SSE | Done | `arcan-lago/src/sse_bridge.rs` | OpenAI, Anthropic, Vercel, Lago |
| Format query param | Done | `arcand/src/server.rs` | `?format=aisdk_v5` |
| text-start / text-end | Not Done | - | Clients must infer boundaries |
| start-step / finish-step | Not Done | - | No step markers |
| tool-input-available | Not Done | - | Missing from AI SDK mapping |
| SSE event IDs | Not Done | - | No reconnection support |
| SSE retry: header | Not Done | - | No client retry strategy |
| reasoning-start/delta/end | Not Done | - | No extended thinking support |
| abort signal | Not Done | - | No client-initiated abort |

### 2.8 Persistence

| Feature | Status | Location | Tests |
|---|---|---|---|
| SessionRepository trait | Done | `arcan-store/src/session.rs` | - |
| InMemorySessionRepository | Done | `arcan-store/src/session.rs` | 5 |
| JsonlSessionRepository | Done | `arcan-store/src/session.rs` | 2 |
| LagoSessionRepository (ACID) | Done | `arcan-lago/src/repository.rs` | 4 |
| Event sourcing (append-only) | Done | All backends | - |
| EventRecord with parent_id | Done | `arcan-store/src/session.rs` | 1 |
| Bidirectional event mapping | Done | `arcan-lago/src/event_map.rs` | 9 |
| AppStateProjection | Done | `arcan-lago/src/state_projection.rs` | 6 |
| Session fork API | Not Done | - | parent_id exists but no API |
| Session compaction/snapshots | Not Done | - | Long sessions accumulate indefinitely |

### 2.9 Policy and Governance

| Feature | Status | Location | Tests |
|---|---|---|---|
| LagoPolicyMiddleware | Done | `arcan-lago/src/policy_middleware.rs` | 6 |
| Risk level derivation | Done | `arcan-lago/src/policy_middleware.rs` | Included |
| Rule-based tool blocking | Done | `arcan-lago/src/policy_middleware.rs` | Included |
| RequireApproval decision | Partial | `arcan-lago/src/policy_middleware.rs` | Returns error, no interactive flow |
| Approval workflow (interactive) | Not Done | - | No pause/resume mechanism |

### 2.10 MCP and Skills

| Feature | Status | Location | Tests |
|---|---|---|---|
| MCP stdio bridge (rmcp) | Done | `arcan-harness/src/mcp.rs` | - |
| McpTool wrapper | Done | `arcan-harness/src/mcp.rs` | - |
| MCP annotation mapping | Done | `arcan-harness/src/mcp.rs` | - |
| SKILL.md discovery | Done | `arcan-harness/src/skills.rs` | - |
| System prompt injection | Done | `arcan-harness/src/skills.rs` | - |
| Allowed tools filtering | Done | `arcan-harness/src/skills.rs` | - |

### 2.11 Infrastructure

| Feature | Status | Location | Notes |
|---|---|---|---|
| CI (GitHub Actions) | Done | `.github/workflows/ci.yml` | fmt, clippy, test, build, MSRV, audit, deny |
| Pre-commit hooks | Done | `.claude/settings.local.json` | cargo fmt --check + cargo check |
| MSRV pinned | Done | `Cargo.toml` | 1.80.0 |
| cargo-deny | Done | `deny.toml` | License and advisory checks |
| Dockerfile | Done | `Dockerfile` | Builds `arcan` binary, installs curl, HEALTHCHECK works |
| Release workflow | Done | `.github/workflows/release.yml` | Cross-platform builds |

---

## 3. Scorecard

| Dimension | Score | Justification |
|---|---|---|
| **Edit reliability** | 9/10 | BLAKE3 hashline with positional uniqueness. Can.ac benchmark winner. Missing: multiline blocks, transactional batches |
| **Filesystem tools** | 9/10 | All 9 tools with workspace sandboxing. Missing: patch_file for large files |
| **Agent loop** | 9/10 | Deterministic orchestrator, 5 middleware hooks, budget control, cancellation, token tracking. Missing: parallel tools |
| **Persistence** | 10/10 | ACID via Lago + JSONL + InMemory. Content-addressed blobs. Exceeds industry standard |
| **MCP integration** | 8/10 | Full rmcp bridge with annotation mapping. Missing: HTTP transport |
| **Skills/knowledge** | 8/10 | SKILL.md discovery + frontmatter + prompt injection. Missing: skill versioning |
| **Sandbox enforcement** | 7/10 | Env filtering, cwd validation, timeout, output truncation. Missing: OS-level isolation |
| **Provider coverage** | 7/10 | Anthropic (full streaming), Rig bridge (any model). Missing: native OpenAI, retry/backoff |
| **Streaming protocol** | 6/10 | Core AI SDK v5 + multi-format Lago SSE. Missing: boundary signals, step markers, reconnection IDs |
| **Testing** | 6/10 | 121 unit tests across 5 crates. Zero integration tests for arcand/arcan. Zero end-to-end tests |
| **Documentation** | 5/10 | 2 of 6 docs stale. AGENTS.md missing crates. No operational guide |
| **Context management** | 2/10 | Messages accumulate indefinitely. No compaction, no sliding window, no token counting |

**Overall: 7.1/10** -- Strong technical foundation with best-in-class editing and persistence. Primary gaps: context management, integration testing, documentation freshness.

---

## 4. Gap Analysis

### Critical (blocks production use)

1. **Context window management** -- Sessions will exceed provider token limits with no mitigation. No sliding window, no compaction, no token counting.
2. **No `/health` endpoint** -- Dockerfile HEALTHCHECK assumes one but neither binary implements it.
3. **Dockerfile targets wrong binary** -- Builds `arcand` library binary, but `arcan` is the production daemon.
4. **No integration tests** -- arcand and arcan have zero tests. AgentLoop, SSE streaming, and end-to-end flows are untested.
5. **Run cancellation** -- No way to stop a runaway agent. No CancellationToken in the orchestrator loop.

### High (significantly limits functionality)

6. **Streaming boundary signals** -- Missing text-start/end, step markers. Breaks Vercel AI SDK `useChat` compatibility.
7. **SSE event IDs** -- Clients cannot reconnect after network interruption. No duplicate detection.
8. **Approval workflow** -- Destructive tools execute without user confirmation. `RequireApproval` returns error instead of pausing.
9. **Token usage tracking** -- No visibility into prompt/completion token counts. Can't monitor cost per run.
10. **OpenAI provider** -- Limits model selection to Anthropic and rig-supported models.

### Medium (quality and safety improvements)

11. **OS-level sandbox isolation** -- BubblewrapRunner/DockerRunner for process/network/memory limits.
12. **Transactional edit batches** -- Partial file mutations if later operation in batch fails.
13. **Multiline edit operations** -- ReplaceRange for replacing function bodies, config sections.
14. **Diff-based editing** -- patch_file tool for files >400 lines where full rewrites are fragile.
15. **Session fork API** -- parent_id field exists but no fork/branch endpoint or semantics.
16. **Parallel tool execution** -- Sequential-only limits throughput when model requests multiple independent tools.

### Low (nice to have)

17. **CLI client** -- No terminal chat interface. HTTP/SSE only.
18. **Web client** -- No browser-based frontend.
19. **Subagent execution** -- No nested agent loops with restricted toolsets.
20. **Session compaction** -- Old messages are never summarized or dropped.
21. **Reasoning chain visualization** -- No reasoning-start/delta/end events for extended thinking.

---

## 5. Roadmap

> Phases use alphabetic labels (A-G) to supersede conflicting numeric phases in older docs.

### Phase A: Stabilization

**Goal**: Fix infrastructure issues that block production deployment.

Deliverables:
- Add `/health` endpoint to arcand server (returns 200 OK with version)
- Update Dockerfile to build `arcan` binary (not `arcand`)
- Update release workflow to produce `arcan` binary
- Create this STATUS.md document
- Mark `architecture.md` and `roadmap.md` as superseded
- Update `AGENTS.md` to reflect current 7-crate structure

Exit criteria:
- Docker image builds and passes HEALTHCHECK
- All docs internally consistent

Dependencies: None

### Phase B: Context and Cancellation

**Goal**: Enable long sessions and safe interruption.

Deliverables:
- `ContextWindowMiddleware`: tracks cumulative tokens, triggers compaction or sliding window
- Token counting from AnthropicProvider API responses (prompt_tokens, completion_tokens)
- `prompt_tokens` and `completion_tokens` fields on `ModelOutput` event
- `CancellationToken` (tokio) threaded through `Orchestrator::run()`, checked at iteration and tool call boundaries
- `RunCancelled` event variant
- Integration tests for `AgentLoop`

Exit criteria:
- Long sessions (>100 messages) don't exceed provider token limits
- Running agents can be cancelled mid-stream
- 5+ integration tests for AgentLoop

Dependencies: None (can run in parallel with Phase A)

### Phase C: Streaming Protocol Alignment

**Goal**: Full Vercel AI SDK v1 compatibility for frontend integration.

Deliverables:
- `TextStart`, `TextEnd`, `ToolInputAvailable`, `StartStep`, `FinishStep` variants in `AiSdkPart`
- Update `to_aisdk_parts()` to emit boundary signals at correct lifecycle points
- Monotonic SSE event IDs on all frames
- `retry:` header for client reconnection
- `ReasoningDelta` variant for extended thinking

Exit criteria:
- Vercel AI SDK `useChat` hook works without custom parsing
- Clients can reconnect after network interruption

Dependencies: Phase B (token fields inform usage in Finish part)

### Phase D: Safety and Testing

**Goal**: Interactive approval workflow and atomic edits.

Deliverables:
- Approval workflow: `RequireApproval` pauses agent loop, emits `ApprovalRequired` event, waits for confirmation via `/approve` endpoint
- Transactional edit batches (apply to copy, commit only if all succeed)
- `ReplaceRange { start_tag, end_tag, new_text }` multiline edit operation
- Integration test suite for `arcan` binary (end-to-end SSE)
- Property tests for hashline edit correctness

Exit criteria:
- Destructive tools pause for user approval when policy requires it
- Edit batches are atomic (all-or-nothing)
- 15+ integration tests total

Dependencies: Phase B (CancellationToken needed for approval timeout)

### Phase E: Provider and Sandbox Expansion

**Goal**: Broader model support and OS-level isolation.

Deliverables:
- `OpenAIProvider` implementing `Provider` trait
- Retry/backoff logic for all providers
- `BubblewrapRunner` for Linux namespace isolation
- `DockerRunner` for container-based isolation
- `patch_file` tool for diff-based editing of large files

Exit criteria:
- 3+ LLM providers available
- OS-level isolation enforced on Linux
- Diff editing works for files >400 lines

Dependencies: Phase D (test infrastructure)

### Phase F: Session Management and Clients

**Goal**: Advanced session features and user interfaces.

Deliverables:
- Session fork API (branch from any event, child sessions)
- Session compaction (summarize old messages, emit `SessionCompacted` event)
- CLI client binary consuming SSE stream
- Parallel tool execution with dependency analysis

Exit criteria:
- Sessions can be branched and resumed
- Long sessions are compacted automatically
- CLI client provides interactive terminal chat

Dependencies: Phase B (context management), Phase C (streaming)

### Phase G: Advanced Runtime (Future)

**Goal**: Extensible agent ecosystem.

Deliverables:
- Subagent execution (nested agent loops with restricted toolsets)
- Extension SDK (WASM or native middleware)
- Web client
- Multi-frontend adapters (Telegram, Discord, Slack)

Exit criteria:
- Sub-agent runs are replayable in session tree
- Extensions can add tools/middleware without core changes

Dependencies: Phases D, E, F

---

## 6. Known Issues

1. **Dockerfile targets wrong binary**: Line 37 builds `arcan-daemon` (old name for `arcand`), but the production binary is `arcan`. The `ENTRYPOINT` and `COPY` lines need updating.

2. **No `/health` endpoint**: The Dockerfile includes a `HEALTHCHECK` that curls `http://localhost:3000/health`, but no route handles this path. Container orchestrators (Docker Compose, k8s) will report the service as unhealthy.

3. **MockProvider silent fallback**: If `ANTHROPIC_API_KEY` is not set, the daemon silently falls back to `MockProvider` which returns canned responses. A warning is logged via tracing, but users without log access may be confused.

4. **Command parsing**: `BashTool` passes raw command strings to `/bin/bash -c`. The comment in sandbox.rs notes this should use shlex for proper parsing.

5. **No graceful shutdown**: Neither `arcand` nor `arcan` handles SIGTERM beyond tokio's default behavior. Long-running tool executions are not drained.

---

## 7. Documentation Index

| Document | Status | Description |
|---|---|---|
| **`docs/STATUS.md`** | Current | Single source of truth (this file) |
| **`docs/harness.md`** | Current | Deep dive on harness architecture, tools, sandbox, data layer |
| **`docs/harness-report.md`** | Current | Research synthesis from 4 industry sources, scorecard, action plan |
| **`docs/lago-integration.md`** | Current | Bridge crate documentation, 5 modules, 33 tests |
| **`docs/vision-and-status.md`** | Current | Vision, design philosophy, feature matrix, roadmap phases |
| `docs/architecture.md` | Superseded | Predates arcan-provider, arcan-lago. See STATUS.md |
| `docs/roadmap.md` | Superseded | Phase numbering conflicts. See STATUS.md |
| **`AGENTS.md`** | Current | Project quick reference (symlinked as CLAUDE.md) |
