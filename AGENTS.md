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
