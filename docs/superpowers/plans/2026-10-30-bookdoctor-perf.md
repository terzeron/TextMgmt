# bookdoctor 성능 최적화 계획 (EPUB · PDF)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans (사용자가 inline 실행을 골랐다). Steps use checkbox (`- [ ]`) syntax.

**Goal:** 표본 측정으로 찾은 낭비를 줄여 EPUB·PDF 스캔 시간을 줄인다. 검사 결과는 바뀌지 않는다.

**Architecture:** 결과를 바꾸지 않는 네 가지만 한다: (1) EPUB 비텍스트 멤버의 불필요한 UTF-8 변환 제거, (2) EPUB 렌더링 단계의 중복 압축 해제 제거, (3) PDF 참조 계수 정규식을 직접 짠 스캐너로 교체, (4) 다음 파일을 미리 읽는 선읽기 스레드. 각 최적화는 "변경 전 출력과 같음"이 통과 조건이다.

**Spec:** 설계서 `docs/superpowers/specs/2026-10-30-bookdoctor-malware-scanner-design.md` (성능 원칙: 대량 파일 검사에 적합하도록 불필요한 전체 파싱을 줄인다).

## 측정 기준선 (2026-10-30, release 빌드, 기본 옵션 = 워커 1개)

| 대상                                                        | 수치                                                                                                                  |
| ----------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------- |
| EPUB 표본 200건(258MB)                                      | cold 18.84s(읽기 197MB), warm 7.83s(user 7.48s). 파일당 CPU 37ms                                                      |
| PDF 표본 196건(4,826MB, 중앙값 6.5MB, p90 84MB, 최대 280MB) | 103~108s, user 23s, sys 8s, 읽기 4.6GB(약 45MB/s). CPU는 약 31s라 I/O가 지배한다                                      |
| PDF 40건(작은 것, 38MB), warm                               | poppler 있음 1.08s, `--no-poppler` 0.27s. `pdfinfo` 프로세스가 75%                                                    |
| callgrind EPUB 25건                                         | 7.79G 명령. 보안 48.8%, 렌더링 41.5%. inflate 46%(보안과 렌더링이 각각 푼다). `decode_text`의 `from_utf8_lossy` 19.5% |
| callgrind PDF 20건(`--no-poppler`)                          | 4.46G 명령. `load` 70.9%(lopdf `Reader::read` 57%, 참조 계수 정규식 13.3%), `scan_loaded` 24.4%                       |

## Global Constraints

- 새 의존성을 추가하지 않는다(필요하면 멈추고 승인을 받는다). `libc`·`memmap2` 같은 crate를 새로 들이지 않는다.
- 검사 결과는 같아야 한다. 통과 조건은 (a) 해당 crate 테스트 전부, (b) 표본 EPUB 200건·PDF 표본에서 `--min-severity low --json` 출력의 (파일, 분류, 심각도, 코드, 위치, 상세)와 `incomplete` 사유 집합이 변경 전 바이너리와 같음이다.
- 변경 전 바이너리는 `scratchpad/p0/bookdoctor-perfbase`(이 계획 시작 커밋 `7d0ec435`의 release 빌드)다. 비교 스크립트는 `scratchpad/p0/cmp_sets.py`다.
- 속도는 같은 조건에서 3회 재어 중앙값을 적는다. cold는 같은 파일을 처음 읽는 실행이 아니므로 "읽은 바이트(`/usr/bin/time %I`)"와 함께 적고, warm 수치와 구분한다.
- 전체 workspace 테스트는 동시에 두 번 돌리지 않는다. 커밋: Conventional Commits, 본문 `-` bullet.

## Review Focus

1. EPUB에서 텍스트가 아닌 멤버(이미지, 폰트)의 매직 검사·압축비·CRC 오류 사유가 그대로다. (Task 1, 2)
2. 렌더링 단계가 보안 단계에서 이미 읽은 멤버를 건너뛰어도, 예산 소진(`budget exhausted`)·`skipped` 판정과 CRC 오류 보고가 변하지 않는다. (Task 2)
3. 직접 짠 참조 계수기가 정규식과 같은 개수를 센다(공백·주석 `%...`·`R` 직전 숫자 경계, `0 0 1 RG` 같은 연산자, 큰 파일 경계). (Task 3)
4. 선읽기 스레드가 소비자보다 너무 앞서가 페이지 캐시를 밀어내지 않고, 소비자가 멈추거나 오류로 끝나도 스레드가 남지 않는다. (Task 4)

---

### Task 1: EPUB 비텍스트 멤버의 UTF-8 변환 제거

**Files:**

- Modify: `utils/bookdoctor/src/epub/security.rs`(`check_member`)
- Test: `tests/rust/bookdoctor/epub_security.rs`(기존), `tests/rust/bookdoctor/epub_perf_equiv.rs`(신규, `[[test]]` 등록)

**Interfaces:** 공개 인터페이스는 바뀌지 않는다.

- [ ] **Step 1: 실패하는 테스트.** `check_member`는 `Kind::Other` 멤버에도 `decode_text`(손실 UTF-8 변환)를 한다. 결과를 바꾸지 않으므로 동작 테스트가 아니라 비용 테스트를 쓴다: `non_text_member_is_not_decoded`. 1MiB 무작위 `.jpg` 멤버 하나를 가진 EPUB을 검사하는 동안 `decode_text`가 불린 횟수를 세는 카운터(`#[cfg(test)]`가 아니라 `pub static DECODE_CALLS: AtomicUsize`, 테스트에서만 읽는다)가 0이다. 카운터는 `decode_text` 안에서만 증가한다.
  - 동치 테스트: `perf_equiv::epub_findings_unchanged_for_mixed_members`. 텍스트(xhtml, css, js, opf)와 비텍스트(jpg, png, ttf, bin) 멤버가 섞인 fixture 12종의 `scan_file` 결과(지표·사유)가 기대값(변경 전에 `scratchpad`에서 생성해 테스트에 하드코딩하지 않고, 변경 전 바이너리로 만든 JSON을 `tests/rust/bookdoctor/fixtures/perf_equiv/*.json`에 둔다)과 같다.
- [ ] **Step 2: 실행해 실패를 확인한다.**
- [ ] **Step 3: 구현.** `check_member`에서 `Kind::Other`이고 확장자가 `txt`가 아닐 때는 `decode_text`와 그 뒤 인코딩·텍스트 검사를 건너뛴다. `check_magic`과 압축비 검사는 그대로 둔다(둘은 `decode_text` 앞이다). `scan_text`는 `Kind::Other`에서 아무 일도 하지 않으므로 동작이 같다.
- [ ] **Step 4: 통과 확인과 측정.** crate 테스트 전부, 표본 EPUB 200건 동치, `callgrind` EPUB 25건 명령 수(기준 7.79G)를 적는다.
- [ ] **Step 5: 커밋.** `perf: EPUB 비텍스트 멤버의 불필요한 UTF-8 변환을 건너뛴다`

---

### Task 2: EPUB 렌더링 단계의 중복 압축 해제 제거

**Files:**

- Modify: `utils/bookdoctor/src/epub/security.rs`(`Stats`에 읽기 성공 멤버 기록), `utils/bookdoctor/src/epub/render.rs`(`inspect_with`가 건너뛸 멤버를 받는다), `utils/bookdoctor/src/epub/mod.rs`
- Test: `tests/rust/bookdoctor/epub_perf_equiv.rs`, 기존 `epub_render.rs`·`epub_security*.rs`

**Interfaces:**

- Produces: `security::Stats.verified: HashMap<String, u64>`(보안 단계가 끝까지 읽어 CRC까지 통과한 멤버 이름 → 읽은 바이트). `render::inspect_with(path, report, limits)`는 그대로 두고 `render::inspect_with_verified(path, report, limits, verified: &HashMap<String, u64>)`를 더한다.
- 규칙: 렌더링 단계는 `verified`에 있고 내용이 필요 없는 멤버(spine 문서·NCX가 아닌 것)를 읽지 않는다. 읽은 것으로 치고 예산에서 `min(읽은 바이트, limit)`만 뺀다(예산 소진 판정이 같게).

- [ ] **Step 1: 실패하는 테스트.**
  - `render_skips_members_already_verified_by_security`: 이미지·폰트가 든 EPUB에서 렌더링 단계가 `io::copy`로 읽은 멤버 수를 세는 카운터(`pub static RENDER_READS`)가 `verified`의 크기만큼 줄어 있다.
  - 동치: `epub_findings_unchanged_when_security_has_verified_members`와 예산 경계 `budget_exhaustion_notes_unchanged`(`--max-total-mb 1`로 예산이 바닥나는 EPUB)와 `crc_mismatch_still_reported_once`(손상 멤버는 `verified`에 없으므로 렌더링이 읽어 보고한다)가 변경 전 JSON과 같다.
- [ ] **Step 2: 실행해 실패를 확인한다.**
- [ ] **Step 3: 구현.** `scan_epub_file`이 `security::scan_stats`가 돌려준 `Stats.verified`를 렌더링에 넘긴다. 보안 단계가 건너뛴 멤버(암호화, 예산 초과, 큰 멤버)는 `verified`에 넣지 않는다.
- [ ] **Step 4: 측정.** 표본 EPUB 200건 동치와 warm 시간, callgrind 명령 수를 적는다.
- [ ] **Step 5: 커밋.** `perf: EPUB 렌더링 단계가 이미 검증한 멤버를 다시 풀지 않는다`

---

### Task 3: PDF 참조 계수를 직접 짠 스캐너로

**Files:**

- Modify: `utils/bookdoctor/src/pdf/security.rs`(참조 계수 부분)
- Test: `tests/rust/bookdoctor/pdf_ref_count.rs`(신규, `[[test]]` 등록)

**Interfaces:**

- Produces: `pdf::security::count_references(bytes: &[u8], cap: usize) -> usize`: 기존 정규식 `(?-u)[0-9]+(?:WS|%comment)+[0-9]+(?:WS|%comment)*R`(`\b` 없음)과 같은 개수를 세되 `cap`을 넘으면 멈춘다. 기존 정규식 경로는 비교용 `count_references_regex`로 테스트에만 남긴다.

- [ ] **Step 1: 실패하는 테스트.** 정규식 구현을 기준 구현으로 두고 두 함수의 개수를 비교한다.
  - 고정 사례: `1 0 R`, `12 0 R 3 0 R`, `1 0 R`이 줄바꿈·탭·폼피드로 나뉜 것, 사이에 `%주석\n`이 낀 것, `0 0 1 RG`(과계수 방향), 숫자 없이 `R`만 있는 것, 숫자 하나 뒤 `R`, 파일 끝에서 잘린 `1 0`, 매우 긴 숫자열.
  - 속성 테스트(의존성 없이 시드 고정 의사난수): 바이트 8,000,000개짜리 무작위 입력(숫자·공백·`%`·`R`·`\n`·그 밖의 바이트를 섞음)에서 두 함수가 같은 값을 낸다. `cap` 경계에서도 같다(`min(정규식 개수, cap)`).
  - 성능: 같은 입력 64MiB에서 새 함수가 정규식보다 빠르다(비율은 측정만 기록하고 단언은 "느리지 않다").
- [ ] **Step 2: 실행해 실패를 확인한다.**
- [ ] **Step 3: 구현.** `memchr`는 이미 쓰는 의존성(`regex`가 끌고 오는 것)이 아니라면 쓰지 않는다. 표준 `slice::iter().position`으로 `R`을 찾고 뒤로 걸어 공백·주석·숫자 두 덩어리를 확인한다.
- [ ] **Step 4: 통과 확인과 측정.** crate 테스트, PDF 표본 동치, callgrind PDF 20건 명령 수(기준 4.46G)를 적는다.
- [ ] **Step 5: 커밋.** `perf: PDF 참조 계수를 정규식 대신 직접 짠 스캐너로 센다`

---

### Task 4: 다음 파일을 미리 읽는 선읽기

**Files:**

- Create: `utils/bookdoctor/src/prefetch.rs`
- Modify: `utils/bookdoctor/src/cli.rs`, `utils/bookdoctor/src/lib.rs`
- Test: `tests/rust/bookdoctor/prefetch.rs`(신규, `[[test]]` 등록)

**Interfaces:**

- Produces: `prefetch::Prefetcher::start(paths: Vec<PathBuf>, ahead_files: usize, ahead_bytes: u64, max_file_bytes: u64) -> Prefetcher`, `Prefetcher::advance(&self, done: usize)`(소비자가 `done`번째까지 처리했음을 알린다), `Drop`(스레드를 멈추고 기다린다). 스레드는 `paths`를 순서대로 읽되 소비자보다 `ahead_files`개 또는 `ahead_bytes`바이트 이상 앞서가지 않고, `max_file_bytes`보다 큰 파일은 건너뛴다. 읽는 방식은 1MiB씩 읽어 버리는 것이다(페이지 캐시를 채우는 용도, 새 의존성 없음).
- 기본값: `ahead_files = 16`, `ahead_bytes = 512 MiB`, `max_file_bytes = 128 MiB`. `--no-prefetch`로 끈다(`Options.prefetch: bool`).

- [ ] **Step 1: 실패하는 테스트.**
  - `prefetcher_reads_ahead_but_not_beyond_the_limits`: 임시 파일 10개(각 1MiB), `ahead_files = 3`일 때 `advance(0)` 직후 스레드가 읽은 파일 수가 3 이하이고 곧 3이 된다(읽은 수는 `Prefetcher::prefetched()`로 읽는다). `advance(5)` 뒤에는 8까지 간다.
  - `prefetcher_skips_files_over_the_size_cap`.
  - `dropping_the_prefetcher_stops_the_thread`: `Drop` 뒤 `Prefetcher`의 스레드 핸들이 끝났다(`is_finished`).
  - `missing_files_do_not_stop_the_prefetcher`.
  - CLI: `--no-prefetch`가 파싱되고 도움말에 있다. 선읽기 켠 실행과 끈 실행의 출력이 같다.
- [ ] **Step 2: 실행해 실패를 확인한다.**
- [ ] **Step 3: 구현.** `scan_streaming`의 `emit` 콜백이 처리한 수를 세어 `advance`를 부른다. 소비자는 워커 수와 무관하게 "윈도우 단위로 끝난 파일 수"를 넘긴다.
- [ ] **Step 4: 측정.** 표본 EPUB 200건·PDF 196건을 `--no-prefetch`와 기본으로 번갈아 3회씩 재어 중앙값과 `/usr/bin/time %I`(읽은 바이트)를 적는다. 효과가 5% 미만이면 기본값을 끄고(옵션 `--prefetch`) 수치를 보고한다.
- [ ] **Step 5: 커밋.** `perf: 다음 파일을 미리 읽어 디스크 대기와 검사를 겹친다`

---

## 완료 검증

- [ ] `cargo test --workspace --locked --no-fail-fast`(한 번), `cargo clippy --workspace --all-targets --locked`(bookdoctor·scan_common·corpus_common·quarantine_broken 경고 0).
- [ ] 표본 600건 기본 출력의 High 이상 집합이 `scratchpad/p0/old-*.ndjson`과 같다.
- [ ] 기준선 표를 같은 방법으로 다시 재어 변경 전후를 표 한 장으로 보고한다.
- [ ] 하지 않은 것(PDF 이미지 스트림을 건너뛰는 지연 로딩, `pdfinfo` 호출 줄이기)과 이유를 보고한다.
