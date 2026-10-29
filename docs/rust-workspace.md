# Rust workspace

저장소 루트의 `Cargo.toml`이 Rust workspace다.

```
backend/    Rust 백엔드 API
frontend/   프론트엔드 (React)
utils/      Rust 실행 도구
libs/       공통 모듈 crate (도구와 backend가 함께 쓴다)
tests/      Rust 테스트(tests/rust/<crate>/)
docs/       문서
```

## libs (공통 모듈)

| crate | 용도 |
| --- | --- |
| `corpus_common` | 본문 표본 읽기, 인코딩 판별, ZIP 열기, 경로·URL 도구, 빈 디렉토리 청소, 병렬 풀 |
| `corpus_layout` | 색인 제외 디렉토리 상수 |
| `es_client` | Elasticsearch REST 클라이언트: HTTP와 인증서 검증 HTTPS, 검색·갱신·조회·삭제 |
| `htmlkit` | HTML 토큰화와 텍스트 추출 |
| `bookstore` | Yes24·알라딘·교보문고 검색과 상세 추출 |
| `pyfmt` | 일관된 문자열·숫자 출력 형식 |
| `stat` | 색인·파싱 통계 카운터 |
| `file_time` | 파일 생성 시각 |
| `parser_timeout` | 파서 시간 상한, 스레드 제한, 자식 프로세스 종료 |
| `isbn` | 파일에서 ISBN 추출(TXT·EPUB·PDF·DJVU·HWP) |
| `hwp3_tables`, `hwp3_parser` | HWP 3.x 텍스트 추출 |

## utils (도구)

| crate | 용도 |
| --- | --- |
| `bookdoctor` | epub·pdf·txt 보안 위험·파일 결함·렌더링 불가 구조 진단 |
| `quarantine_broken` | 손상 문서를 `0_broken`으로 격리·복구 |
| `cluster_hybrid_genres` | 5대 장르 비율 클러스터링 |
| `search` | ES 제목·내용 검색 점검 |
| `update` | ES 갱신·조회·삭제 점검 |
| `move_semi_clusters` | 준확실 클러스터 이동 및 미리보기 |
| `classify_books_client` | REST API 감시와 직접 분류 옵션 |
| `classify_cli` | 수집·학습·평가·보정·서점 정책·분류·재분류·모델 정보 |
| `normalize_comic_directories` | 만화 경로 스캔과 이름 변경 |

## 규칙

- `-h`/`--help`는 도움말을 출력한다. 각 CLI의 기본 실행과 종료 코드는 아래 계약을 따른다.
  `update`, `move_semi_clusters`, `classify_books_client`, `normalize_comic_directories`는 기본 작업을 실행하고,
  `loader`와 `classify_cli`는 사용법을 출력하고 코드 1로 끝난다.
- 잘못된 인자의 출력 위치와 종료 코드는 각 CLI 계약을 따른다.
- **테스트를 먼저 쓴다(TDD).** 정해진 동작 사례를 Rust 테스트로 먼저 옮기고 실패를 확인한 뒤 구현한다.
- **커버리지는 100%에 가깝게 유지한다.** `cargo llvm-cov --workspace --summary-only`.
- **라이브러리는 도구에 의존하지 않는다.** 공유할 코드는 `libs/`로 올린다.
- `bookstore`는 공개 서점 HTTPS를 사용한다. ES client도 HTTP와 인증서 검증 HTTPS를 지원하며, 사설 CA는 `TM_ES_CA_CERT` PEM 경로로 지정한다. 인증서 검증을 끄는 우회로는 두지 않는다.
- ZIP은 `corpus_common::reader::open_zip`으로 연다. `zip` 크레이트 기본 설정(`Detect`)은 파일 전체를 훑어 느리다
  (1.5MB EPUB 하나에 읽기 1만 3천 번, 48ms). `open_zip`은 `Known(0)`으로 먼저 열고(0.2ms) 실패하면 `Detect`로 다시 연다.

## 분류 CLI

`cargo run -p classify_cli -- <명령>`으로 실행한다. 인자 없이 실행하면 도움말을 출력하고 종료 코드 1을 반환한다.
`collect`, `train`, `evaluate`, `score-calibration`, `bookstore-policy`, `classify`, `reclassify`, `info`를 지원한다.

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
- `evaluate`와 `score-calibration`은 저장된 holdout 명단을 우선한다. 명단이 없으면 경고 후 기존 해시 분할을 재구성한다.
  보정은 `--dry-run`을 지원하며, 저장할 때 가중치는 유지하고 `model.json`만 원자적으로 교체한다.
- ES는 기존 `TM_ES_URL`, `TM_ES_BOOK_INDEX`, `TM_ES_USER`, `TM_ES_PASSWORD`를 사용한다. `classify`와 `reclassify`는 `--no-es`로 파일만 읽는다.
- `bookstore-policy`는 기존 서점 HTTPS 클라이언트와 분류 서비스의 캐시를 재사용한다. 표본 순서는 seed로 재현할 수 있다.
  `--reband PATH`는 서점 접속 없이 저장된 행을 다시 집계한다. 실서점 응답의 정확성은 네트워크와 HTML 상태에 달려 있다.
- `reclassify`는 기본 미리보기다. `--apply`를 붙여야 이동하며, 목적지 충돌과 카테고리 심볼릭 링크를 거부한다.
  같은 filesystem에서는 inode를 보존하고, 다른 filesystem에서는 복사 후 원본을 지운다. 파일 이동 실패를 출력·집계한 뒤 종료 코드 0을 반환한다.
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

PDF 텍스트 추출과 cover 렌더링은 `poppler-utils`의 `pdftotext`, `pdfinfo`, `pdftoppm`을 쓴다.
개발 환경에서는 Debian/Ubuntu의 `sudo apt-get install poppler-utils`로 설치하고 실행 파일을 `PATH`에서 찾는다.
별도 native library 경로 설정은 필요하지 않다. Docker runtime image에도 `poppler-utils`가 포함된다.
