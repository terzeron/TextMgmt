# bookdoctor P4 구현 계획 (HTML · 엔트로피 · 재귀 검사 · `--isolate`)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans (사용자가 inline 실행을 골랐고 subagent 독립 실행을 금지했다). Steps use checkbox (`- [ ]`) syntax.

**Goal:** 독립 HTML 파일을 검사하고, 텍스트 이름인데 압축·암호화된 듯한 멤버를 엔트로피로 알리고, 컨테이너 안의 컨테이너(중첩 ZIP·DOCX·PDF 등)를 깊이·예산 상한 안에서 재귀 검사하고, 파일마다 하위 프로세스에서 메모리·시간 상한을 거는 `--isolate`를 추가한다.

**Architecture:** HTML은 EPUB의 `Scanner::scan_html` 규칙을 그대로 재사용한다(`epub::security`에 공개 진입점 하나를 더한다). 엔트로피는 `scan_common::entropy`에 두고 ZIP 계열 멤버 검사에서 쓴다. 재귀는 `utils/bookdoctor/src/recurse.rs`가 멤버를 비공개 임시 디렉터리에 풀어 기존 `scan_file`을 부르고 결과를 위치 접두와 함께 합친다. `--isolate`는 자기 자신을 `sh -c 'ulimit -v N; exec …'`로 다시 실행하는 부모-자식 구조다.

**Tech Stack:** 표준 라이브러리, `sysdeps::SystemRunner`(프로세스 그룹 종료·시간 상한), 기존 `zip`·`flate2`. 새 의존성은 없다(`tempfile`은 dev 전용이라 임시 디렉터리는 표준 라이브러리로 만든다).

**Spec:** `docs/superpowers/specs/2026-10-30-bookdoctor-malware-scanner-design.md`

## Global Constraints

- 새 의존성을 추가하지 않는다. 필요하면 멈추고 승인을 받는다.
- 재귀는 깊이 상한(3)과 총 예산(추출 바이트 합계 `max_total`, 추출 파일 수 64)을 한 검사 전체가 공유한다. 상한에 걸리면 `incomplete`다.
- 임시 파일 이름에 멤버 이름을 쓰지 않는다. 임시 디렉터리는 소유자만 접근하고(0700), 검사가 끝나거나 panic해도 지운다.
- 자식 프로세스가 죽거나 시간·메모리 상한을 넘으면 그 파일을 `incomplete`로 보고하고 부모는 다음 파일로 간다.
- 출력 규칙은 P0~P3 그대로다(기본 `--min-severity high`, 암호화·DRM cyan).
- 해석하지 못했거나 상한에 걸린 부분은 `incomplete`로 남긴다. 깨끗하다고 하지 않는다.
- cargo는 `env -u TM_MYSQL_HOST -u TM_ES_URL`, `CARGO_TARGET_DIR=/home/terzeron/workspace/tm_dev/target`. 커밋은 Conventional Commits, 본문 `-` bullet. push·머지 없음. subagent를 쓰지 않는다.

## Review Focus

1. 저장된 일반 웹 페이지(스크립트·`onclick`이 흔하다)에 소음 High가 없다. 코퍼스 `.html` 3,034건으로 측정한다. (Task 1)
2. 엔트로피가 정상 멤버(이미 압축된 이미지·내장 파일)를 오탐하지 않는다. 텍스트 이름 멤버만 대상으로 한다. (Task 2)
3. 중첩 압축 폭탄(작은 입력, 깊은·넓은 중첩)이 시간·메모리·디스크를 터뜨리지 않는다. (Task 3)
4. 임시 파일이 멤버 이름 때문에 경로를 벗어나지 않고, 실패·panic 뒤에도 남지 않는다. (Task 3)
5. `--isolate`에서 자식이 멈추거나 죽어도 부모가 계속 돌고, 고아 프로세스가 남지 않는다. (Task 4)

---

### Task 1: HTML

**Files:** `libs/scan_common/src/finding.rs`(`Format::Html`), `utils/bookdoctor/src/{cli,common,scan,viewer,lib}.rs`, 신규 `utils/bookdoctor/src/html.rs`, `utils/bookdoctor/src/epub/security.rs`(공개 진입점), 테스트 `tests/rust/bookdoctor/html.rs`(신규, `[[test]]`).

**Interfaces:** Produces `epub::security::scan_html_document(text: &str, name: &str, report: &mut FileReport)`, `html::scan_html_file(path, opts, sniffed, report)`.

- `.html`·`.htm`·`.xhtml` 수집, `Format::Html`(JSON `"html"`). 내용이 바이너리(PDF·ZIP·OLE·RTF·실행 파일)면 `FORMAT_MISMATCH`이고 그 형식이 맞는 분석기로 넘기거나(PDF·ZIP) 실행 파일이면 `DISGUISED_EXECUTABLE`이다. 텍스트는 시작이 `<!DOCTYPE`·`<html`이 아니어도 허용한다(저장한 웹 페이지는 주석이나 `<meta>`로 시작하는 일이 흔하다).
- 읽기는 16MiB까지. 넘으면 앞부분만 보고 `incomplete`. 인코딩은 `epub::security::decode_text`를 쓴다.
- 규칙은 EPUB과 같다. 단 독립 HTML에서는 `SCRIPT_TAG`·`EVENT_HANDLER_ATTR`처럼 정상 웹 페이지에 흔한 지표를 Medium으로 낮출지 Step 4의 측정으로 정한다. 악용형 지표(`JS_URL`의 exploit 스킴, `DYNAMIC_CODE_EXEC`, `HOST_COMMAND_EXEC`, ActiveX)는 EPUB과 같은 등급을 유지한다.

- [ ] **Step 1: 실패하는 테스트.** `html_is_collected_and_lookalikes_are_not`(`a.html`·`b.HTM`·`c.xhtml` 수집, `d.htmlx` 제외), `plain_html_is_clean`, `comment_first_saved_page_is_html_not_a_mismatch`, `script_tag_event_handler_and_remote_script_use_the_epub_rules`, `binary_content_named_html_is_a_mismatch`(PDF는 PDF로, PE는 `DISGUISED_EXECUTABLE`), `large_html_is_incomplete_after_the_limit`, `utf16_html_is_decoded`.
- [ ] **Step 2: 실행해 실패를 확인한다.** **Step 3: 구현.**
- [ ] **Step 4: 코퍼스 `.html` 3,034건(`find -maxdepth 3`)을 `--min-severity low --json`으로 돌려 지표별 건수와 High 이상 비율을 보고하고, 일반 저장 페이지의 High가 1%를 넘으면 해당 지표를 독립 HTML에서만 Medium으로 낮춘다(등급 낮춤은 `ledger`에 `Ruling:`으로 남긴다).**
- [ ] **Step 5: 테스트·clippy 통과 후 커밋.** `feat: bookdoctor가 독립 HTML을 EPUB 규칙으로 검사한다`

### Task 2: 엔트로피

**Files:** 신규 `libs/scan_common/src/entropy.rs`(`pub mod`), `utils/bookdoctor/src/zipcheck/{fast,ooxml,hwpx}.rs` 중 멤버를 읽는 곳, 테스트 `tests/rust/bookdoctor/entropy.rs`(신규, `[[test]]`) 와 `libs/scan_common`의 단위 테스트.

**Interfaces:** Produces `entropy::shannon(bytes: &[u8]) -> f64`(비트/바이트, 0.0~8.0, 빈 입력은 0.0), `entropy::is_text_name(name: &str) -> bool`(확장자 `xml`·`rels`·`html`·`htm`·`xhtml`·`js`·`css`·`txt`·`json`·`svg`).

- ZIP·DOCX·HWPX 멤버 중 이름이 텍스트인데 앞 64KiB의 엔트로피가 임계값 이상이면 `HIGH_ENTROPY_TEXT_MEMBER`(Medium)다. 임계값은 Step 4의 측정으로 정한다(초안 7.2). 이미 압축된 형식의 이름(`jpg`·`png`·`zip`·`gz` 등)은 보지 않는다.
- 내용이 압축·암호화된 듯해도 이름이 텍스트면 렌더러가 읽지 못하므로 Medium으로 둔다. 같은 멤버가 매직으로 실행 파일이면 기존 규칙이 High 이상으로 잡는다.

- [ ] **Step 1: 실패하는 테스트.** `shannon_of_known_inputs`(전부 0 → 0.0, 256가지 바이트 균등 → 8.0, 두 값 균등 → 1.0, 빈 입력 → 0.0), `is_text_name_cases`(대소문자·경로·확장자 없음), `random_bytes_named_xml_in_a_zip_is_flagged`, `random_bytes_named_jpg_is_not_flagged`, `normal_xml_is_not_flagged`, `the_check_reads_only_the_first_64_kib`.
- [ ] **Step 2: 실행해 실패를 확인한다.** **Step 3: 구현.**
- [ ] **Step 4: 코퍼스 EPUB 표본(`tmp/bookdoctor-baseline/sample-epub.txt`)의 텍스트 이름 멤버 엔트로피 분포(최대, 99.9퍼센타일)를 재고, 표본에서 오탐이 없도록 임계값을 정한다.**
- [ ] **Step 5: 테스트·clippy 통과 후 커밋.** `feat: bookdoctor가 텍스트 이름 멤버의 높은 엔트로피를 알린다`

### Task 3: 재귀 검사

**Files:** 신규 `utils/bookdoctor/src/recurse.rs`, `utils/bookdoctor/src/{scan,cli,lib}.rs`, 테스트 `tests/rust/bookdoctor/recurse.rs`(신규, `[[test]]`).

**Interfaces:** Produces `recurse::Budget { depth: u8, bytes_left: u64, files_left: u32 }`(`Budget::root(max_total) -> Budget`, `child(&self) -> Option<Budget>`), `recurse::scan_zip_members(path: &Path, opts: &Options, budget: &mut Budget, report: &mut FileReport)`.

- 대상 포맷은 `zip`·`docx`·`hwpx`(P1 `Format::{Zip, Docx, Hwpx}`). 멤버를 앞 8바이트로 판별해 ZIP·PDF·RTF·Word/HWP OLE(루트에 `WordDocument`·`FileHeader`·`EncryptedPackage`가 있는 것)면 임시 파일로 풀어 `scan_file`과 같은 경로로 검사한다. 나머지 멤버(이미지, `vbaProject.bin`, 내장 `.bin` OLE, 일반 파일)는 재귀하지 않는다.
- 깊이 상한 3, 추출 바이트 합계는 `max_total`, 추출 파일 수 64. 하나라도 넘으면 `incomplete`(`recursion limit`).
- 결과는 `중첩경로!멤버` 위치 접두와 함께 바깥 보고서에 합친다(코드와 등급은 그대로). 안쪽 미완료 사유도 접두와 함께 합친다. 바깥 보고서가 이미 `--only` 필터 대상이므로 안쪽 보고서는 필터 전 상태로 합친다.
- 임시 디렉터리: `std::env::temp_dir()` 아래 `bookdoctor-<pid>-<카운터>`(0700), 파일 이름은 멤버 이름과 무관한 `m<번호>.<감지한 확장자>`. 정리는 `Drop`으로 한다.
- `--no-recurse`로 끈다. 기본은 켠다.

- [ ] **Step 1: 실패하는 테스트.** `a_zip_inside_a_docx_inside_a_zip_is_scanned_with_a_path_prefix`(안쪽 DOCX의 `vbaProject.bin`이 `a.zip!inner.docx!word/vbaProject.bin`로 나온다), `a_pdf_member_is_scanned`, `depth_limit_marks_incomplete`(5겹 중첩), `file_count_limit_marks_incomplete`(멤버 100개), `byte_budget_is_shared_across_members`(작은 예산에 큰 멤버 둘), `zip_bomb_quine_style_nesting_is_bounded`(같은 ZIP이 자기 안에 든 구조를 흉내 낸 깊은 중첩), `member_name_never_reaches_the_filesystem`(`../../x`·절대 경로·NUL·긴 이름), `temp_dir_is_removed_after_success_failure_and_panic`, `--no-recurse`.
- [ ] **Step 2: 실행해 실패를 확인한다.** **Step 3: 구현.**
- [ ] **Step 4: 코퍼스 ZIP·DOCX 표본으로 재귀가 지표를 더하지 않는지(정상 파일), 시간이 늘지 않는지 확인한다.**
- [ ] **Step 5: 테스트·clippy 통과 후 커밋.** `feat: bookdoctor가 컨테이너 안의 컨테이너를 상한 안에서 재귀 검사한다`

### Task 4: `--isolate`

**Files:** `utils/bookdoctor/src/cli.rs`, 신규 `utils/bookdoctor/src/isolate.rs`, `utils/bookdoctor/src/lib.rs`, 테스트 `tests/rust/bookdoctor/isolate.rs`(신규, `[[test]]`).

**Interfaces:** Produces `isolate::scan_isolated(exe: &Path, path: &Path, opts: &Options) -> FileReport`, 옵션 `--isolate`, `--isolate-mem-mb N`(기본 4096), `--isolate-timeout-s N`(기본 120), 내부 옵션 `--child`(자식은 한 파일을 `--json` 한 줄로 낸다).

- 부모는 파일마다 `SystemRunner`로 `sh -c 'ulimit -v "$0"; exec "$@"' <KiB> <exe> --child --json … <path>`를 실행하고 stdout의 JSON 한 줄을 `FileReport`로 읽는다(`scan_common`의 `serde` 기능).
- 자식이 죽거나(시그널·비정상 종료), 시간 상한을 넘기거나(프로세스 그룹 전체 종료), JSON을 못 읽으면 `incomplete`다: `isolated scan failed: <사유>`. 메모리 초과는 할당 실패로 abort하므로 이 경우에 해당한다.
- 자식에는 부모의 모든 옵션(`--zip-deep`, `--max-total-mb`, `--no-recurse` 등)을 그대로 넘기고 `--isolate`는 넘기지 않는다. 출력 필터(`--min-severity`, `--only`)는 부모가 건다.
- `--isolate`가 켜지면 선읽기(prefetch)는 끈다(자식이 읽으므로 의미가 없다).

- [ ] **Step 1: 실패하는 테스트.** `isolated_scan_matches_the_in_process_report`(같은 파일의 지표 집합이 같다), `a_child_that_aborts_is_reported_incomplete_and_the_parent_continues`(자식이 죽는 가짜 실행 파일 `sh` 스크립트), `a_child_over_the_timeout_is_killed_with_its_group`(`sleep` 손자를 낳는 가짜 자식, 종료 뒤 프로세스가 남지 않음), `memory_limit_kills_an_allocating_child`(`--isolate-mem-mb 64`로 큰 할당을 하는 가짜 자식), `garbage_child_output_is_incomplete`, `isolate_disables_prefetch_and_passes_options_through`.
- [ ] **Step 2: 실행해 실패를 확인한다.** **Step 3: 구현.**
- [ ] **Step 4: 표본 PDF 100건을 `--isolate`로 돌려 결과가 프로세스 안 검사와 같은지, 시간이 얼마나 늘어나는지 보고한다.**
- [ ] **Step 5: 테스트·clippy 통과 후 커밋.** `feat: bookdoctor에 파일별 하위 프로세스 격리(--isolate)를 추가한다`

### Task 5: 문서·완료 검증

- [ ] 갭 분석(HTML 행, 재귀 행, 난독화 행의 엔트로피), 사용법 문자열(`USAGE`, `print_usage`)을 갱신한다.
- [ ] `cargo test --workspace --locked --no-fail-fast`(한 번), `cargo clippy --workspace --all-targets --locked`, `Cargo.lock` diff(변경 없음이어야 한다), 표본 TXT·PDF·EPUB이 기준과 같은지 확인(재귀·엔트로피로 EPUB 출력이 바뀌지 않아야 한다).
- [ ] 자기 리뷰(Review Focus 5개)와 변조·폭탄 실험.
