# bookdoctor 규칙 엔진

`--rules PATH`와 `--builtin-rules`로 켜는 자체 규칙 엔진이다. YARA 문법의 작은 부분집합으로 쓴 규칙을 파일의 **원시 바이트**에 적용한다. 외부 의존성은 없다.

기본은 꺼져 있다(비용 0). 켜면 `scan_file`이 분석기가 끝난 뒤 파일의 앞 16MiB에 규칙을 적용하고, 일치한 규칙마다 지표 `RULE_MATCH`를 더한다.

## 지표

| 항목   | 값                                                                            |
| ------ | ----------------------------------------------------------------------------- |
| 코드   | `RULE_MATCH`                                                                  |
| 분류   | Security                                                                      |
| 심각도 | 규칙의 `severity`(기본 medium). `--min-severity`와 `--only`가 똑같이 적용된다 |
| 위치   | 규칙 이름                                                                     |
| 설명   | 규칙의 `description`, 이어서 일치한 문자열 `$id ×횟수 @오프셋,…`(처음 5개)    |

중첩 파일(ZIP 안의 ZIP·PDF·RTF·Word·HWP)은 재귀 검사가 같은 옵션으로 `scan_file`을 부르므로 중첩 파일에도 적용되고, 위치는 `멤버!규칙이름`이다. `--isolate`의 자식에도 같은 규칙이 적용된다.

## 문법

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

- 이름은 `[A-Za-z_][A-Za-z0-9_]*`. 주석은 `//`와 `/* */`. 문자열 이스케이프는 `\\ \" \n \r \t \xNN`.
- 연산자: `and`, `or`, `not`(우선순위는 `not` > `and` > `or`), 괄호, 개수 비교 `== != < <= > >=`(`#a`와 정수만), `at`(정확한 오프셋), `N of them`, `any of them`, `all of them`.
- `ascii`와 `wide`를 함께 쓰면 둘 다 본다. `wide`는 ASCII 문자열을 UTF-16LE로 본다. `nocase`는 ASCII 대소문자만 무시한다.
- 문자열은 최소 2바이트다(1바이트 패턴은 일치가 폭증해 작업량 상한을 무의미하게 만든다).

## 한도

| 항목                     | 상한                                             |
| ------------------------ | ------------------------------------------------ |
| 규칙 원문(파일 하나)     | 1MiB                                             |
| 규칙 수(내장 포함)       | 1,000                                            |
| 규칙당 문자열            | 256                                              |
| 문자열 하나              | 4,096바이트                                      |
| 조건 중첩(괄호, `not`)   | 32단계                                           |
| 규칙당 조건 항           | 1,000                                            |
| 디렉터리의 `.rules` 파일 | 100                                              |
| 파일에서 읽는 바이트     | 앞 16MiB(넘으면 `incomplete`)                    |
| 한 파일의 비교 작업량    | 5억(훑기와 조건 평가 각각, 넘으면 `incomplete`)  |
| 문자열 일치 횟수         | 1,000에서 멈춤(`#a`는 그 이상을 구별하지 못한다) |

한도를 넘는 규칙 파일은 오류다(`파일:줄: 메시지`를 stderr에 쓰고 종료 코드 2).

## YARA와의 차이

지원하지 않는다: `import`, 정규식 `/…/`, 16진 점프 `[n-m]`, 16진 대안 `( | )`, 반 바이트 와일드카드 `?A`, `of ($a*)` 같은 문자열 집합, `for`, `filesize`, `uint8()`류, `private`·`global`, 태그, `fullword`·`xor`·`base64` 수식어. 만나면 줄 번호와 함께 `unsupported syntax` 오류다.

`at`은 기록한 오프셋이 아니라 그 위치에서 직접 패턴을 확인한다. 같은 이름의 규칙이 여러 파일에 있으면 오류다.

## 한계

- 규칙은 원시 바이트에 적용된다. **압축된 내용(ZIP 멤버, PDF 스트림, OLE 스트림, HWP 스트림)은 보지 못한다.** 압축된 구성 요소의 검사는 기존 분석기가 맡는다.
- 코퍼스에 악성 파일이 없어 내장 규칙은 정상 쪽(오탐)만 측정했고 재현율은 모른다.

## 내장 규칙(`--builtin-rules`)

강한 IOC만 둔다.

| 규칙                         | 심각도   | 일치 조건                                                                              |
| ---------------------------- | -------- | -------------------------------------------------------------------------------------- |
| `EICAR_TEST_FILE`            | critical | EICAR 표준 시험 문자열(악성 코드가 아니라 탐지기 점검용)                               |
| `POWERSHELL_ENCODED_COMMAND` | high     | `powershell`과 (`-enc `, `-ec `, `-encodedcommand`)                                    |
| `POWERSHELL_DOWNLOAD_CRADLE` | high     | `Net.WebClient`와 (`DownloadString` 또는 `DownloadFile`)                               |
| `LOLBIN_REMOTE_EXEC`         | high     | `mshta`+URL, `regsvr32`+`/i:http`, `certutil`+`-urlcache`, `bitsadmin`+`/transfer`+URL |
| `BASE64_PE_HEADER`           | high     | base64로 인코딩한 MZ 헤더 3종                                                          |
| `HEX_PE_HEADER`              | high     | 16진 문자열로 쓴 MZ 헤더                                                               |
| `WSCRIPT_SHELL_RUN`          | medium   | `WScript.Shell`과 (`.Run` 또는 `.Exec`)                                                |

`EICAR_TEST_FILE`을 뺀 규칙은 ASCII와 UTF-16LE 둘 다 본다(EICAR는 ASCII만). `BASE64_PE_HEADER`는 base64 특성상 대소문자를 구별한다.
