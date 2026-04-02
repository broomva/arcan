# Arcan Shell: End-to-End Testing Guide

**Date**: 2026-04-02
**Scope**: `arcan shell` interactive REPL — all subsystems

This document describes how to run end-to-end tests for the Arcan shell and what to evaluate to detect regressions across the Life framework.

---

## Prerequisites

```bash
# 1. Build arcan
cd ~/broomva/core/life/arcan
cargo build --bin arcan

# 2. Export Anthropic API key (macOS keychain)
export ANTHROPIC_API_KEY="$(security find-internet-password -s 'https://api.anthropic.com' -w 2>/dev/null)"

# 3. Verify key is loaded
echo "Key length: ${#ANTHROPIC_API_KEY}"  # should be > 0
```

---

## Test Levels

### Level 0: Unit + Integration Tests (no API key needed)

```bash
cargo test --workspace
```

Expected: all tests pass (79 unit + 10 integration for `arcan` crate, plus all other workspace crates).

### Level 1: Shell Boot + Slash Commands (mock provider, no API key)

Tests that the shell initializes all subsystems and every slash command works.

```bash
DATA_DIR="/tmp/arcan-e2e-L1-$(date +%s)"

printf '/help\n/status\n/context\n/cost\n/history\n/config\n/memory\n/skill\n/model\n/diff\n/commit\n/sessions\n/consolidate\n' \
  | cargo run --bin arcan -- shell --provider mock --data-dir "$DATA_DIR" --budget 10.0 -y 2>&1
```

**What to verify in the output:**

| Check | Expected |
|-------|----------|
| Banner: `Provider:` | `mock-provider` |
| Banner: `Tools:` | `17` |
| Banner: `Hooks:` | `2` |
| Banner: `Skills:` | `307` (may vary as skills are added) |
| Banner: `[nous] N evaluators active` | `6` |
| Banner: `Journal:` | path to `.redb` file |
| Banner: `Workspace:` | path to `workspace.lance` |
| `/help` | Lists all 17 commands with descriptions |
| `/status` | Shows provider, model, tools, hooks, skills, turns, tokens, cost, economic budget |
| `/context` | Shows CACHEABLE / DYNAMIC / CONVERSATION breakdown, utilization % |
| `/cost` | Shows turns, tokens, cost |
| `/history` | Shows message count, turns, tool calls, tokens |
| `/config` | Shows provider, model, workspace, data dir, memory dir, permissions |
| `/memory` | Lists MEMORY.md (at minimum) |
| `/skill` | Lists all discovered skills alphabetically |
| `/model` | Shows current model |
| `/diff` | Shows git diff output (may be empty) |
| `/commit` | Shows git status output |
| `/sessions` | Lists sessions or reports none |
| `/consolidate` | Prints "Running memory consolidation... Done." |

### Level 2: Tool Execution + Nous Safety (mock provider, no API key)

Tests that tool calls trigger Nous evaluators and update tracking.

```bash
DATA_DIR="/tmp/arcan-e2e-L2-$(date +%s)"

# "file" keyword triggers mock provider's write_file tool call
printf 'ping\nfile\n/cost\n/history\n/status\n' \
  | cargo run --bin arcan -- shell --provider mock --data-dir "$DATA_DIR" --budget 10.0 -y 2>&1
```

**What to verify:**

| Check | Expected |
|-------|----------|
| `ping` response | `Echo: ping` |
| `file` response | `I will write a file.` + `[tool: write_file]` + `OK:` |
| `/cost` | `Turns: 2` |
| `/history` | `Tool calls: 1` |
| `/status` Safety line | `safety_compliance: 1.00` (Nous evaluated the tool call) |
| `/status` Economic line | Shows `$0.0000 / $10.00 budget` |

**Cleanup:** Remove the `test.txt` file the mock provider creates:
```bash
rm -f test.txt
```

### Level 3: Real LLM + Streaming (requires ANTHROPIC_API_KEY)

Tests real Anthropic provider, streaming, tool execution, and cost tracking.

```bash
DATA_DIR="/tmp/arcan-e2e-L3-$(date +%s)"

printf '/status\nWhat is 2+2? Answer in one word.\nPlease read the file Cargo.toml and tell me the package name. Use the read_file tool.\n/cost\n/history\n/status\n' \
  | cargo run --bin arcan -- shell --provider anthropic --data-dir "$DATA_DIR" --budget 1.0 -y 2>&1
```

**What to verify:**

| Check | Expected |
|-------|----------|
| Banner: `Provider:` | `claude-sonnet-*` (real model name) |
| Simple question | Coherent answer (e.g. "Four") |
| Tool call | `[tool: read_file]` + `OK:` with Cargo.toml content |
| `/cost` | `Turns: 2`, non-zero tokens and cost |
| `/history` | `Tool calls: 1`, non-zero token counts |
| `/status` Safety | `safety_compliance: 1.00` |
| `/status` Economic | Cost < $1.00 budget |

### Level 4: Memory System (requires ANTHROPIC_API_KEY)

Tests governed memory tools: offload, search, browse, and MEMORY.md index generation.

```bash
DATA_DIR="/tmp/arcan-e2e-L4-$(date +%s)"

printf 'Remember this fact using the memory_offload tool: "Arcan E2E test passed on 2026-04-02." Use title "e2e-test-result" and tier "episodic".\n/memory\nNow search your memory for "e2e test" using the memory_search tool.\n/cost\n' \
  | cargo run --bin arcan -- shell --provider anthropic --data-dir "$DATA_DIR" --budget 2.0 -y 2>&1
```

**What to verify:**

| Check | Expected |
|-------|----------|
| `[tool: memory_offload]` | `OK: Memory saved: ...e2e-test-result.md` |
| `/memory` | Lists `MEMORY.md` and `e2e-test-result` |
| `[tool: memory_search]` | `OK:` with matches containing "e2e test" |
| File on disk | `$DATA_DIR/memory/e2e-test-result.md` exists with YAML frontmatter |
| MEMORY.md | Contains `## Episodic` section with link to `e2e-test-result.md` |

**Post-test verification:**

```bash
echo "=== Memory files ==="
ls -la "$DATA_DIR/memory/"
echo ""
echo "=== MEMORY.md ==="
cat "$DATA_DIR/memory/MEMORY.md"
echo ""
echo "=== Memory file content ==="
cat "$DATA_DIR/memory/e2e-test-result.md"
```

### Level 5: Session Persistence + Resume

Tests that journal events persist and can be restored across shell restarts.

```bash
DATA_DIR="/tmp/arcan-e2e-L5-$(date +%s)"

# Session 1: Create a conversation
printf 'The secret word is "chrysanthemum".\n' \
  | cargo run --bin arcan -- shell --provider anthropic --data-dir "$DATA_DIR" --budget 2.0 -y 2>&1

# Get the session ID
SESSION_ID=$(ls "$DATA_DIR/shell-journals/" | sed 's/\.redb$//')
echo "Session ID: $SESSION_ID"

# Session 2: Resume and verify context is restored
printf '/sessions\n/history\nWhat was the secret word I told you?\n/cost\n' \
  | cargo run --bin arcan -- shell --provider anthropic --data-dir "$DATA_DIR" --session "$SESSION_ID" --resume -y 2>&1
```

**What to verify:**

| Check | Expected |
|-------|----------|
| Session 2 banner | `[lago] Restored N messages from session ...` |
| `/sessions` | Shows session with event count and `*` marker |
| `/history` | `Messages: N` (N > 1, restored from journal) |
| LLM recall | Correctly answers "chrysanthemum" |

**Post-test verification:**

```bash
echo "=== Journal files ==="
ls -la "$DATA_DIR/shell-journals/"
echo ""
echo "=== Lance workspace ==="
ls -laR "$DATA_DIR/workspace.lance/" | head -15
```

### Level 6: Budget Enforcement

Tests that the budget gate prevents LLM calls when exhausted.

```bash
DATA_DIR="/tmp/arcan-e2e-L6-$(date +%s)"

# Set a very low budget ($0.01) — should allow ~1 turn then warn/block
printf 'Hello\nTell me a long story about a dragon.\nAnother story please.\n/cost\n' \
  | cargo run --bin arcan -- shell --provider anthropic --data-dir "$DATA_DIR" --budget 0.01 -y 2>&1
```

**What to verify:**

| Check | Expected |
|-------|----------|
| First turn | Succeeds normally |
| Subsequent turns | Warning at 80% or "budget exhausted" message |
| `/cost` | Shows cost near or exceeding $0.01 |

---

## Regression Checklist

Use this checklist after any significant change to arcan, arcan-core, arcan-provider, arcan-commands, lago-journal, lago-lance, nous, or praxis crates.

### Critical Path (must pass)

- [ ] `cargo test --workspace` — all tests pass
- [ ] Shell boots with mock provider (Level 1)
- [ ] All 17 slash commands produce output without errors
- [ ] Tool execution works and Nous evaluators fire (Level 2)
- [ ] Journal file created on disk (`.redb`)
- [ ] Workspace Lance directory created with `events.lance/`
- [ ] Memory directory created with `MEMORY.md`

### Provider Integration (requires API key)

- [ ] Real Anthropic streaming works (Level 3)
- [ ] Token counts and cost tracking are non-zero and reasonable
- [ ] Tool calls execute and return results

### Memory & Persistence (requires API key)

- [ ] `memory_offload` creates file with YAML frontmatter (Level 4)
- [ ] `memory_search` finds saved memory by keyword
- [ ] `MEMORY.md` auto-generates with correct sections
- [ ] Session resume restores messages from journal (Level 5)
- [ ] `/sessions` lists sessions with event counts

### Subsystem Health Indicators

| Subsystem | Banner Check | Command Check |
|-----------|-------------|---------------|
| Provider | `Provider: <name>` | `/status` shows provider |
| Tools | `Tools: 17` | Tool calls succeed |
| Hooks | `Hooks: 2` | Hooks fire on session end |
| Skills | `Skills: 307` | `/skill` lists all |
| Nous | `[nous] 6 evaluators active` | `/status` shows safety score |
| Budget | `Budget: $X.XX` | `/cost` tracks spending |
| Session Journal | `Journal: <path>` | `.redb` file exists |
| Workspace Journal | `Workspace: <path> (shared)` | `workspace.lance/` exists |
| Memory | `/memory` lists files | `memory/MEMORY.md` exists |
| Prompt Cache | `/context` shows breakdown | Cacheable > 10K tokens |

### Known Regression Vectors

These areas are most likely to break:

1. **Tokio runtime context** — The shell runs sync. All async journal calls need `Handle::try_current()` with `Runtime::new()` fallback. Watch for `Err(_) => 0` or `Err(_) => Vec::new()` patterns without fallback (was a bug, fixed 2026-04-02).

2. **Provider API changes** — Anthropic may change response format. Check `arcan-provider/src/anthropic.rs` streaming parser.

3. **Lago schema evolution** — If `EventPayload` variants change, `replay_session_messages` may silently skip events. Check `EventPayload::Message` match arm.

4. **Tool registry count** — Adding/removing tools changes the `Tools: N` count. Update expected values.

5. **Skills discovery** — Adding bstack skills changes the `Skills: N` count. The catalog is capped to ~96 tokens so count changes are cosmetic.

6. **MEMORY.md generation** — The `generate_memory_index` function in `prompt.rs` parses YAML frontmatter. Malformed frontmatter in memory files can break the index.

7. **Lance version** — `lago-lance` depends on `lance 0.24`. Major version bumps may change Arrow schema or transaction format.

8. **redb version** — `lago-journal` depends on `redb 2`. Lock contention or schema changes can break session persistence.

---

## Automated Smoke Test Script

Save as `scripts/e2e-smoke.sh` and run with `bash scripts/e2e-smoke.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

echo "=== Arcan E2E Smoke Test ==="
echo ""

DATA_DIR="/tmp/arcan-e2e-smoke-$(date +%s)"
PASS=0
FAIL=0

check() {
  local desc="$1"
  shift
  if "$@" >/dev/null 2>&1; then
    echo "  PASS: $desc"
    ((PASS++))
  else
    echo "  FAIL: $desc"
    ((FAIL++))
  fi
}

# Level 0: Build + Tests
echo "--- Level 0: Build + Tests ---"
check "cargo build" cargo build --bin arcan
check "cargo test"  cargo test --workspace --quiet

# Level 1: Shell Boot (mock)
echo ""
echo "--- Level 1: Shell Boot ---"
OUTPUT=$(printf '/help\n/status\n/context\n/memory\n/sessions\n' \
  | cargo run --bin arcan -- shell --provider mock --data-dir "$DATA_DIR" -y 2>&1)

check "banner shows Tools: 17"       echo "$OUTPUT" | grep -q "Tools: 17"
check "banner shows Hooks: 2"        echo "$OUTPUT" | grep -q "Hooks: 2"
check "banner shows [nous]"          echo "$OUTPUT" | grep -q "\[nous\].*evaluators active"
check "banner shows Journal:"        echo "$OUTPUT" | grep -q "Journal:"
check "banner shows Workspace:"      echo "$OUTPUT" | grep -q "Workspace:.*shared"
check "/help lists commands"         echo "$OUTPUT" | grep -q "Available commands:"
check "/status shows provider"       echo "$OUTPUT" | grep -q "Provider: mock-provider"
check "/context shows CACHEABLE"     echo "$OUTPUT" | grep -q "CACHEABLE"
check "/context shows DYNAMIC"       echo "$OUTPUT" | grep -q "DYNAMIC"
check "/memory lists MEMORY"         echo "$OUTPUT" | grep -q "MEMORY"

# Level 2: Tool Execution (mock)
echo ""
echo "--- Level 2: Tool Execution ---"
DATA_DIR2="/tmp/arcan-e2e-smoke2-$(date +%s)"
OUTPUT2=$(printf 'file\n/status\n' \
  | cargo run --bin arcan -- shell --provider mock --data-dir "$DATA_DIR2" -y 2>&1)

check "write_file tool called"       echo "$OUTPUT2" | grep -q "\[tool: write_file\]"
check "tool returned OK"             echo "$OUTPUT2" | grep -q "OK:"
check "Nous safety score present"    echo "$OUTPUT2" | grep -q "safety_compliance"
rm -f test.txt

# Persistence checks
echo ""
echo "--- Persistence ---"
check "redb journal created"         test -f "$DATA_DIR/shell-journals/"*.redb
check "workspace.lance created"      test -d "$DATA_DIR/workspace.lance/events.lance"
check "MEMORY.md created"            test -f "$DATA_DIR/memory/MEMORY.md"

# Summary
echo ""
echo "=== Results: $PASS passed, $FAIL failed ==="
[ "$FAIL" -eq 0 ] && exit 0 || exit 1
```

---

## Cross-Project Validation

For full Life framework regression testing, run all workspace test suites:

```bash
cd ~/broomva/core/life

# All projects
(cd arcan && cargo fmt && cargo clippy --workspace && cargo test --workspace) && \
(cd lago && cargo fmt && cargo clippy --workspace && cargo test --workspace) && \
(cd autonomic && cargo fmt && cargo clippy --workspace -- -D warnings && cargo test --workspace) && \
(cd praxis && cargo fmt && cargo clippy --workspace -- -D warnings && cargo test --workspace) && \
(cd spaces && cargo fmt && cargo clippy --workspace -- -D warnings && cargo check)
```

Then run the Arcan E2E smoke test:

```bash
cd arcan && bash scripts/e2e-smoke.sh
```
