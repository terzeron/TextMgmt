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
| 6   | 난독화·인코딩           | 부분 | `OBFUSCATION`, `OBFUSCATED_LOADER`, `OBFUSCATED_TEXT`, `hex-obfuscated action name`                                                  | 엔트로피는 P4                                                |
| 7   | 압축 공격               | 부분 | `HIGH_COMPRESSION_RATIO`, `ZIP_PATH_TRAVERSAL`, `NESTED_ARCHIVE`, `long filter chain`                                                | 일반 ZIP, 중첩 깊이 상한은 P1                                |
| 8   | 파서 취약점 악용        | 부분 | `XREF_BROKEN`, `nesting limit exceeded`, `MALFORMED_MARKUP`                                                                          | OLE·HWP 레코드 검증은 P2·P3                                  |
| 9   | 내장 리소스 재귀 검사   | 없음 | -                                                                                                                                    | P4 (현재는 `NESTED_ARCHIVE`·`embedded file`로 존재만 알린다) |
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

## 새 포맷(ZIP, DOCX, HWPX, DOC, HWP, HTML)

검사기가 아직 없다. 담당은 설계서의 P1~P4다. 이 문서의 "없음"·"부분" 행이 각 단계 계획의 입력이다.

근거 코드 검증: 문서의 백틱 항목 64개 중 58개가 소스의 지표 문자열과 일치한다. 나머지 6개는 코드가 아니라 경로·용어(`AcroForm`, `encryption.xml`은 "없음" 행)다.
