---
name: check
description: Run full workspace validation (format, lint, build, test)
user-invocable: true
argument-hint: "[--quick]"
---

Run the full Lago workspace validation pipeline:

1. `cargo fmt --check` — verify formatting
2. `cargo clippy --workspace -- -D warnings` — strict lint
3. `cargo build --workspace` — compile all crates
4. `cargo test --workspace` — run all tests

If $ARGUMENTS contains "--quick", skip the full test suite and only run `cargo check --workspace` instead of build+test.

Report a summary table of pass/fail for each step.
