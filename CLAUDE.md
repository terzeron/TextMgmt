# TextMgmt — Project Rules

## Language

- 한국어 입력에는 한국어로 응답한다.
- 기술 용어는 English 유지.

## Tech Stack

- **Backend**: Python 3.13+ / FastAPI, uv for package management
- **Frontend**: React (npm), Vite
- **Testing**: pytest (backend), jest (frontend)
- **Infra**: Kubernetes (`k8s/`), Docker
- **Search**: Elasticsearch (`backend/es_manager.py`)
- **Auth**: JWT (access + refresh token), Google OAuth2

## Project Rules

- Python 패키지 추가/제거는 `uv add` / `uv remove`를 사용한다. `pip` 직접 사용 금지.
- 백엔드 변경 후 `pytest tests/` 통과를 확인한다.
- 프론트엔드 변경 후 `cd frontend && npm test` 통과를 확인한다.
- auth 관련 코드(`backend/auth.py`, `backend/refresh_token_store.py`) 수정 시 `tests/test_auth.py`를 반드시 실행한다.
- k8s YAML 수정 시 `kubectl apply --dry-run=client`로 유효성을 검증한다.
- FastAPI 엔드포인트 추가 시 `require_auth` 또는 `require_admin` 의존성을 명시적으로 붙인다.

## Commit Format

- 제목: Conventional Commits (`feat:`, `fix:`, `chore:` 등)
- 본문: `-` bullet, 명령형, 72자 이내

---

<!-- gitnexus:start -->
# GitNexus — Code Intelligence

This project is indexed by GitNexus as **TextMgmt** (4899 symbols, 14055 relationships, 300 execution flows). Use the GitNexus MCP tools to understand code, assess impact, and navigate safely.

> Index stale? Run `node .gitnexus/run.cjs analyze` from the project root — it auto-selects an available runner. No `.gitnexus/run.cjs` yet? `npx gitnexus analyze` (npm 11 crash → `npm i -g gitnexus`; #1939).

## Always Do

- **MUST run impact analysis before editing any symbol.** Before modifying a function, class, or method, run `impact({target: "symbolName", direction: "upstream"})` and report the blast radius (direct callers, affected processes, risk level) to the user.
- **MUST run `detect_changes()` before committing** to verify your changes only affect expected symbols and execution flows. For regression review, compare against the default branch: `detect_changes({scope: "compare", base_ref: "main"})`.
- **MUST warn the user** if impact analysis returns HIGH or CRITICAL risk before proceeding with edits.
- When exploring unfamiliar code, use `query({search_query: "concept"})` to find execution flows instead of grepping. It returns process-grouped results ranked by relevance.
- When you need full context on a specific symbol — callers, callees, which execution flows it participates in — use `context({name: "symbolName"})`.
- For security review, `explain({target: "fileOrSymbol"})` lists taint findings (source→sink flows; needs `analyze --pdg`).

## Never Do

- NEVER edit a function, class, or method without first running `impact` on it.
- NEVER ignore HIGH or CRITICAL risk warnings from impact analysis.
- NEVER rename symbols with find-and-replace — use `rename` which understands the call graph.
- NEVER commit changes without running `detect_changes()` to check affected scope.

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
