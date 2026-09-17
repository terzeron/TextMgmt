# 카테고리 분류 제안·승인 — 설계 및 구현 문서

**브랜치:** `feature/classify-proposal` (develop 대비 커밋 38개)
**설계 문서:** `docs/superpowers/specs/2026-09-16-category-classification-proposal-design.md`
**구현 계획:** `docs/superpowers/plans/2026-09-16-category-classification-proposal.md`
**작성일:** 2026-09-17
**용도:** 코드 리뷰용. 이 문서 하나로 변경의 의도·구조·불변식·한계를 파악할 수 있게 썼다.

---

## 1. 무엇을 바꿨나

기존 `자동 분류` 버튼은 누르는 즉시 카테고리 직하위 파일을 분류하고 **바로 옮겼다.** 관리자가 결과를 미리 볼 수 없었고, 잘못된 판정을 되돌리려면 수동으로 파일을 되돌려야 했다.

이 변경은 그 흐름을 **제안 → 검토 → 승인** 3단계로 쪼갠다.

```
[분류 제안]  →  분류만 하고 결과를 DB에 쌓는다. 파일은 그대로.
     ↓
  표 렌더링   →  책마다 추천 카테고리 1~2개 + 직접 선택 셀렉트박스 + 등급별 체크 상태
     ↓
[분류 승인]  →  체크된 행만 실제로 이동(파일 이동 + ES 갱신)
     ↓
[완료 기록 삭제] → 이동이 끝난 행만 DB에서 제거
```

### 사용자 요구사항과 대응

| #   | 요구사항                            | 구현 위치                                                                     |
| --- | ----------------------------------- | ----------------------------------------------------------------------------- |
| 1   | 버튼 그룹을 키워드 영역 아래로      | `CategoryAdmin.jsx` (커밋 `3e38470`)                                          |
| 2   | 즉시 이동 → 제안·승인 방식          | `book_manager.propose_category_changes` / `apply_category_changes`            |
| 3   | 확실=체크, 애매=해제, 불확실=비활성 | `_propose_category_for_file`의 `grade` + `ClassifyProposalTable.isSelectable` |
| 4   | 제3의 카테고리 수동 지정            | 행별 셀렉트박스 → `proposalTargets` → 승인 payload의 `target_category`        |
| 5   | 추천 카테고리 1~2개 표시            | `candidates` 배열 (최대 2) + `추천 1` / `추천 2` 열                           |
| 6   | 진행 중 상태 표시                   | 건별 DB 적재 + 3초 폴링                                                       |
| 7   | 파일 이동 상태 관리 + 기록 삭제     | `classify_proposal_items.apply_status` + `DELETE .../applied`                 |

---

## 2. 아키텍처

### 책임 분리

| 레이어    | 파일                                     | 책임                                                 | 모르는 것                           |
| --------- | ---------------------------------------- | ---------------------------------------------------- | ----------------------------------- |
| 분류·이동 | `backend/book_manager.py`                | 등급 판정, 후보 조립, 파일 이동, ES 갱신             | DB를 전혀 모른다. 콜백으로만 알린다 |
| 분류 판정 | `backend/book_classifier.py`             | 모델/서점 판정, `confidence`, `bookstore_candidates` | 등급 규칙을 모른다                  |
| 저장      | `backend/category_mapping.py`            | `classify_proposal_items` CRUD                       | 분류 로직을 모른다                  |
| 조율      | `backend/main.py`                        | 엔드포인트, 작업 상태 기계, 콜백 → DB 배선           | —                                   |
| 표시      | `frontend/src/ClassifyProposalTable.jsx` | 순수 표현 컴포넌트(내부 상태 없음)                   | 등급을 다시 계산하지 않는다         |
| 화면 상태 | `frontend/src/CategoryAdmin.jsx`         | 선택·목적지·폴링                                     | —                                   |

`book_manager`가 DB를 모른다는 점이 핵심이다. 건별 기록은 `on_item_done` 콜백으로 `main.py`에 넘기고, `main.py`가 `asyncio.to_thread`로 DB에 쓴다. 콜백이 코루틴을 돌려주면 `book_manager`가 대신 `await` 한다(`inspect.isawaitable`).

### 상태를 어디에 두나

| 무엇                                                 | 어디                                     | 왜                                                               |
| ---------------------------------------------------- | ---------------------------------------- | ---------------------------------------------------------------- |
| 작업 상태(`status`, 카운트, 하트비트, `apply_token`) | JSON 파일 `.classify_proposal_book.json` | 작고, 자주 안 바뀌고, 하트비트 만료 처리가 이미 파일에 묶여 있다 |
| 제안 항목 목록                                       | MySQL `classify_proposal_items`          | 책 수만큼 늘어난다. 파일에 담으면 비용이 제곱이 된다             |

**이 분리는 실측에 근거한다.** 처음에는 항목도 JSON 파일에 담았다. 한 권 기록할 때마다 지금까지의 전체 목록을 다시 직렬화하므로 항목당 0.038ms, 240번째 항목에서 9.2ms가 들었다. 이 코퍼스 최대 카테고리인 `3_판타지`(79,589권)로 환산하면 기록에만 약 **33시간**이다. 간격 제한과 개수 상한으로 덮는 방법을 한 번 커밋했다가(`bfc1822`) 되돌리고(`8360399`), 행 단위 적재로 원인을 없앴다(`210de5c`).

---

## 3. 데이터 모델

```sql
CREATE TABLE IF NOT EXISTS classify_proposal_items (
  id            INT AUTO_INCREMENT PRIMARY KEY,
  content_type  VARCHAR(10)  NOT NULL DEFAULT 'book',
  source_category VARCHAR(255) NOT NULL,
  file_path     VARCHAR(1024) NOT NULL,
  payload       JSON          NOT NULL,
  apply_status  VARCHAR(20)   NOT NULL DEFAULT 'pending',
  apply_error   TEXT          NULL,
  created_at    TIMESTAMP     DEFAULT CURRENT_TIMESTAMP,
  INDEX idx_content_source (content_type, source_category),
  INDEX idx_content_type   (content_type),
  INDEX idx_apply_status   (content_type, apply_status)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci
```

설계 판단:

- **`file_path`에 UNIQUE를 걸지 않았다.** 1024자라 InnoDB 인덱스 길이 제한에 걸린다. 중복은 새 제안 시작 시 `clear_classify_proposal_items`로 관리한다.
- **항목 본문은 `payload` JSON 한 덩어리.** 필드가 여럿이고(`candidates` 배열 등) 앞으로 늘 수 있어, 스키마를 고치지 않고 확장할 수 있게 했다.
- **`apply_status`만 컬럼으로 뺐다.** `DELETE ... WHERE apply_status = 'moved'`가 되어야 해서 JSON 안에 두면 안 된다.
- **기존 환경 마이그레이션.** `CREATE TABLE IF NOT EXISTS`만으로는 이미 만들어진 테이블에 컬럼이 안 생긴다. `_migrate_add_apply_status`가 `information_schema`를 보고 없으면 `ALTER TABLE` 한다. 저장소의 기존 `_migrate_add_content_type` 방식을 그대로 따랐다.

**⚠️ 이 변경은 스키마 변경을 포함한다.** 새 테이블 1개 + 기존 테이블 대상 마이그레이션 1개.

### DB 메서드

| 메서드                                                                                     | 동작                                                            |
| ------------------------------------------------------------------------------------------ | --------------------------------------------------------------- |
| `clear_classify_proposal_items(content_type)`                                              | 해당 content_type 행 전체 삭제                                  |
| `add_classify_proposal_items(items, source_category, content_type)`                        | `executemany` 1회 + commit 1회                                  |
| `get_classify_proposal_items(content_type)`                                                | `id ASC`로 읽어 `payload` + `apply_status` + `apply_error` 반환 |
| `update_classify_proposal_item_status(file_path, apply_status, apply_error, content_type)` | 한 건 갱신                                                      |
| `delete_applied_classify_proposal_items(content_type)`                                     | `apply_status='moved'`만 삭제, 삭제 건수 반환                   |

---

## 4. 등급 판정 규칙

사용자가 정한 규칙이다. **화면에서 다시 계산하지 않는다.** 백엔드가 준 `grade`를 그대로 쓴다.

| 조건                                                     | grade     | 화면                 |
| -------------------------------------------------------- | --------- | -------------------- |
| 키워드가 목적지를 정함 + 모델 점수 높음 + 모델도 같은 답 | `certain` | 체크됨               |
| 키워드가 목적지를 정함 (그 외)                           | `unsure`  | 체크 해제            |
| 키워드 동점 다수                                         | `unknown` | 목적지 없음 → 비활성 |
| 모델 판정 + 점수 높음                                    | `certain` | 체크됨               |
| 서점 다수결(`bookstore_majority`)                        | `certain` | 체크됨               |
| 서점 1곳만(`bookstore_single`)                           | `unsure`  | 체크 해제            |
| 그 외 (점수 낮은 모델 답, 서점 갈림, 정보 없음)          | `unknown` | 목적지 없음 → 비활성 |

### `_is_high_confidence` — 실패 시 닫히게(fail closed)

```python
if confidence is None:
    return False
policy = getattr(classifier_service, "bookstore_policy", None)
if policy is None or getattr(policy, "override_below", 0.0) <= 0.0:
    return False
return not policy.prefers_bookstore(confidence)
```

**리뷰 포인트.** `prefers_bookstore`는 서점 정책 파일이 없으면 `override_below == 0`이라 **모든 점수에 대해 False**를 돌려준다. `not prefers_bookstore(...)`를 그대로 쓰면 점수와 무관하게 전부 `certain`이 되어 자동 체크된다. 정책이 없으면 근거가 없는 것이므로 '낮음'으로 본다. 이 성질은 `test_high_confidence_needs_a_calibrated_boundary`가 잠근다.

### 후보(`candidates`) 조립

최대 2개, 좋은 순서. **불변식: `target_category`가 `None`이 아니면 `candidates[0]["category"]`와 같다.**

| 경로               | candidates                                       |
| ------------------ | ------------------------------------------------ |
| 키워드 단독 최고점 | 1위 + (있으면) 다른 카테고리 1개                 |
| 키워드 동점        | 동점 상위 2개                                    |
| 모델 판정          | 모델 카테고리 1개                                |
| 서점 다수결        | 최다 득표 + 차점                                 |
| 서점 1곳           | 1개                                              |
| 서점 갈림          | 득표 상위 2개                                    |
| 정보 없음          | 낮은 확신도 모델 답이 있으면 1개, 없으면 빈 배열 |

**후보가 2개 있어도 `unknown`은 `unknown`이다.** 시스템이 못 정했다는 사실은 후보를 더 보여준다고 바뀌지 않는다. 사람이 셀렉트박스로 골라야 체크할 수 있다.

---

## 5. API 계약

| 메서드 | 경로                                    | 인증        | 역할                                                                  |
| ------ | --------------------------------------- | ----------- | --------------------------------------------------------------------- |
| POST   | `/categories/classify-proposal`         | `admin_dep` | `{category, use_bookstore, use_content_meta, delay}` — 제안 생성 시작 |
| GET    | `/categories/classify-proposal`         | `admin_dep` | 상태(파일) + 항목(DB) 합쳐서 반환                                     |
| POST   | `/categories/classify-proposal/apply`   | `admin_dep` | `{items: [{file_path, target_category}], clean_existing}`             |
| DELETE | `/categories/classify-proposal`         | `admin_dep` | 제안 폐기(상태 + DB 항목 모두)                                        |
| DELETE | `/categories/classify-proposal/applied` | `admin_dep` | `moved` 행만 삭제, `{deleted_count}`                                  |

GET 응답 항목 하나의 모양:

```
{
  file_path, title, current_category,
  target_category,   // 못 정하면 null
  grade,             // certain | unsure | unknown
  confidence,        // 없으면 null
  candidates: [{category, source, detail}],   // 최대 2
  apply_status,      // pending | moving | moved | failed
  apply_error,
  reason, matched_keywords, model_category, source
}
```

---

## 6. 작업 상태 기계

```
idle ──POST propose──> running ──완료──> ready ──POST apply──> applying ──완료──> done
                          │                                        │
                          └──실패──> failed <───────실패───────────┘
                                       │
                                       └──POST apply(재시도)──> applying
```

- **하트비트 만료:** 상태가 `CLASSIFY_PROPOSAL_STALE_SECONDS = 5분` 넘게 갱신되지 않으면 `stale_running_status`가 중단된 작업으로 보고 `failed`로 굳힌다.
- **승인 수락 조건:** `status ∈ {ready, done, failed}`. `running`/`applying`은 다른 작업이 도는 중이라 거절, `idle`은 승인할 제안이 없다.

  **이 조건이 왜 넓은가 (리뷰 포인트).** 원래는 `ready`일 때만 받았다. 그러면 승인 도중 pod가 죽었을 때 상태가 `applying` → 하트비트 만료 → `failed`로 굳고, 남은 `pending` 행을 다시 승인하려 해도 거절된다. 관리자는 파일이 반쯤 옮겨진 채로 아무것도 할 수 없다. 사용자 승인을 받아 조건을 넓혔다.

- **`applying` 선점 시점:** 엔드포인트가 응답을 돌려주기 **전에** 동기적으로 `applying`을 쓴다. `await` 지점 없이 거기까지 도달하므로, 동시에 들어온 두 번째 apply는 이 쓰기 뒤에 상태를 읽어 거부된다. 선점을 백그라운드 작업 안으로 미루면 응답 반환과 상태 변경 사이에 창이 열린다.

- **`apply_token` (uuid):** 하트비트 만료로 `failed`가 됐는데 원래 작업이 아직 살아 있을 수 있다. 그 상태에서 두 번째 apply가 들어오면 새 토큰을 받은 쪽이 유일한 주인이 되고, 먼저 돌던 작업은 `should_continue`에서 토큰 불일치를 보고 멈춘다. 이게 없으면 진 쪽이 이긴 쪽이 방금 기록한 행을 덮어써 망가뜨린다.

---

## 7. 보안 불변식

**승인 요청은 브라우저가 보낸다. 브라우저가 보낸 값은 아무것도 믿지 않는다.**

| 불변식                                                        | 구현                                                                            |
| ------------------------------------------------------------- | ------------------------------------------------------------------------------- |
| 허용 경로 집합은 **요청 body가 아니라 DB**에서만 만든다       | `apply_classify_proposal`이 `get_classify_proposal_items`로 읽어 `allowed` 구성 |
| `moved` 행은 허용 집합에서 뺀다                               | `apply_status in ("pending", "failed", "moving")`만 담는다                      |
| `file_path`는 제안 목록에 있던 경로인지 재검증                | `file_path_value not in allowed_file_paths` → 거부                              |
| `target_category`는 최상위 카테고리이고 corpus 밖으로 못 나감 | `_is_top_level_target_category` + `_is_safe_category_name`                      |
| 심볼릭 링크는 옮기지 않는다                                   | `absolute_path.is_symlink()` → 거부                                             |
| 실제 경로가 corpus 안인지 확인                                | `resolve(strict=False).is_relative_to(root)`                                    |

**심볼릭 링크 검사가 왜 따로 필요한가.** 제안을 만든 뒤 승인까지 시간차가 있다(TOCTOU). 그 사이 경로가 corpus 밖을 가리키는 링크로 바뀔 수 있다. `is_file()`은 링크를 따라가 True를 주므로, 링크 여부와 실제 위치를 목적지처럼 다시 확인해야 한다. 그러지 않으면 옮기는 건 링크뿐인데 재색인은 링크가 가리키는 파일을 읽어 **corpus 밖 파일이 ES에 들어간다.**

허용 집합을 body에서 만들면 `apply_category_changes`가 가진 "제안 목록에 없는 파일은 거부한다"는 검증이 통째로 무의미해진다. 이건 이 변경의 가장 중요한 보안 불변식이고, 테스트가 잠그고 있다.

---

## 8. 중단 복구 설계

사용자 요구: "파일을 분류된 카테고리 디렉토리로 이동하다가 중단될 수 있으니 파일 이동 상태를 잘 관리하고, 이동이 정상 완료되면 기록 삭제 버튼으로 레코드를 삭제할 수 있도록 제공해."

### 건별 기록

기존에는 전부 옮긴 뒤 결과를 **한꺼번에** 기록했다. 도중에 죽으면 파일은 옮겨졌는데 기록은 그대로다. 한 권 옮길 때마다 바로 기록하면 사라지는 문제다. 승인은 한 권에 파일 이동 + ES 갱신으로 수백 ms가 드니 DB 한 줄 갱신은 무시할 수준이다.

### `moving` 상태가 왜 필요한가

```
파일 이동 직전  →  "moving" 기록
파일 이동 + ES 갱신
성공/실패 확정  →  "moved" / "failed" 기록
```

`moving`을 안 찍고 `pending`으로 두면, 중간에 죽었을 때 그 행이 `pending`으로 남는다. 재개하면 다시 시도하는데 원본은 이미 옮겨져 없다. 그러면 **실제로는 옮겨진 책을 `failed`로 잘못 기록한다.** `moving`은 "시도는 했고 결과를 모른다"를 정직하게 남긴다.

백엔드는 `moving` 행을 자동 판정하지 않는다. 관리자가 직접 확인해야 하는 유일한 상태라, 표에서 다른 색(`warning`) 배지로 구분한다.

### 재시도 경로

`pending` / `failed` / `moving` 행 모두 다시 승인할 수 있다. `moved`만 뺀다. 이미 끝난 이동을 `moving`으로 재시도해도 `apply_category_changes`가 이동 직전 원본 존재를 다시 확인하므로 이중 이동은 일어나지 않는다.

### 기록 삭제

`DELETE /categories/classify-proposal/applied`는 `moved` 행만 지운다. 대기·실패 행은 남겨 재시도하거나 목적지를 고쳐 다시 승인할 수 있게 한다. `running`/`applying` 중에는 거절한다 — 도는 중에 지우면 건별 기록이 방금 찍은 행을 그 작업 밑에서 지울 수 있다.

---

## 9. 진행 상황 표시

사용자 요구: "내가 중간중간에 웹페이지에 접근해서 진행 상황을 살펴볼 텐데, 표에 그런 진행 상태가 나오겠지?"

- `propose_category_changes`의 `on_progress`는 **이번 틱에서 새로 생긴 항목(`new_items`)만** 싣는다. 누적 목록을 매번 통째로 실으면 보고 비용이 책 수의 제곱이 된다(§2의 33시간 문제와 같은 원인).
- `main.py`가 `new_items`를 모아 **10건마다** `add_classify_proposal_items`로 넘긴다(`CLASSIFY_PROPOSAL_ITEMS_BATCH_SIZE = 10`). 마지막 잔여분을 반드시 넘긴다.

  **배치 크기를 100에서 10으로 낮춘 이유.** 이 저장소 카테고리 2,034개의 **중앙값이 5권**이고 대부분이 100권 미만이다. 100이면 흔한 카테고리는 분류가 끝날 때까지 표가 빈 채로 있어, 사용자가 원한 "중간에 들어와서 보기"가 정작 흔한 경우에서 안 된다. 비용은 최대 카테고리(79,589권)에서도 왕복 7,959번, 20초 남짓이다.

- DB 적재는 `asyncio.to_thread`로 이벤트 루프 밖에서 돈다(`94e259c`). 순차 루프 하나이고 각 `on_progress`를 다음 파일 전에 `await` 하므로 순서가 보장된다.
- 화면은 3초 간격 폴링. 종료 상태(`ready`/`done`/`failed`)가 되면 멈춘다. 마운트 시 GET을 한 번 불러 `ready`면 `source_category`를 선택 상태로 복원해 표를 바로 보여준다.

---

## 10. 화면

`ClassifyProposalTable.jsx`는 **순수 표현 컴포넌트**다. 내부 상태가 없고 등급을 다시 계산하지 않는다.

| 열              | 내용                                                                    |
| --------------- | ----------------------------------------------------------------------- |
| 체크박스        | `certain` 체크, `unsure` 해제, 목적지 없거나 `moved`면 비활성           |
| 책              | `title` 없으면 `file_path`                                              |
| 현재            | `current_category`                                                      |
| 추천 1 / 추천 2 | `candidates[0]`, `candidates[1]`. 카테고리 + 근거(`detail`). 없으면 `-` |
| 직접 선택       | 셀렉트박스. 제안된 목적지도 드롭다운에 포함                             |
| 점수            | `confidence`                                                            |
| 이동 상태       | `대기` / `이동 중(중단됨)` / `이동 완료` / `실패: {apply_error}`        |

추가 표시:

- **`후보 동률` 배지** — `grade === "unknown"` 이고 후보가 2개일 때. 근거 문구가 후보마다 득표수만 보여주면 "서로 달랐다"는 사실이 안 보인다.
- **`모델: {model_category}` 배지** — 키워드가 정한 목적지와 모델 판정이 갈릴 때.

### 승인 payload의 2차 방어

```jsx
const items = (proposal?.items || [])
  .filter((item) => proposalSelection.has(item.file_path))
  .filter((item) => isSelectable(item, proposalTargets)) // 2차 방어
  .map((item) => ({
    file_path: item.file_path,
    target_category: proposalTargets[item.file_path] ?? item.target_category,
  }));
```

표가 목적지를 지울 때 선택에서 빼주지만, 제출 직전에 한 번 더 거른다. 목적지 없는 항목이 승인 요청에 실리면 안 된다.

### 제거한 코드

`grep -rn "auto-classify\|autoClassify" frontend/src` 가 **0건**이다. `autoClassifying`, `autoClassifyPolling`, `applyAutoClassifyStatus`, `handleAutoClassifyCategory`, 자동 분류 모달, `categoryAdminUtils.getAutoClassifyRemainingCount`, 그리고 `/categories/auto-classify*` 호출을 모두 걷어냈다. 백엔드의 `auto_classify_category`와 `_classify_file_to_top_category`도 함께 제거했다.

---

## 11. 테스트

| 대상                            | 결과                     |
| ------------------------------- | ------------------------ |
| `pytest tests/`                 | **2057 passed** (64.85s) |
| `cd frontend && npx vitest run` | **1432 passed**          |
| lint                            | clean                    |

### DB 테스트 방침

`tests/test_classify_proposal_items_db.py`는 **testcontainers로 일회용 MySQL을 띄운다.** `tests/conftest.py`의 세션 스코프 `mysql_container` fixture가 `TM_MYSQL_*` 환경변수를 덮어쓴다. 서비스용 DB(`192.168.0.10:30306`)에는 붙지 않는다.

### 고친 테스트 격리 결함 (리뷰 포인트)

`tests/test_category_mapping.py`의 `build_cm()` 헬퍼가 `mock.patch.dict(sys.modules, ...)` 블록 **안에서** `importlib.reload()`를 했다. `patch.dict`는 블록을 나올 때 `sys.modules`만 되돌리고 모듈 자신의 `__dict__`는 안 되돌린다. 그래서 `backend.category_mapping`의 모듈 수준 `pymysql` 이름이 **세션이 끝날 때까지 가짜로 고정됐다.**

당시 피해자는 없었다. 하지만 알파벳 순서상 그 뒤에 실제 DB를 쓰는 테스트 파일이 생기면, 그 파일은 진짜 DB에 붙는 줄 알면서 가짜 커서를 보고 **조용히 통과한다.** 테스트가 아무것도 검증하지 못하는데 초록불이 뜬다.

`try/finally`로 블록을 나올 때 원상 복구하도록 고쳤다(`51f571e`). 확인 방법: `test_classify_proposal_items_db.py` fixture의 방어용 `importlib.reload()`를 **지우고도** 전체 suite가 통과하는지. 지우고 돌려서 통과함을 확인했고, 이중 안전장치로 방어 코드는 남겼다.

---

## 12. 알려진 한계 — 리뷰에서 봐주면 좋을 곳

1. **`moving` 행의 종착지.** 파일이 이미 옮겨진 `moving` 행을 재승인하면 원본이 없어 `failed`가 된다. `failed`는 `완료 기록 삭제` 대상이 아니므로, 그 행을 정리하려면 제안을 새로 만들어야 한다. 의도한 동작이지만(실패를 조용히 지우지 않는다) 관리자 입장에서 막다른 길로 느껴질 수 있다.

2. **`_token_still_valid`가 이벤트 루프에서 파일을 동기로 읽는다.** 항목마다 `_read_classify_proposal()`을 호출한다. 상태 파일은 작지만 `asyncio.to_thread` 밖이다. 항목당 수백 ms가 드는 파일 이동에 비하면 무시할 수준이나, 구조적으로는 일관되지 않다.

3. **`matched_keywords`가 승인 요청에서 빠진다.** `ClassifyApplyItemModel`은 `file_path`와 `target_category`만 받는다(보안상 의도적으로 최소화). `apply_category_changes`가 `item.get("matched_keywords") or []`로 읽으므로 승인 시에는 항상 빈 배열이다. `_move_classified_file`이 이 값을 반환 dict에만 싣고 ES나 메타데이터에는 쓰지 않아 기능 영향은 없으나, 죽은 인자다.

4. **`remaining_count` 시드가 남아 있다.** `classify_proposal_state` 초기값(`backend/main.py:439`)에 있으나 제안 흐름에서는 아무도 읽지 않는다. 같은 이름을 읽는 `categoryAdminUtils.getReloadRemainingCount`는 재적재 상태 전용이고 제안 상태에는 쓰이지 않는다. 상태 파일이 없을 때만 GET 응답에 `remaining_count: 0`이 섞여 나간다.

5. **`DELETE /categories/classify-proposal`에 화면 호출자가 없다.** 엔드포인트와 테스트는 있지만 UI에 "제안 폐기" 버튼을 붙이지 않았다. 새 제안을 시작하면 `clear`가 먼저 돌아 같은 효과가 나기 때문이다.

6. **프론트엔드 테스트가 부하 상황에서 한 번 흔들렸다.** 다른 작업과 동시에 돌렸을 때 1건 실패를 관측했고, 단독으로 4회 연속 돌렸을 때는 모두 통과했다. 원인은 확인하지 못했다. 부하 하 타임아웃으로 추정하나 **미확인**이다.

7. **분류 정확도 자체는 이 변경 범위 밖이다.** 등급 규칙과 모델·서점 판정은 기존 로직 그대로다. 오답의 주된 원인은 카테고리 체계의 중복이며(별도 기록됨) 이번 변경은 그것을 건드리지 않는다.

---

## 13. 배포 시 주의

- **스키마 변경 포함.** 새 테이블 `classify_proposal_items`가 생기고, 이미 그 테이블이 있는 환경에는 `apply_status` / `apply_error` 컬럼과 `idx_apply_status` 인덱스가 추가된다. `_init_database`가 기동 시 수행한다.
- **API 제거.** `/categories/auto-classify`와 `/categories/auto-classify-status`가 없어졌다. 이 브랜치의 프론트엔드와 백엔드는 함께 배포해야 한다.
- **상태 파일 이름이 바뀌었다.** `.classify_proposal_{content_type}.json`. 이전 자동 분류 상태 파일은 더 이상 읽지 않는다.
