# Arcan Project Documentation

Arcan is a Rust-based agent daemon designed for reliability, streaming, and secure tool execution. It is inspired by Vercel AI SDK, Claude Code, and other modern agentic systems.

## Architecture

The project is structured as a Rust workspace:

- **`crates/arcan-core`**: Defines the core traits (`Provider`, `Tool`, `Middleware`), protocol types (`AgentEvent`, `ChatMessage`), and state management (`AppState` with JSON Patch).
- **`crates/arcan-harness`**: Contains the tool harness, including filesystem operations (`fs`), safe editing logic (`edit` / "Hashline"), and sandboxing (`sandbox`).
- **`crates/arcan-store`**: Handles persistence using an append-only log format (JSONL) and manages session history.
- **`crates/arcan-provider`**: LLM provider implementations. Currently includes `AnthropicProvider` for the Claude Messages API.
- **`crates/arcand`**: The agent loop, SSE server, and HTTP routing library for the daemon.
- **`crates/arcan-lago`**: Bridge between Arcan and Lago event-sourced persistence.
- **`crates/arcan`**: The installable binary (`cargo install arcan`) — production entry point with Clap CLI, structured logging, and policy middleware.

## Key Concepts

### Agent Loop
The agent loop (`arcand/src/loop.rs`) follows a strict cycle:
1. **Reconstruct State**: Loads session history to build the current state.
2. **Provider Call**: Sends context to the LLM (Provider).
3. **Execution**: Processes LLM directives (Text, Tool Calls, State Patches).
4. **Consistency**: Applies state patches and executes tools.
5. **Streaming**: Emits typed `AgentEvent`s via SSE to the client.

### Hashline Editing
To avoid "blind" edits, Arcan uses a "Hashline" technique (`arcan-harness/src/edit.rs`). Files are read with line numbers and content hashes. Edits (replace, delete, insert) must reference these tags, ensuring that the agent is editing the version of the file it "sees".

### Sandboxing
Tools are executed within a policy-driven sandbox (`arcan-harness/src/sandbox.rs`). While currently a local wrapper, the design allows for enforcing network policies, resource limits, and filesystem isolation.

## Running the Project

Build the daemon:
```bash
cargo build -p arcan
```

Run the daemon (mock provider):
```bash
cargo run -p arcan
```

Run with real LLM (Anthropic Claude):
```bash
ANTHROPIC_API_KEY=sk-ant-... cargo run -p arcan
```

Install from crates.io:
```bash
cargo install arcan
```

The server listens on `http://localhost:3000`.

Test with curl:
```bash
curl -N -X POST http://localhost:3000/chat \
  -H "Content-Type: application/json" \
  -d '{"session_id": "test-1", "message": "Hello, what can you do?"}'
```

## Project Status

For the comprehensive implementation status, scorecard, gap analysis, and roadmap, see **[`docs/STATUS.md`](docs/STATUS.md)** -- the single source of truth for project status.

## Useful Commands

- **Test**: `cargo test --workspace`
- **Lint**: `cargo clippy`
- **Format**: `cargo fmt`



**Design philosophy:** The agent's message history IS the application state. Every action produces immutable events; the system's state is a projection of its event log.


### AI Assistant Guidelines
### AI Assistant Guidelines
- **Cursor**: Follow `.cursorrules` for coding standards.
- **Claude Code**: Refer to `CLAUDE.md` for project commands.
- **Linter**: Run `cargo clippy` to verify code quality.
- **Fixes**: Run `cargo fmt` to auto-fix formatting.
- **Rules**:
  - All new code must have valid tests.
  - All code must pass `cargo clippy` (Linting).
  - All code must pass `cargo check` (Type checking / Compilation).
  - The full project must build successfully via `cargo build --workspace`.
  - When finishing and validating a feature, agents must perform a brief self-learning loop (observe outcomes, reflect on gaps, and capture durable guidance) and update `AGENTS.md` plus the relevant `docs/` structure when new workflow or architecture knowledge is discovered.
  - Commits that fail these checks will be rejected by pre-commit hooks.

### Claude Code Configuration

This project uses comprehensive Claude Code settings for automation and security:

**Configuration Files:**
- `.claude/settings.local.json` — Permissions, hooks, and local settings (gitignored)
- `.claude/rules/` — Topic-specific guidelines (code-style, testing, workspace)
- `CLAUDE.md` — Quick reference and commands

**Automated Hooks:**
- **SessionStart (compact)**: Re-injects project conventions after context compaction
- **PostToolUse (Write/Edit)**: Auto-formats code with `cargo fmt` after file modifications
- **Stop**: Reminds to verify formatting, types, and tests after task completion

**Security & Permissions:**
- **Deny rules**: Blocks access to `.env` files, secrets directories, `.git/config`, and lockfiles
- **Allow rules**: Pre-approved commands for Cargo, Git, testing, and documentation fetching
- **Defense-in-depth**: Multiple layers of protection for sensitive files

**Topic-Specific Rules:**
See `.claude/rules/` for detailed guidelines on:
- Code style and naming conventions
- Testing structure and coverage requirements
- Rust Workspace dependencies

### Pre-Commit Workflow for AI Agents

**IMPORTANT**: Before committing any code changes, AI agents MUST follow this workflow:

#### For All Changes
1. **Auto-fix formatting:**
   ```bash
   cargo fmt
   ```
   This fixes formatting issues automatically.

2. **Verify correctness:**
   ```bash
   cargo check
   ```
   All compilation errors must be resolved before committing.

#### For Larger Implementations (New Features, Refactors)
Additionally run:

3. **Verify tests pass:**
   ```bash
   cargo test
   ```
   All existing tests must pass. Add new tests for new functionality.

4. **Verify linting:**
   ```bash
   cargo clippy
   ```
   Ensure code follows best practices.

5. **Verify build succeeds:**
   ```bash
   cargo build --workspace
   ```
   The entire workspace must build without errors.

#### Commit Pattern
```bash
# 1. Fix formatting
cargo fmt

# 2. Stage changes
git add <files>

# 3. Commit (pre-commit hooks will run checks automatically)
git commit -m "feat: description"
```

**Note**: The pre-commit hook will automatically run `cargo fmt --check` and `cargo check`. If you've already run these manually and fixed all issues, the commit will succeed immediately.
