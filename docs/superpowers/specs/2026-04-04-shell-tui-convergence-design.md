# Shell / TUI Convergence — Design Spec

## Problem

The shell and TUI are divergent implementations of the same user interface:
- **Shell** has intelligence (14 commands, memory extraction, Autonomic regulation, context budgets, bare mode) but poor UX (blocking REPL, can't type while thinking)
- **TUI** has great UX (concurrent input/output, ratatui rendering, split panes, autocomplete) but lacks intelligence (9 commands, no memory, no Autonomic)

## Goal

Converge on a single interactive experience where:
1. The daemon (`arcand`) owns all intelligence (memory, Autonomic, context regulation)
2. The TUI is the primary interactive interface with Noesis-style input
3. The shell becomes a pipe-friendly non-interactive mode (`arcan run`)

## Architecture

```
┌──────────────────────────────────────────────────────┐
│ arcand (daemon)                                       │
│  ├─ Agent loop (Provider + ToolRegistry)              │
│  ├─ Memory extraction (post-turn, async)              │
│  ├─ Autonomic regulation (ContextPressureRule)        │
│  ├─ Context budgets (bare mode, compaction)            │
│  ├─ Lago persistence (journal + workspace)            │
│  ├─ Nous evaluation (quality signals)                 │
│  └─ HTTP/SSE API                                      │
│       ├─ POST /runs (send message)                    │
│       ├─ GET /events/stream (SSE)                     │
│       ├─ GET /context (token breakdown)               │
│       ├─ GET /memory (list/query)                     │
│       ├─ GET /autonomic (current ruling)              │
│       └─ POST /compact (force compaction)             │
├──────────────────────────────────────────────────────┤
│ arcan-tui (interactive TUI)                           │
│  ├─ Noesis-style input (always editable)              │
│  ├─ Message queuing (type while thinking)             │
│  ├─ Status bar (provider, context %, cost, ruling)    │
│  ├─ Streaming output with markdown                    │
│  ├─ Slash commands (all 14, proxied to daemon)        │
│  ├─ Escape to exit                                    │
│  └─ Session browser + state inspector                 │
├──────────────────────────────────────────────────────┤
│ arcan shell (non-interactive / pipe mode)             │
│  ├─ arcan run "message" (single shot)                 │
│  ├─ echo "msg" | arcan run (piped)                    │
│  └─ Thin client over daemon (no in-process loop)      │
└──────────────────────────────────────────────────────┘
```

## Migration Plan

### Phase 1: Move intelligence to daemon
- [ ] Memory extraction → arcand post-turn hook
- [ ] Autonomic context regulation → arcand middleware
- [ ] Context budget / bare mode → arcand config
- [ ] /context, /cost, /memory → daemon HTTP endpoints
- [ ] /compact → daemon POST endpoint

### Phase 2: TUI feature parity
- [ ] Add missing slash commands (proxy to daemon endpoints)
- [ ] Status bar: show Autonomic ruling, context %, cost
- [ ] Spinner with phases (initializing → reasoning → streaming)
- [ ] Memory file browser (/memory command)

### Phase 3: Noesis-style input
- [ ] Fixed input area at bottom (always editable)
- [ ] Message queuing with visual stack
- [ ] "Press up to edit queued messages"
- [ ] Escape to exit gracefully
- [ ] Status bar: provider | model | context % | tokens | cost | ruling

### Phase 4: Shell simplification
- [ ] Remove in-process agent loop from shell.rs
- [ ] Shell becomes `arcan run` (single-shot via daemon)
- [ ] Keep shell.rs for backward compat but delegate to daemon
- [ ] arcan-tui becomes `arcan` default command

## Feature Parity Checklist

| Feature | Daemon | TUI | Shell |
|---------|--------|-----|-------|
| Agent loop | ✅ has | via HTTP | remove |
| Memory extraction | move here | via API | remove |
| Autonomic regulation | move here | via API | remove |
| Context budgets | move here | via API | remove |
| Bare mode | move here | via API | remove |
| 14 slash commands | endpoints | proxy | remove |
| Streaming output | SSE | ratatui | stdout |
| Concurrent input | N/A | crossterm | N/A (pipe) |
| Session persistence | ✅ has | via API | via API |
| OAuth login | ✅ has | ✅ has | ✅ has |

## Non-Goals (first iteration)
- Mouse-based interaction
- Multi-session tabs
- File tree sidebar
- Inline code editing
