# 서점 근거 합산 구현 계획

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 서점 세 곳(yes24, aladin, kyobo)의 합의 정도를 0~1 점수로 바꿔 분류 제안 합산 점수에 더한다.

**Architecture:** 카테고리 대응·표·합의 점수는 `libs/book_manager`의 순수 함수다. 서점 조회와 캐시는 backend 모듈이 `BookstoreLookup` trait으로 구현해 `BookManagerDeps`에 꽂는다(모델 `Classify`와 같은 방식). 캐시는 MySQL 테이블 두 개다. 편집 화면의 JS 대응 로직은 backend 엔드포인트로 옮기고 지운다.

**Tech Stack:** Rust workspace(axum, diesel MySQL), React(vitest).

**Spec:** `docs/superpowers/specs/2026-11-08-bookstore-evidence-design.md`

## Global Constraints

- 새 외부 의존성 없음. 워크스페이스 안 crate 사이 path 의존만 더한다.
- 서점 대응 로직은 JS와 동작이 같아야 한다. 새 규칙을 만들지 않는다. 11-04에 지운 `map_category`·`bookstore_policy`를 되살리지 않는다.
- 서점: `STORES = ["yes24", "aladin", "kyobo"]`. 만화(`content_type == "comic"`)에는 서점 근거를 쓰지 않는다.
- 합의 점수: `AGREEMENT_SCORES = [0.0, 1.0/3.0, 2.0/3.0, 1.0]`(표 수 0~3). 분모는 언제나 3.
- 가중치: ISBN 0.45, 서점 0.30, 키워드 0.15, 모델 0.10. `EVIDENCE_WEIGHTS` 한 곳에만 둔다.
- 캐시: 조회 순서 inode → ISBN → 서점 검색. 결과 있음은 만료 없음, 결과 없음은 30일. 차단·오류는 적지 않는다.
- 연속 차단 상한: 서점별 20번이면 그 작업에서 그 서점을 더 부르지 않는다.
- `use_bookstore` 기본 `true`. `delay`(기본 1.2초)는 서점을 실제로 부른 파일 뒤에만 둔다.
- 테스트는 `tests/rust/<crate>/<모듈>.rs` 1:1, 흐름은 `integration/<시나리오>.rs`. 모듈 이름은 "목적어*행위자". Rust 테스트는 `test_runner_rs`로만 돌리고 실행 전 `TM*\*` env를 비운다.
- 모든 Rust 변경 뒤 `cargo clippy --workspace --all-targets` 경고 0건. 프런트엔드 변경 뒤 `cd frontend && npm test` 통과.
- 커밋: Conventional Commits 제목, `-` bullet 본문, 명령형, 72자 이내.

## Review Focus

1. **세 서점 모두 차단** — 서점 근거 없음으로 처리하고 점수를 깎지 않아야 한다. Task 3에 "모두 차단이면 `bookstore_scores`가 `None`" 테스트를 둔다.
2. **ISBN 없는 책의 이름 변경** — 옛 검색어 결과를 쓰지 않고 다시 검색해야 한다. Task 5에 "inode 행의 검색어가 다르면 재검색" 테스트를 둔다.
3. **같은 ISBN 사본 여러 개** — 서점은 한 번만 불러야 한다. Task 5에 "두 inode, 같은 ISBN → 서점 호출 1번" 테스트를 둔다.
4. **inode 재사용** — inode 행의 ISBN과 지금 ISBN이 다르면 그 행을 쓰지 않아야 한다. Task 5에 테스트를 둔다.
5. **서점 카테고리가 어디에도 대응되지 않음** — 그 서점은 기권이어야 한다. Task 3에 "점수 0인 대응 → `abstain: no_match`" 테스트를 둔다.

---

### Task 1: 서점 카테고리 대응 로직 이전 — `libs/book_manager/src/store_category_matcher.rs`

**Files:**

- Create: `libs/book_manager/src/store_category_matcher.rs`, `tests/rust/book_manager/store_category_matcher.rs`, `tests/rust/book_manager/fixtures/store_category_scores.json`
- Create (일회성, 커밋함): `frontend/tests/storeCategoryFixture.gen.test.jsx` — 기존 JS 함수로 fixture JSON을 만든다
- Modify: `libs/book_manager/src/lib.rs`, `libs/book_manager/Cargo.toml`(`[[test]]`)

**Interfaces:**

- Produces:
  - `two_level_category(category: &str) -> String` (`getTwoLevelCategory`)
  - `multi_path_categories(category: &str) -> Vec<String>` (`extractMultiPathCategories`)
  - `calculate_similarity(a: &str, b: &str) -> f64` (`calculateSimilarity`)
  - `category_scores(store_category: &str, categories: &[String], mappings: &BTreeMap<String, Vec<String>>) -> BTreeMap<String, f64>` (`calculateCategoryScores`)
  - `top_similar_categories(suggested: &BTreeMap<String, String>, categories: &[String], mappings: &BTreeMap<String, Vec<String>>, top_n: usize) -> Vec<String>` (`findTopSimilarCategories`, 동점은 JS의 정렬 안정성대로 입력 순서)
  - `similarity_debug(suggested, categories, mappings, top_n) -> SimilarityDebug` (`getSimilarityDebugInfo`와 같은 필드, serde 직렬화 키도 JS와 같게)

- [ ] **Step 1: fixture 생성** — `storeCategoryFixture.gen.test.jsx`가 실제 `Actions.jsx`·`Bookstore.jsx` 함수(`calculateSimilarity`, `getTwoLevelCategory`, `extractMultiPathCategories`, `getSimilarityDebugInfo`, 그리고 `findTopSimilarCategories`는 export를 더해 호출)를 불러 30개 이상의 사례를 JSON으로 쓴다. 사례에는 `||` 다중 경로, `일반` 노이즈, 한글 1글자 포함, 부분문자열 키워드, 키워드 11개 이상, 키워드 매핑이 있는 카테고리를 넣는다. `cd frontend && npx vitest run tests/storeCategoryFixture.gen.test.jsx` 실행 후 JSON 생성 확인
- [ ] **Step 2: 실패 테스트** — `fixture_cases_match_js_implementation`이 fixture의 모든 사례에서 Rust 결과를 비교한다(점수는 `abs < 1e-12`, 목록은 동일)
- [ ] **Step 3: 실패 확인** — `test_runner_rs -f tests/rust/book_manager/store_category_matcher.rs` → 컴파일 실패
- [ ] **Step 4: 구현** — JS 함수를 한 줄씩 옮긴다. 문자열 길이·부분문자열·n-gram은 JS처럼 UTF-16 code unit으로 센다(`encode_utf16`). 한글은 BMP라 `char`와 같지만 BMP 밖 문자에서 갈린다. **fixture가 다르게 나오면 JS 기준에 맞춘다**
- [ ] **Step 5: 통과 확인** — 같은 명령 PASS
- [ ] **Step 6: 커밋** — `feat: 서점 카테고리 대응 로직을 backend로 옮긴다`

### Task 2: 파일 이름 → 검색어 — `libs/book_manager/src/bookstore_query_builder.rs`

**Files:**

- Create: `libs/book_manager/src/bookstore_query_builder.rs`, `tests/rust/book_manager/bookstore_query_builder.rs`
- Modify: `libs/book_manager/src/lib.rs`, `libs/book_manager/Cargo.toml`

**Interfaces:**

- Produces:
  - `pub struct BookQuery { pub isbn13: Option<String>, pub title: String, pub author: String }` (`Clone, Debug, PartialEq`)
  - `normalize_isbn13(raw: &str) -> Option<String>` — 하이픈·공백 제거, ISBN-10이면 13으로 변환, `isbn_extractor::validate_isbn13` 통과만
  - `query_from_file(file_stem: &str, raw_isbn: Option<&str>) -> BookQuery` — 저자는 `stripComicDirectoryAuthor`가 떼는 `[저자]`·`(저자)`·`제목 [저자]`·`제목 @ 저자` 부분, 작품명은 `stripComicDirectoryMeta(stripComicDirectoryAuthor(..))`와 같은 규칙
  - `BookQuery::title_key(&self) -> String` — `"{author}\u{1f}{title}"`, inode 캐시 비교용

- [ ] **Step 1: 실패 테스트** — 사례: `"[홍길동] 검과 마법 1권"` → (`홍길동`, `검과 마법`); `"개구리 인간 1~47화[완결]"` → (``, `개구리 인간`); `"제목 @ 저자"`; `"(저자) 제목 (1-34화)"`; ISBN `"89-7059-123-X"`류 ISBN-10 → 13 변환; 잘못된 ISBN → `None`
- [ ] **Step 2: 실패 확인** — `test_runner_rs -f tests/rust/book_manager/bookstore_query_builder.rs`
- [ ] **Step 3: 구현** — JS 정규식을 `regex`가 아니라 기존 crate가 쓰는 방식으로 옮긴다(이미 `regex`를 쓰면 그것을 쓴다. 새 의존성 금지)
- [ ] **Step 4: 통과 확인**
- [ ] **Step 5: 커밋** — `feat: 파일 이름에서 서점 검색어를 만든다`

### Task 3: 서점 표와 합의 점수, 합산 — `libs/book_manager/src/bookstore_evidence.rs`, `evidence_scorer.rs`, `category_proposer.rs`

**Files:**

- Create: `libs/book_manager/src/bookstore_evidence.rs`, `tests/rust/book_manager/bookstore_evidence.rs`
- Modify: `libs/book_manager/src/evidence_scorer.rs`, `libs/book_manager/src/category_proposer.rs`, `libs/book_manager/src/lib.rs`(deps에 `with_bookstore_lookup_factory`), `tests/rust/book_manager/evidence_scorer.rs`, `tests/rust/book_manager/category_proposer.rs`

**Interfaces:**

- Consumes: Task 1 `category_scores`, `multi_path_categories`; Task 2 `BookQuery`, `query_from_file`
- Produces:
  - `pub const STORES: [&str; 3]`, `pub const AGREEMENT_SCORES: [f64; 4]`
  - `pub enum LookupStatus { Found, NotFound, Blocked, Error }`
  - `pub struct StoreRow { pub title: String, pub author: String, pub category: String }`
  - `pub struct StoreResult { pub store: String, pub status: LookupStatus, pub rows: Vec<StoreRow> }`
  - `pub trait BookstoreLookup: Send + Sync { fn lookup(&self, inode: i64, query: &BookQuery) -> LookupOutcome; }` — `pub struct LookupOutcome { pub results: Vec<StoreResult>, pub network_used: bool }`
  - `pub type BookstoreLookupFactory = Arc<dyn Fn() -> Option<Arc<dyn BookstoreLookup>> + Send + Sync>`; `BookManagerDeps::with_bookstore_lookup_factory(self, BookstoreLookupFactory) -> Self`
  - `pub struct StoreVote { pub store: String, pub category: Option<String>, pub abstain: Option<String> }` — `abstain`은 `not_found`/`blocked`/`error`/`no_match`/`tie`/`excluded`
  - `store_vote(result: &StoreResult, categories: &[String], mappings: &BTreeMap<String, Vec<String>>, allowed: &dyn Fn(&str) -> bool) -> StoreVote` — 행마다 `multi_path_categories`로 모은 문자열의 `category_scores`를 카테고리별로 더해 1등
  - `bookstore_scores(votes: &[StoreVote]) -> Option<Vec<(String, f64)>>` — 표가 하나도 없으면 `None`
  - `Evidence::Bookstore`(`name()` = `"bookstore"`, label `"서점"`)
  - `Proposal.bookstore_votes: Vec<StoreVote>`(직렬화 키 `bookstore_votes`, 서점 근거를 안 썼으면 빈 배열)
  - `propose_category_for_file_with(&self, file_path, source_category, mappings, model, bookstore: Option<&dyn BookstoreLookup>, categories: &[String]) -> (Proposal, bool /*network_used*/)`; 기존 `propose_category_for_file`은 `bookstore = None`으로 위임

- [ ] **Step 1: 실패 테스트 (`bookstore_evidence.rs`)** — 설계 "3. 서점 점수" 표의 6행 각각; 차단·결과 없음·오류는 `abstain` 각각; 동점 → `tie`; 1등이 `0_` → `excluded`; 대응 점수 0 → `no_match`; 1등이 현재 카테고리면 표(제자리)
- [ ] **Step 2: 실패 테스트 (`evidence_scorer.rs`)** — `weights_follow_isbn_keyword_model_order`를 새 표 `[(Isbn,0.45),(Bookstore,0.30),(Keyword,0.15),(Model,0.10)]`로 바꾼다; 서점 `[("A",2/3),("B",1/3)]` + 모델 `[("A",0.4),("B",0.6)]` → A = (0.30·2/3 + 0.10·0.4)/0.40 = 0.6, B = 0.4
- [ ] **Step 3: 실패 테스트 (`category_proposer.rs`)** — 대역 `FixedLookup`으로: 세 서점 합의 → 1위 `source == "bookstore"`, `bookstore_votes` 3개; 모두 차단 → 서점 근거 없음(점수는 다른 근거만으로); 만화 content_type → 조회 안 함; `use_bookstore=false` → 조회 안 함. 기존 `proposal_top_two_follow_combined_score`의 기대값을 새 가중치로 다시 계산한다
- [ ] **Step 4: 실패 확인** — 세 파일 각각 `test_runner_rs -f`
- [ ] **Step 5: 구현** — `propose_category_changes`는 제안 시작 때 팩토리를 한 번 부르고, 대응용 카테고리 목록(디스크 최상위 카테고리)을 한 번 만든다. 파일마다 `query_from_file(file_stem, ES isbn)`으로 검색어를 만들어 `lookup`한다. 표의 `allowed`는 `is_recommendable_category && is_top_level_target_category && category_dir().is_dir()`이다. `network_used`면 `delay`초 쉰다(쉬는 방법은 BookManager의 기존 clock seam을 쓴다)
- [ ] **Step 6: 통과 확인** — 세 파일 PASS, 그다음 옵션 없이 `test_runner_rs`
- [ ] **Step 7: 커밋** — `feat: 서점 세 곳의 합의를 분류 제안 점수에 더한다`

### Task 4: 캐시 저장소 — `libs/db/src/bookstore_cache_store.rs`

**Files:**

- Create: `libs/db/src/bookstore_cache_store.rs`, `tests/rust/db/bookstore_cache_store.rs`, `tests/rust/db/integration/bookstore_cache_store.rs`
- Modify: `libs/db/src/lib.rs`, `libs/db/Cargo.toml`

**Interfaces:**

- Produces:
  - `pub struct CachedLookup { pub status: String /*found|not_found*/, pub rows_json: String, pub method: String, pub query: String, pub isbn13: Option<String>, pub fetched_at: i64 }`
  - `BookstoreCacheStore::new() -> Result<Self, DbError>`(테이블 생성), `with_connection_provider(ConnectionProvider) -> Self`
  - `get_by_inode(&self, inode: i64, store: &str, now: i64) -> Result<Option<CachedLookup>, DbError>`; `get_by_isbn(&self, isbn13: &str, store: &str, now: i64) -> Result<Option<CachedLookup>, DbError>` — 만료된 행은 `None`
  - `put_inode(&self, inode, store, &CachedLookup) -> Result<(), DbError>`; `put_isbn(&self, isbn13, store, &CachedLookup) -> Result<(), DbError>` — upsert. `not_found`면 `expires_at = fetched_at + 30일`, `found`면 `NULL`
- 테이블:
  - `bookstore_inode_cache(inode BIGINT, store VARCHAR(16), method VARCHAR(16), query VARCHAR(1024), isbn13 VARCHAR(13) NULL, status VARCHAR(16), rows_json MEDIUMTEXT, fetched_at BIGINT, expires_at BIGINT NULL, PRIMARY KEY(inode, store))`
  - `bookstore_isbn_cache(isbn13 VARCHAR(13), store VARCHAR(16), status VARCHAR(16), rows_json MEDIUMTEXT, fetched_at BIGINT, expires_at BIGINT NULL, PRIMARY KEY(isbn13, store))`
  - `ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci`, `view_history_store.rs`와 같은 `CREATE TABLE IF NOT EXISTS` 방식

- [ ] **Step 1: 실패 테스트(mock 연결)** — upsert SQL·인자, 만료 판정(`not_found` 30일 경계 전후), `found`는 만료 없음
- [ ] **Step 2: 실패 테스트(컨테이너)** — put → get 왕복, 같은 키 재기록이 덮어씀. 컨테이너를 못 띄우면 건너뛴다(기존 `mysql_fixture` 규칙)
- [ ] **Step 3: 실패 확인** — `test_runner_rs -f` 두 파일
- [ ] **Step 4: 구현**
- [ ] **Step 5: 통과 확인**
- [ ] **Step 6: 커밋** — `feat: 서점 검색 결과를 inode·ISBN으로 캐시하는 테이블을 추가한다` (본문에 DB 스키마 추가를 적는다)

### Task 5: 조회·캐시 조립 — `backend/src/bookstore_result_fetcher.rs`

**Files:**

- Create: `backend/src/bookstore_result_fetcher.rs`, `tests/rust/backend/bookstore_result_fetcher.rs`
- Modify: `backend/src/lib.rs`, `backend/src/api_router/app_deps.rs`(팩토리 조립), `backend/src/api_router/book_routes/category_proposer_routes.rs`(`use_bookstore` 전달, 상태에 서점별 집계), `backend/Cargo.toml`

**Interfaces:**

- Consumes: Task 3 `BookstoreLookup`, `LookupOutcome`, `StoreResult`, `STORES`; Task 4 `BookstoreCacheStore`; 기존 `bookstore::store_clients::BookstoreSearch`
- Produces:
  - `pub trait CacheStore: Send + Sync`(Task 4의 네 메서드, 테스트 대역용)
  - `BookstoreResultFetcher::new(stores: Vec<(String, Box<dyn BookstoreSearch>)>, cache: Arc<dyn CacheStore>, clock: Arc<dyn Clock>) -> Self`, `impl BookstoreLookup`
  - `BookstoreResultFetcher::counts(&self) -> BTreeMap<String, StoreCounts>`(`found`/`not_found`/`blocked`/`error`/`cache_hit`/`skipped`)

- [ ] **Step 1: 실패 테스트** — 대역 서점·캐시로:
  - inode 적중 → 서점 호출 0번, `network_used == false`
  - inode 행이 `title_author`이고 `query`가 지금 `title_key()`와 다름 → 재검색
  - inode 행의 `isbn13`이 있고 지금 ISBN과 다름 → 그 행 무시
  - ISBN 캐시 적중 → 서점 호출 0번, inode 캐시에 기록
  - 두 inode, 같은 ISBN → 서점 호출 1번
  - ISBN 검색 결과 → 두 캐시에 기록; 저자명+작품명 검색 결과 → inode 캐시에만
  - 차단·오류 → 캐시에 안 씀
  - 한 서점이 연속 20번 차단 → 21번째부터 호출 안 함(`skipped`)
- [ ] **Step 2: 실패 확인** — `test_runner_rs -f tests/rust/backend/bookstore_result_fetcher.rs`
- [ ] **Step 3: 구현** — 세 서점은 `std::thread::scope`로 병렬 호출한다. ISBN 검색은 `search(isbn13, "", "")`, 이름 검색은 `search("", title, author)`. 결과 행의 `category`만 남겨 `rows_json`에 적는다. `app_deps`는 `TM_MYSQL_*`로 캐시를 열고, 실패하면 경고를 남기고 캐시 없이(매번 검색) 동작한다
- [ ] **Step 4: 라우트 연결** — `run_classify_proposal_job`이 `use_bookstore`를 `ProposeOptions`로 넘기고, 끝날 때 `counts()`를 상태 `bookstore_counts`로 병합한다. 기존 `category_proposer_routes` 테스트와 `tests/rust/backend/integration/classify_proposal_jobs.rs`가 통과해야 한다
- [ ] **Step 5: 통과 확인** — `-f` 단독, 그다음 옵션 없이 `test_runner_rs`
- [ ] **Step 6: 커밋** — `feat: 분류 제안에서 서점을 캐시 우선으로 조회한다`

### Task 6: 편집 화면이 backend 대응 로직을 쓴다

**Files:**

- Modify: `backend/src/api_router/bookstore_routes.rs`(엔드포인트 추가), `frontend/src/Actions.jsx`, `frontend/src/SimilarityDebug.jsx`, 관련 frontend 테스트
- Delete: `Actions.jsx`의 대응 함수(`generateNgrams`~`calculateCategoryScores`, `findTopSimilarCategories`, `getSimilarityDebugInfo`), `frontend/tests/storeCategoryFixture.gen.test.jsx`
- Test: `tests/rust/backend/bookstore_routes.rs`(없으면 만든다), `frontend/tests/Actions.test.jsx`, `frontend/tests/SimilarityDebug.test.jsx`

**Interfaces:**

- Consumes: Task 1 `top_similar_categories`, `similarity_debug`
- Produces: `POST /bookstore/category-suggestions` (`require_admin`), body `{suggested: {key: string}, categories: [string], top_n?: number, debug?: bool}`, 응답 `{"status":"success","result":{"top":[...],"debug":{...}|null}}`. 키워드 매핑은 서버가 `category_keywords`에서 읽는다(`content_type` 쿼리, 기본 `book`)

- [ ] **Step 1: 실패 테스트(backend)** — 인증 없으면 401, viewer면 403, 정상 요청이 Task 1 함수와 같은 결과
- [ ] **Step 2: 실패 테스트(frontend)** — `Actions`가 엔드포인트 응답의 `top`으로 강조 카테고리를 그린다; `SimilarityDebug`가 `debug` 응답을 그린다; 요청 실패 시 강조 없음
- [ ] **Step 3: 실패 확인** — `test_runner_rs -f ...`, `cd frontend && npx vitest run tests/Actions.test.jsx tests/SimilarityDebug.test.jsx`
- [ ] **Step 4: 구현** — 엔드포인트를 만들고 frontend가 그것을 부르게 바꾼 뒤 JS 대응 함수를 지운다
- [ ] **Step 5: 통과 확인** — 위 명령 PASS, `cd frontend && npm test` 전체 PASS
- [ ] **Step 6: 커밋** — `refactor: 편집 화면의 서점 카테고리 추천을 backend로 옮긴다`

### Task 7: 분류 제안 화면

**Files:**

- Modify: `frontend/src/CategoryAdmin.jsx`(제안 시작 체크박스, 요청 `use_bookstore`), `frontend/src/ClassifyProposalTable.jsx`(점수 툴팁에 서점별 표, 근거 이름 `서점`)
- Test: `frontend/tests/CategoryAdmin.test.jsx`, `frontend/tests/ClassifyProposalTable.test.jsx`

- [ ] **Step 1: 실패 테스트** — 체크박스 기본 체크, 끄면 요청 body에 `use_bookstore: false`; 툴팁에 `yes24 3_SF · aladin 3_SF · kyobo 기권(차단)` 모양; `bookstore_votes`가 없는 옛 payload도 깨지지 않음
- [ ] **Step 2: 실패 확인** — `cd frontend && npx vitest run tests/CategoryAdmin.test.jsx tests/ClassifyProposalTable.test.jsx`
- [ ] **Step 3: 구현**
- [ ] **Step 4: 통과 확인** — 같은 명령, 그다음 `npm test`
- [ ] **Step 5: 커밋** — `feat: 분류 제안 화면에 서점 조회 선택과 서점별 표를 보인다`

### Task 8: 최종 검증

- [ ] `test_runner_rs`(증분), `cargo clippy --workspace --all-targets`, `cd frontend && npm test`. 결과를 그대로 보고한다
- [ ] 배포·머지는 사용자 승인 뒤에 한다
