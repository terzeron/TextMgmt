# bookdoctor P2 구현 계획 (OLE 파서 · DOC · 암호화 OOXML 판별)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans (사용자가 inline 실행을 골랐다). Steps use checkbox (`- [ ]`) syntax.

**Goal:** OLE 복합 파일(CFB)을 직접 읽는 관대한 파서(`libs/ole_reader`)를 만들고, 그 위에 구형 `.doc` 검사(VBA 매크로, OLE 객체, DDE 필드, 손상된 CFB, RTF 위장)와 암호화 OOXML 판별을 넣는다. P3(HWP 5.x)가 같은 파서를 쓴다.

**Architecture:** `libs/ole_reader`는 스트림 이름으로 읽는 API와, 파일이 어떻게 비정상인지를 목록(`Anomaly`)으로 돌려주는 API를 가진다. 정상 파서(`cfb` crate)는 손상 구조를 오류로 거부하므로 "손상 구조 탐지"에 쓸 수 없어 직접 만든다(설계서 결정). `utils/bookdoctor/src/doc/`는 파서 위의 분석기다. 압축 해제·읽기는 모두 바이트 상한 안에서 한다.

**Tech Stack:** 표준 라이브러리만 쓴다. 새 의존성은 없다. MS-OVBA 압축 해제는 직접 구현한다.

**Spec:** `docs/superpowers/specs/2026-10-30-bookdoctor-malware-scanner-design.md`

## 확인한 명세 (2026-10-30, Microsoft Open Specifications와 구현체)

| 항목                            | 값                                                                                                                                                                                                                                                                                                                                                                                                                                                             | 출처                                                                                                                                                                                                                                                                                                                                                     |
| ------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| CFB 헤더                        | 시그니처 `D0 CF 11 E0 A1 B1 1A E1`(0), CLSID(8, 16바이트), 부버전(24), 주버전(26: 3 또는 4), 바이트 순서(28: `0xFFFE`), 섹터 시프트(30: 주버전 3이면 9=512B, 4이면 12=4096B), 미니 섹터 시프트(32: 6=64B), 예약(34, 6바이트), 디렉터리 섹터 수(40, 주버전 3이면 0), FAT 섹터 수(44), 첫 디렉터리 섹터(48), 트랜잭션(52), 미니 스트림 한도(56: `0x1000`), 첫 미니 FAT 섹터(60), 미니 FAT 섹터 수(64), 첫 DIFAT 섹터(68), DIFAT 섹터 수(72), DIFAT 109개(76~511) | [MS-CFB 2.2](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-cfb/05060311-bfce-4b12-874d-71fd4ce63aea)                                                                                                                                                                                                                                  |
| 디렉터리 항목                   | 128바이트. 이름 UTF-16LE 64바이트(0), 이름 길이(64, 널 포함 바이트), 종류(66: 0 미사용, 1 저장소, 2 스트림, 5 루트), 색(67), 왼쪽(68)·오른쪽(72)·자식(76) ID, CLSID(80), 상태(96), 생성(100)·수정(108) 시각, 시작 섹터(116), 스트림 크기(120, 8바이트; 주버전 3은 상위 32비트를 무시하는 것을 권장). NOSTREAM=`0xFFFFFFFF`, MAXREGSID=`0xFFFFFFFA`                                                                                                             | [MS-CFB 2.6.1](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-cfb/60fe8611-66c3-496b-b70d-a504c94c9ace)                                                                                                                                                                                                                                |
| 특수 섹터 값                    | FREESECT `0xFFFFFFFF`, ENDOFCHAIN `0xFFFFFFFE`, FATSECT `0xFFFFFFFD`, DIFSECT `0xFFFFFFFC`, MAXREGSECT `0xFFFFFFFA`                                                                                                                                                                                                                                                                                                                                            | 헤더 명세와 같은 문서군. **이 값들은 이번에 페이지에서 직접 확인하지 못했고 기억에 기대었다**. Task 1 Step 1에서 공식 문서로 확인한다                                                                                                                                                                                                                    |
| FIB(`WordDocument` 스트림 머리) | `wIdent`(0, 2바이트), `nFib`(2), `flags1`(10, 2바이트): `fEncrypted`=`0x0100`, `fObfuscated`=`0x8000`(암호화일 때만 의미), `nFibBack`(12), `lKey`(14, 4바이트)                                                                                                                                                                                                                                                                                                 | [POI FibBase](https://apache.googlesource.com/poi/+/226ae7245895f34eca39a252e673349d5becfa5a/src/types/definitions/FibBase_type.xml), [MS-DOC](https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/3e9a4e92-bdfe-4379-8144-74d033892ede). `wIdent` 값(`0xA5EC`)은 확인하지 못했다. Word 문서 판정은 `WordDocument` 스트림 존재로 한다 |
| MS-OVBA 압축                    | 컨테이너 첫 바이트 `0x01`. 청크 머리 2바이트(LE): 하위 12비트+3 = 청크 크기, 비트 15 = 압축 여부. 압축 청크는 플래그 바이트(낮은 비트부터) 뒤에 리터럴 1바이트 또는 복사 토큰 2바이트가 온다. 복사 토큰은 비트 수 `BitCount`(= 청크 안 현재 해제 위치로 정해지는 4~12)로 오프셋·길이를 가른다. 비압축 청크는 4096바이트 그대로다                                                                                                                               | [MS-OVBA 2.4.1](https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-ovba/4742b896-b32b-4eb0-8372-fbf01e3c65fd), 구현체 설명                                                                                                                                                                                                               |

## Global Constraints

- 새 의존성을 추가하지 않는다. 필요하면 멈추고 승인을 받는다.
- 모든 읽기는 상한이 있다: 스트림 하나 `max_stream`(기본 64MiB), 파일 전체 `max_total`, 섹터 체인 길이는 파일 크기를 넘을 수 없고, 디렉터리 항목 수는 `max_entries`(기본 100,000)를 넘지 않는다. 순환 체인은 방문 표시로 끊는다.
- 파서는 panic하지 않는다. 인덱스·길이는 모두 검사하고, 비정상은 `Anomaly`로 남긴 뒤 읽을 수 있는 만큼 읽는다.
- 파일을 실행하지 않는다. VBA는 텍스트로만 읽는다.
- 출력 규칙은 P0·P1 그대로다: 기본 `--min-severity high`, 구조·렌더링 에러 red, HIGH purple, CRITICAL red, 암호화 cyan.
- cargo 명령은 `env -u TM_MYSQL_HOST -u TM_ES_URL`, `CARGO_TARGET_DIR=/home/terzeron/workspace/tm_dev/target`. 전체 workspace 테스트는 동시에 두 번 돌리지 않는다. 커밋은 Conventional Commits, 본문 `-` bullet. push·머지 없음.
- 새 crate는 루트 `Cargo.toml`의 workspace 멤버에 넣는다. `Cargo.lock`은 이 crate 한 줄만 늘어야 한다(외부 crate 추가 없음).

## Review Focus

1. 손상·악의적 CFB(순환 FAT, 범위 밖 섹터, 거대한 선언 크기, 디렉터리 항목 수 폭탄, 미니 스트림 순환)가 무한 루프·큰 할당·panic을 만들지 않는다. (Task 1, 2)
2. 정상 파일을 비정상으로 보지 않는다. 코퍼스의 실제 OLE 파일(`.hwp` 200건 표본)에서 `Anomaly`가 0건이다. (Task 2)
3. VBA 압축 해제가 선언 크기를 믿지 않고, 복사 토큰 오프셋이 범위를 벗어나도 panic하지 않는다. (Task 3)
4. 암호화 문서를 "깨끗함"이라 하지 않고 `incomplete`(cyan)로 낸다. 구형 `.doc`를 `.docx`로 이름만 바꾼 것은 분석한다. (Task 4, 6)
5. 일반 매크로 없는 `.doc`와 OLE 객체가 있는 정상 문서(내장 그림)에 소음 HIGH가 생기지 않는다. (Task 4)

---

### Task 1: `libs/ole_reader` — CFB 읽기 코어

**Files:**

- Create: `libs/ole_reader/Cargo.toml`, `libs/ole_reader/src/lib.rs`, `tests/rust/ole_reader/ole_reader.rs`, `tests/rust/ole_reader/common.rs`(CFB 만드는 fixture 도우미)
- Modify: 루트 `Cargo.toml`(workspace 멤버), `docs/rust-workspace.md`(crate 목록)

**Interfaces:**

- Produces:
  - `pub struct Limits { pub max_stream: u64, pub max_total: u64, pub max_entries: usize }` + `Default`.
  - `pub struct Entry { pub id: u32, pub name: String, pub kind: EntryKind, pub start: u32, pub size: u64, pub clsid: [u8; 16], pub parent: Option<u32> }`, `pub enum EntryKind { Unallocated, Storage, Stream, Root, Invalid(u8) }`.
  - `pub struct OleFile` : `OleFile::open(path: &Path, limits: &Limits) -> Result<OleFile, OleError>`, `OleFile::from_bytes(data: Vec<u8>, limits: &Limits) -> Result<OleFile, OleError>`, `entries(&self) -> &[Entry]`, `find(&self, path: &str) -> Option<&Entry>`(`"Macros/VBA/dir"`처럼 `/`로 이은 경로, 대소문자 무시), `read_stream(&mut self, entry: &Entry) -> Result<Vec<u8>, OleError>`(상한 안에서 읽고 선언 크기만큼 자른다), `anomalies(&self) -> &[Anomaly]`, `major_version(&self) -> u16`.
  - `pub enum OleError { NotOle, TooSmall, Unreadable(String), Limit(String) }`(헤더가 OLE가 아니거나 디렉터리를 전혀 못 읽을 때만 오류).
  - `pub enum Anomaly`는 Task 2가 채운다. Task 1은 빈 열거형을 두고 `anomalies()`는 빈 목록이다.
- Consumes: 없음.

- [ ] **Step 1: 명세 확인.** 특수 섹터 값(FREESECT, ENDOFCHAIN, FATSECT, DIFSECT, MAXREGSECT)을 MS-CFB 공식 문서에서 확인해 이 계획의 표와 `lib.rs` 상수 주석에 출처를 적는다. 확인하지 못하면 "미확인"으로 적고 Task 1을 멈춘다.
- [ ] **Step 2: 실패하는 테스트.** `common.rs`에 `make_cfb(streams: &[(&str, &[u8])]) -> Vec<u8>`(주버전 3, 512바이트 섹터, 4096바이트 미만 스트림은 미니 스트림, 이상은 일반 섹터, 평평한 디렉터리 트리)와 `make_cfb_v4`를 둔다.
  - `opens_and_lists_streams`: 스트림 3개(작은 것 둘, 5000바이트 하나)를 넣고 `entries()`에 루트와 3개가 있고 이름·크기가 맞다.
  - `reads_mini_stream_and_regular_stream_exactly`: 두 종류 스트림의 바이트가 입력과 같다.
  - `find_is_case_insensitive_and_follows_storage_paths`: 저장소 아래 스트림을 `"Macros/VBA/dir"`와 `"macros/vba/DIR"`로 찾는다.
  - `version_4_with_4096_byte_sectors_is_read`.
  - `not_ole_and_too_small_are_errors`: 평문, 511바이트 파일.
  - `stream_larger_than_the_limit_is_an_error_not_an_allocation`: 선언 크기 10GB 스트림을 `read_stream`하면 `Limit` 오류다.
  - `the_directory_walk_does_not_trust_the_sibling_tree`: 항목을 선형으로 훑어 이름을 찾으므로 형제·자식 ID가 순환해도 멈추지 않는다.
- [ ] **Step 3: 실행해 실패를 확인한다.**
- [ ] **Step 4: 구현.** 헤더 → DIFAT 따라 FAT 구성(DIFAT 섹터 수는 파일 크기로 상한) → 디렉터리 체인 → 항목을 선형으로 읽어 부모는 자식 ID를 따라가되 방문 표시로 순환을 끊는다 → 스트림 크기가 4096 미만이면 미니 FAT·미니 스트림. 모든 인덱스는 `get`으로 읽는다.
- [ ] **Step 5: 테스트·clippy 통과 후 커밋.** `feat: OLE 복합 파일을 관대하게 읽는 ole_reader를 추가한다`

---

### Task 2: 손상 구조 탐지(`Anomaly`)와 실제 파일 검증

**Files:**

- Modify: `libs/ole_reader/src/lib.rs`
- Test: `tests/rust/ole_reader/anomalies.rs`(신규, `[[test]]` 등록)

**Interfaces:**

- Produces: `pub enum Anomaly { BadByteOrder, BadSectorShift(u16), BadMiniSectorShift(u16), BadMiniCutoff(u32), FatChainCycle { start: u32 }, SectorOutOfRange { sector: u32 }, FatSectorCountMismatch { header: u32, found: u32 }, DifatTruncated, DirectoryTruncated, DirectoryCycle, TooManyEntries(usize), StreamSizeExceedsFile { id: u32, size: u64 }, MiniStreamMissing, CrossLinkedSectors { sector: u32 }, TrailingGarbage(u64), NameLengthMismatch { id: u32 }, InvalidObjectType { id: u32, value: u8 } }`와 `Anomaly::code(&self) -> &'static str`(`OLE_FAT_CYCLE` 같은 안정적인 이름)과 `detail(&self) -> String`.
- 규칙: 읽을 수 있는 만큼 읽은 뒤 이상을 모은다. 오류(`OleError`)는 Task 1의 경우만 낸다.

- [ ] **Step 1: 실패하는 테스트.** `make_cfb`로 정상 파일을 만들고 바이트를 직접 망가뜨린다.
  - 정상 파일은 `anomalies()`가 비어 있다(`clean_file_has_no_anomalies`).
  - FAT 항목을 자기 자신으로 돌려 순환(`FatChainCycle`), 디렉터리 시작 섹터를 범위 밖으로(`SectorOutOfRange`), 헤더 FAT 섹터 수를 거짓으로(`FatSectorCountMismatch`), 스트림 크기를 파일보다 크게(`StreamSizeExceedsFile`), 두 스트림이 같은 섹터를 가리키게(`CrossLinkedSectors`), 항목 수 폭탄(`TooManyEntries`), 이름 길이 거짓(`NameLengthMismatch`), 섹터 시프트 거짓(`BadSectorShift`), 파일 끝 쓰레기(`TrailingGarbage`)를 각각 한 파일로 확인한다. 각 파일이 panic·무한 루프 없이 1초 안에 끝난다.
  - 악의적 입력: 헤더 DIFAT 섹터 수 `0xFFFFFFFF`, FAT 섹터 수 `0xFFFFFFFF`, 디렉터리 섹터가 모든 섹터를 순환 — 모두 오류나 `Anomaly`로 끝나고 메모리를 파일 크기의 몇 배 이상 쓰지 않는다(할당량을 `Limits`로 막는다).
- [ ] **Step 2: 실행해 실패를 확인한다.**
- [ ] **Step 3: 구현.**
- [ ] **Step 4: 실제 파일 검증(게이트).** `find /mnt/data -maxdepth 3 -iname '*.hwp' | head -200`의 파일 중 OLE 매직으로 시작하는 것(HWP 5.x)을 모두 `OleFile::open`으로 열어 열린 수, 열지 못한 수, `Anomaly`가 있는 수를 센다. 정상 파일에서 `Anomaly`가 나오면 파서 오탐인지 파일 손상인지 가려 보고하고, 오탐이면 고친다. 결과를 태스크 보고서에 적는다. HWP3(`HWP Document File` 시작)는 OLE가 아니므로 제외한다.
- [ ] **Step 5: 테스트·clippy 통과 후 커밋.** `feat: ole_reader가 손상된 CFB 구조를 Anomaly로 알린다`

---

### Task 3: MS-OVBA 압축 해제와 VBA 프로젝트 읽기

**Files:**

- Create: `libs/ole_reader/src/vba.rs`
- Test: `tests/rust/ole_reader/vba.rs`(신규, `[[test]]` 등록)

**Interfaces:**

- Produces: `pub fn decompress(data: &[u8], max_out: usize) -> Result<Vec<u8>, VbaError>`(MS-OVBA 2.4.1), `pub struct VbaModule { pub name: String, pub source: String }`, `pub fn read_project(file: &mut OleFile, vba_root: &str, max_total: u64) -> Result<Vec<VbaModule>, VbaError>`(`dir` 스트림을 풀어 모듈 이름·스트림 이름·텍스트 오프셋을 읽고 각 모듈 스트림에서 소스를 푼다), `pub enum VbaError { NotCompressed, Truncated, OutputLimit, BadDir(String) }`.

- [ ] **Step 1: 명세 확인.** MS-OVBA의 `dir` 스트림 레코드 형식(PROJECTMODULES, MODULENAME, MODULESTREAMNAME, MODULEOFFSET 등의 레코드 ID와 크기)과 3.2.2 압축 예제(입력·기대 출력)를 공식 문서에서 읽어 `docs/bookdoctor-ole-notes.md`에 쓰고, 테스트의 기대값은 그 예제에서 가져온다. 레코드 ID를 확인하지 못하면 "미확인"으로 적고 모듈 소스 읽기를 `Attribut` 시그니처 탐색 방식(압축 컨테이너 시작을 찾는)으로 대신하는 안을 ruling으로 기록한다.
- [ ] **Step 2: 실패하는 테스트.**
  - `decompresses_the_spec_example`(Step 1의 예제).
  - `uncompressed_chunk_is_copied`, `multiple_chunks_are_concatenated`.
  - `copy_token_with_offset_before_chunk_start_is_an_error_not_a_panic`, `output_limit_stops_a_bomb`(복사 토큰으로 4MiB를 만드는 입력, `max_out` 1MiB), `bad_signature_and_truncated_chunk_are_errors`.
  - `read_project_extracts_module_sources`: 합성 VBA 프로젝트(`make_cfb`에 `VBA/dir`와 모듈 스트림을 압축해서 넣음)에서 모듈 둘의 소스가 나온다. 압축기는 테스트 도우미(리터럴만 내는 압축)로 만든다.
- [ ] **Step 3: 실행해 실패를 확인한다.**
- [ ] **Step 4: 구현.**
- [ ] **Step 5: 테스트·clippy 통과 후 커밋.** `feat: ole_reader가 MS-OVBA 압축을 풀고 VBA 모듈을 읽는다`

---

### Task 4: DOC 분석기

**Files:**

- Create: `utils/bookdoctor/src/doc/mod.rs`
- Modify: `libs/scan_common/src/finding.rs`(`Format::Doc`), `utils/bookdoctor/Cargo.toml`(`ole_reader` 의존), `utils/bookdoctor/src/scan.rs`, `utils/bookdoctor/src/cli.rs`(`wanted`), `utils/bookdoctor/src/viewer.rs`, `utils/bookdoctor/src/zipcheck/ooxml.rs`(`classify_field`를 `pub(crate)`로)
- Test: `tests/rust/bookdoctor/doc.rs`(신규, `[[test]]` 등록), `tests/rust/bookdoctor/common/mod.rs`(DOC fixture)

**Interfaces:**

- Produces: `Format::Doc`(JSON `"doc"`), 확장자 `doc`·`dot`을 모은다. `doc::scan_doc_file(path, opts, sniffed, report)`와 `doc::inspect(file: &mut OleFile, report)`:
  - 파일이 OLE가 아니면(`Sniffed::Ole`이 아님) `FORMAT_MISMATCH`는 `common`이 내고 여기서는 열지 않는다.
  - OLE지만 `WordDocument` 스트림이 없으면 `FORMAT_MISMATCH`(Defect MEDIUM, 상세 "OLE 파일이지만 Word 문서가 아니다").
  - `Anomaly` 각각을 `OLE_*` 코드(Defect, HIGH 이상은 구조 손상, 나머지 MEDIUM)로 낸다. 파서 악용 가능성(순환, 범위 밖, 거대 선언 크기)은 Security HIGH `OLE_MALFORMED_STRUCTURE`도 한 번 낸다.
  - FIB `fEncrypted`(`flags1 & 0x0100`)이면 `incomplete`로 `encrypted: Word binary document (FIB.fEncrypted)`를 남긴다(cyan). 본문 분석은 하지 않는다.
  - VBA: 저장소 `Macros`(Word 97 문서의 VBA 저장소)나 이름이 `VBA`인 저장소가 있으면 `DOC_VBA_MACRO`(High, 위치는 저장소 경로). 모듈 소스에서 자동 실행 진입점(`AutoOpen`, `AutoExec`, `AutoClose`, `Auto_Open`, `Document_Open`, `Document_Close`, `DocumentOpen`, `Workbook_Open`)이 있으면 `DOC_VBA_AUTOEXEC`(High), 거기에 실행·다운로드 키워드(`Shell`, `CreateObject`, `WScript.Shell`, `powershell`, `cmd.exe`, `URLDownloadToFile`, `XMLHTTP`, `ADODB.Stream`, `Environ`)가 함께 있으면 `DOC_VBA_SUSPICIOUS`(Critical). 매크로 읽기에 실패하면 `incomplete`다.
  - OLE 객체: 저장소 `ObjectPool`이 있으면 `DOC_OLE_OBJECT`(High). 스트림 `\x01Ole10Native`가 있으면 내장 파일이므로 앞 64KiB를 풀어 이름 확장자가 `ZIP_EXEC_EXTS`거나 PE·ELF·Mach-O 매직이면 `EMBEDDED_EXECUTABLE`(High)도 낸다.
  - DDE·필드: `WordDocument` 스트림의 필드(`0x13`~`0x15`)의 지시문을 8비트와 UTF-16LE 두 방식으로 읽어 `classify_field`로 분류한다(`DDEAUTO`는 `OOXML_DDE_FIELD`가 아니라 `DOC_DDE_FIELD`로 낸다: 코드 이름만 다르다).
  - `viewer_blocked(Format::Doc, _)`는 `NotApplicable`.
- Consumes: Task 1~3의 `ole_reader`, P1의 `Member`·`classify_field`.

- [ ] **Step 1: 실패하는 테스트.** fixture는 `make_cfb`로 `WordDocument`(FIB 32바이트 + 본문)와 필요한 저장소·스트림을 넣어 만든다. 저장소는 `make_cfb`에 경로(`"Macros/VBA/dir"`)로 넣을 수 있게 확장한다.
  - `minimal_doc_has_no_findings`, `ole_without_worddocument_is_a_format_mismatch`, `encrypted_doc_is_incomplete_cyan_not_clean`(`flags1`에 `0x0100`).
  - `vba_project_is_high`, `autoexec_alone_is_high_and_autoexec_with_shell_is_critical`, `macro_free_doc_with_the_word_Shell_in_text_is_clean`(본문 텍스트 속 단어는 매크로가 아니다).
  - `object_pool_is_high`, `ole10native_with_exe_name_or_pe_magic_is_embedded_executable`, `plain_embedded_picture_object_is_not_flagged_as_executable`.
  - `dde_field_in_8bit_and_utf16_text_is_flagged`, `normal_fields_like_PAGE_and_TOC_are_clean`.
  - 손상: Task 2의 손상 파일 몇 개가 `OLE_*` 지표를 낸다. 비정상 파일이 panic하면 `scan_file`이 `incomplete`로 남긴다(`panic:`).
  - 디렉토리 수집·확장자: `.doc`·`.dot`은 모으고 `.docs`는 모으지 않는다. `.doc`인데 내용이 ZIP이면 `Format::Doc`가 아니라 위장(`FORMAT_MISMATCH`)이다.
- [ ] **Step 2: 실행해 실패를 확인한다.**
- [ ] **Step 3: 구현.**
- [ ] **Step 4: 소음 게이트.** 코퍼스에 `.doc`가 없다(`find -maxdepth 3`로 0건 확인). 실제 DOC로는 검증하지 못했다고 적는다. 대신 Task 2의 실제 OLE 파일(HWP 5.x)을 DOC 분석기에 넣어 `FORMAT_MISMATCH`(Word 아님)만 나오고 다른 지표가 없는지 확인한다.
- [ ] **Step 5: 테스트·clippy 통과 후 커밋.** `feat: bookdoctor가 구형 DOC의 매크로·OLE 객체·DDE를 탐지한다`

---

### Task 5: RTF 위장

**Files:**

- Create: `utils/bookdoctor/src/doc/rtf.rs`
- Modify: `utils/bookdoctor/src/doc/mod.rs`, `libs/scan_common/src/sniff.rs`(`Sniffed::Rtf`)
- Test: `tests/rust/bookdoctor/rtf_disguise.rs`(신규, `[[test]]` 등록)

**Interfaces:**

- Produces: `Sniffed::Rtf`(앞부분이 `{\rtf`), `doc::rtf::inspect(bytes: &[u8], report)`:
  - `.doc`·`.docx` 등 Word 계열 확장자인데 내용이 RTF면 `FORMAT_MISMATCH`(common이 낸다)와 함께 RTF 안을 본다.
  - `\objdata`(내장 OLE 객체의 16진 본문), `\objclass`, `\objupdate`, `\objautlink`, `\*\datastore` 같은 제어어가 있으면 `RTF_EMBEDDED_OBJECT`(High). `\objdata` 본문이 OLE 매직(`d0cf11e0`)이나 PE 매직(`4d5a`)으로 시작하면 `RTF_EMBEDDED_EXECUTABLE`(Critical)이나 `RTF_EMBEDDED_OLE`(High)로 올린다.
  - 제어어 이름이 `\objd\'61ta`처럼 16진 이스케이프나 주석으로 쪼개져 있어도 찾는다(RTF 파서는 이를 합쳐 읽는다): 제어어 앞뒤의 `\'xx`와 `{\*\...}` 건너뛰기를 해제한 뒤 비교한다.
  - 읽기는 최대 `max_stream` 바이트다.
- Consumes: Task 4의 분기.

- [ ] **Step 1: 실패하는 테스트.** `{\rtf1 ... {\object\objemb{\*\objclass Word.Document.8}{\*\objdata d0cf11e0...}}}`, 쪼갠 제어어 `\obj\'64ata`, 16진 본문이 `4d5a`로 시작, 평범한 RTF(글꼴·문단만)는 지표 없음, 1MiB 반복 `{`(깊이 폭탄)가 시간·메모리를 터뜨리지 않음.
- [ ] **Step 2: 실행해 실패를 확인한다.**
- [ ] **Step 3: 구현.** 중첩 깊이는 세지 않고 바이트를 한 번 선형으로 훑는다.
- [ ] **Step 4: 실제 파일 확인.** 코퍼스의 `.rtf` 11건(`find -maxdepth 3`)을 돌려 지표 수를 보고한다.
- [ ] **Step 5: 테스트·clippy 통과 후 커밋.** `feat: bookdoctor가 RTF 위장과 내장 객체를 탐지한다`

---

### Task 6: 암호화 OOXML과 이름만 바꾼 `.doc`를 `.docx`에서 구분

**Files:**

- Modify: `utils/bookdoctor/src/zipcheck/mod.rs`(`scan_docx_file`의 OLE 분기), `utils/bookdoctor/src/doc/mod.rs`
- Test: `tests/rust/bookdoctor/ooxml_structure.rs`(P1의 OLE 테스트를 새 동작으로 바꾼다), `tests/rust/bookdoctor/doc.rs`

**Interfaces:**

- 동작: `.docx` 계열의 내용이 OLE면 `ole_reader`로 연다. `EncryptedPackage` 스트림이 있으면 `encrypted: OOXML encrypted package (EncryptedPackage)`로 `incomplete`(cyan). `WordDocument`가 있으면 구형 `.doc`를 이름만 바꾼 것이므로 `FORMAT_MISMATCH`와 함께 `doc::inspect`로 분석한다. 둘 다 없으면 P1과 같은 일반 미완료(`OLE container ... cannot be inspected`)로 남긴다. `.hwpx`(P1)는 그대로 둔다(P3).
- P1의 판단("OLE docx는 일반 미완료, cyan 아님")은 이 태스크로 대체된다. 이 태스크가 CFB를 읽어 암호화를 확인하기 때문이다.

- [ ] **Step 1: 실패하는 테스트.** `docx_that_is_an_encrypted_package_is_incomplete_cyan`(`EncryptedPackage`와 `EncryptionInfo` 스트림), `docx_that_is_a_renamed_doc_is_analyzed_as_doc`(VBA가 든 `.doc`를 `.docx`로 이름만 바꾸면 `DOC_VBA_MACRO`가 나온다), `docx_ole_without_either_stream_stays_plain_incomplete`.
- [ ] **Step 2: 실행해 실패를 확인한다.**
- [ ] **Step 3: 구현.**
- [ ] **Step 4: 갭 분석 문서(`docs/bookdoctor-gap-analysis.md`) 갱신.** DOC·암호화 OOXML·RTF 행을 "있음/부분"으로 바꾸고, 실제 DOC로 검증하지 못한 범위를 적는다.
- [ ] **Step 5: 테스트·clippy 통과 후 커밋.** `feat: bookdoctor가 암호화 OOXML과 이름만 바꾼 DOC를 구분한다`

---

## P2 완료 검증 (실행자)

- [ ] `cargo test --workspace --locked --no-fail-fast`(한 번), `cargo clippy --workspace --all-targets --locked`(ole_reader·bookdoctor·scan_common 경고 0), `Cargo.lock`이 `ole_reader` 한 줄만 늘었는지 `git diff`로 확인.
- [ ] 표본 EPUB·PDF·TXT 출력이 변경 전과 같다(기준: `scratchpad/p0/base-*.ndjson`).
- [ ] 최종 리뷰어(opus)에게 Review Focus와 ledger의 `Ruling:`을 주고 검토받는다. Critical·Important는 한 번에 고친다.
