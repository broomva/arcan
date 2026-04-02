#!/usr/bin/env bash
# Arcan Shell E2E Smoke Test
# Run: bash scripts/e2e-smoke.sh
# Optionally set ANTHROPIC_API_KEY for Level 3+ tests.
set -uo pipefail

echo "=== Arcan E2E Smoke Test ==="
echo ""

PASS=0
FAIL=0

pass() { echo "  PASS: $1"; PASS=$((PASS + 1)); }
fail() { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); }

# Check that a variable contains a pattern
has() {
  local desc="$1" var="$2" pattern="$3"
  if echo "$var" | grep -q "$pattern" 2>/dev/null; then
    pass "$desc"
  else
    fail "$desc"
  fi
}

# Check a command succeeds
ok() {
  local desc="$1"; shift
  if "$@" >/dev/null 2>&1; then
    pass "$desc"
  else
    fail "$desc"
  fi
}

# ── Level 0: Build + Tests ─────────────────────────────────────────
echo "--- Level 0: Build + Tests ---"
if ! cargo build --bin arcan >/dev/null 2>&1; then
  fail "cargo build --bin arcan"
  echo "Build failed — cannot continue."
  exit 1
fi
pass "cargo build --bin arcan"

if cargo test --workspace --quiet 2>&1 | tail -1 | grep -q "^$"; then
  # --quiet only outputs on failure; check exit code directly
  true
fi
if cargo test --workspace --quiet >/dev/null 2>&1; then
  pass "cargo test --workspace"
else
  fail "cargo test --workspace"
fi

# ── Level 1: Shell Boot (mock) ─────────────────────────────────────
echo ""
echo "--- Level 1: Shell Boot (mock provider) ---"
DATA_L1="/tmp/arcan-e2e-L1-$$"
OUTPUT_L1=$(printf '/help\n/status\n/context\n/cost\n/history\n/config\n/memory\n/skill\n/model\n/diff\n/commit\n/sessions\n/consolidate\n/search\n' \
  | cargo run --bin arcan -- shell --provider mock --data-dir "$DATA_L1" --budget 10.0 -y 2>&1)

has "banner: Tools: 17"            "$OUTPUT_L1" "Tools: 17"
has "banner: Hooks: 2"             "$OUTPUT_L1" "Hooks: 2"
has "banner: [nous] evaluators"    "$OUTPUT_L1" "evaluators active"
has "banner: Journal path"         "$OUTPUT_L1" "Journal:"
has "banner: Workspace (shared)"   "$OUTPUT_L1" "Workspace:.*shared"
has "banner: Budget shown"         "$OUTPUT_L1" "Budget:"
has "/help lists commands"         "$OUTPUT_L1" "Available commands:"
has "/status shows provider"       "$OUTPUT_L1" "Provider: mock-provider"
has "/status shows safety line"    "$OUTPUT_L1" "Safety:"
has "/status shows economic line"  "$OUTPUT_L1" "Economic:"
has "/context CACHEABLE section"   "$OUTPUT_L1" "CACHEABLE"
has "/context DYNAMIC section"     "$OUTPUT_L1" "DYNAMIC"
has "/context CONVERSATION"        "$OUTPUT_L1" "CONVERSATION"
has "/context utilization %"       "$OUTPUT_L1" "Utilization:"
has "/cost shows turns"            "$OUTPUT_L1" "Turns:"
has "/history shows messages"      "$OUTPUT_L1" "Messages:"
has "/config shows workspace"      "$OUTPUT_L1" "Workspace:"
has "/memory lists MEMORY"         "$OUTPUT_L1" "MEMORY"
has "/skill lists skills"          "$OUTPUT_L1" "Discovered skills"
has "/model shows model"           "$OUTPUT_L1" "model"
has "/consolidate runs"            "$OUTPUT_L1" "consolidation"
has "/search shows usage"          "$OUTPUT_L1" "Usage:"
ok  "redb journal created"         test -f "$DATA_L1/shell-journals/"*.redb
ok  "workspace.lance created"      test -d "$DATA_L1/workspace.lance"
ok  "MEMORY.md created"            test -f "$DATA_L1/memory/MEMORY.md"

# ── Level 2: Tool Execution + Nous (mock) ──────────────────────────
echo ""
echo "--- Level 2: Tool Execution + Nous Safety (mock provider) ---"
DATA_L2="/tmp/arcan-e2e-L2-$$"
OUTPUT_L2=$(printf 'ping\nfile\n/cost\n/history\n/status\n' \
  | cargo run --bin arcan -- shell --provider mock --data-dir "$DATA_L2" --budget 10.0 -y 2>&1)

has "echo response works"          "$OUTPUT_L2" "Echo: ping"
has "write_file tool called"       "$OUTPUT_L2" "\[tool: write_file\]"
has "tool returned OK"             "$OUTPUT_L2" "OK:"
has "turns incremented to 2"       "$OUTPUT_L2" "Turns:  2"
has "tool call counted"            "$OUTPUT_L2" "Tool calls: 1"
has "Nous safety_compliance"       "$OUTPUT_L2" "safety_compliance"
rm -f test.txt  # clean up mock tool artifact

# ── Level 3: Session Persistence (mock) ────────────────────────────
echo ""
echo "--- Level 3: Session Persistence + Resume (mock provider) ---"
DATA_L3="/tmp/arcan-e2e-L3-$$"

# Session 1: create a conversation with events
printf 'ping\nfile\n' | cargo run --bin arcan -- shell --provider mock --data-dir "$DATA_L3" -y >/dev/null 2>&1
rm -f test.txt

SESSION_ID=$(ls "$DATA_L3/shell-journals/" 2>/dev/null | head -1 | sed 's/\.redb$//')
if [ -n "$SESSION_ID" ]; then
  # Session 2: resume and verify
  OUTPUT_L3=$(printf '/sessions\n/history\n' \
    | cargo run --bin arcan -- shell --provider mock --data-dir "$DATA_L3" --session "$SESSION_ID" --resume -y 2>&1)

  has "resume restores messages"   "$OUTPUT_L3" "Restored.*messages"
  has "/sessions shows session"    "$OUTPUT_L3" "$SESSION_ID"
  has "/history shows restored"    "$OUTPUT_L3" "Messages:"
else
  fail "no session journal file found"
fi

# ── Level 4: Real LLM (optional) ───────────────────────────────────
if [ -n "${ANTHROPIC_API_KEY:-}" ]; then
  echo ""
  echo "--- Level 4: Real Anthropic Provider ---"
  DATA_L4="/tmp/arcan-e2e-L4-$$"
  OUTPUT_L4=$(printf 'What is 2+2? Answer in one word.\n/cost\n/status\n' \
    | cargo run --bin arcan -- shell --provider anthropic --data-dir "$DATA_L4" --budget 1.0 -y 2>&1)

  has "real provider connects"       "$OUTPUT_L4" "Provider: claude"
  has "LLM returns response"         "$OUTPUT_L4" "[Ff]our"
  has "tokens are non-zero"          "$OUTPUT_L4" "Tokens: [1-9]"
  has "cost is non-zero"             "$OUTPUT_L4" 'Cost:.*\$0\.[0-9]'

  echo ""
  echo "--- Level 5: Memory Tools (real provider) ---"
  DATA_L5="/tmp/arcan-e2e-L5-$$"
  OUTPUT_L5=$(printf 'Save this to memory using memory_offload with title "smoke-result" and tier "episodic": "smoke test passed on today"\nSearch memory for "smoke" using the memory_search tool.\n/memory\n' \
    | cargo run --bin arcan -- shell --provider anthropic --data-dir "$DATA_L5" --budget 2.0 -y 2>&1)

  has "memory_offload called"        "$OUTPUT_L5" "\[tool: memory_offload\]"
  ok  "memory file created"          test -f "$DATA_L5/memory/smoke-result.md"
  has "memory_search called"         "$OUTPUT_L5" "\[tool: memory_search\]"
  has "search found result"          "$OUTPUT_L5" "matches"
  has "/memory lists new file"       "$OUTPUT_L5" "smoke-result"
else
  echo ""
  echo "--- Levels 4-5: SKIPPED (no ANTHROPIC_API_KEY) ---"
fi

# ── Cleanup ─────────────────────────────────────────────────────────
rm -rf /tmp/arcan-e2e-L*-$$

# ── Summary ─────────────────────────────────────────────────────────
echo ""
echo "==========================================="
echo "  Results: $PASS passed, $FAIL failed"
echo "==========================================="
[ "$FAIL" -eq 0 ] && exit 0 || exit 1
