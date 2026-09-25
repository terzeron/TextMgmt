# ADR-001: EPUB 뷰어의 렌더링 방식과 반응 기계 재설계

## Status
Accepted

## Date
2026-09-25

## Context

EPUB 뷰어(paginated 좌우 페이지 넘김)에서 오랫동안 해결되지 않던 결함이 있었다.

1. **빈 페이지 / 건너뜀**: 텍스트가 페이지를 다 채우면 잘리고, 다음 페이지는 아예
   보이지 않았다. 챕터의 첫 페이지만 렌더링되고 나머지는 비어 보이기도 했다.
2. **입력별 렌더링 불일치**: 화살표 버튼으로 넘긴 것과 스와이프로 넘긴 것의
   렌더링 결과가 달랐다.
3. **메타 영역 접근 불가**(모바일): 뷰어가 화면을 다 차지해 위의 책 정보로
   스크롤업이 안 됐다.
4. **깜빡임**: 읽는 중 iframe이 통째로 지워졌다 다시 그려졌다 했다.
5. **대형 그림책**: 수백 MB짜리 책은 브라우저 탭이 죽었다.

사용자 기기는 아이폰/맥 사파리(WebKit)다. 문제는 Chrome(headless)에서
재현되지 않는 경우가 많아, 픽셀 단위 검증(스크린샷 → 비흰색 픽셀 비율)을
도입해야 겨우 재현·확인됐다.

## 근본 원인 (실측으로 확인한 것)

### 1. 본문 overflow 클리핑이 컬럼을 안 보이게 한다 (최종 원인)

epub.js paginated 방식은 챕터 전체를 하나의 **아주 넓은 iframe**(수천 px)에
넣고 본문을 다중 컬럼으로 쪼갠다. 본문의 컬럼들은 본문 박스(한 페이지 폭)
**밖으로** 뻗는다.

- 터치 팬 차단을 위해 본문에 `overflow: hidden`(양축)을 걸었더니, 본문 자체의
  클리핑이 **박스 밖으로 뻗은 컬럼들을 그리지 않게** 만들었다. 레이아웃
  (range rect)은 정상이지만 픽셀이 비었다.

  WebKit 스크린샷 실측 (비흰색 픽셀 비율):

  | 상태 | 첫 컬럼 | 뒷컬럼들 |
  |---|---|---|
  | `overflow: hidden` | 23% | **0%, 0%, 0%, 0%** (빈 페이지) |
  | `touch-action: pan-y` | 23% | 13%, 13%, 12%, 12% (모두 그려짐) |

- 교훈: **레이아웃 검사(rects/scrollWidth)만으로는 "그려짐"을 확인할 수
  없다.** 그리기 문제는 스크린샷 픽셀 분석으로만 검출된다.

### 2. 터치 팬이 스크롤 상태를 어긋나게 한다

epub.js는 본문에 `overflow-y: hidden`만 걸어 CSS 규칙상 `overflow-x`가
`auto`로 계산된다. 모바일에서 터치 팬이 본문 컬럼을 네이티브로 직접 밀고,
버튼은 컨테이너 `scrollLeft`를 움직여 **두 스크롤 상태가 어긋난다** — 빈
페이지(컬럼 사이 간격), 건너뜀, "스와이프와 화살표의 렌더링 불일치"의 원인.

- 차단은 `overflow: hidden`(→ 클리핑, 위 1 참조)이 아니라
  **`touch-action: pan-y`**로 한다. 가로 팬만 막고 그리기는 유지된다.

### 3. epub.js의 재확장 자가치유는 죽어 있다

`contents.js`의 ResizeObserver가 iframe의 `documentElement`를 관찰하지만 그
박스는 뷰포트라 **절대 변하지 않아 절대 발화하지 않는다**(정준 예제에서도
reflow 후 scrollWidth 불변 확인). 글자 크기 변경·늦은 폰트/이미지 로드로
본문이 다시 쪼개지면 뷰가 낡은 폭에 머물러 잘린 컬럼(건너뜀)과 iframe 끝
너머 빈 페이지가 생긴다. reflow를 일으키는 UI(글자 크기/글꼴 변경)를
제공하는 쪽이 재확장을 책임져야 한다.

### 4. 측정·재계산 기계는 오히려 안정성을 해친다

뷰어 높이를 문서/viewport에서 측정해 `containerHeight` state로 쓰고, 그
변화마다 `rendition.resize()`를 부르면 epub.js는 **모든 view를 clear()하고
다시 표시**한다(`managers/default/index.js` clear + `rendition.js`
onResured → display). ResizeObserver/visualViewport 리스너가 문서 어딘가의
변화마다 발화해 읽는 중 깜빡임과 진동이 생겼다.

### 5. 기타 epub.js API의 함정 (대조로 확인)

- `displayed` 이벤트는 **명시적 `display()` 때만** 발화하고 `next()/prev()`
  섹션 전환 때는 발화하지 않는다. 섹션 추적은 `relocated`(모든 이동·스크롤마다
  발화)로 해야 한다.
- `locationChanged`의 payload는 **평면 구조**다: `{index, href, start, end,
  percentage}` — `location.start`로 읽으면 안 된다.
- 휠/터치 이벤트는 iframe 안쪽 문서로만 간다. 부모 문서로 체이닝되지 않는다.

### 6. 대형 그림책은 epub.js가 감당하지 못한다

epub.js는 책 전체를 JSZip으로 브라우저 메모리에 올린다. 이미지 수십 장(각
수 MB)이면 탭이 죽거나 일부만 렌더링된다. backend preview rebuild에서
이미지를 축소한다(본 ADR 범위 밖 — 별도 변경, `book_manager.py` 참조).

## Decision

1. **react-reader를 버리고 epub.js를 직접 사용한다.** react-reader의
   SwipeWrapper/기본 스타일/prop 가정이 문제의 상당 부분이었다.
2. **`flow: "paginated"`(좌우 페이지 넘김)를 유지한다.** 세로 스크롤은
   요구사항이 아니다(사용자 명시적 거절).
3. **본문 스크롤 잠금은 `touch-action: pan-y`**(`overflow: hidden` 금지).
4. **페이지 넘김은 버튼(‹ ›)과 키보드 방향키만.** 스와이프/터치 팬 처리는
   하지 않는다 — 어떤 입력으로 넘겨도 렌더링이 같아야 한다.
5. **뷰어 높이는 상수**: standalone `100%`, 임베디드 전체보기 `100dvh`,
   미리보기 `60vh`. 측정·재계산 기계(ResizeObserver 문서 감시,
   visualViewport 리스너, `containerHeight`→`resize()` chain)는 두지 않는다.
6. **reflow 뒤 재확장은 직접 한다**: 글자 크기·글꼴 변경 시 현재 view의
   `expand()`를 호출하고, `rendered` 뒤 폰트(`fonts.ready`)·이미지
   (`load`)·지연 타이머 신호마다 다시 측정한다(installReexpand).
7. **기존 패치를 전부 버린다**: XHTML 강제 파싱(backend가 rebuild에서
   자기닫힘 RCDATA를 이미 편다), spine.get 폴백, display 래핑, scrollbar
   숨김 스타일.
8. **읽기 위치는 CFI로 저장**(`epub_location_<id>`, 기존 키와 호환).
   `relocated`의 `location.start.cfi`를 throttle 저장하고, 로드 뒤
   `display(cfi)`로 복원한다.
9. **빌드 ID를 첫 화면 하단에 표시**한다(vite `define` → `Home.jsx`
   footer). 배포 확인용(`APP_BUILD_ID` 환경변수로 덮어쓰기 가능).
10. **무해 브라우저 공지는 에러 로그에서 걸러낸다**("ResizeObserver loop
    ..." — `clientLogger.js` BENIGN_ERRORS).

## Alternatives Considered

### 세로 스크롤(`flow: "scrolled-doc"`)
- Pros: 컬럼 계산이 아예 없어 모든 결함이 구조적으로 사라진다. WebKit에서도
  안정적(실측).
- Cons: 사용자가 좌우 페이지 넘김을 요구(세로 스크롤 거부).
- Rejected: 요구사항 불일치. 다만 컬럼이 없다는 발상(와이드 iframe 제거)은
  원인 분석에 결정적 힌트가 됐다.

### `overflow: hidden`(양축)으로 터치 팬 차단
- Pros: 팬이 원천 차단된다.
- Cons: **WebKit에서 본문 밖으로 뻗은 컬럼이 그려지지 않는다**(픽셀 실측으로
  확인 — 레이아웃은 되지만 픽셀이 비었다).
- Rejected: `touch-action: pan-y`가 같은 효과를 내면서 그리기를 유지한다.

### react-reader 유지 + 옵션/훅 보정
- Pros: 기존 코드 유지.
- Cons: SwipeWrapper의 터치 가로채기, `displayed`/payload 함정, 기본
  paginated 스타일 가정이 결함과 맞물린다. 패치 bisect 결과 모든 보정으로는
  근본 해결이 안 됐다.
- Rejected: epub.js 직접 사용이 더 단순하다.

### scroll-snap으로 모바일 뷰어 정렬
- Pros: 뷰어가 화면에 정확히 맞는다.
- Cons: 뷰포트 크기와 같은 snap area는 **스크롤을 트래핑**한다(로드 시 뷰어로
  스냅되어 메타 영역이 가려지고, 스크롤업이 재캡처된다 — 실측).
- Rejected: 세로 스크롤 모드 자체가 제거됐고, snap은 함정이다.

## Consequences

- 렌더링 경로가 단순해진다: epub.js 기본 응용 + 스타일 + 넘김 버튼.
- 사파리(WebKit)에서 모든 컬럼이 그려진다(픽셀 실측 확인).
- 어떤 입력으로 넘겨도 렌더링이 같다(스와이프는 아무 동작도 하지 않음).
- 반응 기계가 없어 읽는 중 재표시(깜빡임)가 없다. 창 크기 변경은 epub.js
  내장 리스너가 처리한다.
- 세부 좌표(CFI) 기반 위치 복원이라 기존 독서 이력과 호환된다.
- epub.js의 내부 구조(manager/views/expand)에 의존하는 부분이 남는다 —
  라이브러리 업그레이드 시 `installReexpand`와 `relocated` 핸들러를
  재확인해야 한다.

## Verification (검증 방법)

- **그리기 문제는 스크린샷 픽셀 분석으로만 검출된다**: Playwright
  screenshot → PIL로 비흰색 픽셀 비율. rect/scrollWidth 검사로는 부족하다.
- e2e: `epub-viewer.spec.js`(기본 동작·일관성·잘림·복원·메타 공존),
  `epub-view-render.spec.js`(viewport 변경 안정성),
  `book-view-epub-fit.spec.js`(높이 상수·메타 노출).
- 단위: `tests/ViewEPUB.test.jsx`(epubjs mock 기반).

## 관련 파일

- `frontend/src/ViewEPUB.jsx` — 뷰어 전체
- `frontend/src/ViewEPUB.css` — 툴바/버튼/페이지 정보
- `frontend/src/ViewSingle.jsx` — standalone/embedded 호스트
- `frontend/src/clientLogger.js` — 무해 공지 필터
- `frontend/src/Home.jsx`, `frontend/vite.config.js` — 빌드 ID
- `backend/book_manager.py` — preview rebuild(폰트 제거·이미지 축소, 캐시 버전)
