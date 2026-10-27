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

## Baseline (2026-10-28)

- `cargo test --workspace`: passed, including all Rust integration targets.
- `cargo llvm-cov --workspace --summary-only`: passed.
- Rust lines: 94.20% (34,932 / 37,081).
- Rust regions: 92.46% (81,699 / 88,358).
- Rust functions: 88.54% (6,071 / 6,857).
- Frontend: 2,473 / 2,473 tests passed; line coverage 100%.
- Python source and tests: none remain in the repository.

The Rust baseline still has 2,151 uncovered lines. Lowest production modules
by line coverage are:

| Module | Line coverage | Uncovered lines |
| --- | ---: | ---: |
| `backend/src/main/google.rs` | 81.82% | 20 |
| `backend/src/classifier/model.rs` | 82.66% | 30 |
| `backend/src/book_manager/file_reader.rs` | 83.21% | 23 |
| `backend/src/classifier/config.rs` | 83.72% | 14 |
| `backend/src/book_manager/update.rs` | 84.62% | 30 |
| `backend/src/cover_status.rs` | 84.70% | 43 |
| `utils/classify_cli/src/commands.rs` | 85.14% | 44 |

The two utility `main.rs` wrappers have 76% line coverage, with six uncovered
lines each. Review their process-level error paths separately from service
logic. Coverage must include all source files; do not hide uncovered code.

This pass added regression coverage for malformed PDF cover metadata and JPEG
crop parser input. Continue with the lowest meaningful service module, simplify
verbose logic where that reduces test burden, and rerun the workspace coverage
baseline after each coherent batch.
