# ADR-002: 만화·웹툰 PDF 쪽을 canvas 대신 `<img>`로 보인다

## Status

Accepted (임시 배포 후 iPad 확인 중)

## Date

2026-11-22

## Context

PDF 뷰어는 쪽마다 pdf.js로 canvas에 그렸다. 사용자는 iPad에서 만화·웹툰을 본다.

- **스크롤이 뻑뻑하다**: 같은 작품을 네이버웹툰·카카오웹툰에서 보면 스크롤이 부드럽다.
- **원인**: pdf.js 렌더가 main thread에서 스크롤과 경쟁한다.
- **canvas 상한**: `pdfRenderScale.js`의 `MAX_CANVAS_PIXELS = 3_000_000`은 iOS Safari canvas 메모리 크래시를 막는다(42225b7e).
- **상한의 대가**: 큰 쪽은 원본보다 낮은 해상도로 그린다. 타일 렌더는 폭의 3배 이상 긴 쪽에만 적용한다.
- **실측**: `5_완결`의 999개 시리즈 첫 책 앞쪽을 조사했다.
  - 973개가 쪽 하나에 JPEG 한 장이다.
  - 133개는 쪽 이미지가 3M px를 넘는다.
  - 130개는 iPad 가로 화면에서 canvas 상한 때문에 화질이 떨어진다.
- 일본 만화는 가로로도 넓다. 사용자는 iPad 가로 화면으로 읽는다. 화질 열화는 허용하지 않는다.

## Decision

쪽이 이미지 한 장이면 서버가 원본 이미지를 그대로 보내고, 프론트엔드는 `<img>`로 보인다.

### 백엔드

- **API**: `GET /pdf-page-image/{book_id}?page=N[&raster=1]`과 `/comics/` 경로를 더한다. 둘 다 `require_auth`다.
- **JPEG(DCTDecode)**: 압축 데이터를 바이트 그대로 보낸다.
- **8-bit Flate RGB/Gray**: MuPDF Pixmap으로 같은 화소의 PNG를 만든다.
- **판정 조건**: 그리기 연산이 `q/Q/cm/Do`뿐이고, `Do`가 한 번이며, 이미지가 MediaBox를 꽉 채워야 한다. 마스크·Rotate·CropBox·Decode가 있으면 제외한다. 이미지는 40M px 이하만 받는다.
- **그 밖의 쪽**: `raster=1`이면 폭 2,400px(24M px 상한) PNG로 그린다. 없으면 415를 돌려준다.
- **캐시**: `.preview_cache/{id}_img{page}[r]_v1.bin`. 응답 헤더는 `Cache-Control: private, max-age=3600, no-transform`이다.

### 프론트엔드 (`ViewPDF.jsx`)

- **모드 판정**: 책을 열 때 1쪽을 probe한다. 200이면 모든 쪽을 `<img>`로 보인다.
- **canvas 유지**: 편집 화면(`editable`)이거나 1쪽이 415이면 기존 canvas 경로를 쓴다.
- **표시**: blob을 object URL로 바꿔 `img.src`에 넣는다. `img.decode()` 뒤에 그린 쪽으로 표시한다.
- **미리 받기**: 근접 영역 끝에서 앞쪽 `PREFETCH_AHEAD_PAGES`(5)쪽을 한 번에 하나씩 받는다.
  - 받는 중인 쪽이 화면에 들어오면 그 요청을 기다린다.
  - 실패한 쪽은 다시 미리 받지 않는다.
- **해제**: 먼 쪽을 비울 때 object URL을 revoke한다.

## Consequences

### 좋아진 점

- **스크롤**: Chrome(GPU 없음) 측정 결과다.

  | 모드   | CPU 1× p50 | CPU 1× long task | CPU 4× p50 | CPU 4× long task |
  | ------ | ---------- | ---------------- | ---------- | ---------------- |
  | canvas | 33.3ms     | 51               | 116.7ms    | 649              |
  | img    | 16.7ms     | 0                | 16.7ms     | 0                |

- **화질**: JPEG는 원본과 바이트가 같다(`cmp`). PNG는 원본과 다른 화소가 0개다(`compare -metric AE`).
- **canvas 상한 우회**: 이미지 디코드는 브라우저가 main thread 밖에서 한다.

### 대가와 위험

- **전송량**: 무손실 PNG 쪽은 크다. 확인한 웹툰은 쪽당 평균 1.74MB다.
- **미리 받기 의존**: 미리 받기가 빠지면 웹툰 로딩이 쪽마다 4~6초씩 늦었다(23ac6c8d에서 수정).
- **iPad 메모리**: 아주 큰 이미지를 여러 장 디코드할 때 Safari 동작은 아직 확인하지 않았다.
- **미확인**: 사용자 iPad와 로그인한 운영 경로에서 아직 측정하지 않았다.

### 하지 않은 것

- **무손실 WebP**: 크기를 더 줄일 수 있지만 새 의존성(`image-webp`)이 필요하다. 승인 전이라 하지 않았다.

## 되돌리기

커밋 3개는 서로 독립적으로 되돌릴 수 있다.

```sh
git revert 23ac6c8d d6ba0d44   # 프론트엔드만 canvas로 되돌린다
./build.sh frontend
```

백엔드 API(e86edcb7)는 남겨도 프론트엔드가 쓰지 않으면 영향이 없다.

## 관련

- `docs/pdf-engine.md` — page-image worker 동작
- `docs/superpowers/specs/2026-11-10-pdf-viewer-tall-page-tiling-design.md` — canvas 타일 렌더(이미지 모드가 아닐 때 적용)
