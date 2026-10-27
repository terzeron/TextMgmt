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

- 도서·만화 수정/삭제, 카테고리 이름 변경/삭제/삭제 미리보기/PDF 통계 API를 연결했다.
  파일 삭제와 이동, 기본 파일 보존, 하위 카테고리 정책 정리, viewer JWT 차단, 라이브러리 밖 이동 차단을 검증했다.
  axum static 경로와 충돌하는 `rename`·`delete` 카테고리 GET도 유지했다.
  `book_admin_api` 8개를 포함한 관련 회귀 417개가 통과했다(기존 ignore 13개).

- 서버 CLI의 실패 테스트 3개를 먼저 확인하고 `backend serve --addr HOST:PORT`를 구현했다.
  loopback 임시 포트의 실제 HTTP 응답, 잘못된 인자, 포트 충돌, SIGTERM 정상 종료가 통과했다.
  당시 workspace 전체 검사(doctest 포함): 2,155 passed, 0 failed, 92 ignored. Backend Clippy도 통과했다.
- loader 연동을 TDD로 진행했다. RED에서 `LoaderFileReader` 누락을 확인하고 adapter를 구현했다.
  실제 TXT·EPUB metadata/본문, 만화 PDF의 `skip_text`, 요청 경로 prefix 동작을 2개 테스트로 검증했다.
  `loader`의 Clippy 경고 2건도 타입 별칭과 재귀 인자 제거로 정리했다.
  `cargo test -p backend --test loader_file_reader --locked` 2개 통과,
  `cargo test -p loader --locked` 52개 통과·10개 무시,
  `cargo clippy -p backend --all-targets --locked -- -D warnings` 통과.
  최종 workspace 검사: 2,157 passed, 0 failed, 92 ignored.
- PDF 페이지 삭제·크롭·분할·삽입 테스트 묶음을 등록하고 구 fixture를 현재 `FileReader`, `ImageFetcher`, `EsManagerApi` seam에 맞췄다.
  multipart 업로드 미리보기, 비유한 회전각 거부, 이미지 pixel limit 주입, PDF 손상 데이터 오류, 배치 실패 전 원본 보존을 보완했다.
  이미 존재하는 PDF edit 회귀와 충돌하는 backup 정책은 유지하고, 실패 주입은 backup 전 sync 단계로 옮겼다.
  `pdf_delete_pages` 71개, `pdf_delete_pages_gaps` 10개, `pdf_edit` 24개가 통과했다.
  workspace 검사: 2,238 passed, 0 failed, 92 ignored. Backend Clippy도 통과했다.
- 파일 일괄 이름 변경 API를 TDD로 연결했다. RED에서 미등록 route의 405를 확인한 뒤
  카테고리 하위 목록 paging, 정규식 치환, 안전한 파일명 검증, 충돌 suffix 예약,
  디렉터리 조회 및 manager 갱신 실패 보고를 구현했다. `main_bulk_rename` 16개 테스트와
  Backend Clippy가 통과했다. 전체 Rust workspace 검사도 2,254 passed, 0 failed, 92 ignored로 통과했다.
- 관리 API 공백을 TDD로 확인하고 `/similar-names`, classify-proposal 삭제를 연결했다.
  기존 분류 적용 로직을 facade에 노출하고 proposal status 저장소 seam을 추가했다.
  `main_gaps` 9개와 `main_bulk_rename` 16개, Backend Clippy가 통과했다.
  workspace 검사: 2,263 passed, 0 failed, 92 ignored.
- Python 원본의 directory score 사례를 `main_gaps`에 추가했다. 해당 테스트 묶음 10개와
  Backend Clippy가 통과했다.
- 분류기 학습의 RED 테스트에서 Python 잔류 오류를 확인한 뒤 Rust 희소 TF-IDF와
  One-vs-Rest squared-hinge 학습, holdout 평가, threshold/calibration 메타데이터를 구현했다.
  `classify_cli train`도 Python subprocess와 export 단계를 제거하고 Rust 모델·holdout manifest를 저장한다.
  학습 round-trip·예측 테스트 1개, classifier parity 11개, model 24개, calibration 18개,
  classify_cli 23개가 통과했다. Backend/CLI Clippy와 scanner 제외 workspace 전체 테스트도 통과했다.

## 다음 구현과 남은 제한

- 전체 전환은 미완료다. 현재 startup script와 Kubernetes 이미지의 backend/loader 실행은 Rust다.
  Docker Python stage는 PDFium shared library만 가져오며 Python app을 실행하지 않는다.
  남은 일은 Rust/Python 기능 동등성 전수 검증, 운영 corpus 비교, 외부 서비스 의존 테스트 검증이다.
- PDF 삭제·순서 변경·크롭·분할·삽입과 도서/만화 admin 편집 API를 추가했다.
  원본 백업 보존, 파일 교체 실패, 재색인 실패, 픽셀·색 공간·EXIF를 실제 fixture로 검증한다.
  테스트 등록 파일은 `tests/rust/backend/pdf_edit.rs`, `pdf_delete_pages.rs`, `pdf_delete_pages_gaps.rs`다.
- 사용자 승인으로 `jpeg-encoder 0.7.1`을 추가했다. JPEG 크롭에서 원본 양자화·서브샘플링·progressive·grayscale을 보존한다.
  8비트 양자화 값을 지원하며, 255를 넘는 양자화 값과 지원하지 않는 sampling은 명시적 오류로 처리한다.
  비대칭 양자화 테이블과 실제 픽셀을 포함한 PDF 편집 테스트 24개가 통과했다.
- URL 이미지와 multipart 업로드 이미지 미리보기 API를 모두 연결했다.
  업로드 파일 누락, 손상 이미지, admin 권한, no-store 응답을 HTTP 테스트로 검증했다.
  `SystemNet`은 DNS 검증 IP에 고정 연결하고 TLS hostname을 검증한다. proxy와 자동 redirect를 끄고
  response를 제한된 chunk로 읽는다. loopback 실제 HTTP 회귀를 포함해 `direct_cover_url` 9개가 통과했다.
- 사용자 승인으로 `rusqlite 0.40.2`의 `bundled` 기능을 추가했다. 기존 SQLite 스키마를 유지한다.
  Python 생성 DB의 Rust 읽기, Rust 갱신 DB의 Python 읽기, 파일 시각·크기 기반 무효화를 검증했다.
- 표지 판정·미표지 조회·도서/만화 `/browse`를 연결했다. 실제 첫 이미지(XObject/inline), EPUB 표지,
  시간·페이지·동시 작업 제한, 숨긴 카테고리와 인증을 포함해 `cover_status` 테스트 22개가 통과했다.
- `AppDeps::from_env`가 ES 연결·index 생성과 `LoaderFileReader`를 포함한 실제 BookManager/ComicsManager를 lazy 조립한다.
  `BookManager::from_env_with_es`와 `ComicsManager::from_env_with_es`로 이미 연결한 ES를 주입한다.
  fake ES 조립 테스트 2개, manager 테스트 157개(12 ignored), server CLI 3개와 Backend Clippy가 통과했다.
- Google verifier는 RS256 signature, audience, issuer, expiration을 확인하고 JWKS를 Google `cache-control` 기간 동안 cache한다.
  key ID 교체 시 60초당 한 번 갱신하며 credential은 64 KiB로 제한한다.
  cache 지시어 단위 테스트 1개, 정적 서명 fixture 3개와 기존 auth 테스트 96개가 통과했다.
- bookstore 기본 세션을 `ureq`로 연결했다. 검색 query, HTTP status, headers와 response body를 보존한다.
  loopback HTTP 테스트 1개, bookstore 105개(1 ignored), book classifier 40개와 manager classifier 6개가 통과했다.
- cover_sources의 기본 세션도 실제 HTTP GET을 사용한다. 자동 redirect를 끄고 각 target을 재검증한다.
  loopback fetch·redirect 검증 포함 26개 테스트가 통과했다.
- direct cover URL fetch의 기존 `SystemNet` stub을 제거했다. ureq custom resolver에 사전 검증한 IP만 전달하고,
  TLS는 URL hostname으로 인증한다. loopback 연결 및 chunk response 검증 9개와 Backend Clippy가 통과했다.
- `classify_books_client`의 직접 실행 모드를 Rust `BookClassifierService`에 연결했다.
  library root/category, 재귀 탐색, limit/offset, uncached 필터, 재평가, dry-run과 파일 이동·중복 정리·cache 저장을 처리한다.
  같은 카테고리 파일을 중복 삭제할 수 있던 `process_file` 경계도 TDD로 수정했다.
  임시 library/API 테스트 13개, book classifier 41개와 utility Clippy가 통과했다.
- manager 운영 조립, Google verifier, bookstore session 변경 후
  `cargo test --workspace --exclude pdfscan --exclude epubscan --locked` 전체가 통과했다.
- 이전 기록의 일반 관리 API 미등록 테스트 및 loader 재색인 연결 항목은 현재 상태와 맞지 않아 해소 처리했다.
  관리 API target은 `main_api`, `main_gaps`, `main_mock`, `book_admin_api`로 등록되어 있고 loader는 Rust 실행 경로를 사용한다.
  남은 일은 외부 서비스 의존 테스트와 전체 기능 동등성 전수 확인이다.
- Rust classifier 학습은 작은 corpus 테스트만 검증했다. 실제 운영 corpus에서 sklearn parity와
  메모리·학습 시간 비교가 남아 있다. Python source는 기준 구현과 테스트 입력으로 유지한다.
- `class_weight="balanced"`를 One-vs-Rest 이진 label의 양성·음성 빈도로 계산하도록 맞췄다.
  기존 구현은 원래 다중 클래스 빈도를 써서 3개 이상 카테고리에서 sklearn과 달랐다.
  3-class 불균형 가중치 테스트와 classifier training 2개가 통과했다.
- Python과 Rust classifier 학습 테스트를 대조해 spill 학습의 입력 문서 보존 차이를 발견했다.
  Rust도 holdout 문서를 유지하고 학습 문서 원문을 block 처리 중 단계적으로 줄이도록 바꿨다.
  Python spill parity 테스트 3개를 활성화하고 classifier target 64개가 통과했다.
- Rust spill이 block 파일을 곧바로 다시 메모리로 읽던 동작을 고쳤다. block을 순차 병합해 disk-backed
  row matrix를 만들고 학습 시 행 단위로 읽으며, matrix가 drop되면 임시 파일을 지운다.
  matrix equality·파일 수명 테스트, classifier 64개, classifier_training 2개와 Backend Clippy가 통과했다.
- One-vs-Rest category 학습 결과를 전체 category 수만큼 누적하던 버퍼를 bounded channel로 바꿨다.
  반환된 weight를 즉시 모델 계수로 복사해 동시 `n_jobs` 개만 유지한다.
  Serial/parallel 점수 parity를 포함한 classifier_training 5개와 test_runner 묶음, Clippy가 통과했다.
- classify-proposal 상태/항목 조회와 항목·완료 항목 삭제 route를 연결했다.
  이번 묶음에서 제안 생성·적용 route, 작업 중복 방지, 허용된 proposal 경로 제한,
  항목별 상태 저장과 진행 상태 갱신을 추가했다. 요청 content type과 delay도 전달한다.
  `main_gaps` 18개, `book_manager_2` 154개, `book_manager_gaps` 46개,
  `book_manager_classifier` 6개 통과(각 묶음의 무시 테스트 제외), Backend Clippy 통과.
- `main.rs` API 테스트 초안을 현재 facade로 정렬해 등록했다. 48개가 통과하고 37개는 ES/MySQL 또는
  아직 연결되지 않은 테스트 seam 때문에 무시 상태다. `main_mock.rs`는 별도 HTTP mock target으로 등록돼 있다.
- PDF 표지 썸네일을 Poppler 렌더링으로 바꿔 텍스트·벡터·이미지 페이지를 처리한다.
  Python 규칙대로 tall page 상단과 펼침면 왼쪽을 자르고, 빈 첫 쪽이면 다음 쪽을 사용한다.
  `book_manager_gaps` PDF 벡터 테스트와 `cover` 23개가 통과했다.
  Poppler 실행 파일은 `poppler-utils` 패키지로 제공한다.
- 다운로드 응답을 Python Starlette `FileResponse`와 대조해 파일 검증 헤더와 byte range를 연결했다.
  single/suffix/multiple range, malformed·unsatisfiable range, `If-Range` 동작을 검증했다.
  일반·single range와 multipart range body를 64 KiB chunk로 스트리밍한다.
  `book_preview` 104개, Backend Clippy와 fmt 검사가 통과했다.
- 기존 facade의 `get_book_preview`를 유지하고 pages 인자를 받는 `get_book_preview_with_pages`를 추가했다.
- epubcheck 테스트는 실제 구현 경계인 CommandRunner로 옮겼다. 60초 전달을 확인하며,
  프로세스 종료는 workspace의 sysdeps `run_kills_a_command_that_exceeds_the_timeout` 테스트가 검증한다.
- GitNexus CLI 인덱스를 갱신했다. FTS 구축은 Invalid UTF8로 실패했다.
  MCP는 오래된 인덱스와 잘못 연결된 심볼 ID를 보고하므로 실제 소스와 회귀 검증을 병행한다.
  이번 변경의 재색인 후에도 Rust 파일에 frontend/Python 심볼명이 연결된다.
  `detect_changes`의 전체 diff HIGH·선별 diff MEDIUM 결과를 안전성 판정 근거로 쓰지 않았다.
- 이 세션은 Python 원본·배포 설정·스캐너 작업을 수정하지 않았다.

## 승인되어 적용한 의존성 변경

- `axum 0.8.9`의 `multipart`: 이미지 업로드 라우트와 크기 제한·손상 이미지 HTTP 테스트.
- workspace `ureq 3.5.0`의 `rustls`: 실제 HTTPS·Google 공개 키 조회·URL 이미지 fetch 연결.
- 기존 `doc_reader`·`loader` path dependency: 실제 파일 읽기, 재색인, EPUB 행수 및 PDF 편집 초안 테스트 등록.
- `stat` path dependency: loader 기록 API에서 통계 누적 타입을 직접 사용한다.
- `fancy-regex 0.14.0`: Python의 lookaround/backreference를 포함하는 파일명 정규식 치환.
- `sysdeps` Poppler adapter: `pdftotext`, `pdfinfo`, `pdftoppm`을 PATH에서 찾아 실행한다.

미등록 Backend 테스트 초안은 `classify_cli`, `classify_cli_gaps` 두 파일이다. `api_exploration`과 `main_mock`은
`backend/Cargo.toml`에 등록되어 있다. 두 초안은 과거 API 가정이 현재 Rust 계약과 맞지 않아 그대로 등록하지 않았다.
`classify_cli` 학습·추론·일반 export와 sklearn `joblib` 변환은 Rust 경로로 전환했다.
Python `backend/classifier/export.py`는 기준 구현과 fixture 생성용으로 남아 있다.
- `category-mismatches`의 요약 조회, 단일 파일 색인·삭제, ES 문서 삭제 route를 Rust에 연결했다.
  `BookManagerApi`에 파일 작업을 노출하고 실제 `BookManager` adapter를 연결했다.
  `main_gaps` 19개, Backend Clippy, scanner 제외 workspace 전체 테스트가 통과했다.
  입력 누락(400)과 ES 삭제 거부 응답도 확인했다. 카테고리별 재적재·상태·상세 route는 다음 묶음이다.
- 같은 불일치 API의 상세 조회와 동기 카테고리 전체 재적재를 추가했다. `/comics` 경로는 ComicsManager와 `comic` 유형을 사용한다.
  `main_gaps` 20개, Backend Clippy, scanner 제외 workspace 전체 테스트가 통과했다.
  백그라운드 불일치 재적재의 lock 획득·상태 조회·완료 기록 route는 다음 구현 묶음으로 남아 있다.
- 재적재 상태 조회도 연결했다. `CategoryMappingApi`가 기존 reload lock 조회 결과를 JSON으로 전달하며,
  이력이 없으면 `idle`을 반환한다. `main_gaps` 20개와 Backend Clippy가 통과했다.
  불일치 항목 background reload 시작·진행 heartbeat·완료 기록은 아직 남아 있다.
- `category-mismatches/reload-mismatches`와 `reload-all` background API를 Rust에 연결했다.
  기존 DB reload lock으로 중복 작업을 막고, manager progress callback과 주기 heartbeat를 저장하며,
  완료 시 indexed/deleted/failed/before/after 카운트를 기록한다. `/comics`는 `comic` lock을 사용한다.
  `main_gaps` 21개, Backend Clippy, fmt 검사가 통과했고 scanner 제외 workspace 전체 테스트도 통과했다.
  카테고리 mismatch API 이관은 완료했다. `main.rs` 테스트는 facade 정렬 뒤 등록했다.
  `main_mock.rs`의 이전 내부 `ItemRouter` 직접 호출 draft 13개는 비활성 상태이며,
  최종 API parity 감사에서 공개 HTTP facade 대응 근거를 확인해야 한다.
- category mismatch API의 background 재적재를 완성했다. category/bulk lock을 획득하고,
  중복 요청을 현재 상태에 연결하며, 진행 callback·주기 heartbeat·완료 카운트를 저장한다.
  동일 category 재요청, 상태 완료, 카운트 보존, bulk 무본문 요청을 검증했다.
- `/wake`를 Rust에 추가했다. 설정된 frontend host만 허용하고 `FsOps`로 book root를 읽는다.
  storage 오류는 Python과 같은 503 `{"status":"failure"}` 응답을 반환한다.
- 최신 scanner 제외 workspace 전체 테스트, `main_gaps` 22개, Backend Clippy,
  formatting과 diff 검사가 통과했다.
- Python 원본과 Rust `CategoryMapping`을 대조했다. CRUD·reload lock·proposal 상태와 schema migration 경로가 대응한다.
  `category_mapping` Rust 개별 테스트 77개와 Python `tests/test_category_mapping.py` 52개가 통과했다.
- 표지 서점 검색도 Python/Rust API 표면과 취소·요청 제한 경로를 대조했다. 별도 `check_cancelled` 함수는
  Rust 취소 callback으로 처리한다. `cover_sources` Rust 27개와 Python 48개 개별 테스트가 통과했다.
- `/search/bookstore/{store_name}`을 Rust BookstoreSearch facade와 AppDeps factory에 연결했다.
  13개 서점 이름, 미지원 서점 404, 검색어 누락 400, 입력 trim, 결과 최대 4건,
  ISBN 포함, 결과 없음 응답과 검색 URL을 검증했다. `main_gaps` 24개와 scanner 제외 workspace 전체 테스트,
  Backend Clippy 및 formatting 검사가 통과했다.
- `/wake`도 frontend host 검사와 `FsOps` storage 접근을 사용하도록 이관했다.
  접근 거부 404, 성공 응답, storage 오류의 503 JSON을 확인했다.
- bookstore API의 공백 입력 검증도 Python 순서와 맞췄다. 원문 입력이 모두 빈 경우만 400이며,
  공백만 있는 검색어는 trim 뒤 빈 결과로 처리한다. 관련 route 회귀 24개가 통과했다.
- `main.rs` 테스트 초안을 현재 API facade와 manager signature에 맞춰 등록했다. 기존 동작 assertion을 보존하고,
  HTTP background reload 검사는 상태 완료를 polling하도록 바꿨다. bookstore API 테스트 3개를 무시 목록에서 활성화했다.
- 회귀에서 `reload-all`의 `reload_source`가 Python의 `Literal["bulk", "mismatch"]`와 달리 검증되지 않고,
  progress flusher가 주입된 주기를 무시하는 점을 수정했다. background 작업 panic도 `failed` lock과 generic 오류로 마감한다.
  `/wake` 테스트에는 허용 frontend Host를 전달했다.
- `main_api` 48개 통과, 37개 무시. `cargo fmt --all -- --check`, Backend Clippy,
  scanner 제외 workspace 전체 테스트가 통과했다.
- EPUB line-count 테스트를 등록하고 `EpubHooks` 주입과 ZIP fallback 직접 실행 API를 추가했다.
  논리 행수, 장 전체 누적, 일반 읽기 실패 시 fallback 동작을 4개 테스트로 검증했다.
  `doc_reader` 39개, `loader` 52개, scanner 제외 workspace 테스트, Backend Clippy와 fmt 검사가 통과했다.
- loader 후반부 테스트 초안 `loader_2`를 등록했다. PDF xref helper 공개, HWP 변환 주입, PDF plumber 추출 경계,
  TXT 후보·replace hook과 mojibake 패턴 helper를 추가했다. 활성 `loader_2` 65개와 기존 loader/doc_reader 테스트가 통과했다.
  `RunHooks` type은 추가했지만 현재 `run_with_hooks`가 실제 처리 경로에 연결되지 않았다.
  `loader_2`의 mock 기반 테스트 39개는 여전히 ignore 상태이며, 다음 묶음에서 hook을 실제 흐름에 연결하고 활성화해야 한다.
  library Clippy와 diff 검사는 통과했다. all-target Clippy는 등록된 draft test 파일의 기존 unused import·복잡 타입 lint로 실패했다.
- `RunHooks`를 loader batch 흐름에 연결해 file read, stat, worker 생성 관찰을 주입 가능하게 했다.
  관련 테스트 3개를 활성화해 `loader_2` 68개 통과, 36개 무시를 확인했다.
  `cargo clippy -p loader --lib --offline -- -D warnings`와 `git diff --check`가 통과했다.
  등록 테스트 전체 Clippy는 초안 파일 자체의 lint 오류로 아직 통과하지 않는다.
- loader 후반부 seam을 이어서 활성화했다. TXT 인코딩 후보·replace 오류, mojibake 패턴,
  PDF plumber/page-count/xref hooks, stat·worker hooks 테스트를 포함해 `loader_2` 80개 통과, 24개 무시다.
  loader all-target Clippy와 scanner 제외 workspace 전체 테스트가 통과했다.
- workspace 회귀에서 드러난 EPUB ZIP fallback의 문서 전체 text 누락과 HWP 빈 변환 오류 기록 차이를 수정했다.
  EPUB fallback은 BeautifulSoup `get_text()`처럼 `head`를 포함하며, 일반 EPUB reader는 기존 pretty body 추출을 유지한다.
  HWP 일반 경로는 LibreOffice 오류를 보존하면서 빈 출력이면 HWP3 fallback을 수행한다.
- `main_mock`을 현재 `AppDeps`, manager facade, refresh store, bookstore factory API에 맞췄다.
  테스트 실행기는 background job을 별도 스레드에서 마치고 응답 전 join해 순차 검증의 race를 없앴다.
  Google login의 refresh store 주입, `/wake` Host 헤더, missing `book_id` 채우기를 확인했다.
  `main_mock` 114개, Backend Clippy, fmt와 diff 검사가 통과했다.
  오래된 내부 `ItemRouter`를 직접 호출하는 draft 테스트 13개는 현재 HTTP facade로 옮기지 못해 비활성 상태다.
  GET classify proposal 응답의 missing `book_id`도 inode로 채우며, 파일이 없으면 Python처럼 null을 넣는다.
  `main_gaps` 24개와 scanner 제외 workspace 전체 순차 테스트가 통과했다.
- Python API 탐색 테스트의 Rust 대응 파일 `api_exploration`을 Backend test target으로 등록했다.
  빈 입력, 명사 필터·빈도순 안정성, 요청 실패 단계별 종료와 전체 탐색 흐름 9개가 통과했다.
- Docker 운영 이미지를 Python/Uvicorn에서 Rust `backend`와 `loader` 실행 파일로 전환했다.
  PDFium, LibreOffice, Poppler, Java, EPUBCheck 런타임 의존성을 포함하고 비루트 사용자로 실행한다.
  로컬 startup script, README, scanner 제외 Rust CI workflow도 Rust 실행 경로에 맞췄다.
  release Docker build, 두 CLI 도움말, 동적 라이브러리와 EPUBCheck 확인, 실제 컨테이너 `/wake` 200 응답이 통과했다.
  로컬 startup은 backend와 loader를 `--locked`로 함께 빌드하고 loader 실행 경로를 지정한다.
  loader 후반부의 기존 ignored 테스트는 활성화했다. 남은 작업은 Python/Rust 기능 동등성 전수 확인과
  외부 서비스에 묶인 테스트의 환경별 검증이다.
- `tests/books/_hwp` fixture가 저장소에 있음을 확인하고 `loader_2`의 HWP·fallback 테스트 19개와
  import guard 대응 테스트 1개를 활성화했다. `loader_2` 104개 전체와 loader 전체 검사,
  loader all-target Clippy가 통과했다. 이어 `loader_hwp3` fixture 테스트 10개도 활성화해
  61개 전체가 통과했다. 최종 scanner 제외 workspace 순차 테스트는 2,755 passed, 0 failed,
  124 ignored다. 남은 ignore는 외부 서비스 의존 등 별도 사유로 유지한다.
- classify-proposal의 Python batching 동작을 `backend/main.py`와 대조했다. 진행 이벤트 항목 수가
  10개 이상이면 해당 이벤트의 전체 항목을 한 번에 저장하고, 잔여분은 작업 종료 때 flush한다.
  Rust가 큰 이벤트를 10개 묶음으로 쪼개던 차이를 RED 테스트로 재현한 뒤 Python과 같게 고쳤다.
  관련 HTTP 테스트 36개, Backend Clippy, fmt, diff 검사가 통과했다.
  10번째 항목 시점의 중간 flush, manager 오류 때 잔여분 보존,
  적용 callback의 허용 경로 필터와 제안 진행률 보존도 HTTP facade에서 검증했다.
  전체 workspace 회귀는 마지막 전수 검증까지 보류한다.
- `normalize_comic_directories.py`의 파일 분석, 서점 결과 선택·캐시, 충돌 판정, TSV 및 rename rollback을
  대응 Rust 구현과 다시 대조했다. 개별 테스트 37개, crate Clippy, fmt와 diff 검사가 통과했다.
  frontend와 두 scanner는 이 점검에서 실행하거나 수정하지 않았다.
- `move_semi_clusters.py`의 cluster별 이동, 그리스 로마 신화 예외, 중복 제거와 빈 폴더 청소를
  Rust 경로와 대조했다. 개별 테스트 12개와 crate Clippy가 통과했고 추가 동작 차이는 찾지 못했다.
- `utils/update.py`의 ES 갱신→단건 조회→삭제 순서와 실패 종료 코드를 mock ES로 검증했다.
  개별 테스트 10개와 crate Clippy가 통과했다. backend/utils Python 파일 인벤토리를 다시 확인했으며,
  launcher가 Python 애플리케이션을 실행하는 경로는 찾지 못했다. 기능별 동등성 전수 검증은 남아 있다.
- `classify_books_client.py`의 REST와 direct mode, 옵션, 캐시 필터, dry-run 및 파일 처리를 대조했다.
  Rust는 Python과 같은 REST endpoint/payload, Bearer token, 상태 polling을 지원한다.
  Rust가 REST 상태 HTTP 오류를 30회 뒤 중단하던 차이를 찾아 Python처럼 완료까지 재시도하게 바꿨다.
  31회 연속 503 뒤 완료 응답을 받는 테스트를 RED/GREEN으로 확인했고, client 11개·bin 3개 테스트가 통과했다.
- `classify_cli.py`의 CLI 경로를 재확인했다. collect, Rust training/export, evaluate/calibration,
  bookstore policy, classify/reclassify/info 테스트 23개와 Clippy가 통과했다.
  해당 Rust crate에는 Python subprocess 실행이 없다.
- classify client REST mode의 `/categories/auto-classify*` 경로는 Python/Rust backend 양쪽에 없다.
  client 모드는 기존 계약을 유지해 외부 호환 server를 호출하며, mock HTTP 테스트로 동작을 확인한다.
- `backend/es_manager.py`의 loader용 index create/mapping 보강, bulk retry/chunking, mget, scroll,
  delete-by-path/id와 refresh 동작을 Rust `es_client`와 대조했다. 관련 테스트 38개와 crate Clippy가 통과했다.
- `backend/refresh_token_store.py`의 MySQL 운영 경로를 Rust와 대조했다. 회전 grace, 재사용 감지,
  family revoke, 클라이언트 정보 절단, 세션 집계·페이지 처리, DB rollback 테스트를 확인했다.
  당시 MySQL 대조 테스트 59개가 통과했고 5개는 SQLite 미지원/Docker 필요로 ignore 상태였다.
  SQLite 기능은 후속 묶음에서 구현해 아래 테스트 결과로 검증했다.
- `backend/auth.py`의 JWT 발급·검증, role 재검증, 쿠키 인증, refresh/logout과 Google 인증 테스트를 확인했다.
  `auth` 개별 테스트 96개가 통과했다. Rust는 빈 이메일을 admin으로 인정하지 않는다.
  Python의 빈 `TM_ADMIN_EMAIL` 비교가 만드는 승격 경로를 차단한 보안 차이다.
- `backend/view_history_store.py`의 CRUD/정렬/상한 로직을 대조했다. 개별 테스트는 2개 통과했고,
  MySQL 컨테이너가 필요한 통합 테스트 13개는 ignore 상태다. 기능 단언 대부분이 실DB fixture에 묶여 있어
  격리 실행만으로는 아직 폭넓은 동작 증거가 부족하다.
- `backend/image_edit.py`의 좌표 검증, Python 짝수 반올림, 최소 크기, 회전 crop과 1-bit 변환을
  Rust 구현과 대조했다. 이미지 크롭 개별 테스트 4개가 통과했다.
- `backend/cover_status.py`의 PDF 첫 이미지 판정, EPUB metadata 판정, SQLite cache 무효화와
  Python/Rust DB 호환성을 대조했다. cover status/browse 개별 테스트 22개가 통과했다.
- `backend/cover_browse.py`의 scope/group 규칙, sort cursor, 표지 대표본 선택과 page merge를
  Rust 단위 테스트와 대조했다. 개별 테스트 28개가 통과했다.
- EPUB cover repair의 손상 판정, 원본 body/backup 보존, header-only inspection 테스트 5개가 통과했다.
- `backend/bookstore_cache.py`의 JSONL 복구, ISBN 우선 검색, 식별자 검증, append 실패 허용을
  Rust와 대조했다. 개별 테스트 19개가 통과했다. Rust는 cache 파일/줄 크기와 symlink도 제한한다.
- `book`·`comics` 모델과 manager 초기화 계약을 Python 원본에 맞춰 확인했다. 관련 개별 테스트는 각각 6개 통과했다.
- 공용 Python 유틸리티 Rust 대응을 확인했다. `stat` 11개, `file_time` 10개, `isbn` 35개,
  `parser_timeout` 12개, `corpus_layout` 5개, `hwp3_tables` 4개, `hwp3_parser` 24개가 각각 통과했다.
  workspace 회귀는 실행하지 않았다.
- 별도 유틸리티 CLI를 Python과 대조하고 개별 테스트를 실행했다. `quarantine_broken` 51개,
  `detect_mojibake` 35개, `search` 12개, `cluster_hybrid_genres` 35개가 통과했다.
- HTML sanitizer 7개와 EPUB cover edit/API 70개도 각각 통과했다.
- `corpus_common` 29개, `pyfmt` 5개, `sysdeps` 27개 개별 테스트가 통과했다.
- `htmlkit` 16개, EPUB cover set/edit/API 70개 개별 테스트가 통과했다.
- `backend/cover_management.py`의 목록·검색·캐시·취소·표지 교체·복구·미리보기와 HTTP 인증/입력을 대조했다.
  cover management 개별 테스트 69개가 통과했다.
- view history HTTP API의 관리자 인증, 입력 제한, 서버 측 snapshot, 실패 시 viewing 허용 동작을 확인했다.
  `view_history_api` 17개가 통과했고 2개는 MySQL 컨테이너 필요로 ignore 상태다.
- 실행 경로 전수 검색에서 Python 원본 `backend/book_manager.py`가 `sys.executable`로 loader를 부르는 부분을 재확인했다.
  배포 Rust 구현은 `BookManager::reload_category`의 `CommandRunner`로 `loader` 바이너리를 실행한다.
  startup은 backend/loader를 함께 빌드하고 `TM_LOADER_BIN`을 설정하며 Docker도 두 바이너리를 복사한다.
  reload 관련 개별 테스트 7개가 통과했다. 운영 경로의 Python 애플리케이션 호출은 확인되지 않았다.
- `doc_reader::pdf::pdf_text_by_pdftotext_with`의 미검증 테스트 두 개를 RED로 실행해 실패를 확인했다.
  Python `Loader._pdf_text_by_pdftotext`와 비교해 Rust가 `-enc UTF-8`/`-l 40` 인자를 전달하지 않고,
  executable 탐색을 매 호출 반복하는 차이를 수정했다. `PdftotextHooks`에 scoped `OnceLock` cache를 넣고
  전체 argv를 테스트 대역에 전달하도록 바꿨다. 해당 두 테스트를 활성화했다.
  `doc_reader` 39개 통과(외부 corpus 5개 ignore), `loader_1` 174개 통과·1개 ignore,
  `loader_2` 104개 통과다. loader all-target Clippy, workspace fmt, diff 검사가 통과했다.
- Docker가 준비되어 있어 앞서 ignore였던 view history store/API 통합 34개를 개별 실행했다.
  각각 15개와 19개가 통과했다. refresh token의 MySQL aggregate·legacy migration Docker 테스트 3개도 통과했다.
- Python `backend/classifier/export.py`를 기준으로 Rust exporter와 CLI 경로를 확인했다.
  `export_format::write_dir`는 같은 `model.json`·vocab·idf·coef 형식을 쓰고 Rust loader가 읽는다.
  Python `write_golden`은 sklearn 기대값을 만드는 fixture 도구다. 동등한 golden 생성 기능을 Rust CLI로 옮겼다.
  sklearn golden parity와 Rust train/export CLI 테스트를 각 1개씩 실행해 통과했다.
  오래된 미등록 `classify_cli_gaps` draft는 현재 crate 의존·API와 맞지 않아 등록하지 않았다.
- `backend/refresh_token_store.py`를 대조해 Rust의 SQLite 미지원이 기본 backend 동작 차이임을 확인했다.
  Rust에도 SQLite schema/migration, token issue/rotation/reuse/revoke, observation과 session aggregate를 구현하고
  factory 기본값을 Python과 같은 SQLite로 맞췄다. Kubernetes는 이미 `TM_REFRESH_TOKEN_BACKEND=mysql`을 명시한다.
  SQLite factory·legacy migration·회전/재사용 탐지·세션 통합 테스트를 활성화했다. 저장소 테스트 63개 통과, Docker 필요 3개 ignore;
  auth 96개 통과, Backend all-target Clippy, fmt와 diff 검사가 통과했다.
  기존 공통 저장소 동작 테스트도 임시 SQLite 파일을 직접 사용하도록 바꿔 메모리 MySQL 대역에만 의존하지 않는다.
- Kubernetes의 분류기 설정이 여전히 `TM_CLASSIFIER_MODEL=/books/.classifier/model.joblib`을 사용해
  Rust runtime에서 실제 모델을 읽지 못할 경로임을 확인했다. Rust 기본 모델 경로도 `model.joblib` 파일을 가리켜
  문서의 export 디렉터리 기본값과 달랐다. TDD에서 기본 경로 테스트가 RED로 실패한 뒤
  `backend/classifier/export`로 맞추고, 배포 env를 `TM_CLASSIFIER_MODEL_DIR=/books/.classifier/rust-export`로 바꿨다.
  PVC의 306 MB joblib을 잠금 sklearn 1.9.1로 export해 `/mnt/data/text/.classifier/rust-export`를 만들었다.
  79개 클래스·952,717개 feature·golden 266건을 생성했다. 초기에 global `PYTHONPATH`가 sklearn 1.8.0을
  가리킨 사실을 발견해 이를 제거하고 잠금 환경에서 다시 export했다. 실제 모델 parity 266건은 최대 score 오차
  1.634e-7, confidence 오차 1.045e-8, rank/reason/category 불일치 0건이다.
  Rust `classify_cli info`가 `TM_CLASSIFIER_MODEL_DIR` 경로에서 모델을 읽었다. `classifier` 64개,
  `classifier_model` 24개 통과·2개 무시, Backend/CLI Clippy, fmt/diff와 `kubectl apply --dry-run=client`가 통과했다.
  kubectl 설정을 실제 클러스터에 적용하지 않았다.
- backend와 utils의 Python 모듈 목록을 Rust workspace 파일과 대조했다. classifier의 `export.py`는
  `classifier/export_format.rs`와 `classify_cli import-joblib`가 포맷 쓰기·읽기와 joblib 변환을 맡는다.
  소스 검색에서 앱을 실행하는 Python launcher 호출은 찾지 못했다. `classify_books_client --api-url`은
  과거 `/categories/auto-classify*` 경로를 호출하지만 해당 API는 migration spec에서 제거됐다.
  이는 Rust client에도 Python과 같은 호환 모드로 남아 있으며, 현재 backend 기능으로 계산하지 않는다.
  `isbn` 개별 테스트 35개, `book` 6개, `comics` 6개가 통과했다. 전체 회귀는 실행하지 않았다.
- 기존 `tests/test_classify_cli_gaps.py`의 500개 재분류 진행 로그 검증을 현재 `classify_cli` target으로 옮겼다.
  임시 파일 500개로 `500/500 처리` 출력을 확인했고 개별 테스트 1개가 통과했다.
  미등록 `classify_cli_gaps.rs` draft는 과거 `ClassifyCliDeps` API를 가정해 현재 `Deps` 계약과 맞지 않으므로 등록하지 않았다.
- Python `cmd_collect`의 12개 이상 카테고리 하위 목록 출력 검증도 현재 ES transport fake로 옮겼다.
  `cmd_train --save-config`에서는 저장 경로 미출력을 RED로 재현한 뒤 `설정 저장:` 출력을 구현했다.
  설정 파일 생성과 출력 경로를 함께 확인했다. boundary 표본 선택, 빈 구간 reband, 잘못된 manifest 형태도
  Python gap 사례에서 추가해 `classify_cli` 개별 target 27개가 통과했다.
- `tests/test_main.py`의 Python stale heartbeat 사례를 Rust `category_mapping` 동작과 대조했다.
  `running`/`applying` 만료, 최근 heartbeat, terminal 상태, 진행 수치 보존은 이미 구현돼 있었다.
  다만 stale 상태에 기존 오류가 있으면 Python은 안내 오류로 교체하지만 Rust는 기존 오류를 유지했다.
  해당 조건을 개별 테스트에서 RED로 재현하고 Python과 같은 메시지로 교체하도록 수정했다.
  `category_mapping` 55개와 해당 test target Clippy가 통과했다.
- `backend/main.py`는 `TM_FRONTEND_URL`이 없으면 앱 import 때 종료하지만 Rust 서버 진입점에는 검사가 없었다.
  Python의 환경 설정 guard를 기준으로 서버 시작 검증을 추가하고, 누락/설정 상태를 다루는 진입점 단위 테스트를 작성했다.
  `cargo test -p backend --bin backend --offline --locked -- --test-threads=1`에서 1개 통과.
  ComicsManager factory는 Python의 단순 생성자 wrapper 대신 Rust `build_comics_manager` 조립 테스트가 경로·ES 연결을 검증한다.
  CategoryMapping factory도 `AppDeps`의 lazy slot이 실제 생성자를 직접 보유해 별도 wrapper 테스트 대상은 아니다.
  대상 binary Clippy, 전체 fmt 검사와 `git diff --check`가 통과했다.
- `book_manager_2`의 ignored EPUB 빈 경로 초안은 Python 전역 `normpath` 교체 seam을 요구해 그대로는 Rust에 맞지 않았다.
  대신 Python의 실제 empty manifest href 입력으로 바꾸고 EPUB 유효성 거부 단언을 유지해 ignore를 제거했다.
  `cargo test -p backend --test book_manager_2 --offline --locked -- --test-threads=1`에서 155개 통과했다.
- loader의 ignored PDF handle 검사는 Python `mock.patch` 타임아웃을 재현하지 않고, 실제 Rust parser 실패 뒤 fd 누수로 단언을 구체화했다.
  `loader_1` 개별 target 175개와 해당 target Clippy가 통과했다. 기존 `loader_2`도 개별 target 104개를 재실행해 통과했다.
- `utils/update.py`는 인자 없이 기본 문서 ID 3384를 갱신·조회·삭제하지만 Rust는 `run`을 강제했다.
  mock ES 테스트에서 no-argument 동작을 RED로 확인한 뒤 `run`을 선택 명령으로 바꿨다.
  바이너리도 no-argument 경로로 갱신→조회→삭제 요청을 검증했다. `cargo test -p update --offline --locked`에서 11개 통과했고,
  update all-target Clippy와 workspace fmt/diff 검사가 통과했다.
- `utils/move_semi_clusters.py`도 무인자 실행으로 기본 라이브러리의 실제 이동을 수행한다.
  Rust가 사용법만 보이던 차이를 임시 라이브러리 테스트로 RED 확인 후 기본 실행을 Python과 맞췄다.
  기본 경로를 주입하는 실행 함수를 두어 같은 로직을 테스트했고, `move_semi_clusters` 13개가 통과했다.
- `normalize_comic_directories.py`는 인자 없이 기본 만화 디렉토리를 스캔하고 리포트를 생성한다.
  Rust의 무인자 도움말 동작을 RED로 확인한 뒤 기본 옵션 실행으로 바꾸고, 테스트에서는 임시 루트와 offline cache를 주입했다.
  `normalize_comic_directories` 38개, all-target Clippy, workspace fmt와 diff 검사가 통과했다.
- `classify_books_client.py`도 인자 없이 기본 라이브러리를 직접 분류한다. Rust의 무인자 도움말 동작을
  임시 라이브러리 주입 테스트에서 RED로 확인한 뒤 기본 직접 실행으로 맞췄다. 바이너리 테스트는 실제
  기본 경로를 건드리지 않고 `--help`를 확인한다. `classify_books_client` 14개와 all-target Clippy,
  workspace fmt 및 diff 검사가 통과했다.
- `loader.py`는 인자가 없으면 사용법과 종료 코드 1을 반환하지만 Rust는 0을 반환했다. 기존 두 테스트를
  Python 기준으로 수정하고 loader_2 개별 104개가 통과했다. `classify_cli.py`도 subcommand 없이 실행하면
  argparse가 코드 1을 반환한다. Rust library와 binary를 맞추고 관련 `cli` 테스트 27개가 통과했다.
  두 crate all-target Clippy가 통과했다.
- workspace 문서의 “모든 무인자 CLI가 도움말과 코드 0을 반환한다”는 규칙이 Python CLI들과 모순되어,
  각 Python 진입점의 실제 기본 동작을 따르도록 고쳤다. `classify_cli` 설명도 코드 1로 바로잡았다.
- backend `main_api`의 Google 로그인 성공·refresh 접근 거부 테스트 두 개는 저장소 대역 오류와 환경 초기화 순서 때문에
  ignore 상태였다. 실제 Python route 순서와 `RefreshTokenStoreApi` 계약에 맞게 대역·초기화를 고치고 ignore를 제거했다.
  TDD에서 Google 성공 경로가 500으로 실패하는 것을 확인한 뒤 수정했다. `main_api` 50개 통과·35개 외부 ES/MySQL ignore;
  target Clippy, workspace fmt와 diff 검사가 통과했다.
- Python FastAPI 앱·관리자 router 경로를 Rust `Routes` 등록과 대조했다. Python의 book/comics router prefix를 펼친
  118개 method/path가 Rust에 모두 대응한다. Rust의 추가 6개 GET 경로는 `rename`·`delete` 같은 reserved path guard다.
  `startup.sh`와 Docker runtime은 Rust `backend`·`loader`만 실행하며, Docker의 Python stage는 PDFium 공유 라이브러리를 가져온다.
  운영 Python launcher는 찾지 못했다. joblib 오프라인 변환은 아래 Rust CLI로 연결했다.
- Python `lxml`의 `recover=True`는 잘린 OPF에서도 메타데이터를 복구하지만 Rust strict parser는 빈 결과를 반환했다.
  닫히지 않은 문서와 끝의 부분 태그 사례에서 RED를 확인하고, DTD·알 수 없는 entity 제거 후 태그 구조를 복구하도록 맞췄다.
  Python lxml 결과와 비교해 이미 읽은 필드를 보존한다. `book_classifier` 개별 테스트 43개, Backend lib Clippy,
  workspace fmt와 diff 검사가 통과했다.
- Python classifier exporter는 manifest 저장 전에 `_`로 시작하는 `meta` 키를 제외하지만 Rust `write_dir`는 포함했다.
  `CategoryModel::save` 결과를 확인하는 테스트에서 RED를 재현하고, Rust도 private key만 제외하도록 맞췄다.
  `classifier_model` 25개 통과·2개 환경 의존 ignore, Backend lib Clippy, workspace fmt와 diff 검사가 통과했다.
- API·manager 묶음을 전체 회귀 없이 다시 확인했다. `main_mock` 120개, `main_gaps` 24개,
  `book_manager_gaps` 47개가 통과했고 색상 없는 이미지 대역 테스트 1개는 환경 제약으로 ignore다.
  `main_api`는 50개 통과·ES/MySQL 의존 35개 ignore다. 실제 배포 Rust model directory도
  `classify_cli info`로 읽어 79개 클래스·952,717개 feature와 holdout 통계를 확인했다.
- 실제 운영 classifier export를 Python golden과 대조하는 ignored 단일 parity 테스트를 명시적으로 실행했다.
  `classifier_model::real_model_matches_sklearn_golden`이 통과했다.
- classifier의 ignored Unicode analyzer 비교는 sklearn 기본 `token_pattern`을 사용해 프로젝트 Python 설정과 달랐다.
  Python `make_vectorizer`와 같은 `(?u)\b\w+\b`로 기대값을 다시 만들었다. 임시 2,952개 비교가 모두 일치했고,
  336개 고정 fixture를 추가해 기존 테스트를 활성화했다. `classifier_model` 26개 통과·실제 모델 parity 1개 ignore,
  target Clippy, workspace fmt와 diff 검사가 통과했다.
- Python ISBN 검증은 Unicode decimal digit을 `int()`로 읽지만 Rust는 ASCII만 받았고 검색 결과도 Arabic-Indic 숫자를 버렸다.
  ISBN-10/13 검증과 본문 추출의 Unicode 숫자 사례를 RED로 확인했다. Rust가 Unicode `DecimalNumber` 값을 해석하고
  검색 결과에 원문 숫자를 보존하도록 맞췄다. 기존 lockfile에 있는 `unicode-general-category`를 직접 사용한다.
  `isbn` 36개, all-target Clippy, workspace fmt와 diff 검사가 통과했다.
- Python `tests/test_isbn.py` 50개, Rust `isbn` 36개, Python/Rust `file_time` 각 10개,
  Python/Rust `parser_timeout` 각 12개를 개별 실행해 통과를 확인했다.
  추가 소스 대조에서 Rust timeout은 파서 thread를 남기지만 Python은 파싱 자식 process를 종료하는 차이를 찾았다.
  `LoaderFileReader`의 제한 시간 읽기를 backend worker process로 격리하고, 만료 시 강제 종료하도록 바꿨다.
  `loader_file_reader` 4개와 timeout 오류를 다루는 BookManager 개별 테스트가 통과했다.
- `utils/stat.py::Stat.set_value`는 알 수 없는 이름으로도 속성을 만들지만 Rust는 이름을 거부했다.
  Rust 테스트에 Python 동작을 먼저 추가해 RED를 확인하고, 동적 카운터 생성·조회·증가를 지원했다.
  Rust `stat` 12개와 Python `tests/test_stat.py` 5개, crate Clippy, workspace fmt 검사가 통과했다.
- HWP3 Python 테스트에 있던 재귀 깊이 제한 사례가 Rust 포트에서 빠져 있었다.
  `depth=11` 입력을 Rust `loader_hwp3` target에 옮겼다. 원본 `tests/test_loader.py` HWP 클래스 69개와 Rust target 62개가 통과했다.
  별도 `tests/test_hwp3_parser.py` 97개도 통과했으며, 이전 기록에서 두 Python 테스트 수를 혼동한 점을 바로잡았다.
- 같은 HWP 클래스의 존재하지 않는 파일 테스트도 Rust 포트에서 빠져 있었다.
  `test_nonexistent_file_returns_empty`를 추가했다. `loader_hwp3` 63개가 통과했다.
- Python classifier exporter는 vectorizer가 없으면 모델 파일을 쓰기 전에 실패하지만 Rust `write_dir`는 빈 모델을 썼다.
  Rust exporter 테스트에서 빈 vectorizer 모델을 RED로 재현하고 같은 오류를 반환하도록 수정했다.
  Python exporter 11개, `classifier_model` 27개 통과·실제 모델 1개 ignore, Backend lib Clippy가 통과했다.
- Python exporter는 float32 변환 뒤 weight·IDF·계수·intercept의 NaN/Inf를 거부하지만 Rust writer는 저장했다.
  네 입력을 새 Rust 테스트로 RED 재현하고 파일별 `NotFinite` 오류를 반환하도록 고쳤다.
  Python exporter 11개, `classifier_model` 28개 통과·실제 모델 1개 ignore, Backend lib Clippy, workspace fmt 검사가 통과했다.
- `write_dir`가 불연속 필드 offset을 넣어 자체 loader가 거부하는 manifest를 만들 수 있었다.
  잘못된 offset 입력을 RED로 재현하고 각 offset이 앞 필드 크기의 합과 맞는지 검사하도록 수정했다.
  Python exporter 11개, `classifier_model` 29개 통과·실제 모델 1개 ignore, Backend lib Clippy, workspace fmt 검사가 통과했다.
- Python exporter는 config의 f64 field weight `0.3`을 저장하지만 Rust는 vectorizer f32를 다시 변환해 `0.3000000119`를 기록했다.
  Python fixture를 저장하는 Rust 테스트에서 RED를 확인하고 config 원본 weight를 manifest에 보존했다.
  Python exporter 11개, `classifier_model` 29개 통과·실제 모델 1개 ignore, Backend lib Clippy가 통과했다.
- Python bookstore cache는 `cat` 값의 truthiness를 검사하지만 Rust는 비어 있지 않은 문자열만 인정했다.
  정수 카테고리의 캐시 miss를 Rust 개별 테스트에서 재현하고 JSON null·bool·number·string·array·object의 truthiness를 맞췄다.
  Python `test_bookstore_cache.py` 19개와 Rust `bookstore_cache` 20개, Backend lib Clippy, workspace fmt 및 diff 검사가 통과했다.
- 운영 `AppDeps::from_env`는 view history의 `BookAccess`를 미구현 기본값으로 남겨 조회 이력 POST가 항상 500을 냈다.
  Python처럼 book/comic manager에서 책을 읽고 viewer의 숨김 카테고리와 하위 경로를 거부하도록 연결했다.
  빈 카테고리 제외 규칙도 Python과 일치시켰다. Rust `view_history_api` 17개 통과·MySQL 2개 ignore,
  BookAccess 3개 단위 테스트와 Python `test_view_history_api.py` 19개,
  Backend lib Clippy, workspace fmt 및 diff 검사가 통과했다. 전체 회귀는 실행하지 않았다.
- `test_book_manager.py::test_in_names_empty_path`는 Rust preview EPUB 검증 테스트에서 빠져 있었다.
  빈 spine href가 ZIP 경로로 인정되지 않는 같은 경계 사례를 `book_preview` target에 옮겼다.
  Python 원본 테스트 1개와 Rust 포트 테스트 1개가 통과했다.
- 마지막 Python joblib 변환 경로를 Rust `pyferrite` importer와
  `classify_cli import-joblib --model FILE --out DIR`로 연결했다. 변환 규칙은 Python
  `backend/classifier/export.py`의 vectorizer 검증, vocabulary 인덱스, float32 배열 배치와
  `CategoryModel` 구조를 기준으로 맞췄다. Python 코드를 실행하지 않는 parser로 joblib을 읽는다.
  CLI에서 `--golden N --seed S`도 지원하며 Python exporter와 합성 문서, 순위, 사유,
  결정값 및 확신도를 대조한다. `legacy_golden.json` 기준 12개 사례와 seeded 문서가 일치했다.
  `classify_cli` 29개, Python exporter 11개, backend importer 1개 테스트가 통과했다.
  CLI Clippy, workspace fmt와 diff 검사가 통과했다. Rust 소스에서 Python/uv subprocess 호출은 찾지 못했다.
  전체 회귀는 실행하지 않았다.
- `BookManager._normalize_classification_text`의 Python `casefold()`와 Rust `to_lowercase()` 차이를
  `Straße`/`STRASSE` 입력에서 RED로 재현했다. Python 3.13.7·Unicode 15.1.0에서 생성한 전체
  casefold 표를 적용하고 Python 정규식 `\s`와 같은 공백 집합으로 압축한다. Rust `book_manager_2`
  156개와 Python `test_book_manager.py` 330개, casefold 단위 테스트, Backend lib Clippy,
  workspace fmt 및 diff 검사가 통과했다. 전체 회귀는 실행하지 않았다.
- comic directory `title_key`도 Python의 NFKC 후 `casefold()` 대신 `to_lowercase()`를 써서
  `Straße` 사례에서 RED를 확인했다. 전체 casefold 구현을 공용 `pyfmt::py_casefold`로 옮겨
  classifier와 comic 정규화가 공유한다. comic 전체 테스트 39개, pyfmt 테스트 6개,
  backend casefold 회귀 테스트, Python 원본 대조, workspace fmt와 diff 검사가 통과했다.
  관련 두 crate의 Clippy도 경고 없이 통과했다.
  전체 회귀는 미뤘다.
- Python `datetime.fromisoformat()`은 timezone suffix를 받지만 Rust `Book::parse_iso()`는
  거부했다. `+09:30`·`Z` 입력에서 RED를 확인하고 offset을 포함한 timestamp를 파싱하도록 수정했다.
  Python `dict()`의 기존 형식대로 timezone 표시는 직렬화에서 제외한다. Rust `book` 7개,
  Python `test_book.py` 6개, Backend lib Clippy, workspace fmt와 diff 검사가 통과했다.
  전체 회귀는 실행하지 않았다.
- Python `CategoryMapping`의 `get_all_mappings`, `get_keywords`, `get_hidden_categories`,
  `get_latest_excluded_categories`, `search_by_keyword`는 DB 조회 오류를 전달하지만 Rust는 빈 결과로 숨겼다.
  오류 주입 테스트에서 RED를 확인하고 다섯 읽기 경로의 `DbError` 전파를 맞췄다. API는 내부 오류로 처리하고
  bookstore policy는 오류를 반환해 hardcoded fallback 경로를 유지한다.
  Rust `category_mapping` 58개, `mapping_api` 9개, `bookstore_policy` 26개, `main_gaps` 24개,
  Python `test_category_mapping.py` 52개가 통과했다. Backend lib Clippy, workspace fmt 및 diff 검사도 통과했다.
  전체 회귀는 실행하지 않았다.
- Python `get_classify_proposal_items`는 DB 조회 오류를 그대로 전달하지만 Rust는 빈 목록으로 숨겼다.
  오류 주입 단위 테스트에서 RED를 확인하고 `DbError`를 API adapter와 조회·적용 route까지 전달하도록 수정했다.
  Rust `category_mapping` 59개와 `main_gaps` 24개 통과, MySQL 테스트 target 컴파일,
  workspace fmt와 diff 검사가 통과했다. 전체 회귀는 실행하지 않았다.
- Python `get_reload_status`도 DB 조회 오류를 전달하지만 Rust는 이력을 찾지 못한 것처럼 `None`을 돌려줬다.
  오류 주입 단위 테스트에서 RED를 확인하고 `DbError`를 API adapter와 reload-status route까지 전파했다.
  Rust `category_mapping` 60개, `main_gaps` 24개, `main_api` 50개가 통과했고 `main_mock` target을 컴파일했다.
  Backend lib Clippy, workspace fmt와 diff 검사도 통과했다. 전체 회귀는 실행하지 않았다.
- Python `get_classify_proposal_status`는 DB 조회 오류를 전달하지만 Rust는 `idle` 상태로 숨겼다.
  오류 주입 단위 테스트에서 RED를 확인하고 fallible API로 바꿔 status route는 오류 응답을 내며,
  background job은 상태를 확인할 수 없으면 진행을 중단하고 로그를 남긴다.
  Rust `category_mapping` 61개, `main_gaps` 24개, `main_api` 50개가 통과했고
  MySQL status 테스트 target 컴파일, Backend lib Clippy, workspace fmt와 diff 검사가 통과했다.
  전체 회귀는 실행하지 않았다.
- Python의 제안 항목 삭제 메서드 둘은 DB 오류를 전달하지만 Rust는 0건 삭제로 숨겼다.
  오류 주입 단위 테스트에서 RED를 확인하고 `DbError`를 mapping API와 삭제 route까지 전파했다.
  Rust `category_mapping` 62개, `main_gaps` 24개, Python `test_category_mapping.py` 52개가 통과했다.
  MySQL 항목 테스트 target 컴파일, Backend lib Clippy, workspace fmt와 diff 검사도 통과했다.
  전체 회귀는 실행하지 않았다.
- Python `clear_classify_proposal_items`는 DB 오류를 올리지만 Rust는 로그만 남기고 정상 처리처럼 반환했다.
  오류 주입 단위 테스트에서 RED를 확인한 뒤 `DbError`를 반환하도록 바꿨다. 분류 제안 job은 초기화 실패 시
  중단하고, 삭제 API는 오류 응답을 보낸다. Rust `category_mapping` 63개, `main_gaps` 24개,
  Python `test_category_mapping.py` 52개가 통과했다. MySQL 항목 테스트 target 컴파일,
  Backend lib Clippy, workspace fmt와 diff 검사도 통과했다. 전체 회귀는 실행하지 않았다.
- Python `set_classify_proposal_status`는 DB 저장 오류를 전파하지만 Rust는 로그만 남겼다.
  오류 주입 단위 테스트에서 RED를 확인하고 fallible 반환을 추가했다. background 작업은 저장 실패를 기록하며,
  삭제 API는 상태 초기화 저장 실패를 오류로 반환한다. Rust `category_mapping` 64개,
  `main_gaps` 24개, `main_api` 50개, Python MySQL 상태 테스트 16개, Rust MySQL 상태 테스트 11개가 통과했다.
  Backend lib Clippy, workspace fmt와 diff 검사도 통과했다. 전체 회귀는 실행하지 않았다.
- Python `merge_classify_proposal_status`도 부분 상태 저장 오류를 전파하지만 Rust는 로그로 삼켰다.
  오류 주입 테스트에서 RED를 확인하고 `DbError` 반환을 추가했다. 두 progress callback은 실패를 로그로 남긴다.
  Rust `category_mapping` 65개, `main_gaps` 24개, Python `test_category_mapping.py` 52개,
  Rust MySQL 상태 테스트 11개가 통과했다. Backend lib Clippy, workspace fmt와 diff 검사도 통과했다.
  전체 회귀는 실행하지 않았다.
- Python `update_classify_proposal_item_payloads`와 `update_classify_proposal_item_status`는 DB 오류를 전파하지만
  Rust는 로그만 남겼다. 오류 주입 테스트에서 RED를 확인하고 두 API에 `DbError` 반환을 추가했다.
  progress callback은 실패를 로그로 남긴다. fake cursor의 `executemany` 오류 주입 동작도 보강했다.
  Rust `category_mapping` 66개, `main_gaps` 24개, Python CategoryMapping 52개,
  Rust MySQL 항목 테스트 16개가 통과했다. Backend lib Clippy, workspace fmt와 diff 검사도 통과했다.
  전체 회귀는 실행하지 않았다.
- Python `add_classify_proposal_items`는 DB 삽입 오류를 전달하지만 Rust는 로그만 남겼다.
  오류 주입 테스트에서 RED를 확인한 뒤 `DbError` 반환을 추가했고 progress callback은 삽입 실패를 기록한다.
  Rust `category_mapping` 67개, `main_gaps` 24개, Python CategoryMapping 52개,
  Rust MySQL 항목 테스트 16개가 통과했다. Backend lib Clippy, workspace fmt와 diff 검사도 통과했다.
  전체 회귀는 실행하지 않았다.
- Python `release_reload_lock`은 DB 삭제 오류를 호출자에게 전달하지만 Rust는 로그만 남겼다.
  오류 주입 테스트에서 RED를 확인한 뒤 `DbError` 반환을 추가했다. Rust `category_mapping` 68개와
  Python `test_category_mapping.py` 52개가 통과했다. Backend lib Clippy, workspace fmt와 diff 검사도 통과했다.
  전체 회귀는 실행하지 않았다.
- Python `add_keyword`는 `IntegrityError`만 중복으로 처리하지만 Rust는 다른 DB 오류도 `false`로 숨겼다.
  오류 주입 테스트에서 RED를 확인하고 중복은 `Ok(false)`, 다른 오류는 `Err`로 나누었다.
  Rust `category_mapping` 69개, `mapping_api` 9개, `main_api` 50개,
  Python `test_category_mapping.py` 52개가 통과했다. Backend lib Clippy, workspace fmt와 diff 검사도 통과했다.
  전체 회귀는 실행하지 않았다.
- Python `remove_keyword`는 DB 오류를 전파하지만 Rust는 삭제 실패를 `false`로 바꿨다.
  오류 주입 테스트에서 RED를 확인하고 `DbError`를 facade와 route 경계까지 전달했다.
  Rust `category_mapping` 70개, `mapping_api` 9개, `main_api` 50개,
  Python `test_category_mapping.py` 52개가 통과했다. Backend lib Clippy, workspace fmt와 diff 검사도 통과했다.
  전체 회귀는 실행하지 않았다.
- Python `delete_category`와 숨김·최신 제외 해제는 DB 오류를 전파하지만 Rust는 `false`로 숨겼다.
  오류 주입 테스트에서 RED를 확인하고 세 경로를 fallible 반환으로 바꿨다. 설정 추가는 Python 규칙대로
  오류를 `false`로 유지한다. Rust `category_mapping` 73개, `mapping_api` 9개, `main_api` 50개,
  Python CategoryMapping 52개가 통과했다. Backend lib Clippy, workspace fmt와 diff 검사도 통과했다.
  전체 회귀는 실행하지 않았다.
- Python `acquire_reload_lock`은 락 조회나 충돌 상태 확인의 DB 오류를 호출자에게 전달하지만 Rust는
  락을 얻지 못한 결과로 숨겼다. 오류 주입 테스트에서 RED를 확인하고 `DbError` 반환을 추가했다.
  route와 adapter도 오류 응답을 내도록 바꿨다. Rust `category_mapping` 74개, `main_gaps` 24개,
  `main_api` 50개, Python `test_category_mapping.py` 52개가 통과했다.
  전체 프로젝트 회귀는 뒤로 미뤘다.
- Python `_try_transition_classify_proposal`는 행 생성·잠금·상태 갱신의 DB 오류를 전파하지만 Rust는
  `{status: error}` 상태처럼 반환해 오류 응답이 되지 않았다. 오류 주입 테스트에서 RED를 확인한 뒤
  두 상태 전이 API를 `Result`로 바꾸고 adapter와 route까지 전달했다.
  Rust `category_mapping` 75개, `main_gaps` 24개, `main_api` 50개가 통과했고
  MySQL 상태 target과 `main_mock` target을 컴파일했다. Python 제안 상태 테스트 5개도 통과했다.
  전체 프로젝트 회귀는 뒤로 미뤘다.
- Python `get_classify_proposal_items`는 잘못된 JSON payload를 전달하지만 Rust는 로그만 남기고 해당 항목을
  건너뛰었다. 오류 주입 테스트에서 RED를 확인한 뒤 누락·잘못된 JSON·객체가 아닌 payload를 오류로 반환한다.
  Rust `category_mapping` 76개가 통과했다. 전체 프로젝트 회귀는 뒤로 미뤘다.
- Python `_read_classify_proposal_status_row`는 잘못된 JSON payload를 전달하지만 Rust는 `null`로 대체했다.
  오류 주입 테스트에서 RED를 확인하고 payload JSON 파싱 오류를 전파하도록 수정했다.
  Rust `category_mapping` 77개가 통과했다. 전체 프로젝트 회귀는 뒤로 미뤘다.
- 로컬 실모델 golden 266건으로 Rust classifier parity 검사를 실행했다. 모델 로드 15.19초,
  예측 0.49 ms/doc, 최대 score 오차 `1.634e-7`, 순위·사유·카테고리 불일치 0건으로 통과했다.
  실제 운영 corpus 재학습의 sklearn parity와 메모리·학습 시간 비교는 별도 잔여 작업이다.
- Python의 `/category-mismatches/{category:path}`는 slash가 포함된 카테고리를 받지만 Rust는
  Axum catch-all이 아닌 동적 segment를 써서 `/A/B` 요청에 404를 반환했다. 경로 전달 테스트를 RED로 확인한 뒤
  `{*category}`로 수정했다. Rust `main_gaps` 24개가 통과했다. 전체 회귀는 미뤘다.
- Rust `ESManager`의 ignored 실서비스 테스트를 실행해 Elasticsearch 9.1.0에 연결했다.
  nori mapping, 한글 bulk 색인·검색, paging·집계, 갱신·삭제와 오류 응답 검증이 통과했다.
  전용 Docker 컨테이너도 테스트 종료 후 제거됐다. 전체 회귀는 미뤘다.
- `backend::db`의 Docker ignored integration 중 Python DictCursor 대응 round-trip 하나를 실행했다.
  MySQL 8.4에서 한글·NULL·DATETIME의 왕복 변환이 통과했고, 일회용 컨테이너는 종료 후 정리됐다.
- 분류 제안 상태의 MySQL integration 11개를 `--test-threads=1`로 순차 실행했다.
  stale takeover, applying/start 동시 선점, 상태 병합과 content type 분리가 모두 통과했다.
  일회용 DB 컨테이너는 종료 후 제거됐다.
- Python `ComicsManager`는 `BookManager`의 분류 제안과 callback 적용 동작을 상속하지만 Rust facade는 trait 기본값을 사용했다.
  proposal 테스트에서 `unimplemented` 오류로 RED를 확인하고 두 API를 공유 `BookManager` 구현에 위임했다.
  comics proposal 및 취소 callback 개별 테스트 2개와 Backend lib Clippy가 통과했다.
  전체 회귀 테스트는 실행하지 않았다.
- Python의 `trained` fixture로 동일한 120문서를 Rust에서 학습해 report와 decision score를 비교했다.
  새 parity 테스트가 기존 dual optimizer 종료 기준에서 최대 score 오차 `4.69e-4`로 실패했다.
  projected-gradient 기준을 `1e-4`에서 `1e-8`로 낮춘 뒤 오차가 `2.13e-4`가 되어 `3e-4` 허용치 안에 들었다.
  Python 학습 개별 테스트, Rust parity·spill 학습 개별 테스트, Backend lib Clippy와 workspace fmt가 통과했다.
  전체 회귀는 실행하지 않았다.
- Python `classify_cli reclassify`는 파일별 이동 실패를 집계·출력해도 종료 코드 `0`을 반환하지만 Rust는 `1`을 반환했다.
  Python 기준 테스트의 동작을 Rust 실패 이동 테스트에 반영해 RED를 확인하고 종료 코드를 맞췄다.
  Rust 이동 실패·정상 이동 테스트, Python 원본 테스트, classify_cli all-target Clippy와 workspace fmt가 통과했다.
  전체 회귀는 실행하지 않았다.
- Python/Rust `view_history_store`의 MySQL 단일 조회 기록 계약을 같은 fixture 시나리오로 대조했다.
  양쪽 `test_record_and_list_single_view`가 통과했다. 전체 회귀는 실행하지 않았다.
- Python의 classify proposal 진행 callback은 새 항목·분류 결과·진행 상태 저장 오류를 잡아 작업을 `failed`로 마감한다.
  Rust는 세 저장 경로의 오류를 로그만 남기고 성공 결과면 `ready`로 기록했다. DB 오류 주입 테스트에서 RED를 확인한 뒤
  Rust도 flush 또는 progress 상태 저장 실패를 `failed`로 마감하게 수정했다. 관련 Rust 실패 테스트 3개,
  배치 성공 테스트와 Python 기준 배치 테스트가 통과했다. 전체 회귀는 실행하지 않았다.
- Python의 classify apply callback은 항목 상태·진행 상태 DB 오류를 잡아 작업을 `failed`로 마감한다.
  Rust는 두 오류를 로그로 삼킨 뒤 `done`으로 기록했다. 오류 주입 테스트에서 각각 RED를 확인하고,
  Rust도 callback 저장 실패 시 `failed`를 기록하도록 맞췄다. Rust 오류 테스트 2개, 허용 항목 callback 테스트,
  Python 기준 항목 상태 테스트가 통과했다. 전체 회귀는 실행하지 않았다.
- Python classify apply는 완료 상태 조회·저장 오류도 잡아 `failed`를 기록한다.
  Rust는 완료 상태 DB 호출 오류를 로그로 남기고 `applying`에 두었다. 일시 오류 주입 테스트 2개에서 RED를 확인한 뒤
  Rust가 재조회해 실패 상태를 기록하도록 수정했다. 관련 apply 오류 테스트 4개와 정상 callback 테스트가 통과했다.
  전체 회귀는 실행하지 않았다.
- Python classify proposal도 완료 상태 조회·저장 오류를 잡아 `failed`로 마감한다.
  Rust는 해당 오류를 로그만 남겨 `running`에 두었다. 일시 DB 오류 주입 테스트 2개에서 RED를 확인하고,
  Rust도 실패 상태를 저장하도록 수정했다. proposal 오류 테스트 5개와 배치 성공 테스트가 통과했다.
  전체 회귀는 실행하지 않았다.
- Python classify proposal/apply는 manager 예외를 잡아 작업 상태를 `failed`로 마감한다.
  Rust task는 panic을 전파해 제안은 `running`, 적용은 `applying`에 남겼다. 두 HTTP 테스트에서 RED를 확인한 뒤
  task 경계에서 panic을 잡고 실패 상태를 기록하도록 보완했다. 대체 승인 token은 보존한다.
  Rust panic 테스트 3개, 정상 proposal/apply 테스트 2개, Python 예외 기준 테스트가 통과했고 대상 Clippy도 통과했다.
  전체 회귀는 실행하지 않았다.
- Python `test_train_spill_cleanup_preserves_other_files_in_requested_directory`가 검증하는 spill 디렉터리 보호 계약을
  활성 Rust CLI 테스트 target에 추가했다. 구현은 기존대로 사용자 marker를 보존하고 생성 spill 파일만 정리했다.
  새 Rust 개별 테스트 1개가 통과했다. 전체 회귀는 실행하지 않았다.
- Rust/Python `view_history_store`의 유형별 최대 최근 조회 수 제한을 동일 시나리오로 대조했다.
  양쪽 개별 테스트가 통과했다. 전체 회귀는 실행하지 않았다.
- 조회 이력의 book/comic 상한 독립성도 Python/Rust 동일 테스트로 대조했다.
  양쪽 개별 테스트가 통과했다. 전체 회귀는 실행하지 않았다.
- 조회 이력 API의 요청 limit 상한 동작을 Python/Rust 동일 테스트로 대조했다.
  양쪽 개별 테스트가 통과했다. 전체 회귀는 실행하지 않았다.
- 최근 조회 활동에 따른 사용자 정렬도 Python/Rust 개별 테스트에서 통과했다.
  전체 회귀는 실행하지 않았다.
- 사용자 간 조회 이력 분리도 Python/Rust 개별 테스트에서 통과했다.
  전체 회귀는 실행하지 않았다.
- 500자 제목의 조회 이력 저장도 Python/Rust MySQL 개별 테스트에서 통과했다.
  전체 회귀는 실행하지 않았다.
- 원본 도서 삭제 뒤 조회 시점 제목 보존도 Python/Rust MySQL 개별 테스트에서 통과했다.
  전체 회귀는 실행하지 않았다.
- Python ISBN PDF 추출은 `SIGALRM` timeout으로 파서를 중단하지만 Rust loader 경로는 timeout 뒤 작업 thread를 남길 수 있었다.
  실행기에서 worker binary를 전달하는 seam과 ISBN PDF hidden command를 추가해 PDFium/fallback을 자식 프로세스에서 실행한다.
  hard-timeout 시 worker 종료, worker 응답 파싱, loader/backend command dispatch를 각각 테스트했다.
  Rust ISBN worker 테스트 2개, loader/backend dispatch 테스트 각 1개, 기존 ISBN timeout 테스트와 Python timeout 기준 테스트가 통과했다.
  `cargo clippy -p isbn -p loader -p backend --all-targets -- -D warnings`와 workspace fmt 검사가 통과했다.
  전체 회귀는 실행하지 않았다. 사용자 주입 `Backends`와 `extract_default()` 직접 호출은 기존 thread timeout 경로를 유지한다.
- loader 실행 파일을 worker로 사용해 synthetic PDF에서 ISBN 추출까지 end-to-end로 검증했다.
  대응 Python PDF ISBN 테스트도 개별 실행해 통과했다. loader test Clippy와 workspace fmt가 통과했다.
- `read_file`이 `StageRunner`의 실행 파일을 ISBN 파서에 전달하는 경로를 테스트했다.
  CPU 무한 루프 worker 강제 종료와 `test_loader.py::TestIsbnPdfTimeout` 기준 테스트가 통과했다.
  관련 loader/isbn target Clippy와 workspace fmt가 통과했다. 전체 회귀는 실행하지 않았다.
- Python PIL의 회전 crop 기준 픽셀과 Rust 출력이 다른 문제를 Python fixture로 RED 재현했다.
  Rust bicubic을 Pillow `Geometry.c`와 같은 가로·세로 polynomial 순서 및 정수 절삭으로 바꿨다.
  회전 grayscale 픽셀 parity를 포함한 Rust `image_edit` 5개와 target Clippy,
  workspace fmt 검사가 통과했다.
  Python pytest는 현재 `.venv`의 pytest entrypoint가 전역 pytest 9와 섞이고 `pypdf`도 없어 실행하지 못했다.
  전체 회귀는 실행하지 않았다.
- Python `classify_books_client.py`는 API 모드에서 direct 전용 옵션을 무시하지만 Rust CLI는 오류로 거부했다.
  `--api-url`과 `--limit`·`--offset`·`--uncached-only`·`--re-evaluate` 조합을 새 테스트로 RED 재현한 뒤,
  Python argparse와 같이 허용하도록 수정했다. `classify_books_client` client 테스트 12개,
  target Clippy, workspace fmt 검사가 통과했다. 전체 회귀는 실행하지 않았다.
- `normalize_comic_directories`의 Python 생성 이름 golden과 source-backed 계획 테스트를 대조했다.
  names 10개와 plan 26개가 통과했고, 이 대조에서 새 동작 차이는 발견되지 않았다.
  전체 회귀는 실행하지 않았다.
- 운영 `model.joblib` import가 13분 이상 걸리는 병목은 pyferrite의 insertion-ordered `Dict`가
  문자열 key를 선형 검색하던 동작이었다. vendored `libs/pyferrite`에 collision 검증을 포함한 문자열 index를 추가했고,
  25,000개 key 성능 테스트는 15.126초에서 0.04초로 줄었다. Rust importer release 실행은 빌드 포함 1분 38초,
  메모리 최대 5.1GB였으며 coefficient 파일은 Python export와 SHA256이 같았다.
- 운영 joblib의 nested `config.fields`가 importer의 JSON 변환에서 비어 필드 weight가 기본값으로 기록됐다.
  학습 시 실제 사용한 `feature_space.fields`로 config의 fields를 복원했다. author/publisher/file_type weight 확인과
  운영 모델 Python golden 266건 parity가 통과했고, classifier model 개별 테스트 30개, target Clippy 및 workspace fmt도 통과했다.
  전체 회귀 테스트는 사용자 요청대로 변환 완료 시점까지 미뤘다.
- Rust `_is_mostly_blank_cover`는 0×0 이미지 resize 결과를 단색 표지로 오판했다.
  기존 ignore 테스트를 활성화해 RED를 확인하고 빈 이미지면 `false`를 반환하도록 수정했다.
  `book_manager_gaps` 48개, 해당 target Clippy, workspace fmt와 Python 기준 테스트 1개가 통과했다.
  전체 회귀 테스트는 실행하지 않았다.
- Python corpus 수집은 ES hit를 JSONL에 순차 기록하지만 Rust `collect`는 전체 문서를 메모리에 쌓았다.
  다음 ES scroll 요청 시 첫 페이지 기록 여부를 확인하는 테스트에서 RED를 재현했다.
  Backend collector와 `classify_cli collect`를 page streaming으로 바꿔 페이지마다 JSONL을 flush하고,
  건수별 통계만 메모리에 유지한다. CLI custom label pattern과 index 전달도 유지했다.
  Rust `classifier_corpus` 14개와 classify_cli 31개, Python corpus 13개 및 CLI collect 3개,
  target Clippy, workspace fmt와 diff 검사가 통과했다.
  전체 회귀 테스트는 실행하지 않았다.
- Python 설정의 `label_pattern`은 `re` lookahead를 허용하지만 Rust `regex`는 거부했다.
  설정 패턴에 lookahead를 넣는 CLI 테스트에서 RED를 확인했다. backend 내부를 `fancy_regex`로 바꿔
  라이브러리 의존성 추가 없이 Python 패턴을 지원한다. Python corpus 기준 테스트 14개,
  Rust corpus 14개와 classify_cli 33개, target Clippy, workspace fmt와 diff 검사가 통과했다.
  동률 카테고리는 Rust가 첫 등장 순서를 유지함을 별도 parity 테스트로 확인했다.
  전체 회귀 테스트는 실행하지 않았다.
- `ESManager.search_by_id`가 Python처럼 404만 빈 객체로 처리하고 다른 ES 오류는 전파하도록 맞췄다.
  unexpected error 테스트에서 기존 Rust가 오류를 삼키는 차이를 재현한 뒤 `Result`로 반환하게 수정했다.
  Rust 관련 테스트 3개, Python 기준 테스트 3개, target Clippy와 workspace fmt가 통과했다.
  전체 회귀 테스트는 실행하지 않았다.
- backend와 운영 `utils`의 Python 모듈·공개 함수 이름을 Rust 소스와 대조했고,
  실행 설정에서 Python backend를 직접 띄우는 경로는 찾지 못했다. HWP3 관련 Rust 63개와 Python 34개,
  parser timeout Rust·Python 각 12개 테스트가 통과했다. 이는 모듈 존재·선택 테스트 근거이며
  기능 동등성 전수 검증은 계속한다. 전체 회귀 테스트는 실행하지 않았다.
- Python `get_default_model`의 전역 cache는 운영 코드에서 한 번만 초기화되고,
  `reset_default_model`은 pytest fixture 전용이다. Rust는 공유 운영 `BookManager`의 `OnceLock`로
  기본 서비스를 한 번 구성한다. 모델 경로 저장·로드 기준 테스트는 Python·Rust 각각 통과했다.
- Python classifier CLI의 `--n-jobs`는 설정값뿐 아니라 category별 One-vs-Rest 학습 병렬도를 제어한다.
  Rust CLI의 옵션 누락을 RED로 재현한 뒤 설정 전달과 제한된 worker 학습을 구현했다.
  추가 대조에서 `n_jobs=-1` 의미 누락도 발견해 사용 가능한 CPU 수로 해석하도록 맞췄다.
  직렬·병렬 score 일치와 `-1` CLI 학습, Rust CLI 35개와 Python CLI 43개 테스트가 통과했고,
  관련 target Clippy, workspace fmt, diff 검사가 통과했다. 전체 회귀 테스트는 실행하지 않았다.
- Python `classify_cli -v`는 DEBUG 로그를 켜지만 Rust 실행 파일은 해당 옵션을 버리고 logger도 등록하지 않았다.
  손상 EPUB 읽기 진단을 출력하는 테스트로 RED를 확인한 뒤 CLI에 INFO/DEBUG logger를 연결했다.
  verbose/quiet 진단 테스트와 Rust CLI 35개, Python CLI 43개, classify_cli all-target Clippy,
  workspace fmt, diff 검사가 통과했다. 전체 회귀 테스트는 실행하지 않았다.
- Python `ESManager._search`는 10,000건 초과 요청을 scroll로 읽지만 Rust `es_client::search`는 첫 10,000건만 반환했다.
  10,001건 조회와 scroll ID 정리를 테스트해 RED를 확인한 뒤 페이지별 score 정규화, early exit,
  scroll context cleanup을 구현했다. max_score 누락·scroll 오류 정리도 Python 동작과 대조했다.
  Rust ES client 25개와 Python scroll 기준 7개, crate Clippy, workspace fmt, diff 검사가 통과했다.
  전체 회귀 테스트는 실행하지 않았다.
- MySQL 컨테이너가 사용 가능함을 확인해 기존 ignore였던 조회 이력 저장소 검증을 실행했다.
  Rust MySQL 테스트 13개와 Python 기준 모듈 테스트 15개가 모두 통과했다.
  이는 view history 저장소 범위만 검증했으며 전체 회귀 테스트는 실행하지 않았다.
- ES indexing의 100건 bulk 단위, `num_docs` 경계, 전송 실패 재시도를 Python `ESManager.insert`와 대조했다.
  Rust indexing target 15개와 Python insert 기준 3개가 통과했고, 추가 동작 차이는 찾지 못했다.
  전체 회귀 테스트는 실행하지 않았다.
- Rust 분류 CLI에 joblib importer가 있는데 workspace 가이드는 Python exporter 실행을 안내하고 있었다.
  가이드를 `classify_cli import-joblib`로 바꾸고 기존 CLI 변환 테스트 1개를 실행해 통과를 확인했다.
  Python exporter는 기준 동작 검증용으로 유지하며, 전체 회귀 테스트는 실행하지 않았다.
- Python PDF 페이지 API는 페이지 수 cache miss에서 연 `PdfReader`를 쪽 추출까지 재사용하지만 Rust는 쪽수만 cache하고 파일을 다시 열었다.
  reader 재사용 테스트에서 기존 `PdfPageOps`가 열린 reader 추출을 제공하지 않음을 RED로 확인했다.
  Rust는 parsed lopdf document를 reader cache에 보관하고 추출마다 복제해 사용한다.
  Rust PDF pages 25개, reader/page-count cache 3개, preview 생성·cache hit 2개와 Python 기준 4개,
  target Clippy, workspace fmt와 diff 검사가 통과했다. 전체 회귀는 실행하지 않았다.
- Python pikepdf는 손상 xref를 복구해 열더라도 syntax warning을 반환하지만 Rust 검증기는 항상 빈 경고를 냈다.
  잘린 PDF fixture로 Python `valid=false`와 Rust 빈 warning을 확인해 RED를 재현했다.
  Rust `LopdfDoc`가 lopdf의 xref offset repair/reconstruction 상태를 warning으로 반환하도록 연결했다.
  Rust PDF validation 10개와 Python 기준 5개, target Clippy, workspace fmt, diff 검사가 통과했다.
  전체 회귀 테스트는 실행하지 않았다.
- Rust 빈 디렉터리 정리는 `FsOps` 주입을 받으면서 `std::fs::remove_dir`를 직접 호출했다.
  삭제 거부 hook 테스트가 실제 디렉터리를 지우는 RED를 확인한 뒤, `FsOps::remove_dir`를 추가하고
  classifier와 BookManager 정리 경로가 같은 주입 판을 쓰도록 맞췄다. classifier cleanup 5개,
  seam 9개, BookManager classifier 6개, Python 기준 5개, target Clippy·workspace fmt·diff 검사가 통과했다.
  전체 회귀 테스트는 실행하지 않았다.
- Python 서점 요청은 기본 10초, Toptoon catalog는 20초, 표지 fetch는 connect 5초/read 15초를 지정한다.
  Rust `HttpSession`은 timeout을 전달하지 않아 전역 15초로 동작했다. Toptoon·표지 전달 테스트와
  늦은 response header/body 테스트에서 RED를 확인하고, 호환 기본 메서드와 ureq 요청별 connect·response·body 제한을 추가했다.
  Rust bookstore 108개, cover_sources 27개, Python 기준 145개, target Clippy·workspace fmt·diff 검사가 통과했다.
  전체 회귀 테스트는 실행하지 않았다.
- Python `direct_cover_url`은 DNS 검증 IP 연결에 connect 5초/read 15초를 준다. Rust `SystemNet`은 전역 15초만 설정했다.
  timeout이 `ConnectRequest`까지 전달되는 assertion과 지연 response-header 테스트가 RED를 보였다.
  느린 연속 body가 ureq의 전체 body deadline 때문에 실패하는 RED를 확인했다. bounded channel reader를 두고
  각 read마다 idle timeout을 적용했으며, timeout 초과 시 Agent shutdown으로 reader를 중단한다.
  idle timeout·느린 연속 응답을 포함한 direct URL 테스트 12개와 Python 기준 26개,
  target Clippy·workspace fmt·diff 검사가 통과했다. 전체 회귀는 실행하지 않았다.
- Python 서점 세션도 `requests` read timeout을 전체 body 시간이 아닌 다음 데이터 대기 시간으로 적용한다.
  청크 사이 60ms 간격으로 180ms 동안 응답하는 테스트에서 Rust의 total-body deadline 실패를 재현했다.
  bounded reader가 청크마다 idle timeout을 다시 적용하고, 완료한 Agent는 세션 pool에 반환한다.
  timeout 난 Agent만 폐기하며, 연결 재사용과 timeout 뒤 요청 복구 테스트도 통과했다.
  Rust bookstore 테스트 111개와 Python 기준 97개, target Clippy, workspace fmt, diff 검사가 통과했다.
  전체 workspace 회귀 테스트는 실행하지 않았다.
- Python `cover_status`·`cover_browse`와 Rust 포트를 다시 대조했다. 표지 판정/cache, 페이지 병합,
  대표 권 선택, 동시 scan 제한에서 추가 차이를 찾지 못했다. Rust target 22개와 28개,
  Python 기준 45개, target Clippy, workspace fmt, diff 검사가 통과했다. 전체 회귀는 실행하지 않았다.
- 비활성 `ItemRouter` classify-proposal 테스트를 HTTP facade 테스트와 대조했다. 배치 크기·꼬리 flush·진행 중 노출,
  이름 placeholder 후 payload 갱신, 오류·재시도·토큰 제한은 활성 facade 테스트에서 확인했다.
  Rust `main_mock` 분류 제안 테스트 33개와 Python 기준 28개가 통과했다. Python `to_thread` 관찰 테스트에
  대응하는 Rust 호출은 `TaskRunner`가 만든 전용 작업 스레드 안에서 실행된다. Python 이벤트 루프를 막지 않는다는
  사용자 동작은 Rust facade 경계에서 유지되며, `to_thread` 자체를 그대로 옮길 필요는 없다. batch facade 테스트와
  Python 기준을 각각 통과했고 전체 회귀는 실행하지 않았다.
- 최종 workspace 실행에서 `main_gaps` apply 성공 테스트가 실패했다. `MockCat`이 item status callback을
  구현하지 않아 기본 `unimplemented` 오류를 냈다. 테스트 대역에 항목 상태 저장을 구현했고 해당 테스트가 통과했다.
  재실행에서 `server_cli` 2개가 필수 `TM_FRONTEND_URL` 누락으로 실패해 테스트 command에 값을 고정했다.
  해당 target 3개가 통과했다. scanner 제외 Rust workspace 회귀와 all-target Clippy가 직렬 실행으로 통과했다.
- Python 기준 회귀는 frontend 전용 테스트 두 파일을 제외하고 3,041개 통과, PDF 표지 테스트 3개 실패했다.
  세 실패는 기존 Pillow API 호환성 실패와 같으며 Python 원본 파일은 이번 묶음에서 수정하지 않았다.
  이 실행은 Rust 전환 코드 검증을 위한 기준 비교다.
- loader의 비재귀 샘플 탐색은 Python `DirEntry.is_dir(follow_symlinks=False)`와 달리 Rust `Path::is_dir()`가
  디렉터리 symlink도 샘플 카테고리로 선택했다. Rust 회귀 테스트에서 중복 샘플을 RED로 확인하고 entry의
  `file_type()`으로 실제 디렉터리만 고르게 수정했다. 파일 symlink는 포함하고 디렉터리 symlink는 건너뛰는
  동작도 테스트했다. `loader_2` 106개, loader all-target Clippy, workspace fmt와 diff 검사가 통과했다.
  전체 회귀는 실행하지 않았다.
- Python `classify_cli train`은 corpus에 publisher가 없으면 전집 판정 품질 경고를 stderr에 출력하지만 Rust는 조용했다.
  Python 기준 테스트를 Rust CLI에 옮겨 RED를 확인한 뒤 publisher 존재 여부에 Python truthiness를 적용하고 같은 경고를 출력했다.
  Rust `classify_cli` 36개, Python 기준 43개, crate all-target Clippy, workspace fmt와 diff 검사가 통과했다.
  전체 회귀는 실행하지 않았다.
- Python `classify_cli`는 corpus가 없을 때 전용 안내를 출력하지만 Rust는 파일 I/O 오류를 노출했다.
  Python 기준의 missing/empty 테스트를 Rust CLI에 추가해 missing 경로의 RED를 확인한 뒤 오류 문구를 구분했다.
  Rust `classify_cli` 37개와 Python 기준 43개, crate all-target Clippy, workspace fmt와 diff 검사가 통과했다.
  전체 회귀는 실행하지 않았다.
- `BookManagerApi`와 `CategoryMappingApi`의 기본 `unimplemented` 메서드를 운영 adapter와 대조했다.
  BookManager, ComicsManager, CategoryMapping 구현이 선언된 모든 메서드를 override하며 서버는 `AppDeps::from_env()`를 사용한다.
  source inventory와 함께 Rust `main_mock` 132개, Python 기준 139개가 통과했다.
  Python 테스트 경고 9건은 기존 coroutine 미대기·Starlette cookie deprecation이다. 전체 회귀는 실행하지 않았다.
- `EsManagerApi` 29개 메서드를 운영 `ESManager` 구현과 대조해 미구현 override가 없음을 확인했다.
  Rust `es_manager` 92개, gaps 8개, HTTP 12개가 통과하고 Docker 의존 1개는 ignore됐다.
  Python ES manager 기준 103개가 통과했다. 전체 회귀는 실행하지 않았다.
- refresh-token 저장소의 Python/Rust 개별 테스트를 비교했다. 회전·재사용 탐지·유예 시간·세션 집계·폐기 동작이 통과했다.
  Rust 63개 통과·3개 Docker 테스트 무시, Python 70개 통과다. 전체 회귀는 실행하지 않았다.
- ISBN 추출 유틸리티의 Rust/Python 개별 테스트를 비교했다. PDF·EPUB·DjVu·HWP·OCR 번호 변형과 timeout 경로가 통과했다.
  Rust 38개와 Python 50개가 통과했다. 전체 회귀는 실행하지 않았다.
- 공용 `Stat`과 parser timeout 유틸리티를 원본 테스트와 대조했다. 출력 형식·thread safety·deadline·hard timeout이 통과했다.
  Rust 각 12개, Python 각 5개와 12개가 통과했다. 전체 회귀는 실행하지 않았다.
- `file_time`의 Linux btime, 플랫폼 birthtime/ctime fallback과 `corpus_layout` 제외 경로를 원본과 대조했다.
  Rust `file_time` 10개, Python 10개, Rust `corpus_layout` 5개가 통과했다. 전체 회귀는 실행하지 않았다.
- HWP3 `hnc_to_unicode` 결과 전체 65,536개 입력을 Python 기준 digest와 대조하는 Rust 테스트를 추가했다.
  Python lone surrogate는 Rust 파서의 내부 sentinel로 정규화해 비교했다. HWP3 parser 25개, table 4개,
  Python parser 97개 테스트가 통과했다. 전체 회귀는 실행하지 않았다.
- HWP3 parser의 loader fallback 통합 경로도 대조했다. Rust `loader_hwp3` 63개와
  Python loader의 `hwp3` 선택 테스트 34개가 통과했다. 나머지 loader 테스트는 실행하지 않았다.
- loader 모듈 테스트를 단독 실행했다. Rust의 `loader_1` 178개, `loader_2` 106개,
  `loader_gaps` 1개, `loader_hwp3` 63개와 Python `tests/test_loader.py` 371개가 통과했다.
  전체 회귀는 실행하지 않았다.
- `main_mock`의 비활성 내부 `ItemRouter` draft 13개를 활성 HTTP 테스트와 대조했다.
  batching·placeholder/update·허용 경로·건별 상태·apply token·진행 카운터는 공개 경로 테스트가 검증한다.
  누락된 DB write worker 보장은 `main_gaps` HTTP 작업에 thread assertion을 추가해 확인했다.
  `main_mock` 132개, `main_gaps` 24개, Python `test_main_mock` 139개가 통과했다.
- `classify_cli` 등록 target과 Python 기준 CLI 테스트를 단독 실행했다. Rust 37개와 Python 43개가 통과했다.
  `backend/classify_cli*.rs`의 sklearn 표시 테스트는 미등록 draft이며 실행 대상이 아님을 확인했다.
- 검증 비용을 줄이기 위해 변환 묶음마다 관련 Rust/Python 개별 테스트만 실행한다.
  workspace 전체 회귀는 모든 변환 묶음과 실행 경로 점검을 마친 뒤 마지막 단계에서 한 번 실행한다.
- 일반 API target을 단독 확인했다. Rust `main_api` 51개 통과, 35개는 외부 ES/MySQL 의존으로 무시됐다.
  Python `tests/test_main.py` 95개가 통과했다. 서비스가 필요한 테스트는 로컬 서비스가 없어 실행하지 않았다.
- 관리 API 간극 테스트를 단독 실행했다. Rust `main_gaps` 24개와 Python `tests/test_main_gaps.py` 11개가 통과했다.
  Python fixture의 testcontainers deprecation 경고 1건이 있었다. 전체 회귀는 실행하지 않았다.
- 관리 API 묶음을 다시 대조했다. Rust `book_admin_api` 8개와 Python `tests/test_main.py` 95개가 통과했다.
  Python testcontainers와 Starlette cookie deprecation 경고 7건이 있었다. 전체 회귀는 실행하지 않았다.
- 표지 탐색 로직을 다시 대조했다. Rust `cover_browse` 28개와 Python `tests/test_cover_browse.py` 28개가 통과했다.
  업데이트 도구도 대조해 Rust `update` 11개, Python `tests/test_update.py` 4개가 통과했다.
  Python 경고는 testcontainers deprecation과 `runpy` 모듈 경고였다. 전체 회귀는 실행하지 않았다.
- `move_semi_clusters`를 Python 원본과 직접 대조했다. 보고서 처리, 파일 이동·중복 제거, 빈 디렉터리 청소,
  그리스 신화 도서 예외가 Rust 테스트 13개에서 통과했다. 대응하는 Python pytest 파일은 저장소에 없다.
  기능 구현의 명시적 Python 기준 테스트가 없는 항목이어서 원본 코드 비교로 확인했고 전체 회귀는 실행하지 않았다.
- `normalize_comic_directories`를 Python 원본 및 Python에서 생성한 parity cases와 대조했다.
  Rust CLI 3개, 이름·메타데이터 10개, 계획·cache·rename 26개가 통과했다.
  대응 Python pytest 파일은 저장소에 없어 Rust parity fixtures와 원본 코드 비교를 사용했다. 전체 회귀는 실행하지 않았다.
- MySQL 의존 조회 이력 API 테스트를 각각 단독 실행했다. Rust의 기존 ignored 테스트 2개를 활성 실행해 통과했고,
  Python `tests/test_view_history_api.py` 19개도 통과했다. testcontainers deprecation 경고 1건이 있었다.
  전체 회귀는 실행하지 않았다.
- 조회 이력 저장소의 Docker 전용 테스트를 단독 활성 실행했다. Rust 13개와 Python `tests/test_view_history_store.py` 15개가 통과했다.
  Python testcontainers deprecation 경고 1건이 있었다. 전체 회귀는 실행하지 않았다.
- 표지 관리 API와 파일 처리 묶음을 단독 대조했다. Rust `cover_management` 69개,
  Python `tests/test_cover_management.py` 141개가 통과했다. Python testcontainers deprecation 경고 1건이 있었다.
  전체 회귀는 실행하지 않았다.
- MySQL 전용 refresh-token 통합 사례 3개를 활성 실행해 통과했다. Python `tests/test_refresh_token_store.py` 70개도 통과했다.
  MySQL DB adapter 통합 테스트 8개도 통과했으며 직접 대응하는 Python `test_db.py`는 없다.
  전체 회귀는 실행하지 않았다.
- classify-proposal MySQL 저장소 테스트를 단독 활성 실행했다. 항목 DB Rust/Python 각각 16개,
  상태 DB Rust 11개와 Python 16개가 통과했다. Python testcontainers deprecation 경고 각 1건이 있었다.
  전체 회귀는 실행하지 않았다.
- Elasticsearch 9.1 + Nori 일회용 컨테이너에서 `es_manager_http` ignored 통합 1개가 통과했다.
  Python `tests/test_es_manager.py` 95개도 통과했다. Rust `main_api` 실제 ES/MySQL GET 사례 8개를
  고유 index와 일회용 MySQL로 실행해 통과했다. fixture 파일과 컨테이너 정리를 확인했다.
- 표지 판정·미표지 조회 API를 단독 대조했다. Rust `cover_status` 22개와 Python `tests/test_cover_status.py` 17개가 통과했다.
  Python testcontainers deprecation 경고 1건이 있었다. 전체 회귀는 실행하지 않았다.
- EPUB 편집 세 묶음을 각각 대조했다. 표지 설정 Rust 70개/Python 78개, 이미지 편집 양쪽 46개,
  행수 계산 Rust 4개/Python 25개 테스트가 통과했다. 행수 Rust 테스트는 parameterized Python 사례를 루프로 묶어
  두 reader와 10개 논리 행수 사례, 긴 chapter, ZIP fallback을 모두 검사한다. 전체 회귀는 실행하지 않았다.
- 만화 모델·관리자 조립을 단독 대조했다. Rust `comics` 6개, `comics_manager` 15개와 Python `tests/test_comics.py` 6개가 통과했다.
  Python testcontainers deprecation 경고 1건이 있었다. 전체 회귀는 실행하지 않았다.
- `tests/books` fixture에서 Python reader golden을 만들어 ignored doc_reader corpus 비교를 활성 실행했다.
  TXT 48개, RTF 6개, HTML 2개, EPUB 12개, PDF 5개가 일치했다.
  HTML sample에서 암묵적 head 앞 개행 하나가 빠지는 차이를 재현했다. 회귀 테스트를 먼저 추가해 RED를 확인하고
  `html_head_gap`이 암묵적 body 앞 공백도 보존하도록 수정했다. markup target 4개와 HTML corpus 비교가 통과했다.
  생성 golden은 임시 파일이며 운영 corpus parity와 전체 회귀는 남아 있다.
- HWP preview의 LibreOffice 빈 출력 fallback을 Rust와 Python에서 각각 단독 검증했다.
  Rust `book_manager_hwp_preview` 1개와 대응 Python pytest 1개가 통과했다.
  Python에서 testcontainers deprecation 경고 1건이 있었고, 전체 회귀는 실행하지 않았다.
- 과거 기록의 `loader_2` 무시 테스트 수는 현재 상태와 달랐다. 실제 test listing에서 무시 0개를 확인했다.
  Rust `loader_2` 106개와 Python `tests/test_loader.py` 371개를 단독 실행해 모두 통과했다.
  Python 경고 7건(testcontainers·multiprocessing fork·runpy)이 있었고, 전체 workspace 회귀는 실행하지 않았다.
- `main_mock`도 과거 미등록 기록과 달리 현재 backend test target으로 등록되어 있다.
  Rust `main_mock` 132개와 Python `tests/test_main_mock.py` 139개를 각각 단독 실행해 통과했다.
  Python 경고 9건(testcontainers·coroutine·Starlette cookies)이 있었고, 주요 proposal 흐름의 대응 assertion도 확인했다.
  전체 workspace 회귀는 실행하지 않았다.
- Python classifier exporter의 파일 포맷·joblib 변환 기준을 Rust 구현과 대조했다.
  Python `tests/test_classifier_export.py` 11개, Rust `classifier_model` 30개와 `classify_cli` 37개가 통과했다.
  Rust 모델 target의 실제 운영 모델 비교 2개는 corpus 경로가 없어 무시 상태다.
  Python testcontainers deprecation 경고 1건이 있었고, 전체 회귀는 실행하지 않았다.
- 분류 CLI의 기준 테스트를 모듈 단위로 대조했다. Python `tests/test_classify_cli.py` 43개와
  Rust `classify_cli` 37개가 통과했다. Rust 테스트는 이름이 다른 추가 parity assertion을 포함하므로
  개수 차이를 1:1 대응으로 해석하지 않았다. Python 경고 2건(testcontainers·runpy)이 있었고
  전체 회귀는 실행하지 않았다.
- 도서 관리 테스트 이름을 대응해 PDF 복구 경고 사례가 Rust에 빠진 것을 찾았다.
  손상된 xref PDF를 Rust `book_manager_1`에 추가해 Python 기준과 동일하게 invalid·warning·messages를 확인했다.
  Python 해당 사례와 Rust 신규 테스트가 통과했다. 같은 대조에서 찾은 빈 EPUB spine 경로와 Unicode casefold 사례는
  기존 Rust `book_preview`·`book_manager_2` 테스트로 각각 통과했다. 전체 회귀는 실행하지 않았다.
- 도서 관리 모듈 테스트를 각각 단독 실행했다. Python `tests/test_book_manager.py` 331개가 통과했다.
  Rust `book_manager_1` 145개 통과·12개 ES 통합 ignore, `book_manager_2` 156개,
  `book_manager_gaps` 48개, `book_manager_classifier` 6개가 통과했다.
  Python 경고 23건(testcontainers·multiprocessing fork)이 있었고 전체 workspace 회귀는 실행하지 않았다.
- bookstore 계열 테스트를 모듈별로 대조했다. Python/Rust 통과 수는 `bookstore` 97/111,
  cache 19/20, policy 26/26, `cover_sources` 48/27이다. 테스트 이름을 비교해 동작 사례 누락은 없었고,
  Rust는 일부 Python parameterized 사례를 루프로 묶었다. Rust bookstore의 추상 기본 메서드 테스트 1개는
  적용할 Rust 동작이 없어 ignore 상태다. Python 경고는 각 모듈에서 testcontainers deprecation 1건씩이었다.
  전체 workspace 회귀는 실행하지 않았다.
- PDF 편집의 예상 밖 저장 panic이 JoinError 상세 내용을 노출하는 차이를 찾았다.
  Python 기준은 예외 종류만 반환한다. Rust 회귀 테스트를 먼저 추가해 RED를 확인하고
  panic 상세를 `PDF 페이지 삭제 실패 (RuntimeError)`로 감췄다.
  Rust `pdf_delete_pages_gaps` 11개와 Python `tests/test_pdf_delete_pages_gaps.py` 17개,
  Rust corpus reader 23개와 Python `tests/test_classifier_reader.py` 29개가 통과했다.
  Python 각 모듈에 testcontainers deprecation 경고 1건이 있었다. 전체 회귀는 실행하지 않았다.
- PDF 편집 본 테스트도 개별 실행했다. Rust `pdf_delete_pages` 71개와
  Python `tests/test_pdf_delete_pages.py` 128개가 통과했다.
  Python testcontainers deprecation 경고 1건이 있었다. 전체 회귀는 실행하지 않았다.
- 분류 제안 상태의 누락 heartbeat 처리를 Python 원본과 대조했다. Python은 갱신 시각이
  없거나 잘못된 상태를 stale로 보지만 Rust가 SQL 나이 `NULL`을 0초로 처리했다.
  회귀 테스트에서 RED를 확인한 뒤 누락 나이를 stale로 바꿨다. Rust `category_mapping` 78개와
  Python `tests/test_main.py -k TestStaleRunningStatus` 6개가 통과했다.
  Python testcontainers deprecation 경고 1건이 있었다. 전체 회귀는 실행하지 않았다.
- `doc_reader`의 ignored Python corpus 비교 5개를 저장소 fixture로 활성 실행했다.
  TXT 48개, RTF 6개, HTML 2개, EPUB 12개, PDF 5개에서 요약·줄 수·쪽 수가 모두 일치했다.
  RTF 비교는 33.40초 걸렸다. golden JSONL은 `/tmp`에만 두었고 저장소에는 추가하지 않았다.
  workspace 전체 회귀는 실행하지 않았다.
- 분류 CLI 활성 target을 확인했다. Rust `classify_cli` 37개와 Python `tests/test_classify_cli.py` 43개가 통과했다.
  `tests/rust/backend/classify_cli*.rs`의 ignored 초안은 Cargo에 등록되지 않아 실행 대상이 아니다.
  Rust `classifier_model`의 운영 모델 parity 2개는 실제 모델 export 경로가 없어 계속 ignore 상태다.
  전체 회귀는 실행하지 않았다.
- manager facade의 `BookManagerApi` 41개 메서드는 BookManager와 ComicsManager 구현이 모두 덮는다.
  classifier service 대응 테스트도 Rust와 Python 각각 10개가 통과했다.
  Python testcontainers deprecation 경고 1건이 있었고 전체 회귀는 실행하지 않았다.
- client error log endpoint를 Python 기준과 대조했다. 익명·인증 사용자, 필드 검증·상한,
  단일 줄 로그 정리와 stack 개행 escape를 Rust/Python 각각 8개 테스트로 확인했다.
  Rust `client_error_log` 8개와 Python `tests/test_client_error_log.py` 8개가 통과했다.
  Python `pytest` 실행 래퍼는 global pytest 경로 불일치로 실패해 `uv run python -m pytest`로 실행했다.
  testcontainers·Starlette 경고 2건이 있었고 전체 회귀는 실행하지 않았다.
- 공통 유틸의 Python 계약과 Rust 테스트 묶음을 추가 대조했다. Rust/Python 개별 결과는
  ISBN 38/50, parser timeout 12/12, file time 10/10, stat 12/5 통과다.
  각 Rust 테스트가 Python 기준 사례를 합쳐 검증하는 구조를 확인했고 동작 차이는 찾지 못했다.
  Python 경고는 testcontainers deprecation 3건과 multiprocessing fork deprecation 4건이다.
  전체 회귀는 실행하지 않았다.
- HWP 3.x parser와 변환표를 Python 기준과 대조했다. Rust parser 25개, table decoder 4개,
  Python `tests/test_hwp3_parser.py` 97개가 통과했다. Rust는 전체 HNC 변환표 비교와 Python의
  경계 사례를 묶어 검증한다. 동작 차이는 찾지 않았고 전체 회귀는 실행하지 않았다.
- 실행 경로를 다시 확인했다. Docker CMD와 `backend/startup.sh`는 Rust `backend serve`를 실행하고,
  Kubernetes backend container도 image 기본 실행 경로를 쓴다. Docker의 Python 단계는 PDFium library를
  가져오는 빌드 보조이며 앱을 실행하지 않는다. `pyferrite/scripts/crossvalidate.py`는 Python 수치 라이브러리와
  포맷 호환성을 대조하는 개발 검증 도구로만 bootstrap에서 호출된다. 전체 기능 동등성 감사는 계속 남아 있다.
- Python OpenAPI에서 118개 route/method 쌍과 인증 dependency를 추출해 Rust `route_table()`과 대조했다.
  Rust의 GET categories/rename·delete 정적 경로 4개는 Axum 우선순위 보정이며 같은 category handler를 호출한다.
  테스트 manifest와 `main_route_parity` target을 추가했고 route, method, `require_auth`·`require_admin`·`optional_auth`가 모두 일치했다.
  scanner 제외 전체 회귀는 실행하지 않았다.
- Python OpenAPI의 JSON 요청 계약도 테스트했다. 29개 필수 body schema는 Rust에서 빈 객체를 422로 거부하고,
  43개 OpenAPI body 예시는 Rust 역직렬화 단계에서 422가 발생하지 않았다.
  EPUB 이미지 출처의 상호 배타 검증은 OpenAPI에 표시되지 않아 예시에서 URL을 명시했다.
  `main_schema_parity` 두 테스트가 통과했고 전체 회귀는 실행하지 않았다.
  fixture 43개는 Python의 실제 Pydantic 요청 모델에서도 모두 검증됐다.
- EPUB 손상 표지 복구 묶음을 기준 테스트와 대조했다. Rust `cover_repair` 5개와 Python
  `tests/test_cover_repair.py` 8개가 통과했다. Rust는 parameterized Python 사례 일부를 반복문으로 묶어 검증한다.
  Python testcontainers deprecation 경고 1건이 있었고 전체 회귀는 실행하지 않았다.
- 외부 표지 URL 다운로드의 SSRF 방지·redirect 재검증·stream byte 제한을 대조했다.
  Rust `direct_cover_url` 12개와 Python `tests/test_direct_cover_url.py` 26개가 통과했다.
  Rust는 Python parameterized 사례 일부를 반복문으로 묶었고, response header·body idle timeout 사례를 추가 검증했다.
  Python testcontainers deprecation 경고 1건이 있었고 전체 회귀는 실행하지 않았다.
- 표지 thumbnail·캐시 무효화·viewer 접근 제어 endpoint를 다시 대조했다. Rust `cover` 23개와
  Python `tests/test_cover.py` 32개가 통과했다. Python parameterized 입력 일부는 Rust 반복문으로 묶어 검사한다.
  Python testcontainers와 Starlette deprecation 경고 2건이 있었고 전체 회귀는 실행하지 않았다.
- 인증·JWT·쿠키·로그인 세션·refresh rotation 묶음을 양쪽에서 단독 실행했다.
  Rust `auth` 96개와 Python `tests/test_auth.py` 96개가 통과했다.
  Python에서 testcontainers·Starlette·짧은 테스트 JWT key·httpx 쿠키 경고 75건이 나왔다.
  전체 회귀는 실행하지 않았다.
- EPUB 미리보기·다운로드·검증 경로를 다시 단독 대조했다. Rust `book_preview` 106개와
  Python `tests/test_book_preview.py` 105개가 통과했다. Python 경고 3건(testcontainers, Starlette,
  미대기 AsyncMock coroutine)이 있었고 전체 회귀는 실행하지 않았다.
- 도서 관리의 오류·경계·비동기 callback 묶음을 다시 대조했다. Rust `book_manager_gaps` 48개와
  Python `tests/test_book_manager_gaps.py` 50개가 통과했다. Python parameterized 사례를 Rust 반복문과 비교했고,
  testcontainers deprecation 경고 1건이 있었다. 전체 회귀는 실행하지 않았다.
- 분류기 서비스의 ES 미적재·stat 실패·빈 본문·reader 오류·모델 부재 경로를 다시 대조했다.
  Rust `classifier_service`와 Python `tests/test_classifier_service.py`가 각각 10개 통과했다.
  Python testcontainers deprecation 경고 1건이 있었고 전체 회귀는 실행하지 않았다.
- 분류기 빈 모델·파일 정리·EPUB 메타데이터 경계 사례를 대조했다. Rust `classifier_gaps` 9개와
  Python `tests/test_classifier_gaps.py` 8개가 통과했다. Python보다 Rust에 title matching 경계 사례가 추가돼 있다.
  Python testcontainers deprecation 경고 1건이 있었고 전체 회귀는 실행하지 않았다.
- classifier config·corpus·feature vector·학습·평가·model save/load 기준 테스트를 재실행했다.
  Rust `classifier` 64개와 Python `tests/test_classifier.py` 71개가 통과했다.
  테스트 함수 수는 동일하며 Python parameterized 입력이 추가 사례를 만든다.
  Python testcontainers deprecation 경고 1건이 있었고 전체 회귀는 실행하지 않았다.
- Python exporter와 Rust 모델 loader/importer를 대조했다. Python `tests/test_classifier_export.py` 11개,
  Rust `classifier_model` 30개가 통과했고 운영 모델 parity 2개는 운영 export 경로가 없어 무시됐다.
  Rust legacy joblib importer는 tiny fixture를 Python export golden과 맞췄다.
  Python testcontainers deprecation 경고 1건이 있었고 전체 회귀는 실행하지 않았다.
- 도서 분류 규칙·파일명·서점 투표와 Python 생성 parity fixture를 대조했다.
  Rust `book_classifier` 43개와 `book_classifier_parity` 9개, Python `tests/test_book_classifier.py` 및
  parity fixture 검증 119개가 통과했다. parameterized 사례와 fixture 내부 반복 대조가 있어 개수는 1:1이 아니다.
  Python testcontainers deprecation 경고 1건이 있었고 전체 회귀는 실행하지 않았다.
- classifier calibration과 corpus 수집·publisher cache를 양쪽에서 단독 검증했다.
  Rust `classifier_calibration`·`classifier_corpus` 32개와 Python 대응 테스트 32개가 통과했다.
  Rust corpus 테스트는 scroll JSONL streaming과 cache 재사용도 확인했다.
  Python testcontainers deprecation 경고 1건이 있었고 전체 회귀는 실행하지 않았다.
- ES manager의 검색·scroll·갱신·삭제 및 gap 테스트를 순차 실행했다. Rust `es_manager`·`es_manager_gaps` 100개와
  Python 대응 테스트 103개가 통과했다. Rust HTTP transport 12개와 Elasticsearch Docker 실서비스 사례 1개도 통과했다.
  Python testcontainers deprecation 경고 1건이 있었다. 전체 회귀는 실행하지 않았다.
- 서점 HTML 파싱·검색·HTTP session을 기준 테스트와 대조했다. Python `tests/test_bookstore.py` 97개,
  Rust backend `bookstore` 111개와 공통 `bookstore` crate 14개가 통과했다.
  Rust의 추상 기본 메서드 테스트 1개는 대응 동작이 없어 ignore 상태다.
  Python testcontainers deprecation 경고 1건이 있었고 전체 회귀는 실행하지 않았다.
- 서점 결과 cache와 분류 policy도 개별 대조했다. Rust/Python 통과 수는 cache 20/19,
  policy 26/26이다. cache에는 Python 기준에 없는 truthy category 입력 사례를 Rust에서 추가 검증했다.
  Python 각 테스트 파일에서 testcontainers deprecation 경고 1건이 있었고 전체 회귀는 실행하지 않았다.
- mock API 통합 흐름을 재실행했다. Rust `main_mock` 132개와 Python `tests/test_main_mock.py` 139개가 통과했다.
  Python 경고 10건(testcontainers·Starlette·미대기 coroutine·httpx cookie)이 있었고 전체 회귀는 실행하지 않았다.
- API 관리 경계·오류 경로도 단독 대조했다. Rust `main_gaps` 24개와 Python `tests/test_main_gaps.py` 11개가 통과했다.
  Rust는 Python 사례 외에도 proposal job 상태·queue·진행 저장 실패 경로를 검증한다.
  Python testcontainers와 Starlette 경고 2건이 있었고 전체 회귀는 실행하지 않았다.
- 만화 모델과 manager 조립 target을 재실행했다. Rust `comics` 6개·`comics_manager` 15개,
  Python `tests/test_comics.py` 6개가 통과했다. manager 전용 Python 테스트 파일은 없으며 API 동작은 `test_main.py` 대상이다.
  Python testcontainers deprecation 경고 1건이 있었고 전체 회귀는 실행하지 않았다.
- 일반 API 통합 테스트를 다시 실행했다. Rust `main_api` 51개가 통과하고 ES/MySQL 필요 사례 35개는 무시됐다.
  Python `tests/test_main.py` 95개가 통과했으며 testcontainers·Starlette·BeautifulSoup·httpx deprecation warning 346건이 있었다.
  전체 회귀는 실행하지 않았다.
- bulk rename API를 별도 대조했다. Rust `main_bulk_rename` 16개와 Python `tests/test_main_bulk_rename.py` 21개가 통과했다.
  Python parameterized 경로·이름 입력으로 실행 사례 수가 더 많다.
  Python testcontainers와 Starlette deprecation 경고 2건이 있었고 전체 회귀는 실행하지 않았다.
- 조회 이력 API의 server-side snapshot·인증·열람 실패·기록 실패 처리를 검증했다.
  Rust `view_history_api` mock 17개와 MySQL end-to-end 2개, Python `tests/test_view_history_api.py` 19개가 통과했다.
  Python testcontainers와 Starlette deprecation 경고 2건이 있었고 전체 회귀는 실행하지 않았다.
- 조회 이력 저장소의 MySQL 기록·정렬·유저 격리·보존 한도를 단독 대조했다.
  Rust 활성 DB 테스트 13개와 Python `tests/test_view_history_store.py` 15개가 통과했다.
  Python testcontainers deprecation 경고 1건이 있었고 전체 회귀는 실행하지 않았다.
- refresh-token 저장소의 회전·재사용 감지·세션 집계·migration을 대조했다.
  Rust 일반 및 활성 MySQL 테스트 66개와 Python `tests/test_refresh_token_store.py` 70개가 통과했다.
  Python testcontainers deprecation 경고 1건이 있었고 전체 회귀는 실행하지 않았다.
- 도서 관리 본 테스트를 재실행했다. Rust `book_manager_1` 일반 145개와 isolated ES 통합 12개,
  `book_manager_2` 156개, Python `tests/test_book_manager.py` 331개가 통과했다.
  Python 경고 361건(BeautifulSoup·multiprocessing fork·testcontainers)이 있었고 전체 회귀는 실행하지 않았다.
  `book_manager_1` ES 테스트는 `tests/books/_epub`, `_txt`를 scratch library로 복사하고 disposable Elasticsearch 8.15.0에 연결해 활성 실행했다.
- Loader 묶음의 Rust target 네 개와 Python 기준 두 파일을 단독 실행했다.
  Rust `loader_1` 178개, `loader_2` 106개, `loader_gaps` 1개, `loader_hwp3` 63개와
  Python `tests/test_loader.py`·`tests/test_loader_gaps.py` 372개가 통과했다.
  Rust/Python 테스트 구성은 parameterization과 분리 단위가 달라 count만으로 1:1 대응을 판단하지 않았다.
  Python warning 52건(testcontainers·BeautifulSoup·multiprocessing fork·runpy)이 있었고 전체 회귀는 실행하지 않았다.
- `doc_reader` TXT·EPUB·RTF·HTML·PDF corpus를 현재 Python reader 출력과 다시 대조했다.
  Rust의 ignored corpus target 다섯 개가 모두 통과했고, 48 TXT·12 EPUB·6 RTF·2 HTML·5 PDF 입력을 확인했다.
  별도 fast PDF page-count corpus 5개도 통과했다. 전체 회귀는 실행하지 않았다.
- API 전수 parity의 개별 target을 실행했다. Rust route·method 등록은 Python OpenAPI 경로 전부와 일치했고,
  필수 body field 거부와 Python JSON body 예시 역직렬화 테스트 2개도 통과했다.
  `main_route_parity` 1개, `main_schema_parity` 2개가 통과했으며 전체 회귀는 실행하지 않았다.
- Rust 분류 CLI의 model info 경로에서 exported model 표시와 missing model 안내를 단독 검증했다.
  `classify_cli`의 관련 테스트 1개가 통과했다. 전체 회귀는 실행하지 않았다.
- 분류 CLI 활성 target 전체를 양쪽에서 단독 실행했다. Rust `classify_cli` 37개와
  Python `tests/test_classify_cli.py` 43개, `tests/test_classify_cli_gaps.py` 9개가 통과했다.
  Rust target은 gap 사례를 포함하며 테스트 분리는 parameterization 차이가 있어
  단순 개수로 1:1을 판단하지 않았다. Python testcontainers deprecation과 runpy warning 2건이 있었다.
  `.venv/bin/pytest` entrypoint는 global pytest와 섞여 import 오류가 났고,
  `uv run --no-sync python -m pytest`로 기준 테스트를 실행했다. 전체 회귀는 실행하지 않았다.
- 실제 79개 category classifier export와 Python golden 266건을 대조했다. Rust 직접 로드 parity에서
  최대 score 오차 1.634e-7, confidence 오차 1.045e-8, 순위·tie·사유·category mismatch 0건이었다.
  운영 `model.joblib`도 Rust importer로 임시 경로에 변환한 뒤 같은 golden과 일치했다.
- 테스트 inventory를 다시 집계했다(테스트 실행 아님). `uv run --no-sync python -m pytest --collect-only -q`는
  3,048개를 수집했다. scanner 제외 Rust workspace의 Cargo `--list`는 3,098개를 등록했고,
  `--ignored --list`는 그중 96개를 표시했다. 따라서 active는 3,002개다.
  Cargo 등록 대상이 아닌 `tests/rust/backend/classify_cli.rs`·`classify_cli_gaps.rs`에는 테스트 함수 초안 45개
  (그중 `ignore` 27개)가 더 있다. 두 파일은 현재 API와 맞지 않아 등록 대상에서 제외했으며, 위 Cargo 합계에는 넣지 않았다.
  따라서 현재 Rust source test inventory는 등록 3,098개와 미등록 초안 45개로 구분한다.
  `main_api`는 86개 중 active 59개, ignored 27개다.
  Python 전체 수집과 Rust scanner 제외 집계는 범위가 완전히 같다고 보장되지 않으며,
  parameterization·Rust 전용 사례·테스트 묶음 차이도 있어 count 비율을 함수 parity로 간주하지 않는다.
  이전 Python 기준 수 2,814는 오래된 값이었다. `.venv/bin/pytest` 직접 실행은 global pytest와의
  import 불일치로 실패했지만, `uv run --no-sync python -m pytest` 수집은 성공했다.
- `doc_reader`의 Python golden corpus를 repository-relative fixture로 고정하고 기존 ignored 테스트 5개를 활성화했다.
  본문 원문 대신 FNV-1a summary checksum과 line/page/raw 길이만 저장해 저작물 복제를 피했다.
  TXT 48, EPUB 12, RTF 6, HTML 2, PDF 5개 파일을 검증했고, PDF fast page count도 대조했다.
  다섯 corpus 테스트가 개별 통과했다. `cargo fmt --all -- --check`와 `git diff --check`도 통과했고,
  전체 regression은 실행하지 않았다. Rust ignored 수는 101에서 96으로 줄었다.
- 운영 facade의 기본 미구현 구현을 대조했다. `BookManagerApi` 41개 메서드는 `BookManager`와
  `ComicsManager`가 각각 전부 재정의한다. `UnavailableBookManager`·`UnavailableBookAccess`는
  설정 누락 fallback이고, 운영 `RuntimeBookAccess`도 별도로 연결돼 있다.
- refresh·auth/me·logout endpoint 테스트를 service-free active 사례로 정리했다. Python `_issue_auth_tokens`처럼
  Rust도 `AuthRefreshStore` 대역으로 저장소 회전을 주입했고, 만료·access token·비인가 이메일·쿠키 누락 사례는
  ES/MySQL 초기화를 제거했다. Rust `main_api` 8개와 대응 Python `tests/test_main.py` 8개가 통과했다.
  auth 범위 밖의 ES/MySQL API 통합 사례는 기존대로 ignored다.
  Python testcontainers·Starlette cookie deprecation warning이 발생했다. 전체 회귀는 실행하지 않았다.
- ISBN 추출의 Python 호출 경로를 대조했다. `utils/loader.py`는 Python 원본 `utils/isbn.py`를 사용하지만,
  운영 Rust loader는 `isbn` crate를 직접 호출한다. Rust `cargo test -p isbn --locked` 38개와
  Python `tests/test_isbn.py` 50개가 각각 통과했다. 테스트 단위 분할은 달라 개수로 1:1 parity를 주장하지 않으며,
  기존 EPUB·PDF·DJVU·HWP·TXT 경로의 추가 미구현은 발견하지 않았다. 전체 회귀는 실행하지 않았다.
- Python classifier exporter를 fixture 생성·기준 구현으로 유지하는 범위를 확인했다. Rust exporter/loader는
  저장 형식과 legacy joblib import를 별도 테스트로 검증한다. `classifier_model` 30개 통과·운영 모델 2개 무시,
  Python `tests/test_classifier_export.py` 11개 통과다. 두 실제 운영 모델 테스트는 모델 경로가 없어 실행하지 않았다.
  전체 회귀는 실행하지 않았다.
- 실제 Elasticsearch 컨테이너로 Rust `docker_es_manager_flow_against_real_elasticsearch`를 실행해 통과했다.
  Python `tests/test_es_manager.py` 95개도 단독 통과했다(기존 testcontainers deprecation warning 1건).
  Elasticsearch 연결·mapping·문서 처리에서 추가 동작 차이는 발견하지 않았다. 전체 회귀는 실행하지 않았다.
- Docker MySQL 통합 검증을 진행했다. Rust `refresh_token_store`의 ignored 3개와 `db`의 ignored 8개가 통과했고,
  Python `tests/test_refresh_token_store.py` 70개도 통과했다(기존 testcontainers deprecation warning 1건).
  Rust의 MySQL 집계·legacy migration·cursor/transaction adapter를 확인했다. 전체 회귀는 실행하지 않았다.
- classify-proposal MySQL 통합 ignored 테스트를 실행했다. Rust items 16개와 status 11개,
  Python `tests/test_classify_proposal_items_db.py`·`tests/test_classify_proposal_status_db.py` 32개가 통과했다.
  schema/migration, payload·status 갱신, content type 분리, 동시 시작·적용 lock을 대조했다.
  Python testcontainers deprecation warning 1건이 있었고 전체 회귀는 실행하지 않았다.
- 실제 운영 분류기 artifact를 사용해 이전 ignored parity 테스트 둘을 실행했다. export 79개 category의
  sklearn golden 비교와, 운영 `model.joblib`을 Rust CLI로 import한 결과의 Python golden 비교가 통과했다.
  두 비교 모두 952,717 features에서 score/confidence 오차 기준과 rank·category·reason 일치를 만족했다.
  import 산출물은 `/tmp` 임시 경로에 만들었다. 전체 회귀는 실행하지 않았다.
- 저장소 HWP fixture로 BookManager preview의 변환 빈 결과 응답을 비교했다. Rust target 1개와
  Python `tests/test_loader.py::TestBookManagerPreviewEmptyContent` 1개가 통과했다.
  전체 회귀는 실행하지 않았다.
- Python 학습 테스트의 95% 목표 정확도 경로를 Rust parity에 추가했다. `trained_95` fixture의 report,
  결정 점수, calibrated threshold/target을 대조한다. 새 Rust assertion은 첫 실행부터 통과했고,
  `classifier_training` 5개와 Python 대응 테스트 1개가 통과했다. fmt 검사도 통과했다.
  운영 corpus 학습 parity와 메모리·시간 실측은 별도 잔여 작업이다.
- classifier confidence calibration의 Python/Rust 전용 target을 나란히 실행했다.
  Rust `classifier_calibration` 18개와 Python `tests/test_classifier_calibration.py` 18개가 통과했고,
  추가 동작 차이는 발견하지 않았다. 전체 회귀는 실행하지 않았다.
- classifier 본체의 구성·문서 수집·vectorizer·학습·spill·추론 Rust target 64개와
  Python `tests/test_classifier.py` 실행 71개가 통과했다. Python parameterization과 Rust 내부 사례 묶음이 달라
  단순 개수는 1:1 대응을 뜻하지 않는다. 전체 회귀는 실행하지 않았다.
- classifier 경계 사례 Rust 9개와 Python `tests/test_classifier_gaps.py` 8개가 통과했다.
  Rust 전용 EPUB malformed metadata 방어 검증 1개를 포함하며 추가 parity 차이는 찾지 못했다.
  전체 회귀는 실행하지 않았다.
- bookstore policy 테스트를 나란히 실행했다. Rust target 26개와 Python `tests/test_bookstore_policy.py` 26개,
  auth/bookstore gap Rust·Python 테스트 각 2개가 통과했다. 경계 quantile, keyword precedence, cache fallback,
  Kyobo fallback link cap 동작에서 차이는 찾지 못했다. 전체 회귀는 실행하지 않았다.
- client error log route는 Rust·Python 테스트가 각각 8개 통과했다.
  입력 검증, 인증·익명 접근, 줄바꿈 정리, payload 크기 제한을 확인했다.
  Python testcontainers와 Starlette cookie warning 2건이 있었고 전체 회귀는 실행하지 않았다.
- classify-proposal의 동기 DB flush가 요청 runtime을 막지 않는지 확인했다. Python 대응 테스트는
  `asyncio.to_thread` 사용을 통과했고, Rust HTTP facade 테스트는 `TaskRunner` worker thread에서 flush됨을 확인한다.
  Rust `main_mock` 133개가 통과했다. `~/bin/test_runner_rs -f tests/rust/backend/main_mock.rs`도
  의존 모듈 8개를 분석하고 해당 target만 재실행했다. 보고된 `routes_books/admin.rs` coverage는
  line 86.18%, function 72.73%, region 88.52%다. 전체 회귀는 실행하지 않았다.
- 같은 변경에 `test_runner_rs -f tests/rust/backend/main.rs`를 적용해 의존 모듈 16개를 분석했다.
  영향 target `main_api`는 59개 통과·27개 ignored였다. route별 coverage는 `routes_books/admin.rs`
  line 86.18%, function 72.73%; `routes_books/browse.rs` line 84.14%, function 66.67%였다.
  workspace 전체 회귀는 실행하지 않았다.
- `classifier_training`에 95% parity test를 추가한 뒤 scanner 제외 Cargo inventory를 다시 셌다.
  등록 3,099개, active 3,003개, ignored 96개다. Cargo target에 없는 CLI 초안 45개는 합계에서 제외했다.
- browse 경로 응답의 `file_path` 동작을 Python 원본과 대조했다. Python은 절대 경로를 그대로 반환하지만
  Rust handler는 `path_prefix`를 제거하고 있었다. 회귀 테스트가 기존 Rust 동작에서 실패하는 것을 확인한 뒤
  축약 로직을 제거했다. Rust browse 관련 6개와 viewer 경로 테스트 15개, Python browse 7개가 통과했다.
  `test_runner_rs -f tests/rust/backend/main_mock.rs`는 의존 모듈 8개를 분석하고 `main_mock` 134개를 통과시켰다.
  browse handler coverage는 line 74.51%, function 60.00%, region 64.71%다. 전체 회귀는 실행하지 않았다.
- `cover_status` HTTP 테스트가 manager root 기준 상대 경로를 기대해 `test_runner_rs`에서 실패했다.
  Python route가 `book.dict()` 결과를 그대로 반환하는 점을 확인해 mock dict의 절대 경로 기대값으로 맞췄다.
  Rust `cover_status` 22개와 Python `tests/test_cover_status.py` 17개가 통과했다.
  `test_runner_rs -f tests/rust/backend/cover_status.rs`는 의존 모듈 5개를 분석했다.
  `cover_status.rs` coverage는 line 71.67%, function 38.10%, region 73.98%이며,
  `book_manager/uncovered.rs`는 각각 93.21%, 100.00%, 97.22%다. 전체 회귀는 실행하지 않았다.
- loader CLI의 잘못된 옵션 종료 코드를 Python과 대조했다. 실제 Python `print_usage()`는 `SystemExit(0)`을
  발생시키지만 Rust는 2를 반환했다. Rust 회귀 테스트에서 차이를 재현하고 종료 코드를 0으로 맞췄다.
  Python의 기존 대역 테스트와 실제 exit 테스트 2개, `test_runner_rs -f tests/rust/loader/loader_2.rs`의
  관련 target 106개가 통과했다. loader 의존 모듈 13개를 분석했으며 CLI coverage는 line 81.05%,
  function 63.33%, region 80.46%다. 전체 회귀는 실행하지 않았다.
- classifier 학습 묶음의 parity를 다시 확인했다. `test_runner_rs`로 classifier target 64개와
  `classifier_training` 5개가 통과했고, Python `tests/test_classifier.py` 71개도 통과했다.
  학습·features·model 의존 모듈 3개를 분석했다. `training.rs` coverage는 line 93.15%,
  function 93.33%, region 94.01%다. Rust/Python 테스트 분할이 달라 개수만으로 1:1 대응을 주장하지 않는다.
  전체 회귀는 실행하지 않았다.
- 만화 디렉터리 정규화 묶음을 Python 생성 golden과 다시 대조했다. Rust `names` 10개, `plan` 26개,
  CLI `bin` 3개가 통과했다. `test_runner_rs` 기준 `names.rs` coverage는 line 97.14%, function 95.51%,
  region 97.74%이고, `plan.rs`는 94.43%, 97.73%, 96.33%다. CLI는 line 90.48%, function 84.38%,
  region 92.90%다. 이 묶음에는 별도 Python pytest 파일이 없고 기존 Python-generated parity fixture를 사용한다.
  전체 회귀는 실행하지 않았다.
- main API의 외부 ES/MySQL ignored 테스트 27개를 disposable MySQL과 Nori 포함 Elasticsearch에서 활성 실행해 통과했다.
  `test_runner_rs -f tests/rust/backend/main.rs`의 active 테스트 59개도 통과했고, 의존 모듈 16개를 분석했다.
  Python `tests/test_main.py` 95개도 통과했다. `admin.rs` coverage는 line 86.18%, function 72.73%,
  region 88.52%이고 `catalog.rs`는 92.11%, 91.67%, 92.78%다.
  첫 시도에서 기본 ES image의 Nori 미설치가 드러나 Nori 포함 image로 바꿨다. Python testcontainers 경고 7건이 있었다.
  `test_update_book`의 이동 후 파일 잔존을 통합 테스트 assertion으로 재현하고 Defer cleanup이 목적지까지 지우도록 수정했다.
  단일 재현 테스트와 수정 뒤 ignored 27개 전체가 통과했고, fixture 디렉터리 잔여 파일도 제거했다.
  전체 workspace 회귀는 실행하지 않았다.
- ISBN·parser timeout·file time·통계 공통 crate를 `test_runner_rs`로 개별 검증했다.
  의존 모듈은 각각 2, 1, 1, 1개로 분석됐고 Rust 테스트 72개와 Python 대응 테스트 77개가 통과했다.
  crate별 coverage는 ISBN line/function/region 95.31/92.41/93.65%, file_time 100/100/96.60%,
  parser_timeout 96.49/100/96.49%, stat 89.69/82.61/88.84%다. 전체 workspace 검사는 실행하지 않았다.
- `main_mock.rs`에서 이전 내부 router 접근 때문에 비활성이던 제안 flush 경계를 현재 HTTP/TaskRunner 경로로 옮겼다.
  신규 25개 항목의 저장 batch가 `10+10+5`이고 마지막 항목까지 `ready` 상태에 반영되는 테스트를 추가했다.
  RED 실행 없이 현재 구현에서 통과했다. `main_gaps` 25개, Python `tests/test_main_gaps.py` 11개가 통과했고,
  `test_runner_rs -f tests/rust/backend/main_gaps.rs`는 의존 모듈 4개를 분석했다.
  coverage는 156/185 target만 측정한 partial 보고라 전체 구현율로 해석하지 않는다.
- 실제 ES에서 분류 corpus를 `/tmp`로 수집했다(222,348건, 87개 category). 첫 Python 학습은 tmpfs spill이
  5.7 GB까지 차 SIGBUS로 끝나 즉시 spill을 정리했다. corpus를 ext4로 옮긴 재시도는 66분 뒤에도 fit 중이었고,
  사용자 메모리 우려에 따라 중단했다(최대 RSS 13.75 GB). 학습 산출물은 없으며 sklearn parity·성능 비교는 미완료다.
  corpus JSONL은 `/home/terzeron/.cache/tm_dev/classifier-corpus.jsonl`에 있다. 다음 실측은 sample과 `n_jobs=1` 설정으로 시작한다.
- OOM 위험 때문에 대규모 학습은 재개하지 않았다. Rust `classify_cli`에 `collect --with-publisher`의 인접 캐시 생성·재사용
  통합 테스트를 추가했다. Rust `cli` target 38개와 Python `tests/test_classify_cli.py` 43개가 통과했고,
  rustfmt·diff 검사를 통과했다. 전체 회귀는 실행하지 않았다.
- `book_manager` 경계 묶음의 Python 테스트 함수 47개(실행 50건)와 Rust target 48개를 대조했다.
  대응 Python `tests/test_book_manager_gaps.py` 50건과 Rust `book_manager_gaps` 48개가 모두 통과했다.
  Python parameterization 때문에 실행 건수가 다르며, 각 Python 테스트 함수에 Rust 대응 사례가 있다.
  전체 회귀는 실행하지 않았다.
- Rust classifier의 spill 학습 중 holdout 원문을 미리 복제하던 순서를 바꿨다. SVM fitting 후 sparse 행렬을 해제하고
  holdout을 복제해 fitting 동안 중복 원문을 보유하지 않는다. 변경 전 spill 회귀 기준선 1개와 변경 후 spill 테스트 2개,
  Rust classifier target 64개, training parity 5개, Python `tests/test_classifier.py` 71개가 통과했다.
  의존 CLI target 38개와 `cargo clippy -p backend --test classifier --locked -- -D warnings`도 통과했다.
  전체 corpus RSS 재측정은 OOM 위험 때문에 실행하지 않았다.
- 공통 corpus reader·경로 유틸과 문서 reader를 직접 실행해 coverage 범위를 다시 확인했다.
  corpus reader 23개, 공통 유틸 6개, detect-mojibake 32개, cluster utility 33개,
  doc_reader formats 8개와 EPUB 7개가 통과했다. Python classifier reader 대응 29개도 통과했다.
  직접 의존 target 기준 reader coverage는 line/function/region 96.67/97.37/97.55%,
  공통 util은 97.10/100/100%, doc_reader EPUB은 91.32/88.61/89.02%다.
  전체 회귀는 실행하지 않았다.
- EPUB 표지 변경에서 `BookLookup`을 주입하지 않았을 때 ES `get_book` fallback을 타는
  Rust 테스트를 추가했다. `test_runner_rs -f tests/rust/backend/epub_cover_set.rs`는
  71개 테스트가 통과했고 Python `tests/test_epub_cover_set.py` 78개도 통과했다.
  Python 테스트 함수 70개 모두 Rust의 이름이 같은 대응 테스트를 가진다.
  `book_manager/cover.rs` coverage는 line/function/region 86.59/83.64/89.16%다.
  전체 회귀는 실행하지 않았다.
- classifier spill이 각 field의 sparse rows 전체를 쌓은 뒤 파일로 쓰던 경로를 행 단위 기록으로 바꿨다.
  다음 행을 생성하기 전에 이전 행이 파일에 기록되는지 확인하는 RED/GREEN 단위 테스트를 추가했다.
  classifier 64개, training 5개, backend library 단위 테스트 1개와 Backend Clippy가 통과했다.
  training target에서 `features.rs` coverage는 line/function/region 83.89/75.24/85.18%다.
  전체 corpus RSS는 재측정하지 않았다. 각 반복의 현재 document text는 일시적으로 메모리에 만든다.
- classifier의 field text 전체 복사본과 `&str` 참조 배열을 없앴다. `Vectorizer::fit_iter`가
  source document에서 text를 한 건씩 만들어 fit하고, spill row 생성 때도 한 건씩 변환한다.
  slice 기반 fit과 어휘·IDF가 같다는 테스트를 RED/GREEN으로 확인했다. classifier 64개,
  training 5개, backend library 단위 테스트 2개, Backend Clippy, rustfmt와 diff 검사가 통과했다.
  training target에서 `features.rs` coverage는 line/function/region 77.57/68.70/76.44%다.
  전체 corpus RSS와 학습 시간은 재측정하지 않았다.
- Python `tests/test_main.py`에서 Rust `main.rs`와 이름이 달랐던 9개를 의미 기준으로 대조했다.
  stale heartbeat 6개는 `category_mapping`의 DB 상태 테스트가, frontend URL 검사는 backend binary 테스트가
  다룬다. 두 factory 테스트는 Rust의 명시적 dependency injection 구조에 해당 wrapper가 없어 직접 대응하지 않는다.
  `test_runner_rs`의 `main.rs` target은 59개 통과·27개 외부 ES/MySQL 테스트 제외,
  `category_mapping.rs` 78개 통과, Python `tests/test_main.py` 95개 통과다.
  코드 변경은 없으며 전체 회귀는 실행하지 않았다.
- Python classifier corpus의 custom label regex 테스트가 Rust target에 없어 lookahead 사례를 옮겼다.
  Rust 구현은 이미 `fancy_regex`를 사용해 Python 패턴과 같은 결과를 냈다. 새 Rust 회귀 테스트를 포함해
  Rust `classifier_corpus` 15개와 Python `tests/test_classifier_corpus.py` 14개가 통과했다.
  전체 회귀는 실행하지 않았다.
- classifier export 모델 형식 묶음을 다시 확인했다. `test_runner_rs`의 `classifier_model` target은
  30개 통과·운영 모델 의존 테스트 2개 제외, Python `tests/test_classifier_export.py` 11개가 통과했다.
  workspace 전체 회귀와 실제 운영 모델 parity는 실행하지 않았다.
- loader 테스트명의 미대응 HWP3·batch metadata 사례를 Rust 소스와 대조했다. HWP 변환·재귀 제한·문자 검증과
  batch 밖 inode metadata 무시는 기존 Rust 테스트로 이미 다뤘다. Rust `loader_hwp3` 63개,
  `loader_gaps` 1개, Python의 해당 HWP3 6개와 `tests/test_loader_gaps.py` 1개가 통과했다.
  동작 변경은 없으며 전체 회귀는 실행하지 않았다.
- MySQL 의존 테스트 중 아직 활성 실행하지 않았던 backend DB wrapper 8개와 refresh-token SQL·migration 3개를
  disposable MySQL에서 직렬 실행해 모두 통과했다. workspace 전체 회귀는 실행하지 않았다.
- OOM 위험이 있는 전체 classifier 학습은 실행하지 않았다. 메모리 부담과 무관한 표지 관리 묶음을 재검증해
  Rust `cover_management` 69개와 Python `tests/test_cover_management.py` 141개가 통과했다.
  Rust 테스트는 무시 사례 없이 끝났고, Python testcontainers deprecation warning 1건이 있었다.
- Python `test_reclassify_uses_the_model_threshold_when_none_is_given`에 대응하는 Rust CLI 회귀가 없어 추가했다.
  기본 임계값과 모델 임계값을 명시한 결과가 같은지 tiny fixture로 확인했다. 추가 직후 Rust CLI target 39개와
  Python `tests/test_classify_cli.py`·`tests/test_classify_cli_gaps.py` 52개가 통과했다. 전체 corpus 학습과 workspace 회귀는 실행하지 않았다.
- CLI 옵션 대조에서 Python `--class-weight none`은 기존 config 값을 보존하지만 Rust가 `null`로 덮는 차이를 찾았다.
  tiny synthetic corpus로 먼저 실패를 재현한 뒤 해당 입력은 기존 값을 유지하도록 수정했다. Rust CLI target 40개,
  Python CLI 52개, `classify_cli` Clippy와 rustfmt가 통과했다. 전체 corpus 학습과 workspace 회귀는 실행하지 않았다.
- Python argparse는 반복된 `--char-ngram`·`--no-char-ngram` 중 마지막 값을 적용한다. Rust는 입력 순서를 무시하던 차이를
  tiny corpus RED 테스트로 재현하고 마지막 옵션을 반영하도록 수정했다. Rust CLI target 41개, Python CLI 52개,
  Python parser 순서 확인, `classify_cli` Clippy와 rustfmt가 통과했다. 전체 corpus 학습과 workspace 회귀는 실행하지 않았다.
- `--body-max-features`와 `--body-min-df`가 Python에서는 `body_word`·`body_char` 양쪽에 적용되지만 Rust는 한쪽만 바꿨다.
  설정 저장을 확인하는 tiny corpus RED 테스트로 재현하고 두 field에 모두 적용하도록 수정했다. Rust CLI target 42개,
  Python CLI 52개와 `classify_cli` Clippy·rustfmt가 통과했다. 전체 corpus 학습과 workspace 회귀는 실행하지 않았다.
- Python `bookstore-policy --reband`는 결과 파일이 없을 때 전용 안내를 내지만 Rust는 OS 오류를 노출했다.
  기존 Python 사례를 Rust 회귀로 옮겨 실패를 재현한 뒤 오류 메시지를 맞췄다. Rust CLI target 43개,
  Python CLI 52개, `classify_cli` Clippy·rustfmt가 통과했다. 전체 corpus 학습과 workspace 회귀는 실행하지 않았다.
- `--reband` 입력에 건별 결과가 없을 때 Rust 오류에 Python과 달리 파일 경로가 빠졌다. 기존 동작을 따르는
  RED 테스트를 추가해 실패를 확인하고 경로와 다음 조치를 표시했다. Rust CLI target 44개, Python CLI 52개,
  `classify_cli` Clippy·rustfmt가 통과했다. 전체 corpus 학습과 workspace 회귀는 실행하지 않았다.
- `--reband`에서 서점이 어느 확신도 구간에서도 모델보다 낫지 않을 때 Rust 출력이 Python 안내 문구와 달랐다.
  RED 회귀를 추가해 “서점이 모델을 이긴 구간이 없다” 문구를 표시하도록 수정했다. Rust CLI target 45개,
  Python CLI 52개, `classify_cli` Clippy·rustfmt가 통과했다. 전체 corpus 학습과 workspace 회귀는 실행하지 않았다.
- 기본 Rust 학습도 feature matrix를 RAM에 누적하지 않도록 `/var/tmp` disk spill을 사용한다.
  이 환경의 `/tmp`는 `tmpfs`라 기본 경로에서 제외했다. 임시 경로는 학습 종료 후 정리하고,
  사용자가 지정한 `--spill-dir`는 보존한다. 관련 단위 테스트와 CLI 45개가 통과했다.
  검증에는 tiny corpus만 썼으며, 전체 corpus 문서 자체는 RAM에 남으므로 운영 학습은 미실행이다.
- `classify_books_client` 파일 수집에서 Python `Path.is_file()`은 따라가는 symlink 파일을 Rust가 누락했다.
  symlink fixture로 RED를 재현한 뒤 지원 확장자의 symlink 파일을 포함하도록 수정했다.
  같은 묶음에서 같은 카테고리·거부 대상도 Rust 요약의 `분류 결정` 수에 포함되던 차이를 고쳤다.
  library 단위 테스트 2개, `client` 12개, `bin` 3개와 Clippy·rustfmt가 통과했다.
- Python `Path.samefile()`은 hard link 목적지를 중복 삭제에서 제외한다. Rust `process_file`은 이를 지우는 차이가 있었다.
  동일 inode fixture에서 RED를 재현한 뒤 `skipped_same_file`로 원본을 보존하도록 수정했다.
  `book_classifier` 44개와 Python `tests/test_book_classifier.py` 118개가 통과했다.
- 실행 구성도 확인했다. Backend Docker와 `startup.sh`는 Rust `backend` 바이너리를 실행하고,
  Python을 실행하는 배포 경로는 검색 결과에 없었다. PDFium 공유 라이브러리와 Office/EPUB 외부 도구는 유지한다.
- `file_time`의 statx가 Unix epoch 이전 생성 시각을 돌려줄 때 Rust가 `None`으로 버리던 차이를 찾았다.
  Python처럼 음수 시각을 변환하는 회귀를 RED로 재현하고 처리했다. Rust 11개, Python `test_file_time.py` 10개,
  Clippy와 rustfmt가 통과했다.
- `Book` 시각 파서에서 Python `datetime.fromisoformat`은 소수 초의 쉼표 구분자도 받지만 Rust는 거부했다.
  Python 런타임으로 입력 형식을 확인하고 Rust 회귀 테스트에서 실패를 재현한 뒤 쉼표를 소수점으로 정규화했다.
  `test_runner_rs -f tests/rust/backend/book.rs`의 대상 8개가 통과했다. Python 테스트는 실행 환경의
  pytest shim 오류가 있어 `.venv/bin/python -m pytest tests/test_book.py -q`로 실행해 6개가 통과했다.
- `Comics::dict`는 만화 루트 밖의 절대 `file_path`를 Rust에서 그대로 반환했지만 Python `relative_to`는 오류를 낸다.
  경로 밖 입력 회귀가 수정 전 실패하는 것을 확인하고 루트 밖 경로를 거부하도록 맞췄다.
  Rust `comics` 7개와 Python `tests/test_comics.py` 6개가 통과했다.
- 같은 경로 처리 차이를 `Book::dict`에서도 확인했다. Python은 책 루트 밖 경로를 `ValueError`로 거부하지만 Rust는
  절대 경로를 내보냈다. RED 확인 후 Rust도 루트 밖 경로를 거부하도록 수정했고, Rust `book` 9개와
  Python `tests/test_book.py` 6개가 통과했다.
- `BookstoreResponseCache` 테스트 inventory를 대조했다. Python 19개 모두 Rust에 대응하고, Rust에는 Python truthy
  비문자열 category 처리 테스트 1개가 추가로 있다. Rust cache target 20개와 Python `tests/test_bookstore_cache.py`
  19개가 통과했다. 이 묶음에서는 새 동작 차이를 찾지 못했다.
- `utils/update.py`의 ES 갱신·조회·삭제 경로와 Rust CLI target을 재검증했다. Rust target 8개와 Python
  `tests/test_update.py` 4개가 통과했다. Rust CLI는 기본 문서 ID 3384를 유지하며, 테스트 실행은 fake ES만 썼다.
- Rust `train` CLI는 holdout manifest를 만들 때 `filter_by_size`와 `split_holdout`가 문서 전체를 복제했다.
  작은 corpus에서 복제 기반 결과와 holdout key 목록이 같은지 먼저 회귀 테스트로 고정한 뒤, index 기반 key helper로
  바꿔 본문 `Value` 복제를 없앴다. 이어 `train` 내부 holdout 평가도 문서 복제 대신 참조 목록을 쓰도록 바꾸고,
  기존 holdout report와 직접 평가 결과가 같은지 확인했다. classifier training 5개, parity 12개, CLI 45개,
  Backend·CLI Clippy가 통과했다. 운영 corpus 학습은 실행하지 않았고, 전체 JSONL 적재는 여전히 남아 있다.
- `category_mapping` Python/Rust 테스트 묶음을 다시 대조했다. Rust target 78개와 Python
  `tests/test_category_mapping.py` 52개가 통과했고, 누락된 동작 사례를 찾지 못해 코드 변경은 하지 않았다.
  `test_runner_rs`가 보고한 coverage는 부분 집계라 전체 모듈 coverage로 해석하지 않았다.
- OOM 우려를 반영해 학습 프로세스 상태와 적재 경로를 확인했다. 현재 classifier 학습은 실행 중이지 않다.
  Rust는 feature matrix를 `/var/tmp`로 spill하지만 JSONL 전체를 `Vec<Value>`로 적재한다. 전체 적재 개선 전까지
  운영 corpus 학습과 전체 회귀는 실행하지 않는다.
- Python 학습은 corpus 처리·벡터화·행렬·SVM 단계에서 RSS를 기록하지만 Rust에는 해당 로그가 없었다.
  Rust CLI 적재 전후와 분할·벡터화·SVM 단계에도 익명/파일 RSS와 행렬 크기를 기록하도록 맞췄다.
  Rust classifier training 5개와 CLI 45개, `cargo build -p classify_cli --bin classify_cli --offline`,
  rustfmt가 통과했고, 16건 tiny corpus CLI 학습 로그를 확인했다. 운영 corpus의 피크 RSS는 아직 측정하지 않았다.
- spill 학습에서도 holdout 원문이 벡터화와 SVM 동안 RAM에 남는 점을 줄였다. 먼저 기존 API의 원문 보존 계약을
  확인하는 RED 테스트를 추가했고, 기본 동작은 유지하면서 CLI가 명시적으로 요청할 때만 holdout JSONL을 spill하고
  source 필드를 비운다. 평가 직전에 holdout을 복원하며 임시 파일은 종료 시 삭제한다. classifier_training 6개,
  classifier 64개, CLI 45개와 Clippy·rustfmt가 통과했다. 16건 tiny corpus에서 spill 단계와 파일 정리를 확인했다.
  전체 JSONL은 아직 `Vec<Value>`로 적재하며, 운영 corpus RSS 감소량은 미측정이다.
- OOM 위험의 주 원인인 CLI 전체 corpus 적재를 제거했다. `IndexedJsonl`은 JSONL의 offset과 학습 metadata만 보유하고,
  feature fitting과 변환은 한 번에 한 문서씩 읽으며 sparse matrix는 기존 spill 경로를 쓴다. CLI holdout 평가는
  학습 후 필요한 문서만 다시 읽는다. 먼저 인메모리 학습과 holdout key·report가 같은지 RED 테스트로 고정한 뒤 구현했다.
  classifier 65개, training 6개, parity 14개, CLI 45개가 통과했다. `cargo fmt --all -- --check`와
  `git diff --check`도 통과했다. 운영 corpus 학습과 RSS 비교는 실행하지 않아 실제 피크 감소량은 아직 모른다.
- Holdout OOM 경로를 추가로 점검해 `train_indexed`가 평가 직전에 holdout `Value` 전체를 다시 적재하는 것을 찾았다.
  새 indexed 평가 helper를 RED 테스트로 먼저 고정한 뒤 문서를 한 건씩 읽어 report와 calibration 입력을 만들도록 바꿨다.
  기존 materialized 평가와 report·calibration 값이 같은지 확인했다. classifier training 7개와 CLI 45개,
  Backend·CLI Clippy, rustfmt가 통과했다. corpus 학습 및 RSS 실측은 실행하지 않았다.
- `loader`가 운영 Elasticsearch의 HTTPS endpoint에 연결하지 못하는 문제를 수정했다. Python은 인증서 검증을 끄고
  접속했지만 Rust는 HTTPS를 거부했다. Rust는 검증을 유지하고 `TM_ES_CA_CERT`의 PEM CA로 사설 인증서를 검증한다.
  Loader의 bulk 요청과 Backend ES transport, classify CLI에도 같은 HTTPS 동작을 적용했다. Kubernetes backend에는
  기존 `elasticsearch-es-ca` ConfigMap을 읽기 전용 mount하고 환경변수를 설정했다. local TLS 성공·신뢰되지 않는
  인증서 거부 테스트와 ES client 41개, backend ES HTTP·ES manager 104개, Loader 178개 테스트가 통과했다.
  classify CLI 45개 테스트와 Kubernetes client dry-run도 통과했다. 운영 ES 접속과 배포는 실행하지 않았다. 로컬 실행에서는
  `TM_ES_CA_CERT`를 운영 CA PEM 파일로 설정해야 한다.
- Python `test_upload_preview_hides_unexpected_errors`에 대응하는 Rust panic 경로 테스트가 없어 추가했다.
  worker 실행과 panic 변환을 작은 helper로 분리했고, RED에서 helper 부재를 확인한 뒤 실패 응답에 panic 내용을
  노출하지 않는지 검증했다. PDF 편집 24개, 페이지 삭제 71개, gap 11개 테스트와 Backend Clippy·rustfmt가 통과했다.
  Python 대응 사례 2개와 `test_runner_rs -f tests/rust/backend/pdf_delete_pages_gaps.rs`의 target 11개도 통과했다.
- `main_mock` 전체 실행에서 절대 경로 browse 테스트가 실패했다. Python `Book.dict()`는 책 루트 내부 절대 경로를
  상대 경로로 직렬화하므로, 테스트가 임시 루트를 지정하면서 별도 전역 `TM_BOOK_DIR`을 기대한 것이 잘못이었다.
  Python 구현에 맞춰 fixture와 assertion을 수정했다. 실패 재현 뒤 `main_mock` 134개와 test_runner target이 통과했다.
- Python classifier export 11개와 Rust classifier model 32개(환경 의존 2개 무시), import-joblib CLI 45개를 대조했다.
  manifest·field offset·vocabulary/IDF·coefficient layout·golden prediction과 fixture 무결성 테스트가 통과했다.
  Python route 목록과 Rust route parity 1개, body schema parity 2개도 통과했다. 전체 corpus 학습과 workspace 회귀는 실행하지 않았다.
- Python `refresh_token_store` 테스트 70개가 testcontainers MySQL을 포함해 통과했다. Rust 기본 target 63개도 통과했고,
  기본 실행에서 Docker 때문에 무시하던 실제 MySQL 통합 테스트 3개를 `--ignored --test-threads=1`로 따로 실행해 통과했다.
  `test_runner_rs -f tests/rust/backend/refresh_token_store.rs`도 같은 target 63개 통과를 확인했다.
  부분 coverage 보고는 156/185 target만 집계하므로 전체 구현율로 쓰지 않았다.
- classify proposal DB의 Python 테스트 32개가 testcontainers MySQL에서 통과했다. Rust의 실제 MySQL target도
  `classify_proposal_items_db` 16개와 `classify_proposal_status_db` 11개가 각각 `--ignored --test-threads=1`로 통과했다.
  `test_runner_rs`는 기본 실행에서 이 27개를 의도대로 ignored로 표시했다. 운영 DB와 workspace 전체 회귀는 실행하지 않았다.
- view history store Python 테스트 15개가 testcontainers MySQL에서 통과했다. Rust MySQL 통합 13개도
  `--ignored --test-threads=1`로 통과했다. `test_runner_rs -f tests/rust/backend/view_history_store.rs`는
  일반 단위 테스트 2개 통과, Docker 통합 13개 ignored를 확인했다.
- View history API Python 테스트 19개와 Rust `view_history_api` 기본 target 17개가 통과했다.
  Docker 통합 2개도 별도 실행해 실제 MySQL 저장소가 관리자 목록에 반영되는지 확인했다.
- Python `tests/conftest.py`의 Elasticsearch 8.15.0 + `analysis-nori` fixture를 Rust로 옮겼다.
  Rust fixture는 로컬 이미지를 재사용하고 없으면 Docker CLI로 같은 이미지를 만든다. `main_api` 기본 target 59개와
  fake ES 테스트가 통과했고, 기존 ignore 통합 27개도 MySQL·Nori Elasticsearch testcontainer에서 별도 실행해 통과했다.
- 컨테이너 시작 시 testcontainers blocking runner가 Tokio runtime을 중첩 실행하던 실패를 재현하고,
  fixture 시작을 `spawn_blocking`으로 옮겨 해결했다. 이미지가 없을 때 쓰는 Dockerfile stdin build 경로도 별도 scratch tag로 확인 후 정리했다.
- Python `tests/test_main.py` 95개가 통과했다. `test_runner_rs -f tests/rust/backend/main.rs`는 Rust 59개 통과와
  Docker 통합 27개 ignored를 확인했다. Python testcontainers deprecation 및 Starlette 쿠키 경고 7개가 남았다.
- Rust main 통합 테스트에서만 실패한 `/latest` 두 사례를 추적했다. Python `Book.__init__`은 `TM_BOOK_DIR` 아래 절대 경로를 만들지만
  Rust `dummy_book` fixture는 상대 경로를 저장했다. fixture를 Python 의미에 맞춘 뒤 Rust 두 테스트와 Python `test_main.py` 95개,
  `test_runner_rs -f tests/rust/backend/main.rs` target이 통과했다. 부분 coverage 집계는 전체 구현율로 해석하지 않았다.
- `client_error_log`의 익명·인증 요청, 입력 검증, 길이 제한, 개행 처리 동작을 Python과 대조했다.
  `test_runner_rs -f tests/rust/backend/client_error_log.rs`와 Python `tests/test_client_error_log.py`가 각각 8개 통과했다.
  추가 구현이 필요한 동작 차이는 발견하지 않았다. test_runner의 coverage는 156/185 target 부분 집계로 표시됐다.
- 손상 EPUB 표지 검사·목록·교체의 기존 대응을 다시 실행했다. `cover_repair` Rust target 5개와 Python
  `tests/test_cover_repair.py` 8개가 통과했다. `test_runner_rs` coverage는 부분 집계이므로 전체 함수 진행율로 쓰지 않았다.
- MySQL container에서 `backend::db`의 Docker 전용 통합 테스트 8개를 실행해 모두 통과했다.
  `test_runner_rs -f tests/rust/backend/db.rs`의 일반 target은 14개 통과, Docker 테스트 8개 ignored를 확인했다.
  전체 workspace 회귀는 실행하지 않았다.
- classifier CLI의 `evaluate`, `score-calibration`, `bookstore-policy`가 학습 corpus 전체를 `Vec<Value>`로 읽던 경로를 찾았다.
  manifest/fallback holdout 인덱스 선택을 추가하고, 평가와 calibration은 한 문서씩 처리하게 바꿨다.
  bookstore policy도 holdout index만 섞고 후보 문서를 순서대로 읽으며 최대 표본 수만 본문으로 보유한다.
  중복 document key에서도 materialized holdout과 같은 index를 고르는 RED 테스트를 추가했다.
  Rust `classifier_training` 8개, classify CLI 45개와 Python `tests/test_classify_cli.py` 43개가 통과했다.
  Backend·CLI Clippy, workspace fmt와 diff 검사가 통과했다. 운영 corpus 실행은 OOM 위험 때문에 하지 않았다.
- `collect --with-publisher`도 JSONL 전체를 `Vec<Value>`로 읽어 다시 쓰고 있었다.
  256건 batch 처리, publisher cache 재사용, 임시 파일 완성 후 원본 교체를 구현했다.
  잘못된 JSONL에서는 원본을 유지하고 임시 파일을 지우는 실패 테스트도 추가했다.
  Rust `classifier_corpus` 18개, classify CLI 45개와 Python corpus/CLI 테스트 57개가 통과했다.
  Backend·CLI Clippy, workspace fmt와 diff 검사가 통과했다.
- `book_manager_1`의 운영 ES URL 의존 통합 테스트 12개를 Python과 같은 Nori Elasticsearch testcontainer로 전환했다.
  기존 helper는 `TM_TEST_ES_URL`이 없으면 panic하는 RED를 확인했고, fixture가 컨테이너 URL과 테스트별 새 index를 쓰도록 바꿨다.
  12개 Rust 통합 테스트와 Python `TestBookManager` 12개가 통과했다. `test_runner_rs`의 기본 target은
  145개 통과·Docker 통합 12개 ignored를 확인했다. Backend target Clippy, workspace fmt와 diff 검사가 통과했다.
- `utils/stat.py`와 `libs/stat`의 카운터 동작을 다시 대조했다. Rust `stat` target 12개와 Python `test_stat.py` 5개,
  `stat` Clippy, workspace rustfmt, `git diff --check`가 통과했다. 통계 모듈에서 추가 수정이 필요한 동작 차이는 찾지 못했다.
- 운영 Rust 코드의 Python subprocess 호출을 검색했다. backend/loader는 Python을 실행하지 않고,
  LibreOffice·EPUBCheck·시스템 PDF 도구 등 기존 외부 프로그램만 실행한다. Python 원본의 카테고리 재적재 호출은
  Rust `loader` 바이너리 호출로 대응되며, 런타임 이미지도 Python 앱이 아닌 Rust backend·loader를 실행한다.
- `normalize_comic_directories`의 이름 분석·계획·캐시·파일 적용 경로를 재검증했다. Rust crate 전체 39개 테스트와
  `test_runner_rs -f tests/rust/normalize_comic_directories/plan.rs`의 대상 26개가 통과했다.
  Python과 대조하는 이름 fixture 및 rename rollback 사례를 포함한다. live bookstore 요청은 실행하지 않았다.
- HWP3 parser 현재 상태를 다시 검증했다. `test_runner_rs -f tests/rust/hwp3_parser/hwp3.rs`의 Rust 25개와
  Python `tests/test_hwp3_parser.py` 97개가 통과했다. testcontainers의 기존 deprecation warning 1건이 남았다.
  변환 table 전체 대조를 포함한 기존 parity 검증과 함께, 이번 확인에서도 추가 차이는 나오지 않았다.
- `utils/search` 출력이 Python `6dc7153` 동작과 다른 점을 찾았다. Python은 hit의 `summary` 누락을 예외로 처리하지만
  Rust는 빈 문자열로 숨겼다. 기존 Rust 테스트를 반대로 바꿔 RED(exit 0)를 확인한 뒤 누락·비문자열 summary 오류를
  exit 1로 전달하도록 수정했다. search suite 12개, Clippy, fmt, `test_runner_rs` 대상 9개가 통과했다.
  Python 원본 파일은 현재 저장소에서 제거돼 별도 pytest는 실행하지 않았다.
- `BookManager`의 ES 문서 변환이 Python `Book.__init__`과 달리 필수 필드 누락을 빈 값으로 숨기는 차이를 찾았다.
  `get_book`의 category 누락 테스트가 RED임을 확인한 뒤 필수 string·integer 필드 검증을 추가했다.
  이어 목록·검색·유사 검색·최신순 조회 경로에도 같은 검증을 적용하고, 7개 API에서 오류 tuple을 확인하는 테스트를 추가했다.
  `book_manager_1` 147개 통과·Docker 12개 ignored, Python 검색 관련 14개 통과, target Clippy, workspace fmt와 diff 검사가 통과했다.
  Python `tests/test_book.py` 6개도 통과했다. `get_book_content` 등 catalog 바깥의 ES→Book 변환은 후속 대조가 필요하다.
- Book preview, PDF page, HTML resource, cover도 필수 ES 필드 누락을 기본값으로 숨기는 차이를 찾았다.
  category가 빠진 fixture에서 두 경로가 200을 반환하는 RED를 확인한 뒤, Python `Book` 생성 오류와 같이
  변환 실패를 500 응답으로 전달하도록 했다. PDF page 조회도 같은 누락에서 404를 내는 RED를 고쳐 500을 반환한다.
  `get_book_content`는 누락 문서와 오류를 구분하는 `BookContent::Error`를 추가하고 download route에서 500으로 변환한다.
  `book_manager_2` 161개, `main_mock` 135개, `test_runner_rs` 대상, Python preview·HTML tests 47개가 통과했다.
  Python `test_cover.py`는 29개 통과·3개 실패로, 기존 Pillow `Image.get_flattened_data` 환경 오류를 재현했다.
  target Clippy, fmt와 diff 검사도 통과했다.
- `update_book`과 `delete_book`도 malformed ES 문서의 기본값을 사용해 파일 작업을 진행하는 차이를 찾았다.
  category 누락 테스트에서 기존 경로가 파일 이동 오류·삭제 경고로 끝나는 RED를 확인했다.
  두 함수 모두 먼저 필수 필드를 검증하고 오류 tuple을 반환한다. Rust `book_manager_1` 149개 통과·Docker 12개 ignored,
  `test_runner_rs` 대상, Python update/delete 테스트 20개, target Clippy, fmt와 diff 검사가 통과했다.
  cover list 파일·작품 대표 문서도 변환 오류를 삼키는 RED를 고쳐 오류 tuple을 반환한다. Rust `book_manager_gaps` 49개,
  `test_runner_rs` 대상, Python cover-list 테스트 12개, target Clippy, fmt와 diff 검사가 통과했다.
  PDF 편집도 category 누락에서 파일을 실제 변경하는 RED를 고쳐 편집 전에 오류를 반환한다.
  사용처가 사라진 비검증 `make_book` helper도 제거했다. `pdf_delete_pages_gaps` 12개와 test_runner target,
  Python `test_pdf_delete_pages_gaps.py` 17개, target Clippy, fmt와 diff 검사가 통과했다.
  현재 BookManager의 ES→Book 생성 지점은 모두 검증 helper를 사용한다. 프로젝트 전체 함수 동등성 검증은 계속 남아 있다.
- 서점 응답 캐시 JSONL에서 숫자 ISBN·저자를 Python은 문자열로 바꾸지만 Rust가 무시하는 차이를 찾았다.
  이어 truthy list/dict metadata도 Python `str()`와 다르게 fallback하던 차이를 RED로 확인했다.
  JSON scalar·list·dict의 truthiness와 문자열 표현을 Python에 맞췄다. Python 캐시 테스트 21개, Rust `bookstore_cache` 22개와
  `test_runner_rs -f tests/rust/backend/bookstore_cache.rs`가 통과했다.
  Backend target Clippy, workspace rustfmt와 `git diff --check`도 통과했다. test_runner coverage는 156/185 target 부분 집계다.
- Rust 배포 경로는 SQLite 대신 공유 MySQL을 사용하기로 한 결정을 반영했다. 기존 factory의 기본값이 SQLite이고
  `sqlite`·알 수 없는 backend도 SQLite로 열리는 RED를 확인한 뒤, factory를 MySQL 전용으로 바꾸고 나머지 값은 거부한다.
  refresh-token Rust target 63개 통과·Docker 3개 ignored다. 기존 SQLite 직접 구현 테스트는 유지했으며 운영 factory에서는 선택하지 않는다.
- Python module-level model cache와 달리 Rust `BookClassifierService::new`는 기본 모델을 매번 직접 읽었다.
  Manager의 기존 `OnceLock` 동작은 유지하면서, standalone service도 누락 결과를 포함해 경로별로 cache하도록 했다.
  `reset_default_model` API와 경로 변경 재로드도 추가하고 기본 분류 경로 두 곳에 연결했다.
  cache test를 RED로 추가한 뒤 Rust classifier 66개, book classifier 44개,
  book manager classifier 6개와 `test_runner_rs -f tests/rust/backend/classifier.rs`가 통과했다.
  Python classifier/book-classifier 189개와 classifier service 10개, Backend target Clippy, fmt와 diff 검사도 통과했다.
- `cover_status`의 실제 HTTP fixture가 `Book::new`에 절대 경로를 넘겨 `Book::dict()`의 상대 경로 계약과 달랐다.
  `test_runner_rs -f tests/rust/backend/cover_status.rs`에서 해당 HTTP 사례가 500으로 실패했다.
  fixture 파일을 `TM_BOOK_DIR` 아래에 만들고 상대 경로로 구성했으며, 응답도 Python과 같은 상대 경로인지 검사한다.
  Rust target 22개, Python `tests/test_cover_status.py` 17개, Backend target Clippy, rustfmt와 diff 검사가 통과했다.
  test_runner coverage는 156/185 target만 측정한 부분 집계다.
- Python loader 테스트 359개와 Rust loader target의 이름을 비교했다. 이름이 다른 timeout, ISBN, HWP3,
  HWP preview 사례는 별도 crate/target으로 옮겨져 있었다. Rust ISBN 38개와 Python `test_isbn.py` 50개,
  parser timeout 12개, HWP3 parser 25개, HWP preview 1개, loader의 파일 핸들 사례 1개가 통과했다.
  이 대조는 테스트 이름 이동을 확인한 것이며, loader 전체 기능 동등성 감사를 끝낸 것은 아니다.
- `.env.example`에 HTTPS Elasticsearch private CA용 `TM_ES_CA_CERT` 설정 예시를 추가했다.
  Rust client는 인증서 검증을 유지하므로, 사설 CA를 쓰는 로컬 실행 경로에서 인증서를 지정할 수 있다.
  이는 설정 안내 변경이며 운영 ES 접속 검증은 수행하지 않았다.
- `classify_books_client`의 REST polling에서 `result: null` 응답을 무시하고 계속 polling하는 차이를 찾았다.
  후속 `done` 응답을 둔 회귀 테스트가 수정 전 성공 종료하는 RED를 확인한 뒤, Python처럼 잘못된 상태 객체에서 코드 1로 끝내도록 수정했다.
  `classify_books_client` 테스트 18개, all-target Clippy, workspace rustfmt와 `git diff --check`가 통과했다.
- 일반 API의 `main_api` Docker 통합 테스트를 실제 MySQL·Nori Elasticsearch testcontainers에서 실행했다.
  기존 무시 테스트 27개가 통과했고 두 테스트 컨테이너가 종료된 것을 확인했다. 전체 회귀는 실행하지 않았다.
- classifier CLI 기준 suite를 다시 대조했다. Python `test_classify_cli.py`·`test_classify_cli_gaps.py`는 52개 통과,
  Rust 활성 `classify_cli::cli` target은 45개 통과했다. Python parameterization 때문에 개수는 직접 1:1 비교하지 않는다.
  Python testcontainers deprecation과 `runpy` 경고 2건이 있었다. 전체 회귀는 실행하지 않았다.
- refresh-token 저장소의 실제 MySQL 통합 테스트 3개를 실행했다. 집계 SQL·email 필터·기존 테이블 migration이 통과했고,
  테스트 종료 후 MySQL 컨테이너가 정리됐다. 전체 회귀는 실행하지 않았다.
- 운영 ES TLS 실행 구성을 재점검했다. Kubernetes의 `TM_ES_CA_CERT`, read-only CA mount, ConfigMap key가 맞고,
  `kubectl apply --dry-run=client -f k8s/tm-deployment.yml`이 통과했다. local ES 인증 테스트 2개도 통과했지만 production ES 접속은 확인하지 않았다.
- 서점 검색 묶음을 다시 대조했다. Python `tests/test_bookstore.py` 97개와 Rust `bookstore` 111개가 통과했고,
  Rust 전용 Joara credential/cache 테스트 15개도 실행됐다. abstract method 기본값 테스트 1개만 Rust trait에 대응 개념이 없어 무시됐다.
  전체 회귀는 실행하지 않았다.
- Python에 있던 apply 요청 경합, 교체 토큰 보호, 진행 상태·카운터 보존 테스트를 Rust HTTP 경로와 대기형
  `DeferredRunner`로 복원했다. placeholder 제안 행을 먼저 INSERT한 뒤 분류 결과를 UPDATE하는 테스트도 공개 route로 옮겼다.
  항목별 apply 결과를 DB에 즉시 저장하는 성공·실패 두 건 테스트도 공개 apply route로 옮겼다.
  Rust `main_mock` 141개, Python 대응 테스트 2개, `test_runner_rs` target이 통과했다.
  coverage는 156/185 target 부분 집계다. Python `pytest` launcher는 `_console_main` ImportError가 났지만
  `uv run --no-sync python -m pytest`는 정상 실행됐다. Backend target Clippy, workspace fmt와 diff 검사도 통과했다.
- 로컬 `.env`의 production ES 주소에 읽기 전용 `/_cluster/health` 요청을 보냈다. 이름은 사설 주소로 해석됐지만
  현재 실행 환경에서 HTTP 80/TCP 연결이 timeout되어 ES 응답을 받지 못했다. 따라서 이는 Rust TLS 구현 실패의 증거가 아니며,
  이 환경에서는 production 접속을 검증할 수 없다. Kubernetes는 별도 내부 HTTPS 주소와 `TM_ES_CA_CERT`를 사용한다.
- `BookManager` 미커버 표지 판정의 Rust cache가 Python 경로를 따라 SQLite 파일을 열던 것을 발견했다.
  사용자 결정에 맞춰 cache를 공유 MySQL로 바꾸고, 경로 SHA-256 key와 파일 mtime·size를 비교해 stale row만 재계산한다.
  RED 단계에서 기존 API로는 MySQL cache를 구성할 수 없음을 확인한 뒤 provider 기반 저장소로 옮겼다.
  MySQL testcontainer 통합 7개, Rust `cover_status` 기본 target 14개, Python `tests/test_cover_status.py` 17개가 통과했다.
  `test_runner_rs` target, Backend target Clippy, workspace fmt와 diff 검사가 통과했다.
  coverage는 156/185 target의 부분 측정이다. MySQL 통합 테스트 7개는 Docker 필요로 기본 실행에서 제외된다.
- MySQL 전환 뒤에도 refresh-token SQLite 구현과 `rusqlite` dependency가 Rust binary에 남아 있음을 확인했다.
  기존 refresh-token 동작 테스트를 상태 있는 MySQL 대역으로 옮기고 SQLite 저장소·오류·factory variant와 dependency를 제거했다.
  일반 Rust target 62개, 실제 MySQL testcontainer 3개, Python `tests/test_refresh_token_store.py` 70개가 통과했다.
  `test_runner_rs`, Clippy, workspace fmt와 diff 검사가 통과했다. Rust manifest·source·lockfile에서 SQLite dependency가 사라졌다.
  coverage는 156/185 target 부분 측정이며 함수 61.33%다.
- PDF 편집 target 실행에서 19개 테스트가 공통으로 실패했다. Python `Book`과 Rust `Book::new`가 필수로 요구하는
  `updated_time`과 `file_size`가 Rust 테스트의 가짜 ES 문서에서 빠져 있었다. 유효한 문서 필드를 fixture에 추가했다.
  Rust `pdf_edit` 24개, Python PDF 페이지 삭제·경계 테스트 145개, `test_runner_rs` target이 통과했다.
  제품 코드는 바꾸지 않았다. test_runner coverage는 156/185 target 부분 측정이다.
- Python `training.fit_confidence_calibration`은 독립 API였지만 Rust는 학습 경로 안에 계산을 복제하고 있었다.
  새 Rust 테스트가 함수 부재로 RED가 됐다. `fit_confidence_calibration` API를 추가하고 일반 학습 경로가 같은 helper를 쓰게 했다.
  Indexed JSONL 학습은 본문을 계속 문서별로 읽는다. Rust `classifier_training` 9개, Python classifier·calibration 89개,
  `test_runner_rs`, Backend target Clippy, workspace fmt와 diff 검사가 통과했다. 운영 코퍼스 학습은 실행하지 않았다.
- `backend.main.stale_running_status`를 Rust에서 이름으로 찾지 못해 호출 경로를 확인했다. Python에서는 테스트 전용 함수이고,
  실제 API 동작은 `CategoryMapping`이 담당한다. Rust는 MySQL의 `updated_at` 나이를 계산해 `running`·`applying`을 실패로 바꾸며
  진행 값을 보존한다. Python 해당 테스트 1개, Rust `category_mapping` 78개와 `test_runner_rs` 대상 테스트가 통과했다.
  Docker 통합 테스트도 Rust와 Python에서 각각 1개씩 통과했다. 이 상태 복구 동작의 누락은 확인하지 않았다.
- 실행 경로에서 legacy Python `BookManager.reload_category`만 `sys.executable`로 loader를 호출한다.
  Rust `BookManager`는 같은 작업을 `CommandRunner`로 loader 바이너리에 전달한다. 나머지 Python subprocess 호출은
  LibreOffice·djvused·djvutxt·strings 같은 외부 도구 호출이며 Rust 구현도 외부 프로그램을 직접 실행한다.
- API의 도서 경로 serialization에서 사용자 지정 root를 쓰는 manager가 global `TM_BOOK_DIR` 기준으로 경로를
  변환하는 차이를 전체 Rust 회귀 실행에서 찾았다. `Book::dict_with_prefix`와 공통 `book_json` 변환을 연결하고,
  browse 목록도 같은 변환 경로를 사용하도록 했다. 이후 book·book_admin_api·main_api·main_mock 단위 검증과
  test_runner 대상 실행, Python 대응 라우트 4개, Clippy, rustfmt가 통과했다. scanner 두 crate를 제외한 workspace
  회귀 실행은 모든 출력된 테스트 결과가 통과했지만, 세션 종료 직후 실행 핸들이 사라져 최종 Cargo 종료 코드는
  별도로 수집하지 못했다. production ES 연결은 환경변수가 없어 검증하지 않았다.
- Docker 필요 ignored 통합 테스트를 target별 순차 실행했다. book manager ES 12개, main_api 27개,
  proposal MySQL 27개, cover_status 7개, MySQL adapter 8개, ES manager 1개, refresh-token 3개,
  view-history API/store 15개가 통과했다. `cover_status` 통합 fixture가 `Book`의 `TM_BOOK_DIR`와 다른 manager root를
  사용해 경로 기대값이 어긋났고, async test에서 동기 testcontainer runner를 직접 호출해 재실행이 panic했다.
  fixture root를 `Book::path_prefix()`와 맞추고 container 시작을 `spawn_blocking`으로 옮겨 해결했다.
  `test_runner_rs -f tests/rust/backend/cover_status.rs`는 활성 14개 통과·7개 Docker 테스트 무시,
  rustfmt와 diff 검사도 통과했다. 전체 `--ignored` 실행은 운영 classifier 경로 두 개가
  `TM_CLASSIFIER_IMPORTED_DIR`·`TM_CLASSIFIER_EXPORT_DIR` 미설정으로 실패해 중단됐다.
- Python OpenAPI route fixture와 Rust 등록 method를 대조했다. 전체 route/method parity 1개,
  필수 JSON field 거부 1개, Python body 예제 deserialization 1개가 통과했다.
  별도 `.env` 자동 로딩은 Python loader도 하지 않으며 startup script와 Kubernetes가 환경을 전달하므로 변경하지 않았다.
- `.env`의 기존 classifier joblib 파일을 학습 없이 Python export와 Rust importer로 변환했다.
  Python sklearn golden 266건을 만들고 `real_model_matches_sklearn_golden`과
  `imported_operational_joblib_matches_python_golden` 두 ignored parity test를 실행했다.
  79개 class·952,717 feature 실제 모델에서 2개 모두 통과했다. 모델 변환과 추론만 수행했고 학습은 실행하지 않았다.
- 최종 workspace 회귀를 다시 실행했다. `cargo test --workspace --exclude pdfscan --exclude epubscan --locked`는
  3,057 passed, 0 failed, 103 ignored로 종료 코드 0이다. 이어 임시 model/golden 경로를 전달해 같은 범위의
  ignored 테스트를 순차 실행했고 103 passed, 0 failed로 종료 코드 0이다. frontend와 두 scanner는 제외했다.
- workspace 전체 ignored 목록은 103개이며 각각 서비스 통합 또는 운영 classifier parity다.
  이번 순차 `--ignored --test-threads=1` 실행에서 103개 모두 통과해 무시 상태를 남기지 않았다.
- 최종 Python 기준 suite `uv run --no-sync python -m pytest tests/ -q`는 3,048 passed, 3 failed다.
  실패 3개는 기존 Pillow `Image.get_flattened_data` API 부재로 cover 추출이 404가 되는 사례다.
  Rust workspace 테스트는 같은 시점에 3,057 passed, 0 failed였고 ignored 103개도 별도 통과했다.
- loader 묶음의 기존 Rust 테스트를 `test_runner_rs -f tests/rust/loader/loader_1.rs`로 다시 실행했다.
  178개 테스트가 통과했고, runner는 의존 모듈 15개의 부분 coverage를 보고했다(전체 라인 44.35%).
  이 수치는 loader 기능 완성률이 아니라 해당 테스트 실행 경로의 coverage다. 전체 target 실행 옵션은
  `pdfscan`·`epubscan`도 포함해 시작 직후 중단했고 scanner 테스트는 실행하지 않았다.
- loader 배치 처리에서 ES의 기존 경로 조회·metadata 조회·bulk insert·두 `refresh()` 오류를 무시하고 성공 종료하는 차이를 찾았다.
  회귀 테스트 5개가 수정 전 종료 코드 0을 재현했다. 오류를 `Result`로 전파해 CLI가 255로 종료하게 수정했다.
  Rust loader target 178개·106개·오류 회귀 6개와 Python `tests/test_loader.py` 372개,
  loader Clippy, workspace rustfmt, `git diff --check`가 통과했다. 전체 회귀는 실행하지 않았다.
- 만화 디렉토리 정리 도구에서 `--refresh-cache` 초기화가 실패해도 Rust CLI가 계속 진행하는 차이를 찾았다.
  Python 원본은 cache 파일 truncate/create 실패를 전파한다. 실패 경로 테스트가 수정 전 보고서를 쓰고 0으로 끝나는 RED를 확인했다.
  `BookstoreSearchCache::try_new`를 추가하고 CLI가 초기화 오류에서 종료 코드 1로 끝나게 했다.
  Rust `plan` 27개·`bin` 3개, package Clippy, workspace rustfmt, `git diff --check`가 통과했다.
- 표지 blank 판정에서 RGB 채널을 2비트씩 자르는 근사가 Pillow `quantize(colors=16)`과 달라질 수 있었다.
  근접한 16색이 있는 synthetic 회귀 테스트가 기존 Rust에서 오분류하는 RED를 확인했다. Pillow MEDIANCUT의
  최대 색상 그룹 우선 분할, 가중 RGB 축·중앙값 분할, 평균 palette 최근접 색 재매핑을 적용했다.
  Pillow 10.4의 같은 fixture 최대 색 비율은 39.58%였다. Rust cover target 24개,
  histogram 입력 순서 독립 unit test 1개, Backend all-target Clippy, workspace rustfmt,
  `git diff --check`가 통과했다. HashMap iteration에 따른 median-cut 동률 결과를 없애도록 RGB 순서를 고정했다.
  실제 이미지 전반의 Pillow 양자화 완전 일치는 아직 확인하지 않았다.
- 공용 utility 검증에서 전역 `PYTHONPATH`가 project venv 대신 다른 pytest를 먼저 가져오는 환경 문제를 확인했다.
  `env -u PYTHONPATH`로 실행해 Python `stat`·`file_time` 15개와 cover의 blank 첫 페이지 사례 2개가 통과했다.
  Rust `stat` 12개와 `file_time` 11개도 개별 통과했다. pytest 경고는 testcontainers와 Starlette deprecation이다.
- `utils/search.py`의 `int(sys.argv[4])`는 밑줄 구분 숫자와 앞뒤 공백을 허용하지만 Rust `parse::<i64>()`는 거부했다.
  `1_000`, 공백·부호가 있는 `+42`, 전각·아라비아 decimal digit 테스트의 RED를 확인한 뒤 호환 파서를 추가했다.
  잘못된 연속 밑줄과 decimal 범주가 아닌 숫자 문자는 거부한다. 정수 저장 범위는 ES API의 `i64`를 따른다.
  Rust search 10개, pyfmt 6개, 두 crate Clippy, workspace rustfmt, `git diff --check`가 통과했다.
- `quarantine_broken.load_corrupted`가 Python 원본과 달리 객체가 아닌 리포트 항목을 조용히 버렸다.
  해당 항목이 섞인 리포트를 입력하는 회귀 검사가 수정 전 성공하는 RED를 확인했다. Rust도 잘못된 항목에서
  `InvalidData`를 반환하도록 바꿨다. quarantine target 49개와 `test_runner_rs` 대상·CLI 검증이 통과했다.
  Python 원본 테스트 파일은 현재 worktree에 없어 이 사례는 Rust에서 재현했으며, 전체 회귀는 실행하지 않았다.
- `move_semi_clusters`는 Python에서 준확실 클러스터 값이 목록이 아니면 예외가 나지만 Rust는 조용히 건너뛰었다.
  `null` 클러스터 입력의 실패 테스트가 수정 전 성공하는 RED를 확인하고, 잘못된 목록 형식을 오류로 처리했다.
  Rust mover 11개와 `test_runner_rs` 대상 검증이 통과했다. Python 전용 테스트 파일은 없어 Rust 회귀 입력으로 확인했으며,
  package Clippy·workspace rustfmt·diff 검사를 통과했다. 전체 회귀는 실행하지 않았다.
- Git 이력의 Python `detect_mojibake.py`에 EPUB 구조 검사 기능과 CLI 하위명령이 남아 있었지만 Rust 전환에서 빠졌다.
  `epubscan` crate는 수정하지 않고 detector에 `inspect_epub_structure`, EPUB 전용 scan/report와 CLI 경로를 복원했다.
  RED에서 공개 API 부재를 확인했다. Python edge case를 옮기며 손상된 ZIP local header를 missing으로 오분류하는 문제와
  4 KiB 초과 markup을 unreadable로 오분류하는 문제를 각각 RED로 재현해 수정했다. 잘못된 mimetype, 압축된 ZIP 멤버,
  spine·media-type 분기, 누락/손상 XML, 개별 리포트 경로도 검사한다. Python의 unexpected exception 사례도
  parser seam panic으로 주입해 `scan_error`와 이후 스캔 지속을 확인한다. 구조 검사·scan·CLI 19개,
  panic 격리 unit 1개, 기존 mojibake target 32개, detector Clippy, workspace rustfmt,
  diff 검사가 통과했다. Python 원본 테스트 파일은 Git 이력에만 있어 직접 실행하지 않았다.
  이후 이력의 EPUB 검사 test function 31개를 다시 대조해 Rust 통합·unit·공용 경로 테스트의 대응을 확인했고,
  `epub_structure` `test_runner_rs` target도 통과했다. 전체 회귀는 실행하지 않았다.
- `classify_books_client --uncached-only`의 Python 캐시 조회는 `or` 체인과 `dict.get(... ) is None`을 쓴다.
  Rust는 첫 키가 빈 객체여도 다음 키로 넘어가지 않고, target key가 빠진 비어 있지 않은 entry도 재처리했다.
  Python truthiness·fallback 순서를 고정하는 RED 회귀를 추가한 뒤 이를 맞췄다. 해당 unit test와 package 검증,
  `test_runner_rs`, Clippy, workspace rustfmt, diff 검사가 통과했다. 전체 회귀는 실행하지 않았다.
- `normalize_comic_directories.parse_filename`에서 Python `re`의 `\w`는 결합문자만으로 이뤄진 stem을 거부하지만,
  Rust 정규식 `\w`는 이를 유효한 제목으로 받았다. 해당 입력의 회귀 테스트가 수정 전 실패하는 것을 확인하고,
  Python `str.isalnum()`과 같은 범주를 제공하는 `pyfmt::py_isalnum` 및 밑줄 검사로 바꿨다. Python 기준 확인,
  Rust names target 11개, `test_runner_rs`, package Clippy, workspace rustfmt, `git diff --check`가 통과했다.
  전체 회귀는 실행하지 않았다.
- 만화 정리 캐시의 UTF-8 디코딩 오류에서 Python은 `UnicodeDecodeError`를 내보내지만 Rust는 손상 바이트를
  대체문자로 읽고 계속했다. CLI 초기화가 잘못된 UTF-8을 `InvalidData`로 반환하는 RED 테스트를 추가해 확인한 뒤,
  cache loader를 손실 없는 UTF-8 디코딩으로 바꿨다. Python 기준 확인, Rust plan 28개, package Clippy,
  workspace rustfmt와 `git diff --check`가 통과했다. 전체 회귀는 실행하지 않았다.
- `docs/rust-workspace.md`가 ES client를 HTTP 전용으로 설명해 Rust의 HTTPS 지원 및 `TM_ES_CA_CERT`와 어긋났다.
  실제 client 구현을 확인해 지원 프로토콜과 사설 CA 설정 설명을 갱신했다. 문서 변경만 수행했다.
- HWP3 `HwpStream.skip`은 위치를 `i64`로 더해 큰 양수에서 overflow panic을 냈다. 스트림 일부를 소비한 뒤
  `skip(i64::MAX)`를 호출하는 테스트가 수정 전 panic을 재현했다. 남은 길이와 먼저 비교하도록 고쳤다.
  Rust HWP3 target 26개, Python `tests/test_hwp3_parser.py` 97개, `test_runner_rs`, package Clippy,
  workspace rustfmt와 diff 검사가 통과했다. 전체 회귀는 실행하지 않았다.
- `parser_timeout.run_with_hard_timeout`은 자식이 끝나도 손자 프로세스가 출력 pipe를 상속하면 reader thread를
  기다리며 timeout을 넘겼다. 재현 테스트에서 0.2초 timeout이 0.7초 뒤에야 반환했다. 명령을 별도 process group으로
  실행하고, 출력 drain도 같은 deadline으로 제한했으며 만료 시 그룹 전체를 종료한다. Rust parser_timeout 13개,
  doc_reader 45개, isbn 38개, Python `tests/test_parser_timeout.py` 12개, `test_runner_rs`, Clippy,
  workspace rustfmt와 diff 검사가 통과했다. `libc` 기존 버전을 직접 사용하며 전체 회귀는 실행하지 않았다.
- hard timeout 변경 뒤 loader 호출 경로도 확인했다. PDF 파서·복구·페이지 수 timeout Rust 테스트 4개,
  Python `tests/test_loader.py -k timeout` 15개, loader `test_runner_rs` target이 통과했다.
- 런타임 실행 구성을 확인했다. `startup.sh`는 Rust backend·loader binary를 띄우고 Docker runtime에는 Python
  interpreter가 없다. Docker의 Python stage는 PDFium shared library를 추출하는 build-only 용도다.
  `utils/stat`(Rust 12·Python 5)과 `utils/update`(Rust 11·Python 4) 테스트 및 두 `test_runner_rs` target이 통과했고,
  이 범위에서 동작 차이는 찾지 못했다. 전체 기능 동등성 감사는 계속 남아 있다.
- 서점 구현 대응표를 확인했다. `backend/bookstore.py`의 13개 구현은 Rust factory 13개와 대응하고,
  표지 검색 store 8개도 Python/Rust 목록이 같다. Python `test_bookstore.py`·`test_cover_sources.py` 145개와
  Rust `backend::bookstore`, `backend::cover_sources` `test_runner_rs` target이 통과했다. 이 범위에서
  미대응 서점은 찾지 못했다.
- Python classifier exporter의 manifest, vocabulary·IDF·계수 파일, golden 생성과 CLI 경로를 Rust
  `export_format`, `classify_cli import-joblib` 구현 및 테스트와 대조했다. Python exporter 11개와
  Rust `classifier_model` target 30개가 통과했고, Rust target의 운영 모델 2개는 환경 입력이 없어 ignore됐다.
  이 범위에서 기능 공백은 찾지 못했다.
- `backend/image_edit.py`의 분수 좌표 crop, 회전·보간, 1-bit grayscale 변환과 잘못된 영역 오류를 재검증했다.
  Python `tests/test_image_edit.py` 8개와 Rust `image_edit` `test_runner_rs` target이 통과했다.
- Pillow 10.4.0의 MEDIANCUT·FIND_EDGES 결과를 8종 색 패턴, seed 32개로 고정해 Rust blank-cover 판정을 비교했다.
  Python oracle과 Rust `cover` target 25개가 통과했다. 이 합성 입력 32개에서는 불일치가 없으며, 전체 이미지 분포의 parity는 남아 있다.
- Git 이력의 `test_detect_mojibake.py` EPUB 검사 31개 시나리오를 Rust 통합·private unit·`corpus_common` report-path 테스트와 다시 대조했다.
  손상 ZIP, 각 XML/spine 실패, binary spine, markup media type, panic 격리, scan limit, CLI 및 경로 정규화 사례가 대응한다.
  Rust `epub_structure` `test_runner_rs` target이 통과했다. 원본 Python 테스트는 worktree에서 삭제돼 직접 실행하지 않았다.
- 함수 인벤토리에서 직접 이름이 다른 `scan_documents`를 대조했다. Python generator의 lazy scan은 Rust `scan_each` callback 경로로
  이어지고 classifier corpus 수집도 이 스트리밍 경로를 쓴다. Python corpus 14개와 Rust `classifier_corpus` `test_runner_rs` target이 통과했다.
- `utils/isbn.py`의 PDFium·pypdf 순서, 주어진 본문 우선, timeout, EPUB spine, DjVu/HWP fallback을 Rust API·worker 경로와 대조했다.
  Python `tests/test_isbn.py` 50개와 Rust ISBN target 38개가 통과했다. Rust target은 실제 PDF fallback parser와 worker 종료 검증도 포함한다.
- HWP3 stream의 `read_bytes(usize::MAX)`가 일부 바이트를 소비한 뒤 Rust에서 overflow panic을 내는 차이를 RED로 재현했다.
  남은 길이와 먼저 비교하도록 고쳤다. Python HWP3 97개, Rust HWP3 27개, `test_runner_rs`, crate Clippy,
  rustfmt와 `git diff --check`가 통과했다.
- Rust 배포 코드에서 SQLite 사용이 남았는지 다시 확인했다. Rust manifest·lockfile·backend·utils·libs에
  SQLite 구현이나 `rusqlite` 의존성은 없고, `cargo tree -i rusqlite --workspace --locked`도 패키지를 찾지 못했다.
  운영 Rust 경로에서 Python interpreter를 호출하는 코드도 찾지 못했다. 재적재 도구는 Rust `loader` 바이너리를 호출한다.
  이 확인은 기존 MySQL 전환 결정을 검증하며 기능 테스트는 추가 실행하지 않았다.
- Loader의 두 번째 기존 Rust target `tests/rust/loader/loader_2.rs`를 `test_runner_rs`로 실행했다.
  106개 테스트가 통과했다. runner가 함께 출력한 coverage는 변경 파일 의존성 묶음 전체의 34.33%이며,
  Loader 단독 기능 완성률로 해석하지 않는다.
- `blank-cover` Pillow parity의 실제 이미지 증거를 보강했다. Python Pillow 10.4.0 oracle에서 저장소 PNG·GIF·JPEG fixture 5개가 모두 비어 있지 않다고 판정했다.
  같은 fixture를 Rust 회귀 테스트에 넣어 `test_runner_rs -f tests/rust/backend/cover.rs` 26개가 통과했다.
  이 저장소 표본의 일치만 확인했으며 전체 운영 이미지 분포 parity는 여전히 미확인이다.
- Pillow 최신 버전에서 Python blank-cover oracle가 제거된 `Image.get_flattened_data()`를 불러 표지 추출을 실패하는 것을 재현했다.
  픽셀 바이트를 순회하는 `Image.tobytes()`로 바꿔 같은 grayscale 순서를 유지했다. RED에서 실패한 blank-first-cover 2개,
  Python cover target 32개와 Rust `cover` target 26개가 통과했다. 전체 Python 회귀는 실행하지 않았다.
- Pillow `BILINEAR`와 Rust `Triangle` resize parity를 600×900 원본 입력 32개로 비교했다.
  Python Pillow 10.4.0 oracle와 Rust 결과열이 같았고 `cover` `test_runner_rs` target 27개가 통과했다.
  이번 입력 분포는 기존 64×96 테스트와 다른 resize 경로를 검증하지만 전체 이미지 parity를 증명하지는 않는다.
- `search`의 Python 정수 파서가 Rust `unicode-general-category` 16.0을 따라 Python 3.13 Unicode 15.1에 없는 U+10D40을 숫자로 받는 차이를 찾았다.
  Python 3.13.7 `str.isdecimal()`에서 만든 680 codepoint/64 range golden으로 전체 Unicode 범위를 검사해 RED를 재현했다.
  또 `py_isalnum`이 같은 Rust Unicode 16.0 데이터로 Python에서 아직 배정하지 않은 U+1C89를 허용하는 RED도 재현했다.
  `py_decimal_digit`과 `py_isalnum`을 Python 3.13/Unicode 15.1 golden 범위표로 고정하고 CLI 거부 회귀를 추가했다.
  전체 Unicode 검증은 각각 decimal 680 codepoint/64 range, alphanumeric 138,557 codepoint/748 range를 대조했다.
  `pyfmt` 8개, `search` 11개, normalize comic names 11개, doc_reader text 14개가 통과했다.
  네 target `test_runner_rs`, Clippy, rustfmt와 `git diff --check`도 통과했다. pyfmt에서 더 쓰지 않는 직접 의존성은 제거했다.
  Python `utils/search.py`와 pytest 파일은 현재 없어 oracle fixture는 Python 3.13.7 built-in에서 직접 생성했다.
- `doc_reader::pytext::is_space`를 Python 3.13.7 `str.isspace()`의 전체 Unicode 15.1 범위와 대조했다.
  1,114,112개 codepoint 검사에서 차이는 없었다. `doc_reader` text target 15개, `test_runner_rs`, Clippy,
  rustfmt와 diff 검사가 통과했다. 이 검증은 현재 구현의 일치를 확인했으며 동작 코드는 바꾸지 않았다.
- comic directory NFKC가 Unicode 18 데이터를 써 Python 3.13의 Unicode 15.1에 미배정된 U+0558을 정규화하는 RED를 재현했다.
  Python 3.13.7에서 만든 전체 단일 codepoint NFKC oracle 4,928개 매핑과 모든 Unicode 15.1 미배정 scalar가
  결합문자 정규화 경계를 지키는지 검사했다. Python 15.1 배정 범위 707개를 생성해 미배정 문자를 보존하고
  주변 문자열을 별도 NFKC 처리하도록 했다. `unicode-normalization`은 0.1.26으로 유지해 PDF 암호 처리의
  `stringprep` transitive 동작에는 영향을 주지 않는다. `normalize_comic_directories` crate 44개와
  `test_runner_rs -f tests/rust/normalize_comic_directories/names.rs` 11개, Clippy, workspace fmt 및
  diff 검사가 통과했다. runner는 names target만 실행했고 성능 요약의 186개 target·3,202개 수치는 누적 통계다.
- `target/debug/loader`를 현재 셸에서 실행하면 `TM_BOOK_DIR` 누락으로 종료한다. 셸에는 ES 설정이 없지만
  저장소 `.env`에는 값이 있으며, 그 환경을 로드한 뒤 Rust `EsClient`로 production ES root GET이 성공했다.
  따라서 Rust TLS·인증 연결은 동작한다. 직접 실행할 때는 `.env`를 먼저 export해야 한다.
- 직접 표지 URL의 IDNA 결과를 Python 3.13.7 oracle와 비교했다. 64자 DNS label과 중간 빈 label을
  Python은 거부하지만 Rust IDNA helper가 통과시키는 차이를 새 테스트 RED로 재현했다. ASCII 변환 뒤
  label 길이(최대 63 byte)와 빈 label을 검사하고 마지막 root dot은 허용했다. 직접 URL target 14개,
  `test_runner_rs` target 14개, 관련 Clippy, workspace rustfmt와 `git diff --check`가 통과했다.
- Docker testcontainers를 사용해 disposable MySQL에서 DB backend 8개, proposal items 16개,
  proposal status 11개, view history 13개, cover status 7개를 실행해 모두 통과했다.
  `test_runner_rs -f tests/rust/backend/db.rs`도 기본 DB target 14개를 통과시켰다.
  testcontainers 테스트는 default run에서 ignore되므로 `--ignored --test-threads=1`로 별도 실행했다.
- Python 3.13/Unicode 15.1에서 미배정인 U+1C89를 Rust `to_lowercase()`가 새 소문자 U+1C8A로 바꾸는
  classifier tokenizer 차이를 RED 테스트로 재현했다. Python 3.13.7에서 만든 1,433개 전체 lowercase mapping과
  707개 assigned range를 이용해 `pyfmt::py_lowercase`를 추가했다. Python 15.1 미배정 문자는 보존하고,
  배정 구간은 따로 lowercase해 final sigma 같은 문맥 규칙을 유지한다. 모든 Unicode scalar mapping 및
  미배정 문자 경계 테스트, classifier tokenizer 회귀, pyfmt 3개 unit, classifier 66개와 parity 14개가 통과했다.
  `test_runner_rs` pyfmt 8개와 classifier 66개, 관련 crate Clippy, workspace fmt, diff 검사가 통과했다.
  전체 Clippy는 기존 `tests/rust/backend/cover.rs`의 `let_and_return` lint 2건으로 실패했으며,
  변경 관련 `pyfmt`·normalizer 전체 대상과 backend library Clippy는 통과했다. 대규모 모델 학습과 전체 회귀는 실행하지 않았다.
- Linux `file_time`의 statx 변환에서 Rust는 나노초를 직접 반올림했지만 Python은 `tv_sec + tv_nsec / 1e9`를
  float로 만든 뒤 `datetime.fromtimestamp()`에 넘긴다. Python 3.13.7에서 `(1700000000, 123457500)`은
  `123457µs`로 변환됨을 확인하고 기존 Rust 경계 테스트를 RED로 재현했다. Rust도 같은 float 변환 순서를 따르게 고쳤다.
  `file_time` target 11개, `test_runner_rs` 11개와 crate Clippy가 통과했다. Python pytest는 시스템 환경의 `pypdf`
  누락 및 `.venv` pytest/모듈 불일치로 실행되지 않았다. 전체 회귀는 실행하지 않았다.
- Toptoon 제목 검색에서 Rust Unicode 데이터가 Python 3.13보다 새로워 U+1C89 입력과 U+1C8A 제목을 같은 값으로
  소문자 변환하는 차이를 RED 테스트로 재현했다. Python 3.13.7은 두 문자를 다르게 유지한다.
  검색 정규화가 공용 `pyfmt::py_lowercase`를 쓰게 바꿨다. `backend::bookstore` `test_runner_rs` target 112개가
  통과했고 1개는 기존 환경 의존 ignore다. backend library와 bookstore target Clippy, workspace fmt 및 diff 검사가 통과했다.
  전체 회귀는 실행하지 않았다.
- 표지 관리 목록 정렬에서 Python은 `str.casefold()`를 사용하지만 Rust는 `to_lowercase()`를 사용해
  `ßeta`/`zeta`와 `ß.epub`/`z.epub` 순서가 달라지는 RED 테스트를 추가했다. 디렉터리·파일 정렬 키를
  `pyfmt::py_casefold`로 바꿨다. `backend::cover_management` `test_runner_rs` target 70개와 backend library·target
  Clippy, workspace rustfmt 및 diff 검사가 통과했다. 전체 회귀는 실행하지 않았다.
- 관리자 유사 이름 검색과 bulk rename의 정규화·정렬이 Python `casefold()` 대신 Rust `to_lowercase()`를 쓰는
  차이를 찾았다. `Straße`/`STRASSE` 점수와 `ß`/`z` 정렬의 RED 테스트를 추가하고 세 경로를 `pyfmt::py_casefold`로
  바꿨다. `main_gaps` 26개와 `main_bulk_rename` 17개 `test_runner_rs` target, 해당 backend library·target Clippy,
  workspace rustfmt 및 diff 검사가 통과했다. 전체 회귀는 실행하지 않았다.
- 운영 classifier corpus의 층화 표본(category별 최대 100건, 7,698건)으로 Python 학습을 재개했다.
  Python은 `n_jobs=1`에서 84초 fit, 최대 RSS 0.75 GB, holdout top-1 71.0%를 기록했다.
  Rust는 같은 train/holdout 분할과 feature 수를 확인했지만 `n_jobs=1` 10분 24초 및 `n_jobs=8` 5분 14초 뒤에도
  fit을 끝내지 못해 중단했다. 각 실행 최대 RSS는 약 0.20 GB였다. 성능·정확도 parity는 미완료이며 solver 최적화가 필요하다.
- 위 Rust 시간은 spill 행을 매 epoch 디코딩하던 초기 실행이다. release build를 사용하고 학습 직전에 sparse matrix를
  memory-backed로 materialize하도록 바꿨다. spill matrix materialization 보존 테스트와 classifier 학습 golden 테스트가 통과했다.
  같은 표본에서 Rust `n_jobs=8`, `max_iter=100`은 62초, holdout top-1 69.1%, 최대 RSS 0.23 GB였고,
  `max_iter=500`은 4분 4초, 70.7%, 0.23 GB였다. 당시 Python 기본 `max_iter=2000` 결과와 비교하면
  아직 동등한 설정의 parity는 아니다. Rust release `max_iter=2000`도 10분 내 완료하지 못해 중단했다.
  solver threshold `1e-4`는 golden score 허용 오차를 넘어 실패해 채택하지 않았고, `1e-5`는 golden fixture를 통과했다.
  운영 corpus 전체 학습 및 성능·정확도 parity는 여전히 미완료다.
- liblinear dual coordinate descent 흐름에 맞춰 매 epoch 문서 순서를 결정적으로 섞고, projected-gradient 기준으로
  active set을 줄이도록 바꿨다. TDD에서 shuffle helper 부재를 RED로 확인한 뒤 구현했고, memory-backed row를 직접 읽어
  epoch마다 spill 파일을 다시 순회하지 않는다. Python `max_iter=2000` 설정의 동일한 7,613건 표본에서 Rust release 학습은
  45.94초, 최대 RSS 0.23GB였고 Python은 84초, 0.75GB였다. 두 쪽 holdout 1,523건이 같고 top-1은 모두 70.6500%였다.
  coverage curve precision은 모두 같았고 confidence threshold의 최대 차이는 `9.5e-8`이었다.
  새 solver에서 tolerance `1e-4`도 Python golden fixture를 통과한다.
- 전체 corpus 222,348건에서도 Rust release 학습을 완료했다. 7개 소표본 category 제외 후 222,263건,
  80개 category 중 177,809건 train과 44,454건 holdout으로 나뉘었다. 952,887개 feature와 353,714,631개 nonzero를
  만들었고 vectorization은 315초, SVM fit은 3,887초, 전체 실행은 66분 14초였다. 최대 RSS는 5.9GB다.
  holdout top-1은 86.42%, precision 90% 이상을 유지하는 coverage는 91.0%였다.
  같은 전체 corpus에서 Python full fit은 이전 실행 때 66분 뒤에도 끝나지 않아 중단했으므로 full-data score parity는
  아직 직접 비교하지 않았다. 같은 Python/Rust holdout을 쓴 7,613건 표본에서는 top-1과 precision curve가 일치했다.
- Python 전체 회귀 `uv run --no-sync python -m pytest tests/`는 3,051개 통과했다.
  Rust workspace 회귀 `cargo test --workspace --exclude pdfscan --exclude epubscan --locked`도 통과했다.
  첫 실행의 HWP 테스트 1건 실패는 격리 실행과 `doc_reader` 전체 target에서 재현되지 않았고, 재실행 workspace에서는 통과했다.
  이 두 scanner crate는 이미 Rust 구현이 있으며 해당 workspace 명령에서만 제외했다.
- `detect_mojibake`의 `epub-structure` 복원본(위 "복원했다" 항목)은 `epubscan structure`와 같은 기능이라 삭제했다.
  `epubscan`은 이미 `detect_mojibake`의 EPUB 구조 검사 테스트를 이식해 갖고 있었다. 실제 EPUB 70개와 이슈 코드를 다양하게
  만드는 합성 파일 10개(비 zip, 빈 파일, 없는 파일, mimetype 누락·압축, container 누락, OPF 파싱 실패, spine 누락, 접두어 루트)에서
  두 도구의 JSON 출력이 80개 모두 같았다. 지운 것은 `epub_structure.rs`, 그 테스트, `cli.rs`·`lib.rs`·`Cargo.toml`의 연결 코드다.
  EPUB 구조 검사는 `epubscan structure` 한 곳에서만 유지한다. `detect_mojibake`는 커밋된 `HEAD`와 같아졌다.
  `corpus_common`의 `decode_korean_replace` 변경은 이 삭제와 무관하게 자체 테스트가 있어 그대로 두었다.
