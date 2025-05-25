# Arcan 🪄

> A powerful spellbook for modern development

[![PyPI version](https://badge.fury.io/py/arcan.svg)](https://badge.fury.io/py/arcan)
[![Python Support](https://img.shields.io/pypi/pyversions/arcan.svg)](https://pypi.org/project/arcan/)
[![Tests](https://github.com/broomva/arcan/actions/workflows/python-ci.yml/badge.svg)](https://github.com/broomva/arcan/actions/workflows/python-ci.yml)
[![Coverage](https://codecov.io/gh/broomva/arcan/branch/main/graph/badge.svg)](https://codecov.io/gh/broomva/arcan)

## 📖 Overview

Arcan is a powerful development toolkit that provides a collection of "spells" - modular, composable tools for modern software development. Built with a focus on developer experience, observability, and pragmatic iteration.

## ✨ Features

- **Modular Architecture**: Each spell is a self-contained module that can be composed with others
- **Type-Safe**: Full type hints and runtime validation with Pydantic
- **Observable**: Built-in metrics, logging, and tracing support
- **Test-Driven**: Comprehensive test coverage with pytest
- **CLI Interface**: Beautiful CLI with rich formatting and helpful commands

## 🚀 Installation

### From PyPI

```bash
pip install arcan
```

### From Source

```bash
git clone https://github.com/broomva/arcan.git
cd arcan/packages/arcan
pip install -e ".[dev,test]"
```

### Using Docker

```bash
docker run -it ghcr.io/broomva/arcan:latest
```

## 📚 Usage

### Command Line Interface

```bash
# Show version
arcan --version

# List available spells
arcan list-spells

# Cast a spell
arcan cast transform --power 3

# Get help
arcan --help
```

### Python API

```python
import arcan

# Get version
print(arcan.__version__)

# Import specific spells (coming soon)
# from arcan.spells import transform, analyze
```

## 🛠️ Development

### Setup Development Environment

```bash
# Clone the repository
git clone https://github.com/broomva/arcan.git
cd arcan

# Install dependencies
bun install

# Setup Python environment
cd packages/arcan
python -m venv .venv
source .venv/bin/activate  # On Windows: .venv\Scripts\activate
pip install -e ".[dev,test]"
```

### Running Tests

```bash
# Run all tests
bun run test:python

# Run with coverage
pytest --cov=arcan --cov-report=html

# Run specific test markers
pytest -m unit
pytest -m integration
```

### Code Quality

```bash
# Format code
bun run format

# Run linters
bun run lint:python

# Type checking
mypy .
```

### Building

```bash
# Build Python package
bun run build:python

# Build Docker image
docker build -t arcan:local .
```

## 🏗️ Architecture

Arcan follows a modular architecture with these key principles:

1. **Evidence-Driven Development**: All features are tested and measured
2. **Systemic Design**: Components are loosely coupled and independently deployable
3. **Pragmatic Iteration**: Ship early, learn fast, iterate quickly
4. **Resilient Engineering**: Graceful degradation and comprehensive error handling

## 🤝 Contributing

We welcome contributions! Please see our [Contributing Guide](../../CONTRIBUTING.md) for details.

### Development Workflow

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-spell`)
3. Make your changes with tests
4. Run the test suite (`bun run test:python`)
5. Commit your changes (`git commit -m 'Add amazing spell'`)
6. Push to the branch (`git push origin feature/amazing-spell`)
7. Open a Pull Request

## 📄 License

This project is licensed under the MIT License - see the [LICENSE](../../LICENSE) file for details.

## 🙏 Acknowledgments

- Built with [Typer](https://typer.tiangolo.com/) for the CLI
- Formatted with [Black](https://black.readthedocs.io/) and [Ruff](https://beta.ruff.rs/)
- Tested with [pytest](https://pytest.org/)
- Packaged with [Hatch](https://hatch.pypa.io/)

## 📞 Support

- 📧 Email: broomva@gmail.com
- 🐛 Issues: [GitHub Issues](https://github.com/broomva/arcan/issues)
- 💬 Discussions: [GitHub Discussions](https://github.com/broomva/arcan/discussions)
