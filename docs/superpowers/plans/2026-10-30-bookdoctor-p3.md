# bookdoctor P3 구현 계획 (HWP 5.x)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans (사용자가 inline 실행을 골랐고 subagent 독립 실행을 금지했다). Steps use checkbox (`- [ ]`) syntax.

**Goal:** `.hwp`(HWP 5.x, OLE 복합 파일)를 검사한다: 암호화·DRM 표시, 스크립트, `BinData`의 실행 파일·OLE·EPS, 연결 파일, 깨진 레코드. P2의 `ole_reader`를 그대로 쓴다.

**Architecture:** `utils/bookdoctor/src/hwp/{mod,header,script,inflate,bindata,records}.rs`. `mod.rs`가 `OleFile`을 열고 각 검사를 부른다. `FileHeader`가 모든 검사의 입력(압축 여부)이다. 스트림을 읽는 모든 곳은 바이트 상한 안에서 한다. HWP 3.x(OLE가 아님, 서명 `HWP Document File V3.00`)는 서명만 확인한다.

**Tech Stack:** `ole_reader`, `scan_common`. 압축 해제에 `flate2`가 필요하다(Task 4). `flate2`는 이미 `Cargo.lock`에 있고 `libs/hwp3_parser`·`libs/doc_reader`가 쓰지만 `bookdoctor`의 의존성에는 새로 넣는 것이라 승인이 필요하다.

**Spec:** `docs/superpowers/specs/2026-10-30-bookdoctor-malware-scanner-design.md`. 포맷 사실은 `docs/bookdoctor-hwp5-notes.md`(한컴 공식 명세 revision 1.3로 확인).

## Global Constraints

- 새 외부 의존성은 승인 전에 넣지 않는다. Task 1~3은 의존성 없이 끝낸다.
- 모든 읽기는 상한이 있다(스트림 64MiB, 압축 해제 출력 상한, 레코드 수·스크립트 길이 상한). 파서는 panic하지 않는다.
- 해석하지 못했거나 상한에 걸린 부분은 `incomplete`로 남긴다. 깨끗하다고 하지 않는다.
- 출력 규칙은 P0~P2 그대로다(기본 `--min-severity high`, 암호화·DRM cyan).
- 파일을 실행하거나 렌더링하지 않는다. 스크립트는 텍스트로만 본다.
- cargo는 `env -u TM_MYSQL_HOST -u TM_ES_URL`, `CARGO_TARGET_DIR=/home/terzeron/workspace/tm_dev/target`. 커밋은 Conventional Commits, 본문 `-` bullet. push·머지 없음.
- subagent를 쓰지 않는다. 최종 리뷰는 자기 리뷰로 한다.

## Review Focus

1. 정상 HWP를 비정상으로 보지 않는다(배포용·압축 문서의 `FileHeader`, 빈 `Scripts`). 건강한 HWP5 실파일이 코퍼스에 없어 합성 fixture와 명세로만 확인한다는 한계를 보고한다. (Task 2, 3)
2. 암호·DRM·인증서 암호화 문서를 깨끗하다고 하지 않는다. cyan `incomplete`로 낸다. (Task 2)
3. 압축 해제 폭탄(작은 입력, 거대한 출력)이 메모리·시간을 터뜨리지 않는다. 상한에 걸리면 미완료다. (Task 4)
4. `BinData` 안의 실행 파일은 이름이 아니라 해제한 내용의 매직으로 잡는다. (Task 5)
5. 깨진 레코드 헤더(길이가 스트림을 넘음, 확장 길이 누락, 레벨 폭주)가 무한 루프·panic을 만들지 않는다. (Task 6)

---

### Task 1: HWP 식별·수집·위장

**Files:** `libs/scan_common/src/sniff.rs`(`Sniffed::Hwp3`), `libs/scan_common/src/finding.rs`(`Format::Hwp`), `utils/bookdoctor/src/{cli,common,scan,viewer}.rs`, `utils/bookdoctor/src/zipcheck/mod.rs`(`.hwpx` 이름의 OLE를 HWP 분석기로), 신규 `utils/bookdoctor/src/hwp/mod.rs`, 테스트 `tests/rust/bookdoctor/hwp.rs`(신규, `[[test]]`).

**Interfaces:** Produces `hwp::scan_hwp_file(path, opts, sniffed, report)`, `hwp::scan_ole_named_hwpx(path, opts, report)`.

- `Sniffed::Hwp3`: 앞부분이 `HWP Document File V3.00`. `.hwp` 수집, `Format::Hwp`(JSON `"hwp"`).
- `.hwp` 이름: 내용이 OLE면 HWP5 분석, Hwp3면 서명만 확인(미완료 아님, 한계는 갭 분석에 적는다), 그 밖이면 `FORMAT_MISMATCH`와 `DISGUISED_EXECUTABLE`(실행 파일). 내용이 ZIP이면 HWPX 패키지로 검사한다(이름만 바꾼 `.hwpx`).
- `.hwpx` 이름의 OLE: P1의 일반 미완료 대신 HWP 분석기로 넘긴다(`FileHeader`가 없으면 일반 미완료).

- [ ] **Step 1: 실패하는 테스트.** `hwp_is_collected_and_lookalikes_are_not`(`a.hwp` 수집, `b.hwpp` 제외), `hwp3_signature_is_recognized_and_not_incomplete`, `hwp_with_other_content_is_a_mismatch`(텍스트, PE는 `DISGUISED_EXECUTABLE`), `a_zip_named_hwp_is_scanned_as_hwpx`, `hwpx_named_ole_with_a_fileheader_is_analyzed_as_hwp`.
- [ ] **Step 2: 실행해 실패를 확인한다.**
- [ ] **Step 3: 구현.**
- [ ] **Step 4: 코퍼스 `.hwp` 200건 표본을 돌려 HWP3 198건에 지표·미완료가 없고 OLE 2건이 구조 이상으로 나오는지 확인한다.**
- [ ] **Step 5: 테스트·clippy 통과 후 커밋.** `feat: bookdoctor가 .hwp를 식별하고 이름 위장을 알린다`

### Task 2: `FileHeader`

**Files:** 신규 `utils/bookdoctor/src/hwp/header.rs`, 테스트 `tests/rust/bookdoctor/hwp.rs`.

**Interfaces:** Produces `header::parse(bytes: &[u8]) -> Result<FileHeader, HeaderError>`, `FileHeader { version: u32, flags: u32, flags2: u32, encrypt_version: u32 }` 메서드 `compressed()`·`encrypted()`·`distributable()`·`has_scripts()`·`drm()`·`cert_encrypted()`.

- 256바이트 미만이거나 서명 `HWP Document File`이 아니면 `HWP_BAD_HEADER`(Defect Medium).
- bit1(암호), bit4(DRM), bit8(공인 인증서 암호화), bit10(공인 인증서 DRM) 중 하나면 `incomplete`: `encrypted: HWP password (FileHeader.encrypted)` 등. 본문 검사는 건너뛴다(스크립트·`BinData` 구조 검사는 암호화가 걸리지 않는 부분만 한다).
- bit2(배포용)는 알리지 않는다. 단 `BodyText`가 없고 `ViewText`가 있으면 미완료 `distribution document: ViewText not analyzed`.
- OLE 구조 이상은 DOC와 같은 `OLE_*` 코드로 보고한다(`doc::report_anomalies`를 공용으로 쓴다).

- [ ] **Step 1: 실패하는 테스트.** 합성 `FileHeader`(256바이트)를 `make_cfb`로 감싼다. `plain_header_is_clean`, `password_drm_and_cert_flags_are_incomplete_and_encrypted_reason`(비트마다), `bad_signature_and_short_header_are_defects`, `distribution_doc_without_bodytext_is_incomplete`, `ole_without_fileheader_is_a_mismatch_defect`.
- [ ] **Step 2: 실행해 실패를 확인한다.** **Step 3: 구현.** **Step 4: 테스트·clippy 통과 후 커밋.** `feat: bookdoctor가 HWP FileHeader의 암호·DRM 표시를 읽는다`

### Task 3: `Scripts`

**Files:** 신규 `utils/bookdoctor/src/hwp/script.rs`, 테스트 `tests/rust/bookdoctor/hwp.rs`.

**Interfaces:** Produces `script::inspect(file: &mut OleFile, header: &FileHeader, report)`.

- `Scripts` 스토리지가 있거나 `FileHeader` bit3이면 `HWP_SCRIPT`(Security High). 비트와 스토리지가 어긋나면 `HWP_SCRIPT_FLAG_MISMATCH`(Defect Medium).
- `Scripts/DefaultJScript`를 표 9 구조(DWORD 길이 + WCHAR 4개 + 끝 `-1`)로 읽는다. 길이는 스트림을 넘지 않아야 하고 합계 상한 4MiB. 구조가 맞지 않으면 원시 UTF-16으로 키워드를 찾고 `incomplete`.
- 소스에 `ActiveXObject`, `WScript`, `Shell`, `eval`(`new Function` 포함), `XMLHttp`, `ADODB.Stream`, `powershell`, `cmd.exe`, `URLDownloadToFile`이 있으면 `HWP_SCRIPT_SUSPICIOUS`(Critical). 대소문자 무시.
- 압축 여부를 가리지 않는다: 표 구조로 읽히지 않으면 `FileHeader`가 압축이고 `inflate`가 있을 때(Task 4 이후) 풀어서 다시 읽는다. Task 4 전에는 미완료로 둔다.

- [ ] **Step 1: 실패하는 테스트.** `no_scripts_storage_is_clean`, `scripts_storage_is_high_even_when_empty`, `flag_without_storage_and_storage_without_flag_are_mismatches`, `suspicious_script_is_critical`, `benign_script_is_high_not_critical`, `malformed_script_lengths_are_incomplete_and_never_panic`, `huge_declared_lengths_do_not_allocate`.
- [ ] **Step 2: 실행해 실패를 확인한다.** **Step 3: 구현.** **Step 4: 테스트·clippy 통과 후 커밋.** `feat: bookdoctor가 HWP 스크립트를 탐지한다`

### Task 4: 압축 해제 (승인 필요)

**Files:** `utils/bookdoctor/Cargo.toml`(`flate2 = "1"`), 신규 `utils/bookdoctor/src/hwp/inflate.rs`, 테스트 `tests/rust/bookdoctor/hwp.rs`.

**Interfaces:** Produces `inflate::decompress(data: &[u8], limit: usize) -> Result<Vec<u8>, InflateError>`(`TooLarge`, `Invalid`). zlib 헤더가 유효하면 zlib, 아니면 raw deflate로 읽고, 첫 시도가 실패하면 다른 쪽으로 한 번 더 시도한다. 출력이 `limit`을 넘으면 `TooLarge`다.

- [ ] **Step 0: 사용자 승인.** `bookdoctor`에 `flate2` 의존성을 추가하는 것을 승인받는다(`Cargo.lock`은 `bookdoctor`의 의존성 한 줄만 늘어야 한다).
- [ ] **Step 1: 실패하는 테스트.** `zlib_and_raw_deflate_both_decode`, `output_over_the_limit_is_too_large`(1KiB 입력, 1GiB 출력 폭탄), `truncated_and_garbage_are_invalid`, `empty_input_is_an_empty_output`. fixture는 `zip` crate의 deflate로 만든다.
- [ ] **Step 2~4:** 실패 확인, 구현, 통과·clippy·커밋. `feat: bookdoctor가 HWP 압축 스트림을 상한 안에서 푼다`
- [ ] **Step 5:** Task 3의 압축된 스크립트 경로를 연다(테스트 `compressed_script_is_read`).

### Task 5: `BinData`

**Files:** 신규 `utils/bookdoctor/src/hwp/bindata.rs`, 테스트 `tests/rust/bookdoctor/hwp.rs`.

**Interfaces:** Produces `bindata::inspect(file, header, report)`.

- `DocInfo`(압축이면 풀어서)의 `HWPTAG_BIN_DATA`(BEGIN+2=`0x12`) 레코드를 읽어 Type을 얻는다: LINK(0)이면 경로를 `HWP_LINKED_FILE`(Medium, UNC `\\`나 URL이면 High), STORAGE(2)는 OLE.
- `BinData/*` 스트림마다 앞부분(압축이면 푼 앞 4KiB)의 매직을 본다: PE·ELF·Mach-O면 `EMBEDDED_EXECUTABLE`(High), OLE 복합 파일이면 `HWP_OLE_OBJECT`(High), `%!PS-Adobe` 또는 DOS EPS `C5D0D3C6`이면 `HWP_EPS_OBJECT`(Medium, 규칙은 노트의 EPS 항목 참조). 확장자 `exe`·`dll`·`scr` 등 `ZIP_EXEC_EXTS`이면 이름만으로 High.
- 푸는 데 실패하거나 상한에 걸리면 `incomplete`.

- [ ] **Step 1: 실패하는 테스트.** `pe_in_a_compressed_bindata_is_flagged_by_content`, `pe_with_a_png_name_is_still_flagged`, `ole_and_eps_payloads`, `linked_file_unc_is_high_and_local_is_medium`, `decompression_bomb_in_bindata_is_incomplete`, `plain_png_bindata_is_clean`.
- [ ] **Step 2~4:** 실패 확인, 구현, 통과·clippy·커밋. `feat: bookdoctor가 HWP BinData의 실행 파일·OLE·EPS를 탐지한다`

### Task 6: 레코드 검증

**Files:** 신규 `utils/bookdoctor/src/hwp/records.rs`, 테스트 `tests/rust/bookdoctor/hwp.rs`.

**Interfaces:** Produces `records::validate(stream: &[u8], name: &str, report)`와 `records::walk(stream) -> impl Iterator<Item = Record>`(`Record { tag: u16, level: u16, data: &[u8] }`, 깨지면 멈춘다).

- 헤더 32비트(TagID 10, Level 10, Size 12), Size가 `0xFFF`면 다음 DWORD가 길이. 길이가 남은 바이트를 넘으면 `HWP_BAD_RECORD`(Defect High, 위치는 스트림 이름), 확장 길이 DWORD가 잘렸어도 같다. 레벨이 직전보다 2 이상 깊어지면 Medium.
- `DocInfo`와 `BodyText/Section*`(압축이면 푼 뒤, 합계 상한 64MiB)를 훑는다. `HWPTAG_SHAPE_COMPONENT_OLE`(BEGIN+68)가 있으면 개수를 센다(`HWP_OLE_OBJECT` 위치 정보에만 쓴다).
- 한 스트림의 레코드 수 상한 5,000,000. 넘으면 `incomplete`.

- [ ] **Step 1: 실패하는 테스트.** `valid_records_are_clean`, `size_past_the_end_is_a_high_defect`, `truncated_extended_length_is_a_high_defect`, `level_jump_is_medium`, `record_count_limit_is_incomplete`, `a_megabyte_of_zero_headers_is_linear`.
- [ ] **Step 2~4:** 실패 확인, 구현, 통과·clippy·커밋. `feat: bookdoctor가 HWP 레코드 구조를 검증한다`

### Task 7: 문서·완료 검증

- [ ] 갭 분석(`docs/bookdoctor-gap-analysis.md`)의 HWP 5.x 행을 갱신하고, 정상 HWP5 실파일로 검증하지 못한 한계와 HWP3은 서명만 본다는 점을 적는다.
- [ ] `cargo test --workspace --locked --no-fail-fast`(한 번), `cargo clippy --workspace --all-targets --locked`, `Cargo.lock` diff 확인, 표본 TXT·PDF·EPUB이 기준과 같은지 확인.
- [ ] 자기 리뷰(Review Focus 5개)와 변조 실험(합성 HWP 바이트 변조 수백 회, panic·지연 0).
