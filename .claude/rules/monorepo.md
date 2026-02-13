# Workspace Guidelines

## Structure

```
arcan-rs/
├── crates/             # Workspace members
│   ├── arcan-core/    # Shared types & traits
│   ├── arcan-harness/ # Tool implementations & sandboxing
│   ├── arcan-store/   # Persistence layer
│   └── arcan-daemon/  # Executable & Agent Loop
├── Cargo.toml          # Workspace definition
├── AGENTS.md           # Project documentation
└── CLAUDE.md           # Quick reference
```

## Dependency Rules

- **`arcan-core`** is the foundation. It should have minimal dependencies and defines the shared vocabulary.
- **`arcan-harness`** depends on `arcan-core`.
- **`arcan-store`** depends on `arcan-core`.
- **`arcan-daemon`** depends on all of the above.

## Build Orchestration

Cargo handles the workspace build natively.

```bash
cargo build --workspace      # Build all crates
cargo check --workspace      # Check all crates
cargo test --workspace       # Test all crates
cargo clean                  # Clean build artifacts
```

## Adding a New Crate

1. Create `crates/<name>`
2. Add `Cargo.toml` with `[package]` metadata.
3. Add to root `Cargo.toml` `workspace.members`.
4. Use path dependencies for internal crates:
   ```toml
   arcan-core = { path = "../arcan-core" }
   ```
