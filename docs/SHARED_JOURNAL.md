# Shared Workspace Journal — Architecture Proposal

> **Date**: 2026-04-01
> **Status**: Proposed
> **Ticket**: BRO-378

## Problem

Arcan has three journal isolation levels, none of which provide shared workspace context:

| Mode | Journal | Scope | Can coexist? | Shares discoveries? |
|------|---------|-------|-------------|-------------------|
| Daemon (`arcan serve`) | `.arcan/journal.redb` | All daemon sessions | No (exclusive lock) | Yes (within daemon) |
| Shell (`arcan shell`) | `.arcan/shell-journals/<session>.redb` | Single session | Yes (per-file lock) | No (isolated) |
| Shell (ephemeral) | In-memory | Single session | Yes | No |

**Key gaps:**
1. Shell sessions can't see each other's discoveries
2. Shell can't see daemon's workspace history
3. Daemon can't see shell sessions' contributions
4. Governed memory (MemoryQuery/Propose/Commit) targets session journal, not workspace
5. No shared workspace "consciousness" across all agents

## Proposed Architecture: Dual-Journal Model

```
.arcan/
├── workspace.redb            ← SHARED journal (all agents read/write)
│                               Concurrency: read-many, write-serialized
│                               Content: decisions, discoveries, memory, workspace state
│
├── sessions/                 ← PER-SESSION journals (one lock per session)
│   ├── daemon/               ← Daemon sessions
│   │   ├── <session-a>.redb
│   │   └── <session-b>.redb
│   └── shell/                ← Shell sessions
│       ├── <session-c>.redb
│       └── <session-d>.redb
│
├── memory/                   ← Filesystem memory (always readable, exported from workspace.redb)
│
└── blobs/                    ← Content-addressed blob storage (shared)
```

### workspace.redb — The Shared Brain

This is the workspace's collective memory. Every agent writes here, every agent reads from here.

**What goes in the shared journal:**
- `MemoryProposed` / `MemoryCommitted` events (governed memory)
- `DecisionMade` events (key architectural decisions from any session)
- `DiscoveryLogged` events (insights, patterns, learned rules)
- `FileChanged` events (workspace file mutations tracked by LagoTrackedFs)
- `SessionSummary` events (what each session accomplished)

**What stays in session journals:**
- Conversation messages (user/assistant turns)
- Tool call details (inputs, outputs, errors)
- Streaming deltas
- Per-session token usage and cost

### Concurrency Model

redb supports one writer + multiple readers. The shared journal uses:

1. **Write serialization**: A lightweight write-ahead log or channel
   - Each agent sends events to a shared `mpsc` channel
   - A single writer task drains the channel and appends to workspace.redb
   - This is the pattern `run_event_writer()` already uses in the daemon

2. **Read access**: Multiple agents can read simultaneously
   - redb supports concurrent read transactions
   - Each shell session opens workspace.redb in read-only mode for queries
   - Write access goes through the serialized channel

3. **Fallback**: If write serialization isn't available (no daemon running):
   - Shell sessions try to open workspace.redb with exclusive write access
   - If locked, fall back to writing a batch file (`.arcan/pending-events.jsonl`)
   - Next session or daemon ingests pending events on startup

### Integration with Liquid Prompt

The shared journal feeds into the liquid prompt as a new section:

```
# Workspace Context (from shared journal)

## Recent Decisions
- [2h ago] Chose arcan shell over noesis for CLI (session 01KN58...)
- [4h ago] Merged PR #43 with governed memory + Spaces (session 01KN52...)

## Active Knowledge
- Project is arcan v0.2.1, Rust 2024 edition
- 12 tools registered, 307 skills discovered
- Last test run: 101 passing, 0 failures

## Peer Sessions
- Session 01KN58... (shell, active, 5 turns, $0.12)
- Session 01KN52... (shell, ended, 20 turns, $0.45)
```

### Integration with Governed Memory

Currently, governed memory tools (MemoryQuery/Propose/Commit) target the session journal. With the shared journal:

```
MemoryQueryTool   → reads from workspace.redb (shared)
MemoryProposeTool → writes proposal to workspace.redb (shared)
MemoryCommitTool  → commits to workspace.redb (shared)
```

This means memory proposed by one session is immediately visible to all other sessions. The workspace accumulates intelligence over time.

### Integration with Spaces

The shared journal bridges local and distributed:

```
Local:  workspace.redb  ←→  Spaces #agent-logs channel
                              ↑
                         Events synced bidirectionally:
                         - Local decisions → published to Spaces
                         - Remote agent activity → ingested to local journal
```

## Implementation Plan

### Phase A: Shared Journal Infrastructure

1. Create `workspace.redb` alongside session journals
2. Implement write-serialized channel (mpsc → single writer)
3. Implement read-only access for concurrent sessions
4. Add pending-events.jsonl fallback for when writer isn't running

### Phase B: Wire Governed Memory to Shared Journal

1. MemoryQuery/Propose/Commit target workspace.redb
2. Session journal gets conversation-only events
3. Memory projection reads from shared journal

### Phase C: Liquid Prompt Integration

1. On shell startup, read recent events from workspace.redb
2. Build "Workspace Context" section in liquid prompt
3. Include recent decisions, active knowledge, peer sessions

### Phase D: Session Summaries

1. On session end, write SessionSummary event to shared journal
2. Summary includes: key decisions, files changed, tools used, cost
3. Next session sees this in "Peer Sessions" context

### Phase E: Spaces Sync

1. Bidirectional sync between workspace.redb and Spaces #agent-logs
2. Local events → published to Spaces (for remote agents)
3. Remote events → ingested to local journal (for local context)

## Relationship to Existing Architecture

```
                     ┌────────────────────────┐
                     │   workspace.redb        │
                     │   (shared workspace     │
                     │    consciousness)       │
                     └───────┬────────────────┘
                             │
              ┌──────────────┼──────────────┐
              │              │              │
     ┌────────┴────┐  ┌─────┴─────┐  ┌─────┴─────┐
     │ Daemon      │  │ Shell A   │  │ Shell B   │
     │ sessions/   │  │ sessions/ │  │ sessions/ │
     │ daemon/*.db │  │ shell/*.db│  │ shell/*.db│
     └─────────────┘  └───────────┘  └───────────┘
                             │
                     ┌───────┴────────┐
                     │  Spaces        │
                     │  #agent-logs   │
                     │  (distributed) │
                     └────────────────┘
```

This creates a three-tier persistence model:
1. **Session** — conversation detail (per-session .redb)
2. **Workspace** — shared knowledge (workspace.redb)
3. **Network** — distributed coordination (Spaces)
