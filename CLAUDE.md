# TextMgmt — Project Rules

## Language

- 한국어 입력에는 한국어로 응답한다.
- 기술 용어는 English 유지.

## Tech Stack

- **Backend**: Rust workspace (`backend` crate)
- **Frontend**: React (npm), Vite
- **Testing**: cargo test / llvm-cov (backend and utilities), jest (frontend)
- **Infra**: Kubernetes (`k8s/`), Docker
- **Search**: Elasticsearch (`backend/src/es_manager.rs`)
- **Auth**: JWT (access + refresh token), Google OAuth2

## Directory Layout

- `backend/` Rust API, `frontend/` React, `utils/` Rust 실행 도구,
  `libs/` 공통 모듈 crate, `tests/rust/<crate>/` Rust 테스트.
- Rust workspace는 저장소 루트 `Cargo.toml`이다. 규칙과 crate 목록은 `docs/rust-workspace.md`를 본다.
- Rust 변경 후 `cargo test --workspace`와 `cargo clippy --workspace --all-targets`를 통과시킨다.
- 모든 Rust 바이너리는 인자 없이 실행하면 `print_usage()`로 용도와 사용법을 출력한다.

## Project Rules

- 백엔드 변경 후 관련 Rust 테스트를 실행한다. 넓은 변경은 `cargo test --workspace`를 실행한다.
- 프론트엔드 변경 후 `cd frontend && npm test` 통과를 확인한다.
- auth 또는 refresh-token Rust 코드를 수정하면 대응하는 Rust 테스트를 실행한다.
- k8s YAML 수정 시 `kubectl apply --dry-run=client`로 유효성을 검증한다.
- API 엔드포인트 추가 시 인증 요구사항을 명시적으로 적용한다.

## Commit Format

- 제목: Conventional Commits (`feat:`, `fix:`, `chore:` 등)
- 본문: `-` bullet, 명령형, 72자 이내

---

<!-- gitnexus:start -->
# GitNexus — Code Intelligence

This project is indexed by GitNexus as **TextMgmt** (7824 symbols, 21699 relationships, 683 execution flows).

> Index stale? Run `node .gitnexus/run.cjs analyze --index-only` from the project root — it auto-selects an available runner. No `.gitnexus/run.cjs` yet? Bootstrap with `npx`, `bunx`, or `pnpm dlx` — e.g. `bunx gitnexus@latest analyze` (npm 11 npx crash; #1939).

## Always Do

- **MUST run impact before editing.** Use `impact({target: "symbolName", direction: "upstream"})` or `node .gitnexus/run.cjs impact "symbolName" --direction upstream --repo .`; report callers, processes, and risk. Never substitute grep for graph analysis.
- **MUST analyze graph changes before committing.** Use `detect_changes({scope: "all"})` (MCP) or `node .gitnexus/run.cjs detect-changes --scope all --repo .` (CLI fallback). `partial: true` or `truncated: true` is not a clean check — a zero means unseen, not unaffected; re-run it. For regression review: `detect_changes({scope: "compare", base_ref: "main"})` or `node .gitnexus/run.cjs detect-changes --scope compare --base-ref "main" --repo .`.
- MUST warn on HIGH/CRITICAL `risk` pre-edit; never use `riskSharedAxes` to waive a HIGH/CRITICAL `risk` warning. Compare File/symbol: MCP File omits axes; Graph-RAG expands File.
- **MUST treat `risk: UNKNOWN` as unresolved, not as low.** An empty caller set is not evidence the symbol is unused — it can also mean the callers are not resolvable by the index (plain-object property access, dynamic dispatch, cross-language calls). `impact` pairs `UNKNOWN` with a `riskNote` saying so. Confirm with a text search before treating the symbol as safe to change or delete; do not proceed on the strength of a zero.
- **MUST use `query({search_query: "concept"})` for concepts/flows, `context({name: "symbolName"})` for a named symbol, or `impact` for blast radius, on read-only callers, dependencies, imports, or execution flow.** Graph first; text search only for empty/`UNKNOWN`/literals.
- For security review, `explain({target: "fileOrSymbol"})` lists taint findings (source→sink flows; needs `analyze --pdg`).

## Never Do

- NEVER edit a function, class, or method before MCP/CLI impact analysis.
- NEVER ignore HIGH or CRITICAL risk warnings from impact analysis, and never read `UNKNOWN` as an all-clear — it means the walk could not answer, which is the one verdict that requires confirming by other means.
- NEVER rename symbols with find-and-replace — use `rename` which understands the call graph.
- NEVER commit before MCP/CLI graph change analysis.

## Resources

| Resource | Use for |
| --- | --- |
| `gitnexus://repo/TextMgmt/context` | Codebase overview, check index freshness |
| `gitnexus://repo/TextMgmt/clusters` | All functional areas |
| `gitnexus://repo/TextMgmt/processes` | All execution flows |
| `gitnexus://repo/TextMgmt/process/{name}` | Step-by-step execution trace |

## CLI

| Task | Read this skill file |
| --- | --- |
| Understand architecture / "How does X work?" | `.claude/skills/gitnexus-exploring/SKILL.md` |
| Blast radius / "What breaks if I change X?" | `.claude/skills/gitnexus-impact-analysis/SKILL.md` |
| Trace bugs / "Why is X failing?" | `.claude/skills/gitnexus-debugging/SKILL.md` |
| Rename / extract / split / refactor | `.claude/skills/gitnexus-refactoring/SKILL.md` |
| Tools, resources, schema reference | `.claude/skills/gitnexus-guide/SKILL.md` |
| Index, status, clean, wiki CLI commands | `.claude/skills/gitnexus-cli/SKILL.md` |

<!-- gitnexus:end -->
