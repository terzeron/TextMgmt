# Spec: EPUB 책 표지 관리

상태: 1차 구현과 검증을 완료했다. `./build.sh`로 backend와 frontend를 배포했다.
추가 요청: 캐시 수명과 이미지 크기 표시를 구현·검증하고 수정분도 배포했다.

## Objective

관리 탭에 `책 표지 관리` 서브탭을 추가한다.
관리자는 표지가 없는 EPUB에 서점 표지를 선택해서 추가한다.
저장한 표지를 화면에서 확인한 뒤 해당 행을 숨긴다.

## 확인한 현재 구조

- `frontend/src/Admin.jsx`: 관리 서브탭을 `SUBTABS` 배열로 구성한다.
- `frontend/src/Folder.jsx`: 공용 디렉토리 컴포넌트다.
- `frontend/src/folderUtils.js`: 디렉토리 계층을 구성한다.
- `frontend/src/CoverGrid.jsx`, `CoverGrid.css`: 최신 책의 2:3 표지 레이아웃이다.
- `backend/bookstore.py`: 요청한 8개 서점의 검색 클라이언트가 모두 있다.
- 기존 서점 검색 결과는 6개 항목의 tuple이며 표지 URL을 포함하지 않는다.
- RIDI와 조아라는 JSON 검색을 사용한다. HTML 전용 처리를 적용할 수 없다.
- 표지 전용 RIDI 검색은 browser header로 공개 HTML에서 링크와 표지를 읽는다.
- 문피아 표지 검색은 공식 PC client의 `/api/v1/main/search`와 `coverUrl`을 사용한다.
- `BookManager._find_epub_cover_member`: EPUB 2/3의 표지 선언과 이름을 탐색한다.
- `BookManager.get_cover`: EPUB 표지를 추출하고 `.cover_cache`에 저장한다.
- `Book.path_prefix`: `TM_BOOK_DIR`를 사용한다. 운영 대상은 `/mnt/data/text`다.
- 숨김 디렉토리와 symlink 대상 제외 기준으로 EPUB 62,690개를 읽기 전용 집계했다.
  표지 유무 전수 검사는 아직 실행하지 않았다.
- `frontend/package.json`: 실제 테스트 실행기는 Vitest다.
- `build.sh`: backend/frontend 이미지를 build하고 registry에 push한 뒤 Kubernetes에 배포한다.

## 범위와 가정

- 대상은 설정된 책 루트 아래의 `.epub` 파일이다. 확장자는 대소문자를 구분하지 않는다.
- 디렉토리 트리는 파일시스템 기준이다. ES 미색인 EPUB도 처리한다.
- 디렉토리를 선택하면 직하위 EPUB을 검사한다. 하위 디렉토리는 트리에서 따로 선택한다.
- 내부 캐시·백업 디렉토리는 탐색에서 제외한다. symlink를 따라 루트 밖으로 나가지 않는다.
- 사용할 표지 이미지가 없는 EPUB을 목록에 표시한다.
- 손상된 ZIP, 해석할 수 없는 OPF, 지원하지 않는 기존 표지는 검사 오류로 구분한다.
- 기존 표지가 유효한 EPUB은 수정 대상에서 제외한다.
  표지 이미지 선언이 없으면 guide, meta 또는 이름으로 지정한 표지 XHTML의 img·SVG image도 탐색한다.
  URL 인코딩한 상대 경로를 해석한다. 본문의 일반 삽화와 외부 이미지는 표지로 추정하지 않는다.
- 서점에서 찾은 판본이 다른 경우 관리자가 제목·저자·출처를 보고 선택한다.
- 표지 선택은 EPUB 저장을 실행한다. 완료 표지 클릭은 화면의 행만 숨긴다.
- 새로 조회하면 저장한 EPUB은 표지 없는 목록에서 자연스럽게 제외한다.

## 화면 동작

1. 왼쪽에는 기존 `Folder` 컴포넌트로 디렉토리 트리를 표시한다.
2. 오른쪽에는 선택한 디렉토리의 표지 없는 EPUB을 표시한다.
3. 각 행에는 책 제목, 파일명, 빈 2:3 표지 영역, 서점별 후보를 표시한다.
   책 제목은 1.25rem 크기로 표시하고 옆에 `편집`, `조회` 버튼을 배치한다.
   기존 `/book-edit/{book_id}`, `/book-view/{book_id}` 경로를 category와 함께 새 창으로 연다.
   `재검색` 오른쪽 `구글` 버튼은 책 제목을 URL 인코딩해 Google 이미지 검색을 새 탭에서 연다.
4. 후보는 YES24, 알라딘, 교보문고, 네이버쇼핑, RIDI, 문피아, 시리즈, 조아라 순서다.
5. 각 서점은 최대 4개 후보를 제공한다. 후보에는 표지, 제목, 저자, 상세 링크를 표시한다.
   확보한 이미지의 너비 × 높이(px)와 파일 크기(B/KB)도 표시한다.
   해당 책의 모든 서점 후보를 픽셀 수(너비 × 높이)로 비교한다.
   후보 이미지에 5px 패딩을 두고, 가장 큰 후보는 `#00ff00`, 가장 작은 후보는 `#ff0000`를 사용한다.
   확보한 후보 N개의 순위 r(0부터 N-1)에 대해 t = r/max(1, N-1)로 계산한다.
   R = round(255*min(1, 2*t)), G = round(255*min(1, 2*(1-t))), B = 0으로 초록색부터 노란색을 거쳐 빨간색까지 변한다.
   N=1이면 초록색을 사용한다. 같은 픽셀 수는 같은 순위와 색을 사용한다.
   이전 방문에서 로컬에 캐싱한 이미지도 추가 다운로드 없이 크기를 읽어 순위에 포함한다.
6. 후보 영역은 가로로 배치하고 좁은 화면에서는 가로 스크롤을 사용한다.
7. 표지 이미지는 화면에 보이는 후보 N개와 추가 `ceil(N * 0.25)`개까지만 fetch한다.
   세로 목록과 가로 후보 영역의 실제 viewport 교차를 기준으로 노출량을 계산한다.
   추가 후보는 현재 화면의 인접 영역에서 선택한다. 스크롤 시 fetch 대상을 갱신한다.
   이미지 후보의 메타데이터를 받은 것과 이미지 bytes를 fetch한 것을 구분한다.
   화면 밖의 모든 후보 이미지를 `<img src>`로 미리 요청하지 않는다.
   목록과 후보 검색도 현재 화면과 필요한 인접 영역으로 제한한다.
   이미 받은 이미지의 cache hit는 추가 외부 fetch를 발생시키지 않는다.
   같은 서점은 모든 행을 합친 하나의 queue에서 직렬로 fetch한다.
   서로 다른 서점의 queue는 병렬로 처리한다.
   화면 밖 책은 5초 간격으로 하나씩 시작한다. 같은 책의 서로 다른 서점은 병렬로 처리한다.
   화면에 들어온 책은 이 대기를 건너뛴다. 대기 timer와 필요 없는 요청은 디렉토리 전환 시 취소한다.
   RIDI·문피아·시리즈·조아라는 각 행의 가로 스크롤 영역에 노출된 뒤 검색과 이미지 요청을 시작한다.
   초기 빈 칸의 폭으로 뒤쪽 노출을 판단하지 않는다. 앞쪽 네 서점의 검색이 끝난 뒤 실제 배치로 판단한다.
   사용자가 그 전에 오른쪽으로 스크롤하면 앞쪽 검색 완료를 기다리지 않고 노출한 서점을 검색한다.
   이미 받은 결과와 Blob은 재노출 때 재사용한다. 네이버쇼핑까지의 검색과 기존 세로 미리 받기는 유지한다.
8. 서점별 로딩, 결과 없음, 요청 실패를 구분한다. 한 서점 실패가 다른 후보를 막지 않는다.
9. 후보를 선택하면 해당 행의 중복 저장을 막고 저장 진행 상태를 표시한다.
10. 저장 성공 후 빈 영역에 EPUB에서 다시 읽은 표지를 표시한다.
11. 성공 행은 `저장 완료 · 표지를 누르면 목록에서 숨깁니다` 문구와 함께 유지한다.
12. 완료 표지를 클릭하거나 keyboard로 활성화하면 해당 행을 숨긴다.
13. 저장 실패 시 빈 표지 영역과 행을 유지하고 오류와 재시도 동작을 제공한다.
14. 디렉토리 전환 후 늦게 도착한 응답이 새 디렉토리 목록을 바꾸지 않게 한다.
15. `다음 EPUB 검사` 버튼 없이 목록 끝이 화면 아래 25% 범위에 들어오면 다음 페이지를 자동 검사한다.
    표지 없는 책이 없는 페이지도 다음 cursor가 있으면 검사를 이어간다.
    한 번에 한 페이지씩 요청하고 결과를 기존 목록에 덧붙인다. 오류가 나면 자동 검사를 중단한다.
    기존 새로고침 버튼으로 검사를 다시 시작한다.
16. 현재 표지 아래 `직접 입력` 버튼은 이미지 URL 모달을 연다. `표지 적용`으로 fetch와 저장을 실행한다.
    성공 후 기존 완료 표지 표시와 행 숨기기 흐름을 사용한다. 실패하면 모달과 원본 EPUB을 유지한다.
17. `조회` 오른쪽 `재검색` 버튼은 해당 책의 후보 캐시를 지우고 모든 서점 검색을 다시 실행한다.
18. 상태 문구 오른쪽의 작은 휴지통 버튼은 confirm 후 왼쪽 미리보기만 비운다.
    상태 문구가 저장 완료로 바뀌어도 같은 위치에 표시한다. EPUB 표지·백업·서점 후보·Blob은 유지한다.
    다른 후보를 선택하면 최신 revision을 확인하고 EPUB 표지를 교체한다. 저장 완료 후에도 재검색할 수 있다.
19. 직접 URL은 공개 HTTP/HTTPS의 기본 port만 허용한다. credentials와 내부·loopback 주소를 거부한다.
    DNS 검증한 IP에 연결하고 HTTPS hostname 검증을 유지한다. redirect마다 재검증하며 크기·시간을 제한한다.
20. 각 표지 이미지의 우상단 돋보기 클릭으로 원본 픽셀 크기 모달을 연다. hover로 열지 않는다.
    이미 받은 Blob을 사용하고 여러 후보는 이전·다음으로 이동한다. 확대 이미지 선택으로 저장한다.
    돋보기는 표지 선택 버튼과 형제 요소다. 누른 후보를 바로 확대하며 돋보기 클릭만으로 저장하지 않는다.
21. 목록 이미지는 절반으로 줄였던 너비에서 150%로 키운다. 왼쪽 미리보기는 90px, 서점 후보는 84px이다.
    모바일 왼쪽 미리보기는 69px이다. 2:3 비율과 후보의 5px 패딩을 유지한다.
    디렉토리와 목록 컬럼은 Bootstrap의 2:10 비율이다. 모바일에서는 두 컬럼을 세로로 배치한다.
    왼쪽 표지 위에 `내 책` 레이블을 배치한다. 돋보기는 이미지 우상단에 배치한다.
    서점 후보와 같은 높이에서 표지를 시작한다.
    저장한 표지를 읽은 뒤 돋보기를 활성화한다. 저장된 이미지 크기로 확대하며 행 숨기기를 실행하지 않는다.
22. 서점 재시도는 오류 캐시를 건너뛰며 진행 상태와 disabled 버튼을 표시한다.
    완료 이후 이미지 재시도도 가능하다. 실패하면 오류와 재시도 버튼을 다시 표시한다.
23. 카테고리명 오른쪽에 `표기 교체` toggle 버튼을 표시한다. 초기 상태는 꺼짐이다.
    꺼짐은 표지가 없는 EPUB만 조회하고, 켜짐은 표지가 있는 EPUB만 조회한다.
    원래 표지는 `현재 표지`로 표시한다. 후보 적용이 성공한 뒤 `저장 완료` 상태로 전환한다.
    원래 표지와 후보의 확대·직접 URL·재검색·재시도·미리보기만 지우기를 제공한다.
    교체 전에 backup과 최신 revision을 확인한다. 저장 실패 시 현재 표지와 목록을 유지한다.
    저장 중에는 toggle을 비활성화한다. 모드 전환 시 검사와 서점 요청을 취소하고 Blob을 해제한다.
    디렉토리 전환·새로고침·자동 다음 페이지에서도 선택한 모드를 유지한다.

## Backend와 API

신규 API는 모두 명시적인 `Depends(require_admin)`를 선언한다.
아래 경로는 신규 기능 전용이며 기존 서점 검색 응답을 변경하지 않는다.

| 메서드 | 경로 | 역할 |
| --- | --- | --- |
| GET | `/admin/book-covers/directories` | 책 루트의 디렉토리 트리 데이터 |
| GET | `/admin/book-covers/missing` | `directory`, `cursor`, `limit`로 파일 조회. 기본은 표지 없는 파일, `covered=true`는 표지 있는 파일 |
| GET | `/admin/book-covers/candidates/{store_name}` | 상대 `file_path`의 제목·저자·ISBN으로 후보 검색 |
| GET | `/admin/book-covers/candidate-image/{candidate_id}` | 검증한 후보의 이미지 제공 |
| POST | `/admin/book-covers/apply` | `{file_path, candidate_id, revision}`으로 표지 저장 |
| POST | `/admin/book-covers/apply-url` | `{file_path, url, revision}`으로 직접 URL의 표지 저장 |
| POST | `/admin/book-covers/apply-image` | `{file_path, image_data, revision}`의 Base64 이미지로 표지 교체 |
| POST | `/admin/book-covers/cache/clear` | `{file_path, revision}`의 임시 후보 삭제 |
| GET | `/admin/book-covers/preview` | 상대 `file_path`와 저장 revision으로 EPUB 표지 확인. `original=true`는 저장 이미지 크기로 확대 |

- 응답은 상대 경로, 제목, 검사 상태, 후보, 서점 상태, 다음 커서를 명시한다.
  missing 항목의 `book_id`는 파일 inode이며 `category`는 상대 부모 디렉토리 또는 `_root`다.
  두 필드를 추가하고 기존 응답 필드는 유지한다. 미색인 EPUB의 편집·조회는 기존 화면의 오류 처리를 따른다.
- 후보 ID는 서버가 확인한 서점·상세 페이지·이미지의 식별자다.
- replica 간 처리도 가능하도록 공유 볼륨에 책별 캐시를 저장한다.
  `.cover_management/book-<책 경로 hash>/`에 검색 결과, 후보 정보, JPEG를 보관한다.
  이미지의 너비, 높이, 파일 크기도 후보 정보와 함께 저장한다.
  EPUB 적용 전에는 날짜나 파일 수 기준으로 캐시를 삭제하지 않는다.
  EPUB 적용이 성공하면 선택한 이미지와 나머지 서점 후보의 이미지·정보를 모두 삭제한다.
  실패하면 캐시를 유지한다. 다른 책의 캐시와 `.cover_backups`는 보존한다.
  이전 배포의 flat cache도 재사용하고 해당 책의 적용 성공 시 함께 삭제한다.
  책별 shared/exclusive lock으로 다운로드 완료와 적용 후 삭제의 순서를 보장한다.
  coordination에 쓰는 작은 lock 파일은 남겨서 대기 요청 간 lock inode를 유지한다.
- 직접 URL은 require_admin과 공개 주소 검증을 거친다. 서점 후보 다운로드는 서버가 만든 candidate_id를 사용한다.
  후보 API의 `include_covered=true`는 저장 이후 재검색에 사용한다. `refresh=true`는 오류 캐시를 건너뛴다.
  직접 URL의 `replace_existing=true`는 명시한 표지 교체에 사용한다. 기존 기본 동작은 중복 표지를 거부한다.
  image_data는 최대 15MB 이미지의 Base64 길이로 제한하고 형식·해상도·최신 revision을 검사한다.
  최초 저장 후 서버 후보 캐시를 삭제해도, 교체에는 클라이언트가 받은 Blob을 사용할 수 있다.
  재검색으로 candidate_id가 바뀌면 같은 서점의 제목·상세 URL이 같은 후보에 Blob을 연결한다.
- 파일 경로는 정규화 후 책 루트 내부인지 확인한다.
- 이미지와 redirect의 host를 검증한다. private IP와 loopback 접근을 차단한다.
- 요청 timeout, 최대 응답 크기, 이미지 decode와 크기 제한을 적용한다.
- 후보가 있는 검색 결과와 이미지는 EPUB 적용까지 캐싱한다.
  실패와 결과 없음 응답은 짧게 캐싱하고 동시 요청 수를 제한한다.
- 표지 기능의 검색·상세·이미지 요청은 서점별 직렬 queue로 처리한다.
  기존 검색의 상세 페이지 병렬 fetch를 새 기능에 그대로 적용하지 않는다.
- Backend도 동일 서점의 외부 요청을 직렬화한다.
  여러 행·사용자·replica가 요청해도 동일 서점의 외부 fetch가 겹치지 않게 한다.
  서로 다른 서점은 별도 lock을 사용해 병렬 처리를 허용한다.
- 스크롤·디렉토리 전환으로 대상을 벗어난 대기 요청은 취소한다.
  이미 실행한 요청의 결과가 바뀐 화면에 반영되지 않게 한다.
- 서점의 HTML·JSON 원본에서 이미지 필드를 확인한다. URL 형식을 추측하지 않는다.
- 인증이 필요하거나 차단된 서점은 실패 상태와 검색 링크를 표시한다.
- 외부 사이트의 실제 가용성은 별도 smoke test 결과로 보고한다.

## EPUB 저장과 원본 보호

- 별도 모듈에서 ZIP/OPF 갱신을 처리하고 기존 표지 탐색 도우미를 재사용한다.
- 전체 책을 ebooklib로 재작성하지 않고 표지 관련 ZIP entry와 OPF만 갱신한다.
- EPUB 2의 cover metadata와 EPUB 3의 `cover-image` 선언을 버전에 맞게 기록한다.
- 표지 XHTML을 추가하고 기존 본문·목차·spine 순서를 보존한다.
- 저장 파일의 첫 ZIP entry는 압축하지 않은 `mimetype`로 유지한다.
- 원본을 숨김 백업 디렉토리에 보관한다. 백업 파일은 책 목록에 들어가지 않는다.
- 같은 디렉토리에 임시 EPUB을 만들고 ZIP, OPF 참조, 표지 decode를 검증한다.
- ES book ID가 inode이므로 atomic replace는 사용하지 않는다.
  기존 `Loader.reencode_text_file_to_utf8` 패턴처럼 검증한 임시 파일을 같은 inode에 저장한다.
  저장 실패 시 백업에서 원본을 복구한다. 프로세스 강제 종료 시 백업을 복구에 사용한다.
- 파일 단위 lock과 revision 검증으로 동시 저장과 오래된 요청을 거절한다.
- 기존 book ID와 생성 시각을 보존한다. ES 문서가 있으면 파일 크기 등 변경 정보를 갱신한다.
- 표지 캐시를 무효화하고 revision이 포함된 URL로 새 표지를 제공한다.
- 개발 검증은 임시 EPUB 복사본으로 실행한다. 운영 EPUB 일괄 수정은 하지 않는다.

EPUB 저장 규칙은 [W3C EPUB 3.3](https://www.w3.org/TR/epub-33/)을 기준으로 검증한다.

## Tech Stack와 파일 구조

- Python 3.13+, FastAPI, 기존 requests/BeautifulSoup/lxml/Pillow와 표준 ZIP 도구를 사용한다.
- React, React Bootstrap, 기존 Folder와 표지 CSS 패턴을 사용한다.
- 새 dependency, DB schema, CI/CD 변경은 계획하지 않는다.
- `backend/epub_cover.py`: 표지 상태 검사와 안전한 EPUB 갱신을 담당한다.
- `backend/cover_management.py`: 목록, 후보, 캐시, 저장 orchestration을 담당한다.
- `backend/main.py`: admin API를 연결한다.
- `backend/cover_sources.py`: 기존 서점 클라이언트를 재사용하고 표지 후보를 수집한다.
  `backend/bookstore.py`의 검색 계약은 유지한다.
- `frontend/src/BookCoverAdmin.jsx`, `BookCoverAdmin.css`: 새 관리 화면이다.
- `frontend/src/Admin.jsx`: 서브탭을 연결한다.
- `frontend/src/coverFetchQueue.js`: viewport와 추가 25%를 계산하고 서점별 queue를 실행한다.
- `frontend/src/coverAdminApi.js`: cookie 인증, refresh, 요청 취소를 처리한다.
- `frontend/src/coverImageRanking.js`: 책별 해상도 순위, 패딩 색상, 파일 크기 표시를 계산한다.
- `tests/test_cover_management.py`: EPUB과 API 회귀 검증이다.
- `tests/test_cover_sources.py`: 서점 추출, 직렬·병렬 lock, URL 검증을 확인한다.
- `frontend/tests/BookCoverAdmin.test.jsx`: 후보 선택과 완료 확인 검증이다.
- `frontend/e2e/book-cover-admin.spec.js`: 실제 browser에서 화면 흐름을 검증한다.

## Code Style

기존 naming과 요청 utility를 따른다. API 권한 선언은 다음 패턴을 따른다.

```python
@app.get("/admin/book-covers/directories", dependencies=[Depends(require_admin)])
async def list_book_cover_directories():
    ...
```

## Commands와 Testing Strategy

```sh
env -u PYTHONPATH uv run --no-sync python -m pytest tests/test_cover.py tests/test_cover_management.py tests/test_cover_sources.py tests/test_bookstore.py tests/test_main.py
env -u PYTHONPATH uv run --no-sync python -m pytest tests/
env -u PYTHONPATH uv run --no-sync python -m mypy --follow-imports=silent backend/epub_cover.py backend/cover_sources.py backend/cover_management.py
env -u PYTHONPATH uv run --no-sync python -m pylint -E backend/epub_cover.py backend/cover_sources.py backend/cover_management.py
cd frontend && npm test
cd frontend && npm run lint
cd frontend && npm run build
cd frontend && npm run test:e2e -- e2e/book-cover-admin.spec.js
git diff --check
./build.sh
kubectl rollout status deployment/tm-backend -n textmanager --timeout=300s
kubectl rollout status deployment/tm-frontend -n textmanager --timeout=300s
```

- Backend: EPUB 2/3, metadata 없는 EPUB, Unicode/공백 경로, 손상 파일, 경로 이탈을 검증한다.
- 저장: 본문·목차 보존, 표지 재추출, backup, 저장 실패 복구, 충돌, 캐시 갱신을 검증한다.
- 캐시: 서비스 재시작과 오래된 캐시 재사용, 성공 후 책별 삭제, 실패 시 보존을 검증한다.
  진행 중인 다운로드의 캐시가 적용 성공 후 다시 만들어지지 않는지 검증한다.
- 이미지 크기: 픽셀 수 순위, 같은 크기, 후보 0/1개, 이전 방문의 캐시를 검증한다.
  browser에서 실제 크기 표시, 5px 패딩, 초록색·중간색·빨간색을 확인한다.
- API: admin 권한, 입력·출력 계약, 임의 URL, redirect, 비이미지 응답을 검증한다.
- 서점: fixture 기반으로 8개 추출기를 검증하고 실제 요청의 가용성을 별도로 확인한다.
- Fetch: visible N개와 prefetch `ceil(N * 0.25)`개 외에 외부 이미지 요청이 없는지 검증한다.
- Fetch: N=0이면 prefetch도 0개다. 스크롤·가로 스크롤·resize에 따라 대상을 다시 계산한다.
- Fetch: 동일 서점의 최대 동시 외부 요청이 1인지 검증한다.
  서로 다른 서점의 요청은 동시에 실행할 수 있어야 한다.
- Fetch: cache hit와 대기 요청 취소가 외부 요청 수를 늘리지 않는지 검증한다.
- Frontend: 선택, 저장 중 중복 클릭, 성공 후 행 유지, 완료 클릭 후 숨김, 실패와 전환을 검증한다.
- Browser: 실제 크기와 가로 스크롤, keyboard 동작, 표지 갱신을 확인한다.
- 배포 전에 관련 테스트, 전체 회귀, frontend build/lint 결과를 보고한다.
- 배포 후 rollout과 실행 서비스의 상태를 확인한다.

## Boundaries

- Always: 편집 전 GitNexus impact와 실제 소스 확인을 수행한다.
- Always: graph 결과가 모순이면 실제 caller/import를 확인하고 불확실성을 보고한다.
- Always: 원본 backup과 임시 EPUB 검증 후 저장한다.
- Ask first: dependency, DB schema, CI/CD 또는 원래 요청 범위 변경.
- Never: 운영 EPUB 자동 일괄 수정, 인증 우회, 임의 URL fetch, 원본 본문 삭제.
- `./build.sh` 배포는 이번 사용자 요청에 포함된 승인이다. 재승인을 요구하지 않는다.

## 구현 순서와 성공 기준

1. 표지 없는 EPUB 조회와 안전한 ZIP 갱신을 구현하고 Backend 회귀를 검증한다.
2. 8개 서점 후보와 검증된 이미지 제공을 구현하고 fixture/live 결과를 확인한다.
3. 관리 서브탭과 선택·저장·완료 확인 흐름을 구현하고 Frontend/browser를 검증한다.
4. 전체 관련 검증과 변경 검토 후 `./build.sh`를 실행하고 rollout을 확인한다.

모든 대상 디렉토리에 접근할 수 있어야 한다. ES 미색인 EPUB도 목록에 나타나야 한다.
표지 선택 후 재개봉한 EPUB에서 해당 이미지가 표지로 인식되어야 한다.
저장 성공 행은 완료 표지를 클릭할 때까지 현재 화면에 남아야 한다.
실패한 저장은 원본을 보존하고 재시도할 수 있어야 한다.

## Open Questions

사용자가 2026-10-14에 구현을 승인했다.
8개 서점의 실제 검색 결과와 표지 다운로드를 확인했다.

## 검증과 운영 참고

- Backend 전체 회귀: 2,492개 통과. 기존 warning 810개를 보고했다.
- Frontend 전체 회귀: 1,907개 통과.
- 표지 관리 browser 검사: viewport fetch 제한, 저장 후 표지 표시, 완료 클릭을 3회 연속 확인했다.
- 새 Backend 모듈 3개의 mypy와 pylint 오류 검사는 통과했다.
- Frontend lint는 warning 없이 통과했다. production build도 성공했다.
- production build는 대형 chunk warning을 보고했다.
- npm audit은 기존 dependency에서 취약점 7건을 보고했다. High 1건을 포함한다.
  이번 변경에는 dependency 변경이 없다. 기존 `@xmldom/xmldom` 문제는 별도 조치가 필요하다.
- `/mnt/data/text` 최상위 EPUB 20개를 읽기 전용 검사했다. 표지 누락과 검사 오류는 없었다.
- 운영 EPUB을 수정하는 개발 검증은 실행하지 않았다. 저장 검증에는 임시 EPUB을 사용했다.
- GitNexus 변경 분석의 빈 심볼 ID 문제는 index-only force 재생성으로 해소했다.
  재생성 후 tracked 변경 분석은 LOW를 보고했다. 신규 파일은 소스와 테스트로 별도 검토했다.
- 프로세스 강제 종료 시 in-place 저장이 중단될 수 있다.
  `.cover_backups`의 원본과 검증한 임시 파일로 복구할 수 있다.
- 배포 직전 이미지 정보: `/tmp/tm-cover-before-deploy.txt`.
- `./build.sh`는 exit code 0으로 종료했다. 두 deployment 모두 rollout을 완료했다.
- backend/frontend는 각각 새 replica 2개가 Ready이며 restart는 0이다.
- 배포 후 frontend `/admin`과 `/`는 HTTP 200을 반환했다.
  첫 요청의 HTTP 504는 재조회에서 해소됐다.
- 인증 없는 신규 표지 API 요청은 HTTP 401을 반환했다.
- 배포한 backend image digest:
  `sha256:99ed493923411dfc4478f5a46981440ecd824095de8133f00fe0468c18a5f104`.
- 배포한 frontend image digest:
  `sha256:f1c8bc4ea8e12e27b9c9fa6a934c83b0cd4dffe034f0dab495ee2cce4f395eaf`.
- 배포 로그: `/tmp/tm-cover-deploy.log`.
- 필요 시 이전 application revision으로 복구하는 명령은 다음과 같다.
  `kubectl rollout undo deployment/tm-backend -n textmanager`
  `kubectl rollout undo deployment/tm-frontend -n textmanager`
  EPUB 파일 복구는 application rollback과 별개이며 `.cover_backups` 원본을 사용한다.

## 추가 변경 검증

- Backend 전체 회귀 2,497개, Frontend 전체 회귀 1,913개가 통과했다.
- 표지 관련 Backend 검사 26개가 통과했다.
  날짜·서비스 재시작에 따른 보존, 책별 삭제, 저장 실패 시 보존, 다운로드와 적용의 경합을 확인했다.
  이전 flat cache의 이미지 크기 표시와 적용 후 삭제도 확인했다.
- browser에서 해상도와 파일 크기 표시, 5px 패딩, `#00ff00`·중간색·`#ffffff`를 확인했다.
  화면 노출량 +25% fetch 제한과 저장 후 완료 확인 흐름도 통과했다.
- mypy, pylint 오류 검사, Frontend lint, production build가 통과했다.
- 픽셀 수를 크기 순위의 기준으로 사용한다. 같은 픽셀 수는 같은 색을 사용한다.
- 캐시 삭제 실패는 저장 성공과 구분하여 warning으로 알린다.
  `.cover_backups`의 원본은 후보 캐시 삭제 대상에서 제외한다.
- 수정분 `./build.sh`가 exit code 0으로 종료했다.
  두 deployment 모두 새 replica 2개가 Ready다.
  관리 화면은 HTTP 200, 인증 없는 표지 API는 HTTP 401을 반환했다.
- 수정분 backend digest:
  `sha256:448ec681317152785b083c78da5a5fde7176c06e901b5b2107f43a75f5c21dd1`.
- 수정분 frontend digest:
  `sha256:b4a6d3c41e99ee74695990d5e62eebe24ed6e534e9fa89640e468edf4ca6bc70`.
- 수정분 배포 로그: `/tmp/tm-cover-update-deploy.log`.
- GitNexus incremental index의 빈 render UID는 force 재생성으로 다시 해소했다.
  재분석한 tracked 변경의 위험도는 LOW다.

## 후보 색상 변경 및 검증

- 크기 순위의 색상을 초록색 `#00ff00`에서 노란색을 거쳐 빨간색 `#ff0000`까지 변경했다.
  후보 3개에서는 중간 후보가 `#ffff00`이다. 5px 패딩과 동일 크기의 동일 색상 규칙을 유지한다.
- 새 기대값으로 순위 테스트 2개가 실패하는 것을 확인한 뒤 색상 계산을 수정했다.
- Frontend 전체 테스트 1,913개, lint, production build가 통과했다.
- Playwright에서 후보 4개의 초록색·연두색·주황색·빨간색과 5px 패딩을 확인했다.
  화면 노출량 +25% fetch 제한, 저장 완료 표시, 확인 후 행 숨기기도 통과했다.
- `./build.sh frontend`가 exit code 0으로 종료했다. 새 Frontend replica 2개가 Ready다.
  관리 화면은 HTTP 200을 반환했다. 실제 배포 bundle에서도 새 색상 계산을 확인했다.
- Frontend digest: `sha256:d02611bc5161eab87c9fd3e2f230a15f812141ff23b46487fa89e6b0698b645e`.
- 배포 로그: `/tmp/tm-cover-palette-deploy.log`.

## 자동 검사와 제목 옆 작업 버튼 검증

- `다음 EPUB 검사` 버튼을 제거하고 화면 끝에 접근하면 다음 페이지를 자동으로 덧붙인다.
  빈 페이지의 연속 검사, 화면 아래 25% 진입, 요청 직렬화, 오류 시 중단을 단독 테스트로 확인했다.
- 제목을 1rem에서 1.25rem으로 키우고 기존 경로로 연결하는 `편집`, `조회` 버튼을 추가했다.
  category에 한글·슬래시·공백·앰퍼샌드가 있어도 URL 인코딩을 유지한다.
- missing 목록의 book_id와 category를 정상 EPUB과 검사 오류 항목에서 확인했다.
  최상위 디렉토리의 category는 `_root`다. API 인증 규칙과 기존 필드를 유지한다.
- Backend 전체 테스트 2,497개, Frontend 전체 테스트 1,917개가 통과했다.
  Frontend lint와 production build, 변경 Backend 모듈의 pylint 오류 검사가 통과했다.
- 일반 mypy 실행은 수정하지 않은 import 모듈 12개에서 기존 오류 38개를 보고했다.
  `--follow-imports=silent`로 표지 Backend 모듈 3개를 검사한 결과 오류가 없었다.
- Playwright에서 제목 크기와 작업 버튼, 자동 추가 검사와 기존 행 보존을 확인했다.
  후보 표시로 행 높이가 늘어나므로 실제 목록 끝까지 스크롤하며 추가 요청을 검증했다.
  viewport +25% 이미지 fetch 제한, 5px 색상 패딩, 저장 후 행 숨기기도 통과했다.
- `./build.sh`가 exit code 0으로 종료했다. Backend와 Frontend 모두 새 replica 2개가 Ready다.
  관리 화면 HTTP 200과 인증 없는 표지 API HTTP 401을 확인했다.
  배포 bundle에서 자동 검사 marker와 제목 옆 작업 버튼, 수동 다음 검사 버튼 제거를 확인했다.
  Backend pod에서도 missing 목록에 book_id와 category를 제공하는 코드를 확인했다.
- Backend digest: `sha256:106edb99fcffcf3060db76bc1b31c6c7120f99ff448c818e78fd63ec7abd36b1`.
- Frontend digest: `sha256:a1a28bd9cb13ff54c6d78e4d3500c403b4f189e0b6e98744cbf7425a4b2e804d`.
- 배포 로그: `/tmp/tm-cover-auto-deploy.log`.

## XHTML 표지 감지 오류 수정

- 요재지이는 `OEBPS/Text/cover.xhtml`의 img가 `../Images/120%28550%29.jpg`를 참조한다.
  이미지 자체에 cover 선언이나 cover 이름이 없어 기존 탐색기가 놓쳤다.
- 기존 EPUB 2/3 이미지 선언의 우선순위를 유지하고 명시한 표지 XHTML에서 이미지를 찾도록 수정했다.
  guide·meta·manifest 이름을 지원하며 img와 SVG image의 내부 상대 경로를 해석한다.
  본문 삽화와 외부 URL은 제외한다. 표지 페이지 XML은 4MB로 제한한다.
- 공통 함수의 GitNexus 위험도는 HIGH다. 목록·후보·저장·preview·썸네일에 영향을 주므로 사전 보고했다.
- 추가 사례 7개 중 표지 XHTML 사례 4개의 실패를 재현한 뒤 수정했다.
  표지 목록 제외, 썸네일 추출, 기존 표지에 대한 중복 저장 차단도 확인했다.
- 관련 테스트 74개, Backend 전체 테스트 2,504개가 통과했다.
  실제 요재지이의 표지를 추출하고 누락 목록에서 제외되는 것을 확인했다. EPUB 원본은 수정하지 않았다.
- mypy는 기존 오류 32개를 보고했다. 이전 검사와 메시지를 비교한 결과 새 오류는 없었다.
  pylint 오류 2개는 수정하지 않은 기존 행의 collections.abc import와 PIL.Image.LANCZOS 진단이다.
- 요청한 구운몽 이미지는 국립중앙도서관 서버 접속 timeout으로 아직 받지 못했다.
  로컬·Backend pod의 HTTPS 및 HTTP 접속을 확인했다. 해당 EPUB의 표지 적용은 보류했다.
- `./build.sh backend`가 exit code 0으로 종료했다. Backend 새 replica 2개가 Ready다.
  배포 pod에서 실제 요재지이를 기존 표지가 있는 책으로 인식하고 누락 목록에서 제외했다.
  원본 inode 204212198과 파일 크기 13,582,403 bytes를 유지했다. 인증 없는 API는 HTTP 401을 반환했다.
- Backend digest: `sha256:392f9aab711fdb479d88e3dc602716ec0ddf31ff4db0891b64afc1fca4c36c0a`.
- 배포 로그: `/tmp/tm-cover-html-deploy.log`.

## 직접 입력, 재검색, 임시 삭제, 확대 및 저장 크기 검증

- 표지 아래 `직접 입력` 버튼에서 URL 모달을 열고 이미지를 받아 저장한다.
  HTTP/HTTPS만 허용하며 DNS·redirect 주소를 검증하고 실제 연결 IP를 고정한다.
  해당 서점 URL은 기존 서점 직렬 lock을 사용한다. 새 endpoint는 require_admin을 선언한다.
- 제목의 `조회` 오른쪽에 `재검색`을 추가했다. 해당 책의 임시 캐시를 지우고 검색을 다시 시작한다.
- 상태 문구 오른쪽에 휴지통을 배치했다. confirm 후 임시 후보를 지우고 자동 검색을 중지한다.
  저장 완료 상태에서도 같은 위치에 표시한다. EPUB 표지와 backup은 삭제하지 않는다.
- 후보 hover는 받은 Blob을 재사용하는 원본 픽셀 크기의 layer를 연다.
  큰 이미지에는 내부 스크롤을 제공한다. 확대 이미지 선택으로 저장하며 Escape·닫기·mouseleave로 닫는다.
- EPUB 저장 이미지는 가로 최대 600px로 줄이고 비율을 유지한다. 작은 이미지는 확대하지 않는다.
  후보 캐시의 최대 2000×3000px 정책과 화면의 레이아웃 크기 표시는 유지한다.
  큰 JPEG는 decoder에서 먼저 축소하며, decode 이후 픽셀 제한은 8,000만이다.
- 취소 직후 같은 fetch task를 다시 등록하면 사라지던 queue 오류를 수정했다.
  책별 generation으로 재검색·삭제·디렉토리 변경 이후 도착한 응답을 무시한다.
- Backend 전체 2,601개, Frontend 전체 1,963개, Chrome Playwright 2개가 통과했다.
  Backend 표지 모듈 4개와 공통 탐색 함수는 line·branch coverage 100%다.
  Frontend 표지 모듈 5개는 line 310/310 (100%), branch 309/322 (95.96%)다.
  coverage 제외 규칙은 추가하지 않았다. 저장소 전체의 Backend coverage 수치를 의미하지 않는다.
- Frontend lint·production build, Backend 표지 모듈 4개의 mypy·pylint 오류 검사와 diff 검사가 통과했다.
  mypy는 `--follow-imports=silent`를 사용했다. 기존 bundle 크기 warning은 남아 있다.
- 보고서: `/mnt/data/artifacts/book-cover/coverage-report.md`.

  HTML과 원본 coverage JSON·lcov를 같은 artifact 디렉토리에 보관했다.
- `./build.sh`가 exit code 0으로 종료했다. 두 deployment의 새 replica 2개가 각각 Ready다.
  관리 화면 HTTP 200, 표지 목록·직접 URL·임시 삭제의 인증 없는 요청 HTTP 401을 확인했다.
  배포 bundle에서 확대·직접 입력·휴지통·재검색을 확인했다.
  Backend pod에서 600px 저장과 JPEG decoder 축소, 직접 URL·임시 삭제 메서드를 확인했다.
- Backend digest: `sha256:d1313cf7d2cbf0c791851ab8f61a1b4363a2d63bfd6bc189cf34e13b7304c022`.
- Frontend digest: `sha256:50cde644a58b9a5b575ccc26329f98363e62c55bcd836d0f5c47df575eb33124`.
- 배포 로그: `/tmp/tm-cover-final-deploy.log`.

## OPF namespace 호환성과 돋보기 클릭 개선

- `해가 죽던 날`과 `레닌의 키스`는 OPF의 `opf:role` namespace 선언이 없다.
  기존 표지 탐색은 읽었지만 표지 관리의 엄격한 OPF 검사는 XMLSyntaxError로 실패했다.
- 엄격한 파싱이 실패하면 namespace 선언 누락 여부를 확인한다.
  알려진 `opf:` 속성을 OPF namespace로 보완하고 다시 엄격하게 파싱한다.
  다른 prefix·XML 구조 오류는 거부한다. 외부 entity·DTD·network 접근 제한을 유지한다.
- 이 변경은 검사와 저장이 공유하는 `_read_package`에 적용한다. GitNexus 위험도 HIGH를 사전 보고했다.
  저장할 때도 유효한 namespace를 쓰며, 검사만 할 때는 원본 EPUB을 수정하지 않는다.
- 수정 전 회귀 테스트 3개가 실패했고, 수정 후 통과했다.
  기존 표지 인식·누락 목록 제외·중복 저장 차단·namespace 보완 후 저장·다른 XML 오류·XXE 차단을 검증했다.
- 실제 두 책의 SHA256·inode·mtime을 유지하며 표지를 인식하고 누락·오류 목록에서 제외했다.
  전체 Backend 2,609개가 통과했다. `epub_cover.py`의 line·branch coverage는 100%다.
- 마우스 오버 확대를 제거하고 서점명 옆 돋보기 클릭으로 Modal을 연다.
  이미 받은 Blob만 사용하며 새 이미지 fetch를 하지 않는다.
  후보가 여러 개면 모달에서 이전·다음 후보로 이동하고 현재 후보를 선택한다.
  원본 픽셀 크기·내부 스크롤을 유지한다. 닫기·Escape·배경 클릭으로 닫으며 focus를 관리한다.
- Chrome Playwright 2개, Frontend lint·production build, Backend 표지 모듈의 mypy·pylint 오류 검사가 통과했다.
- 당시 휴지통이 후보 전체를 지우고 저장 후 재검색을 막는 결함을 확인했다.
  이후 사용자가 미리보기만 비우고 다른 표지로 교체하는 동작으로 확정했다. 아래 수정에 반영했다.
- 전체 Frontend 1,964개가 통과했다. 표지 기능 line 308/308 (100%), branch 310/322 (96.27%)다.
- `./build.sh` 배포가 통과했고 두 deployment 모두 새 replica 2개가 Ready다.
  관리 화면 HTTP 200과 인증 없는 표지 API HTTP 401, 배포 bundle의 돋보기·이전·다음 버튼을 확인했다.
- Backend digest: `sha256:fca3abca5d09bfae59373b2996369a3163180b23704b4b600092ba2dfe80cf02`.
- Frontend digest: `sha256:49bf2d4937180c8c2d9df8df2c2819d1091947bbc6909de580353324b869c8b6`.
- 배포 로그: `/tmp/tm-opf-magnifier-deploy.log`.
- 배포 Backend에서도 두 책의 기존 표지를 인식하고 누락·오류 목록에서 제외했다.
  운영 pod의 책 경로는 `/books`다. 검사 전후 EPUB SHA256이 같다.

## 미리보기 교체, 알라딘, 재시도 및 느린 미리 받기 수정

- 휴지통은 미리보기만 비운다. 후보 Blob·원본 EPUB·백업은 유지하며, 재검색도 활성화한다.
  선택한 새 표지는 최신 revision 확인 후 교체한다. 브라우저 Blob은 /apply-image로 전달한다.
  직접 URL 교체도 지원한다. 신규 endpoint는 require_admin을 선언하며 입력 크기와 이미지 형식을 검사한다.
- 저장 성공 후 서버 후보 cache 삭제 정책을 유지했다. 반복 교체에서도 같은 정책을 적용한다.
  서버 후보 ID가 바뀌면 제목·상세 URL이 같은 후보의 Blob을 재사용하며 다운로드를 늘리지 않는다.
- EPUB 교체에서는 현재 생성한 tm-cover 이미지·페이지·spine 항목만 교체한다.
  본문·스타일·inode·mtime을 보존하고 매번 직전 EPUB을 백업한다. 실패 시 기존 내용을 복구한다.
  같은 이미지 재선택과 EPUB 2/3에서 생성 표지와 spine 중복 방지를 확인했다.
- 알라딘 All 검색의 저자 주석·권수·긴 부제가 서로 다른 책에 같은 CD 후보를 반환하는 것을 재현했다.
  제목을 정리하고 SearchTarget=Book을 사용한다. ISBN은 유지한다. 공통 img_no.jpg도 후보에서 제외한다.
  알라딘 표지 검색에 books-v2 key를 적용해 이전 오류 결과를 재사용하지 않는다.
  일반 bookstore 모듈의 검색은 변경하지 않았다.
- 실제 알라딘 검색에서 불멸과 소멸과 우리말의 상상력이 서로 다른 도서 후보를 반환하는 것을 확인했다.
  공식 검색·상세 HTML을 기준으로 요청과 placeholder를 검증했다. 실제 이미지 bytes는 추가로 받지 않았다.
- 재시도는 해당 서점의 오류 cache를 건너뛰고 Spinner·재시도 중 버튼을 표시한다.
  실패 후 다시 클릭할 수 있으며 저장 이후 이미지 재시도도 재개한다. 재검색은 최신 revision을 사용한다.
- 화면 밖의 인접 책은 5초 간격으로 시작한다. 같은 서점 직렬·다른 서점 병렬과 N+25% 범위를 유지한다.
  보이는 책은 즉시 처리하고 foreground를 우선한다. 취소·화면 진입·queue 폐기 시 timer를 관리한다.
- 미리보기는 60px, 서점 후보는 56px로 줄였다. 모바일 미리보기는 46px다.
  2:3 비율·5px 색상 패딩·원본 크기 모달은 유지한다. Chrome에서 실제 너비를 확인했다.
- Backend 전체 2,620개, Frontend 전체 1,973개, Chrome Playwright 3개가 통과했다.
  표지 Backend 모듈 4개는 line·branch coverage 100%다.
  Frontend 모듈 5개는 line 362/362 (100%), branch 361/381 (94.75%)다. 제외 규칙은 추가하지 않았다.
- Frontend lint·production build, Backend 모듈 4개의 mypy·pylint 오류 검사, diff 검사가 통과했다.
  mypy는 --follow-imports=silent를 사용했다. 기존 bundle 크기 warning은 남아 있다.
- 보고서: `/mnt/data/artifacts/book-cover/coverage-report.md`.
- `./build.sh`가 exit code 0으로 종료했다. Backend·Frontend 새 replica가 각각 2개씩 Ready다.
  관리 화면은 HTTP 200, 인증 없는 표지 목록·교체 API는 HTTP 401이다.
  운영 bundle의 교체·재시도·background queue와 60px/56px CSS를 확인했다.
- Backend digest: `sha256:a6f2549e8923547639d22b203d6ecd56fc92c55829f08816bd36196c1ea11c3f`.
- Frontend digest: `sha256:4ab5a6c9cb9bcf3d11fd92bf52cf265651fc4ec7c4a93a3d51c64b2f07d3eb3c`.
- 배포 로그: `/tmp/tm-cover-corrections-deploy.log`.

## 최종 레이아웃과 테스트 보강

- 디렉토리:목록 비율은 2:10이다. 왼쪽 표지는 90px, 서점 후보는 84px, 모바일 왼쪽 표지는 69px이다.
  `내 책` 헤더와 돋보기를 서점 헤더와 같은 높이로 배치했다.
  확대는 `original=true`로 저장한 최대 600px 이미지를 읽는다. 기존 기본 썸네일 300px은 유지한다.
- 검색 응답 역전·교체 중 이중 클릭·최신 revision·비표지 파일 보존·실제 file lock·잘못된 Base64를 보강했다.
  foreground 우선·timer 취소·미리 받기 간격·DNS·IPv6·redirect·누적 다운로드 제한도 검증했다.
- Backend 전체 2,645개, Frontend 전체 1,987개, Chrome Playwright 3개가 통과했다.
  표지 Backend 모듈 4개와 공통 EPUB 탐색 함수의 line·branch coverage는 100%다.
  Frontend 모듈 5개의 line은 366/366 (100%), branch는 378/396 (95.45%)다.
  coverage 제외 규칙은 추가하지 않았다. 저장소 전체 Backend coverage 수치를 의미하지 않는다.
- Frontend lint·production build, Backend 모듈 4개의 mypy·pylint 오류 검사와 staged diff 검사가 통과했다.
  기존 Backend warning 810개와 Vite bundle 크기 warning은 남아 있다.
- GitNexus 전체 변경 분석은 파일 24개·심볼 373개·관련 흐름 59개를 보고했다.
  partial·truncated 표시 없이 심볼 373개를 모두 반환했다. risk_level은 critical이다.
  공통 EPUB 탐색 함수의 호출자와 썸네일·검사·저장·확대 경로를 실제 소스와 전체 회귀 결과로 검토했다.
- 최신 보고서와 HTML·JSON·lcov는 `/mnt/data/artifacts/book-cover/`에 보관했다.
