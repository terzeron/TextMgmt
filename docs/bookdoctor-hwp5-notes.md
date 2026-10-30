# HWP 5.x 구조 메모 (P3)

P3 구현 전에 한컴 공식 명세 "한글문서파일형식 5.0 revision 1.3" (2018-11-08, [PDF](https://cdn.hancom.com/link/docs/한글문서파일형식_5.0_revision1.3.pdf))를 `pdftotext`로 읽고 확인한 내용이다. 쪽 번호는 명세 본문 기준이다.

## 명세에서 확인한 것

| 항목                     | 내용                                                                                                                                                                                                                                                                          | 위치                |
| ------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------- |
| 컨테이너                 | 윈도우 복합 파일(CFB). 문자는 UTF-16LE                                                                                                                                                                                                                                        | 3장 서두            |
| 압축                     | "zlib.org의 zlib을 사용". 압축 대상은 `DocInfo`·`BodyText/SectionN`·`BinData/*`·`DocHistory/*`(표 2)                                                                                                                                                                          | 표 2, 서두          |
| 스토리지                 | `FileHeader`, `DocInfo`, `BodyText/Section0..`, `\005HwpSummaryInformation`, `BinData/BinaryData0..`, `PrvText`, `PrvImage`, `DocOptions/_LinkDoc·DrmLicense…`, `Scripts/DefaultJScript·JScriptVersion`, `XMLTemplate`, `DocHistory/VersionLog0..`                            | 표 2                |
| `FileHeader`             | 256바이트: 서명 32(`"HWP Document File"`), 버전 DWORD(`0xMMnnPPrr`), 속성1 DWORD, 속성2 DWORD, EncryptVersion DWORD, KOGL 국가 1, 예약 207                                                                                                                                    | 표 3                |
| 속성1 비트               | 0 압축, 1 암호 설정, 2 배포용, 3 스크립트 저장, 4 DRM 보안 문서, 5 XMLTemplate 존재, 6 문서 이력, 7 전자 서명, 8 공인 인증서 암호화, 9 전자 서명 예비, 10 공인 인증서 DRM, 11 CCL, 12 모바일 최적화, 13 개인 정보 보안, 14 변경 추적, 15 KOGL, 16 비디오 컨트롤, 17 차례 필드 | 표 3                |
| 레코드 헤더              | 32비트: TagID 10, Level 10, Size 12. Size가 모두 1(`0xFFF`)이면 뒤따르는 DWORD가 길이                                                                                                                                                                                         | 4.1                 |
| `HWPTAG_BEGIN`           | `0x010`. `HWPTAG_BIN_DATA`=BEGIN+2, `HWPTAG_SHAPE_COMPONENT_OLE`=BEGIN+68, `HWPTAG_CTRL_DATA`=BEGIN+71                                                                                                                                                                        | 표 13, 본문 태그 표 |
| `HWPTAG_BIN_DATA`        | 속성 UINT16(bit0–3 Type: 0 LINK, 1 EMBEDDING, 2 STORAGE(OLE); bit4–5 압축: 0 기본, 0x10 압축, 0x20 비압축), LINK면 절대·상대 경로(길이 WORD + UTF-16), EMBEDDING·STORAGE면 BinData 아이디(UINT16), EMBEDDING이면 확장자                                                       | 표 17, 18           |
| `Scripts/DefaultJScript` | DWORD 길이 + WCHAR[len]을 헤더·소스·Pre 소스·Post 소스 순으로, 끝에 DWORD `-1`                                                                                                                                                                                                | 표 9                |
| `DocOptions/_LinkDoc`    | 연결된 문서의 경로                                                                                                                                                                                                                                                            | 3.2.8               |

## 명세에 없는 것 (코드로 확정하지 못한 부분)

| 항목                             | 상태                                                                                                                                                                          |
| -------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 스트림 압축 형식(zlib 헤더 유무) | 명세는 zlib이라 쓰지만 오픈소스 구현은 헤더 없는 raw deflate로 읽는다. 둘 다 받도록 구현하고 합성 fixture로 두 경로를 확인한다                                                |
| `Scripts` 스트림의 압축 여부     | 표 2에서 `Scripts`는 압축 표시가 없다. 압축 여부를 가리지 않고 읽도록 한다                                                                                                    |
| 배포용 문서(`ViewText`)          | 이 명세에는 `ViewText` 스토리지 설명이 없다. 배포용 비트(bit2)만 알리고 본문은 읽지 않는다                                                                                    |
| EPS 등 알려진 exploit 패턴       | 명세에 EPS 언급이 없다. 설계서가 "미검증"으로 둔 항목이며 BinData 내용 매직(`%!PS-Adobe`, DOS EPS `C5D0D3C6`)으로만 탐지한다. 매직 값은 명세가 아니라 일반 파일 서명 지식이다 |
| 건강한 HWP5 실파일               | 코퍼스 `.hwp` 200건 중 OLE는 2건뿐이고 둘 다 구조 이상이다. 정상 샘플로 검증하지 못했다                                                                                       |
