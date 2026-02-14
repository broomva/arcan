# The Harness Problem

The quality of an agentic system is determined as much by its **tool implementations** as by the LLM driving it. A model that can reason perfectly but cannot reliably read, edit, search, and execute against a codebase is useless. This is the harness problem: the gap between what a model can express as intent and what the execution layer can faithfully carry out.

Arcan treats harness quality as a first-class concern. The `arcan-harness` crate implements a filesystem and execution layer designed for correctness, safety, and auditability -- not just convenience.

---

## 1. Why the Harness Matters

Consider a simple task: "fix the typo on line 42 of main.rs."

A naive agent would:
1. Read the file.
2. Generate a new version with the fix.
3. Write the entire file back.

This fails in practice because:
- The file may have changed between read and write (no concurrency control).
- Writing the full file is fragile -- any hallucinated content overwrites real code.
- There's no way to verify the agent edited what it *intended* to edit.
- The operation is invisible -- no audit trail of what changed or why.

Every major agent system (Claude Code, Cursor, Codex) has converged on a similar set of tools: read, write, edit, glob, grep, bash. The difference is in how reliably and safely these tools execute. That execution layer is the harness.

---

## 2. Filesystem Layer (`arcan-harness/src/fs.rs`)

### Workspace Sandboxing: FsPolicy

All filesystem tools share a single `FsPolicy` that enforces workspace boundaries:

```rust
pub struct FsPolicy {
    pub workspace_root: PathBuf,
}
```

Two resolution modes:

- **`resolve_existing(path)`**: For reads. Canonicalizes the path (resolves symlinks), then checks that the canonical path starts with `workspace_root`. Prevents symlink escape attacks.

- **`resolve_for_write(path)`**: For writes. Doesn't require the file to exist yet, but the parent directory must exist and be within workspace bounds.

Both reject paths outside the workspace with a structured error. This is defense-in-depth: even if the LLM hallucinates a path like `/etc/passwd`, the tool refuses.

### ReadFileTool

Returns file content annotated with line numbers and blake3 content hashes:

```
  1 a1b2c3d4 | fn main() {
  2 e5f6g7h8 |     println!("hello");
  3 f9a0b1c2 | }
```

This output format serves two purposes:
1. The model sees line numbers for precise references.
2. The hashes anchor the `EditFileTool` operations (see Hashline Editing below).

Annotations: `read_only: true`, `idempotent: true`.

### WriteFileTool

Atomic full-file overwrite. Used when the model needs to create a new file or replace content entirely. The tool writes to the resolved path within workspace bounds.

Annotations: `destructive: true`.

### ListDirTool

Returns directory entries as JSON: `[{name, kind}]` where kind is `"file"` or `"dir"`. Uses `FsPolicy` to ensure the target directory is within workspace.

Annotations: `read_only: true`, `idempotent: true`.

### GlobTool

Pattern-based file search using the `glob` crate. Accepts patterns like `**/*.rs` or `src/**/*.ts`. Returns matching paths relative to workspace root.

The tool resolves the search base against `FsPolicy` and only returns results within workspace bounds. Useful for discovery: "find all test files," "which configs exist."

Annotations: `read_only: true`, `idempotent: true`.

### GrepTool

Regex content search across the workspace. Accepts:
- `pattern`: Regex string
- `include` (optional): Glob filter for file types (e.g. `*.rs`)
- `max_matches` (optional, default 100): Result limit

Implementation:
- Walks the workspace tree via `walkdir`
- Skips binary files and files larger than 10MB
- Applies glob filter if provided
- Matches regex per line
- Returns `[{file, line, text}]`

Annotations: `read_only: true`, `idempotent: true`. Timeout: 60 seconds (search can be expensive).

---

## 3. Hashline Editing (`arcan-harness/src/edit.rs`)

### The Problem with Blind Edits

Most agent edit tools use "find and replace" or line-number based targeting. Both are fragile:

- **Find/replace** fails when the target string appears multiple times or has been slightly modified.
- **Line numbers** drift as soon as any earlier line is added or removed.
- **Full file rewrites** risk hallucinating content in untouched sections.

### The Hashline Solution

Every line in a file is tagged with an 8-character blake3 hash:

```rust
pub struct HashedLine {
    pub line_no: usize,   // 1-indexed
    pub tag: String,       // 8-char blake3 hash
    pub text: String,      // Line content
}
```

The tag is computed as `blake3(line_no + ":" + content)[..8]`. This means:
- The same text on different lines produces different tags (line number is part of the hash).
- Tags are stable as long as the line content hasn't changed.
- Tags are short enough (8 chars) for the model to include in edit operations.

### Edit Operations

Three operations reference tags, never raw content:

```rust
pub enum TaggedEditOp {
    ReplaceLine { tag: String, new_text: String },
    InsertAfterTag { tag: String, new_text: String },
    DeleteLine { tag: String },
}
```

### Stale Tag Rejection

When the model submits an edit referencing tag `a1b2c3d4`, the system:
1. Hashes the current file content.
2. Looks up the tag.
3. If the tag doesn't match any current line -> **reject with error**.

This is optimistic concurrency control. If the file was modified since the model read it (by another tool, a user, or a previous edit in the same turn), the stale tag catches the conflict. The model must re-read the file and try again with fresh tags.

### Why This Works

The hashline technique dramatically improves edit success rates across all models because:

1. **No ambiguity**: Tags are unique identifiers, not pattern matches.
2. **No drift**: Tags survive insertions and deletions on other lines.
3. **Fail-safe**: Stale tags are caught before any modification occurs.
4. **Composable**: Multiple edits in a single request are applied sequentially; each operation re-hashes, so later operations see the effect of earlier ones.

The `EditFileTool` wraps this into a standard arcan `Tool`:
- Input: file path + list of `TaggedEditOp`
- Reads file, computes hashes, applies operations, writes atomically
- Returns the new file content with fresh hashes
- Annotations: `destructive: true`

---

## 4. Sandboxed Execution (`arcan-harness/src/sandbox.rs`)

### Policy Model

```rust
pub struct SandboxPolicy {
    pub workspace_root: PathBuf,
    pub shell_enabled: bool,
    pub network: NetworkPolicy,      // Disabled | AllowAll | AllowList(hosts)
    pub allowed_env: BTreeSet<String>,
    pub max_execution_ms: u64,
    pub max_stdout_bytes: usize,
    pub max_stderr_bytes: usize,
    pub max_processes: u16,
    pub max_memory_mb: u32,
}
```

The policy captures every dimension of execution constraint. Even if the current executor doesn't enforce all limits, the policy contract is explicit and forward-compatible.

### CommandRunner Trait

```rust
pub trait CommandRunner: Send + Sync {
    fn run(
        &self,
        policy: &SandboxPolicy,
        request: &CommandRequest,
    ) -> Result<CommandResult, SandboxError>;
}
```

This trait decouples policy from execution mechanism. Implementations:

- **`LocalCommandRunner`** (current): Wraps `std::process::Command`. Clears environment except allowed vars. Sets working directory. No true resource isolation yet.
- **`BubblewrapRunner`** (future): Linux user namespaces via `bwrap` for process/network/filesystem isolation.
- **`DockerRunner`** (future): Container-based isolation for maximum security.

### BashTool

Wraps `CommandRunner` into an arcan `Tool`:

```rust
pub struct BashTool {
    policy: SandboxPolicy,
    runner: Box<dyn CommandRunner>,
}
```

- Executes `/bin/bash -c "command"` with the configured policy.
- Captures stdout/stderr (truncated per policy limits).
- Returns exit code + output.
- Annotations: `destructive: true`, `open_world: true`, `requires_confirmation: true`.
- Timeout: 60 seconds.

### Layered Security

Tool execution passes through multiple security layers:

```
1. Middleware: LagoPolicyMiddleware evaluates rules (risk level, tool name)
2. Tool Annotations: Model-visible hints (destructive, requires_confirmation)
3. FsPolicy: Workspace boundary enforcement for filesystem tools
4. SandboxPolicy: Process-level constraints for shell execution
5. CommandRunner: OS-level isolation (current: local, future: namespaces/containers)
6. Persistence: Every action recorded in append-only event log
```

Each layer operates independently. A failure at any layer blocks the operation.

---

## 5. Agent Memory (`arcan-harness/src/memory.rs`)

### Persistent Cross-Session Knowledge

Two tools provide key-value markdown storage:

**`ReadMemoryTool`**: Reads a markdown file from the memory directory.
- Input: `key` (alphanumeric, hyphens, underscores, max 64 chars)
- Output: `{content, exists, path}`
- Returns empty content + `exists: false` for missing keys

**`WriteMemoryTool`**: Writes markdown to the memory directory.
- Input: `key` + `content`
- Creates the memory directory if needed
- Overwrites existing keys

### Use Cases

- Long-term facts about a project (architecture decisions, conventions)
- Solutions to recurring problems
- Session summaries and learnings
- User preferences discovered during interaction

Memory persists across sessions. The agent can read previous notes and update them. Combined with lago's journal persistence, this gives agents durable context without requiring the full conversation history in every prompt.

---

## 6. MCP Integration (`arcan-harness/src/mcp.rs`)

### Model Context Protocol Bridge

Arcan connects to external MCP tool servers and presents their tools through the same `Tool` trait as built-in tools. The agent sees no difference between internal and external tools.

```rust
pub struct McpTool {
    peer: Arc<RunningService<...>>,
    definition: ToolDefinition,
    runtime: Handle,
}
```

#### Connection Flow

1. Spawn MCP server process via stdio transport
2. Initialize protocol handshake
3. Call `tools/list` to discover available tools
4. Wrap each tool as `McpTool` with arcan-compatible `ToolDefinition`

#### Naming Convention

MCP tools are namespaced: `mcp_{server_name}_{tool_name}`. This avoids collisions with built-in tools and makes the tool's origin visible in event logs.

#### Annotation Mapping

MCP tool annotations are mapped to arcan's `ToolAnnotations`:
- `readOnlyHint` -> `read_only`
- `destructiveHint` -> `destructive`
- `idempotentHint` -> `idempotent`
- `openWorldHint` -> `open_world`

Missing annotations default to `false`.

#### Async Boundary

MCP communication is async (tokio). The `McpTool::execute()` method uses `Handle::current().block_on()` to cross the sync/async boundary, same pattern as `LagoSessionRepository`.

---

## 7. Skills System (`arcan-harness/src/skills.rs`)

### SKILL.md Discovery

Skills are domain-specific knowledge and workflow definitions loaded from markdown files:

```yaml
---
name: commit-helper
description: Helps create git commits
tags: [git, workflow]
allowed_tools: [bash, read_file]
user_invocable: true
---
## Instructions
When the user asks to commit, always run tests first...
```

#### Discovery

`SkillRegistry` scans directories for `SKILL.md` files using `walkdir`. Each file is parsed:
1. YAML frontmatter between `---` delimiters
2. Markdown body as instructions
3. Malformed files are logged and skipped (no crash)

#### Integration

- `system_prompt_catalog()`: Returns a compact listing of available skills for the system prompt (~100 tokens per skill)
- `allowed_tools(name)`: Returns tool restrictions for a skill, enabling sandboxed skill execution
- Skills inject context into the agent's prompt, guiding behavior without modifying the tool set

---

## 8. The Three-Layer Tool System

Arcan's tool registry unifies three distinct tool sources behind one trait:

```
+-----------------------------------------------------------+
|                     ToolRegistry                           |
|          (all tools present the same interface)            |
+----------------+------------------+-----------------------+
| Internal Tools | MCP Bridge       | Skill Loader          |
|                | (rmcp)           | (SKILL.md)            |
| read_file      | stdio/HTTP       | Discover, parse YAML, |
| write_file     | tools/list ->    | inject into system    |
| edit_file      | tools/call ->    | prompt context        |
| list_dir       | McpTool wrapper  |                       |
| glob           |                  |                       |
| grep           |                  |                       |
| bash           |                  |                       |
| read_memory    |                  |                       |
| write_memory   |                  |                       |
+----------------+------------------+-----------------------+
```

**Layer 1: Internal tools** -- Native Rust, lowest latency, tightest sandbox integration. These are the foundation.

**Layer 2: MCP bridge** -- Connects to external tool servers. Discovers tools at runtime. Same `Tool` trait, same middleware pipeline, same event logging.

**Layer 3: Skills** -- Markdown-defined knowledge injection. Not tools themselves, but context that guides tool usage. A skill can restrict which tools are available, creating sandboxed workflows.

The orchestrator doesn't know or care which layer a tool comes from. Every tool call goes through the same middleware pipeline, the same event logging, the same policy checks.

---

## 9. Data Layer Architecture

### Event Sourcing

Every tool execution, model output, state patch, and lifecycle event is recorded as an immutable `AgentEvent`. The system's state is always a projection of its event log.

```
AgentEvent stream:
  RunStarted -> IterationStarted -> ModelOutput ->
  ToolCallRequested -> ToolCallCompleted ->
  StatePatched -> TextDelta -> ... -> RunFinished
```

### Persistence Backends

Three `SessionRepository` implementations, same trait:

| Backend | Storage | Durability | Query | Use Case |
|---|---|---|---|---|
| `InMemorySessionRepository` | `HashMap` | None | O(n) scan | Testing |
| `JsonlSessionRepository` | JSONL files | Filesystem | O(n) scan | Development |
| `LagoSessionRepository` | redb (ACID) | ACID transactions | Indexed | Production |

The lago backend adds:
- **ACID guarantees**: Crash recovery without data loss
- **Sequence numbers**: Ordered, gapless event streams per session/branch
- **Branch support**: `parent_id` enables forking sessions
- **Content-addressed blobs**: Large outputs stored once, referenced by hash
- **Policy evaluation logging**: Every decision recorded as a `PolicyEvaluated` event

### State Reconstruction

On each session load, the event log is replayed to rebuild `AppState` + conversation history:

```
for event in journal.read(session, branch):
    match event:
        StatePatched  -> apply_patch(state)
        TextDelta     -> aggregate into assistant message
        ToolCompleted -> add tool result message
        _             -> skip
```

This is simple and correct. No stale cache, no sync bugs. For long sessions, lago's `Projection` trait enables incremental computation and snapshots.

---

## 10. Design Trade-offs

### Correctness over Performance

- Full state reconstruction from event log on every load (no caching)
- Hashline tags force re-read before edit (no blind writes)
- FsPolicy canonicalizes paths on every access (no cached path trust)

### Safety over Convenience

- All filesystem tools enforce workspace boundaries
- Sandbox policy is explicit even when not fully enforced
- Stale edit tags fail loudly rather than applying a best-guess fix
- Policy middleware blocks before execution, not after

### Simplicity over Generality

- Synchronous `Tool::execute()` -- no streaming partial results (yet)
- Synchronous `Provider::complete()` -- run in `spawn_blocking`
- Default branch "main" for all sessions -- no multi-branch complexity (yet)
- `LocalCommandRunner` -- works everywhere, swap for container isolation later

### Auditability as Default

- Every event persisted to append-only log
- Tool annotations visible in event metadata
- Policy decisions logged
- No silent failures -- all errors produce structured events
