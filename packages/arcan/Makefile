.PHONY: help install dev test lint format build clean

help:
	@echo "Available commands:"
	@echo "  make install    - Install the package in production mode"
	@echo "  make dev        - Install the package in development mode with all extras"
	@echo "  make test       - Run tests with coverage"
	@echo "  make lint       - Run linters (black check, ruff, mypy)"
	@echo "  make format     - Format code with black and ruff"
	@echo "  make build      - Build distribution packages"
	@echo "  make clean      - Remove build artifacts and caches"

install:
	uv sync

dev:
	uv sync --all-extras

test:
	uv run pytest -v --cov=arcan --cov-report=term-missing --cov-fail-under=90

lint:
	./lint.sh

format:
	uv run black .
	uv run ruff check --fix .

build:
	uv build

clean:
	rm -rf dist build *.egg-info .coverage htmlcov .pytest_cache .mypy_cache .ruff_cache
	find . -type d -name __pycache__ -exec rm -rf {} + 2>/dev/null || true
	find . -type f -name "*.pyc" -delete 