# bookdoctor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `pdfscan`, `epubscan`, `detect_mojibake`를 `utils/bookdoctor` 하나로 합치고, 세 도구와 산출물을 모두 제거한다.

**Architecture:** 공통 모델(`Finding`, `FileReport`, 매직바이트 판별)은 `libs/scan_common`에 둔다. 포맷별 검사기(`epub`, `pdf`, `txt`)와 `mojibake`는 `utils/bookdoctor`의 모듈이다. 기존 검사 코드는 옮겨 오되 **옛 도구는 동등성 검증이 끝날 때까지 지우지 않는다**(Task 16이 옛 출력을 기준으로 쓴다). 삭제는 Task 17에서 한다.

**Tech Stack:** Rust 2024 workspace, `lopdf`, `zip`, `roxmltree`, `regex`, `fancy-regex`, `encoding_rs`, `rayon`, `serde_json`, Poppler(`libs/sysdeps::poppler`).

**Spec:** `docs/superpowers/specs/2026-10-29-bookdoctor-design.md` (§6 검사 목록, §12 확정 사항). 실행자는 두 문서를 모두 읽는다.

## Global Constraints

- 새 외부 crate를 추가하지 않는다(의존성 변경은 승인 대상). 모든 의존성은 이미 `Cargo.lock`에 있는 버전을 쓴다. 마지막에 `git diff Cargo.lock`이 `bookdoctor` 추가와 세 crate 제거만 보여야 한다. 그렇지 않으면 멈추고 사용자에게 묻는다.
- 심각도는 `scan_common::Severity`(`Low`·`Medium`·`High`·`Critical`)만 쓴다. 분류는 `security`·`defect`·`render` 셋이다.
- 종료 코드: 발견이 하나라도 있으면 `1`, 없고 `incomplete`만 있으면 `2`, 둘 다 없으면 `0`(`Summary::exit_code()`가 이미 이 규칙이다). 잘못된 인자는 `2`다.
- 인자 없이 실행하면 `print_usage()`로 용도·사용법을 출력한다(프로젝트 규칙).
- UTF-8 BOM(`EF BB BF`)은 정상이다. UTF-8이 아닌 TXT는 모두 `High`다. UTF-16 BOM은 UTF-8이 아니다.
- `incomplete`는 발견 항목이 아니라 `FileReport`의 별도 필드(`incomplete`, `reasons`)다.
- 인코딩 손상 판정은 TXT·EPUB는 기본, PDF는 `--pdf-text`일 때만 한다. 본문 추출 실패는 `incomplete`다.
- `quarantine_broken`은 코드 `MOJIBAKE`인 항목만 격리한다. 동작은 지금과 같다.
- 파서 panic은 `catch_unwind`로 격리하고 그 파일을 `incomplete`로 보고한다. `Cargo.toml`의 `panic = "unwind"`는 유지한다.
- 커밋: Conventional Commits 제목, 본문은 `-` bullet 명령형 72자 이내. 한 task에 커밋 한 번이 기본이다.
- 테스트 배선은 프로젝트 관례를 따른다: crate에 `autotests = false`와 `[[test]] name=… path = "../../tests/rust/<crate>/<file>.rs"`.
- 각 task의 마지막에 `cargo clippy -p <crate> --all-targets -- -D warnings`가 통과해야 한다.
- 개발은 격리된 worktree에서 한다(`superpowers:using-git-worktrees`). 테스트 전에 셸의 `TM_MYSQL_HOST`, `TM_ES_URL`을 확인한다(테스트가 실서비스에 붙을 수 있다).

## Review Focus

설계가 말하지 않았지만 사용자가 만날 입력이다. 각 줄의 테스트는 소유 task에 들어 있다.

1. UTF-16 BOM을 가진 TXT: `NOT_UTF8`으로 잡고 panic하지 않는다. UTF-8 BOM TXT는 아무것도 잡히지 않는다 (Task 6).
2. 확장자만 `.pdf`인 EPUB(실제 5건): EPUB로 검사하고 `FORMAT_MISMATCH`를 한 번만 보고한다 (Task 3, 5).
3. 없는 경로나 읽을 수 없는 디렉토리: 조용히 건너뛰지 않고 `incomplete`, 종료 코드 2다 (Task 3).
4. 제어문자·비UTF-8 바이트가 들어간 파일명: 출력과 JSON이 깨지지 않고 이스케이프된다 (Task 4).
5. 파서가 panic하는 손상 입력(잘린 ZIP, 잘린 PDF): 배치가 중단되지 않고 그 파일만 `incomplete`다 (Task 8, 10).

---

## File Structure

| 경로                                                                        | 책임                                                                      |
| --------------------------------------------------------------------------- | ------------------------------------------------------------------------- |
| `libs/scan_common/src/finding.rs` (신규)                                    | `Format`, `Category`, `ViewerBlocked`, `Finding`, `FileReport`            |
| `libs/scan_common/src/sniff.rs` (신규)                                      | `Sniffed`, `sniff()` 매직바이트 판별                                      |
| `libs/scan_common/src/names.rs` (신규)                                      | `windows_reserved()` 파일명 규칙                                          |
| `utils/bookdoctor/src/{main,lib,cli,scan,report,viewer,common,mojibake}.rs` | 진입점, 옵션, 디스패치, 출력, `viewer_blocked` 표, 공통 검사, 인코딩 손상 |
| `utils/bookdoctor/src/txt.rs`                                               | TXT 검사                                                                  |
| `utils/bookdoctor/src/epub/{mod,security,structure,render}.rs`              | EPUB 보안(이식), 구조(이식), 프론트 규칙·손상·렌더링(신규)                |
| `utils/bookdoctor/src/pdf/{mod,security,structure}.rs`                      | PDF 보안(이식·확장), 구조·렌더링(신규)                                    |
| `tests/rust/bookdoctor/*.rs`                                                | 테스트. 공용 fixture는 `common/mod.rs`                                    |

---

### Task 1: worktree와 옛 도구 기준선

**Files:** 변경 없음. 산출물은 `tmp/bookdoctor-baseline/`(gitignore됨)다.

- [ ] **Step 1:** `superpowers:using-git-worktrees`로 worktree를 만든다. 브랜치 이름은 `feature/bookdoctor`다.
- [ ] **Step 2:** 옛 바이너리를 빌드한다: `cargo build -p pdfscan -p epubscan -p detect_mojibake`. Expected: 성공.
- [ ] **Step 3:** 옛 도구의 테스트 기준선을 기록한다: `cargo test -p pdfscan -p epubscan -p detect_mojibake -p quarantine_broken 2>&1 | tail -30`. 통과 건수를 `tmp/bookdoctor-baseline/old-tests.txt`에 저장한다. Expected: 실패 0.
- [ ] **Step 4:** 아래 입력 목록을 만든다(`tmp/bookdoctor-baseline/`). 목록은 Task 16이 쓴다.
  - `epub-structure-flagged.txt`: `tmp/epub_structure_report*.json`의 `flagged[].path`를 모두 모아 중복 제거.
  - `pdf-redownload.txt`: `/home/terzeron/tmp/redownload_list.txt`에서 `#`로 시작하지 않는 줄.
  - `epub-redownload.txt`: `tmp/epub_redownload_20261022.txt`에서 `#`로 시작하지 않는 줄.
  - `sample-epub.txt`, `sample-pdf.txt`, `sample-txt.txt`: `/mnt/data/text`에서 각각 200개를 `shuf --random-source=<(yes)`로 고정 추출.
  - `broken-dir.txt`: `/mnt/data/text/0_broken`이 있으면 그 파일 목록, 없으면 빈 파일.
    Expected: 각 파일의 `wc -l`을 `old-tests.txt` 끝에 적는다.
- [ ] **Step 5:** 커밋하지 않는다(산출물이 gitignore 대상이다).

---

### Task 2: 공통 모델 (`scan_common`)

**Files:**

- Create: `libs/scan_common/src/{finding,sniff,names}.rs`
- Modify: `libs/scan_common/src/lib.rs`, `libs/scan_common/Cargo.toml`(`[[test]] name="finding"`, `"sniff"`)
- Test: `tests/rust/scan_common/{finding,sniff}.rs`

**Interfaces:**

- Produces (`scan_common::finding`), `serde` feature에서 `Serialize`:
  ```rust
  pub enum Format { Epub, Pdf, Txt }                       // "epub" | "pdf" | "txt"
  pub enum Category { Security, Defect, Render }           // "security" | "defect" | "render"
  pub enum ViewerBlocked { Blocked, NotBlocked, NotApplicable } // "blocked" | "not_blocked" | "n/a"
  pub struct Finding { pub category: Category, pub severity: Severity, pub code: String,
                       pub location: String, pub detail: String, pub viewer_blocked: ViewerBlocked }
  pub struct FileReport { pub path: String, pub format: Option<Format>, pub findings: Vec<Finding>,
                          pub incomplete: bool, pub reasons: Vec<String>, pub likeness: Option<f64> }
  impl FileReport {
      pub fn new(path: &Path, format: Option<Format>) -> Self;
      pub fn push(&mut self, category: Category, severity: Severity, code: &str, location: &str, detail: &str);
      pub fn mark_incomplete(&mut self, reason: &str);
      pub fn has(&self, code: &str) -> bool;
  }
  impl Report for FileReport   // severities() = findings의 심각도, is_incomplete() = incomplete
  ```
  `push`는 `viewer_blocked`를 `NotApplicable`로 둔다(Task 14가 채운다). `path`는 `text::sanitize`로 제어문자를 제거한 값이다.
- Produces (`scan_common::sniff`): `pub enum Sniffed { Pdf, Zip, Elf, MachO, Pe, Html, Unknown }`, `pub fn sniff(head: &[u8]) -> Sniffed`.
  `Pdf`는 앞 1024바이트 안의 `%PDF-`, `Zip`은 `PK\x03\x04`·`PK\x05\x06`, `Elf`는 `\x7fELF`, `Pe`는 `MZ`, `MachO`는 `FEEDFACE/FEEDFACF/CEFAEDFE/CFFAEDFE/CAFEBABE`, `Html`은 BOM·공백을 건너뛴 `<!doctype html` 또는 `<html`(대소문자 무시).
- Produces (`scan_common::names`): `pub fn windows_reserved(component: &str) -> bool`. 로직은 `utils/epubscan/src/security.rs:416-424`의 `WINDOWS_RESERVED_NAME` 판정을 옮긴 것이다.

- [ ] **Step 1: 실패하는 테스트를 쓴다**
  ```rust
  #[test] fn push_records_a_finding_and_report_trait_sees_it() {
      let mut r = FileReport::new(Path::new("a.txt"), Some(Format::Txt));
      r.push(Category::Defect, Severity::High, "NOT_UTF8", "", "encoding=cp949");
      assert!(r.has("NOT_UTF8") && !r.has("X"));
      assert_eq!(r.severities().collect::<Vec<_>>(), vec![Severity::High]);
      assert!(!r.is_incomplete());
  }
  #[test] fn mark_incomplete_keeps_the_reason() { /* reasons == ["skipped"], is_incomplete() */ }
  #[test] fn path_is_sanitized_of_control_characters() { /* "a\u{1b}b.txt" -> no \u{1b} in r.path */ }
  #[test] fn json_uses_lowercase_category_and_na_viewer() { /* serde_json: "defect", "n/a" (feature serde) */ }
  ```
  `sniff.rs` 테스트: 이름 `sniffs_<kind>`로 Pdf(오프셋 300의 `%PDF-` 포함), Zip, Elf, MachO(4종 magic), Pe, Html(BOM+공백 뒤 `<HTML>`), 평문은 `Unknown`. `names.rs`: `windows_reserved("CON")`, `("com1.txt")`, `("aux")`는 참, `("console")`은 거짓.
- [ ] **Step 2:** `cargo test -p scan_common --features serde` → FAIL(모듈 없음).
- [ ] **Step 3:** 위 시그니처대로 구현한다. `lib.rs`에 `pub mod finding; pub mod names; pub mod sniff;`와 `pub use finding::{…}`를 추가한다. `Severity`의 `Serialize`는 기존 `serde` feature를 쓴다.
- [ ] **Step 4:** `cargo test -p scan_common --features serde` → PASS.
- [ ] **Step 5:** `git add libs/scan_common tests/rust/scan_common && git commit -m "feat: 파일 진단 공통 모델과 매직바이트 판별을 추가한다"` (본문 bullet 2줄).

---

### Task 3: bookdoctor 뼈대, CLI, 디스패치

**Files:**

- Create: `utils/bookdoctor/{Cargo.toml,src/{main,lib,cli,scan}.rs}`, `tests/rust/bookdoctor/{bin,cli,scan}.rs`, `tests/rust/bookdoctor/common/mod.rs`
- Modify: 루트 `Cargo.toml` members에 `"utils/bookdoctor"` 추가.

**Interfaces:**

- Consumes: Task 2의 `FileReport`, `Format`, `sniff`; `scan_common::{collect_files, scan_streaming, pool, DEFAULT_WORKERS, args::{long_option, number}}`.
- Produces:

  ```rust
  // cli.rs
  pub struct Options { pub paths: Vec<PathBuf>, pub recursive: bool, pub json: bool,
      pub report: Option<PathBuf>, pub workers: usize, pub limit: usize, pub max_total_mb: u64,
      pub only: Vec<Category>, pub pdf_text: bool, pub no_poppler: bool }   // Default 구현
  pub fn parse(argv: &[String]) -> Result<Options, String>;
  pub fn print_usage(out: &mut dyn Write) -> io::Result<()>;
  pub fn run(argv: &[String], out: &mut dyn Write) -> i32;
  // scan.rs
  pub fn dispatch_format(path: &Path, sniffed: Sniffed) -> Option<Format>;
  pub fn scan_file(path: &Path, opts: &Options) -> FileReport;
  // 포맷 모듈 진입점(이 task에서는 빈 본문, 이후 task가 채운다)
  // epub::scan_epub_file / pdf::scan_pdf_file / txt::scan_txt_file:
  //   pub fn …(path: &Path, opts: &Options, report: &mut FileReport)
  ```

  `dispatch_format` 규칙: 확장자가 `pdf`·`epub`이고 `sniffed`가 `Pdf`·`Zip`이면 **내용**을 따른다(`.pdf`인데 Zip이면 `Epub`). 그 밖에는 확장자(`pdf`→Pdf, `epub`→Epub, `txt`→Txt)를 따른다. 그 밖의 확장자는 `None`이다.
  CLI: `bookdoctor [-r] [--json] [--report PATH] [--workers N] [--limit N] [--max-total-mb N] [--only security,defect,render] [--pdf-text] [--no-poppler] PATH...`. 파일 수집은 확장자 `epub|pdf|txt`(대소문자 무시)만 `wanted`로 하고, 명시된 파일·없는 경로는 그대로 남긴다(`collect_files` 동작). `--max-total-mb` 기본 256, 0이면 `Err("Limits must be positive")`. `parse`에서 `PATH` 없음은 `Err("the following arguments are required: PATH...")`.

- [ ] **Step 1: 실패하는 테스트를 쓴다.** `common/mod.rs`는 `tests/rust/epubscan/common/mod.rs`를 복사하고 `use`만 고친다.
      | 테스트 | 입력 | 기대 |
      |---|---|---|
      | `no_args_prints_usage_and_exits_2` (`bin.rs`, 바이너리 실행) | 인자 없음 | stdout에 `bookdoctor`·`사용법`, 종료 코드 2 |
      | `parse_reads_every_option` | `-r --json --report r.json --workers 3 --limit 5 --max-total-mb 9 --only security,defect --pdf-text --no-poppler a` | 각 필드 일치 |
      | `parse_rejects_unknown_only_category` | `--only bogus x` | `Err` (메시지에 `bogus`) |
      | `dispatch_follows_content_for_pdf_that_is_a_zip` | `("a.pdf", Zip)` | `Some(Format::Epub)` |
      | `dispatch_follows_extension_otherwise` | `("a.txt", Unknown)` | `Some(Format::Txt)` |
      | `missing_path_is_incomplete_exit_2` | 없는 경로 | `run(...) == 2`, 출력에 `unreadable` |
      | `clean_empty_dir_exits_0` | 빈 디렉토리 `-r` | `0` |
      | `unreadable_subdir_is_reported_incomplete` (Review Focus 3) | `chmod 000` 하위 디렉토리 | 종료 코드 2 |
- [ ] **Step 2:** `cargo test -p bookdoctor` → FAIL.
- [ ] **Step 3:** `Cargo.toml`(edition·version은 workspace)에 의존성을 둔다: `scan_common`(features `serde`), `corpus_common`, `sysdeps`, `lopdf.workspace`, `zip`, `roxmltree`, `regex`, `fancy-regex`, `htmlkit`, `encoding_rs`, `chrono`, `serde`, `serde_json`, `rayon` — 버전은 `utils/epubscan/Cargo.toml`·`utils/pdfscan/Cargo.toml`과 동일하게 복사한다. `main.rs`는 `utils/epubscan/src/main.rs`처럼 panic hook을 끄고 `run`을 부른다. `scan_file`은 `common::inspect`(Task 5 전에는 없으므로 이 task에서는 `std::fs::metadata` 오류만 `mark_incomplete("unreadable: …")`) 후 디스패치한다. `scan_file` 끝에서 `opts.only`가 비어 있지 않으면 해당 분류만 남긴다.
- [ ] **Step 4:** `cargo test -p bookdoctor` → PASS.
- [ ] **Step 5:** 커밋: `feat: bookdoctor 뼈대와 CLI를 추가한다`.

---

### Task 4: 출력과 리포트 (`report.rs`)

**Files:** Create `utils/bookdoctor/src/report.rs`; Modify `cli.rs`(`run`이 `report`를 호출); Test `tests/rust/bookdoctor/report.rs`.

**Interfaces:**

- Produces:

  ```rust
  pub fn write_text(r: &FileReport, out: &mut dyn Write) -> io::Result<()>;   // 파일 1건당 블록
  pub fn write_json_line(r: &FileReport, out: &mut dyn Write) -> io::Result<()>; // NDJSON 1줄
  pub struct Aggregate { /* 발견·incomplete가 있는 FileReport만 보관 */ }
  impl Aggregate { pub fn new() -> Self; pub fn add(&mut self, r: &FileReport); }
  pub fn write_report_file(path: &Path, agg: &Aggregate, summary: &Summary, root: &str) -> io::Result<()>;
  ```

  리포트 파일 JSON: `{"scanned_at": "...", "root": "...", "summary": {"scanned": N, "by_severity": {...}, "by_category": {...}, "incomplete": N}, "files": [FileReport...]}`.
  발견이 없는 정상 파일은 텍스트·JSON 출력에서 건너뛴다(옛 도구와 같다). 마지막에 요약 한 줄(`검사 N건, 발견 M건, 검사하지 못함 K건`)을 텍스트 모드에서만 낸다.

- [ ] **Step 1: 실패하는 테스트를 쓴다.**
      | 테스트 | 기대 |
      |---|---|
      | `json_line_has_category_severity_code_and_viewer` | 한 줄 JSON에 `"category":"security"`, `"severity":"HIGH"`, `"viewer_blocked":"n/a"` |
      | `file_name_with_control_chars_never_breaks_json_or_text` (Review Focus 4) | 파일명 `a\u{1b}[31m\n.txt`: 출력에 ESC·개행 없음, `serde_json::from_str`로 파싱됨 |
      | `non_utf8_file_name_is_lossy_not_a_panic` | `OsStr::from_bytes(b"\xff.txt")` 입력도 출력됨 |
      | `report_file_contains_only_flagged_files_and_summary` | 정상 1건·발견 1건 → `files.len()==1`, `summary.scanned==2` |
      | `clean_file_prints_nothing` | 발견 없는 `FileReport` → 출력 빈 문자열 |
- [ ] **Step 2:** FAIL 확인. **Step 3:** 구현(JSON 이스케이프는 `serde_json`을 쓰고 `pdfscan`의 `json_str` 같은 수제 이스케이프를 옮기지 않는다). **Step 4:** PASS 확인.
- [ ] **Step 5:** 커밋: `feat: bookdoctor 텍스트·JSON·리포트 출력을 추가한다`.

---

### Task 5: 공통 검사 (§6.1)

**Files:** Create `utils/bookdoctor/src/common.rs`; Modify `scan.rs`; Test `tests/rust/bookdoctor/common_checks.rs`.

**Interfaces:**

- Consumes: Task 2 `sniff`, `names::windows_reserved`; Task 3 `Options`.
- Produces: `pub fn inspect(path: &Path, opts: &Options, report: &mut FileReport) -> Option<Vec<u8>>`. 앞 64KiB를 돌려주고, 이후 검사를 계속할 수 없으면 `None`이다. 코드와 심각도:
  | 코드 | 분류 | 심각도 | 조건 |
  |---|---|---|---|
  | `EMPTY_FILE` | defect | Medium | 길이 0 (`None` 반환) |
  | (incomplete) | — | — | 읽기 실패 `unreadable: …`, 크기 > `max_total_mb` → `skipped: N bytes exceeds --max-total-mb` (`None` 반환) |
  | `SPARSE_FILE` | defect | High | 길이 ≥ 1MiB 이고 `blocks()*512*100 < len*99` (unix) |
  | `ZERO_TAIL` | defect | High | 길이 ≥ 4096 이고 마지막 4096바이트가 전부 0 |
  | `FORMAT_MISMATCH` | defect | Medium | 확장자 `pdf`인데 `sniff`≠Pdf이고 ≠Zip, 확장자 `epub`인데 ≠Zip, 확장자 `txt`인데 ∈{Pdf,Zip,Elf,MachO,Pe}. `.pdf`+Zip(EPUB)도 보고한다 |
  | `DISGUISED_EXECUTABLE` | security | Critical | `sniff` ∈ {Elf,MachO,Pe} 이고 확장자가 `epub`·`pdf`·`txt` |
  | `FILENAME_CONTROL_CHAR` | defect | Medium | 파일명에 제어문자 |
  | `FILENAME_TOO_LONG` | defect | Low | 이름 구성요소 255바이트 초과 |
  | `FILENAME_RESERVED_NAME` | defect | Low | `windows_reserved(stem)` |
  `scan_file`은 `inspect`가 `None`이면 포맷 검사를 건너뛴다. `sniff`로 `dispatch_format`을 부르고 `report.format`을 채운다.

- [ ] **Step 1: 실패하는 테스트를 쓴다.** 각 코드마다 `flags_<code_lowercase>` 테스트를 둔다. 예:
  ```rust
  #[test] fn flags_format_mismatch_for_pdf_that_is_really_an_epub() {   // Review Focus 2
      let f = fixtures::epub_named("book.pdf");               // 정상 EPUB 내용, 확장자만 pdf
      let r = scan_file(f.path(), &Options::default());
      assert_eq!(r.format, Some(Format::Epub));
      assert_eq!(r.findings.iter().filter(|x| x.code == "FORMAT_MISMATCH").count(), 1);
  }
  ```
  `sparse`는 `File::set_len(2 MiB)`만 한 파일, `zero_tail`은 `vec![1u8; 8192]` 뒤 4096 바이트를 0으로 덮은 파일, `unreadable`은 `chmod 000`(root에서는 건너뛰도록 `#[cfg(unix)]`와 uid 검사), `skipped_over_limit`은 `max_total_mb=1`과 2MiB 파일로 `incomplete`와 `reasons[0]`이 `skipped:`로 시작하는지 확인한다. `clean_text_file_has_no_common_findings`는 UTF-8 한 줄 `.txt`가 아무것도 안 잡히는지 확인한다.
- [ ] **Step 2:** `cargo test -p bookdoctor --test common_checks` → FAIL. **Step 3:** 구현. **Step 4:** PASS.
- [ ] **Step 5:** 커밋: `feat: bookdoctor 공통 파일 검사를 추가한다`.

---

### Task 6: TXT 검사 (§6.4)

**Files:** Create `utils/bookdoctor/src/txt.rs`; Test `tests/rust/bookdoctor/txt.rs`.

**Interfaces:**

- Produces: `pub fn scan_txt_file(path: &Path, opts: &Options, report: &mut FileReport)`, `pub const LOADER_TEXT_SIZE: usize`(값은 `libs/doc_reader`의 `TEXT_SIZE`에서 읽어 복사하고 출처를 주석으로 단다. `doc_reader`는 의존하지 않는다).
  | 코드 | 분류 | 심각도 | 조건 |
  |---|---|---|---|
  | `NOT_UTF8` | defect | High | UTF-8로 읽히지 않음. `detail`은 `encoding=utf-16le\|utf-16be\|cp949\|unknown` (BOM이 있으면 UTF-16, 아니면 `encoding_rs::EUC_KR`로 오류 없이 디코딩되면 `cp949`) |
  | `INVALID_UTF8` | defect | High | `Utf8Error::valid_up_to()`를 `detail`에 `offset=N`으로 |
  | `REPLACEMENT_CHAR` | defect | Medium | **유효한** UTF-8 안에 U+FFFD |
  | `BINARY_CONTENT` | defect | High | UTF-16 BOM이 없는 파일에 NUL, 또는 제어문자(탭·개행·FF 제외) 비율 > 1% |
  | `BIDI_CONTROL` | security | Medium | U+202A–U+202E, U+2066–U+2069 |
  | `ZERO_WIDTH` | security | Medium | U+200B–U+200D, U+2060, 그리고 오프셋 0이 아닌 U+FEFF |
  | `HTML_IN_TEXT` | security | Medium | `(?i)<\s*(script\|iframe\|object\|embed\|html)\b` 또는 `javascript:` |
  | `TEXT_TOO_LARGE` | render | Medium | 길이 > `LOADER_TEXT_SIZE` |
  | `MIXED_NEWLINES` | render | Low | `\r\n`과 단독 `\n`(또는 단독 `\r`)이 함께 있음 |
  | `LONG_LINE` | render | Low | 한 줄이 `MAX_LINE_CHARS`(= 100_000, 프론트가 줄 단위 `<p>`로 그리므로 근거는 `ViewTXT.jsx`) 초과 |
  | `EMPTY_TEXT` | defect | Medium | 0바이트가 아닌 파일이 공백뿐 |
  UTF-8 BOM은 어느 코드에도 걸리지 않는다. 본문(`from_utf8_lossy`)은 Task 13의 `mojibake::check_text`에 넘긴다(그 task가 호출을 추가한다).

- [ ] **Step 1: 실패하는 테스트를 쓴다.**
  ```rust
  #[test] fn utf8_bom_is_clean() {                                        // Review Focus 1
      let f = txt(b"\xEF\xBB\xBF\xED\x95\x9C\xEA\xB8\x80\n");
      assert!(scan(f.path()).findings.is_empty());
  }
  #[test] fn utf16_bom_is_not_utf8_and_does_not_panic() {                 // Review Focus 1
      let f = txt(b"\xFF\xFEh\x00i\x00");
      let r = scan(f.path());
      let d = r.findings.iter().find(|x| x.code == "NOT_UTF8").unwrap();
      assert_eq!((d.severity, d.detail.as_str()), (Severity::High, "encoding=utf-16le"));
  }
  #[test] fn cp949_text_is_not_utf8() { /* encoding_rs EUC_KR.encode("한글 소설입니다")로 만든 바이트 → detail "encoding=cp949" */ }
  ```
  나머지는 코드마다 `flags_<code_lowercase>`(양성)과 `does_not_flag_<code_lowercase>_on_clean_text`(음성) 한 쌍이다. `INVALID_UTF8`은 `b"ok\xff"`로 `detail == "offset=2"`를 확인한다. `ZERO_WIDTH`는 맨 앞 U+FEFF만 있으면 안 잡히고 중간에 있으면 잡히는 쌍으로 쓴다.
- [ ] **Step 2–4:** FAIL → 구현 → `cargo test -p bookdoctor --test txt` PASS.
- [ ] **Step 5:** 커밋: `feat: bookdoctor TXT 검사를 추가한다`.

---

### Task 7: EPUB 보안 검사 이식

**Files:**

- Create: `utils/bookdoctor/src/epub/{mod,security}.rs`, `tests/rust/bookdoctor/epub_security.rs`, `epub_gaps.rs`
- Modify: `scan.rs`(EPUB 연결)

**Interfaces:**

- Consumes: Task 2 `FileReport`.
- Produces: `epub::security::scan(path: &Path, limits: &Limits, report: &mut FileReport)`. `Limits`는 옛 `Limits`(기본 `max_member` 20MiB, `max_total` 200MiB, `max_files` 10_000)를 그대로 쓰고 `--max-total-mb`가 `max_total`을 덮는다. 옛 `Finding.code`는 **한 글자도 바꾸지 않고** 그대로 `FileReport.findings[].code`가 된다. 분류는 모두 `Security`다. 단 옛 `MISSING_MIMETYPE`·`NONSTANDARD_MIMETYPE`·`MISSING_CONTAINER_XML`·`BAD_MIMETYPE`는 구조 문제이므로 `Render`로 보고하고 Task 8의 구조 코드와 중복되면 한 번만 낸다(구조 쪽이 우선).

- [ ] **Step 1:** 옛 파일을 복사한다.
  ```bash
  cp utils/epubscan/src/security.rs utils/bookdoctor/src/epub/security.rs
  cp tests/rust/epubscan/security.rs tests/rust/bookdoctor/epub_security.rs
  cp tests/rust/epubscan/gaps.rs     tests/rust/bookdoctor/epub_gaps.rs
  ```
  테스트의 `use epubscan::security::{…}`를 `use bookdoctor::epub::security::{…}`로 바꾸고, 옛 `SecurityReport`를 읽던 단언을 `FileReport`로 바꾼다(`report.findings`의 `code`·`severity`는 그대로, `status == Incomplete`는 `incomplete`로). `Cargo.toml`에 `[[test]]` 두 개를 추가한다.
- [ ] **Step 2:** `cargo test -p bookdoctor --test epub_security --test epub_gaps` → FAIL(컴파일 오류).
- [ ] **Step 3:** `security.rs`의 `SecurityReport`, `Status`, `Finding`, `impl Report` 정의와 `scan_epub`·`guarded`를 `FileReport`를 채우는 형태로 바꾼다. 검사 규칙 본문(`Scanner`, `scan_html`, `scan_opf`, `check_member`, `check_magic` 등)은 **수정하지 않는다**. `check_magic`의 ELF·Mach-O 판정은 `scan_common::sniff`를 쓰도록 바꿔 중복을 없앤다. 조각 하나라도 규칙이 바뀌면 이식 테스트가 잡는다.
- [ ] **Step 4:** 같은 명령 → PASS. 옛 도구의 테스트 건수와 이식된 건수를 비교해 `old-tests.txt` 옆에 적는다(이식하지 못한 테스트는 이유를 적는다).
- [ ] **Step 5:** 커밋: `feat: bookdoctor에 EPUB 보안 검사를 이식한다`.

---

### Task 8: EPUB 구조 검사 이식

**Files:** Create `utils/bookdoctor/src/epub/structure.rs`, `tests/rust/bookdoctor/epub_structure.rs`; Modify `epub/mod.rs`.

**Interfaces:**

- Produces: `epub::structure::inspect(path: &Path, report: &mut FileReport)`. 옛 9개 이슈 코드를 `FileReport` 코드로 바꾸는 표를 `pub const ISSUE_TO_CODE: [(&str, &str, Category, Severity); 9]`로 둔다. Task 16이 이 표를 쓴다.
  | 옛 이슈 | 새 코드 | 분류 | 심각도 |
  |---|---|---|---|
  | `not_zip` | `NOT_ZIP` | defect | High |
  | `bad_mimetype` | `BAD_MIMETYPE` | render | Low |
  | `no_container` | `NO_CONTAINER` | render | High |
  | `opf_missing` | `OPF_MISSING` | render | High |
  | `member_unreadable` | `MEMBER_UNREADABLE` | defect | High |
  | `scan_error` | (incomplete, `reasons`에 `panic: …`) | — | — |
  | `opf_unparsable` | `OPF_UNPARSABLE` | render | High |
  | `spine_member_missing` | `SPINE_MEMBER_MISSING` | render | High |
  | `prefixed_root` | `PREFIXED_ROOT` | render | Low |
  `PREFIXED_ROOT`는 `ViewEPUB.jsx`의 `stripElementPrefixes`가 접두사를 벗기므로 `Low`다(설계 §11). 구현 중 `backend/src/book_manager/epub.rs`의 미리보기도 같은 처리를 하는지 읽고, 하지 않으면 `Medium`으로 올리고 이유를 주석으로 남긴다.
  `inspect`는 옛 `inspect_epub_structure_with`처럼 `catch_unwind`로 panic을 격리한다.

- [ ] **Step 1:** `cp tests/rust/epubscan/structure.rs tests/rust/bookdoctor/epub_structure.rs`로 옛 테스트를 복사하고 `EpubResult.issues` 단언을 `report.has("<새 코드>")`로 바꾼다. 추가 테스트 `parser_panic_becomes_incomplete_not_abort`(Review Focus 5)는 옛 `inspect_epub_structure_with`의 `root_tag` 주입 방식을 그대로 쓴다: 패닉하는 클로저를 넣고 `report.incomplete && !report.reasons.is_empty()`를 확인한다.
- [ ] **Step 2–4:** FAIL → `utils/epubscan/src/structure.rs`에서 `check_epub_members`, `read_member`, `read_head`, `spine_members`, `parse_xml`, `root_tag_name`을 복사해 `inspect`에 연결(규칙 본문은 수정하지 않는다. `IndexMap`·`EpubReport`·`scan_epub_structure`는 옮기지 않는다) → `cargo test -p bookdoctor --test epub_structure` PASS.
- [ ] **Step 5:** 커밋: `feat: bookdoctor에 EPUB 구조 검사를 이식한다`.

---

### Task 9: EPUB 신규 검사 (프론트 규칙, 손상, 렌더링)

**Files:** Create `utils/bookdoctor/src/epub/render.rs`, `tests/rust/bookdoctor/epub_render.rs`.

**Interfaces:**

- Consumes: Task 8의 OPF 파싱 헬퍼(`parse_xml`, `spine_members`), Task 3의 `Options`.
- Produces: `epub::render::inspect(path: &Path, report: &mut FileReport)`. 규칙의 근거는 `frontend/src/EpubDiagnose.js`(각 함수)와 `ViewEPUB.jsx`다. 코드:
  | 코드 | 분류 | 심각도 | 조건 (근거) |
  |---|---|---|---|
  | `UNDECLARED_PREFIX` | render | Medium | OPF에 쓰였지만 `xmlns:`로 선언되지 않은 접두사 (`findUndeclaredPrefixes`) |
  | `MANIFEST_EMPTY`, `SPINE_EMPTY` | render | High | manifest/spine 항목 0개 |
  | `SPINE_IDREF_MISSING` | render | High | spine `idref`가 manifest에 없음 |
  | `NAV_MISSING` | render | Low | EPUB3(`version` 3.x)인데 `properties="nav"` 항목 없음 |
  | `NCX_REF_MISSING` | render | Medium | `toc` 속성이 가리키는 NCX 항목·파일 없음, 또는 navPoint가 없는 파일 참조 |
  | `GUIDE_REF_MISSING` | render | Low | guide `href`의 파일이 ZIP에 없음 |
  | `XHTML_PARSE_FAILED` | render | High | spine 마크업 문서의 XML 파싱 실패 |
  | `HEAD_MISSING` | render | High | spine XHTML에 `head` 요소 없음 (`ViewEPUB.jsx`: "EPUB 본문에 head가 없습니다") |
  | `DC_METADATA_MISSING` | render | Low | `dc:title`·`dc:language`·`dc:identifier` 중 없는 것(`detail`에 이름) |
  | `ZIP_CRC_MISMATCH` | defect | High | 멤버를 끝까지 읽으면 CRC 오류 (한도: 멤버당 `Limits.max_member`, 넘으면 건너뛰고 `reasons`에 기록) |
  | `FONT_TOO_LARGE` | render | Low | 폰트 멤버 > 500KB (값은 `backend/src/book_manager/`에서 "500KB"로 검색해 확인하고 상수에 출처 주석) |
  | `OBFUSCATED_TEXT` | render | Medium | 한 XHTML에 인라인/CSS `font-size:0%`·`0px` 요소 ≥ 20개 또는 U+202E ≥ 20개 (임계값은 `scripts/epub_deobfuscate.py`의 난독화 패턴이 근거. 실제 난독화 EPUB으로 확인하고 임계값을 조정하면 주석으로 남긴다) |
- [ ] **Step 1: 실패하는 테스트를 쓴다.** fixture는 `common/mod.rs`의 EPUB 빌더로 만든다.
  ```rust
  #[test] fn flags_head_missing_for_body_without_head() {
      let f = fixtures::epub(&[("OEBPS/c1.xhtml", b"<html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>x</p></body></html>")]);
      let r = scan(f.path());
      let h = r.findings.iter().find(|x| x.code == "HEAD_MISSING").unwrap();
      assert_eq!((h.category, h.severity), (Category::Render, Severity::High));
  }
  ```
  나머지 코드는 `flags_<code_lowercase>`와 `clean_epub_has_no_<code_lowercase>` 쌍이다. `ZIP_CRC_MISMATCH`는 정상 EPUB의 멤버 데이터 한 바이트를 바꾼 바이트열(`zip` crate로 stored 방식으로 쓰고 데이터 구간을 직접 수정)로 만든다.
- [ ] **Step 2–4:** FAIL → 구현 → `cargo test -p bookdoctor --test epub_render` PASS.
- [ ] **Step 5:** 커밋: `feat: bookdoctor에 EPUB 프론트 규칙과 손상 검사를 추가한다`.

---

### Task 10: PDF 보안 검사 이식

**Files:** Create `utils/bookdoctor/src/pdf/{mod,security}.rs`, `tests/rust/bookdoctor/pdf_security.rs`; Modify `scan.rs`.

**Interfaces:**

- Produces: `pdf::security::scan(data: &[u8], report: &mut FileReport)`와 `pdf::scan_pdf_file(path, opts, report)`. 옛 `Finding.kind`는 **그대로** `code`가 된다(`"Launch action"`, `"JavaScript action"`, `"exploit JavaScript"` 등 공백 포함 문자열). 새 형식에 맞춰 대문자 스네이크로 바꾸지 않는다. 동등성 비교(Task 16)와 `ViewerBlocked` 표가 옛 문자열을 키로 쓰기 때문이다. `location`은 옛 `path`, `detail`은 옛 `detail`이고 옛 `object`(번호, 세대)는 `location` 앞에 `obj N G:`로 붙인다. 분류는 모두 `Security`다.

- [ ] **Step 1:** 옛 코드와 테스트를 복사한다.
  - 코드: `utils/pdfscan/src/lib.rs`에서 `shrink*`, `Scanner`, `scan_document`, `name_readings`, `js_*`, `launch_*`, `filespec_is_dangerous`와 그 상수(`RISKY_EXT`, `STREAM_KEEP_BYTES`, `ARRAY_KEEP_LEN` 등)를 `pdf/security.rs`로 옮긴다. `parse`, `print_*`, `run`, `Args`, `USAGE`는 옮기지 않는다.
  - 테스트: `tests/rust/pdfscan/pdfscan.rs`를 `tests/rust/bookdoctor/pdf_security.rs`로 복사하고 `pdfscan::run(...)` 단언을 `scan_file(...).findings`의 `code`·`severity` 단언으로 바꾼다. `lib.rs`의 `#[cfg(test)]` 단위 테스트(`shrink_*` 등 약 10개)는 `pdf/security.rs` 안에 그대로 둔다.
- [ ] **Step 2:** FAIL(컴파일). **Step 3:** `scan_file`을 `FileReport`를 채우는 형태로 바꾼다. `Document::load_mem_with_options`와 `catch_unwind`는 옛 `scan_file`을 그대로 옮긴다. 파싱 실패는 `notes`가 아니라 Task 12의 `XREF_BROKEN` 발견과 `mark_incomplete("parse error: …")`로 쓴다(이 task에서는 `mark_incomplete`만 한다). 헤더 없음도 지금은 `mark_incomplete`만 한다.
- [ ] **Step 4:** `cargo test -p bookdoctor --test pdf_security` PASS와 `cargo test -p bookdoctor --lib` PASS. 잘린 PDF가 panic 없이 `incomplete`가 되는 테스트 `truncated_pdf_is_incomplete_not_abort`(Review Focus 5)를 추가한다.
- [ ] **Step 5:** 커밋: `feat: bookdoctor에 PDF 보안 검사를 이식한다`.

---

### Task 11: PDF 보안 신규 검사

**Files:** Modify `pdf/security.rs`; Test `tests/rust/bookdoctor/pdf_security_new.rs`.

**Interfaces:** Consumes Task 10의 `Scanner::dict`/`walk`. Produces 아래 코드(분류 `Security`). PDF는 `lopdf::dictionary!`와 `Document::with_version("1.5")`로 직접 만든다(옛 `pdfscan.rs` 테스트의 헬퍼를 재사용한다).
| 코드 | 심각도 | 조건 |
|---|---|---|
| `GoToR action` | Medium | `/S /GoToR` (`detail`에 `/F` 대상) |
| `GoToE action` | Medium | `/S /GoToE` |
| `SubmitForm action` | Medium | `/S /SubmitForm` (`detail`에 `/F`) |
| `ImportData action` | Medium | `/S /ImportData` |
| `hex-obfuscated action name` | Medium | `/JS`·`/JavaScript`·`/Launch` 키 이름이 `#xx` 이스케이프를 써서 쓰임 |
| `long filter chain` | Medium | 한 스트림의 `/Filter` 배열 길이 ≥ 5 (`STREAM_KEEP_BYTES` 상수 옆 주석에 근거) |
| `many incremental updates` | Low | 파일 안 `startxref` 개수 ≥ 10 |
| `embedded executable content` | High | 임베디드 파일 스트림(해제 후 앞 1024바이트)의 `sniff`가 Elf·MachO·Pe |

- [ ] **Step 1:** 코드마다 `flags_<kind_slug>` 테스트를 쓴다. 예: `/S /GoToR /F (other.pdf)`인 OpenAction → `("GoToR action", Severity::Medium)`이고 `detail`에 `other.pdf`가 있다. 각각 음성 쌍(`GoTo`는 안 잡힘)을 둔다.
- [ ] **Step 2–4:** FAIL → 구현 → PASS(`cargo test -p bookdoctor --test pdf_security_new`).
- [ ] **Step 5:** 커밋: `feat: bookdoctor에 PDF 외부 참조·은닉 검사를 추가한다`.

---

### Task 12: PDF 구조·렌더링 검사 (lopdf, Poppler)

**Files:** Create `utils/bookdoctor/src/pdf/structure.rs`; Modify `pdf/mod.rs`; Test `tests/rust/bookdoctor/pdf_structure.rs`.

**Interfaces:**

- Consumes: `libs/sysdeps::{SystemRunner, CommandRunner, CommandError, poppler::{page_count, extract_text}}`.
- Produces: `pdf::structure::inspect(data: &[u8], doc: Option<&Document>, parse_error: Option<&str>, path: &Path, opts: &Options, report: &mut FileReport)`. 코드:
  | 코드 | 분류 | 심각도 | 조건 |
  |---|---|---|---|
  | `NO_PDF_HEADER` | defect | High | 앞 1024바이트에 `%PDF-` 없음 (Task 10의 `mark_incomplete`를 이것으로 바꾼다) |
  | `NO_EOF_MARKER` | defect | Medium | 끝 1024바이트에 `%%EOF` 없음 |
  | `XREF_BROKEN` | defect | High | `parse_error`가 있음. 동시에 `mark_incomplete("parse error: …")` |
  | `NO_ROOT` | defect | High | trailer에 `/Root` 없음 |
  | `ZERO_PAGES` | defect | High | 페이지 수 0 |
  | `ENCRYPTED` | render | High | trailer에 `/Encrypt` (pdf.js가 비밀번호 오류로 처리한다) |
  | `TINY_PAGE` | render | High | 어떤 페이지든 MediaBox 너비 또는 높이 < 1.0pt (0.09pt에서 뷰어가 백지가 된 사례) |
  | `POPPLER_OPEN_FAILED` | render | High | `!opts.no_poppler`이고 `page_count`가 `Err`(설치되지 않음은 제외) |
  | `FONT_NOT_EMBEDDED` | defect | Low | FontDescriptor에 `FontFile`·`FontFile2`·`FontFile3` 없음. Base-14 이름은 제외 |
  | `NO_TEXT_LAYER` | defect | Low | `opts.pdf_text`이고 앞 5쪽 `extract_text`가 공백뿐 |
  Poppler 실행 파일이 없으면(`CommandError::NotFound`) 발견이 아니라 `mark_incomplete("poppler not installed")`다.
  쪽수 상한 검사는 만들지 않는다. `ViewPDF.jsx`·`pdfRenderScale.js`·`backend/src/book_manager/preview.rs`에서 쪽수 상한을 찾지 못했으므로(캔버스 픽셀 상한은 스케일 조정이라 결함이 아니다) 구현 중에 다시 검색해 상한을 찾으면 `PAGE_LIMIT_EXCEEDED`(render, Medium)로 추가하고 출처를 주석으로 남긴다.
- [ ] **Step 1: 실패하는 테스트를 쓴다.** `CommandRunner`를 구현한 `FakeRunner`(`run`이 미리 정한 `CommandOutput` 또는 `CommandError`를 돌려줌)를 `common/mod.rs`에 둔다. `inspect`가 러너를 받도록 하기 위해 `Options`에 `runner: Option<Arc<dyn CommandRunner>>`를 두지 않고 `inspect_with(…, runner: &dyn CommandRunner)`를 `pub`으로 분리한다(`inspect`는 `SystemRunner`로 위임).
  ```rust
  #[test] fn flags_tiny_page_for_a_page_smaller_than_one_point() {
      let f = fixtures::pdf_with_mediabox([0.0, 0.0, 0.09, 0.09]);
      let r = scan_with_runner(f.path(), &FakeRunner::pages(1));
      assert!(r.findings.iter().any(|x| x.code == "TINY_PAGE" && x.severity == Severity::High));
  }
  ```
  나머지는 코드마다 양성·음성 쌍이다. `poppler_not_installed_is_incomplete_not_a_finding`은 `FakeRunner::not_found()`로 `incomplete`이고 `POPPLER_OPEN_FAILED`가 없는지 확인한다. 헤더 없는 파일은 `.pdf` 확장자일 때 `NO_PDF_HEADER`와 `FORMAT_MISMATCH`가 함께 나오는지 확인한다.
- [ ] **Step 2–4:** FAIL → 구현 → `cargo test -p bookdoctor --test pdf_structure` PASS. Task 10의 `incomplete` 처리 단언이 바뀌므로 `pdf_security.rs`의 해당 단언(헤더 없음)을 함께 고친다.
- [ ] **Step 5:** 커밋: `feat: bookdoctor에 PDF 구조·렌더링 검사를 추가한다`.

---

### Task 13: 인코딩 손상 판정 이식

**Files:** Create `utils/bookdoctor/src/mojibake.rs`; Modify `txt.rs`, `epub/mod.rs`, `pdf/mod.rs`; Test `tests/rust/bookdoctor/mojibake.rs`.

**Interfaces:**

- Consumes: `corpus_common::reader::{read_document_text, korean_likeness, is_hangul_syllable, TEXT_HEAD_CHARS, TEXT_TAIL_CHARS}`.
- Produces:
  ```rust
  pub const MOJIBAKE_LIKENESS_THRESHOLD: f64 = 0.10;   // utils/detect_mojibake/src/lib.rs 값
  pub const MIN_HANGUL_RATIO: f64 = 0.10;
  pub fn classify_text(text: &str) -> (Verdict, Option<f64>);   // Verdict { Corrupted, Clean, Undetermined, Unreadable }
  pub fn check_text(report: &mut FileReport, text: &str);       // Corrupted면 push + report.likeness = Some(round4)
  pub fn check_file(path: &Path, report: &mut FileReport);      // read_document_text(path, 20000, 10000)
  ```
  `Corrupted`이면 `push(Defect, High, "MOJIBAKE", "", "likeness=0.0031")`. `Unreadable`·`Undetermined`은 발견이 아니다. TXT·EPUB는 기본으로 `check_file`을 부른다. 본문이 비어 있으면(추출 실패) `mark_incomplete("no text extracted")`다. PDF는 `opts.pdf_text`일 때만 `poppler::extract_text`로 앞 5쪽과 끝 5쪽을 모아 `check_text`에 넘긴다.
- [ ] **Step 1:** `tests/rust/detect_mojibake/detect.rs`에서 `classify_text` 관련 테스트(임계값 경계, 한글 비율 미달, 한문 원전 `Undetermined`, 200자 미만 `Unreadable`)를 복사하고 `Verdict` 단언으로 바꾼다. 새 테스트: `txt_with_cp949_misread_as_utf8_is_flagged_mojibake`(`癤`로 시작하는 샘플, `likeness < 0.10`), `pdf_text_is_not_extracted_without_the_flag`(`FakeRunner`가 호출되지 않았는지 확인).
- [ ] **Step 2–4:** FAIL → `utils/detect_mojibake/src/lib.rs:57-89`의 `classify_text`를 `Verdict`를 돌려주는 형태로 옮겨 구현 → `cargo test -p bookdoctor --test mojibake` PASS.
- [ ] **Step 5:** 커밋: `feat: bookdoctor에 인코딩 손상 판정을 이식한다`.

---

### Task 14: `viewer_blocked` 표

**Files:** Create `utils/bookdoctor/src/viewer.rs`; Modify `scan.rs`(`scan_file` 끝에서 호출); Test `tests/rust/bookdoctor/viewer.rs`.

**Interfaces:**

- Produces: `pub fn viewer_blocked(format: Format, code: &str) -> ViewerBlocked`, `pub fn annotate(report: &mut FileReport)`.
  | 대상 | 값 | 근거(주석으로 파일명을 적는다) |
  |---|---|---|
  | EPUB: `SCRIPT_TAG`, `EVENT_HANDLER_ATTR`, `JS_URL`, `REMOTE_SCRIPT`, `SCRIPT_FILE`, `SCRIPT_MEDIA_TYPE`, `ACTIVE_ELEMENT`, `FORM_ELEMENT`, `REMOTE_FORM_ACTION`, `META_REFRESH` | `Blocked` | `ViewEPUB.jsx`의 DOMPurify `FORBID_TAGS`와 CSP `script-src 'none'` |
  | EPUB: `SVG_FOREIGN_OBJECT`, `XINCLUDE`, `XXE_ENTITY`, `ENTITY_DECLARATION`, `EXTERNAL_DTD`, `REMOTE_RESOURCE`, `BASE_HREF` | `NotBlocked` | 같은 파일에서 막는 항목이 아님 |
  | EPUB: `HEAD_MISSING` | `NotBlocked` | 뷰어가 렌더링을 거부한다(위험이 아니라 실패) |
  | PDF: 모든 `security` 코드 | `NotBlocked` | `ViewPDF.jsx`는 pdf.js 기본 동작이며 액션 차단 코드를 두지 않는다. 구현 중 `enableScripting`·`isEvalSupported` 설정을 `grep`으로 확인하고 다르면 표를 고친다 |
  | TXT: 모든 `security` 코드 | `Blocked` | `ViewTXT.jsx`는 텍스트를 `<p>` 자식으로만 렌더한다(React가 이스케이프) |
  | 그 밖의 코드(`defect`, `render`) | `NotApplicable` | |
- [ ] **Step 1:** 테스트 `epub_script_is_blocked`, `pdf_javascript_is_not_blocked`, `txt_html_is_blocked`, `defect_is_not_applicable`, `every_epub_security_code_in_the_table_exists_in_security_rs`(표의 EPUB 코드를 `include_str!("../../utils/bookdoctor/src/epub/security.rs")`에서 `"CODE"` 문자열로 찾아 존재 확인 — 표가 낡는 것을 막는다)를 쓴다.
- [ ] **Step 2–4:** FAIL → 구현 → PASS. **Step 5:** 커밋: `feat: bookdoctor 리포트에 뷰어 차단 여부를 표시한다`.

---

### Task 15: `quarantine_broken` 이전

**Files:**

- Modify: `utils/quarantine_broken/{Cargo.toml,src/lib.rs,src/cli.rs}`, `tests/rust/quarantine_broken/quarantine.rs`

**Interfaces:**

- Consumes: Task 4의 리포트 파일(`files[]`의 `path`, `findings[].code`, `likeness`).
- Produces: `load_corrupted(report_path: &Path) -> io::Result<Vec<Rec>>`는 시그니처를 유지하고 새 형식을 읽는다. `files[]` 중 `findings`에 `code == "MOJIBAKE"`가 있는 항목만 골라 `Rec`(`path`, `likeness`, `verdict: "corrupted"`)로 바꾼다. `DEFAULT_LIBRARY_ROOT`, `thousands`는 `corpus_common`에서 직접 가져온다. `VERDICT_CORRUPTED`는 이 crate의 `pub const VERDICT_CORRUPTED: &str = "corrupted"`로 둔다. 도움말의 "detect_mojibake의 JSON"은 "bookdoctor `--report`의 JSON"으로, 기본 리포트 경로는 `tmp/bookdoctor_report.json`으로 바꾼다. `fix-ext`도 같은 `load_corrupted`를 쓴다.
- [ ] **Step 1: 실패하는 테스트를 쓴다.** `tests/rust/quarantine_broken/quarantine.rs`의 리포트 fixture를 새 형식으로 바꾼다. 새 테스트: `only_mojibake_findings_are_quarantined`(`NOT_UTF8`·`FORMAT_MISMATCH`만 가진 파일은 `plan_moves` 결과에 없음), `rejects_old_detect_mojibake_report_shape`(`{"corrupted": […]}`는 `InvalidData` 오류).
- [ ] **Step 2–4:** FAIL → `Cargo.toml`에서 `detect_mojibake`를 `corpus_common = { path = "../../libs/corpus_common" }`로 바꿔 구현 → `cargo test -p quarantine_broken` PASS.
- [ ] **Step 5:** 커밋: `refactor: quarantine_broken이 bookdoctor 리포트를 읽도록 한다`.

---

### Task 16: 동등성·재현율 검증 (삭제 전 게이트)

**Files:** 코드 변경 없음. 결과는 `docs/rust-migration-progress.md` 끝에 "bookdoctor 동등성 검증" 절로 추가한다.

- [ ] **Step 1: 전체 회귀.** `cargo test --workspace --locked`와 `cargo clippy --workspace --all-targets` 통과를 확인한다. Expected: 실패 0. 실패하면 이 task를 멈추고 원인 task로 돌아간다.
- [ ] **Step 2: EPUB 구조 동등성.** Task 1의 `epub-structure-flagged.txt` 각 파일에 `bookdoctor --json` 을 돌려, 옛 리포트가 기록한 이슈를 `ISSUE_TO_CODE`로 바꾼 코드가 모두 있는지 비교한다. Expected: 누락 0건(누락은 원인을 적고 Task 8로 돌아간다).
- [ ] **Step 3: EPUB·PDF 보안 동등성.** `sample-epub.txt`에 `epubscan scan --json`과 `bookdoctor --json --only security`를, `sample-pdf.txt`에 `pdfscan --json`과 `bookdoctor --json --only security`를 돌려 `(code, location)` 집합을 비교한다. Expected: 옛 집합 ⊆ 새 집합. 새 쪽에만 있는 항목은 건수와 표본 10건을 기록한다(허용, 격리에는 영향 없음).
- [ ] **Step 4: 인코딩 손상 동등성.** `sample-txt.txt`와 `broken-dir.txt`(비어 있지 않을 때)에 옛 `detect_mojibake`와 `bookdoctor`를 돌려 `corrupted` 판정 집합이 같은지 비교한다. `/mnt/data/text/0_broken`이 마운트되어 있지 않으면 "미실행"으로 적는다.
- [ ] **Step 5: 손상 재현율.** `pdf-redownload.txt`(26건)와 `epub-redownload.txt`(5건)의 모든 파일에서 `bookdoctor`가 `defect` 발견 또는 `incomplete`를 하나 이상 내는지 확인한다. Expected: 31건 모두. 놓친 파일은 매직바이트로 원인을 분류해 기록한다.
- [ ] **Step 6: 오탐 측정.** `sample-epub.txt`·`sample-pdf.txt`·`sample-txt.txt` 600건에서 `High` 이상 발견 비율을 분류별·코드별로 센다. `NOT_UTF8`은 따로 센다. 결과를 `docs/rust-migration-progress.md`에 적는다.
- [ ] **Step 7:** 결과 요약(비교 건수, 누락 0 여부, 새로 더 잡은 건수, 미실행 항목)을 사용자에게 보고하고 Task 17로 진행한다. 누락이 있으면 진행하지 않는다.
- [ ] **Step 8:** 커밋: `docs: bookdoctor 동등성 검증 결과를 기록한다`.

---

### Task 17: 옛 도구와 산출물 제거

사용자가 개발 완료 조건으로 지시했다(2026-10-29). Task 16의 Step 7 보고가 끝난 뒤에만 시작한다.

**Files:**

- Delete: `utils/pdfscan`, `utils/epubscan`, `utils/detect_mojibake`, `tests/rust/pdfscan`, `tests/rust/epubscan`, `tests/rust/detect_mojibake`
- Modify: 루트 `Cargo.toml`(members 세 항목 제거, `panic = "unwind"` 주석을 `bookdoctor가 catch_unwind로 파서 panic을 격리한다`로), `Cargo.lock`, `.github/workflows/backend-ci.yml`, `docs/rust-workspace.md`, `docs/rust-migration-progress.md`, `libs/scan_common/src/lib.rs`(머리 주석의 `epubscan`·`pdfscan` 언급), `libs/scan_common/Cargo.toml` description, `libs/corpus_common/src/lib.rs`(주석의 `detect_mojibake` 언급)

- [ ] **Step 1: 기준 수치 보존.** `tmp/epub_structure_report*.json`의 파일 수·flagged 합계·이슈별 건수를 `docs/rust-migration-progress.md`의 Task 16 절 아래에 표로 옮긴다. 이 숫자를 적기 전에는 삭제하지 않는다.
- [ ] **Step 2: 프로젝트 삭제.** `git rm -r utils/pdfscan utils/epubscan utils/detect_mojibake tests/rust/pdfscan tests/rust/epubscan tests/rust/detect_mojibake`.
- [ ] **Step 3: 참조 정리.**
  - 루트 `Cargo.toml` members에서 `"utils/pdfscan"`, `"utils/epubscan"`, `"utils/detect_mojibake"`를 제거한다.
  - `backend-ci.yml`에서 `'!utils/pdfscan/**'`, `'!utils/epubscan/**'`, `'!tests/rust/pdfscan/**'`, `'!tests/rust/epubscan/**'` 줄을 두 곳(pull_request, push) 모두 지우고, `cargo test --workspace --exclude pdfscan --exclude epubscan --locked`에서 `--exclude` 둘을 지운다. 이 제외는 "다른 세션이 맡는다"는 이유였고(`docs/rust-migration-progress.md:13`) 그 세션이 끝났다. `bookdoctor` 테스트는 임시 디렉토리만 쓰는지(`rg "/mnt/data" tests/rust/bookdoctor`가 0건) 확인한 뒤 CI에 포함한다.
  - `docs/rust-workspace.md` 도구 표의 세 줄을 `bookdoctor | epub·pdf·txt 보안 위험·파일 결함·렌더링 불가 구조 진단`(한 줄)으로 바꾼다.
- [ ] **Step 4: 산출물 삭제.**
  ```bash
  cargo clean -p pdfscan -p epubscan -p detect_mojibake   # 크레이트가 없어지기 전에 실행해야 한다. 이미 지웠다면 target/debug 아래 이름이 일치하는 파일만 find로 지운다.
  rm -f tmp/epub_structure_report*.json tmp/mojibake_recheck*.json
  ```
  `cargo clean -p`가 크레이트 삭제 후라 실패하면 `find target -maxdepth 4 \( -name 'pdfscan*' -o -name 'epubscan*' -o -name 'detect_mojibake*' \) -prune -exec rm -rf {} +`를 쓴다. `target/` 전체는 지우지 않는다(18GB).
- [ ] **Step 5: 검증.**
  - `cargo check --workspace`(Cargo.lock 갱신) 후 `git diff --stat Cargo.lock`이 `bookdoctor` 추가와 세 crate 제거만 보이는지 확인한다.
  - `rg -n "pdfscan|epubscan|detect_mojibake" . --glob '!target' --glob '!.git' --glob '!docs/rust-migration-progress.md' --glob '!docs/superpowers/**'` → Expected: 0건.
  - `find target tmp -name '*pdfscan*' -o -name '*epubscan*' -o -name '*detect_mojibake*' -o -name 'epub_structure_report*'` → Expected: 0건.
  - `cargo test --workspace --locked`, `cargo clippy --workspace --all-targets` → Expected: 통과.
  - `cargo llvm-cov -p bookdoctor --summary-only`의 라인 커버리지를 기록한다(목표는 100%에 가깝게. 미달 파일은 이유를 적는다).
- [ ] **Step 6:** 커밋: `chore: pdfscan, epubscan, detect_mojibake를 제거한다`(본문에 bookdoctor로 대체했음과 검증 요약).

---

## Self-Review

**Spec 커버리지.** §2 목표(포맷 3종 → Task 5·6·7–12, 세 분류 → Task 2, 보안 깊이 → Task 7·10·11, TXT → Task 6, mojibake → Task 13, 공통 모듈 → Task 2, 기존 검사 유지 → Task 7·8·10·13) 확인. §6.1 → Task 5, §6.2 → Task 7·8·9, §6.3 → Task 10·11·12, §6.4 → Task 6. §7 CLI·리포트 → Task 3·4, `viewer_blocked` → Task 14. §8 이전·제거 → Task 15·17. §9 검증 → Task 16(§9 문장은 사실에 맞게 이미 바로잡았다). §12 확정 8건: 1→Task 15, 2→Task 16, 3→Task 1, 4→Task 13, 5→Task 2·3, 6→Task 14, 7→사용자 확인 완료, 8→Task 9(한 번 이식, 동기화 테스트 없음). 빠진 요구 없음.

**알려진 차이.**

- 설계 §6.3의 "쪽수 상한 초과"는 코드에서 근거를 찾지 못해 Task 12에서 조건부로 처리했다(설계 §11이 임계값을 구현자 재량으로 두었다).
- PDF 보안 코드는 옛 `kind` 문자열(공백 포함)을 유지한다. 새 코드 이름 체계(대문자 스네이크)와 섞이지만 동등성 비교와 `viewer_blocked` 표 때문이다.

**타입 일관성.** `FileReport::push/mark_incomplete/has`는 Task 2에서 정의하고 Task 3–14가 같은 이름으로 쓴다. `Options` 필드(`pdf_text`, `no_poppler`, `max_total_mb`, `only`)는 Task 3이 정의하고 Task 5·12·13이 쓴다. `ISSUE_TO_CODE`는 Task 8이 정의하고 Task 16이 쓴다. `inspect_with(runner)`는 Task 12에서만 정의한다.

**비례.** 계획은 설계의 약 2배 길이다. 코드 본문은 시그니처·테스트 이름·단언·표로 제한했고, 옮기는 규칙 본문은 복사 명령으로 대신했다.
