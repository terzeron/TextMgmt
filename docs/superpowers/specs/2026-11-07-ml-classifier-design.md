# 머신러닝 도서 자동 분류기 설계

작성일: 2026-11-07 · 브랜치: `feature/ml-classifier`

## 1. 의도

책 카테고리 관리 서브탭의 "분류 제안"은 frontend와 backend 흐름(제안 → 행별 선택 → 분류 승인 → 이동)이 이미 완성돼 있다.
빠진 것은 판정 근거다. 2026-11-04에 모델·학습 CLI·서점 판정을 지운 뒤로 `libs/classifier`는 trait만 남았고,
backend는 카테고리 매핑 키워드만으로 제안하므로 사실상 아무것도 추천하지 않는다.

이 작업은 세 가지를 만든다.

1. 카테고리별 균등 무작위 표본으로 모델을 학습하는 CLI
2. 그 모델로 책 한 권을 분류하는 `Classify` 구현
3. 분류 제안에 ISBN 정확 일치 판정과 모델 판정을 연결하는 backend 조립

서점 분류는 나중에 참고 신호로 추가할 예정이다. 그래서 표본 부족으로 인한 오판정은 이번 범위에서 크게 걱정하지 않는다.

## 2. 목표 · 비목표 · 반목표

**목표**

- 분류 제안을 누르면 책마다 추천 1·2가 채워진다.
- 학습은 CLI 한 번으로 끝나고, 홀드아웃 정답률을 출력한다.
- 모델 파일은 저장소 쪽 별도 디렉토리에 두고 backend 이미지에 넣는다.

**비목표**

- frontend 변경. 기존 응답 필드(`candidates`, `grade`, `confidence`, `expected_accuracy`, `model_category`, `target_category`)를 그대로 채운다.
- 서점 판정 부활. `use_bookstore`는 지금처럼 받기만 한다.
- 카테고리 체계 정리(전집 병합 등). 웹소설 4종은 합치지 않는다는 기존 결정을 유지한다.
- 주기적·자동 재학습.

**반목표 (이러면 실패다)**

- `/mnt/data/text`에 책 이외의 파일(모델, 캐시)을 쓴다.
- 학습 입력에 정답이 새어 들어간다(`file_path`, `category`를 특징으로 쓴다).
- 표본이 균등 무작위가 아니다(ES `random_score`, 앞에서부터 자르기 등).
- 모델 파일이 없을 때 backend가 죽거나 제안 전체가 실패한다.

## 3. 사용자가 정한 것

| 항목        | 결정                                                                      |
| ----------- | ------------------------------------------------------------------------- |
| 학습 대상   | `/mnt/data/text` 아래 카테고리. `0_*`, `9_북스캔OCR`, 루트 낱개 파일 제외 |
| 표본 크기   | 카테고리당 최대 1000권(기본값). 100권과 비교 측정한다                     |
| 학습 입력   | ES 필드 `summary`, `title`, `author`                                      |
| 문서 키     | inode(`book_id`, ES `_id`). `file_path`로 조회하지 않는다                 |
| ISBN        | 모델에는 넣지 않는다. 분류 제안에서 정확 일치 판정에 쓴다                 |
| 우선순위    | ISBN 일치 > 키워드 > 모델                                                 |
| 모델 위치   | 저장소 안 별도 디렉토리. Dockerfile로 이미지에 넣는다                     |
| 클래스 비중 | 실제 권수가 많은 카테고리가 사전확률상 유리해야 한다                      |

## 4. 설계

### 4.1 학습 CLI: `utils/classifier_trainer`

**모집단 열거.** ES composite 집계로 대상 문서의 `(inode, 카테고리)`를 끝까지 받는다.
카테고리는 `file_path`의 첫 경로 조각이다. 제외 규칙(3절)을 여기서 적용한다.
카테고리별 실제 권수도 이때 센다.

**표본 추출.** 카테고리마다 seed를 준 ChaCha 난수로 Fisher–Yates 셔플을 하고 앞에서 N개를 자른다.
이는 균등 비복원 추출이다. N보다 작은 카테고리는 전량을 쓴다.
seed를 주지 않으면 OS 난수로 정하고, 출력과 모델 파일에 기록한다.

**문서 수집.** 뽑힌 inode만 `mget`으로 받아 `summary`·`title`·`author`를 가져온다.
`summary`가 비었으면 그 문서를 빼고 건수를 출력한다.

**홀드아웃.** 카테고리별로 표본의 10%(최소 1권, 표본 5권 미만이면 0권)를 떼어 평가에만 쓴다.

**특징.** 사전 없는 해시 특징을 쓴다. 필드마다 다른 namespace로 해시한다.

- `summary`, `title`: 공백 정규화 뒤 글자 2-gram·3-gram
- `author`: 저자 문자열 전체를 토큰 하나로(여러 명이면 쉼표로 나눠 각각)
- 값은 `log(1 + tf)`, 필드별 L2 정규화

**모델.** 다중 클래스 logistic regression(softmax), L2 정규화, 미니배치 경사 하강.
출력이 확률이라 화면의 확신도 표시와 맞는다.

**사전확률 보정.** 균등 표본은 사전확률을 지운다. 추론 때 logit에 `log(실제 권수 / 학습 권수)`를 더해 되살린다.
같은 유사도면 큰 카테고리가 이기고, 표본을 못 채운 카테고리는 자연스럽게 불리하다.
보정 전·후 홀드아웃 정답률을 둘 다 출력한다. 단, 균등 홀드아웃은 실제 분포가 아니므로 보정 후 수치는 실제 분포 가중 정답률로 함께 계산한다.

**보정값.** 홀드아웃에서 두 값을 구해 모델에 저장한다.

- `min_confidence`: 이 확신도 이상에서 답한 것의 정답률이 90% 이상이 되는 최소 경계
- 확신도 구간별 정답률 표: `expected_accuracy`로 쓴다

**출력.**

- 모델 파일 하나(형식은 4.4)
- 표준 출력: 카테고리 수, 표본 수, seed, top-1·top-2 정답률(보정 전·후), `min_confidence`에서의 판정률·정답률, 카테고리별 정답률

**CLI.** 인자 없이 실행하면 `print_usage()`를 출력한다(저장소 규칙).
주요 옵션: `--output <파일>`, `--per-category <N>`(기본 1000), `--seed <u64>`, `--holdout <비율>`.
ES 접속은 기존 `es_client`의 env(`TM_ES_URL` 등)를 쓴다.

### 4.2 추론: `libs/classifier`

`libs/classifier`는 계약 crate로 유지한다. 여기에 ES를 모르는 순수 모델을 더한다.

- 모델 파일 읽기, 특징 추출(학습과 같은 코드), 점수 계산, 사전확률 보정
- `Classify::classify_document(doc)` 구현: `doc`의 `summary`·`title`·`author`로 점수 순 후보를 낸다
- `min_confidence()`, `expected_accuracy()`는 모델에 저장한 값을 돌려준다
- 특징 추출 코드는 학습 CLI와 이 crate가 공유한다. 두 벌을 두지 않는다

`classify_path(path)`는 ES 조회가 필요하므로 `libs/book_manager` 쪽 어댑터가 맡는다.
경로 → inode(`book_id_for_path`) → ES 문서 조회 → `classify_document` 순서다.
ES에 없는 파일은 `Err("색인되지 않은 파일")`을 돌려주고, 제안은 모델 없이 계속된다.

### 4.3 분류 제안 연결: `libs/book_manager/src/category_proposer.rs`

**ISBN 일치 (새 1순위).** 현재 파일의 ES 문서에서 `isbn`을 읽는다. 값이 있으면 같은 `isbn`의 다른 문서(자기 inode 제외)를 ES `term` 질의로 찾는다.

- 일치 문서의 카테고리가 현재 카테고리와 같으면: "그대로 둠" 근거로만 쓰고 이동 후보로 올리지 않는다.
- 다른 카테고리 하나로 모이면: 그 카테고리를 추천 1, 등급 `certain`.
- 여러 카테고리로 갈리면: 가장 많은 쪽을 추천 1, 다음을 추천 2, 등급 `unsure`.
- 후보 `source`는 `isbn`, `detail`에 일치 건수를 적는다.
- 중복 도서는 의도된 것이다. 이 규칙은 위치 근거로만 쓰고 중복을 지우지 않는다.

**모델 등급 버그 수정.** 현재 `is_high_confidence`는 `override_below() > 0`일 때만 참이다. `ModelOnlyClassifier`는 0을 돌려주므로 모델 판정은 절대 `certain`이 되지 않는다.
또 `ModelOnlyClassifier`는 `min_confidence`·`expected_accuracy`를 모델에 위임하지 않는다. 두 곳을 고쳐, 모델 확신도가 모델의 `min_confidence` 이상이면 `certain`이 되게 한다.

**우선순위.** ISBN 일치 > 키워드 > 모델. 앞 근거가 추천 1을 정하면, 빈 추천 2는 기존 `fill_second_with_model`처럼 다음 근거로 채운다.

### 4.4 모델 파일과 배포

- 위치: 저장소 루트 `models/classifier/model.bin`. 모델 파일은 `.gitignore`에 넣고 `models/classifier/.gitkeep`만 커밋한다(비어 있으면 Docker `COPY`가 실패한다).
- `backend/Dockerfile`: `COPY models/classifier /app/models/classifier`
- backend env `TM_CLASSIFIER_MODEL=/app/models/classifier/model.bin`. `k8s/tm-deployment.yml`에 더한다.
- 파일이 없거나 읽기에 실패하면 경고 로그를 한 번 남기고 키워드·ISBN만으로 제안한다.
- 형식: 작은 JSON 헤더(버전, seed, 생성 시각, 카테고리 목록, 실제·학습 권수, 해시 크기, `min_confidence`, 정답률 표) + 가중치 배열. 목표 크기는 50MB 이하다.
- 모델 로드는 backend 프로세스에서 한 번만 한다. factory는 같은 `Arc`를 돌려준다.

## 5. 제약

- Rust만 쓴다. 저장소에 Python이 없다.
- 새 의존성은 승인 대상이다. 현재 필요하다고 보는 것: `rand`, `rand_chacha` 0.9(이미 `Cargo.lock`에 간접 의존으로 있어 새로 내려받는 crate는 없다).
- 학습 메모리는 수 GB 안이어야 한다. 학습은 사용자 셸에서 foreground로 돌리고, 길면 `systemd-run --user`로 격리한다.
- 테스트는 `tests/rust/<crate>/<모듈>.rs` 1:1, 흐름은 `integration/`. 실행은 `test_runner_rs`로 한다.
- 새 API 엔드포인트는 만들지 않는다. 기존 분류 제안 엔드포인트의 인증 요구사항을 그대로 쓴다.

## 6. 검증

- 단위: 표본 추출의 균등성(작은 모집단에서 반복 추출해 빈도 확인), 같은 seed의 재현성, 특징 해시의 학습·추론 일치, 사전확률 보정, ISBN 규칙 4가지 경우, 모델 없음 경로
- 통합: 가짜 ES로 학습 → 모델 저장 → 로드 → 분류 제안까지
- 실측: 실제 ES로 N=100과 N=1000을 같은 홀드아웃 규칙으로 학습해 정답률 표를 보고한다
- `cargo clippy --workspace --all-targets`, `kubectl apply --dry-run=client -f k8s/tm-deployment.yml`

## 7. 빌더 재량

- 해시 크기, 학습률, epoch 수, L2 계수: 홀드아웃 결과로 정한다. 모델 크기 50MB 이하를 지킨다.
- crate·모듈 이름은 저장소의 "목적어\_행위자" 규칙을 따른다.
- ES 조회(문서 1건, `isbn` term 질의)를 `EsManagerApi`에 더할지, 기존 검색 메서드로 할지.

## 8. 남은 위험

- 정답률은 실측 전까지 모른다. 예전 23만 건 Python 모델의 top-1은 약 79%였다.
- 전집·상위 장르처럼 본문으로 구분이 안 되는 카테고리 쌍은 모델로 풀리지 않는다.
- 모델을 바꾸려면 backend 이미지를 다시 빌드해야 한다.
- ES에 색인되지 않은 파일은 모델·ISBN 추천을 받지 못한다.
