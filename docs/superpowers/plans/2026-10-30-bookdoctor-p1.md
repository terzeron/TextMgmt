# bookdoctor P1 구현 계획 (ZIP · DOCX · HWPX)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans (사용자가 inline 실행을 골랐다) 또는 superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `bookdoctor`가 `.zip`, `.docx`(`.docm`·`.dotx`·`.dotm`), `.hwpx`를 검사한다. ZIP은 압축 해제 가능 여부를 빠르게 확인하고, DOCX·HWPX는 압축 파일로 보고 구조·원격 참조·내장 실행 코드를 본다.

**Architecture:** ZIP 이름 규칙(Zip Slip 등)은 EPUB `Scanner::zip_name`에 묶여 있다. 이를 순수 함수 `zipnames::check`로 뽑아 EPUB과 새 포맷이 함께 쓴다(EPUB 결과는 기존 테스트 전부가 지킨다). 새 모듈은 `utils/bookdoctor/src/zip/`(범용 ZIP 점검 `fast.rs`, 입구 `mod.rs`)와 `ooxml.rs`, `hwpx.rs`다. `libs/zipcheck`로의 승격은 P4로 미룬다.

**Tech Stack:** 기존 `zip` 9.0.5(deflate), `quick-xml` 대신 이미 EPUB이 쓰는 XML 파서와 `regex`. 새 의존성은 없다.

**Spec:** `docs/superpowers/specs/2026-10-30-bookdoctor-malware-scanner-design.md`

## Global Constraints

- 새 의존성을 추가하지 않는다. 필요하면 멈추고 승인을 받는다.
- 주석·문서는 한국어, 식별자는 영어. 테스트는 `tests/rust/<crate>/`에 두고 `[[test]]`로 등록한다.
- cargo 명령은 `env -u TM_MYSQL_HOST -u TM_ES_URL`로, `CARGO_TARGET_DIR=/home/terzeron/workspace/tm_dev/target`을 쓴다. 전체 workspace 테스트는 동시에 두 번 돌리지 않는다(`tests/books` fixture 충돌).
- 각 태스크 끝에 `cargo test -p scan_common -p bookdoctor -p quarantine_broken --locked --features scan_common/serde`와 `cargo clippy -p scan_common -p bookdoctor -p quarantine_broken --all-targets --locked`가 경고 0이어야 한다.
- 압축 해제는 예산 안에서만 한다: 멤버당 `Limits::max_member`, 파일당 `max_total`, 파일 수 `max_files`. 선언 크기를 믿지 않고 읽기는 상한 +1바이트에서 멈춘다(EPUB과 같은 규칙, `read_limited` 재사용).
- 파일을 실행·렌더링하지 않는다. 임베디드 실행 파일은 내용을 풀지 않고 이름·매직바이트만 본다.
- EPUB 동작은 바뀌지 않는다. EPUB 테스트(`epub_*`)와 표본 EPUB 200건 출력이 변경 전과 같아야 한다.
- 커밋: Conventional Commits 제목, 본문 `-` bullet, 명령형, 72자 이내. 브랜치는 `feature/bookdoctor`, push·머지 없음.
- 출력 규칙은 P0 그대로다: 기본 `--min-severity high`, 구조·렌더링 에러 red, HIGH purple, CRITICAL red, 암호화 cyan.

## Review Focus

1. 일반 하이퍼링크(`TargetMode="External"`인 `/hyperlink`)는 소음이 되면 안 된다. 원격 템플릿·OLE·외부 링크 파일만 올린다. (Task 4)
2. 선언 크기가 거짓으로 작은 ZIP 멤버, 중앙 디렉터리 항목 100만 개, 중첩 ZIP이 메모리·시간을 터뜨리지 않는다. (Task 2)
3. `.docx`인데 내용이 ZIP이 아니거나 OLE(옛 `.doc`)·PDF·실행 파일이면 위장을 잡는다. (Task 3)
4. `.docx`에 `vbaProject.bin`이 있으면 CRITICAL, `.docm`에 있으면 HIGH다. 확장자가 매크로 허용 형식인지에 따라 갈린다. (Task 4)
5. 암호화된 DOCX(OLE 컨테이너의 `EncryptedPackage`)와 ZIP 암호화 멤버는 "안전"이 아니라 `incomplete`(cyan)다. (Task 3, 4)
6. 파일명에 ESC·방향 문자가 든 멤버 이름이 출력에 그대로 나오지 않는다. (Task 2)

---

### Task 1: 포맷 확장과 ZIP 이름 규칙 추출

**Files:**

- Create: `utils/bookdoctor/src/zipnames.rs`
- Modify: `libs/scan_common/src/finding.rs`(`Format::Zip`, `Docx`, `Hwpx`), `utils/bookdoctor/src/lib.rs`, `utils/bookdoctor/src/epub/security.rs`(`zip_name`이 `zipnames::check`를 부른다), `utils/bookdoctor/src/scan.rs`, `utils/bookdoctor/src/cli.rs`(`wanted`), `utils/bookdoctor/src/viewer.rs`
- Test: `tests/rust/bookdoctor/zipnames.rs`(신규, `[[test]]` 등록), `tests/rust/bookdoctor/scan.rs`, `tests/rust/bookdoctor/viewer.rs`, 기존 `epub_security*.rs`

**Interfaces:**

- Produces:
  - `Format::Zip | Docx | Hwpx`(JSON 소문자).
  - `zipnames::NameFinding { severity: Severity, code: &'static str, detail: &'static str }`, `zipnames::check(name: &str) -> Vec<NameFinding>`: 현재 `Scanner::zip_name`의 규칙(`ZIP_PATH_TRAVERSAL` CRITICAL, `ZIP_ABSOLUTE_PATH` HIGH, `ZIP_BACKSLASH_PATH`·`ZIP_CONTROL_CHAR_NAME`·`WINDOWS_RESERVED_NAME`·`ZIP_ALT_DATA_STREAM` MEDIUM, `ZIP_NAME_TOO_LONG` LOW)을 그대로 옮긴 순수 함수. 심각도·코드·문구는 바이트 단위로 같다.
  - `scan::dispatch_format`: 확장자 `zip`→`Zip`, `docx|docm|dotx|dotm`→`Docx`, `hwpx`→`Hwpx`. 확장자가 `docx` 계열·`hwpx`·`zip`일 때는 `.pdf`·`.epub`처럼 내용을 먼저 본다(`Sniffed::Pdf`면 `Pdf`).
  - `cli::wanted`: 위 확장자도 모은다. `viewer::viewer_blocked`: 세 포맷 모두 `NotApplicable`.
- Consumes: P0의 `Format::Executable` 분기.

- [ ] **Step 1: 실패하는 테스트.**
  - `zipnames::check` 동치: EPUB 보안 테스트가 쓰는 이름 목록(`../x`, `/abs`, `C:\\x`, `a\\b`, 제어 문자, `con.txt`, `a:b`, 긴 이름)마다 기존 `Scanner::zip_name`이 냈던 `(code, severity)` 집합과 같다. 기대값은 `epub_security.rs`의 기존 단언에서 옮긴다.
  - `dispatch_format`: `a.zip`+`Sniffed::Zip`→`Zip`, `a.docx`→`Docx`, `a.DOCM`→`Docx`, `a.hwpx`→`Hwpx`, `a.docx`+`Sniffed::Pdf`→`Pdf`, `a.epub`는 그대로 `Epub`.
  - `viewer_blocked(Format::Zip|Docx|Hwpx, "ANY") == NotApplicable`.
  - 디렉토리 수집: `-r`에서 `.zip`·`.docx`·`.hwpx`를 모으고 `.docs`는 모으지 않는다.
- [ ] **Step 2: 실행해 실패를 확인한다.**
- [ ] **Step 3: 구현.** `Scanner::zip_name`은 `zipnames::check` 결과를 `self.add`로 옮기는 얇은 함수로 바꾼다.
- [ ] **Step 4: 회귀.** `cargo test -p bookdoctor --locked --test epub_security --test epub_security_hardening --test epub_gaps --test epub_structure --test epub_render`가 전부 통과하고, 표본 EPUB 200건(`tmp/bookdoctor-baseline/sample-epub.txt`)의 `--min-severity low --json` 출력이 변경 전 바이너리와 (파일, code, location, severity) 집합까지 같다.
- [ ] **Step 5: 커밋.** `refactor: ZIP 이름 규칙을 공유 함수로 뽑고 새 포맷을 등록한다`

---

### Task 2: 범용 ZIP 빠른 점검

**Files:**

- Create: `utils/bookdoctor/src/zip/mod.rs`, `utils/bookdoctor/src/zip/fast.rs`
- Modify: `utils/bookdoctor/src/lib.rs`, `utils/bookdoctor/src/scan.rs`, `utils/bookdoctor/src/epub/security.rs`(`read_limited`·`directory_limit_note`를 `pub(crate)`로)
- Test: `tests/rust/bookdoctor/zip_fast.rs`(신규, `[[test]]` 등록), `tests/rust/bookdoctor/common/mod.rs`(zip fixture 도우미)

**Interfaces:**

- Produces:
  - `zip::scan_zip_file(path: &Path, opts: &Options, report: &mut FileReport)`.
  - `zip::fast::check(path, limits, report)`: 열고 점검하는 순서는 (1) 중앙 디렉터리 상한(`directory_limit_note`)이 넘으면 `incomplete`로 끝낸다, (2) `ZipArchive`로 연다. 못 열면 `NOT_ZIP`(Defect HIGH, 렌더링이 아니라 해제 불가), (3) 멤버마다 이름 규칙(`zipnames::check`)·로컬 헤더 일치(`BAD_LOCAL_HEADER`·`TRUNCATED_LOCAL_HEADER`·`ZIP_HEADER_NAME_MISMATCH`)·압축 방식 지원 여부, (4) **빠른 해제 시험**: 멤버마다 앞 `PROBE_BYTES = 4096`바이트만 풀어 본다. 실패하면 `ZIP_ENTRY_UNREADABLE`(Defect HIGH, 위치는 멤버 이름). 전체 CRC는 `--zip-deep`에서만 본다, (5) 압축비 상한 초과는 `HIGH_COMPRESSION_RATIO`(MEDIUM, EPUB과 같은 기준), (6) 멤버 이름의 확장자가 `EXEC_EXTS`면 `EMBEDDED_EXECUTABLE`(HIGH), 매직이 PE·ELF·Mach-O이면 같은 코드로 올린다, (7) 중첩 ZIP(`PK` 매직)은 `NESTED_ARCHIVE`(MEDIUM)로 표시만 한다(재귀는 P4), (8) 암호화 멤버는 `report.mark_incomplete("name: ZIP-encrypted entry cannot be reliably inspected")`.
  - `--zip-deep` 옵션(`Options.zip_deep: bool`): 멤버 전체를 예산 안에서 풀어 CRC까지 확인한다. 불일치는 `ZIP_CRC_MISMATCH`(Defect HIGH, 기존 EPUB 코드와 같은 이름).
  - 멤버 이름은 출력에 `safe()`를 거쳐서만 나간다.

- [ ] **Step 1: 실패하는 테스트.** fixture는 `zip` crate로 만든다.
  - `valid_zip_has_no_findings`.
  - `not_a_zip_is_reported`: 평문 `.zip` → `NOT_ZIP`.
  - `entry_with_corrupt_deflate_stream_is_reported_by_the_probe`: deflate 본문 중간 바이트를 깨뜨린 멤버 → `ZIP_ENTRY_UNREADABLE`, 위치에 멤버 이름. 같은 파일을 `--zip-deep` 없이도 잡는다(앞 4096바이트 안이 깨졌을 때).
  - `crc_mismatch_beyond_the_probe_is_only_found_with_zip_deep`: 8KiB 멤버의 마지막 바이트만 깨뜨림 → 기본은 지표 없음, `--zip-deep`이면 `ZIP_CRC_MISMATCH`.
  - `zip_slip_is_critical`: 멤버 이름 `../../evil` → `ZIP_PATH_TRAVERSAL` CRITICAL.
  - `forged_declared_size_does_not_blow_memory`: 로컬·중앙 헤더가 10바이트로 거짓 선언한 1GiB deflate 폭탄 → 읽기가 상한에서 멈추고 `HIGH_COMPRESSION_RATIO` 또는 `incomplete`, 프로세스 메모리가 한도(테스트는 결과만 확인)를 넘지 않는다.
  - `many_entries_over_the_directory_limit_is_incomplete_not_opened`: 중앙 디렉터리 항목 수가 상한을 넘도록 만든 파일 → `incomplete`.
  - `nested_zip_is_flagged_not_recursed`, `executable_member_is_flagged`(이름 `x.exe`와 이름은 `x.dat`인 PE 내용 둘 다), `encrypted_member_is_incomplete_and_cyan`(출력에 cyan 코드).
  - `member_name_with_escape_never_reaches_the_terminal_raw`.
  - `scan_file` 통합: `.zip`은 `Format::Zip`으로 이 검사를 탄다.
- [ ] **Step 2: 실행해 실패를 확인한다.**
- [ ] **Step 3: 구현.** 상수 `PROBE_BYTES`, 예산·상한은 EPUB `Limits`를 재사용한다.
- [ ] **Step 4: 성능 확인.** `tmp/` 아래 만든 10,000 멤버 ZIP(멤버당 1KiB)을 기본 점검할 때 1초 안에 끝난다(디버그 빌드 기준 측정값을 태스크 보고서에 적는다). 못 지키면 멈추고 보고한다.
- [ ] **Step 5: 도움말에 `--zip-deep`과 `.zip`을 추가하고 커밋.** `feat: bookdoctor가 ZIP을 빠르게 점검하고 해제 가능 여부를 알린다`

---

### Task 3: DOCX 구조와 위장

**Files:**

- Create: `utils/bookdoctor/src/zip/ooxml.rs`
- Modify: `utils/bookdoctor/src/zip/mod.rs`, `utils/bookdoctor/src/scan.rs`, `libs/scan_common/src/sniff.rs`(`Sniffed::Ole`), `utils/bookdoctor/src/common.rs`
- Test: `tests/rust/bookdoctor/ooxml_structure.rs`(신규, `[[test]]` 등록), `tests/rust/scan_common/sniff.rs`

**Interfaces:**

- Produces:
  - `Sniffed::Ole`: 앞 8바이트가 `D0 CF 11 E0 A1 B1 1A E1`. `sniff`에서 실행 파일 매직 다음, PDF 앞에 둔다.
  - `ooxml::inspect_structure(zip, ext, report)`: 필수 부분 확인 `[Content_Types].xml`, `_rels/.rels`, `word/document.xml`. 하나라도 없으면 `OOXML_NOT_DOCX`(Render, HIGH, 위치는 빠진 부분). XML 부분이 파싱되지 않으면 `MALFORMED_MARKUP`(EPUB과 같은 코드). `[Content_Types].xml`이 선언한 부분이 ZIP에 없으면 `OOXML_PART_MISSING`(Render, MEDIUM).
  - 위장·불일치: `.docx` 계열인데 `Sniffed::Ole`이면 `FORMAT_MISMATCH`(Defect MEDIUM)에 더해, OLE가 `EncryptedPackage` 스트림을 가진 암호화 OOXML일 수 있으므로 P2 전까지는 `incomplete`로 표시(`encrypted: OLE container (EncryptedPackage or legacy .doc) cannot be inspected before P2`, cyan). 평문·PDF·실행 파일이면 `FORMAT_MISMATCH`. 실행 파일이면 `DISGUISED_EXECUTABLE`(CRITICAL, P0의 확장자 위장 검사에 `docx` 계열 추가).
  - `common::check_content_vs_name`: `docx` 계열은 `Zip` 또는 `Ole`이 아니면 불일치, `Ole`이면 위 처리. `zip`은 `Zip`이 아니면 불일치.
  - `.docm`·`.dotm`은 매크로 허용 형식, `.docx`·`.dotx`는 매크로 없는 형식으로 `ooxml`에 `macro_enabled: bool`을 넘긴다(Task 4가 쓴다).

- [ ] **Step 1: 실패하는 테스트.** fixture는 최소 유효 DOCX를 `zip` crate로 만드는 도우미(`common::minimal_docx()`)로 쓴다.
  - `minimal_docx_has_no_findings`.
  - `missing_document_xml_is_reported`, `missing_content_types_is_reported`, `broken_xml_part_is_reported`.
  - `content_types_declaring_a_missing_part_is_reported`.
  - `docx_that_is_plain_text_or_pdf_is_format_mismatch`.
  - `docx_that_is_an_exe_is_disguised_executable_critical`.
  - `docx_that_is_ole_is_incomplete_and_cyan_and_format_mismatch`: OLE 매직 8바이트 + 패딩 파일.
  - `zip_extension_with_non_zip_content_is_not_zip`.
  - `sniff`: `Ole` 판별, OLE 매직 뒤에 `%PDF-`가 있어도 `Ole`, 7바이트뿐인 머리는 `Ole`이 아니다.
- [ ] **Step 2: 실행해 실패를 확인한다.**
- [ ] **Step 3: 구현.** `scan_zip_file`이 `Format::Docx`면 `fast::check` 뒤에 `ooxml::inspect_structure`를 부른다.
- [ ] **Step 4: 테스트·clippy 통과 후 커밋.** `feat: bookdoctor가 DOCX 구조와 확장자 위장을 검사한다`

---

### Task 4: DOCX 보안(원격 참조, 매크로, OLE, DDE)

**Files:**

- Modify: `utils/bookdoctor/src/zip/ooxml.rs`, `utils/bookdoctor/src/viewer.rs`(필요하면)
- Test: `tests/rust/bookdoctor/ooxml_security.rs`(신규, `[[test]]` 등록)

**Interfaces:**

- Produces (모두 Security 분류):
  - `OOXML_REMOTE_TEMPLATE`(CRITICAL): 관계 `Type`이 `.../attachedTemplate`이고 `TargetMode="External"`. MITRE T1221.
  - `OOXML_EXTERNAL_OBJECT`(HIGH): 관계 `Type`이 `.../oleObject`·`.../subDocument`·`.../frame`·`.../externalLinkPath`이고 `TargetMode="External"`.
  - `OOXML_UNC_TARGET`(HIGH): 외부 관계의 `Target`이 `\\\\host\\share` 또는 `file://`. 종류와 무관하게 낸다(NTLM 유출 경로).
  - `OOXML_EXTERNAL_IMAGE`(MEDIUM): 관계 `Type`이 `.../image`이고 외부. 추적 픽셀 후보라 기본 출력에서는 숨겨진다.
  - 일반 `.../hyperlink` 외부 링크는 지표를 내지 않는다.
  - `OOXML_VBA_PROJECT`: 멤버 `word/vbaProject.bin`이 있으면 `macro_enabled`가 아닐 때 CRITICAL(매크로 없는 형식으로 위장), 맞을 때 HIGH.
  - `OOXML_OLE_OBJECT`(HIGH): `word/embeddings/*.bin`(OLE 매직) 또는 관계 `Type`이 `.../oleObject`인 내장 객체. `OOXML_ACTIVEX`(HIGH): `word/activeX/` 아래 부분. `OOXML_EMBEDDED_PACKAGE`(MEDIUM): `word/embeddings/` 아래 `.docx`·`.xlsx`·`.pptx`(재귀는 P4).
  - `OOXML_DDE_FIELD`(CRITICAL): `word/document.xml`의 `w:instrText`(또는 `w:fldSimple`의 `w:instr`)가 `DDEAUTO`·`DDE`로 시작. `OOXML_DANGEROUS_FIELD`(HIGH): `INCLUDETEXT`·`INCLUDEPICTURE`·`LINK` 필드가 URL·UNC를 가리킬 때.
  - 모든 XML 부분은 DTD·엔티티 선언 검사를 받는다: 기존 EPUB 코드 `EXTERNAL_DTD`·`XXE_ENTITY`·`ENTITY_DECLARATION`·`XINCLUDE`를 같은 심각도로 낸다. XML 파서는 DTD를 해석하지 않는다.
  - 모든 멤버가 예산 안에서만 읽힌다. 읽다가 한도를 넘으면 `incomplete`.

- [ ] **Step 1: 실패하는 테스트.** `common::minimal_docx_with(parts: &[(&str, &[u8])])`로 부분을 더해 만든다.
  - `attached_template_external_is_critical`, `attached_template_internal_is_clean`.
  - `plain_hyperlink_external_is_clean` (소음 방지).
  - `external_ole_object_is_high`, `unc_target_is_high_for_any_relationship_type`, `external_image_is_medium`.
  - `vba_project_in_docx_is_critical`, `vba_project_in_docm_is_high`, `docm_without_vba_is_clean`.
  - `embedded_ole_bin_is_high`, `activex_part_is_high`, `embedded_docx_is_medium`.
  - `dde_field_is_critical`(`instrText`에 ` DDEAUTO c:\\\\windows\\\\system32\\\\cmd.exe ...`), `dde_in_fldsimple_is_critical`, `include_text_with_url_is_high`, `include_text_local_path_is_clean`.
  - `xxe_in_document_xml_is_reported_with_epub_codes`.
  - `field_instruction_split_across_runs_is_still_detected`: `DDEAUTO`가 두 `w:instrText` run에 나뉘어도 잡는다(공격자가 쓰는 우회).
  - `oversized_part_is_incomplete_not_read_fully`.
  - `severity_filter_default_keeps_critical_and_high_hides_medium`: CLI 기본 출력에서 `OOXML_EXTERNAL_IMAGE`는 보이지 않는다.
- [ ] **Step 2: 실행해 실패를 확인한다.**
- [ ] **Step 3: 구현.** 관계 `Type`은 접미사(`/attachedTemplate` 등)로 비교한다. 이름 공간 접두사는 믿지 않고 local name으로 읽는다.
- [ ] **Step 4: 소음 게이트.** 표본 DOCX가 코퍼스에 있는지 `find /mnt/data -iname '*.docx' | head`로 확인한다(HDD 전수 탐색이 아니라 `-maxdepth 3`, 100건 제한). 있으면 100건 이하를 `--min-severity low --json`으로 돌려 `OOXML_*` 지표 파일 수를 태스크 보고서에 적는다. 없으면 "실제 DOCX 표본 없음, 합성 fixture로만 검증"이라고 적는다.
- [ ] **Step 5: 테스트·clippy 통과 후 커밋.** `feat: bookdoctor가 DOCX의 원격 참조·매크로·DDE를 탐지한다`

---

### Task 5: HWPX

**Files:**

- Create: `utils/bookdoctor/src/zip/hwpx.rs`, `docs/bookdoctor-hwpx-notes.md`
- Modify: `utils/bookdoctor/src/zip/mod.rs`
- Test: `tests/rust/bookdoctor/hwpx.rs`(신규, `[[test]]` 등록)

**Interfaces:**

- Consumes: Task 2의 `fast::check`, Task 4의 XML 검사 함수.
- Produces:
  - 구조(`Render`): `mimetype`에 `hwp`·`owpml`이 없으면 `HWPX_BAD_MIMETYPE`(MEDIUM, 없는 것은 허용: 출처에서 일부 파일이 생략함), `Contents/content.hpf`가 없으면 `HWPX_NO_MANIFEST`(HIGH), manifest가 가리키는 `Contents/section*.xml`이 없으면 `HWPX_SECTION_MISSING`(HIGH), XML이 깨지면 `MALFORMED_MARKUP`.
  - 보안: `BinData/` 아래 멤버 중 확장자가 `EXEC_EXTS`이거나 매직이 PE·ELF·Mach-O면 `EMBEDDED_EXECUTABLE`(HIGH). OLE 매직(`BinData/*.ole`·`.bin` 등)이면 `HWPX_OLE_OBJECT`(HIGH). XML 부분의 DTD·엔티티는 Task 4와 같은 코드로 낸다. 외부 링크(하이퍼링크 필드)는 지표를 내지 않는다(DOCX와 같은 이유).
  - **스크립트 탐지는 이 태스크에서 구현하지 않는다.** `Scripts/`의 위치와 형식이 확인되지 않았다.

- [ ] **Step 1: 명세 확인(구현 전).** 한컴 OWPML 공식 명세(KS X 6101) 또는 한컴 개발자 문서를 찾아 `docs/bookdoctor-hwpx-notes.md`에 (a) 필수 부분 목록, (b) 스크립트·매크로·OLE가 저장되는 위치와 이름, (c) 근거 URL을 적는다. (b)를 공식 자료로 확인하지 못하면 "미확인"으로 적고 스크립트 탐지를 구현하지 않는다. 확인되면 이 계획에 Task 5b(스크립트 탐지)를 추가하고 ledger에 ruling으로 남긴다.
- [ ] **Step 2: 실패하는 테스트.** `common::minimal_hwpx_with(parts)` 도우미로 만든다.
  - `minimal_hwpx_has_no_findings`.
  - `missing_manifest_is_high`, `manifest_pointing_to_missing_section_is_high`, `bad_mimetype_is_medium_and_missing_mimetype_is_clean`.
  - `bindata_exe_by_name_and_by_magic_are_high`, `bindata_ole_is_high`, `plain_bindata_image_is_clean`.
  - `xxe_in_section_xml_is_reported`.
  - `hwpx_that_is_a_zip_without_hwpx_structure_is_reported`, `hwpx_extension_with_ole_content_is_incomplete_cyan_for_p3`: HWP 5.x(OLE)를 `.hwpx`로 이름만 바꾼 경우는 P3 전까지 `FORMAT_MISMATCH` + 미완료.
- [ ] **Step 3: 구현.**
- [ ] **Step 4: 테스트·clippy 통과 후 커밋.** `feat: bookdoctor가 HWPX 구조와 내장 실행 파일을 검사한다`

---

## P1 완료 검증 (실행자)

- [ ] `cargo test --workspace --locked --no-fail-fast`(동시 실행 없이 한 번), `cargo clippy --workspace --all-targets --locked`(bookdoctor·scan_common·corpus_common·quarantine_broken 경고 0).
- [ ] 표본 600건 기본 출력의 High 이상 집합이 변경 전과 같다(`scratchpad/p0/old-*.ndjson` 기준), 표본 EPUB 200건 `--min-severity low --json` 집합이 같다.
- [ ] 갭 분석 문서(`docs/bookdoctor-gap-analysis.md`)의 "새 포맷" 절을 갱신한다.
- [ ] 최종 리뷰어(opus)에게 Review Focus와 ledger의 `Ruling:`을 주고 검토받는다.
