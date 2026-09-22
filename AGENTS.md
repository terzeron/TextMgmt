# TextMgmt — Agent Rules

## Language

- Respond in Korean when input is in Korean.
- Keep technical terms in English.

## Tech Stack

- **Backend**: Python 3.13+ / FastAPI, uv for package management
- **Frontend**: React (npm), Vite
- **Testing**: pytest (backend), jest (frontend)
- **Infra**: Kubernetes (`k8s/`), Docker
- **Search**: Elasticsearch (`backend/es_manager.py`)
- **Auth**: JWT (access + refresh token), Google OAuth2

## Project Rules

- Use `uv add` / `uv remove` for Python packages. Never use `pip` directly.
- After backend changes, verify `pytest tests/` passes.
- After frontend changes, verify `cd frontend && npm test` passes.
- When modifying `backend/auth.py` or `backend/refresh_token_store.py`, always run `tests/test_auth.py`.
- Validate k8s YAML changes with `kubectl apply --dry-run=client` before committing.
- Every new FastAPI endpoint must explicitly declare `require_auth` or `require_admin` as a dependency.

## Commit Format

- Title: Conventional Commits (`feat:`, `fix:`, `chore:`, etc.)
- Body: `-` bullet points, imperative mood, max 72 chars per line

## Code Intelligence

This project is indexed by GitNexus as **TextMgmt**. If GitNexus MCP tools are available:

- Run impact analysis before editing any symbol.
- Run `gitnexus_detect_changes()` before committing.
- Use `gitnexus_query` to explore unfamiliar code instead of grepping.
- Never rename symbols with find-and-replace; use `gitnexus_rename`.

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
