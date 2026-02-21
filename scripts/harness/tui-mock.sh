#!/usr/bin/env bash
# script: tui-mock.sh
# Harness script to quickly test the TUI layout and command line without booting the full persistent orchestrator 
set -euo pipefail

echo "=================================================="
echo "    Arcan TUI - Mock / Dry-Run Harness Script     "
echo "=================================================="

# Just verify the binary can build and the CLI args are stable
cargo run -p arcan -- chat --help

echo ""
echo "[OK] TUI mock test passed."
