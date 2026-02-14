# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2025-06-01

### Added

- Core agent runtime with event-sourced state management (`arcan-core`)
- Tool harness with filesystem, editing, memory, and sandboxing (`arcan-harness`)
- Append-only JSONL session persistence (`arcan-store`)
- Anthropic Claude LLM provider (`arcan-provider`)
- Agent loop with SSE streaming and HTTP routing (`arcand`)
- Lago event-sourced persistence bridge (`arcan-lago`)
- Production binary with Clap CLI, structured logging, and policy middleware (`arcan`)
- Hashline editing for safe file modifications
- Sandbox policy enforcement for tool execution

[0.1.0]: https://github.com/broomva/arcan-rs/releases/tag/v0.1.0
