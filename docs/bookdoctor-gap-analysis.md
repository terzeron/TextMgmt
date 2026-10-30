# bookdoctor 갭 분석

기준: 설계서 `docs/superpowers/specs/2026-10-30-bookdoctor-malware-scanner-design.md`의 공통 검출 12개와 PDF·EPUB 포맷별 항목.
근거는 `utils/bookdoctor/src/**`의 지표 코드 문자열이다. 코드를 읽어 확인하지 못한 것은 "미확인"으로 적는다.

## 공통 검출 12개

| #   | 항목                    | 상태 | 근거 코드                                                                                                                            | 없을 때 담당 단계                                            |
| --- | ----------------------- | ---- | ------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------ |
| 1   | 파일 형식·구조 검증     | 있음 | `FORMAT_MISMATCH`, `DISGUISED_EXECUTABLE`, `SPARSE_FILE`, `ZERO_TAIL`, `NO_PDF_HEADER`, `XREF_BROKEN`, `NOT_ZIP`, `BAD_LOCAL_HEADER` | 새 포맷은 P1~P3                                              |
| 2   | 악성코드 시그니처(YARA) | 없음 | -                                                                                                                                    | P5                                                           |
| 3   | 스크립트·실행 코드      | 부분 | `SCRIPT_TAG`, `EVENT_HANDLER_ATTR`, `JS_URL`, `DYNAMIC_CODE_EXEC`, `HOST_COMMAND_EXEC`, `JavaScript action`                          | VBA·ActiveX(OLE)는 P2, DOCX 매크로는 P1                      |
| 4   | 외부 리소스             | 있음 | `REMOTE_RESOURCE`, `REMOTE_SCRIPT`, `REMOTE_CSS_IMPORT`, `URI action`, `GoToR action`, `SubmitForm action`                           | DOCX 외부 관계는 P1                                          |
| 5   | 내장 실행파일           | 부분 | `EMBEDDED_EXECUTABLE`, `PE_EXECUTABLE_CONTENT`, `ELF_BINARY_CONTENT`, `embedded executable content`                                  | DOCX·DOC·HWP는 P1~P3                                         |
| 6   | 난독화·인코딩           | 부분 | `OBFUSCATION`, `OBFUSCATED_LOADER`, `OBFUSCATED_TEXT`, `hex-obfuscated action name`                                                  | 엔트로피(`HIGH_ENTROPY_TEXT_MEMBER`)는 ZIP 계열의 텍스트 이름 멤버 앞 4KiB만 본다(P4) |
| 7   | 압축 공격               | 부분 | `HIGH_COMPRESSION_RATIO`, `ZIP_PATH_TRAVERSAL`, `NESTED_ARCHIVE`, `long filter chain`                                                | 일반 ZIP, 중첩 깊이 상한은 P1                                |
| 8   | 파서 취약점 악용        | 부분 | `XREF_BROKEN`, `nesting limit exceeded`, `MALFORMED_MARKUP`                                                                          | OLE·HWP 레코드 검증은 P2·P3                                  |
| 9   | 내장 리소스 재귀 검사   | 없음 | -                                                                                                                                    | 부분: ZIP·DOCX·HWPX·EPUB 안의 ZIP·PDF·RTF·Word·HWP OLE를 깊이 3·파일 64개·`--max-total-mb`까지 재귀(`--no-recurse`로 끔). 내장 OLE 패키지(`Ole10Native`)·HWP `BinData` 안의 컨테이너는 재귀하지 않는다 |
| 10  | 복잡한 문서 구조        | 부분 | `nesting limit exceeded`, `many incremental updates`, `FONT_TOO_LARGE`                                                               | 순환 참조·과도한 객체 수는 미확인                            |
| 11  | 피싱·소셜 엔지니어링    | 부분 | `DANGEROUS_URI_SCHEME`, `FILENAME_CONTROL_CHAR`, `BIDI_CONTROL`                                                                      | 실행 유도·위장 콘텐츠의 범위 정의가 먼저 필요                |
| 12  | 위험도 평가             | 부분 | (심각도 4단계만 있음)                                                                                                                | 점수는 P0 Task 5, 신뢰도는 P1 이후                           |

## PDF

| 항목                                                | 상태   | 근거 코드                                                                                                                          | 비고                                                   |
| --------------------------------------------------- | ------ | ---------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------ |
| JavaScript, OpenAction, AA, Launch, URI, SubmitForm | 있음   | `JavaScript action`, `OpenAction (runs on open)`, `additional-actions trigger`, `Launch action`, `URI action`, `SubmitForm action` |                                                        |
| EmbeddedFile, RichMedia, XFA                        | 있음   | `embedded file`, `RichMedia content`, `XFA form`                                                                                   |                                                        |
| AcroForm 분석                                       | 없음   | -                                                                                                                                  | `AcroForm` 문자열이 소스에 없다. XFA만 본다            |
| ObjStm·압축 스트림 내부 객체                        | 미확인 | -                                                                                                                                  | lopdf가 풀어 주는 객체를 보는지 코드로 확인하지 않았다 |
| 증분 업데이트로 숨긴 이전 객체                      | 부분   | `many incremental updates`                                                                                                         | 개수만 센다. 이전 객체 내용 검사는 없다                |
| 암호화·난독화 객체, 비정상 xref                     | 부분   | `ENCRYPTED`, `hex-obfuscated action name`, `XREF_BROKEN`                                                                           | 암호화 문서는 `incomplete`로만 표시한다                |

## EPUB

| 항목                                         | 상태   | 근거 코드                                                                                    |
| -------------------------------------------- | ------ | -------------------------------------------------------------------------------------------- |
| script, iframe, object, embed, 이벤트 핸들러 | 있음   | `SCRIPT_TAG`, `ACTIVE_ELEMENT`, `EVENT_HANDLER_ATTR`                                         |
| 외부 리소스·원격 스크립트                    | 있음   | `REMOTE_RESOURCE`, `REMOTE_SCRIPT`, `REMOTE_MANIFEST_ITEM`                                   |
| SVG 스크립트·XML 공격                        | 부분   | `SVG_FOREIGN_OBJECT`, `SVG_ANIMATE_HREF`, `XXE_ENTITY`, `XINCLUDE`, `EXTERNAL_DTD`           |
| OPF·NCX·Manifest·Spine 무결성                | 있음   | `OPF_MISSING`, `OPF_UNPARSABLE`, `NCX_REF_MISSING`, `SPINE_MEMBER_MISSING`, `MANIFEST_EMPTY` |
| ZIP Bomb, Zip Slip, 중첩                     | 있음   | `HIGH_COMPRESSION_RATIO`, `ZIP_PATH_TRAVERSAL`, `NESTED_ARCHIVE`                             |
| SVG 안의 `<script>`                          | 미확인 | -                                                                                            |
| `encryption.xml` DRM과 폰트 난독화 구분      | 없음   | P1에서 알고리즘 URI로 구분                                                                   |

## 새 포맷 (P1 이후 상태)

| 포맷 | 상태 | 있는 검사 | 아직 없는 것 |
|---|---|---|---|
| ZIP | 있음 | 열기, 멤버별 앞 4096바이트 해제 시험(`--zip-deep`이면 끝까지 CRC), `ZIP_PATH_TRAVERSAL` 등 이름 규칙, 로컬 헤더 일치, 압축비, 선언 크기 위조(`ZIP_SIZE_MISMATCH`), 내장 실행 파일, 중첩 ZIP 표시, 암호화 멤버(미완료) | 엔트로피는 텍스트 이름 멤버만, 재귀는 위 행(P4) |
| DOCX 계열 | 있음 | 필수 부분·XML·선언 부분, 확장자 위장, 원격 템플릿·외부 객체·UNC, `vbaProject.bin`(형식별 등급), OLE·ActiveX·내장 패키지, DDE·INCLUDE 필드(run 분할·중첩), DTD·엔티티, UTF-16 | 내장 OLE 패키지 재귀. 암호화 OOXML(OLE `EncryptedPackage`)은 cyan 미완료로, 이름만 바꾼 구형 DOC는 DOC 분석기로 넘긴다(P2) |
| HWPX | 부분 | mimetype, 매니페스트, 섹션, XML, `BinData` OLE·실행 파일, DTD·엔티티 | 스크립트·매크로(저장 위치 미확인, `docs/bookdoctor-hwpx-notes.md`) |
| DOC | 부분 | `ole_reader`(관대한 CFB 파서, MS-OVBA), OLE 구조 이상(`OLE_*`), FIB 암호화(cyan), VBA 매크로·자동 실행·의심 키워드 조합, `ObjectPool`·`Ole10Native` 내장 실행 파일(레이블·원본·임시 경로), DDE·INCLUDE 필드(중첩·8비트·UTF-16), 이름만 바꾼 `.docx`·`.rtf` 위장 | 실제 DOC로 검증하지 못했다(합성·LibreOffice 생성 3건뿐). 표 스트림 기반 텍스트, 수식 편집기 CVE 패턴은 없다 |
| RTF | 부분 | `{\rtf` 판별, `\objdata`·`\objclass` 등 내장 객체(쪼갠 제어어 포함), 본문의 OLE·PE 매직, `\datastore`(단독은 Medium) | 코퍼스 `.rtf` 11건은 모두 정상 RTF였고 악성 양성 대조군은 없다 |
| HWP 5.x | 부분 | `FileHeader`(암호·DRM·공인 인증서 cyan 미완료, 배포용), `Scripts`(High, 의심 키워드 Critical, 압축·표 뒤 바이트·모든 스트림), `BinData` 내용 매직(실행 파일 High, OLE High, EPS Medium), `DocInfo`의 연결 파일(UNC·URL High), 레코드 구조(`HWP_BAD_RECORD`), OLE 구조 이상, 이름만 바꾼 Word 문서·`.hwpx` 처리 | 실제 HWP5는 EPUB 안에서 1건만 발견해 확인했다(압축 형식=헤더 없는 raw deflate, `Scripts` 스트림도 압축·꼬리 8바이트, 한글은 빈 `Scripts`를 항상 씀). 나머지는 합성 fixture와 한컴 명세로만 확인했고 코퍼스 `.hwp` OLE 2건은 구조 이상이다. 하이퍼링크 필드(`%hlk`), `ViewText`(배포용 본문), 문서 이력, 본문 텍스트 속 개체는 검사하지 않는다. 압축 형식(zlib 헤더 유무)은 두 경로를 모두 받지만 실파일 확인은 못 했다 |
| HWP 3.x 이하 | 최소 | `HWP Document File V` 서명 판별과 이름 위장 표시 | 서명 밖의 내용은 검사하지 않는다(코퍼스 `.hwp` 200건 중 198건이 이 형식이라 미완료로 표시하지 않았다) |
| HTML | 있음 | EPUB의 HTML 규칙 재사용(스크립트, 이벤트 핸들러, 원격 스크립트, iframe, 폼, 난독화), 인코딩 우회(UTF-16·UTF-32·NUL), 이름 위장 | 코퍼스 `.html` 3,034건 중 High 이상 9건(0.3%, 모두 분석·광고 스크립트가 든 저장 페이지). 브라우저가 실제로 실행하는지는 판정하지 못한다 |
| 격리 실행 | 있음 | `--isolate`: 파일마다 하위 프로세스에서 `ulimit -v`(기본 4096MiB)와 시간 상한(기본 120초), 실패는 그 파일의 미완료 | PDF 로딩 메모리 상한(R20)을 완화한다. 자식 출력 크기에는 상한이 없다 |
| 규칙 엔진 | 있음(선택, 기본 꺼짐) | `--rules PATH`·`--builtin-rules`: YARA 문법의 부분집합(문자열·16진·nocase·wide, any/all/N of them, `#개수`, `at`, and/or/not)을 파일의 앞 16MiB 원시 바이트에 적용해 `RULE_MATCH`로 보고. 내장 규칙 7개(EICAR, PowerShell 인코딩·내려받기, LOLBin 원격 실행, base64·16진 PE 헤더, WScript.Shell). 문법·한도·YARA와의 차이는 `docs/bookdoctor-rules.md` | 압축된 내용(ZIP 멤버, PDF·OLE·HWP 스트림)은 못 본다. 코퍼스에 악성 파일이 없어 오탐만 측정했다(3,660개 파일에서 0건). YARA-X는 쓰지 않았다(182개 패키지, Rust 1.95 필요, 현재 Rust로 풀리는 1.19.0은 취약점 8건) |
| 실행 파일 | 있음 | 확장자 + 매직바이트 이중 확인(P0) | - |

실제 문서로 검증한 범위: DOCX는 코퍼스에 2건(악보)뿐이라 오탐률을 측정하지 못했고, HWPX는 0건이라 합성 fixture로만 확인했다. HWP 5.x는 코퍼스에 `.hwp` 정상 샘플이 없고 EPUB 안에서 1건(341KB)만 발견해, 그 1건과 한컴 명세(revision 1.3), 합성 fixture로 확인했다. DOC는 코퍼스에 0건이라 합성 fixture와 LibreOffice가 만든 3건으로만 확인했고, RTF는 11건이 모두 정상이었다(Word 표준 `\datastore` 오탐 1건을 이 과정에서 고쳤다).
