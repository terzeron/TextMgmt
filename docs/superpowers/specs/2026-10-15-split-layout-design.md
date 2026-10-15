# SplitLayout 설계: 두 컬럼 페이지 공통 레이아웃

상태: 설계 확정(사용자 승인), 구현 전. 구현은 다른 세션의 `BookCoverAdmin.*` 수정이 커밋된 뒤에 시작한다.

## 목표

- 왼쪽 디렉토리 / 오른쪽 작업 공간 페이지의 기본 동작을 통일한다.
- 두 컬럼이 서로 독립적으로 스크롤된다.
- 왼쪽 컬럼 너비를 마우스와 터치 드래그로 조절한다.

## 비목표

- 새 의존성 추가(react-resizable-panels 등). 직접 구현한다.
- 폰 세로 화면(768px 미만)의 좌우 조절. 지금처럼 위아래로 쌓고 핸들은 숨긴다.
- 책 보기(View)의 본문 영역을 오른쪽 pane에 넣는 변경. 상단 툴바 영역에만 적용한다.
- `CategoryAdmin`의 트리 코드 복제 정리(별도 작업).

## 현황 (조사 기준, 구현 시 재확인)

- 페이지마다 react-bootstrap `Row/Col`을 직접 쓴다. 공통 레이아웃 컴포넌트가 없다.
- 독립 스크롤은 `Edit.css`와 `CategoryAdmin.css`의 왼쪽 컬럼에만 있다(`calc(100vh - 80px)` + sticky). 오른쪽은 페이지 스크롤을 따른다.
- `App.css`의 `.row/.col-*`에 `margin:0 !important; padding:1px !important`가 있다.

## 컴포넌트

`src/SplitLayout.jsx`, `src/SplitLayout.css`

| prop                  | 설명                                                |
| --------------------- | --------------------------------------------------- |
| `left`, `right`       | 각 pane의 내용                                      |
| `storageKey`          | 페이지별 너비 저장 키                               |
| `defaultLeftRatio` | 저장된 너비가 없을 때 왼쪽 비율(기본 0.25 = 3:9) |
| `minLeft` / `maxLeft` | 기본 200px / 컨테이너의 60%                         |
| `leftCollapsed`       | true면 왼쪽 pane과 핸들을 숨김(책 편집의 접힘 상태) |

- CSS grid `왼쪽 | 핸들 | 오른쪽`. `.row/.col`을 쓰지 않아 `!important` 충돌이 없다.
- 컨테이너 높이는 `calc(100dvh - var(--navbar-h))`. 하드코딩된 `80px`을 CSS 변수로 바꾼다.
- 두 pane 모두 `overflow-y: auto; min-height: 0`.
- 768px 미만은 CSS 미디어쿼리로 세로 쌓기, 핸들 숨김, 페이지 스크롤. JS 분기는 쓰지 않는다.

## 핸들

- Pointer Events + `setPointerCapture`, `touch-action: none`. 마우스와 터치를 한 코드로 처리한다.
- 보이는 선은 얇게, 터치 영역은 16px 이상.
- `role="separator"`, `aria-orientation="vertical"`, `aria-valuenow`. 좌우 화살표 키로 조절한다.
- 더블클릭 시 기본 너비로 복귀. 드래그 중 `user-select: none`.
- 너비는 `localStorage`에 저장한다. 접근 실패는 try/catch로 삼키고 기본값을 쓴다.

## 적용 순서

| 순서 | 대상                          | 메모                                                                                                    |
| ---- | ----------------------------- | ------------------------------------------------------------------------------------------------------- |
| 1    | `SplitLayout` + 단위 테스트   | 기존 파일 변경 없음                                                                                     |
| 2    | 책 편집·만화 편집(`Edit.jsx`) | 기준 페이지. `Edit.test.jsx`가 `.section.directory-menu`를 검증하므로 클래스 유지 또는 테스트 동시 수정 |
| 3    | 카테고리 관리                 | 자체 Card 구조                                                                                          |
| 4    | 표지 관리                     | 다른 세션 커밋 후. `e2e/book-cover-admin.spec.js`의 폭 측정(md=2 가정) 수정 필요                        |
| 5    | 책 보기                       | 상단 툴바 Row만 교체. 본문은 전체 폭 유지                                                               |

## 위험

- `Folder.jsx`의 `height: fit-content`와 `Card.Body`의 `overflow-auto`가 내부 스크롤을 막을 수 있다. pane 높이를 아래로 전달해야 한다.
- jsdom은 레이아웃을 계산하지 않는다.
- 커버리지 임계치와 `/* v8 ignore */` 주석(`Edit.jsx`)은 미확인이다.
- `View.css`는 `#view .directory-menu` 규칙이 있다고 주석에 쓰였지만 실제로는 없다. 구현 시 정리한다.

## 검증 계획

- vitest + RTL: `getBoundingClientRect`를 mock하고 포인터 이벤트로 너비 계산, 최소/최대 clamp, 키보드 조절, 더블클릭 복귀, 저장소 실패 폴백을 검증한다.
- Playwright: 실제 마우스 드래그와 `hasTouch` 터치 드래그, 두 pane 독립 스크롤, 768px 미만 쌓기를 검증한다.
- 각 페이지 적용 후 `cd frontend && npm test`와 `npm run test:e2e`.
