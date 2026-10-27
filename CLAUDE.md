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

# CodeGraph — Code Intelligence

이 프로젝트의 그래프 탐색은 CodeGraph로 한다. 인덱스는 `.codegraph/`에 있고,
MCP 서버(`codegraph serve --mcp`)가 파일 변경을 감지해 자동 동기화한다.

## Always Do

- 함수·메서드를 고치기 전에 호출자와 영향 범위를 확인한다.
  MCP `codegraph_explore`/caller/impact 도구, 또는 CLI
  `codegraph callers <심볼>`, `codegraph impact <심볼> -d <깊이>`를 쓴다.
- 정방향 추적은 `codegraph callees <심볼>`로 한다.
- 커밋 전 변경 범위는 `git status`와 `git diff --stat`으로 확인한다.
- 결과가 0건이거나 경로가 끊기면 텍스트 검색으로 확인한다.
  axum 라우트의 `run(&deps, …, move |service| …)` 같은 클로저 호출은 놓칠 수 있다.

## Never Do

- 0건 결과를 "사용처 없음"의 증거로 삼지 않는다.
- GitNexus 도구와 훅 출력을 근거로 삼지 않는다. 이 프로젝트는 GitNexus 인덱스를 쓰지 않는다.
