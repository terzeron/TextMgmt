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
| `corpus_common` | 본문 표본 읽기, 인코딩 판별, ZIP 열기(`open_zip`), 경로·URL 도구, 병렬 풀 | `backend/classifier/reader.py` 일부 |
| `corpus_layout` | 색인 제외 디렉토리 상수 | `utils/corpus_layout.py` |
| `es_client` | Elasticsearch REST 클라이언트(http 전용): 검색, 갱신, 조회, 삭제 | `backend/es_manager.py` 일부 |
| `htmlkit` | HTML 토큰화와 BeautifulSoup 호환 텍스트 추출 | (bs4) |
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

## 규칙

- **인자 없이 실행하면 용도와 사용법을 출력한다.** 모든 바이너리는 `print_usage()`를 갖고, 인자가 없거나 `-h`/`--help`면
  stdout에 출력하고 종료 코드 0으로 끝낸다(용도, 사용법, 인자, 옵션, 종료 코드). 잘못된 인자는 stderr에 쓰고 2로 끝낸다.
- **테스트를 먼저 쓴다(TDD).** Python 원본이 있으면 그 pytest를 Rust 테스트로 먼저 옮기고 실패를 확인한 뒤 구현한다.
- **커버리지는 100%에 가깝게 유지한다.** `cargo llvm-cov --workspace --summary-only`.
- **라이브러리는 도구에 의존하지 않는다.** 공유할 코드는 `libs/`로 올린다.
- Python 원본은 사용자가 지우라고 한 것만 지운다.
- ZIP은 `corpus_common::reader::open_zip`으로 연다. `zip` 크레이트 기본 설정(`Detect`)은 파일 전체를 훑어 느리다
  (1.5MB EPUB 하나에 읽기 1만 3천 번, 48ms). `open_zip`은 `Known(0)`으로 먼저 열고(0.2ms) 실패하면 `Detect`로 다시 연다.

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
