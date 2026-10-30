# bookdoctor P0 구현 계획 (출력 규칙·실행 파일·점수)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `bookdoctor`의 출력 규칙(심각도 필터, 색)과 실행 파일 이중 확인, 조합 승격·위험 점수를 넣고, 새 포맷 P1~P5를 위한 갭 분석을 남긴다.

**Architecture:** 공통 모델(`libs/scan_common`)에 `Suppressed`, `color`, `risk`를 더하고, `utils/bookdoctor`의 `cli`·`report`·`scan`·`common`을 고친다. 심각도 필터는 `scan()` 안에서 `scan_file` 뒤에 적용하므로 `scan_file`의 기존 테스트는 영향을 받지 않는다. 새 `exe.rs`는 확장자와 매직바이트를 함께 본다.

**Tech Stack:** Rust workspace, `std::io::IsTerminal`(새 의존성 없음), 기존 `serde`·`serde_json`.

**Spec:** `docs/superpowers/specs/2026-10-30-bookdoctor-malware-scanner-design.md`

## Global Constraints

- 새 의존성을 추가하지 않는다. 필요하면 멈추고 사용자 승인을 받는다.
- 주석·문서는 한국어, 기존 모듈 머리 주석(`//!`) 문체를 따른다. 코드 식별자는 영어.
- 테스트는 `tests/rust/<crate>/`에 두고 `Cargo.toml`의 `[[test]]`로 등록한다.
- 모든 cargo 명령은 `env -u TM_MYSQL_HOST -u TM_ES_URL`로 실행하고 `CARGO_TARGET_DIR=/home/terzeron/workspace/tm_dev/target`을 쓴다.
- 각 태스크 끝에 `cargo test -p <touched crates> --locked`와 `cargo clippy -p <touched crates> --all-targets`가 경고 0이어야 한다.
- 모든 Rust 바이너리는 인자 없이 실행하면 `print_usage()`를 낸다(기존 동작 유지, 도움말에 새 옵션 추가).
- 커밋: Conventional Commits 제목, 본문은 `-` bullet, 명령형, 72자 이내.
- 작업 브랜치는 `feature/bookdoctor`다. push·머지·삭제는 하지 않는다.
- `FileReport` 경로는 원문 그대로 둔다. 색 코드는 `sanitize_display` 뒤에만 붙인다.

## Review Focus

각 줄의 테스트는 해당 태스크에 들어 있다.

1. 기본 출력에서도 `MOJIBAKE`(High)가 사라지지 않는다. `quarantine_broken`이 이 코드로 격리한다. (Task 2)
2. 파일명에 ESC(`\x1b`)가 있어도 색을 켠 출력에 공격자의 ESC가 그대로 나오지 않는다. (Task 3)
3. LOW·MEDIUM만 있는 파일은 종료 코드 0, 검사 못 한 파일만 있으면 2다. (Task 2)
4. 파이프·`NO_COLOR`·`--json`에는 ANSI 코드가 없다. (Task 3)
5. `.exe`인데 MZ가 없는 텍스트, MZ만 있고 PE 헤더가 없는 파일, `e_lfanew`가 64KiB 밖인 PE를 정확히 가른다. (Task 4)
6. 조합 승격이 실제 EPUB 표본 200건에서 소음을 만들지 않는다(승격 파일 4건 이하). (Task 5)

---

### Task 1: 갭 분석 문서

**Files:**

- Create: `docs/bookdoctor-gap-analysis.md`

**Interfaces:**

- Consumes: 소스의 지표 코드(`utils/bookdoctor/src/**`), `viewer.rs`의 `EPUB_CODES`·`PDF_CODES`·`TXT_CODES`, 설계서 `docs/superpowers/specs/2026-10-30-bookdoctor-malware-scanner-design.md`.
- Produces: P1~P5 계획이 참조하는 표. 코드는 바꾸지 않는다.

- [ ] **Step 1: 설계서 "공통 검출 기능" 12개와 PDF·EPUB 포맷별 항목을 행으로, 열은 `상태`(있음/부분/없음), `근거 코드`, `없을 때 담당 단계`로 표를 쓴다.** 상태는 소스에서 코드를 찾아 근거를 적는다. 근거가 없으면 "없음"이다. 추측하지 않는다.
- [ ] **Step 2: 확인하지 못한 항목은 "미확인"으로 적는다.**
- [ ] **Step 3: 검증.** 표의 "있음" 근거 코드 문자열이 모두 `rg -F "<코드>" utils/bookdoctor/src`로 1건 이상 나오는지 스크립트로 확인하고 결과를 문서 끝에 한 줄로 적는다.
- [ ] **Step 4: 커밋.** `docs: bookdoctor 갭 분석 추가`

---

### Task 2: 심각도 필터 (`--min-severity`)

**Files:**

- Modify: `libs/scan_common/src/severity.rs`, `libs/scan_common/src/finding.rs`, `utils/bookdoctor/src/cli.rs`, `utils/bookdoctor/src/report.rs`
- Test: `tests/rust/scan_common/finding.rs`, `tests/rust/bookdoctor/cli.rs`, `tests/rust/bookdoctor/report.rs`

**Interfaces:**

- Produces:
  - `Severity::parse(s: &str) -> Option<Severity>`: 소문자 `low|medium|high|critical`.
  - `pub struct Suppressed { pub low: usize, pub medium: usize, pub high: usize }` (`Default`, `Clone`, `Copy`, `PartialEq`, `Eq`, `Debug`), `Suppressed::is_empty(&self) -> bool`, `Suppressed::total(&self) -> usize`.
  - `FileReport.suppressed: Suppressed`. JSON에서는 `is_empty`일 때 생략한다.
  - `FileReport::suppress_below(&mut self, min: Severity)`: `severity < min`인 지표를 지우고 심각도별로 `suppressed`에 더한다.
  - `Options.min_severity: Severity`, 기본 `Severity::High`, 옵션 `--min-severity low|medium|high|critical`, 잘못된 값은 `invalid choice` 오류로 종료 코드 2.
  - `Aggregate`가 억제 건수를 합산하고, 텍스트 요약 줄 끝에 `, 억제 LOW N건, MEDIUM N건`(0이 아닌 것만)을 붙인다. 리포트 파일 요약에 `suppressed: {low, medium, high}`를 넣는다.

- [ ] **Step 1: 실패하는 테스트.**
  - `suppress_below_removes_lower_and_counts_by_severity`: Low 2·Medium 1·High 1이 있는 보고서에 `suppress_below(High)` 뒤 `findings.len()==1`, `suppressed == {low:2, medium:1, high:0}`.
  - `suppress_below_low_keeps_everything`.
  - `severity_parse_accepts_four_names_and_rejects_others`.
  - `suppressed_is_omitted_from_json_when_empty`.
  - CLI: `default_output_hides_low_and_medium`, `min_severity_low_shows_all`, `min_severity_invalid_exits_2`, `only_low_medium_file_exits_0`, `incomplete_only_file_exits_2`.
  - CLI: `summary_line_reports_suppressed_counts`.
  - CLI: `mojibake_high_survives_default_filter` (합성 UTF-8→CP949 오독 TXT가 `--json` 기본 실행에도 `MOJIBAKE`를 낸다).
- [ ] **Step 2: 실행해 실패를 확인한다.** `cargo test -p scan_common -p bookdoctor --locked`
- [ ] **Step 3: 구현.** `scan()`의 스캔 클로저에서 `scan_file(p, opts)` 뒤에 `r.suppress_below(opts.min_severity)`를 부른다(`scan_file` 자체는 바꾸지 않는다). `is_flagged`는 그대로 둔다. 도움말·`USAGE` 문자열에 `--min-severity`를 추가한다.
- [ ] **Step 4: 기존 테스트 정리.** 낮은 심각도 출력을 기대하던 기존 CLI·report 테스트는 `--min-severity low`를 주도록 고친다. 테스트 의미는 바꾸지 않는다.
- [ ] **Step 5: 회귀 확인.** 변경 전 HEAD로 `tmp/bookdoctor-baseline/sample-{epub,pdf,txt}.txt` 600건을 `--json`으로 돌린 출력(High 이상만 추출)과 변경 후 기본 실행 출력의 (파일, code, location) 집합이 같다. 다르면 멈추고 보고한다. 결과는 커밋 메시지가 아니라 태스크 보고서에 적는다.
- [ ] **Step 6: 테스트·clippy 통과 후 커밋.** `feat: bookdoctor 출력에서 LOW·MEDIUM을 기본으로 숨긴다`

---

### Task 3: 색 출력과 암호화 표시

**Files:**

- Create: `libs/scan_common/src/color.rs`
- Modify: `libs/scan_common/src/lib.rs`, `libs/scan_common/src/finding.rs`, `utils/bookdoctor/src/report.rs`, `utils/bookdoctor/src/cli.rs`, `utils/bookdoctor/src/epub/security.rs`(암호화 사유 문구)
- Test: `tests/rust/scan_common/color.rs`(신규, `[[test]]` 등록), `tests/rust/bookdoctor/report.rs`, `tests/rust/bookdoctor/cli.rs`

**Interfaces:**

- Produces:
  - `scan_common::color::Style { Red, Purple, Cyan }`, `paint(text: &str, style: Style, on: bool) -> String`. Red=`\x1b[31m`, Purple=`\x1b[35m`, Cyan=`\x1b[36m`, 끝에 `\x1b[0m`. `on == false`면 원문 그대로.
  - `ColorMode { Auto, Always, Never }`, `enabled(mode: ColorMode, is_tty: bool, no_color: bool) -> bool`: Always→true, Never→false, Auto→`is_tty && !no_color`. `NO_COLOR`는 값이 비어 있지 않을 때 설정된 것으로 본다.
  - `scan_common::finding::ENCRYPTION_CODES: &[&str] = &["ENCRYPTED"]`, `FileReport::is_encrypted_reason(reason: &str) -> bool`(소문자로 바꿔 `encrypted`로 시작).
  - `report::finding_style(f: &Finding) -> Option<Style>`: 코드가 `ENCRYPTION_CODES`면 Cyan, 분류가 `Render`·`Defect`면 Red, 그 밖에 Critical이면 Red, High면 Purple, 나머지는 `None`.
  - `write_text(r, out, color: bool)`: 지표 줄 전체를 `finding_style`로 칠한다. `INCOMPLETE` 줄은 `is_encrypted_reason`이면 Cyan. 색 코드는 `safe()`로 거른 문자열 바깥에만 붙인다.
  - `Options.color: ColorMode`, 기본 `Auto`, 옵션 `--color auto|always|never`. `Auto`는 `std::io::stdout().is_terminal()`과 `NO_COLOR`로 정한다. `--json`은 색을 쓰지 않는다.
- Consumes: Task 2의 `Options`·`write_text` 호출부.

- [ ] **Step 1: 실패하는 테스트.**
  - `paint_wraps_with_ansi_when_on`, `paint_is_identity_when_off`.
  - `enabled_truth_table`: Auto+tty+NO_COLOR 없음→true, Auto+tty+NO_COLOR→false, Auto+비tty→false, Always→true, Never→false.
  - `finding_style_rules`: Render Low→Red, Defect High→Red, Security High→Purple, Security Critical→Red, Security Medium→None, 코드 `ENCRYPTED`(Render High)→Cyan.
  - `encrypted_incomplete_reason_is_cyan`: 사유 `encrypted: string values may be unreadable`.
  - `escape_in_filename_never_reaches_output_raw`: 파일명에 `\x1b[31m`이 든 보고서를 `color=true`로 출력하면 출력의 ESC(`\x1b`)는 우리가 붙인 것뿐이다(`\x1b[` 뒤 숫자가 `31`·`35`·`36`·`0`인 시퀀스 개수가 칠한 줄 수와 일치).
  - CLI: `piped_output_has_no_ansi_in_auto_mode`, `json_output_has_no_ansi_even_with_color_always`, `no_color_env_disables_auto`(테스트가 `NO_COLOR`를 주입할 수 있게 `run`에 환경을 넘기는 내부 함수를 둔다).
- [ ] **Step 2: 실행해 실패를 확인한다.**
- [ ] **Step 3: 구현.** 위 인터페이스대로. EPUB의 zip 암호화 멤버 사유 문구를 `encrypted:`로 시작하게 바꾼다(`epub/security.rs`의 `s.partial(name, ...)` 한 곳). 이 문구를 단언하는 기존 테스트는 새 문구로 고친다.
- [ ] **Step 4: 도움말에 `--color`를 추가한다.**
- [ ] **Step 5: 테스트·clippy 통과 후 커밋.** `feat: bookdoctor 출력에 심각도·암호화 색을 입힌다`

---

### Task 4: 실행 파일 이중 확인

**Files:**

- Create: `utils/bookdoctor/src/exe.rs`
- Modify: `libs/scan_common/src/finding.rs`(`Format::Executable`), `utils/bookdoctor/src/lib.rs`, `utils/bookdoctor/src/scan.rs`, `utils/bookdoctor/src/common.rs`, `utils/bookdoctor/src/cli.rs`, `utils/bookdoctor/src/viewer.rs`, `utils/bookdoctor/src/report.rs`
- Test: `tests/rust/bookdoctor/exe.rs`(신규, `[[test]]` 등록), `tests/rust/bookdoctor/scan.rs`, `tests/rust/bookdoctor/viewer.rs`, `tests/rust/bookdoctor/cli.rs`

**Interfaces:**

- Produces:
  - `Format::Executable`(JSON `"executable"`).
  - `exe::EXEC_EXTENSIONS: &[&str] = &["exe","dll","sys","scr","ocx","cpl","drv","efi","so","elf","ko","dylib","bundle","bin","out"]`, `exe::is_exec_extension(ext: &str) -> bool`(대소문자 무시).
  - `common::inspect_with_sniff(path, opts, report) -> Option<(Vec<u8>, Sniffed)>`: 기존 `inspect`와 같되 `sniff_file`이 정한 `Sniffed`(먼 PE 헤더 확인 포함)를 함께 돌려준다. 기존 `common::inspect`는 이를 감싸는 래퍼로 남겨 기존 테스트 호출을 유지한다.
  - `exe::scan_exe_file(sniffed: Sniffed, ext: &str, report: &mut FileReport)`:
    - 확장자가 실행 파일이고 `sniffed`가 `Elf|MachO|Pe`면 `Category::Security`, `Severity::High`, 코드 `EXECUTABLE_FILE`, 상세 `실행 파일 확정: ELF|Mach-O|PE`.
    - 확장자가 실행 파일인데 `sniffed`가 그 밖이면 `Category::Defect`, `Severity::Low`, 코드 `EXECUTABLE_EXTENSION_NO_MAGIC`, 상세에 판별된 종류.
  - `dispatch_format`: 확장자가 `is_exec_extension`이면 `Some(Format::Executable)`. `scan_inner`가 `inspect_with_sniff`를 쓰고 `Format::Executable`이면 `exe::scan_exe_file`을 부른다.
  - `cli::wanted`: `EXEC_EXTENSIONS`도 수집한다. 명시한 파일 경로가 이미 확장자 필터를 거치는지 확인하고, 거치지 않으면 그 동작을 유지한다.
  - `viewer::viewer_blocked`: `Format::Executable`은 항상 `NotApplicable`. 코드 집합 대조 테스트(`viewer.rs`)가 새 코드를 요구하면 표에 `NotApplicable`로 추가한다.
  - `Aggregate`가 `EXECUTABLE_FILE` 지표가 있는 파일 수를 세고, 텍스트 요약 줄에 `, 실행 파일 N건`(0이면 생략)을 붙인다. 리포트 파일 요약에 `executables`를 넣는다.
  - 기존 `DISGUISED_EXECUTABLE`(확장자 epub·pdf·txt인데 실행 파일 내용)은 그대로 둔다.

- [ ] **Step 1: 실패하는 테스트.**
  - `exe_with_pe_header_is_confirmed`: 최소 PE(`MZ`, `e_lfanew=0x40`, `PE\0\0`) `.exe` → `EXECUTABLE_FILE`.
  - `elf_so_is_confirmed`: `\x7fELF` `.so`. `macho_dylib_is_confirmed`: `\xCF\xFA\xED\xFE` `.dylib`.
  - `exe_extension_with_text_content_is_not_confirmed`: 평문 `.exe` → `EXECUTABLE_EXTENSION_NO_MAGIC`(Low), `EXECUTABLE_FILE` 없음.
  - `mz_without_pe_header_is_not_confirmed`.
  - `pe_with_far_e_lfanew_is_confirmed`: `e_lfanew`가 64KiB 밖이고 파일 안인 PE.
  - `extension_is_case_insensitive`: `.EXE`.
  - `executable_file_is_high_and_survives_default_filter`.
  - `summary_line_counts_executables`.
  - `txt_with_exe_content_still_reports_disguised_executable`(기존 동작 유지).
  - `viewer_blocked_is_not_applicable_for_executable`.
- [ ] **Step 2: 실행해 실패를 확인한다.**
- [ ] **Step 3: 구현.** 위 인터페이스대로.
- [ ] **Step 4: 도움말의 포맷 설명과 PATH 인자 설명에 실행 파일을 추가한다.**
- [ ] **Step 5: 테스트·clippy 통과 후 커밋.** `feat: bookdoctor가 실행 파일을 확장자와 매직바이트로 확인한다`

---

### Task 5: 조합 승격과 위험 점수

**Files:**

- Create: `libs/scan_common/src/risk.rs`
- Modify: `libs/scan_common/src/lib.rs`, `libs/scan_common/src/finding.rs`, `utils/bookdoctor/src/scan.rs`, `utils/bookdoctor/src/viewer.rs`
- Test: `tests/rust/scan_common/risk.rs`(신규, `[[test]]` 등록), `tests/rust/bookdoctor/scan.rs`

**Interfaces:**

- Produces:
  - `risk::risk_score(findings: &[Finding]) -> u8`: 지표가 없으면 0. 있으면 `min(100, 최고 가중치 + 5 × (Medium 이상 지표 수 − 1))`, 가중치는 Critical 100, High 70, Medium 30, Low 10. Medium 이상이 없으면 최고 가중치만.
  - `FileReport.risk_score: u8`(JSON 항상 포함, 기본 0).
  - `risk::COMBINATIONS: &[(&str, &[&str])] = &[("OBFUSCATION", &["NETWORK_API", "NAVIGATION", "DYNAMIC_IMPORT"])]`, `risk::escalate(report: &mut FileReport)`: 보고서에 왼쪽 코드와 오른쪽 중 하나가 함께 있고 `COMBINED_SUSPICIOUS`가 아직 없으면 `Category::Security`, `Severity::High`, 코드 `COMBINED_SUSPICIOUS`, 상세 `OBFUSCATION + NETWORK_API`처럼 실제 만난 코드 쌍을 낸다.
  - `scan_file`은 포맷 검사 뒤 `--only` 필터 전에 `escalate`를 부르고, 마지막에 `report.risk_score = risk_score(&report.findings)`를 정한다. 점수는 `--only`와 심각도 필터를 적용하기 전의 지표로 계산한다(`suppress_below`는 `risk_score`를 건드리지 않는다).
  - `viewer` 표: `COMBINED_SUSPICIOUS`는 `NotApplicable`(EPUB_CODES 대조 테스트가 요구하면 추가).

- [ ] **Step 1: 실패하는 테스트.**
  - `risk_score_empty_is_zero`, `risk_score_uses_max_weight`, `risk_score_adds_five_per_extra_medium_or_higher`, `risk_score_caps_at_100`, `risk_score_ignores_low_for_the_bonus`.
  - `escalate_adds_combined_when_pair_present`, `escalate_does_nothing_when_only_one_side`, `escalate_is_idempotent`.
  - `scan_file_keeps_score_from_suppressed_findings`: Medium 지표만 있는 보고서에 `suppress_below(High)`를 적용해도 `risk_score`가 30 이상으로 남는다.
- [ ] **Step 2: 실행해 실패를 확인한다.**
- [ ] **Step 3: 구현.**
- [ ] **Step 4: 소음 게이트.** `tmp/bookdoctor-baseline/sample-epub.txt` 200건을 `--min-severity low --json`으로 돌려 `COMBINED_SUSPICIOUS`가 있는 파일 수를 센다. 4건을 넘으면 `COMBINATIONS`를 빈 목록으로 두고(규칙 비활성) 수치를 태스크 보고서에 적은 뒤 멈춘다. 4건 이하면 수치를 보고서에 적고 진행한다.
- [ ] **Step 5: 테스트·clippy 통과 후 커밋.** `feat: bookdoctor에 위험 점수와 조합 승격을 추가한다`

---

## P0 완료 검증 (컨트롤러)

- [ ] `cargo test --workspace --locked --no-fail-fast`, `cargo clippy --workspace --all-targets`(bookdoctor·scan_common·corpus_common·quarantine_broken 경고 0).
- [ ] 표본 600건 기본 실행의 High 이상 집합이 P0 전과 같다(Task 2 Step 5 재확인) + `EXECUTABLE_FILE`·`COMBINED_SUSPICIOUS`만 추가.
- [ ] 갭 분석 문서가 P1~P5 계획의 입력으로 충분한지 읽고 확인한다.
