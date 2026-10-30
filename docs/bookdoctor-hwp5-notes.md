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
| 스트림 압축 형식(zlib 헤더 유무) | **실파일로 확인(P4)**: 헤더 없는 raw deflate다(`DocInfo`가 `EC 56…`으로 시작). 명세는 zlib이라 쓰지만 실제는 raw다. 구현은 둘 다 받는다 |
| `Scripts` 스트림의 압축 여부 | **실파일로 확인(P4)**: 압축 비트가 서 있으면 `Scripts` 스트림도 raw deflate로 압축되고 뒤에 8바이트(adler32 + 풀린 길이 LE)가 붙는다. `DefaultJScript` 16바이트 = deflate 8 + 꼬리 8, `JScriptVersion` 13바이트도 같은 모양이다 |
| 배포용 문서(`ViewText`)          | 이 명세에는 `ViewText` 스토리지 설명이 없다. 배포용 비트(bit2)만 알리고 본문은 읽지 않는다                                                                                    |
| EPS 등 알려진 exploit 패턴       | 명세에 EPS 언급이 없다. 설계서가 "미검증"으로 둔 항목이며 BinData 내용 매직(`%!PS-Adobe`, DOS EPS `C5D0D3C6`)으로만 탐지한다. 매직 값은 명세가 아니라 일반 파일 서명 지식이다 |
| 건강한 HWP5 실파일 | 코퍼스 `.hwp` 200건 중 OLE는 2건뿐이고 둘 다 구조 이상이다. 대신 EPUB 안에서 실제 HWP5 1건(341KB, 버전 `5.1.0.1`, 속성 `0x1`=압축, EncryptVersion 4)을 발견해 확인했다. 그 한 건으로 본 것: 한글은 스크립트가 없어도 빈 `Scripts`(빈 표 + 꼬리)를 **항상** 쓰고 속성 비트 3은 서 있지 않다. 그래서 `Scripts` 저장소의 존재는 지표가 아니다 |

## 실제 HWP5 1건에서 확인한 구조 (P4, EPUB 안에서 발견)

| 스트림 | 크기 | 비고 |
|---|---|---|
| `FileHeader` | 256 | 버전 `0x05010001`, 속성 `0x1`, EncryptVersion 4 |
| `DocInfo` | 1,327 | raw deflate |
| `BodyText/Section0` | 252,983 | raw deflate, 레코드 검증 통과 |
| `HwpSummaryInformation` | 457 | 이름이 명세의 `\005HwpSummaryInformation`과 다르다(앞의 제어 문자가 없다) |
| `PrvText`, `PrvImage` | 2,044, 75,424 | |
| `Scripts/JScriptVersion`, `Scripts/DefaultJScript` | 13, 16 | 빈 스크립트(위 설명) |
| `DocOptions/_LinkDoc` | 524 | |
| `BinData` | 없음 | 그림이 없는 문서 |
