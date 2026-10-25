# Rust 전환 진행표

기준: `feature/rust-backend`, 2026-10-25. 기존 `docs/rust-test-seams.md` 설계와 Python 동작을 기준으로 TDD를 수행한다.
미등록 Rust 테스트는 구현을 가정해 작성한 초안이다. 실제 계약으로 맞추되 테스트의 동작 단언을 보존한다.

## 완료 기준

- 운영 backend·utils의 Python 기능을 Rust 실행 경로에 연결한다.
- 각 변경은 실패 테스트 → 구현 → 해당 테스트 통과 순서로 검증한다.
- API의 인증·입출력·오류 코드와 파일 수정 동작을 Python과 대조한다.
- 미구현 기본값, 미등록 테스트, `ignore`, Python 호출을 전수 확인하고 남은 예외를 명시한다.
- Frontend는 사용자 지시에 따라 제외한다. Backend·utils와 모델 학습의 Python 실행 경로는 전환 대상이다.
- `pdfscan`·`epubscan`은 다른 세션이 맡으므로 이 작업에서 수정하지 않는다.
- Python 원본은 검증 기준으로 유지한다. 배포·push·운영 데이터 변경은 별도 승인 대상이다.

## 순서와 검증

1. **PDF 조회·편집**: `pdf_pages`, `pdf_delete_pages`, `pdf_delete_pages_gaps`를 등록한다.
   캐시·페이지 범위·원본 보존·권한·HTTP 라우트를 검증한다.
2. **미리보기·표지**: `book_preview`, `cover`, `cover_status`, `epub_line_count`를 등록한다.
   실제 PDF·EPUB fixture와 I/O 실패 대역으로 검증한다.
3. **일반 API**: `main`, `main_mock`, `main_gaps`, `main_bulk_rename`, `api_exploration`을 등록한다.
   검색·도서·카테고리·분류·재적재 라우트의 Python 계약을 검증한다.
4. **운영 연결**: 실제 HTTP 세션·Google 검증·ES·DB·매니저 생성과 서버 실행을 연결한다.
   로컬 fake 서버와 실제 프로세스 smoke test로 검증한다.
5. **공통 모듈·도구**: 기존 `doc_reader`·`loader` 작업을 확인하고 미등록 테스트 및 남은 Python 호출을 해소한다.
   문서 fixture, 임시 디렉토리, fake ES로 검증한다.
6. **전수 검증**: Python/Rust 기능 대응표, 미구현·ignore 목록, 실행 구성과 패키징을 확인한다.
   스캐너를 제외한 workspace 테스트·Clippy·관련 Python 회귀 결과와 coverage를 기록한다.

## 현재 상태

- 기존 커밋 `ad3e0ee`: 분류 CLI 8개 명령 이식. 학습·export는 Python 호출이다.
- `00d99f0`: PDF 페이지 조회 라우트, 권한·범위·캐시 테스트 등록.
- 도서/만화의 preview·download·html-resource·validate·cover 라우트를 연결했다.
- `book_preview` 100개, `pdf_pages` 24개, `cover` 23개, `comics_manager` 11개가 통과했다.
  EPUB 테스트 54개의 외부 ES 조건과 임시 파일 테스트 2개의 ignore를 제거했다.
- SVG xlink 표지 참조, RGB/Gray/PNG predictor PDF 이미지, 썸네일 추출기 주입 누락을 수정했다.
- ComicsManager가 BookManagerApi를 위임하도록 연결했다. 만화의 preview·validate·HTML 리소스 경로를 실제 관리자와 검증했다.
- `cargo test --workspace --exclude pdfscan --exclude epubscan --locked`: 2,081 passed, 0 failed, 92 ignored.
- `cargo clippy -p backend --all-targets --locked -- -D warnings`: 통과.
- `uv run --no-sync python -m pytest tests/`: 3,040 passed, 3 failed. 아래의 기존 실패와 동일하다.
- 알려진 기존 Python 실패: PDF 표지 테스트 3건에서 `Image.get_flattened_data`가 없다.
- 별도 미커밋 변경: loader·문서 파서·스캐너·frontend·CI·k8s 등. 변경을 선별해 커밋한다.
- 이번 PDF 편집 단계: 신규 테스트 23개 통과. 파일 백업·재색인·크롭·분할·삽입·URL 미리보기·admin 권한을 검증했다.
- 전체 검사 중 기존 Book 테스트 두 개의 환경 초기화 누락을 독립 실행으로 재현했다.
  `2c46ac2`에서 `setup_env()`를 추가했고, 두 테스트가 각각 환경을 준비하도록 수정했다.
- 최신 workspace 검사: 스캐너를 제외하고 2,104 passed, 0 failed, 92 ignored.
  Python 회귀 검사는 3,040 passed, 3 failed로 기존 Pillow 실패와 같다.

- 표지 호환성 단계의 최신 workspace 검사: 2,127 passed, 0 failed, 92 ignored.
  별도 checkout에서 관련 204개 테스트가 통과했고 Backend Clippy도 통과했다.
  Python 회귀 검사는 3,040 passed, 3 failed로 기존 Pillow 실패와 같다.
  lockfile은 별도 checkout에서 승인 의존성만 해석해 선별했다. 전체 대상 offline metadata는
  캐시에 없는 macOS `core-foundation` 때문에 실패했으나 Linux 테스트는 `--locked --offline`으로 통과했다.

- 일반 조회 API: 도서·만화의 메타데이터, 카테고리 목록·커서 조회, 최신 목록, 검색, 유사 자료를 연결했다.
  실제 관리자와 fake ES를 사용하는 HTTP 테스트 8개가 통과했다. 관련 회귀는 337 passed, 13 ignored이며 Clippy도 통과했다.
  검색 facade 누락을 보완하고 숨긴 카테고리, 최신 제외 목록, 만화 상대 경로와 유사 자료 fallback을 검증했다.

- 카테고리 설정 API 11개를 연결했다. 키워드 CRUD, 숨김·최신 제외 설정, Python bool 규칙을 검증했다.
  DB 초기화와 작업을 blocking pool에서 실행하고, viewer JWT가 설정을 바꾸지 못하는지 확인했다.
  `mapping_api` 9개와 별도 checkout의 관련 165개 테스트가 통과했다.
- 최신 workspace 단위·통합 검사는 2,144 passed, 0 failed, 92 ignored다. Clippy도 통과했다.
  첫 doctest는 다른 checkout과 target을 공유해 `bookstore` crate 탐색에 실패했다.
  checkout 검사가 끝난 뒤 workspace 전체 doctest를 순서대로 재실행해 통과했다. 이후 checkout 검사는 순차 실행한다.
  Python은 3,040 passed, 기존 Pillow 오류 3 failed다.

## 다음 구현과 남은 제한

- 전체 전환은 미완료다. 서버 바이너리는 아직 실행 stub이며 AppDeps 운영 조립도 완성하지 않았다.
- PDF 삭제·순서 변경·크롭·분할·삽입과 도서/만화 admin 편집 API를 추가했다.
  원본 백업 보존, 파일 교체 실패, 재색인 실패, 픽셀·색 공간·EXIF를 실제 fixture로 검증한다.
  테스트 등록 파일은 `tests/rust/backend/pdf_edit.rs`다. 기존 `pdf_delete_pages*` 초안의 전수 이식은 아직 남았다.
- 사용자 승인으로 `jpeg-encoder 0.7.1`을 추가했다. JPEG 크롭에서 원본 양자화·서브샘플링·progressive·grayscale을 보존한다.
  8비트 양자화 값을 지원하며, 255를 넘는 양자화 값과 지원하지 않는 sampling은 명시적 오류로 처리한다.
  비대칭 양자화 테이블과 실제 픽셀을 포함한 PDF 편집 테스트 24개가 통과했다.
- URL 이미지 미리보기 API를 연결했다. `preview_insert_upload` 메서드는 구현했지만 multipart 업로드 라우트는 아직 없다.
  실제 URL fetcher의 `SystemNet` 연결 풀도 기존 stub이다. 이 단계의 URL 테스트는 ImageFetcher 대역을 쓴다.
- 사용자 승인으로 `rusqlite 0.40.2`의 `bundled` 기능을 추가했다. 기존 SQLite 스키마를 유지한다.
  Python 생성 DB의 Rust 읽기, Rust 갱신 DB의 Python 읽기, 파일 시각·크기 기반 무효화를 검증했다.
- 표지 판정·미표지 조회·도서/만화 `/browse`를 연결했다. 실제 첫 이미지(XObject/inline), EPUB 표지,
  시간·페이지·동시 작업 제한, 숨긴 카테고리와 인증을 포함해 `cover_status` 테스트 22개가 통과했다.
- 일반 관리 API와 loader 연결을 이어서 구현한다.
- PDF 썸네일은 빈 페이지와 단일 JPEG/RGB/Gray 픽셀 이미지 페이지를 지원한다.
  텍스트·벡터·1비트·CMYK 등 범용 PDF 렌더링은 아직 Python과 동등하지 않다.
- 다운로드 응답은 파일을 메모리에 읽는다. 운영 연결 전에 대형 파일 스트리밍 경로를 마련해야 한다.
- 기존 facade의 `get_book_preview`를 유지하고 pages 인자를 받는 `get_book_preview_with_pages`를 추가했다.
- epubcheck 테스트는 실제 구현 경계인 CommandRunner로 옮겼다. 60초 전달을 확인하며,
  프로세스 종료는 workspace의 sysdeps `run_kills_a_command_that_exceeds_the_timeout` 테스트가 검증한다.
- GitNexus CLI 인덱스를 갱신했다. FTS 구축은 Invalid UTF8로 실패했다.
  MCP는 오래된 인덱스와 잘못 연결된 심볼 ID를 보고하므로 실제 소스와 회귀 검증을 병행한다.
  이번 변경의 재색인 후에도 Rust 파일에 frontend/Python 심볼명이 연결된다.
  `detect_changes`의 전체 diff HIGH·선별 diff MEDIUM 결과를 안전성 판정 근거로 쓰지 않았다.
- 이 세션은 Python 원본·배포 설정·스캐너 작업을 수정하지 않았다.
