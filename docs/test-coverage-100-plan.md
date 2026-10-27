# Test and Coverage Plan

## Scope

- Keep the Rust workspace and frontend test suites passing.
- Raise Rust workspace and frontend source line coverage toward 100%.
- Do not exclude uncovered code. Remove a branch only after confirming its callers and behavior.
- Review classifier quality separately from code coverage.

## Verification

- Rust tests: `cargo test --workspace`
- Rust coverage: `cargo llvm-cov --workspace --summary-only`
- Frontend tests and coverage: `cd frontend && npm test`

Coverage totals from before the Rust-only cleanup are stale. Rebuild the baseline
after the complete Rust test suite passes; then prioritize the largest uncovered
service modules and simplify their logic before adding narrowly targeted tests.
