# Testing Rules

## Test Runner

**`cargo test`** is the standard test runner.

```bash
cargo test                  # Run all tests
cargo test -p arcan-core    # Run specific crate tests
```

## Test Structure

- **Unit Tests**: Place inside the same file or a `tests` module within the file.
  ```rust
  #[cfg(test)]
  mod tests {
      use super::*;

      #[test]
      fn it_works() {
          assert_eq!(2 + 2, 4);
      }
  }
  ```

- **Integration Tests**: Place in `tests/` directory at the crate root.

## Mocking

- Use traits (`Provider`, `ToolUser`) to allow dependency injection.
- Implement mocks manually or use a crate like `mockall` if needed (currently manual mocks are preferred for simplicity).

## Coverage Requirements

- All new features require tests.
- Core logic in `arcan-core` and `arcan-harness` must be well-tested.
- Run `cargo test` before committing.
