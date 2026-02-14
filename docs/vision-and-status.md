> For the latest implementation status, scorecard, and roadmap, see [STATUS.md](./STATUS.md).

# Arcan Vision and Implementation Status

## 1. Arcan Vision

Arcan is a **Rust-Based Agent Harness and Runtime** designed for reliability, streaming, and secure tool execution. It draws inspiration from Vercel AI SDK, Claude Code, and other modern agentic systems. The following sections outline the core concepts that define Arcan's design philosophy and architecture.

### 1.1 Core Agent Loop

The fundamental pattern is: **store -> load state -> call LLM with tools -> execute tool calls -> feed results back -> loop until completion**. The loop ends when the model signals `end_turn`.

> "An autonomous agent is just an LLM + tools + a loop."

This tight cycle ensures deterministic, auditable behavior. Every iteration produces events that are persisted and streamed to clients.

### 1.2 Tool Calling Mechanism

Tools use a **structured JSON `tool_use` format**. Each tool has explicit:

- **Name**: A unique identifier for dispatch.
- **Description**: Human-readable explanation for the LLM.
- **Input Schema**: JSON Schema defining expected parameters.

The `Tool` trait captures this contract: `name()`, `description()`, `execute()`. The interface is **model-agnostic** -- Anthropic `tool_use` or OpenAI function calling are normalized to the same internal representation.

### 1.3 Filesystem Tools and Sandboxed Execution

Built-in tools provide core filesystem and execution capabilities:

- `read_file` (with line numbers)
- `write_file`
- `edit_file` (find-replace with hashline tags)
- `glob` (filename pattern search)
- `grep` (regex content search)
- `bash` (shell command execution)

All tools run in a **sandboxed environment** (Docker/namespaces/chroot). Multiple safety layers provide defense-in-depth: OS-level isolation combined with runtime policy checks.

### 1.4 Session Persistence and Tree-Structured Logs

Sessions use an **append-only event log with branching**. Each `EventRecord` carries a `parent_id`, forming a tree structure. This enables:

- **Forking sessions**: Branch from any intermediate point to explore alternatives.
- **Recovery**: Replay the log to reconstruct state after crashes.
- **Auditability**: Trace every agent action with full provenance.
- **Exploration**: What-if branching for experimentation.

### 1.5 Agent State as Application State

The agent's message history **IS** the application state. This principle has several implications:

- State patches are streamed to the frontend as typed SSE events.
- Tools can read and write persistent memory (markdown/JSON files).
- `StatePatch` events carry diffs for real-time UI synchronization.
- The system's state is always a projection of its event log.

### 1.6 Hashline Tags for Reliable Edits

Each line of a file is tagged with a **short Blake3 hash** of its content. This mechanism ensures safe editing:

- Edits reference **line tags**, not raw content snippets.
- Stale tags are **auto-rejected** (the file changed since the agent last read it).
- This dramatically improves edit success rates across all models.

### 1.7 SSE Streaming Protocol

The streaming protocol emits **typed events** including:

- Message tokens (incremental text)
- Tool invocation updates (call start, progress, result)
- State patches (diffs for UI sync)
- System and status events (errors, completion signals)

The protocol is compatible with **Vercel AI SDK data parts** and distinguishes between transient events (tokens) and persistent events (tool results). The event model is transport-agnostic.

### 1.8 Project Structure

The workspace is organized into focused crates:

- **`arcan-core`**: Traits (`Provider`, `Tool`, `Middleware`), protocol types, state management, AI SDK mapping.
- **`arcan-harness`**: Tool implementations (9 tools), sandboxing, MCP bridge, skills system.
- **`arcan-store`**: Persistence layer for append-only event logs (InMemory, JSONL).
- **`arcan-provider`**: LLM provider adapters (Anthropic, Rig bridge).
- **`arcand`**: Agent loop library, Axum HTTP/SSE server, mock provider.
- **`arcan-lago`**: Bridge to Lago ACID persistence, policy middleware, state projection, multi-format SSE.
- **`arcan`**: Production binary with Lago journal, structured logging, and CLI.

### 1.9 Extensibility

Arcan is designed for extensibility at every layer:

- **Plugin hooks** via the `Tool` and `Middleware` traits.
- **Subagents**: Nested agent loops with restricted toolsets for delegation.
- **Multi-frontend support**: CLI, web, Telegram, Discord, and other interfaces.
- **Persistent cross-session memory**: Agent knowledge that survives restarts.
- **Extension SDK**: WASM or native middleware for third-party integrations.

---

## 2. Current Implementation Status

The following matrix tracks the implementation status of each feature against the vision.

| Component | Vision Feature | Status | Implementation |
|---|---|---|---|
| Core Agent Loop | store -> load -> LLM -> tools -> loop | Done | Orchestrator in `runtime.rs`, AgentLoop in `loop.rs` |
| Tool Calling | Structured JSON `tool_use` | Done | `ToolCall`/`ToolResult`/`ToolDefinition` in `protocol.rs` |
| Provider Abstraction | Model-agnostic interface | Done | `Provider` trait + `AnthropicProvider` |
| Filesystem Tools | `read_file`, `write_file`, `list_dir`, `edit_file`, `bash` | Done | `arcan-harness/src/fs.rs`, `edit.rs`, `sandbox.rs` |
| Hashline Editing | Blake3 line tags for safe edits | Done | `arcan-harness/src/edit.rs` |
| Agent State = App State | `AppState` + `StatePatch` | Done | `state.rs` with JSON Patch (RFC 6902) + Merge Patch (RFC 7386) |
| Session Persistence | Append-only event log | Done | `arcan-store/src/session.rs` (JSONL) |
| Tree-Structured Sessions | `parent_id` in `EventRecord` | Done | `EventRecord.parent_id` field |
| SSE Streaming | 10 typed `AgentEvent` variants | Done | Axum SSE in `server.rs` |
| Middleware Hooks | 5 lifecycle hooks | Done | `before`/`after` model, `pre`/`post` tool, `on_run_finished` |
| Budget Controls | `max_iterations` | Done | `OrchestratorConfig` |
| Workspace Sandboxing | `FsPolicy` boundary checks | Done | `FsPolicy` with canonicalization |
| `glob` tool | Filename pattern search | Done | `GlobTool` in `arcan-harness/src/fs.rs` |
| `grep` tool | Regex content search | Done | `GrepTool` in `arcan-harness/src/fs.rs` with regex + glob filter |
| Memory tools | `read_memory`/`write_memory` | Done | `arcan-harness/src/memory.rs` |
| MCP Integration | External tool server bridge | Done | `arcan-harness/src/mcp.rs` via `rmcp` crate |
| Skills.sh Support | `SKILL.md` discovery and loading | Done | `arcan-harness/src/skills.rs` |
| AI SDK v5 Wire Format | Vercel data parts mapping | Done | `arcan-core/src/aisdk.rs` + lago multi-format SSE |
| Typed State Schema | Well-known state keys | Done | `AppState::well_known()` with cwd, open_files, budget, etc. |
| Tool Annotations | `read_only`, `destructive`, etc. | Done | `ToolAnnotations` in `protocol.rs` (5 fields, MCP-aligned) |
| Lago Persistence | ACID journal backend | Done | `arcan-lago` bridge crate + `RedbJournal` |
| Policy Middleware | Rule-based tool governance | Done | `LagoPolicyMiddleware` wrapping lago `PolicyEngine` |
| State Projection | Replay events to rebuild state | Done | `AppStateProjection` implementing lago `Projection` |
| Multi-Format SSE | OpenAI/Anthropic/Vercel/Lago | Done | `SseBridge` in `arcan-lago/src/sse_bridge.rs` |
| Sandbox Enforcement | Timeout/memory/network limits | Mostly Done | Env filtering, cwd validation, timeout, output truncation enforced. Network/process/memory limits need OS isolation (bwrap/Docker). |
| Subagent Execution | Nested agent loops | Not Done | Not implemented |
| CLI Client | Terminal chat interface | Not Done | HTTP/SSE only |
| Session Fork API | Explicit branch/fork semantics | Not Done | `parent_id` exists but no API |
| Approval Workflow | User confirmation for destructive ops | Not Done | Not implemented |
| Run Cancellation | Interrupt agent mid-stream | Not Done | Not implemented |

---

## 3. Architecture: Three-Layer Tool System

Arcan uses a unified tool registry that abstracts over three distinct tool sources. All tools present the same interface to the agent loop regardless of their origin.

```
┌──────────────────────────────────────────────────────┐
│                    ToolRegistry                       │
│         (unified: all tools look the same)            │
├────────────┬────────────────┬────────────────────────┤
│  Internal  │   MCP Bridge   │    Skill Loader        │
│  Tools     │   (rmcp)       │    (SKILL.md)          │
│            │                │                        │
│  read_file │  stdio/HTTP    │  Scan dirs, parse      │
│  edit_file │  tools/list →  │  YAML frontmatter,     │
│  bash      │  tools/call →  │  inject into system    │
│  write_file│  McpTool impl  │  prompt context        │
│  list_dir  │                │                        │
│  glob      │                │                        │
│  grep      │                │                        │
│  memory    │                │                        │
└────────────┴────────────────┴────────────────────────┘
```

### Layer 1: Internal Tools

These are native Rust implementations of core filesystem and execution tools. They live in `arcan-harness` and are compiled directly into the daemon. They have the lowest latency and tightest integration with the sandbox policy system.

### Layer 2: MCP Bridge

The Model Context Protocol bridge allows Arcan to connect to external tool servers over stdio or HTTP. Using the `rmcp` crate, the bridge discovers tools via `tools/list` and dispatches calls via `tools/call`. Each remote tool is wrapped in an `McpTool` implementation that conforms to the same `Tool` trait as internal tools.

### Layer 3: Skill Loader

Skills are discovered by scanning directories for `SKILL.md` files. Each skill file contains YAML frontmatter defining metadata and instructions. The skill loader parses these files and injects their content into the system prompt context, enabling the agent to leverage domain-specific knowledge and workflows.

---

## 4. Crate Structure

```
arcan-rs/
├── crates/
│   ├── arcan-core/       # Traits (Provider, Tool, Middleware), protocol types, state, AI SDK mapping
│   ├── arcan-harness/    # Tool implementations, sandboxing, MCP bridge, skill loader
│   ├── arcan-store/      # Append-only event persistence, session tree
│   ├── arcan-provider/   # LLM provider adapters (Anthropic raw, Rig bridge)
│   ├── arcand/           # Basic daemon with JSONL storage, Axum HTTP/SSE server
│   ├── arcan-lago/       # Bridge to Lago persistence and governance layer
│   └── arcan/            # Production daemon with Lago ACID journal + policy middleware
├── docs/                 # Architecture and vision documentation
├── Cargo.toml            # Workspace definition
├── AGENTS.md             # Project documentation for AI agents
└── CLAUDE.md             # Quick reference
```

### Crate Dependency Graph

```
arcan (production daemon)
├── arcan-core
├── arcan-harness  → arcan-core
├── arcan-store    → arcan-core
├── arcan-provider → arcan-core
├── arcand         → arcan-core, arcan-harness, arcan-store
└── arcan-lago     → arcan-core, arcan-store, lago-*

arcand (basic daemon)
├── arcan-core
├── arcan-harness  → arcan-core
├── arcan-store    → arcan-core
└── arcan-provider → arcan-core
```

- **`arcan-core`** is the foundation with minimal dependencies. It defines the shared vocabulary of traits and types.
- **`arcan-harness`** depends on `arcan-core` and provides all tool implementations plus sandboxing.
- **`arcan-store`** depends on `arcan-core` and handles persistence of the event log.
- **`arcan-provider`** depends on `arcan-core` and contains LLM provider adapters (Anthropic, Rig bridge, etc.).
- **`arcand`** depends on core, harness, store, and provider. Basic daemon with JSONL storage.
- **`arcan-lago`** bridges arcan's sync traits with lago's async persistence and policy engine. Depends on `lago-core`, `lago-journal`, `lago-store`, `lago-policy`, and `lago-api`.
- **`arcan`** is the production daemon with ACID persistence, policy middleware, and structured logging.

---

## 5. Implementation Roadmap

> **Note**: See [STATUS.md](./STATUS.md) Phases A-G for the current forward-looking roadmap.
> The phases below document completed work.

The following phases represent the approved plan for evolving Arcan from its current state to the full vision.

### Phase 1: Tool Expansion -- Done

All core tools implemented:

- `glob` tool for filename pattern search (using the `glob` crate).
- `grep` tool for regex content search (using `regex` + `walkdir`).
- `read_memory` / `write_memory` tools for persistent agent memory.
- Tool annotations (`read_only`, `destructive`, `idempotent`, `open_world`, `requires_confirmation`) on `ToolDefinition`.

### Phase 2: MCP Integration -- Done

External tool servers bridged into the unified `ToolRegistry`:

- MCP client implemented using the `rmcp` crate.
- Stdio transport for `tools/list` and `tools/call`.
- Remote tools wrapped in `McpTool` adapters implementing the `Tool` trait.
- MCP annotations mapped to arcan `ToolAnnotations`.

### Phase 3: Skill Loader -- Done

`SKILL.md` discovery and loading:

- Directory scanner for skill files via `walkdir`.
- YAML frontmatter parser for skill metadata.
- System prompt injection for skill context.
- `allowed_tools` filtering for sandboxed skill execution.

### Phase 4: AI SDK v5 Wire Format -- Done

Arcan events mapped to Vercel AI SDK v5 data parts:

- `AiSdkPart` enum with all standard part types in `arcan-core/src/aisdk.rs`.
- Multi-format SSE via lago bridge: OpenAI, Anthropic, Vercel, Lago native.
- Server supports `?format=aisdk_v5` query parameter.

### Phase 5: Lago Integration -- Done

Arcan bridged to Lago persistence and governance platform:

- `arcan-lago` bridge crate with event mapping, journal repository, policy middleware, state projection, SSE bridge.
- `arcan` production daemon with ACID persistence via `RedbJournal`.
- `LagoPolicyMiddleware` for rule-based tool governance with risk assessment.
- `AppStateProjection` implementing lago `Projection` trait.
- Multi-format SSE via `SseBridge` (OpenAI, Anthropic, Vercel, Lago).
- See `docs/lago-integration.md` for details.

### Phase 6: Advanced Session Management -- Planned

Build out the full session tree capabilities:

- Session fork API (branch from any event in the tree).
- Run cancellation (interrupt agent mid-stream with graceful cleanup).
- Approval workflow (pause before destructive operations, await user confirmation).
- Sliding window context management for long sessions.

### Phase 7: Sandbox Hardening -- Partial

Process-level enforcement done (P0 security fixes):

- Timeout enforcement on tool execution -- Done (wait-timeout crate).
- Environment variable filtering (empty = deny all) -- Done.
- CWD workspace validation -- Done.
- Output size truncation -- Done.
- Memory limits for spawned processes -- Not Done (needs OS isolation).
- Network policy (allow/deny lists for outbound connections) -- Not Done.
- Bubblewrap or Docker-based isolation backends -- Not Done.
- Subagent execution with restricted toolsets and budgets -- Not Done.

### Phase 8: Client Interfaces -- Planned

Build client interfaces beyond the HTTP/SSE API:

- CLI client (`arcan` binary) with terminal chat interface.
- Web client consuming the SSE stream.
- Multi-frontend adapters (Telegram, Discord, Slack).
