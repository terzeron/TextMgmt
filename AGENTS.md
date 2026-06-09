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

This project is indexed by GitNexus as **TextMgmt** (5793 symbols, 8811 relationships, 89 execution flows). Use the GitNexus MCP tools to understand code, assess impact, and navigate safely.

> If any GitNexus tool warns the index is stale, run `npx gitnexus analyze` in terminal first.

## Always Do

- **MUST run impact analysis before editing any symbol.** Before modifying a function, class, or method, run `gitnexus_impact({target: "symbolName", direction: "upstream"})` and report the blast radius (direct callers, affected processes, risk level) to the user.
- **MUST run `gitnexus_detect_changes()` before committing** to verify your changes only affect expected symbols and execution flows.
- **MUST warn the user** if impact analysis returns HIGH or CRITICAL risk before proceeding with edits.
- When exploring unfamiliar code, use `gitnexus_query({query: "concept"})` to find execution flows instead of grepping. It returns process-grouped results ranked by relevance.
- When you need full context on a specific symbol — callers, callees, which execution flows it participates in — use `gitnexus_context({name: "symbolName"})`.

## Never Do

- NEVER edit a function, class, or method without first running `gitnexus_impact` on it.
- NEVER ignore HIGH or CRITICAL risk warnings from impact analysis.
- NEVER rename symbols with find-and-replace — use `gitnexus_rename` which understands the call graph.
- NEVER commit changes without running `gitnexus_detect_changes()` to check affected scope.

## Resources

| Resource | Use for |
|----------|---------|
| `gitnexus://repo/TextMgmt/context` | Codebase overview, check index freshness |
| `gitnexus://repo/TextMgmt/clusters` | All functional areas |
| `gitnexus://repo/TextMgmt/processes` | All execution flows |
| `gitnexus://repo/TextMgmt/process/{name}` | Step-by-step execution trace |

## CLI

| Task | Read this skill file |
|------|---------------------|
| Understand architecture / "How does X work?" | `.claude/skills/gitnexus/gitnexus-exploring/SKILL.md` |
| Blast radius / "What breaks if I change X?" | `.claude/skills/gitnexus/gitnexus-impact-analysis/SKILL.md` |
| Trace bugs / "Why is X failing?" | `.claude/skills/gitnexus/gitnexus-debugging/SKILL.md` |
| Rename / extract / split / refactor | `.claude/skills/gitnexus/gitnexus-refactoring/SKILL.md` |
| Tools, resources, schema reference | `.claude/skills/gitnexus/gitnexus-guide/SKILL.md` |
| Index, status, clean, wiki CLI commands | `.claude/skills/gitnexus/gitnexus-cli/SKILL.md` |

<!-- gitnexus:end -->
