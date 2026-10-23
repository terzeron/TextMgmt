# utils/ Rust workspace

`utils/*.py` 중 수행 시간이 중요한 도구를 옮긴 Cargo workspace. 실행 프로그램 하나가 crate 하나(디렉토리 하나)다.
Python backend가 직접 import하는 모듈(`loader`, `isbn`, `stat`, `parser_timeout`, `hwp3_*`, `file_time`, `corpus_layout`)은
원본을 유지한다. `corpus_layout` crate는 이후 Rust `loader`가 쓸 상수 라이브러리다.

## 크레이트

| crate | 종류 | 대응 Python | 비고 |
| --- | --- | --- | --- |
| `pdfscan` | bin | (신규) | PDF의 Launch·JavaScript·OpenAction·첨부 파일 등 실행 가능 요소를 정적 스캔. exit 0/1/2 |
| `detect_mojibake` | bin+lib | `detect_mojibake.py` (제거) | 인코딩 손상 탐지(txt·epub). `reader`는 `backend/classifier/reader.py`의 이식. EPUB 구조 검사는 `epubscan`으로 옮김 |
| `epubscan` | bin+lib | `detect_mojibake epub-structure` 통합 | `structure`: 뷰어가 못 그리는 EPUB 구조 검사, `scan`: 스크립트 실행 코드·위험 ZIP 구조 검출 |
| `quarantine_broken` | bin+lib | `quarantine_broken.py` (제거) | 손상 문서를 `0_broken`으로 격리·복구. 매니페스트 기반 undo |
| `cluster_hybrid_genres` | bin+lib | `cluster_hybrid_genres.py` (제거) | 5대 장르 비율 클러스터링 |
| `search` | bin | `search.py` (제거) | ES 제목·내용 검색 점검 |
| `es_client` | lib | `backend/es_manager.py` 검색부 | `search`가 쓰는 ES REST 클라이언트 |
| `corpus_layout` | lib | `corpus_layout.py` (유지) | 색인 제외 디렉토리 상수 |

변환하지 않은 것: `classify_cli`는 `backend.classifier`(scikit-learn, joblib 모델)를 구동하는 도구라 Rust로 옮길 수 없다.
`update.py`는 운영 ES에 문서를 쓰고 지우는 점검용이라 옮기지 않았다.

## 규칙

- **인자 없이 실행하면 용도와 사용법을 출력한다.** 모든 바이너리는 `print_usage()`를 갖고, 인자가 없거나 `-h`/`--help`면
  이것을 stdout에 출력하고 종료 코드 0으로 끝낸다(내용: 용도, 사용법, 인자, 옵션, 종료 코드). 잘못된 인자는 stderr에
  짧은 사용법을 쓰고 2로 끝낸다.
- **테스트를 먼저 쓴다(TDD).** Python 원본이 있으면 그 pytest를 먼저 Rust 테스트로 옮기고 실패를 확인한 뒤 구현한다.
- **커버리지는 100%에 가깝게 유지한다.** `cargo llvm-cov --workspace --summary-only`로 확인한다.
- Python 원본은 사용자가 지우라고 한 것만 지운다.

## 새 프로그램 추가

```
cd utils
cargo new --bin <name> --vcs none   # 루트 Cargo.toml의 members에 이름을 추가한다
```

`<name>/Cargo.toml`은 `version.workspace = true`, `edition.workspace = true`, `publish.workspace = true`를 쓰고,
공유 의존성은 `<dep>.workspace = true`로 받는다(버전은 루트 `[workspace.dependencies]`).

## 빌드·검증

```
cd utils
cargo build --release              # 산출물: utils/target/release/<name>
cargo test                         # 전체 crate
cargo clippy --all-targets
cargo llvm-cov --workspace --summary-only   # 커버리지
```

## 알아 둘 점

- ZIP을 열 때는 `detect_mojibake::reader::open_zip`을 쓴다. `zip` 크레이트 기본 설정(`Detect`)은 앞에 붙은 데이터를
  찾으려고 파일 전체를 훑어 1.5MB EPUB 하나에 읽기 1만 3천 번(48ms)이 든다. `open_zip`은 `Known(0)`으로 먼저 열고(0.2ms)
  실패하면 `Detect`로 다시 연다.
- Python과 결과가 같은지는 실제 코퍼스 표본으로 비교했다(`detect_mojibake` 2,023건·EPUB 1,210건, `cluster_hybrid_genres` 300건 차이 0건).
  비교는 `errors="ignore"` 디코딩 차이(Python 코덱은 잘못된 바이트를 1바이트만 버린다)를 잡아 `decode_korean_ignore`에 반영했다.
