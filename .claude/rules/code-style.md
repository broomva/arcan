# Code Style Rules

## Language & Runtime

- **Rust 2024 Edition** — All code targets the 2024 edition (`edition = "2024"`, `rust-version = "1.85"`).
- **Strict Clippy** — Address all clippy warnings. Use workspace-level `[workspace.lints.clippy]` for consistent lint configuration.
- **Safe Rust** — Avoid `unsafe` unless absolutely necessary (optimize hot paths, FFI).
- **Explicit `unsafe` blocks** — Even inside `unsafe fn`, wrap each unsafe operation in its own `unsafe {}` block (required by 2024 edition).

## Formatting & Linting

**Always run before committing:**
```bash
cargo fmt
cargo clippy --workspace
```

Standard `rustfmt` handles formatting (2024 style edition: sorted imports, updated expression formatting).
Standard `clippy` handles linting.

## Naming Conventions

- **snake_case** for files, functions, variables, modules: `agent_event.rs`, `process_data()`.
- **PascalCase** for types, traits, structs, enums: `AgentEvent`, `ToolHandler`.
- **SCREAMING_SNAKE_CASE** for constants: `MAX_RETRIES`.

## Module Structure

- Organize modules by functionality.
- Use `name.rs` file-based modules (preferred since 2018, standard in 2024). Reserve `mod.rs` only for legacy compatibility.
- Public API should be carefully curated (`pub use`).

## Error Handling

- Use `Result<T, E>` for recoverable errors.
- Use `thiserror` for library errors.
- Use `anyhow` for application/binary errors.
- **Panic** only on unrecoverable bugs (e.g. logic invariants), never on user input.

## Async

- Prefer native `async fn` in traits (stabilized since Rust 1.75) over the `async-trait` crate where dyn-dispatch is not needed.
- Use `BoxFuture` or `async-trait` only when dyn-compatibility (`Arc<dyn Trait>`) is required.

## Comments

- Use `///` for documentation comments on public items.
- Document complex logic inline.
- Avoid obvious comments (`// increment i` is noise).

## Rust 2024 Edition Notes

- `gen` is a reserved keyword — do not use as an identifier.
- `std::env::set_var` and `std::env::remove_var` are now `unsafe` — wrap in `unsafe {}` if used.
- Temporaries in `if let` drop at the end of the `if let` rather than the enclosing block.
- Return-position `impl Trait` captures all in-scope lifetimes by default.
