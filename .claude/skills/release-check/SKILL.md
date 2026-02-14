---
name: release-check
description: Pre-release verification for crates.io publishing
user-invocable: true
disable-model-invocation: true
---

Run pre-release checks for Lago crates:

1. `cargo fmt --check` — formatting
2. `cargo clippy --workspace -- -D warnings` — no warnings
3. `cargo test --workspace` — all tests pass
4. `cargo package --workspace --allow-dirty` — verify packaging
5. Check each crate's `Cargo.toml` for required fields: description, license, repository, readme
6. Verify version consistency across all workspace members
7. Report any missing metadata or issues that would block `cargo publish`
