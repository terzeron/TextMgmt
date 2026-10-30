# bookdoctor P5 구현 계획 (자체 규칙 엔진)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans (사용자가 inline 실행을 골랐고 subagent 독립 실행을 금지했다). Steps use checkbox (`- [ ]`) syntax.

**Goal:** YARA 문법의 작은 부분집합으로 쓴 규칙(문자열·16진 패턴과 간단한 조건)을 파일의 원시 바이트에 적용하는 규칙 엔진을 `bookdoctor`에 넣는다. 외부 의존성 없이 직접 만든다. 기본은 꺼져 있고 `--rules PATH`(사용자 규칙)와 `--builtin-rules`(내장 소규모 규칙)로 켠다.

**왜 YARA-X가 아닌가:** 사용자가 의존성 없는 자체 엔진을 골랐다(2026-10-30). `yara-x` 1.22.1은 취약점 0건이지만 182개 패키지(wasmtime 포함)와 Rust 1.95가 필요하고, 현재 Rust 1.91.1로 풀리는 1.19.0은 wasmtime 취약점이 8건이다. 설계서의 "외부 규칙 대신 소규모 자체 규칙" 취지에도 맞다.

**Architecture:** `utils/bookdoctor/src/rules/{mod,parse,matcher,builtin}.rs`. `parse`는 규칙 텍스트를 `RuleSet`으로, `matcher`는 바이트에 적용해 일치한 규칙을 돌려준다. `scan_file`이 분석기 뒤에서 파일의 앞 16MiB에 적용하고 `RULE_MATCH` 지표를 더한다. 중첩 파일은 재귀 검사가 같은 `scan_file`을 부르므로 자동으로 적용된다. 압축된 내용(ZIP 멤버, PDF·OLE 스트림)은 원시 바이트에 보이지 않으므로 규칙 대상이 아니다(한계로 문서화한다).

**Tech Stack:** 표준 라이브러리만 쓴다. 새 의존성은 없다.

**Spec:** `docs/superpowers/specs/2026-10-30-bookdoctor-malware-scanner-design.md`

## 규칙 문법 (YARA의 부분집합)

```
rule NAME {
  meta:
    severity = "high"          // low|medium|high|critical, 기본 medium
    description = "설명"
  strings:
    $a = "text" nocase wide    // 수식어: nocase, wide, ascii(기본)
    $b = { 4D 5A ?? 00 }       // 16진, ?? 와일드카드. 첫 바이트는 와일드카드 불가
  condition:
    any of them | all of them | 2 of them
    | $a | #a >= 3 | $b at 0 | not $a | ( $a and $b ) or $c
}
```

- 이름: `[A-Za-z_][A-Za-z0-9_]*`. 주석: `//`와 `/* */`. 문자열 이스케이프: `\\ \" \n \r \t \xNN`.
- 연산자: `and`, `or`, `not`, 괄호, 비교 `== != < <= > >=`(개수 `#a`와 정수만), `at`(정확한 오프셋), `N of them`, `any of them`, `all of them`.
- 지원하지 않는 것(YARA와의 차이): `import`, 정규식, 점프 `[n-m]`, 16진 대안 `( | )`, `of ($a*)` 집합, `for`, `filesize`, `uint8()`류, `private`/`global`, 태그.

## Global Constraints

- 새 의존성을 추가하지 않는다. 툴체인을 올리지 않는다.
- 규칙 파일은 1MiB, 규칙 1,000개(내장 포함), 규칙당 문자열 256개, 문자열 하나 4,096바이트, 조건 중첩 32단계를 넘지 못한다. 넘으면 오류다(줄 번호와 함께 `--rules` 읽기에서 종료 코드 2).
- 매칭은 파일의 앞 16MiB만 본다(넘으면 `incomplete`). 한 파일의 비교 작업량에 상한(5억 바이트 비교)이 있다. 넘으면 `incomplete`다.
- 기본은 규칙을 적용하지 않는다(비용 0). `--rules`나 `--builtin-rules`가 있을 때만 읽는다.
- 출력 규칙은 P0~P4 그대로다. 지표 `RULE_MATCH`는 Security, 심각도는 규칙의 `severity`, 위치는 규칙 이름, 설명에 일치한 문자열 ID와 처음 오프셋을 넣는다.
- cargo는 `env -u TM_MYSQL_HOST -u TM_ES_URL`, `CARGO_TARGET_DIR=/home/terzeron/workspace/tm_dev/target`. 커밋은 Conventional Commits, 본문 `-` bullet. push·머지 없음. subagent를 쓰지 않는다.

## Review Focus

1. 악의적 규칙 파일(깊은 중첩, 거대한 문자열, 규칙 폭증)이 파서에서 시간·메모리·스택을 터뜨리지 않는다. (Task 1)
2. 악의적 입력 파일(같은 바이트 반복 × 비슷한 접두 패턴 다수)이 매칭 시간을 터뜨리지 않는다. 작업량 상한이 걸리면 미완료다. (Task 2)
3. 규칙을 안 쓰면 비용이 0이고 기존 출력이 바뀌지 않는다. (Task 3)
4. 내장 규칙이 정상 파일에 소음을 만들지 않는다. 코퍼스 표본으로 측정한다. (Task 4)
5. 규칙 일치가 `--isolate`와 재귀 검사 아래에서도 같은 결과를 낸다. (Task 3)

---

### Task 1: 규칙 파서

**Files:** 신규 `utils/bookdoctor/src/rules/mod.rs`, `utils/bookdoctor/src/rules/parse.rs`, `utils/bookdoctor/src/lib.rs`, 테스트 `tests/rust/bookdoctor/rules_parse.rs`(신규, `[[test]]`).

**Interfaces:** Produces `rules::RuleSet`(`rules: Vec<Rule>`), `rules::Rule { name, severity, description, strings: Vec<RuleString>, condition: Cond }`, `rules::RuleString { id: String, pattern: Pattern }`, `rules::Pattern::{Text { bytes, nocase, wide }, Hex(Vec<Option<u8>>)}`, `rules::Cond`(AST), `rules::parse(source: &str) -> Result<RuleSet, RuleError>`, `RuleError { line: usize, message: String }`.

- [ ] **Step 1: 실패하는 테스트.** `parses_a_full_rule`(위 문법 전부), `comments_and_escapes`, `hex_with_wildcards_and_a_leading_wildcard_is_rejected`, `unknown_modifier_and_unsupported_syntax_are_errors_with_line_numbers`(`import`, 정규식 `/x/`, 점프 `[2-4]`, `for`), `condition_references_must_exist_and_names_must_be_unique`, `precedence_not_and_or_and_parentheses`, `limits_are_errors`(규칙 1,001개, 문자열 257개, 5,000바이트 문자열, 중첩 33단계, 1MiB 초과), `a_megabyte_of_open_parens_does_not_overflow_the_stack`, `empty_source_is_an_empty_set`.
- [ ] **Step 2: 실행해 실패를 확인한다.** **Step 3: 구현.** 재귀 하강 파서, 중첩은 깊이 카운터로 제한한다.
- [ ] **Step 4: 통과·clippy 후 커밋.** `feat: bookdoctor에 규칙 파서를 추가한다`

### Task 2: 매처

**Files:** 신규 `utils/bookdoctor/src/rules/matcher.rs`, `utils/bookdoctor/src/rules/mod.rs`, 테스트 `tests/rust/bookdoctor/rules_match.rs`(신규, `[[test]]`).

**Interfaces:** Produces `RuleSet::scan(&self, data: &[u8]) -> ScanOutcome { matches: Vec<RuleMatch>, work_limit_hit: bool }`, `RuleMatch { rule: usize, strings: Vec<(String, Vec<usize>)> }`(처음 오프셋 최대 5개와 개수).

- 한 번의 훑기: 첫 바이트(대소문자 무시는 양쪽)로 후보 패턴 표를 만들고 각 위치에서 후보만 확인한다. `wide`는 ASCII 문자열을 UTF-16LE로 본다. 16진은 `??`를 건너뛴다.
- 조건 평가: 문자열 일치 여부, `#id`는 개수(상한 1,000), `at N`은 기록한 오프셋(문자열당 256개까지)과 비교, `N of them`.
- 작업량: 후보를 확인하며 비교한 바이트 수를 센다. 5억을 넘으면 중단하고 `work_limit_hit`이다.

- [ ] **Step 1: 실패하는 테스트.** `text_nocase_and_wide`, `hex_with_wildcards`, `any_all_n_of_them`, `counts_and_at_offsets`, `not_and_or_precedence_at_evaluation`, `overlapping_matches_are_all_found`, `empty_data_matches_nothing_and_not_$a_matches`, `a_pattern_spanning_the_end_does_not_panic`, `hostile_repeated_prefix_input_hits_the_work_limit_fast`(16MiB의 `a`와 `aaaa…b` 형 패턴 수백 개가 5초 안에 끝나고 `work_limit_hit`), `a_megabyte_with_a_thousand_rules_is_fast`.
- [ ] **Step 2~4:** 실패 확인, 구현, 통과·clippy·커밋. `feat: bookdoctor가 규칙을 바이트에 적용하는 매처를 추가한다`

### Task 3: 통합 (`--rules`, `--builtin-rules`)

**Files:** `utils/bookdoctor/src/{cli,scan,isolate,lib}.rs`, 테스트 `tests/rust/bookdoctor/rules_cli.rs`(신규, `[[test]]`).

**Interfaces:** Produces `Options { rule_files: Vec<PathBuf>, builtin_rules: bool, ruleset: Option<Arc<RuleSet>> }`, `rules::scan_report(path: &Path, set: &RuleSet, report: &mut FileReport)`.

- `--rules PATH`는 여러 번 쓸 수 있고 파일이나 디렉터리(`*.rules`)다. `run`이 파싱 직후 읽어 `ruleset`을 만든다. 규칙 오류는 `파일:줄: 메시지`로 stderr에 쓰고 종료 코드 2다.
- `scan_file`은 `ruleset`이 있을 때만 분석기 뒤에서 파일의 앞 16MiB를 읽어 적용한다. 일치마다 `RULE_MATCH`(Security, 규칙 severity). 16MiB를 넘으면 `incomplete`(`rules scanned only the first 16 MiB`), 작업량 상한이면 `incomplete`.
- `--isolate`의 자식에는 `--rules`·`--builtin-rules`를 그대로 넘긴다.
- `--only`·`--min-severity` 필터는 `RULE_MATCH`에도 똑같이 적용된다.

- [ ] **Step 1: 실패하는 테스트.** `no_rules_means_no_rule_findings_and_the_default_output_is_unchanged`, `rules_file_matches_a_text_file`(`--rules`), `directory_of_rules_files`, `a_bad_rules_file_exits_2_with_file_and_line`, `severity_and_location_and_detail`, `files_over_16_mib_are_incomplete`, `nested_members_are_matched_under_recursion`(ZIP 안의 무압축 `.txt` 멤버는 중첩 파일이 아니므로 안 됨, 대신 ZIP 안의 RTF 멤버가 됨), `isolate_passes_the_rules_to_the_child`, `the_min_severity_filter_applies`.
- [ ] **Step 2~4:** 실패 확인, 구현, 통과·clippy·커밋. `feat: bookdoctor에 --rules와 --builtin-rules를 추가한다`

### Task 4: 내장 규칙

**Files:** 신규 `utils/bookdoctor/src/rules/builtin.rs`(`BUILTIN: &str`), 테스트 `tests/rust/bookdoctor/rules_builtin.rs`(신규, `[[test]]`).

내장 규칙은 강한 IOC만 둔다: `EICAR_TEST_FILE`(critical, 표준 시험 문자열), `POWERSHELL_ENCODED_COMMAND`(high), `POWERSHELL_DOWNLOAD_CRADLE`(high), `LOLBIN_REMOTE_EXEC`(high: `mshta`+`http`, `regsvr32`+`/i:http`, `certutil`+`-urlcache`, `bitsadmin`+`/transfer`), `BASE64_PE_HEADER`(high), `HEX_PE_HEADER`(high), `WSCRIPT_SHELL_RUN`(medium).

- [ ] **Step 1: 실패하는 테스트.** 규칙마다 양성(일치)과 음성 두 개(비슷하지만 아닌 텍스트), 내장 규칙이 파서를 통과하고 이름이 중복되지 않음, EICAR는 비악성 시험 문자열이며 `--builtin-rules`로 Critical로 나옴.
- [ ] **Step 2~3:** 실패 확인, 구현.
- [ ] **Step 4: 코퍼스 표본(HTML 3,034건, TXT·EPUB·PDF 표본)에 `--builtin-rules`를 켜고 지표별 건수·시간 증가를 보고하고, 정상 파일의 일치가 1%를 넘는 규칙은 조건을 좁히거나 등급을 낮춘다(`Ruling:`으로 남긴다).**
- [ ] **Step 5: 통과·clippy 후 커밋.** `feat: bookdoctor에 내장 소규모 규칙을 추가한다`

### Task 5: 문서·완료 검증

- [ ] 갭 분석(난독화·스크립트 행 또는 새 행), `USAGE`·`print_usage`를 갱신하고 문법 설명과 YARA와의 차이를 사용법 도움말 또는 `docs/bookdoctor-rules.md`에 적는다.
- [ ] `cargo test --workspace --locked --no-fail-fast`(한 번), `cargo clippy --workspace --all-targets --locked`, `Cargo.lock` diff(변경 없음이어야 한다), 표본 TXT·PDF·EPUB이 기준과 같은지(규칙 끔) 확인.
- [ ] 자기 리뷰(Review Focus 5개)와 변조·폭탄 실험(악의적 규칙 파일, 악의적 입력).
