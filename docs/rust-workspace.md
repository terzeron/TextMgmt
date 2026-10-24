# Rust workspace

저장소 루트의 `Cargo.toml`이 workspace다. Python에서 Rust로 점차 옮기는 중이라 디렉토리를 미리 나눠 두었다.

```
backend/    백엔드 API (python -> rust 예정)
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
| `es_client` | Elasticsearch REST 클라이언트(http 전용): 검색, 갱신, 조회, 삭제 | `backend/es_manager.py` 일부 |
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
| `update` | ES 갱신·조회·삭제 점검(`update run` 필요) | `utils/update.py` 유지 |
| `move_semi_clusters` | 준확실 클러스터 도서를 장르 폴더로 이동(`run`, `--dry-run`) | `utils/move_semi_clusters.py` 유지 |
| `classify_books_client` | REST API로 자동 분류 요청·진행 감시(`--api-url`). 로컬 엔진 직접 모드는 미이식 | `utils/classify_books_client.py` 유지 |
| `classify_cli` | 수집·평가·점수 보정·서점 정책·분류·재분류·모델 정보. 학습과 export는 Python 실행 | `utils/classify_cli.py` 유지 |
| `reload_samgukji_epubs` | `2_소설역사`의 삼국지 EPUB 나열(`--list`)·loader 재적재(`--apply`) | `utils/reload_samgukji_epubs.sh` 유지 |
| `normalize_comic_directories` | 만화 디렉토리 이름 정규화. 서점은 HTTPS로 조회하고 캐시에 남긴다(`--offline`이면 캐시만, 없으면 `SKIP_NOT_CACHED`) | `utils/normalize_comic_directories.py` 유지 |

## 규칙

- **인자 없이 실행하면 용도와 사용법을 출력한다.** 모든 바이너리는 `print_usage()`를 갖고, 인자가 없거나 `-h`/`--help`면
  stdout에 출력하고 종료 코드 0으로 끝낸다(용도, 사용법, 인자, 옵션, 종료 코드). 잘못된 인자는 stderr에 쓰고 2로 끝낸다.
- **테스트를 먼저 쓴다(TDD).** Python 원본이 있으면 그 pytest를 Rust 테스트로 먼저 옮기고 실패를 확인한 뒤 구현한다.
- **커버리지는 100%에 가깝게 유지한다.** `cargo llvm-cov --workspace --summary-only`.
- **라이브러리는 도구에 의존하지 않는다.** 공유할 코드는 `libs/`로 올린다.
- Python 원본은 사용자가 지우라고 한 것만 지운다.
- TLS는 `bookstore`만 쓴다(공개 서점 HTTPS). ES는 내부망 http 전용이라 `es_client`가 https 주소를 직접 거부한다. 인증서 검증을 끄는 우회로는 두지 않는다.
- ZIP은 `corpus_common::reader::open_zip`으로 연다. `zip` 크레이트 기본 설정(`Detect`)은 파일 전체를 훑어 느리다
  (1.5MB EPUB 하나에 읽기 1만 3천 번, 48ms). `open_zip`은 `Known(0)`으로 먼저 열고(0.2ms) 실패하면 `Detect`로 다시 연다.

## 분류 CLI

`cargo run -p classify_cli -- <명령>`으로 실행한다. 인자 없이 실행하면 도움말과 종료 코드 0을 반환한다.
`collect`, `train`, `evaluate`, `score-calibration`, `bookstore-policy`, `classify`, `reclassify`, `info`를 지원한다.

```sh
cargo run -p classify_cli -- collect --out tmp/classifier/corpus.jsonl
cargo run -p classify_cli -- train --corpus tmp/classifier/corpus.jsonl --out tmp/classifier/export
cargo run -p classify_cli -- evaluate --corpus tmp/classifier/corpus.jsonl --model tmp/classifier/export
cargo run -p classify_cli -- classify book.txt --model tmp/classifier/export --no-es
cargo run -p classify_cli -- reclassify 0_inbox --library-root /books --model tmp/classifier/export --no-es
```

- `--model`과 `train --out`은 JSON·가중치 파일을 담는 **디렉토리**다. 기본값은 `TM_CLASSIFIER_MODEL_DIR`, 없으면 `backend/classifier/export`다.
- `train`은 기존 Python CLI로 sklearn 학습을 실행하고 `backend.classifier.export`로 내보낸다. 셸을 거치지 않는다.
  Python 실행 파일은 `TM_CLASSIFIER_PYTHON`, 없으면 빌드한 저장소의 `.venv/bin/python`이다. Python 소스와 학습 환경이 필요하다.
  `--out DIR`이면 Python 모델은 `DIR.joblib`, Rust 모델은 `DIR`, holdout 명단은 `DIR.holdout.json`이다.
  Python 실패 코드를 전달하며 학습 실패 후에는 export하지 않는다. 출력은 각 Python 프로세스가 끝난 뒤 전달한다.
- `evaluate`와 `score-calibration`은 저장된 holdout 명단을 우선한다. 명단이 없으면 경고 후 기존 해시 분할을 재구성한다.
  보정은 `--dry-run`을 지원하며, 저장할 때 가중치는 유지하고 `model.json`만 원자적으로 교체한다.
- ES는 기존 `TM_ES_URL`, `TM_ES_BOOK_INDEX`, `TM_ES_USER`, `TM_ES_PASSWORD`를 사용한다. `classify`와 `reclassify`는 `--no-es`로 파일만 읽는다.
- `bookstore-policy`는 기존 서점 HTTPS 클라이언트와 분류 서비스의 캐시를 재사용한다. 표본 순서는 Python의 정수 seed shuffle과 같다.
  `--reband PATH`는 서점 접속 없이 저장된 행을 다시 집계한다. 실서점 응답의 정확성은 네트워크와 HTML 상태에 달려 있다.
- `reclassify`는 기본 미리보기다. `--apply`를 붙여야 이동하며, 목적지 충돌과 카테고리 심볼릭 링크를 거부한다.
  같은 filesystem에서는 inode를 보존하고, 다른 filesystem에서는 복사 후 원본을 지운다. 이동 실패가 있으면 종료 코드 1이다.
  이는 이동 실패에도 0을 반환하는 Python CLI보다 엄격하다.
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
