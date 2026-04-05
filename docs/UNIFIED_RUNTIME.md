# Arcan Unified Runtime — Architecture Proposal

> **Date**: 2026-04-01
> **Status**: Proposed
> **Linear Project**: Arcan Unified Runtime

## Problem

Arcan has two execution modes — **daemon** (`arcan serve`) and **shell** (`arcan shell`) — that share only ~30% of their infrastructure. The daemon has the full Life framework stack (Lago persistence, Nous evaluation, Autonomic gating, Anima identity, Spaces networking, Vigil observability), while the shell is a lightweight REPL with tools and a liquid prompt but no intelligence, persistence, or networking.

### Feature Parity Scorecard

| Category | Daemon | Shell |
|----------|--------|-------|
| Tool execution | 10/10 | 8/10 |
| Persistence (Lago) | 10/10 | 1/10 |
| Intelligence (Nous, Autonomic) | 9/10 | 0/10 |
| Identity & Auth (Anima) | 9/10 | 0/10 |
| Networking (Spaces) | 8/10 | 0/10 |
| Observability (Vigil) | 9/10 | 2/10 |
| Context & Prompt | 8/10 | 7/10 |
| Agent Lifecycle | 10/10 | 4/10 |

The shell also has innovations the daemon lacks: liquid prompt (CLAUDE.md + AGENTS.md + git context + docs/), auto-compact, slash commands, interactive permissions, and skill discovery.

## Solution: Shared Runtime Core

Extract the common infrastructure into `arcan-runtime` (or enhance the existing binary's module structure) so both modes compose from the same building blocks.

```
                    ┌─────────────────────────────┐
                    │    Shared Runtime Core       │
                    │                              │
                    │  SessionManager              │  ← Lago journal + session lifecycle
                    │  ToolRegistryBuilder         │  ← Praxis tools + governed memory + Spaces
                    │  IntelligenceLayer           │  ← Nous evaluators + Autonomic gating
                    │  LiquidPromptBuilder         │  ← prompt.rs (all 9 sources)
                    │  HookRegistry                │  ← 11 lifecycle events
                    │  SkillDiscovery              │  ← praxis-skills registry
                    │  IdentityResolver            │  ← Anima identity or anonymous
                    │  ObservabilityBridge         │  ← Vigil spans + metrics
                    │  AgentLoop                   │  ← Provider → tools → loop (multi-turn)
                    └──────────┬───────────────────┘
                               │
                    ┌──────────┴───────────────────┐
                    │                              │
              ┌─────┴─────┐              ┌────────┴────────┐
              │  Daemon   │              │     Shell       │
              │           │              │                 │
              │ Transport:│              │ Transport:      │
              │  HTTP/SSE │              │  stdin/stdout   │
              │           │              │                 │
              │ Concurrency:            │ Concurrency:    │
              │  Multi-session          │  Single-session │
              │  Multi-user             │  Single-user    │
              │           │              │                 │
              │ Extra:    │              │ Extra:          │
              │  REST API │              │  Slash commands │
              │  TUI client             │  Interactive    │
              │  Rate limiting          │   permissions   │
              │  Tier gating            │  Auto-compact   │
              └───────────┘              └─────────────────┘
```

## Implementation Phases

### Phase 1: Lago Persistence in Shell (P0)

Wire Lago journal into shell mode so sessions are durable and replayable.

**What changes:**
- Shell opens a `RedbJournal` at `.arcan/journal.redb` on startup
- Each user message → `EventKind::Message` appended to journal
- Each tool result → `EventKind::ToolCallCompleted` appended
- Each assistant response → `EventKind::Message` appended
- Session can be resumed with `arcan shell --session <id>`
- `--resume` flag loads last session from journal

**Dependencies:** lago-core, lago-journal (already workspace deps)

**Tickets:** BRO-350 through BRO-353

### Phase 2: Governed Memory Tools (P0)

Replace filesystem-only memory with Lago-backed governed memory.

**What changes:**
- Replace `ReadMemoryTool` / `WriteMemoryTool` with `MemoryQueryTool` / `MemoryProposeTool` / `MemoryCommitTool`
- Memory events flow through Lago journal (immutable, replayable)
- `MemoryProjection` provides cached in-memory view
- Shell's heuristic extraction becomes a fallback/export mechanism

**Dependencies:** Phase 1 (Lago journal must be wired first)

**Tickets:** BRO-354, BRO-355

### Phase 3: Nous Evaluators (P1)

Add post-tool safety evaluation to shell mode.

**What changes:**
- Wire `nous_heuristics::default_registry()` into shell startup
- After each tool execution, run evaluators (same as `NousToolObserver`)
- Log scores to stderr and optionally to Lago journal
- Display safety warnings inline

**Dependencies:** nous-core, nous-heuristics (already workspace deps)

**Tickets:** BRO-356, BRO-357

### Phase 4: Autonomic Budget Awareness (P1)

Add cost tracking and economic mode awareness to shell.

**What changes:**
- Track cumulative cost per session (already have token counts)
- Integrate `autonomic-core` for budget state projection
- Warn when approaching budget limits
- Support `--budget` flag to set session cost cap
- Economic mode display in `/status`

**Dependencies:** autonomic-core (already workspace dep)

**Tickets:** BRO-358, BRO-359

### Phase 5: Liquid Prompt Unification (P1)

Make daemon use shell's liquid prompt builder, and shell use daemon's identity injection.

**What changes:**
- Daemon's `canonical.rs` calls `prompt::build_system_prompt()` for git context, CLAUDE.md, AGENTS.md
- Shell's `prompt.rs` adds identity/persona block (from Anima or anonymous)
- Both modes produce identical system prompts given same inputs
- Prompt sections become a shared `PromptConfig` struct

**Dependencies:** None (refactor only)

**Tickets:** BRO-360, BRO-361

### Phase 6: Spaces Networking (P1)

Add agent-to-agent communication to shell mode.

**What changes:**
- Register Spaces tools (send, list, read, DM) in shell if configured
- `--spaces-backend` flag (mock or spacetimedb)
- Activity logging to `#agent-logs` channel
- Peer context injection into liquid prompt (recent agent activity)

**Dependencies:** arcan-spaces (already workspace dep, feature-gated)

**Tickets:** BRO-362, BRO-363

### Phase 7: Identity & Auth (P2)

Add Anima identity resolution to shell mode.

**What changes:**
- Read identity from `~/.arcan/identity.json` or `ARCAN_IDENTITY_TOKEN` env
- Resolve tier (anonymous/free/pro/enterprise)
- Inject persona block into liquid prompt
- Tier-aware skill filtering (optional)

**Dependencies:** anima-core

**Tickets:** BRO-364, BRO-365

### Phase 8: Vigil Observability (P2)

Add OpenTelemetry tracing to shell mode.

**What changes:**
- Wire `life_vigil` spans around agent loop, tool execution, provider calls
- Export to OTLP endpoint if configured (`OTEL_EXPORTER_OTLP_ENDPOINT`)
- Graceful degradation: structured logging when no endpoint
- GenAI semantic conventions for model calls

**Dependencies:** life-vigil (already workspace dep)

**Tickets:** BRO-366, BRO-367

### Phase 9: Daemon Prompt Improvements (P2)

Backport shell innovations to daemon mode.

**What changes:**
- Daemon uses `prompt::load_project_instructions()` for CLAUDE.md + AGENTS.md + docs/
- Daemon uses `prompt::build_git_section()` for git context
- Daemon uses `prompt::build_environment_section()` for platform info
- Unified prompt builder shared between both modes

**Tickets:** BRO-368, BRO-369

### Phase 10: Shared AgentLoop Trait (P3)

Extract the agentic loop into a trait that both modes implement.

**What changes:**
- Define `AgentLoop` trait with `run()`, `on_tool_call()`, `on_response()` hooks
- Daemon implements via `KernelRuntime::tick_on_branch()`
- Shell implements via `run_agent_loop()`
- Both get identical lifecycle guarantees (max iterations, budget, hooks)

**Dependencies:** All previous phases

**Tickets:** BRO-370, BRO-371

## Ticket Summary

| Phase | Priority | Tickets | Description |
|-------|----------|---------|-------------|
| 1 | P0 | BRO-350 to BRO-353 | Lago persistence in shell |
| 2 | P0 | BRO-354, BRO-355 | Governed memory tools |
| 3 | P1 | BRO-356, BRO-357 | Nous evaluators |
| 4 | P1 | BRO-358, BRO-359 | Autonomic budget |
| 5 | P1 | BRO-360, BRO-361 | Prompt unification |
| 6 | P1 | BRO-362, BRO-363 | Spaces networking |
| 7 | P2 | BRO-364, BRO-365 | Identity & auth |
| 8 | P2 | BRO-366, BRO-367 | Vigil observability |
| 9 | P2 | BRO-368, BRO-369 | Daemon prompt improvements |
| 10 | P3 | BRO-370, BRO-371 | Shared AgentLoop trait |

**Total: 22 tickets across 10 phases**
