# 텍스트 뷰어 글꼴 선택 설계

## 목적

TXT, DOC/DOCX, HWP, RTF 뷰어에서도 EPUB과 같은 글꼴 선택을 제공한다.
EPUB과 텍스트 뷰어 사이에서 선택 가능한 서체와 저장 동작을 일관되게 한다.

## 현재 상태

- EPUB은 자체 툴바와 `epub_fontFamily` localStorage 설정을 사용한다.
- TXT는 일반 DOM으로 렌더링하고, 줄 합치기 설정만 툴바에 둔다.
- DOCX는 Mammoth 변환 HTML, DOC/HWP는 서버 preview HTML로 렌더링한다.
- RTF는 `rtf.js`가 만든 HTML 요소를 DOM에 붙인다.
- HTML은 빈 sandbox iframe에서 렌더링하므로 부모 컴포넌트가 내부 문서 스타일을 바꿀 수 없다.
- PDF는 페이지 이미지 중심의 고정 레이아웃이라 텍스트 뷰어 범위에서 제외한다.
- EPUB, TXT, DOC/DOCX/HWP, RTF는 공통 글꼴 목록과 `reader_fontFamily`를 사용한다.
- EPUB의 기존 `epub_fontFamily` 값은 읽기와 저장을 계속 지원한다.
- Noto Serif/Sans KR subset, 나눔바른고딕 네 굵기, KoPub Batang Medium 파일을 앱에서 제공한다.
- Nanum Myeongjo 한자는 내장 Noto Serif KR로 대체한다.
- 함초롬바탕은 설치된 글꼴을 사용하고, 없으면 Noto Serif KR로 대체한다.
- `Serif`, `Sans-serif` 일반 옵션은 선택 목록에서 제외했다.

## 설계

### 선택 UI와 설정

- 공통 `ViewerFontSelect`를 EPUB, TXT, DOC, DOCX, HWP, RTF 툴바에서 재사용한다.
- 옵션은 기본, 나눔고딕, 나눔바른고딕, 나눔명조, 함초롬바탕, Noto Serif CJK, Noto Sans CJK, KoPub 바탕체로 맞춘다.
- `reader_fontFamily`에 선택값을 저장해 뷰어 종류가 바뀌어도 설정을 유지한다.
- 기존 EPUB 사용자는 `reader_fontFamily`가 없을 때 `epub_fontFamily`를 읽고, 새 키로 저장한다.

### 적용 위치

- TXT: 콘텐츠 wrapper에 CSS custom property 또는 class를 적용한다. 툴바 글꼴은 상속 대상에서 제외한다.
- DOC/DOCX/HWP: `.doc-content` 바깥 wrapper에 글꼴을 적용한다. 변환된 인라인 글꼴보다 사용자의 선택이 우선하도록 콘텐츠 영역에만 `!important`를 둔다.
- RTF: 렌더된 콘텐츠 wrapper에 같은 규칙을 적용한다.
- EPUB: 현재 iframe 문서별 style 주입 방식을 유지하고, 공통 옵션 값을 사용한다.
- HTML: sandbox 경계를 유지한다. HTML 뷰어도 지원하려면 서버 preview 응답에 허용된 사용자 글꼴을 적용하는 별도 방식이 필요하다. HTML 내부 리소스와 원본 스타일을 고려해 별도 작업으로 결정한다.

### 글꼴 파일

- Noto KR subset을 `frontend/public/fonts`에 제공해 기기 설치 여부와 무관하게 적용한다.
- KoPub Batang 원본 TTF와 라이선스를 함께 제공한다.
- 함초롬바탕은 한컴이 글꼴 파일의 영리 목적 재배포를 제한하므로 파일을 포함하지 않는다.

## 단계

1. 완료: 공통 글꼴 목록과 `ViewerFontSelect`, EPUB 저장값 호환성을 적용했다.
2. 완료: TXT와 DOC/DOCX/HWP/RTF 콘텐츠에 선택값을 연결했다.
3. 완료: Noto와 나눔바른고딕 subset, KoPub 원본 파일, 라이선스 파일을 추가했다.
4. 남음: HTML iframe의 안전한 글꼴 적용 방식을 별도 설계한다.

## 검증 기준

- 각 지원 뷰어에서 선택 글꼴이 콘텐츠에 적용되고 툴바에는 번지지 않는다.
- 새로고침과 뷰어 전환 뒤 선택값이 유지된다.
- EPUB에 저장된 기존 `epub_fontFamily` 값이 계속 적용된다.
- 웹폰트를 제공할 경우 한국어, 라틴 문자, 굵은 글씨가 함께 표시된다.
- HTML과 PDF는 별도 결정 전까지 변경하지 않는다.

## 미결 사항

- HTML iframe viewer의 폰트 선택 요구 여부.
