# PDF·EPUB 뷰어 이미지 편집 공통화

## 목적
PDF 뷰어의 이미지 편집(삽입·크롭·분할·삭제)을 EPUB 뷰어도 같은 방식으로 쓰게 한다.

## 공통 모듈
| 계층 | 모듈 | 역할 |
| --- | --- | --- |
| FE | `ImageCropBox.jsx/.css`, `MarkCropBox` | 크롭·분할 박스 UI. 점선 격자, 박스 바깥 흐림 없음, 선 1px |
| FE | `cropRect.js` | 박스 기하, `newMark`, `withBoxRect`, 색 상수 |
| FE | `usePendingImages.js` | 저장 전 삽입 이미지 목록과 blob URL 수명 관리 |
| FE | `insertPreview.js` | 서버 미리보기 요청(기존) |
| BE | `image_edit.py` `crop_pil` | 비율 rect + 기울기를 곧게 펴서 자르는 PIL 크롭 |

## EPUB 이미지 편집
- 대상: spine 항목(idref) 하나. 크롭은 `<img>`가 1장인 쪽, 분할·삭제는 이미지만 있는 쪽.
- 화면: iframe 안 이미지 위치를 `epubImageRect.measureCurrentImage`로 재서 박스를 얹는다.
- 저장: `POST /epub-images/{book_id}` 한 번에 원자적으로 처리(`crops`, `splits`, `deletes`, `inserts`).
- 크롭은 같은 멤버를 제자리 교체하고(JPEG/PNG), 분할은 둘째 영역을 새 쪽으로 spine 바로 뒤에 넣는다.
- 삭제는 목차가 가리키는 쪽과 표지 쪽을 거절한다.

## 제외한 것
- EPUB 쪽 순서 바꾸기: 이미지 처리가 아니라 읽기 순서 편집이라 이번 범위에서 뺐다.
- EPUB 삽화 다중 대기: 기존 삽화·표지 흐름(`pendingCover`)은 그대로 두고 한 번에 한 장만 대기한다.
