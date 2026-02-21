#!/usr/bin/env bash
set -euo pipefail

# Starts arcan daemon in watch mode, waits for /health, then launches arcan-tui.
# Defaults to mock mode to avoid provider-env startup failures while developing.

PORT="${PORT:-3101}"
SESSION="${SESSION:-dev-session}"
DATA_DIR="${DATA_DIR:-/tmp/arcan-dev}"
URL="http://127.0.0.1:${PORT}"
ARCAN_MOCK="${ARCAN_MOCK:-1}"

if ! command -v cargo-watch >/dev/null 2>&1; then
  echo "cargo-watch is required."
  echo "Install with: cargo install cargo-watch"
  exit 1
fi

WATCH_PATHS=(
  "-w" "Cargo.toml"
  "-w" "crates/arcan"
  "-w" "crates/arcand"
  "-w" "crates/arcan-core"
  "-w" "crates/arcan-provider"
  "-w" "crates/arcan-harness"
  "-w" "crates/arcan-lago"
)

DAEMON_CMD=(cargo watch "${WATCH_PATHS[@]}" -x "run -p arcan -- --data-dir ${DATA_DIR} --port ${PORT} serve")

echo "Starting daemon watch on ${URL} (data dir: ${DATA_DIR})"
if [[ "${ARCAN_MOCK}" == "1" ]]; then
  echo "Mode: mock provider (OPENAI_API_KEY / ANTHROPIC_API_KEY unset)"
  env -u OPENAI_API_KEY -u ANTHROPIC_API_KEY "${DAEMON_CMD[@]}" &
else
  echo "Mode: provider auto-detect from environment"
  "${DAEMON_CMD[@]}" &
fi
DAEMON_WATCH_PID=$!

cleanup() {
  kill "${DAEMON_WATCH_PID}" >/dev/null 2>&1 || true
  wait "${DAEMON_WATCH_PID}" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

echo "Waiting for daemon health at ${URL}/health ..."
for _ in {1..300}; do
  if curl -fsS "${URL}/health" >/dev/null 2>&1; then
    echo "Daemon is healthy. Launching TUI session '${SESSION}'."
    exec cargo run -p arcan -- chat --url "${URL}" --session "${SESSION}"
  fi
  sleep 0.2
done

echo "Daemon did not become healthy in time."
exit 1
