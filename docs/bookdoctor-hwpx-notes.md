# HWPX 구조 메모 (P1 Task 5)

HWPX 검사기를 구현하기 전에 확인한 내용이다. 공식 명세(KS X 6101, 한컴 OWPML 문서)는 찾지 못했다. 아래는 개발자 문서와 오픈소스 구현의 설명이다.

## 확인한 것 (2차 자료)

| 항목 | 내용 | 근거 |
|---|---|---|
| 컨테이너 | ZIP 안에 XML 문서가 든다. OWPML(KS X 6101)을 구현한다 | [unhwp container.rs](https://docs.rs/crate/unhwp/0.5.1/source/src/hwpx/container.rs), [HwpForge README](https://docs.rs/crate/hwpforge/0.1.3/source/README.md) |
| `mimetype` | 루트에 두고 첫 멤버·무압축이 관례다. 파서 중에는 `hwp` 또는 `owpml`이 들어 있는지만 보고, 없어도 허용하는 것이 있다 | unhwp container.rs |
| `version.xml` | OWPML 버전과 저장 환경. 루트 요소는 `HCFVersion`이다 | 같은 출처 |
| `Contents/content.hpf` | 패키지 매니페스트(OPC 방식, metadata·manifest·spine). 파서는 spine에서 섹션 순서를 읽는다 | 같은 출처 |
| `Contents/section0.xml` | 본문과 표. 매니페스트 없이 `section*.xml`을 훑는 구현도 있다 | 같은 출처 |
| `Contents/header.xml` | 글자 모양·문단 모양 같은 스타일 | 같은 출처 |
| `BinData/` | 내장 이미지와 OLE 객체 | 같은 출처 |
| HWP 5.0과의 차이 | HWP 5.0은 CFB(OLE) 복합 파일이고 HWPX는 ZIP이다. 이름만 바꾼 파일이 서로 위장할 수 있다 | [HOP 소개 글](https://braindetox.kr/en/posts/hwp_open_source_editor.html) |

## 확인하지 못한 것

- **스크립트·매크로가 저장되는 위치와 이름(`Scripts/` 등)**: 어느 자료에서도 확인하지 못했다. 그래서 P1의 HWPX 검사기는 **스크립트 탐지를 구현하지 않는다**. 명세 원문이나 실제 스크립트가 든 `.hwpx` 파일을 얻으면 별도 태스크로 추가한다.
- 필수 부분의 전체 목록(`META-INF/container.xml`이 필수인지 포함).
- `content.hpf`의 `href`가 루트 기준인지 `Contents/` 기준인지. 코드는 두 가지 모두 받아들인다.

## 이 구조를 쓰는 곳

- 테스트 fixture(`common::minimal_hwpx_with`)는 위 설명에서 만든 합성 파일이다. 실제 한글 프로그램이 만든 HWPX로 검증하지 못했다. 코퍼스에서 `.hwpx`를 찾은 뒤 확인한다(Task 5의 소음 게이트).
