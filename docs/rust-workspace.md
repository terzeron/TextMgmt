# Rust workspace

저장소 루트의 `Cargo.toml`이 workspace다. Rust 실행 경로를 유지하고, Python 원본은 동작 대조에 사용한다.

```
backend/    Rust 백엔드 API (Python 원본은 동작 대조용)
frontend/   프론트엔드 (React)
utils/      실행 도구. Python 스크립트와 Rust 도구 crate가 함께 있다
libs/       공통 모듈 crate (도구와 backend가 함께 쓴다)
tests/      Python 테스트와 Rust 테스트(tests/rust/<crate>/)
docs/       문서
```

## libs (공통 모듈)

| crate | 용도 | 대응 Python |
| --- | --- | --- |
| `corpus_common` | 본문 표본 읽기, 인코딩 판별, ZIP 열기(`open_zip`), 경로·URL 도구, 빈 디렉토리 청소(`clean_empty_parent_dirs`), 병렬 풀 | `backend/classifier/reader.py` 일부 |
| `corpus_layout` | 색인 제외 디렉토리 상수 | `utils/corpus_layout.py` |
| `es_client` | Elasticsearch REST 클라이언트: HTTP와 인증서 검증 HTTPS, 검색·갱신·조회·삭제 | `backend/es_manager.py` 일부 |
| `htmlkit` | HTML 토큰화와 BeautifulSoup 호환 텍스트 추출 | (bs4) |
| `bookstore` | Yes24·알라딘·교보문고 검색과 상세 추출(HTTPS는 `rustls`, 파싱은 `scraper`). Python `html.parser`와 트리가 달라 알라딘 ISBN은 Rust가 더 잘 찾는다 | `backend/bookstore.py` 일부 |
| `pyfmt` | Python 호환 서식: `py_repr`, `py_float`, `py_round` | |
| `stat` | 색인·파싱 통계 카운터 | `utils/stat.py` |
| `file_time` | 파일 생성 시각 | `utils/file_time.py` |
| `parser_timeout` | 파서 시간 상한: `Deadline`, 스레드 상한, 자식 프로세스 강제 종료 | `utils/parser_timeout.py` |
| `isbn` | 파일에서 ISBN 추출(TXT·EPUB·PDF·DJVU·HWP) | `utils/isbn.py` |
| `hwp3_tables`, `hwp3_parser` | HWP 3.x 텍스트 추출 | `utils/hwp3_*.py` |

## utils (도구)

| crate | 용도 | Python 원본 |
| --- | --- | --- |
| `pdfscan` | PDF의 Launch·JavaScript·OpenAction·첨부 등 실행 가능 요소 정적 검사 | (신규) |
| `epubscan` | `structure`: 뷰어가 못 그리는 EPUB 구조, `scan`: 스크립트 실행 코드·위험 ZIP 구조 | `detect_mojibake epub-structure` 통합 |
| `detect_mojibake` | 인코딩 손상 문서 탐지 | 삭제함 |
| `quarantine_broken` | 손상 문서를 `0_broken`으로 격리·복구 | 삭제함 |
| `cluster_hybrid_genres` | 5대 장르 비율 클러스터링 | 삭제함 |
| `search` | ES 제목·내용 검색 점검 | 삭제함 |
| `update` | ES 갱신·조회·삭제 점검(인자 없이 실행 가능, `run` 선택) | `utils/update.py` 동작 대응 |
| `move_semi_clusters` | 인자 없이 준확실 클러스터를 이동(`--dry-run`으로 미리보기; `run` 선택) | `utils/move_semi_clusters.py` 동작 대응 |
| `classify_books_client` | 인자 없이 기본 라이브러리 분류를 실행한다. REST API 감시와 직접 분류 옵션도 지원한다 | `utils/classify_books_client.py` 대응 |
| `classify_cli` | 수집·Rust 학습과 export·평가·보정·서점 정책·분류·재분류·모델 정보 | `utils/classify_cli.py` 동작 대조용 |
| `normalize_comic_directories` | 인자 없이 기본 만화 경로를 스캔해 리포트를 쓴다. `--apply`로 이름을 변경한다 | `utils/normalize_comic_directories.py` 동작 대응 |

## 규칙

- `-h`/`--help`는 도움말을 출력한다. 인자 없는 실행은 Python 원본의 동작과 종료 코드를 따른다.
  `update`, `move_semi_clusters`, `classify_books_client`, `normalize_comic_directories`는 기본 작업을 실행하고,
  `loader`와 `classify_cli`는 사용법을 출력하고 코드 1로 끝난다. 다른 CLI도 Python 원본의 계약을 따른다.
- 잘못된 인자의 출력 위치와 종료 코드는 Python 원본과 일치시킨다.
- **테스트를 먼저 쓴다(TDD).** Python 원본이 있으면 그 pytest를 Rust 테스트로 먼저 옮기고 실패를 확인한 뒤 구현한다.
- **커버리지는 100%에 가깝게 유지한다.** `cargo llvm-cov --workspace --summary-only`.
- **라이브러리는 도구에 의존하지 않는다.** 공유할 코드는 `libs/`로 올린다.
- Python 원본은 사용자가 지우라고 한 것만 지운다.
- `bookstore`는 공개 서점 HTTPS를 사용한다. ES client도 HTTP와 인증서 검증 HTTPS를 지원하며, 사설 CA는 `TM_ES_CA_CERT` PEM 경로로 지정한다. 인증서 검증을 끄는 우회로는 두지 않는다.
- ZIP은 `corpus_common::reader::open_zip`으로 연다. `zip` 크레이트 기본 설정(`Detect`)은 파일 전체를 훑어 느리다
  (1.5MB EPUB 하나에 읽기 1만 3천 번, 48ms). `open_zip`은 `Known(0)`으로 먼저 열고(0.2ms) 실패하면 `Detect`로 다시 연다.

## 분류 CLI

`cargo run -p classify_cli -- <명령>`으로 실행한다. 인자 없이 실행하면 도움말을 출력하고 종료 코드 1을 반환한다.
`collect`, `train`, `evaluate`, `score-calibration`, `bookstore-policy`, `classify`, `reclassify`, `info`, `import-joblib`을 지원한다.

```sh
cargo run -p classify_cli -- collect --out tmp/classifier/corpus.jsonl
cargo run -p classify_cli -- train --corpus tmp/classifier/corpus.jsonl --out tmp/classifier/export
cargo run -p classify_cli -- evaluate --corpus tmp/classifier/corpus.jsonl --model tmp/classifier/export
cargo run -p classify_cli -- classify book.txt --model tmp/classifier/export --no-es
cargo run -p classify_cli -- reclassify 0_inbox --library-root /books --model tmp/classifier/export --no-es
```

- `--model`과 `train --out`은 JSON·가중치 파일을 담는 **디렉토리**다. 기본값은 `TM_CLASSIFIER_MODEL_DIR`, 없으면 `backend/classifier/export`다.
- `train`은 Rust classifier를 학습하고 Rust 모델을 `DIR`에 저장한다. holdout 명단은 `DIR.holdout.json`이다.
  `--spill-dir DIR`을 주면 sparse feature block을 디스크에 쓰고 학습 행렬을 행 단위로 읽는다.
- 기존 sklearn `model.joblib`을 한 번만 이전할 때 Rust importer로 Rust model directory를 만든다.
  런타임은 변환된 디렉터리만 읽으며, Kubernetes는 `/books/.classifier/rust-export`를 가리킨다.

```sh
cargo run -p classify_cli -- import-joblib \
  --model /mnt/data/text/.classifier/model.joblib \
  --out /mnt/data/text/.classifier/rust-export --golden 256 --seed 1
TM_CLASSIFIER_MODEL_DIR=/mnt/data/text/.classifier/rust-export \
  cargo run -p classify_cli -- info
```

- `evaluate`와 `score-calibration`은 저장된 holdout 명단을 우선한다. 명단이 없으면 경고 후 기존 해시 분할을 재구성한다.
  보정은 `--dry-run`을 지원하며, 저장할 때 가중치는 유지하고 `model.json`만 원자적으로 교체한다.
- ES는 기존 `TM_ES_URL`, `TM_ES_BOOK_INDEX`, `TM_ES_USER`, `TM_ES_PASSWORD`를 사용한다. `classify`와 `reclassify`는 `--no-es`로 파일만 읽는다.
- `bookstore-policy`는 기존 서점 HTTPS 클라이언트와 분류 서비스의 캐시를 재사용한다. 표본 순서는 Python의 정수 seed shuffle과 같다.
  `--reband PATH`는 서점 접속 없이 저장된 행을 다시 집계한다. 실서점 응답의 정확성은 네트워크와 HTML 상태에 달려 있다.
- `reclassify`는 기본 미리보기다. `--apply`를 붙여야 이동하며, 목적지 충돌과 카테고리 심볼릭 링크를 거부한다.
  같은 filesystem에서는 inode를 보존하고, 다른 filesystem에서는 복사 후 원본을 지운다. 파일 이동 실패를 출력·집계하고
  Python CLI와 같이 종료 코드 0을 반환한다.
- 테스트는 `tests/rust/classify_cli/cli.rs`에 등록했다. 기존 `tests/rust/backend/classify_cli*.rs`는 구현 전 가정 API로 작성한 미등록 테스트다.
  새 테스트는 실제 export 모델과 현재 `EsTransport`·`CommandRunner` 계약으로 실행한다.

검증: `cargo test -p classify_cli --locked`, `cargo clippy -p classify_cli --all-targets --locked -- -D warnings`.

## 새 crate 추가

```
cargo new --lib libs/<name> --vcs none      # 공통 모듈
cargo new --bin utils/<name> --vcs none     # 도구
```

루트 `Cargo.toml`의 `members`에 경로를 추가하고, 각 `Cargo.toml`에 `version.workspace = true`, `edition.workspace = true`,
`publish.workspace = true`, `autotests = false`를 둔다. 테스트는 `tests/rust/<name>/`에 두고 `[[test]] name/path`로 등록한다.

## 빌드·검증

```
cargo build --release                       # 산출물: target/release/<name>
cargo test --workspace
cargo clippy --workspace --all-targets
cargo llvm-cov --workspace --summary-only   # 커버리지
```

`isbn`의 PDF 추출은 `pypdfium2`가 싣고 있는 `libpdfium.so`를 쓴다. `.cargo/config.toml`이 개발 환경의 경로를 잡아 준다.
배포 환경에서는 `PDFIUM_LIB_PATH`로 지정한다. 라이브러리가 없으면 순수 Rust(`lopdf`)로 내려간다.
