#!/bin/bash
set -e

echo "Running Black formatter check..."
uv run black --check --diff .

echo "Running Ruff linter..."
uv run ruff check .

echo "Running MyPy type checker..."
uv run mypy . --ignore-missing-imports

echo "All linting checks passed!" 