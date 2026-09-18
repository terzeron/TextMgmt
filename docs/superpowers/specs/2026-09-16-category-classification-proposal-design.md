# 카테고리 분류 제안·승인 설계

- 날짜: 2026-09-16
- 상태: 설계 승인 대기
- 범위: 카테고리 관리 탭의 자동 분류를 "제안 → 검토 → 선택 승인"으로 바꾼다

## 1. 배경과 목표

지금 `자동 분류` 버튼은 누르는 즉시 파일을 옮기고 ES를 바꾼다. 관리자가 결과를
미리 볼 수 없고 되돌리기도 어렵다.

이 작업의 목표는 두 가지다.

1. 버튼 그룹을 키워드 영역 아래로 옮긴다.
2. 자동 분류를 수동 분류로 바꾼다. 실행하면 책별 제안 목록을 표로 보여주고,
   관리자가 고른 것만 실제로 옮긴다.

비목표: 재귀 분류, 되돌리기(undo), 제안 이력 보관.

## 2. 확신도 등급

모델 확신도(`confidence`, 0~1)를 1차 기준으로 쓴다. "점수 높음"의 경계는
`/mnt/data/text/.classifier/bookstore_policy.json`의 `override_below`다. 이 값은
구간별 정답률로 이미 보정돼 있어 새 임계값을 만들 필요가 없다.

| 상황                                            | 제안   | 등급    | 체크박스 |
| ----------------------------------------------- | ------ | ------- | -------- |
| 키워드 단독 최고점 + 모델도 같은 답 + 점수 높음 | 키워드 | certain | 체크됨   |
| 키워드 단독 최고점 + 모델도 같은 답 + 점수 낮음 | 키워드 | unsure  | 해제됨   |
| 키워드 단독 최고점 + 모델은 다른 답             | 키워드 | unsure  | 해제됨   |
| 키워드 동점 다수                                | 없음   | unknown | 비활성   |
| 키워드 미일치 + 점수 높음                       | 모델   | certain | 체크됨   |
| 키워드 미일치 + 점수 낮음 + 서점 2곳 이상 일치  | 서점   | certain | 체크됨   |
| 키워드 미일치 + 점수 낮음 + 서점 1곳만          | 서점   | unsure  | 해제됨   |
| 키워드 미일치 + 점수 낮음 + 서점 갈림           | 없음   | unknown | 비활성   |
| 키워드 미일치 + 점수 낮음 + 정보 없음           | 없음   | unknown | 비활성   |

모델 점수는 모델이 고른 카테고리에 대한 확신도다. 키워드와 모델이 다른 답을 내면
그 점수로 키워드 답을 보증할 수 없으므로 `unsure`로 내린다. 목적지는 키워드 쪽을
쓰고, 근거 칸에 모델이 고른 카테고리와 점수를 함께 적는다.

`unknown`은 제안 목적지가 없어서 비활성이다. 사용자가 목적지를 직접 고르면
활성으로 바뀐다(4절).

점수가 낮은데 서점이 답을 못 준 경우, 모델이 낸 낮은 확신도 답은 목적지로 쓰지
않는다. 근거 칸에만 참고로 적는다. 낮은 점수를 목적지 근거로 쓰지 않는다는 것이
이 등급표의 전제이기 때문이다.

## 3. 데이터

제안 상태 파일: `<path_prefix>/.classify_proposal_<content_type>.json`

```
status            running | ready | applying | done | failed
source_category   제안을 만든 카테고리
total_count, processed_count, updated_at
applied_count, failed_count      (적용 단계에서만)
items[]
  file_path        상대 경로. 항목의 고유 키
  title
  current_category
  target_category  제안 목적지. unknown이면 null
  grade            certain | unsure | unknown
  confidence       모델 점수. 없으면 null
  source           keyword | model | bookstore_majority | bookstore_single
  matched_keywords
  model_category   모델이 고른 카테고리. 제안과 같으면 null
  reason           사람이 읽을 근거 한 줄
  apply_status     적용 결과. pending | moved | failed
  apply_error
```

공유 볼륨에 두므로 replica가 둘이어도 같은 값을 본다. 갱신이 끊긴 `running`을
`failed`로 굳히는 처리는 기존 `stale_auto_classify_status`를 그대로 쓴다.

## 4. 백엔드

### 4.1 메서드

`BookManager.propose_category_changes(category, mappings, *, use_bookstore,
use_content_meta, delay, on_progress) -> (result, error)`

- `_iter_category_indexable_files(category, recursive=False)`로 직하위 파일만 훑는다.
- 파일마다 새 헬퍼 `_propose_category_for_file()`을 불러 제안과 등급을 만든다.
- 파일을 옮기지 않는다. ES도 건드리지 않는다.

`BookManager.apply_category_changes(items, *, content_type, clean_existing,
on_progress) -> (result, error)`

- `items`는 `{file_path, target_category}` 목록이다.
- 항목마다 검증한다. 제안 목록에 있는 `file_path`인가, `target_category`가
  `_is_safe_category_name`과 `_is_top_level_target_category`를 통과하는가,
  파일이 아직 있는가.
- 통과한 것만 `_move_classified_file(dry_run=False)`로 옮긴다.
- 실패는 건별로 기록하고 계속 진행한다.

### 4.2 분류기 확장

`BookClassifierService.classify_file`이 지금 모델 점수를 밖으로 내보내지 않는다.
`entry` 딕셔너리에 `confidence`와 `model_category`를 실어 보낸다. 반환 튜플의
모양은 그대로라 기존 호출자는 영향을 받지 않는다.

키워드가 맞아도 모델을 돌린다. 등급을 매기려면 점수가 필요하기 때문이다.

### 4.3 API

모두 `admin_dep`을 붙인다.

| 메서드 | 경로                                  | 역할                                                           |
| ------ | ------------------------------------- | -------------------------------------------------------------- |
| POST   | `/categories/classify-proposal`       | 제안 생성 시작. 백그라운드. 이미 running이면 `already_running` |
| GET    | `/categories/classify-proposal`       | 상태와 목록 조회. 폴링과 복원에 쓴다                           |
| POST   | `/categories/classify-proposal/apply` | 승인 적용 시작. 백그라운드                                     |
| DELETE | `/categories/classify-proposal`       | 제안 폐기                                                      |

승인 요청 본문은 `{"items": [{"file_path": "...", "target_category": "..."}]}`다.
사용자가 표에서 목적지를 바꿀 수 있어야 하므로 카테고리를 함께 받는다. 대신
서버가 4.1의 검증을 모두 다시 한다. 프론트가 보낸 값을 그대로 믿고 파일을
옮기지 않는다.

### 4.4 제거 대상

새 흐름으로 대체되어 쓰이지 않는 것만 걷어낸다.

- `POST /categories/auto-classify` 엔드포인트
- `BookManager.auto_classify_category`와 그 안의 `dry_run` 분기
- `CategoryAutoClassifyModel`

상태 파일 기계(`_read_auto_classify_status`, `_replace_auto_classify_status`,
`stale_auto_classify_status`, 하트비트 만료)는 제안 흐름이 그대로 필요로 하므로
이름만 바꿔 재사용한다. 지우고 다시 쓰지 않는다.

## 5. 프론트엔드

### 5.1 버튼 그룹 재배치

`이름 변경`, `삭제`, `ES 재적재`, `이상 항목 재적재`, `분류 제안`을 키워드 영역
아래로 옮긴다. 같은 패널(`selectedCategory && !selectedMismatch`) 안에서 순서만
바꾼다. 헤더로 옮기지 않는다.

`자동 분류` 버튼의 이름을 `분류 제안`으로 바꾼다. 더 이상 바로 옮기지 않는다.

### 5.2 제안 표

버튼 그룹 아래에 렌더한다. 열은 이렇다.

| 열            | 내용                                            |
| ------------- | ----------------------------------------------- |
| 체크박스      | `certain` 체크, `unsure` 해제, `unknown` 비활성 |
| 파일명        | 제목이 있으면 제목, 없으면 파일명               |
| 현재 카테고리 |                                                 |
| 제안 카테고리 | 드롭다운. 사용자가 바꿀 수 있다                 |
| 점수          | `confidence`. 없으면 `-`                        |
| 근거          | `source`와 `reason` 요약                        |

머리글에 전체 선택 체크박스를 둔다. 비활성 행은 건너뛴다.

**목적지 드롭다운**은 최상위 카테고리 목록에서 고른다. 사용자가 `unknown` 행에서
목적지를 고르면 그 행의 체크박스가 활성으로 바뀐다. 제안과 다른 값을 고른 행은
표시로 구분한다.

표 아래에 `분류 승인 (N건)` 버튼을 둔다. 확인 모달을 한 번 거친다.

### 5.3 상태 복원

마운트할 때 `GET /categories/classify-proposal`을 한 번 부른다. 상태가 `ready`면
`source_category`를 선택 상태로 되돌려 표를 바로 보여준다. 상태가 `running`이나
`applying`이면 진행률을 복원하고 폴링을 재개한다.

진행률 표시는 기존 자동 분류 스피너 방식을 그대로 쓴다.

## 6. 오류 처리

- 제안 생성 중 개별 파일이 실패하면 `failures`에 남기고 계속 진행한다.
- 승인 적용 중 실패는 항목의 `apply_status`와 `apply_error`에 남긴다. 표에 그대로
  보여주어 사용자가 다시 시도할지 판단한다.
- 승인 시점에 파일이 사라졌으면 실패로 기록한다. 제안은 시점 스냅샷이므로 그
  사이 파일이 바뀔 수 있다.
- 백엔드가 재시작되면 `running`이 굳는다. 하트비트 만료가 `failed`로 바꾼다.

## 7. 테스트

백엔드

- 등급 규칙 8가지 경우를 각각 검증한다.
- 제안 목록에 없는 `file_path`로 승인하면 거부한다.
- 안전하지 않은 카테고리명, 최상위가 아닌 카테고리로 승인하면 거부한다.
- 승인 시점에 파일이 없으면 실패로 기록하고 나머지는 계속 처리한다.
- 사용자가 제안과 다른 목적지를 보내면 그 목적지로 옮긴다.

프론트엔드

- 등급별 체크박스 초기 상태(체크·해제·비활성).
- 전체 선택이 비활성 행을 건너뛴다.
- `unknown` 행에서 목적지를 고르면 체크박스가 활성이 된다.
- 승인 요청 payload에 선택한 행만, 사용자가 고친 목적지로 담긴다.
- 마운트 복원에서 `ready`면 표가 보이고 `running`이면 진행률이 보인다.

## 8. 알려진 비용과 위험

- 제안 생성이 느려진다. 키워드가 맞으면 모델을 건너뛰던 것을 이제 항상 돌린다.
  진행률로 보여주는 것 말고 줄일 방법이 없다.
- 제안은 시점 스냅샷이다. 만드는 동안 파일이 바뀌면 승인 때 실패한다.
- 제안 표가 선택 패널 안에 있어 카테고리를 바꾸면 사라진다. 서버에 남아 있으므로
  같은 카테고리를 다시 고르면 되살아난다.
- `auto_classify` 참조가 소스 약 80곳, 테스트 약 90곳이다. 테스트를 새 흐름으로
  다시 써야 한다.
