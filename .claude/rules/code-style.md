# Code Style Rules

## Language & Runtime

- **Rust 2021 Edition** — All code must target the 2021 edition.
- **Strict Clippy** — Address all clippy warnings.
- **Safe Rust** — Avoid `unsafe` unless absolutely necessary (optimize hot paths, FFI).

## Formatting & Linting

**Always run before committing:**
```bash
cargo fmt
cargo clippy
```

Standard `rustfmt` handles formatting.
Standard `clippy` handles linting.

## Naming Conventions

- **snake_case** for files, functions, variables, modules: `agent_event.rs`, `process_data()`.
- **PascalCase** for types, traits, structs, enums: `AgentEvent`, `ToolHandler`.
- **SCREAMING_SNAKE_CASE** for constants: `MAX_RETRIES`.

## Module Structure

- Organize modules by functionality.
- Use `mod.rs` or `name.rs` consistently (Rust 2018+ prefers `name.rs`).
- Public API should be carefully curated (`pub use`).

## Error Handling

- Use `Result<T, E>` for recoverable errors.
- Use `thiserror` for library errors.
- Use `anyhow` for application/binary errors.
- **Panic** only on unrecoverable bugs (e.g. logic invariants), never on user input.

## Comments

- Use `///` for documentation comments on public items.
- Document complex logic inline.
- Avoid obvious comments (`// increment i` is noise).
