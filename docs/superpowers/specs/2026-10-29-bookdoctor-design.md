# bookdoctor 설계

작성일 2026-10-29. 대상 저장소 `tm_dev`. 상태: 사용자 승인 전 초안.

## 1. 의도

`pdfscan`, `epubscan`, `detect_mojibake` 세 도구는 같은 일을 나눠서 한다. 책 파일이 위험하지 않은지, 깨지지 않았는지,
우리 백엔드와 프론트엔드가 열 수 있는지 확인하는 일이다. 도구가 셋이라 옵션, 리포트 모양, 종료 코드가 제각각이다.
같은 파일을 보려면 세 번 돌려야 한다.

`bookdoctor` 하나가 epub, pdf, txt 파일을 한 번에 진단한다. 결과는 하나의 리포트로 나오고, `quarantine_broken` 같은 후속 도구가 이 리포트를 읽는다.

## 2. 목표, 비목표, 반목표

**목표**

- epub, pdf, txt를 지원하고 파일 형식은 확장자가 아니라 매직바이트로도 확인한다.
- 모든 발견 항목을 세 분류로 나눈다.
  - `security`: 실행·유출 위험
  - `defect`: 파일 손상
  - `render`: 백엔드·프론트엔드 렌더링이 열 수 없는 구조
- EPUB·PDF는 실행 가능한 스크립트·액션의 위험도 분석을 가장 깊게 한다.
- TXT는 UTF-8이 아니면 모두 문제로 판정한다.
- 인코딩 손상(mojibake) 판정을 포함한다. 판정 대상은 TXT뿐 아니라 EPUB·PDF에서 추출한 본문이다.
- 공통 모듈은 `utils/` 밖에, 프로그램은 `utils/` 아래에 하나만 둔다.
- 기존 세 도구가 잡던 것은 모두 계속 잡는다.

**비목표**

- 파일을 고치거나 격리하지 않는다. 진단만 한다. 격리는 `quarantine_broken`이 한다.
- 문서를 렌더링하거나 스크립트를 실행하지 않는다.
- EPUB 규격 전체 검증(epubcheck 수준)은 하지 않는다. 백엔드가 이미 `epubveri`를 쓴다.
- DOC, HWP, RTF, HTML 등 다른 형식은 이번 범위가 아니다.

**반목표** (기술적으로 동작해도 실패로 보는 경우)

- 정상 파일에 `High` 이상이 대량으로 붙어서 실제 공격이 묻히는 경우. 기존 `Severity` 설계 원칙(`Critical`은 공격 수법, `High`는 실행 가능 구조)을 지킨다.
- 파서가 panic이나 OOM으로 죽어서 수천 건 스캔이 중단되는 경우.
- 확인하지 못한 부분을 "문제 없음"으로 보고하는 경우. 못 본 곳은 `incomplete`로 알린다.

## 3. 사용자가 정한 것

| 결정               | 내용                                                          |
| ------------------ | ------------------------------------------------------------- |
| 이름               | `bookdoctor` (`utils/bookdoctor`)                             |
| 공통 모듈 위치     | `utils/` 밖. 기존 `libs/scan_common`을 확장한다               |
| utils 구성         | 진단 프로그램 하나                                            |
| 기존 도구          | `pdfscan`, `epubscan`, `detect_mojibake` 코드를 모두 제거한다 |
| PDF·EPUB 구조 검사 | `lopdf`, Poppler를 계속 쓴다                                  |
| UTF-8 BOM          | 있어도 없어도 정상이다                                        |
| TXT 인코딩         | UTF-8이 아니면 모두 문제다                                    |
| detect_mojibake    | 참고만 하고 기능은 bookdoctor에 넣는다                        |

## 4. 코드베이스에서 확인한 사실

- 백엔드는 TXT를 원본 바이트 그대로 내려준다(`get_book_content`는 `BookContent::File`, 미디어 타입은 확장자로 결정). 프론트 `ViewTXT.jsx`가 응답을 문자열로 받아 `\n`으로 나눈다. UTF-8이 아닌 TXT는 브라우저에서 깨진다. 이것이 "UTF-8 아니면 문제" 기준의 근거다.
- EPUB 뷰어(`ViewEPUB.jsx`)는 접두사 요소를 벗기고 DOMPurify로 정제한다. `head`가 없으면 "EPUB 본문에 head가 없습니다"로 렌더링을 거부한다. CSP로 스크립트·하위 문서를 막지만 `allowScriptedContent: true`다.
- PDF 뷰어는 pdf.js legacy 빌드다. 암호화 PDF는 비밀번호 오류로 처리한다. 백엔드는 Poppler(`pdftoppm`, `pdftotext`)를 쓰고 `libs/sysdeps::poppler`에 래퍼가 있다.
- 프론트에 `EpubDiagnose.js`, `PdfDiagnose.js`(브라우저 진단)가 있다. 이것은 UI 기능이므로 제거하지 않는다. 규칙만 Rust로 옮긴다.
- `epubscan/structure.rs`는 접두사 붙은 루트(`prefixed_root`)를 "뷰어 정제기가 거부"한다고 쓴다. 지금 `ViewEPUB.jsx`는 접두사를 벗겨서 처리한다. 이 항목의 심각도는 다시 정해야 한다(§6 참고).
- `quarantine_broken`이 `detect_mojibake`의 `DEFAULT_LIBRARY_ROOT`, `VERDICT_CORRUPTED`, `thousands`와 JSON 리포트에 의존한다.
- `detect_mojibake`는 판정 로직(`classify_text`, `inspect_file`)만 갖고 있고 본문 추출과 `korean_likeness`는 `corpus_common::reader`에 있다.
- `docs/rust-migration-progress.md`에 "`pdfscan`·`epubscan`은 다른 세션이 맡는다"는 기록이 있다. 구현 시작 전에 해당 세션이 끝났는지 확인해야 한다.

## 5. 구조

```
libs/scan_common/        공통: Finding·Severity·Report 모델, 매직바이트 판별, 파일 수집, 스트리밍·워커, 출력 정리
utils/bookdoctor/        프로그램 하나
  src/main.rs            진입점, 인자 없으면 print_usage()
  src/cli.rs             옵션 해석, 종료 코드
  src/epub.rs            EPUB 보안·구조·손상·렌더링 검사
  src/pdf.rs             PDF 보안·구조·렌더링 검사 (lopdf, Poppler)
  src/txt.rs             TXT 검사
  src/mojibake.rs        손상 판정 (classify_text 이식)
tests/rust/bookdoctor/   테스트
```

내 판단: 포맷별 검사기는 `utils/bookdoctor`에 두고, 포맷과 무관한 것만 `libs/scan_common`에 둔다. `scan_common`의 현재 원칙("포맷 해석 규칙은 각 도구에 둔다")과 같다. 본문 추출, `korean_likeness`, `DEFAULT_LIBRARY_ROOT`는 `corpus_common`을 그대로 쓰고 복사하지 않는다.

## 6. 검사 목록

심각도는 `scan_common::Severity`(Low, Medium, High, Critical)를 쓴다. "기존"은 지금 도구가 하는 검사, "신규"는 새로 만드는 검사다.

### 6.1 공통

| 항목                                                                                 | 분류   | 상태                         |
| ------------------------------------------------------------------------------------ | ------ | ---------------------------- |
| 0바이트, 읽기 불가, 크기 상한 초과                                                   | defect | 신규 (상한 옵션만 기존)      |
| 확장자와 매직바이트 불일치                                                           | defect | 신규 (EPUB 내부 멤버만 기존) |
| 중단된 다운로드 (끝부분이 0으로 채워진 sparse 파일)                                  | defect | 신규                         |
| 파일명 제어문자, 길이 255 초과, Windows 예약명                                       | defect | 신규 (EPUB 내부 이름만 기존) |
| 인코딩 손상 (`korean_likeness`가 임계값 0.10 미만, 한글 비율 10% 이상인 문서만 판정) | defect | 이식                         |

### 6.2 EPUB

| 영역               | 항목                                                                                                                                                                                  | 분류     | 상태 |
| ------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------- | ---- |
| ZIP 공격           | 경로 이탈, 절대 경로, 역슬래시, 심볼릭 링크, NTFS 스트림, 이름 충돌, 로컬 헤더 불일치, 압축 폭탄                                                                                      | security | 기존 |
| 실행 파일          | 실행형 확장자, ELF·Mach-O·Java class, shebang, 위장 바이너리, 중첩 ZIP, 미디어타입 불일치                                                                                             | security | 기존 |
| 활성 콘텐츠        | `script`, 이벤트 속성, `javascript:` URL, `object`·`embed`·`applet`, 폼 전송, `meta refresh`, `base href`, SVG `foreignObject`·`animate`                                              | security | 기존 |
| 외부 참조          | 원격 리소스·스크립트, UNC 경로, 위험 URI 스킴, OPF `scripted`·`remote-resources` 선언 누락                                                                                            | security | 기존 |
| XML 공격           | XXE, 엔티티 선언, 외부 DTD, XInclude, `xml-stylesheet`                                                                                                                                | security | 기존 |
| 구조               | `mimetype` 위치·압축, `container.xml`·OPF 누락·파싱 실패, spine 멤버 누락, 접두사 붙은 루트                                                                                           | render   | 기존 |
| 구조 (프론트 이식) | 미선언 네임스페이스 접두사, manifest·spine 비어 있음, spine idref가 manifest에 없음, NCX·nav·guide 참조 파일 누락, XHTML 파싱 실패, `dc:title`·`dc:language` 누락, 본문에 `head` 없음 | render   | 신규 |
| 손상               | 멤버별 CRC 불일치, 멤버 내부 바이트 유실(Bad magic number 유형)                                                                                                                       | defect   | 신규 |
| 렌더링             | 500KB 초과 폰트, 난독화 폰트 징후(숨김 요소·RTL 오버라이드·제로폭 문자)                                                                                                               | render   | 신규 |

### 6.3 PDF

| 영역             | 항목                                                                                                         | 분류     | 상태                    |
| ---------------- | ------------------------------------------------------------------------------------------------------------ | -------- | ----------------------- |
| 실행             | Launch, JavaScript 액션·이름 트리, 익스플로잇 패턴, 플랫폼 실행 파라미터, RichMediaExecute                   | security | 기존                    |
| 자동 실행        | OpenAction, 추가 액션(AA)                                                                                    | security | 기존                    |
| 내장물           | 임베디드 파일, 실행형 파일명, FileAttachment, RichMedia, XFA, 3D, Screen                                     | security | 기존                    |
| 외부 참조        | URI 액션                                                                                                     | security | 기존                    |
| 외부 참조 (확장) | GoToR·GoToE, SubmitForm, ImportData                                                                          | security | 신규                    |
| 은닉             | 이름의 `#hex` 난독화, 과다 필터 체인, 증분 업데이트 다수, 임베디드 파일 내부 매직바이트                      | security | 신규                    |
| 구조             | `%PDF-` 헤더 없음, `%%EOF` 없음·잘림, xref·trailer 손상, `/Root` 없음, 페이지 0개                            | defect   | 신규                    |
| 렌더링           | 암호화(`/Encrypt`)로 열람 불가, 쪽 크기 비정상(0.09pt면 뷰어가 백지), 쪽수 상한 초과, Poppler가 못 여는 파일 | render   | 신규                    |
| 리소스           | 입력 크기 대비 객체 수 과다                                                                                  | defect   | 기존 (로딩 훅으로 억제) |
| 콘텐츠           | 텍스트층 없음(스캔본), 폰트 미내장                                                                           | defect   | 신규 (Low)              |

### 6.4 TXT (전부 신규)

| 항목                                                                                  | 분류     | 심각도 |
| ------------------------------------------------------------------------------------- | -------- | ------ |
| UTF-8이 아님 (CP949, EUC-KR, UTF-16 LE/BE, Latin-1 등). 판정한 인코딩 이름을 기록한다 | defect   | High   |
| 유효하지 않은 UTF-8 바이트 (첫 위치 오프셋 기록)                                      | defect   | High   |
| 대체 문자 U+FFFD 포함                                                                 | defect   | Medium |
| NUL 바이트, 과다 제어문자 (바이너리 위장)                                             | defect   | High   |
| 양방향 제어문자(U+202E 등), 제로폭 문자                                               | security | Medium |
| HTML·스크립트 태그 포함                                                               | security | Medium |
| 로더 크기 상한(`TEXT_SIZE`) 초과                                                      | render   | Medium |
| 개행 혼재, 줄바꿈 없는 초장문 한 줄                                                   | render   | Low    |
| 빈 파일, 공백만 있는 파일                                                             | defect   | Medium |

UTF-8 BOM(`EF BB BF`)은 어떤 항목에도 걸리지 않는다. UTF-16 BOM은 UTF-8이 아니므로 "UTF-8이 아님"에 걸린다.

## 7. 리포트와 CLI

**발견 항목 모양** (내 제안)

```
path, format(epub|pdf|txt), category(security|defect|render), severity, code,
location, detail, viewer_blocked(blocked|not_blocked|n/a)
```

`viewer_blocked`는 우리 뷰어가 그 위험을 막는지 알려 준다. 예를 들어 EPUB의 `<script>`는 `High`여도 우리 뷰어에서는 CSP와 DOMPurify가 막으므로 `blocked`다. 심각도는 낮추지 않는다. 다른 리더에서는 여전히 위험하기 때문이다.

**CLI** (내 제안, 세부는 구현 재량)

```
bookdoctor [-r] [--json] [--report PATH] [--workers N] [--limit N]
           [--max-total-mb N] [--only security,defect,render] PATH...
```

- 인자 없이 실행하면 `print_usage()`로 용도와 사용법을 출력한다.
- `--report PATH`는 JSON 파일을 쓴다. `quarantine_broken`이 이 파일을 읽는다.
- 종료 코드는 기존 `pdfscan` 규칙을 따른다. 0은 발견 없음, 1은 발견 있음, 2는 읽지 못한 부분이 있음(발견이 없을 때).
- 기본 워커 수는 기존 `DEFAULT_WORKERS`다. 코퍼스가 있는 `/mnt/data`는 회전 디스크라 워커 2개가 이득이고 8개는 손해임을 도움말에 적는다.

## 8. 제거와 이전

1. `bookdoctor`를 먼저 만들고 기존 테스트를 `tests/rust/bookdoctor/`로 옮겨 통과시킨다.
2. `quarantine_broken`이 `bookdoctor` 리포트를 읽도록 바꾼다. `DEFAULT_LIBRARY_ROOT`, `thousands`는 `corpus_common`에서 직접 가져오고, `VERDICT_CORRUPTED`에 해당하는 값은 새 리포트의 코드로 대체한다.
3. 동등성을 확인하면 아래 "제거 대상"을 모두 삭제한다. 삭제는 사용자가 개발 완료 조건으로 지시했으므로(2026-10-29) 별도 승인을 다시 받지 않는다. 단, 삭제 직전에 §9의 기준 수치와 기존 리포트를 비교용으로 보존했는지 확인한다.
4. 함께 고칠 곳: 루트 `Cargo.toml` members와 `panic = "unwind"` 주석, `Cargo.lock`, `.github/workflows/backend-ci.yml`의 제외 규칙, `docs/rust-workspace.md`의 도구 표, `docs/rust-migration-progress.md`에 기록.

**제거 대상** (개발이 끝났다고 말하려면 아래가 모두 사라져 있어야 한다)

| 구분 | 대상 |
|---|---|
| 프로젝트 | `utils/pdfscan`, `utils/epubscan`, `utils/detect_mojibake` |
| 테스트 | `tests/rust/pdfscan`, `tests/rust/epubscan`, `tests/rust/detect_mojibake` |
| 빌드 산출물 | `target/` 아래 세 도구의 바이너리, `.d`, `.fingerprint`, `incremental`, `deps` (`cargo clean -p <crate>`로 지운다. `target/` 전체는 18GB라 건드리지 않는다) |
| 실행 산출물 | `tmp/epub_structure_report*.json`(현재 74개와 `mojibake` 리포트). 기준 수치를 따로 보존한 뒤 삭제한다 |
| 참조 | workspace members, `Cargo.lock` 항목, CI 제외 규칙, 문서의 도구 표, `quarantine_broken`의 의존성·도움말 문구 |

검증: 삭제 후 `grep -rnE "pdfscan|epubscan|detect_mojibake"`가 저장소(`target/`, `.git/`, 이력을 적는 `docs/rust-migration-progress.md` 제외)에서 0건이고, `cargo test --workspace`와 `cargo clippy --workspace --all-targets`가 통과한다.

## 9. 검증 기준

- 기존 도구 출력과의 동등성(코퍼스 표본): EPUB 구조는 `tmp/epub_structure_report*.json`의 flagged 목록을 `epubscan structure`와, EPUB·PDF 보안은 무작위 표본을 `epubscan scan`·`pdfscan`과, 인코딩 손상은 표본을 `detect_mojibake`와 비교한다. 옛 도구가 잡은 항목은 대응 코드로 모두 잡혀야 한다.
- 손상 기준(새 검사의 재현율): EPUB 손상 58건(63,282건 중)과 PDF 손상 34건(21,150건 중)은 기존 세 도구의 출력이 아니라 2026-08의 별도 전수 스캔(tail 검사·pypdfium2) 결과다. 새 도구가 남아 있는 목록(`/home/terzeron/tmp/redownload_list.txt` PDF 26건, `tmp/epub_redownload_20261022.txt` EPUB 5건)의 파일을 `defect` 또는 `incomplete`로 모두 잡는지 확인한다. 인코딩 손상 255건은 `0_broken`이 마운트되어 있을 때만 비교한다.
- 기존 세 도구의 테스트가 모두 이식되어 통과한다. 이식하지 못한 테스트가 있으면 이유를 적는다.
- 새 검사(§6의 "신규")는 테스트를 먼저 쓴다. 커버리지는 100%에 가깝게 유지한다(프로젝트 규칙).
- `cargo test --workspace`와 `cargo clippy --workspace --all-targets`가 통과한다.
- 정상 파일 표본에서 `High` 이상 오탐 비율을 측정해서 보고한다. 반목표 확인용이다.

## 10. 위험

- 공유 작업 트리에서 다른 세션이 같은 crate를 건드릴 수 있다. 격리된 worktree에서 개발한다.
- `cargo test` 일부가 셸 환경변수 때문에 실서비스에 붙을 수 있다. 테스트 전에 `TM_MYSQL_HOST`, `TM_ES_URL`을 확인한다.
- PDF 파서 자체가 공격 표면이다. 기존처럼 `catch_unwind`로 파서 panic을 격리하고 메모리 상한을 지킨다.
- 확장자 판별과 매직바이트 판별이 어긋난 파일을 어느 쪽 규칙으로 읽을지 정해야 한다. 기본은 매직바이트이고, 어긋남 자체를 `defect`로 보고한다.

## 11. 구현자 재량

- 검사 코드 이름(`code`) 체계와 JSON 필드 세부.
- PDF 쪽수 상한, EPUB 멤버 크기 상한, TXT 초장문 줄 길이 같은 임계값. 단, 백엔드·프론트 코드에서 근거를 찾아 적는다.
- `prefixed_root` 심각도. 프론트가 접두사를 벗기므로 백엔드 미리보기(`book_manager/epub.rs`)도 같은지 확인한 뒤 낮출지 정한다. 기본 제안은 Low다.
- 바이너리 이름 외 하위 모듈 분할, CLI 옵션 철자.
- `backend-ci.yml`이 두 스캐너를 제외하던 이유를 확인하고 `bookdoctor`에 같은 제외가 필요한지 정한다.

## 12. 구현 검토에서 확정한 것

구현 담당자 검토에서 나온 질문에 답한 내용이다. "코드 확인"은 프로젝트 코드에서 찾은 답이고, "내 판단"은 사용자가 정하지 않았으므로 바꿔도 된다.

| # | 질문 | 답 | 근거 |
|---|---|---|---|
| 1 | `quarantine_broken`은 새 리포트의 무엇을 격리 기준으로 쓰는가 | 인코딩 손상 판정(`defect`, 코드 `MOJIBAKE`)만 격리한다. 동작은 지금과 같다. EPUB·PDF 손상, 암호화 PDF, 0바이트 파일은 격리하지 않는다. 옛 리포트 형식은 한 번에 버리고 새 형식으로 바꾼다. `fix-ext`도 새 리포트를 읽는다 | 코드 확인: `quarantine_broken` 도움말이 "손상 판정만 골라" 옮긴다고 명시한다 |
| 2 | 옛 도구와 결과가 정확히 같아야 하는가 | 상위집합을 허용한다. 옛 도구가 잡은 파일은 같은 이슈 코드로 모두 잡아야 한다. 새로 더 잡은 파일은 건수와 표본을 따로 보고하고, 격리 기준(1번)에는 영향을 주지 않는다 | 내 판단 |
| 3 | 기준 입력은 무엇인가 | 보존한 옛 리포트(`tmp/epub_structure_report*.json`, mojibake 리포트)와 `0_broken` 디렉토리다. 별도 스냅샷은 없다. 구현자는 옛 리포트의 경로 목록에서 입력 목록을 만든다 | 내 판단 |
| 4 | mojibake 본문 추출 비용 | TXT·EPUB는 기본으로 켠다(프로세스 안에서 읽어 싸다). PDF 본문 추출은 Poppler를 부르므로 `--pdf-text` 옵션으로 켠다. 이 옵션이 "텍스트층 없음" 검사와 추출을 공유한다. 추출 실패는 `defect`가 아니라 `incomplete`로 보고한다 | 내 판단 (PDF 21,150건을 회전 디스크에서 돌리는 비용 때문) |
| 5 | `incomplete`의 계약 | 발견 항목이 아니라 파일별 별도 필드(`incomplete: true`, `reasons`)다. 종료 코드는 발견이 하나라도 있으면 1, 없고 `incomplete`가 있으면 2, 둘 다 없으면 0이다 | 내 판단 (기존 `pdfscan` 규칙과 같다) |
| 6 | `viewer_blocked`는 정적 표인가 | 코드별 정적 표다. 파일을 따로 분석하지 않는다. 프론트가 바뀌면 표를 고쳐야 하므로, 표 옆에 근거 파일(`ViewEPUB.jsx` 등)을 주석으로 적는다 | 내 판단 |
| 7 | 다른 세션이 `pdfscan`·`epubscan`을 아직 맡고 있는가 | 사용자 확인이 필요하다 | 확인한 것: 작업 트리는 이 문서 외에 깨끗하고, 두 crate의 마지막 커밋은 2026-10-27이다 |
| 8 | 프론트 `EpubDiagnose.js`·`PdfDiagnose.js`와 Rust 규칙의 동기화 | 한 번만 이식한다. 동기화 테스트는 만들지 않는다. JS 원본은 유지한다. 두 곳이 어긋날 수 있다는 위험은 인정한다 | 내 판단 |

TXT의 "UTF-8 아님"이 `High`라서 비UTF-8 파일이 많으면 `High`가 대량으로 나온다. 사용자가 정한 기준이므로 그대로 따르고, §9의 오탐 비율 측정에서 이 항목을 따로 센다.
