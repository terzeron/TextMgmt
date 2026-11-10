# PDF 뷰어 긴 쪽 타일 렌더 설계

- 작성일: 2026-11-10
- 상태: 승인(2026-11-10), 구현 완료
- 대상: `frontend/src/ViewPDF.jsx`, `frontend/src/pdfRenderScale.js`

## 1. 문제

세로로 아주 긴 쪽(웹툰 스트립)이 흐리게 보인다.

- 원인: `pdfRenderScale.js`의 `MAX_CANVAS_PIXELS = 3_000_000`이 쪽당 canvas 면적을 제한한다.
- 재현 예: book_id=410795227, `142. 고블린 슬레이어 외전 : 이어 원 120화.pdf`.
  - 1쪽 이미지 690×16383px, 쪽 크기 517.5×12287pt.
  - `areaCap ≈ 0.687` → canvas 약 355×8440px. 원본 대비 선형 해상도 약 51%.
  - 표시 폭 690 CSS px, dpr 2이면 약 3.9배 늘려 보인다(계산에 의한 추론).
- 상한은 iOS Safari canvas 메모리 크래시를 막는 장치라서 단순히 올릴 수 없다.

## 2. 목표

- 긴 쪽을 화면 폭 × dpr 해상도로 그린다. 일반 쪽과 같은 선명도를 낸다.
- 살아 있는 canvas 총 면적은 지금 수준(약 30M px, 120MB)을 넘지 않는다.
- 일반 쪽의 렌더 경로와 결과는 바꾸지 않는다.

## 3. 범위

포함:

- 상하 스크롤 + 1장씩 보기 꺼짐 모드(`pdf-fit-width`, 또는 맞춤 꺼짐 + 배율 지정).
- 보기 화면과 편집 화면(`editable`) 둘 다.

제외:

- 1장씩 보기, 좌우 넘김 모드. 이 모드는 쪽을 화면 높이 안에 `object-fit: contain`으로 넣는다. 긴 쪽은 아주 작게 보이므로 상한에 걸리지 않는다. 기존 단일 canvas 경로를 그대로 쓴다.
- 서버, PDF 파일, `merge_and_split.py`. 파일은 바꾸지 않는다.

## 4. 설계

### 4.1 타일 적용 조건

쪽마다 렌더 직전에 판정한다.

- 원하는 배율 `wanted`(기존 `computeRenderScale` 계산)로 그린 면적이 `MAX_CANVAS_PIXELS`를 넘는다.
- 그리고 현재 모드가 3장의 포함 범위다.

두 조건을 모두 만족할 때만 타일로 그린다. 아니면 기존 경로를 그대로 탄다.

### 4.2 타일 계획(순수 함수)

`pdfRenderScale.js`에 순수 함수를 추가한다.

```js
// 반환: null(타일 불필요) 또는 { scale, width, tiles: [{ top, height }] }
// top·height·width 는 정수 장치 픽셀이다.
export function computeTilePlan(page, cssWidth, dpr, { tiled })
```

- `scale = max(floor, wanted)`. 면적 상한(`areaCap`)을 적용하지 않는다.
- `width = round(base.width × scale)`, 전체 높이 `H = round(base.height × scale)`.
- 타일 높이 `tileH = floor(TILE_MAX_PIXELS / width)`. `TILE_MAX_PIXELS`는 `MAX_CANVAS_PIXELS`와 같은 3M으로 시작한다.
- 경계는 정수 장치 픽셀이다. `tiles[i].top = i × tileH`, 마지막 타일은 `H - top`.
- 예: 폭 1380px(690 CSS × dpr 2)이면 `tileH = 2173`, 전체 32766px → 타일 16개.

### 4.3 DOM 구조

타일 쪽은 canvas 하나 대신 타일 컨테이너를 둔다.

```html
<div class="pdf-page pdf-page-tiles" data-page="1" style="aspect-ratio: W / H">
  <canvas
    class="pdf-tile"
    data-page="1"
    data-tile="0"
    style="top: 0%; height: 6.63%"
  />
  ...
</div>
```

- 컨테이너가 쪽 전체 비율(`aspect-ratio`)을 가진다. 레이아웃 높이는 렌더 전후로 같다.
- 타일 canvas는 `position: absolute`로 놓는다. `top`, `height`는 쪽 높이 대비 %로 준다. 타일 사이에 쌓이는 반올림 오차를 없애기 위해서다.
- 편집 화면에서는 이 컨테이너가 `.pdf-page-wrap` 안의 canvas 자리를 대신한다. `MarkCropBox`는 래퍼 기준 %좌표라서 바꾸지 않는다.
- 첫 쪽 placeholder 단계에서는 아직 타일 여부를 모른다. 기존 canvas로 시작하고, 렌더 시점에 타일로 판정되면 컨테이너로 바꾼다. 레이아웃 높이는 `aspect-ratio`가 같으므로 바뀌지 않는다.

### 4.4 렌더

타일 하나를 그리는 방법:

```js
const viewport = page.getViewport({ scale, offsetY: -tile.top });
canvas.width = plan.width;
canvas.height = tile.height;
page.render({ canvasContext: ctx, viewport });
```

- pdf.js 6.4.299의 `PageViewport`는 `offsetY`를 그리기 변환의 이동량에 더한다(`node_modules/pdfjs-dist/build/pdf.mjs` 805행대). canvas 밖은 잘린다.
- 이 동작은 시제품으로 먼저 확인한다(6장 V1).

### 4.5 스케줄러와 메모리

렌더·해제 단위를 "쪽"에서 "렌더 단위"로 바꾼다. 렌더 단위는 일반 쪽 또는 타일 하나다.

- 키: 일반 쪽은 `"7"`, 타일은 `"7:3"`.
- `renderQueueRef`, `activeRenderTasksRef`, `renderedPagesRef`, `nearPagesRef`가 이 키를 쓴다.
- IntersectionObserver는 타일 canvas를 각각 관찰한다. 화면 근처 타일만 그린다. 근접 영역(`rootMargin`)은 지금 값을 유지한다.
- `RETAIN_PAGES_OUTSIDE_VIEW = 6`은 렌더 단위 6개로 해석한다. 타일 하나가 최대 3M px이므로 일반 쪽과 같은 메모리 상한을 지킨다.
- 해제(`evictPage`)는 타일도 1×1로 줄인다. 레이아웃은 컨테이너 `aspect-ratio`가 유지한다.
- 진행률(`everRenderedRef`, `loadedPages`)은 쪽 단위를 유지한다. 쪽의 타일이 하나라도 그려지면 그 쪽을 센다.
- 진단 기록(`trace`)은 타일 canvas도 `live`, `mpx`에 합산한다.

### 4.6 이미지 디코딩 비용

pdf.js는 타일마다 쪽의 operator list를 다시 실행한다. 지금은 쪽을 그린 뒤 `page.cleanup()`을 부른다. 타일마다 부르면 같은 이미지를 타일 수만큼 다시 디코딩할 수 있다.

- 방침: 한 쪽의 타일이 렌더 중이거나 대기 중이면 `cleanup()`을 미룬다. 그 쪽의 마지막 활성 타일이 끝날 때 부른다.
- 시제품에서 타일 렌더 시간을 측정한다(6장 V2). 디코딩이 반복되면 이 방침을 확정한다.

## 5. 경계 처리

- 타일 경계에서 1px 틈이나 겹침이 보일 수 있다. 원인은 CSS 배율 변환의 반올림과 canvas 가장자리 안티앨리어싱이다.
- 1차 대응: 타일 경계를 정수 장치 픽셀로 두고 %배치로 오차를 없앤다(4.2, 4.3).
- 틈이 남으면 2차 대응: 각 타일을 아래로 1 장치 픽셀 겹쳐 그린다. 다음 타일이 위를 덮는다.

## 6. 검증 계획

| 단계 | 항목                | 방법                                                  | 통과 기준                                                   |
| ---- | ------------------- | ----------------------------------------------------- | ----------------------------------------------------------- |
| V1   | `offsetY` 구간 렌더 | 시제품, book 410795227, 데스크톱 Chromium(Playwright) | 타일 이어 붙인 결과가 원본 이미지와 같은 내용               |
| V2   | 디코딩 비용         | 시제품에서 타일당 렌더 시간 측정                      | 첫 화면 렌더가 지금보다 체감 지연 없음(수치는 측정 후 기록) |
| V3   | 선명도              | Playwright 스크린샷, dpr 1·2                          | canvas 폭 = 표시 폭 × dpr                                   |
| V4   | 경계 틈             | 스크린샷 확대 비교, 실기기 iPhone Safari              | 경계에 보이는 선 없음                                       |
| V5   | 메모리              | `trace`의 `mpx`, 긴 PDF 끝까지 스크롤                 | `mpx` 최대값이 지금 상한(약 30M px) 이하                    |
| V6   | iOS 크래시          | 실기기 iPhone Safari로 웹툰 PDF 끝까지 스크롤         | 탭 재로드 없음                                              |
| V7   | 회귀                | 일반 만화 PDF, 1장씩 보기, 좌우 넘김, 편집 크롭·분할  | 지금과 동작 동일                                            |
| V8   | 단위·통합 테스트    | `cd frontend && npm test` (vitest)                    | 전체 통과, 변경 줄 커버리지 100%                            |

테스트 추가 위치:

- `frontend/tests/pdfRenderScale.test.js`: `computeTilePlan` 조건, 경계, 마지막 타일, 비정상 쪽 크기.
- `frontend/tests/ViewPDF.test.jsx`: 타일 쪽 DOM, 타일 단위 관찰·렌더·해제, `cleanup` 지연.
- `frontend/tests/ViewPDFEdit.test.jsx`: 타일 쪽 위 크롭 박스 좌표.

V4·V6은 실기기가 필요하다. 사용자가 확인한다.

## 7. 구현 순서

1. 시제품: `offsetY` 타일 렌더를 별도 브랜치에서 최소 코드로 확인한다(V1, V2). 결과로 4.4, 4.6을 확정한다.
2. `computeTilePlan`과 단위 테스트(TDD).
3. 스케줄러 키를 렌더 단위로 일반화한다. 일반 쪽만 있는 경우 기존 테스트가 그대로 통과해야 한다.
4. 타일 DOM, 관찰, 렌더, 해제를 붙인다.
5. 편집 화면 연결.
6. V3~V8 검증.

## 8. 위험

- `ViewPDF.jsx`는 1475줄이고 스케줄러가 쪽 번호를 전제한다. 키 일반화에서 회귀가 날 수 있다. 3단계에서 기존 테스트로 먼저 막는다.
- 타일 수가 많은 쪽(폭 390 CSS px, dpr 3 휴대폰에서 16383px 쪽은 타일 11개)은 IntersectionObserver 대상이 늘어난다. 성능 영향은 V2·V6에서 본다.
- 맞춤 꺼짐 + 확대(최대 500%)에서는 `wanted`가 커져 타일이 많아진다. 타일 면적 상한이 있으므로 메모리는 지켜진다. 타일 수만 늘어난다.

## 9. 미결정 사항

- `TILE_MAX_PIXELS` 값. 3M으로 시작하고 V2·V5 결과로 조정한다.
- 확대 배율이 클 때 렌더 배율에 상한을 둘지. 원본 이미지 해상도 이상으로 그려도 선명해지지 않는다. `getFullPageImageSize`로 원본 폭을 알 수 있는 쪽은 그 폭 × dpr을 상한으로 두는 안을 검토한다.

## 10. 구현 결과 (2026-11-10)

### 시제품 (V1, V2)

pdfjs-dist 6.4.299 legacy 빌드, Chromium, 120화 1쪽(690x16383px 이미지)으로 측정했다.

| 조건 | 결과 |
|---|---|
| 표시 690 CSS px, dpr 2 (타일 16개) | 타일을 이어 붙인 결과가 한 장 렌더와 픽셀 단위로 같다 |
| 표시 390 CSS px, dpr 3 (타일 11개) | 타일 중간 4개 행에서 차이(최대 204). 소수 배율 보간 위상 차이로 보이고, 경계 행에는 차이 없음 |
| 타일마다 `cleanup()` | 타일당 약 150ms (이미지 재디코딩) |
| 쪽의 마지막 타일 뒤 `cleanup()` | 첫 타일 169ms, 이후 타일당 약 7ms |

### 설계에서 바뀐 점

- **타일 적용 조건에 비율을 추가했다.** 세로가 가로의 3배 이상인 쪽만 타일로 그린다(`MIN_TILE_ASPECT = 3`). 면적 조건만 쓰면 넓은 화면에서 일반 만화 쪽(약 1.4 비율)도 타일이 되어 기존 경로가 크게 바뀐다.
- **`cleanup` 지연 위치.** 렌더 함수 안에서 형제 타일을 확인하면 동시에 끝난 타일들이 서로를 보고 모두 건너뛴다. 실행 중 표시를 지우는 `pumpQueue`의 `finally`에서 판정한다(`releaseTilePage`).
- **1쪽짜리 PDF도 관찰자를 만든다.** 기존 코드는 쪽이 1개면 관찰자 없이 끝났다. 타일은 관찰자가 있어야 그려지므로 1쪽 긴 PDF가 백지가 된다.
- **취소된 렌더의 비트맵을 비운다.** 기존 코드는 취소된 렌더를 "그려진" 목록에서만 지웠다. canvas 크기를 이미 잡은 경우 비트맵이 해제 대상에서 빠진 채 남았다. 일반 쪽에도 있던 누수이고, 타일은 작고 자주 취소되어 크게 드러났다.

### 검증 (V3, V4, V5, V7, V8)

- V3: e2e `frontend/e2e/pdf-tall-page-tiles.spec.js`. 비트맵 폭 = 표시 폭 x dpr, 타일 면적 3M px 이하, 경계 위아래 줄무늬 연속.
- V4: 120화 경계 화면 캡처(dpr 2)에서 보이는 선 없음. 실기기 iPhone Safari는 미확인.
- V5: 표시 760 CSS px, dpr 2, 끝까지 스크롤할 때 살아 있는 canvas 최대값.
  - 일반 쪽 PDF(60쪽): 8개, 24.0M px
  - 120화 수정 전: 14개, 42.0M px (취소 누수)
  - 120화 수정 후: 10개, 30.0M px
- V6: 실기기 iPhone Safari 미확인.
- V7·V8: `npm test` 83개 파일 2,501개 통과. 변경 줄 커버리지 100%. PDF e2e 4개 통과.
