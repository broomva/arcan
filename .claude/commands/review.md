Review the current changes for quality and correctness:

1. Run `git diff` to see all unstaged changes
2. Run `git diff --cached` to see staged changes
3. Check for:
   - Correctness: logic errors, off-by-ones, missing error handling
   - Style: naming conventions, Rust idioms, unnecessary complexity
   - Security: input validation, path traversal, resource exhaustion
   - Tests: are new features covered? Are tests meaningful?
4. Summarize findings with specific file:line references
