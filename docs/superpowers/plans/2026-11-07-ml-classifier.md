# 머신러닝 도서 분류기 구현 계획

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** ES 문서로 학습한 softmax 모델과 ISBN·키워드 근거를 가중치 합산해, 분류 제안의 추천 1·2를 점수 내림차순으로 채운다.

**Architecture:** `libs/classifier`가 특징 해시·모델·핫 리로드를 갖는 순수 crate가 된다(ES 모름). `utils/classifier_trainer`가 ES에서 균등 표본을 뽑아 학습하고 모델 파일을 원자적으로 쓴다. `libs/book_manager`의 제안 단계가 파일마다 ES 문서를 한 번 읽어 ISBN·키워드·모델 근거를 합산하고, 등급 체계를 없앤다. frontend는 등급 대신 점수와 `classified`를 쓴다.

**Tech Stack:** Rust 2024 workspace, `rand`/`rand_chacha` 0.9, `serde_json`, Elasticsearch(`search_manager`), React + vitest

**Spec:** `docs/superpowers/specs/2026-11-07-ml-classifier-design.md`

## Global Constraints

- 새 의존성은 `rand = "0.9"`, `rand_chacha = "0.9"`만 허용한다. 그 밖에는 사용자 승인이 필요하다.
- 학습 대상: `file_path` 첫 경로 조각이 카테고리다. `0_`로 시작하는 카테고리, `9_북스캔OCR`, 경로 조각이 하나뿐인 루트 낱개 파일은 뺀다.
- 학습 입력은 ES `summary`(앞 1,500자), `title`, `author`만 쓴다. `file_path`·`category`·`isbn`은 특징으로 쓰지 않는다.
- 문서 키는 inode(ES `_id`, `book_id`)다. `file_path`로 문서를 조회하지 않는다.
- 초기 가중치: ISBN 0.6, 키워드 0.25, 모델 0.15. 상수 표 한 곳에만 둔다.
- 모델 파일 경로 env: `TM_CLASSIFIER_MODEL`. pod 경로 `/app/models/classifier/model.bin`, 호스트 `/home/terzeron/workspace/tm/models/classifier`.
- 모델 크기 64MB 이하. 해시 크기 기본 2^17.
- `/mnt/data/text`에 쓰지 않는다.
- 모든 Rust 바이너리는 인자 없이 실행하면 `print_usage()`를 출력하고 코드 1로 끝난다.
- 테스트는 `tests/rust/<crate>/<모듈>.rs` 1:1, CLI·흐름은 `integration/<시나리오>.rs`(target 이름 `integration_<시나리오>`). 모듈 이름은 "목적어\_행위자"다.
- Rust 테스트 실행은 `test_runner_rs`로만 한다(`cargo test` 금지). 실행 전 셸의 `TM_*` env를 비운다(`env -u TM_ES_URL -u TM_MYSQL_HOST ...` 또는 `setup_env`가 덮어쓰는지 확인). 단독 확인은 `test_runner_rs -f <테스트 파일>`, 그 뒤 옵션 없이 증분 실행.
- 커밋 메시지: Conventional Commits 제목, `-` bullet 본문, 명령형, 72자 이내. 실행하지 않은 검사를 통과했다고 쓰지 않는다.

## Review Focus

1. **모델 파일이 학습 도중 교체되거나 깨진 경우** — backend는 이전 모델을 계속 쓰고 경고만 남겨야 한다. Task 3에 "깨진 파일로 교체 → 이전 모델 유지" 테스트를 둔다.
2. **카테고리 목록이 모델과 디스크에서 다른 경우**(학습 뒤 카테고리 이름 변경·삭제) — 모델이 없는 카테고리를 추천하면 이동이 실패한다. Task 6에 "모델 후보 중 존재하지 않는 카테고리 디렉토리는 버린다" 테스트를 둔다.
3. **ES 문서는 있는데 `summary`가 빈 책(스캔 PDF 등)** — 모델 근거가 균등 분포에 가까운 잡음을 내면 사전확률 1등(큰 카테고리)만 계속 추천한다. Task 6에 "`summary`·`title`이 모두 비면 모델 근거 없음" 테스트를 둔다.
4. **ISBN이 흔한 값인 경우**(잘못 추출된 `0000000000` 등으로 수백 건 일치) — 한 ISBN이 합산을 지배한다. Task 4에서 ISBN 일치 조회 상한 50건, Task 6에서 "일치가 50건 이상이면 ISBN 근거 없음" 테스트를 둔다.
5. **예전 제안 payload(`grade` 있음, `classified` 없음)가 DB에 남은 채 화면 진입** — "분류 중…"에 갇히면 안 된다. Task 8에 호환 테스트를 둔다.

---

### Task 1: 특징 해시 — `libs/classifier/src/feature_hasher.rs`

**Files:**

- Create: `libs/classifier/src/feature_hasher.rs`
- Modify: `libs/classifier/src/lib.rs` (모듈 선언·재수출), `libs/classifier/Cargo.toml` (`[[test]] feature_hasher`)
- Test: `tests/rust/classifier/feature_hasher.rs`

**Interfaces:**

- Produces:
  - `pub struct BookFields { pub summary: String, pub title: String, pub author: String }`
  - `impl BookFields { pub fn from_es_source(source: &serde_json::Value) -> BookFields }` — 없는 필드는 빈 문자열. `author`가 배열이면 쉼표로 이어 붙인다.
  - `impl BookFields { pub fn is_empty(&self) -> bool }` — `summary`와 `title`이 모두 공백뿐이면 참.
  - `pub type SparseVec = Vec<(u32, f32)>` — 인덱스 오름차순, 중복 없음.
  - `pub const SUMMARY_MAX_CHARS: usize = 1500;`
  - `pub fn hash_features(fields: &BookFields, hash_bits: u32) -> SparseVec`

규칙: 공백을 한 칸으로 접고 소문자화한다. `summary`(앞 1,500자)와 `title`은 글자 2-gram·3-gram, `author`는 쉼표로 나눠 trim한 이름 하나가 토큰 하나다. 필드마다 namespace 문자열(`"s:"`, `"t:"`, `"a:"`)을 앞에 붙여 해시한다. 해시는 FNV-1a 64비트를 직접 구현한다(`std` `DefaultHasher`는 버전마다 바뀔 수 있어 금지). 값은 `ln(1 + tf)`이고, 필드별로 L2 정규화한 뒤 합친다. 같은 버킷에 떨어진 값은 더한다.

- [ ] **Step 1: 실패하는 테스트 작성**

```rust
#[test] fn same_input_gives_identical_vector() // 두 번 호출 결과가 같다
#[test] fn namespaces_keep_title_and_summary_apart() // 같은 글 "헌터"를 title에만 둔 벡터 ≠ summary에만 둔 벡터
#[test] fn summary_is_cut_at_1500_chars() // 1500자 뒤에만 있는 글자는 결과를 바꾸지 않는다
#[test] fn each_field_is_l2_normalized() // summary만 있는 문서의 값 제곱합 ≈ 1.0 (오차 1e-5)
#[test] fn indices_are_sorted_unique_and_below_bucket_count() // hash_bits=10 → 모든 인덱스 < 1024
#[test] fn author_array_and_string_hash_the_same() // {"author":["가","나"]} == {"author":"가, 나"}
#[test] fn fnv1a_matches_known_vector() // fnv1a64(b"a") == 0xaf63dc4c8601ec8c
#[test] fn empty_title_and_summary_is_empty() // author만 있으면 is_empty() == true
```

- [ ] **Step 2: 실패 확인** — `test_runner_rs -f tests/rust/classifier/feature_hasher.rs` → 컴파일 실패(모듈 없음)
- [ ] **Step 3: 구현** — 위 시그니처대로 `feature_hasher.rs` 작성
- [ ] **Step 4: 통과 확인** — 같은 명령 → PASS
- [ ] **Step 5: 커밋** — `feat: 분류기 특징 해시를 추가한다`

---

### Task 2: softmax 모델과 파일 형식 — `libs/classifier/src/softmax_model.rs`

**Files:**

- Create: `libs/classifier/src/softmax_model.rs`
- Modify: `libs/classifier/src/lib.rs`, `libs/classifier/Cargo.toml` (`[[test]] softmax_model`, dev-dep `tempfile` — 이미 workspace의 다른 crate가 쓰는 버전 `3.27.0`)
- Test: `tests/rust/classifier/softmax_model.rs`

**Interfaces:**

- Consumes: Task 1 `BookFields`, `hash_features`, `SparseVec`
- Produces:
  - `pub struct ModelHeader { pub version: u32, pub seed: u64, pub created_unix_secs: u64, pub hash_bits: u32, pub categories: Vec<String>, pub population_counts: Vec<u64>, pub sample_counts: Vec<u64> }` (serde)
  - `pub struct SoftmaxModel { pub header: ModelHeader, pub weights: Vec<f32> /* categories.len() × 2^hash_bits, 카테고리 major */, pub bias: Vec<f32> }`
  - `impl SoftmaxModel {`
    - `pub fn logits(&self, x: &SparseVec) -> Vec<f32>` — 보정 없는 원점수
    - `pub fn prior_offsets(&self) -> Vec<f32>` — `ln(population / sample)`, sample 0이면 해당 카테고리는 `f32::NEG_INFINITY`
    - `pub fn predict_proba(&self, x: &SparseVec, prior_corrected: bool) -> Vec<(String, f64)>` — softmax 확률, 내림차순
    - `pub fn save_atomic(&self, path: &Path) -> std::io::Result<()>` — 같은 디렉토리 `.<이름>.tmp`에 쓰고 `fsync` 뒤 `rename`
    - `pub fn load(path: &Path) -> Result<SoftmaxModel, String>` `}`
  - `impl Classify for SoftmaxModel` — Task 6에서 바뀔 trait이지만 지금은 기존 시그니처를 구현한다: `is_available` = true, `classify_document(doc)` = `BookFields::from_es_source` → `predict_proba(.., true)`를 `Prediction { category: 1등, confidence: 1등 확률, ranked: 전체, reason: "" }`로 담는다. `classify_path`는 `Err("ES 문서로만 분류한다")`.

파일 형식: 매직 `b"TMCLS\0\x01\0"`(8바이트) + 헤더 JSON 길이(u32 LE) + 헤더 JSON + bias(f32 LE × K) + weights(f32 LE × K × 2^hash_bits). `load`는 매직, 길이, 배열 크기 불일치를 모두 `Err`로 돌려준다.

- [ ] **Step 1: 실패하는 테스트 작성**

```rust
#[test] fn save_then_load_round_trips() // header·bias·weights 동일
#[test] fn load_rejects_bad_magic_and_truncated_file() // 둘 다 Err
#[test] fn save_atomic_leaves_no_tmp_and_replaces_existing() // 기존 파일 교체, .tmp 없음
#[test] fn probabilities_sum_to_one_and_are_sorted()
#[test] fn prior_correction_breaks_tie_toward_larger_population()
    // weights 0, population [10, 1000], sample [10, 10] → 보정 후 1등은 두 번째 카테고리, 확률 ≈ 100/101
#[test] fn zero_sample_category_never_ranks_first()
#[test] fn classify_document_reads_es_source() // {"title":..,"summary":..} → ranked 길이 == 카테고리 수
```

- [ ] **Step 2: 실패 확인** — `test_runner_rs -f tests/rust/classifier/softmax_model.rs` → 컴파일 실패
- [ ] **Step 3: 구현**
- [ ] **Step 4: 통과 확인** — PASS
- [ ] **Step 5: 커밋** — `feat: softmax 분류 모델과 파일 형식을 추가한다`

---

### Task 3: 핫 리로드 — `libs/classifier/src/model_reloader.rs`

**Files:**

- Create: `libs/classifier/src/model_reloader.rs`
- Modify: `libs/classifier/src/lib.rs`, `libs/classifier/Cargo.toml` (`[[test]] model_reloader`, `log` workspace 의존)
- Test: `tests/rust/classifier/model_reloader.rs`

**Interfaces:**

- Consumes: Task 2 `SoftmaxModel::load`
- Produces:
  - `pub struct ModelReloader` (`Send + Sync`)
  - `impl ModelReloader { pub fn new(path: PathBuf) -> Self; pub fn current(&self) -> Option<Arc<SoftmaxModel>> }`

동작: `current()`는 호출마다 파일의 (mtime, 크기)를 본다. 캐시와 같으면 캐시를 돌려준다. 다르면 `load`한다. 성공하면 캐시를 바꾸고, 실패하면 `log::warn!` 후 이전 캐시를 돌려준다(처음이면 `None`). 파일이 없으면 `None`이고 경고는 상태가 바뀔 때 한 번만 남긴다. 내부는 `Mutex`.

- [ ] **Step 1: 실패하는 테스트 작성**

```rust
#[test] fn missing_file_gives_none()
#[test] fn loads_once_and_reuses_while_unchanged() // 두 번 호출 → Arc::ptr_eq
#[test] fn reloads_after_file_is_replaced() // 다른 카테고리 목록 모델로 save_atomic → 새 header
#[test] fn keeps_previous_model_when_replacement_is_corrupt() // Review Focus 1
#[test] fn file_deleted_after_load_gives_none()
```

mtime 해상도 문제를 피하려고 교체 테스트는 크기가 다른 모델로 바꾸거나 `filetime` 없이 `File::set_modified`(std 1.75+)로 mtime을 명시한다.

- [ ] **Step 2: 실패 확인** — `test_runner_rs -f tests/rust/classifier/model_reloader.rs`
- [ ] **Step 3: 구현**
- [ ] **Step 4: 통과 확인** — PASS
- [ ] **Step 5: 커밋** — `feat: 분류 모델 핫 리로드를 추가한다`

---

### Task 4: ES 조회 — `libs/search_manager`

**Files:**

- Modify: `libs/search_manager/src/doc_searcher.rs` (ESManager 메서드 3개), `libs/search_manager/src/lib.rs` (`EsManagerApi`에 `search_by_isbn` 기본 구현 `Err(EsError::Unsupported)` + `ESManager` 구현)
- Modify: `tests/rust/shared/es_fake.rs` (`FakeEsManager::search_by_isbn`: `state.docs`에서 `isbn` 일치 문서)
- Test: `tests/rust/search_manager/search_manager.rs` (기존 파일에 추가, 기존 `new_es()`/`es.on("search", ..)` 방식)

**Interfaces:**

- Produces:
  - `ESManager::list_ids_and_paths(&self) -> Result<Vec<(i64, String)>, EsError>` — scroll, `_source: ["file_path"]`, `sort: ["_doc"]`, 5,000건씩. `_id`가 정수가 아니거나 `file_path`가 없는 문서는 건너뛴다. 끝나면 scroll을 지운다(실패해도 지운다).
  - `ESManager::mget_sources(&self, ids: &[i64], fields: &[&str]) -> Result<Vec<(i64, Value)>, EsError>` — 1,000개씩 나눠 `MultiGet`. `found: false`는 뺀다.
  - `EsManagerApi::search_by_isbn(&self, isbn: &str, limit: usize) -> Result<Vec<(i64, String)>, EsError>` — `term {isbn}`, `_source: ["file_path"]`, `size: limit`. `(inode, file_path)`.
  - `pub const ISBN_MATCH_LIMIT: usize = 50;` (`search_manager` 루트)

- [ ] **Step 1: 실패하는 테스트 작성**

```rust
#[test] fn list_ids_and_paths_pages_through_scroll_and_clears_it()
#[test] fn list_ids_and_paths_clears_scroll_on_error()
#[test] fn mget_sources_chunks_by_1000_and_skips_missing() // 2,001개 → mget 3번
#[test] fn search_by_isbn_sends_term_query_with_limit()
#[test] fn search_by_isbn_default_is_unsupported() // 기존 unsupported! 매크로 목록에 추가
```

- [ ] **Step 2: 실패 확인** — `test_runner_rs -f tests/rust/search_manager/search_manager.rs`
- [ ] **Step 3: 구현**
- [ ] **Step 4: 통과 확인** — PASS
- [ ] **Step 5: 커밋** — `feat: 분류기용 ES 열거·일괄 조회·ISBN 조회를 추가한다`

---

### Task 5: 학습 CLI — `utils/classifier_trainer`

**Files:**

- Create: `utils/classifier_trainer/Cargo.toml` (deps: `classifier`, `search_manager`, `rand`, `rand_chacha`, `serde_json`. 생성 시각은 `std::time::SystemTime`의 Unix 초로 기록해 chrono를 쓰지 않는다)
- Create: `utils/classifier_trainer/src/main.rs`, `src/sample_drawer.rs`, `src/model_trainer.rs`, `src/holdout_evaluator.rs`, `src/lib.rs`(모듈 공개, 테스트용)
- Modify: 루트 `Cargo.toml` members에 `"utils/classifier_trainer"`, `docs/rust-workspace.md` utils 표에 한 줄
- Test: `tests/rust/classifier_trainer/sample_drawer.rs`, `model_trainer.rs`, `holdout_evaluator.rs`, `integration/trainer_cli.rs`

**Interfaces:**

- Consumes: Task 1 `BookFields`, `hash_features`; Task 2 `SoftmaxModel`, `ModelHeader`, `save_atomic`; Task 4 `list_ids_and_paths`, `mget_sources`
- Produces:
  - `sample_drawer`:
    - `pub fn category_of(file_path: &str) -> Option<&str>` — 첫 조각. 조각이 1개뿐이거나 `0_` 접두, `9_북스캔OCR`이면 `None`
    - `pub struct Population { pub by_category: BTreeMap<String, Vec<i64>> }` + `pub fn group(rows: &[(i64, String)]) -> Population` — 같은 inode는 한 번만
    - `pub struct Sample { pub train: Vec<(i64, String)>, pub holdout: Vec<(i64, String)> }`
    - `pub fn draw(pop: &Population, per_category: usize, holdout_ratio: f64, seed: u64) -> Sample` — 카테고리 이름 순으로 돌며, 카테고리마다 `ChaCha8Rng::seed_from_u64(seed ^ fnv1a64(카테고리))`로 inode 정렬 목록을 Fisher–Yates(`SliceRandom::shuffle`) 후 앞 N개. 그 N개 중 `round(N × ratio)`개(최소 1, 표본 5 미만이면 0)를 홀드아웃으로.
  - `model_trainer`:
    - `pub struct TrainConfig { pub hash_bits: u32, pub epochs: usize, pub learning_rate: f32, pub l2: f32, pub batch_size: usize, pub seed: u64 }` + `Default`(17, 8, 0.5, 1e-6, 64, 0)
    - `pub fn train(rows: &[(SparseVec, usize)], categories: &[String], cfg: &TrainConfig) -> (Vec<f32>, Vec<f32>)` — 미니배치 SGD, 다중 클래스 cross-entropy, L2, epoch마다 seed 셔플, 학습률 `lr / (1 + epoch)`. 반환 (weights, bias).
  - `holdout_evaluator`:
    - `pub struct Report { pub top1: f64, pub top2: f64, pub top1_prior: f64, pub top1_prior_weighted: f64, pub per_category: Vec<(String, usize, f64)>, pub coverage_by_threshold: Vec<(f64, f64, f64)> }`
    - `pub fn evaluate(model: &SoftmaxModel, holdout: &[(SparseVec, usize)]) -> Report` — `top1_prior_weighted`는 각 문서 가중치 `population/sample`. 임계 표는 t = 0.1, 0.2, …, 0.9마다 (t, 판정률, 판정한 것 중 정답률).
    - `pub fn format_report(r: &Report) -> String`
  - CLI(`main.rs`): `classifier_trainer --output <파일> [--per-category N(1000)] [--seed u64] [--holdout 0.1] [--hash-bits 17] [--epochs 8]`. 인자 없으면 `print_usage()` + 종료 코드 1. `--output` 없으면 사용법 + 1. seed가 없으면 `rand::random()`. 단계마다 stderr에 진행(모집단 건수, 표본 건수, `summary` 빈 문서 수, epoch별 손실). stdout에 seed와 `format_report`. 마지막에 `save_atomic`.

- [ ] **Step 1: `sample_drawer` 실패 테스트**

```rust
#[test] fn category_of_excludes_zero_prefix_ocr_and_root_files()
#[test] fn group_dedupes_inode()
#[test] fn same_seed_same_sample_and_different_seed_differs()
#[test] fn small_category_uses_all_and_large_is_capped()
#[test] fn holdout_rules() // 표본 4 → 0, 표본 10·ratio 0.1 → 1, 표본 1000 → 100
#[test] fn draw_is_uniform() // 모집단 10, N=3, seed 0..20000 → 각 원소 선택 빈도 0.3 ± 0.02
#[test] fn sample_does_not_depend_on_other_categories() // 다른 카테고리 추가해도 A의 표본 불변
```

- [ ] **Step 2: 실패 확인 → 구현 → 통과** — `test_runner_rs -f tests/rust/classifier_trainer/sample_drawer.rs`
- [ ] **Step 3: `model_trainer` 실패 테스트**

```rust
#[test] fn learns_linearly_separable_toy_data() // 카테고리 3개, 각 고유 단어 → 학습 데이터 top-1 100%
#[test] fn same_seed_is_deterministic() // weights 동일
#[test] fn l2_shrinks_weights() // l2 큰 쪽의 weight 제곱합이 더 작다
```

- [ ] **Step 4: 실패 확인 → 구현 → 통과** — `test_runner_rs -f tests/rust/classifier_trainer/model_trainer.rs`
- [ ] **Step 5: `holdout_evaluator` 실패 테스트**

```rust
#[test] fn top1_and_top2_counts() // 손으로 만든 모델·홀드아웃 3건 → 1/3, 2/3
#[test] fn weighted_accuracy_uses_population_over_sample()
#[test] fn threshold_table_has_nine_rows_and_monotone_coverage()
```

- [ ] **Step 6: 실패 확인 → 구현 → 통과** — `test_runner_rs -f tests/rust/classifier_trainer/holdout_evaluator.rs`
- [ ] **Step 7: CLI 통합 테스트** — `integration/trainer_cli.rs`: 인자 없음 → 코드 1·stdout에 `Usage`; `--output`만 빠짐 → 코드 1. ES가 필요한 실제 학습 경로는 여기서 돌리지 않는다(Task 9 실측).
- [ ] **Step 8: 실패 확인 → 구현 → 통과** — `test_runner_rs -f tests/rust/classifier_trainer/integration/trainer_cli.rs`
- [ ] **Step 9: 커밋** — `feat: 균등 표본 분류 학습 CLI를 추가한다`

---

### Task 6: 합산 판정과 등급 제거 — `libs/book_manager`, `libs/classifier`

**Files:**

- Create: `libs/book_manager/src/evidence_scorer.rs` (근거별 `p(c)`와 합산. `category_proposer.rs`가 이미 크므로 순수 계산을 떼어 둔다)
- Modify: `libs/book_manager/src/category_proposer.rs` (`propose_category_for_file`, `propose_category_changes`, 등급·우선순위 분기 삭제), `libs/book_manager/src/lib.rs` (`ClassifierFactory` 타입, `HighConfidenceOverride`·`with_is_high_confidence_override`·`MODEL_CERTAIN_MIN_CONFIDENCE` 재수출 삭제), `libs/classifier/src/lib.rs` (trait 정리)
- Test: `tests/rust/book_manager/evidence_scorer.rs` (새), `tests/rust/book_manager/category_proposer.rs`, `tests/rust/book_manager/seam_accessors.rs`, `tests/rust/book_manager/integration/container/book_manager.rs`, `tests/rust/classifier/classifier.rs`, `tests/rust/backend/integration/api_router_mock.rs` — `grade`·`is_high_confidence`·`expected_accuracy`·`ModelOnlyClassifier` 참조를 새 필드로 바꾼다. `tests/rust/backend/integration/container/classify_proposal_items_db.rs`는 payload를 그대로 저장하는 DB 테스트라 `grade` 문자열을 그대로 둔다.

**Interfaces:**

- Consumes: Task 2 `SoftmaxModel`(`Classify` 구현), Task 3 `ModelReloader`, Task 4 `search_by_isbn`, `ISBN_MATCH_LIMIT`, 기존 `EsManagerApi::search_by_id`, 기존 `match_category_by_keywords` → `KeywordMatch.ranked: Vec<(String, Vec<String>)>`
- Produces:
  - `libs/classifier`: `pub trait Classify: Send + Sync { fn classify_document(&self, doc: &Value) -> Result<Prediction, String>; }`, `pub struct Prediction { pub ranked: Vec<(String, f64)> }`. `is_available`, `classify_path`, `min_confidence`, `expected_accuracy`, `BookClassifierApi`, `ModelOnlyClassifier`는 지운다. Task 2의 `impl Classify for SoftmaxModel`도 이에 맞춘다.
  - `libs/book_manager`: `pub type ClassifierFactory = Arc<dyn Fn() -> Option<Arc<dyn Classify>> + Send + Sync>;` (`None` = 모델 없음)
  - `evidence_scorer`:
    - `pub enum Evidence { Isbn, Keyword, Model }` + `pub const EVIDENCE_WEIGHTS: [(Evidence, f64); 3] = [(Isbn, 0.6), (Keyword, 0.25), (Model, 0.15)];`
    - `pub fn isbn_scores(matched_paths: &[String]) -> Option<Vec<(String, f64)>>` — 카테고리는 각 경로의 부모 디렉토리. 빈 목록이거나 길이 ≥ `ISBN_MATCH_LIMIT`면 `None`(Review Focus 4)
    - `pub fn keyword_scores(ranked: &[(String, Vec<String>)]) -> Option<Vec<(String, f64)>>` — 키워드 수 비율, 합 0이면 `None`
    - `pub fn combine(parts: &[(Evidence, Vec<(String, f64)>)]) -> Vec<ScoredCategory>` — `Σ w·p / Σ w`(있는 근거만), 점수 내림차순, 동점은 카테고리 이름 오름차순
    - `pub struct ScoredCategory { pub category: String, pub score: f64, pub contributions: Vec<(Evidence, f64)> }`
    - `pub fn detail_of(c: &ScoredCategory) -> String` — 예: `"ISBN 0.50 · 키워드 0.00 · 모델 0.41"`(근거별 원래 `p`)
  - `CategoryCandidate`에 `pub score: f64` 추가. `source`는 기여(`w·p`)가 가장 큰 근거 이름(`"isbn"`/`"keyword"`/`"model"`).
  - `Proposal`: `grade`, `confidence`, `expected_accuracy`, `source` 삭제. `pub score: Option<f64>`(1등 점수), `pub classified: bool` 추가. `target_category` = 1등 카테고리, `model_category` = 모델 단독 1등, `matched_keywords`·`reason`·`candidates` 유지. serde는 `#[serde(default)]` 그대로라 예전 payload를 읽을 수 있다.

`propose_category_for_file` 흐름: inode → `es.search_by_id` 한 번 → (ISBN이 있으면) `search_by_isbn(isbn, ISBN_MATCH_LIMIT)`에서 자기 inode를 뺀 경로 → `isbn_scores`; 키워드 → `keyword_scores`; 모델이 있고 문서가 있고 `!BookFields::from_es_source(doc).is_empty()`이면(Review Focus 3) `classify_document` → 디스크에 디렉토리가 없는 카테고리를 뺀 뒤(Review Focus 2) 합 1로 다시 정규화. `combine` → 상위 2개를 후보로. 근거가 하나도 없으면 후보 없음, `classified: true`. ES 조회 실패는 ISBN·모델 근거 "없음"으로 처리하고 `reason`에 남긴다. 팩토리는 `propose_category_changes` 시작 때 한 번 호출한다.

- [ ] **Step 1: `evidence_scorer` 실패 테스트**

```rust
#[test] fn isbn_scores_are_category_shares() // ["A/x","A/y","B/z"] → A 2/3, B 1/3
#[test] fn isbn_scores_none_when_empty_or_at_limit() // 50건 → None
#[test] fn keyword_scores_are_keyword_shares()
#[test] fn combine_normalizes_by_present_weights() // 모델만 → 모델 확률 그대로
#[test] fn combine_isbn_outweighs_keyword_and_model_together() // ISBN A=1.0, 키워드 B=1.0, 모델 B=1.0 → A 1등(0.6 > 0.4)
#[test] fn combine_tie_breaks_by_name()
#[test] fn combine_with_no_evidence_is_empty()
#[test] fn detail_lists_each_evidence()
```

- [ ] **Step 2: 실패 확인 → 구현 → 통과** — `test_runner_rs -f tests/rust/book_manager/evidence_scorer.rs`
- [ ] **Step 3: `category_proposer` 실패 테스트 추가** (`FakeEsManager::put_doc`로 문서, 가짜 `Classify`)

```rust
#[test] fn proposal_top_two_follow_combined_score()
#[test] fn isbn_match_in_current_category_keeps_book_in_place() // 1등 == current_category
#[test] fn own_inode_is_excluded_from_isbn_matches()
#[test] fn unindexed_file_uses_keyword_only()
#[test] fn model_candidates_without_directory_are_dropped() // Review Focus 2
#[test] fn empty_summary_and_title_skip_model() // Review Focus 3
#[test] fn no_model_factory_returns_none_and_proposal_still_runs()
#[test] fn placeholder_rows_are_not_classified_and_finished_rows_are() // classified false → true
#[test] fn old_payload_with_grade_still_deserializes()
```

- [ ] **Step 4: 실패 확인** — `test_runner_rs -f tests/rust/book_manager/category_proposer.rs`
- [ ] **Step 5: 구현 — 합산 흐름 연결, 등급·`is_high_confidence`·`fill_second_with_model`·`build_candidates`·`HighConfidenceOverride`·`MODEL_CERTAIN_MIN_CONFIDENCE` 삭제, `libs/classifier` trait 정리.** 삭제 전에 `codegraph callers`로 각 심볼의 호출자를 확인하고, 0건이면 `grep -rn`으로 다시 확인한다.
- [ ] **Step 6: 기존 테스트 갱신** — 위 Files의 테스트에서 `grade`/`is_high_confidence`/`ModelOnlyClassifier` 참조를 새 필드로 바꾸거나, 사라진 동작을 검증하던 테스트는 지운다.
- [ ] **Step 7: 통과 확인** — `test_runner_rs -f` 로 바꾼 파일 각각, 그다음 `test_runner_rs`(증분), `cargo clippy --workspace --all-targets`
- [ ] **Step 8: 커밋** — `feat: 분류 제안을 근거별 가중치 합산 점수로 판정한다`

---

### Task 7: backend 조립과 배포 설정

**Files:**

- Modify: `backend/src/api_router/app_deps.rs` (`build_book_manager`에서 `TM_CLASSIFIER_MODEL`을 읽어 `ModelReloader`를 만들고 `deps.with_classifier_factory`로 꽂는다. 만화 매니저에는 꽂지 않는다)
- Modify: `k8s/tm-deployment.yml` (tm-backend 컨테이너: env `TM_CLASSIFIER_MODEL=/app/models/classifier/model.bin`, volumeMount `classifier-model` → `/app/models/classifier` `readOnly: true`; volumes: `hostPath: {path: /home/terzeron/workspace/tm/models/classifier, type: DirectoryOrCreate}`)
- Modify: `.gitignore` (`/models/`)
- Test: `backend/src/api_router/app_deps.rs`의 기존 `#[cfg(test)] mod tests`에 추가(private 함수 검증이므로)

**Interfaces:**

- Consumes: Task 3 `ModelReloader::new`, `current`; Task 6 `ClassifierFactory`
- Produces: `fn classifier_factory_from_env(env: EnvFn<'_>) -> Option<ClassifierFactory>` — env가 없으면 `None`(경고 한 번)

- [ ] **Step 1: 실패 테스트** — `classifier_factory_from_env`: env 없음 → `None`; env 있음·파일 없음 → `Some(factory)`이고 `factory()`가 `None`; 유효 모델 파일 → `factory()`가 `Some`
- [ ] **Step 2: 실패 확인 → 구현 → 통과** — `test_runner_rs -f backend/src/api_router/app_deps.rs`가 지원되지 않으면 `test_runner_rs`(증분)로 확인하고 결과에 backend lib 테스트가 포함됐는지 본다
- [ ] **Step 3: k8s 검증** — `kubectl apply --dry-run=client -f k8s/tm-deployment.yml` → 오류 없음
- [ ] **Step 4: 커밋** — `feat: backend에 분류 모델 핫 리로드를 연결한다`

---

### Task 8: frontend — 등급 대신 점수

**Files:**

- Modify: `frontend/src/CategoryAdmin.jsx` (`applyProposalStatus`의 `certain` 미리 체크 블록 삭제 — `setProposalChoices({})`만 남긴다)
- Modify: `frontend/src/ClassifyProposalTable.jsx` (`scoreValue`, 점수 표시·툴팁, `CandidateCell`의 "분류 중…" 조건, `isTie`)
- Test: `frontend/tests/ClassifyProposalTable.test.jsx`, `frontend/tests/CategoryAdmin.test.jsx`

**Interfaces:**

- Consumes: Task 6 항목 필드 `score`, `classified`, `candidates[].score`
- Produces:
  - `scoreValue(item) = item.score ?? item.expected_accuracy ?? item.confidence ?? null` (예전 payload 호환)
  - `isClassified(item) = item.classified ?? (item.grade != null || (item.candidates || []).length > 0)` — "분류 중…"은 `index === 0 && !isClassified(item)`일 때만
  - `isTie` = 후보 2개이고 `candidates[0].score != null && candidates[0].score === candidates[1].score`
  - 툴팁 문구: `합산 점수 ${score.toFixed(2)}`, 점수 없으면 `점수 없음`

- [ ] **Step 1: 실패 테스트 작성** (기존 `grade` 기반 테스트는 새 기준으로 바꾼다)

```js
it("does not pre-check any candidate after proposal is ready");
it("shows 분류 중… only for unclassified placeholder rows");
it("treats legacy payload with grade but no classified as classified"); // Review Focus 5
it("shows tie badge when top two scores are equal");
it("sorts and displays by combined score");
```

- [ ] **Step 2: 실패 확인** — `cd frontend && npx vitest run tests/ClassifyProposalTable.test.jsx tests/CategoryAdmin.test.jsx`
- [ ] **Step 3: 구현**
- [ ] **Step 4: 통과 확인** — 같은 명령 PASS, 그다음 `cd frontend && npm test` 전체 PASS
- [ ] **Step 5: 커밋** — `feat: 분류 제안 표를 등급 대신 합산 점수로 표시한다`

---

### Task 9: 실측 학습과 보고

ES 실제 접속이 필요하다. 사용자 셸의 ES env를 그대로 쓴다(값을 출력하지 않는다). `/mnt/data`는 읽지 않는다(ES만 읽는다).

- [ ] **Step 1: 빌드** — `cargo build --release -p classifier_trainer`
- [ ] **Step 2: 인자 없이 실행** — `target/release/classifier_trainer` → 사용법, 종료 코드 1
- [ ] **Step 3: N=100 학습** — `target/release/classifier_trainer --output /home/terzeron/workspace/tm_dev/tmp/classifier/model-100.bin --per-category 100 --seed 20261107` → stdout 보고서 저장, `/usr/bin/time -v`로 최대 메모리 기록
- [ ] **Step 4: N=1000 학습** — 같은 seed, `--per-category 1000`, 출력 `model-1000.bin`. 메모리가 크면 `systemd-run --user --scope -p MemoryMax=12G`로 감싼다.
- [ ] **Step 5: 비교 표 작성** — top-1, top-2, 보정 후 top-1, 실제 분포 가중 top-1, 임계 0.5에서 판정률·정답률, 학습 시간, 최대 메모리, 모델 크기(64MB 이하인지)
- [ ] **Step 6: 배포 위치에 복사** — 사용자 확인 후 더 나은 모델을 `/home/terzeron/workspace/tm/models/classifier/model.bin`으로 복사(`cp` 뒤 `mv`로 원자 교체)
- [ ] **Step 7: 최종 검증** — `test_runner_rs`(증분), `cargo clippy --workspace --all-targets`, `cd frontend && npm test`, `kubectl apply --dry-run=client -f k8s/tm-deployment.yml`. 결과를 그대로 보고한다.
