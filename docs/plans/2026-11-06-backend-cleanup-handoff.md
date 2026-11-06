# refactor/backend-cleanup 인계: 남은 작업

작성: 2026-11-06. `refactor/backend-cleanup`(커밋 174개)을 develop에 머지하고 worktree `tm_dev-corpus`를 지운 시점의 상태다.
다른 세션은 이 파일에서 시작한다.

## 현재 상태

- develop에 머지했다(로컬). **원격에는 push하지 않았다.**
- 머지 결과 트리는 마지막으로 검증한 브랜치 트리(`05d1c8c6`)와 같다(이 문서 커밋만 더해짐).
- 검증 결과(머지 직전 브랜치 기준)
  - `test_runner_rs` 증분 실행 통과, 재실행 대상 없음. 테스트 함수 3,836개.
  - `cargo clippy --workspace --all-targets` 경고 0건.
  - `docker build -f backend/Dockerfile`(push·배포 없이) 성공. 태그 `terzeron/tm_backend:verify-05d1c8c6`.
    이미지 안에서 `backend`, `loader`, `tm-pdf-worker`, `tm-index-worker`, `epubveri`가 실행되고,
    `tm-index-worker → tm-pdf-worker` 연결로 PDF 본문 추출과 색인 레코드 생성이 동작했다.
- 의존 관계 점검(마지막): crate 간선 83개, 모두 위층→아래층. 쓰이지 않는 의존 0개.
  형제 모듈 순환은 `doc_checker`의 `format_dispatcher ↔ nested_scanner`(의도된 재귀) 하나뿐이다.
- 의존 그래프 페이지: https://claude.ai/artifact/JNssvtfRAFKHS6axFsWYaH (71차까지 반영).
  생성 스크립트(`gen_graph19.py`, `mods_direct.py`, `files_mods_tree.py`, `update_graph*.py`, `metrics2.py`)는
  이전 세션의 scratchpad에 있어 사라질 수 있다. 그래프를 다시 그리려면 스크립트를 새로 만들어야 할 수 있다.

## 남은 작업

### 1. 새로 뗀 모듈의 테스트 1:1 배치 (승인 대기 중이던 제안)

아래 모듈은 원래 모듈의 테스트가 간접으로 검증한다. 테스트는 통과하지만 "테스트 파일은 소스 모듈과 1:1" 규칙에 어긋난다.

| 모듈                                      | 지금 검증하는 곳                                                                | 할 일                                                            |
| ----------------------------------------- | ------------------------------------------------------------------------------- | ---------------------------------------------------------------- |
| `doc_checker::zip_member_reader`          | `tests/rust/doc_checker/epub/security_checker.rs` (`Limits`, `decode_calls` 등) | `tests/rust/doc_checker/zip_member_reader.rs`로 해당 테스트 이동 |
| `doc_checker::ole_checker`                | `doc`·`hwp` 테스트                                                              | `tests/rust/doc_checker/ole_checker.rs`                          |
| `doc_checker::word_field_checker`         | `doc`·`zipcheck/ooxml_checker` 테스트                                           | `tests/rust/doc_checker/word_field_checker.rs`                   |
| `doc_repairer::epub_package_io`           | `epub_sanitizer`·`defect_repairer` 테스트                                       | `tests/rust/doc_repairer/epub_package_io.rs`                     |
| `doc_reader::body_decoder` (`pub(crate)`) | `text`·`epub` 테스트                                                            | 소스 안 `#[cfg(test)] mod tests`                                 |

새 테스트를 만들기보다 옮기는 것이 중심이다. Cargo `[[test]]` target도 함께 추가한다.

### 2. push와 배포 (승인 필요)

- `git push origin develop`은 하지 않았다. 사용자 승인 후 진행한다.
- 배포는 `./build.sh`(레지스트리 push + `kubectl set image`)다. 이것도 승인 후 진행한다.
- Dockerfile 변경 3건이 이번 작업에 들어 있다: `-p pdf_engine`, `-p doc_checker`(epubveri), `-p file_indexer`(`tm-index-worker` 설치).
  로컬 빌드로는 확인했지만 클러스터 기동(환경 변수, ES·MySQL 연결)은 배포해야 확인된다.
- 검증용 이미지 `terzeron/tm_backend:verify-05d1c8c6`이 남아 있다. 디스크 사용률이 98%였으니 필요 없으면 `docker rmi`로 지운다.

### 3. 머지된 로컬 브랜치 정리 (완료, 1건 남음)

develop에 머지된 로컬 브랜치 10개를 `git branch -d`로 지웠다(2026-11-06).
`refactor/parser-timeout`만 남았다. 이번 머지로 develop에 모두 들어갔지만 worktree `tm_dev-cohesion`에서 체크아웃 중이라 지우지 않았다.
지우려면 그 worktree부터 지운다(`git worktree remove`, 커밋하지 않은 변경이 없는지 먼저 확인).

### 4. 분류 시스템 (다음 큰 작업)

- `libs/classifier`는 trait만 있는 계약 crate다(`Classify`, `BookClassifierApi`, `ModelOnlyClassifier`, `Prediction`).
  사용자 결정으로 book_manager에 합치지 않고, 새 분류 시스템의 계약 crate로 쓴다.
- 지금 분류 제안은 `book_manager::category_proposer`가 카테고리 매핑 키워드만으로 만든다.
  `BookManagerDeps::with_classifier_factory`로 `Classify` 구현을 꽂으면 모델 판정을 쓴다.

## 그 밖에 알아 둘 것

- worktree `tm_dev-cohesion`(`refactor/parser-timeout`)은 건드리지 않았다. 그 브랜치의 커밋은 이번 머지로 develop에 들어왔다.
- develop worktree(`tm_dev`)의 추적하지 않는 파일(`0.txt`~`9.txt`, `22222.txt`, `graph-output/`)은 건드리지 않았다.
- crate 하나만 따로 테스트할 때는 워커를 먼저 빌드한다:
  `cargo build -p pdf_engine --bin tm-pdf-worker`, `cargo build -p file_indexer --bin tm-index-worker`.
  backend·loader·book_manager 테스트 일부가 `target/debug`의 워커를 찾는다.
- 이번 작업에서 정한 배치 규칙은 `docs/rust-workspace.md` "규칙" 절에 있다
  (MySQL 저장소 위치, 격리 실행 워커 위치, fixture 전용 crate는 dev 의존).
- 테스트·커버리지는 `test_runner_rs`로만 돌린다(셸 env 우회 목록은 이전 세션 기록과 같다).

## 이번 작업에서 바뀐 큰 구조 (요약)

- 새 crate: `container_parser`, `book_model`, `pdf_engine`, `doc_repairer`, `image_processor`, `http_client`.
  이름 변경: `stat` → `parse_stat_counter`.
- backend는 axum 라우터와 조립만 남았다. Google 토큰 검증과 viewer 카테고리 접근 정책은 `auth_manager`,
  카테고리 매핑·조회 이력 facade는 `db`, 표지 관리 조립은 `BookManagerApi::cover_management`로 옮겼다.
- 워커 바이너리: `tm-pdf-worker`(pdf_engine), `tm-index-worker`(file_indexer, 새로 추가), `epubveri`(doc_checker로 이동).
- 실행 파일 확장자 목록은 `doc_checker::exec_extension_table`의 계층 표로 합쳤다.
