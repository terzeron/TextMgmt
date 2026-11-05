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
| `text_decoder` | 바이트를 텍스트로 푼다: 잘못된 바이트를 버리는 UTF-8·CP949 디코딩과 한글 음절 판정, 인코딩 이름과 코덱(`text_transcoder`), 인코딩 추정 휴리스틱 `detect_hint`·`decode_best`와 `korean_likeness`(`encoding_guesser`). `doc_checker`와 `doc_reader`의 아래 계층이다 |
| `xml_recoverer` | 깨진 XML을 엄격한 파서(`roxmltree`)가 다시 읽을 수 있게 고치는 문자열 변환: 선언 없는 접두사 선언, DOCTYPE·모르는 엔티티 제거, 잘린 태그 닫기. 파싱은 하지 않고 변환 순서는 호출자가 정한다. backend의 EPUB 메타데이터·검사·표지 편집이 쓴다 |
| `doc_reader` | 문서 본문 요약 추출(TXT·EPUB·HTML·RTF·DOCX·PDF·HWP)과 파일 앞뒤 읽기. 인코딩 손상 판정은 `doc_checker::mojibake_detector`를 쓴다 |
| `es_client` | Elasticsearch REST 클라이언트: HTTP와 인증서 검증 HTTPS, 검색·갱신·조회·삭제. backend `ESManager`가 쓰는 저수준 전송(`transport`)과 공용 `EsError`도 여기 있다 |
| `htmlkit` | HTML 토큰화와 텍스트 추출 |
| `doc_checker` | 문서 파일 검사와 교정 기법. EPUB·PDF·TXT·HWP·RTF·ZIP 계열의 보안·결함·렌더링 차단 지표(`format_dispatcher::scan_file`), 보안 정리(`epub_sanitizer`, `pdf::pdf_sanitizer`)와 결함 수리(`defect_repairer`)의 재구성·검증, TXT의 UTF-8 재인코딩(`encoding_repairer`), 인코딩 손상 판정(`mojibake_detector`). OLE 복합 파일 파서(`ole_parser`)를 포함한다. 파일 하나를 받아 보고서나 고친 바이트를 돌려줄 뿐, 파일 순회·적용·출력은 하지 않는다. 검사 설정은 `CheckOptions`다 |
| `common` | 여러 crate가 함께 쓰는 기능: HTTP 요청·응답과 `ureq` 구현(`http_client`), URL 나누기·잇기·인코딩(`url_parser`), JSON 필드 읽기(`json_field_reader`), DTD를 허용하는 XML 해석(`xml_parser`), ZIP 열기와 멤버 읽기 상한(`zip_reader`), panic 메시지 읽기(`panic_message`). 시스템 능력이나 특정 문서 형식에 속하지 않고 두 crate 이상이 쓰는 것만 둔다. `http_client`·`xml_parser`·`zip_reader`는 feature이고, 필요 없는 crate는 `default-features = false`로 끈다 |
| `bookstore` | Yes24·알라딘·교보문고 검색과 상세 추출. backend가 쓰는 전체 서점 구현(`stores`: 리디북스·네이버·문피아·조아라·웹툰 포함)도 여기 있다 |
| `stat` | 색인·파싱 통계 카운터 |
| `db` | MySQL 접근 추상화와 카테고리 매핑 저장소(`category_mapping`) |
| `classifier` | 도서 분류기 껍데기: backend 분류 제안이 쓰는 `Classify`·`BookClassifierApi` trait와 모델 단독 어댑터. 구현은 없어서 backend는 카테고리 매핑 키워드만으로 제안한다 |
| `file_indexer` | 파일 하나를 색인 레코드로 읽기: 종류 판별, 본문·메타데이터·생성 시각 추출, 파서 자식 프로세스 격리, 색인 제외 디렉토리 규칙(`layout`). loader와 backend가 함께 쓴다 |
| `sysdeps` | 시계, 파일시스템, 외부 명령 같은 시스템 능력의 trait와 실제 구현. 파서 시간 상한(`timeout`)과 PDF worker(`tm-pdf-worker`)를 포함한다 |
| `isbn_extractor` | 파일에서 ISBN 추출(TXT·EPUB·PDF·DJVU·HWP) |
| `hwp3_parser` | HWP 3.x 텍스트 추출. 생성된 HNC→유니코드 매핑 표(`tables`)를 포함한다 |

## utils (도구)

| crate | 용도 |
| --- | --- |
| `bookdoctor` | 문서 파일 진단(`scan`), 보안 정리(`sanitize`), 결함 수리(`repair`), 손상 문서 격리·복구(`quarantine`·`unquarantine`) 명령. 명령행, 파일 순회와 병렬 실행, 적용(휴지통 보관·덮어쓰기), 보고 출력을 맡고 검사·교정 기법은 `doc_checker`를 쓴다 |
| `loader` | 책·만화 디렉토리의 파일을 읽어 ES 색인을 만들고 맞춘다 |

## 규칙

- `-h`/`--help`는 도움말을 출력한다. `loader`는 인자 없이 실행하면 사용법을 출력하고 코드 1로 끝난다.
- 잘못된 인자의 출력 위치와 종료 코드는 각 CLI 계약을 따른다.
- **테스트 파일은 소스 모듈과 1:1로 둔다.** `tests/rust/<crate>/<모듈 경로>.rs` 하나가 `src/<모듈 경로>.rs` 하나를 검증한다(예: `doc_checker/epub/security_checker.rs`).
  HTTP 라우트, CLI 진입점, 실행 파일, 소켓, 컨테이너를 거치는 테스트는 `tests/rust/<crate>/integration/<시나리오>.rs`에 두고 target 이름을 `integration_<시나리오>`로 한다.
  `*_gaps`, `*_parity`, `*_1`처럼 작업 이력을 따르는 파일 이름은 쓰지 않는다.
  여러 crate 테스트가 함께 쓰는 helper(TLS 테스트 서버와 인증서)는 `tests/rust/shared/`에 둔다. `tests/rust/common/`은 `common` crate의 테스트다.
  `mod common;`으로 helper 모듈을 둔 테스트 파일에서는 `common` crate를 `::common::`으로 부른다.
  private 항목을 검증하는 테스트는 그 소스 파일 안의 `#[cfg(test)] mod tests`에 둔다. `#[path]`로 `tests/` 밑 파일을 끌어오지 않는다.
- **테스트를 먼저 쓴다(TDD).** 정해진 동작 사례를 Rust 테스트로 먼저 옮기고 실패를 확인한 뒤 구현한다.
- **커버리지는 100%에 가깝게 유지한다.** `cargo llvm-cov --workspace --summary-only`.
- **라이브러리와 backend는 도구에 의존하지 않는다.** 공유할 코드는 `libs/`로 올린다.
- **한 crate만 쓰는 코드는 그 crate 안에 둔다.** 사용자가 둘 이상이 될 때 `libs/`로 올린다.
- `bookstore`는 공개 서점 HTTPS를 사용한다. ES client도 HTTP와 인증서 검증 HTTPS를 지원하며, 사설 CA는 `TM_ES_CA_CERT` PEM 경로로 지정한다. 인증서 검증을 끄는 우회로는 두지 않는다.
- ZIP 안 경로는 zip crate 규칙을 따른다. `std::path::Path`로 잇고 `common::zip_reader::zip_entry_name`으로 항목 이름을 만든다(`.` 무시, `..`은 앞 조각 제거, 루트 밖이면 `None`). 별도 경로 모듈은 두지 않는다.
- ZIP은 `common::zip_reader::open_zip`으로 연다. 멤버는 `read_member`(상한을 넘으면 거절)나 `read_member_prefix`(상한에서 자름)로 읽는다. `zip` 크레이트 기본 설정(`Detect`)은 파일 전체를 훑어 느리다
  (1.5MB EPUB 하나에 읽기 1만 3천 번, 48ms). `open_zip`은 `Known(0)`으로 먼저 열고(0.2ms) 실패하면 `Detect`로 다시 연다.

## Backend 세션 인증

- Access token에 Refresh token과 같은 세션 식별자(`fid`)를 넣는다.
  보호 API는 JWT 서명·만료·현재 역할과 해당 세션의 유효 상태를 확인한다.
- 로그아웃이나 관리자 세션 폐기는 다음 요청부터 해당 세션의 Access token도 거부한다.
  다른 기기의 세션은 유지한다. Refresh token 회전은 같은 `fid`를 유지한다.
- 세션 상태는 기존 `refresh_tokens` 테이블에서 조회한다. DB 스키마 변경은 없다.
  즉시 폐기를 위해 상태를 캐시하지 않으므로, 인증 요청마다 MySQL 조회가 추가된다.
- 세션 저장소 장애 시 보호 API는 `503`을 반환한다. 폐기·만료·누락 세션은 `401`을 반환한다.
- 배포 전 발급한 Access token에는 `fid`가 없으므로 `401`을 반환한다.
  기존 Refresh token으로 갱신하거나 다시 로그인하면 세션에 연결한 Access token을 받는다.
- 운영용 토큰은 `create_session_jwt_token` 또는 `TokenIssuer::create_access_token`으로 발급한다.
  `create_jwt_token`의 세션 없는 토큰은 저수준 JWT 검증 테스트용이며 보호 API 인증에는 사용할 수 없다.
- 쿠키를 사용하는 상태 변경 요청과 Google 로그인은 `TM_FRONTEND_URL`과 같은 Origin만 허용한다.
  Origin이 없으면 Referer의 scheme·host·port를 비교한다. 출처가 없거나 다르면 처리 전에 `403`을 반환한다.
  Origin이 있으면 Referer로 우회할 수 없다. `null` 또는 중복 Origin도 거부한다.
- JSON 본문은 `application/json` 또는 `application/*+json` Content-Type을 지정해야 한다.
  헤더 누락과 다른 본문 형식은 기존 입력 오류 계약에 따라 `422`를 반환한다.
- Bearer만 사용하는 CLI는 Origin 없이 호출할 수 있다. 인증 쿠키도 보내면 Origin 또는 Referer를 지정해야 한다.
  쿠키 없는 익명 디버깅 로그에는 출처 검사를 적용하지 않는다.

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

PDF 쪽수·텍스트·cover 렌더링은 MuPDF Rust library를 사용한다. 원본 객체를 보존하는 미리보기와 편집은 lopdf를 사용한다.
자체 `tm-pdf-worker`에서 네이티브 파서를 실행해 30초·512MiB 제한을 유지한다. 빌드·배포 패키지와 큰 PDF 처리 기준은 [PDF engine](pdf-engine.md)을 본다.
