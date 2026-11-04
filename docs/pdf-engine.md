# PDF engine

Rust 1.91을 유지한다. MuPDF Rust crate 0.8.0(C core 1.27.2)로 쪽수, 텍스트와 cover 이미지를 처리한다. 편집은 기존 lopdf 0.46을 유지한다. Poppler 실행 파일은 운영에 필요하지 않다.

## 실행과 제한

`libs/sysdeps/src/pdf_worker_client.rs`는 공용 adapter다. `tm-pdf-worker`는 같은 workspace에서 빌드하는 Rust 실행 파일이며 MuPDF CLI가 아니다. worker 내부에서 MuPDF Rust library를 직접 호출한다. `prlimit` 주소 공간 512MiB·CPU 25초와 부모의 30초 timeout을 유지한다. 실패·시간 초과 시에는 기존 호출자의 오류/fallback 경로를 사용한다.

Noto Serif CJK collection을 MuPDF FontLoader로 연결한다. KR face index는 1이며 regular/bold를 제공한다. base14-fonts는 번들로 포함한다. JPEG·zlib·FreeType·HarfBuzz·JPEG2000 디코더는 시스템 라이브러리를 사용해 번들 JPEG와 시스템 JPEG 간의 성능 차이를 피한다.

미리보기는 64MiB 이하 PDF에서 lopdf로 요청한 페이지의 상속 속성을 고정하고 페이지 트리를 한 번 구성한다. 비내장 글꼴·주석을 보존하고 암호화를 해제한다. 구조 트리 등 원본 문서 전체를 참조하는 Catalog는 가져오지 않는다. 64MiB 초과 PDF는 MuPDF의 seek 기반 reader와 원본 문서의 페이지 선택 API를 사용한다. 큰 파일 전체를 메모리에 읽지 않으며, 다른 문서로 페이지를 graft하는 API의 주석 유실을 피한다.

cover는 단일 full-page JPEG인 경우에만 압축 이미지를 직접 추출한다. 마스크, 회전, crop, 추가 그리기 연산 또는 큰 이미지는 MuPDF 렌더링으로 처리한다. raster의 긴 변은 1200px 이하이며 기존 blank-first-page/둘째 쪽 fallback을 유지한다. cover queue와 replica 간 생성 슬롯 제한은 기존 구현을 사용한다.

## 개발과 Docker

Debian/Ubuntu build packages: `build-essential cmake pkg-config libclang-dev libssl-dev libjpeg62-turbo-dev zlib1g-dev libfreetype6-dev libharfbuzz-dev libopenjp2-7-dev libfontconfig1-dev fonts-noto-cjk`.

```sh
cargo build -p sysdeps --bin tm-pdf-worker
cargo test -p sysdeps
cargo test -p backend --test pdf_pages --test cover --test pdf_edit
cargo test --workspace
```

`cargo build --workspace`에는 worker가 포함된다. 개별 package만 빌드한 개발 환경은 worker를 먼저 빌드한다. adapter는 PATH와 현재 실행 파일 주변(target/debug 등)에서 worker를 찾는다. Docker는 image profile로 worker를 빌드해 `/usr/local/bin`에 넣으며 필요한 native runtime libraries와 fonts를 설치한다. `./build.sh`가 두 이미지를 빌드·배포한다.

`sysdeps::poppler`, `Stage::Pdfium`, 일부 bookdoctor 옵션/보고서 코드는 이전 호출자의 호환성을 위해 이름을 유지한다. 해당 운영 구현은 MuPDF worker를 사용하며 Poppler를 실행하지 않는다. pdftotext mock hook은 과거 포팅 테스트용이며 기본 실행 경로는 native PDF adapter다.

## 검증

기존 Cairo 구현에서 원본 Type0/UHC 글꼴 유실, 주석 유실, 초과 페이지 범위 축소를 새 테스트로 재현한 뒤 교체했다. 회귀 검사는 암호화 PDF, 비내장 한글 글꼴·주석·페이지 순서, 원본 불변, 중첩 페이지 트리, 1GiB sparse PDF, 많은 구조 참조, 크기 제한, 실제 렌더링을 포함한다. 미리보기 cache version을 올려 이전 renderer의 cache를 재사용하지 않는다.
