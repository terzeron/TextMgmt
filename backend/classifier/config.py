#!/usr/bin/env python3
"""
분류기 설정 - 필드별 가중치와 학습 파라미터

가중치와 임계값은 실험에 따라 자주 바뀐다. 코드가 아니라 `config.json` 에 두고
`classify_cli train` 을 다시 돌리면 반영되게 한다. 파일이 없으면 아래 기본값을 쓴다.
"""

import copy
import hashlib
import json
import logging
from pathlib import Path
from typing import Any, Dict

logger = logging.getLogger(__name__)

CONFIG_PATH = Path(__file__).resolve().parent / "config.json"

# 필드 하나의 설정 형태.
#   source       학습 데이터의 어느 키에서 원문을 가져오는가
#   weight       이 블록에 곱하는 스케일. 선형 모델이라 그 필드의 영향력을 직접 조절한다.
#   analyzer     "word" 또는 "char_wb"
#   ngram        [최소, 최대]. analyzer 가 word 면 보통 [1, 1]
#   min_df       이 문서 수 미만으로 나오는 특징은 버린다
#   max_features 상위 몇 개까지 쓸 것인가
#   max_chars    원문 앞 몇 글자만 볼 것인가 (0 이면 전부)
DEFAULT_CONFIG: Dict[str, Any] = {
    "version": 1,
    "corpus": {
        # ES 의 file_path 는 이 디렉토리를 기준으로 한 상대 경로다.
        "library_root": "/mnt/data/text",
        # ES summary 가 이보다 짧으면 학습에 쓰지 않는다. 본문이 없는 셈이다.
        "min_chars": 200,
        # 표본이 이보다 적은 카테고리는 학습에서 뺀다.
        "min_per_category": 30,
        # 레이블로 쓸 디렉토리는 '숫자_이름' 꼴이다. 0_* 는 미분류/격리라 뺀다.
        # 이 규칙 하나가 trash, _root, .preview_cache 같은 비카테고리 디렉토리를 함께 거른다.
        # 실측에서 그 셋이 789건 섞여 들어왔다.
        "label_pattern": r"^[1-9]_",
        # 위 꼴이지만 레이블이 아닌 것. 9_북스캔OCR 은 책이 아니라 페이지 이미지다.
        "excluded_prefixes": ["9_북스캔OCR"],
    },
    "fields": {
        "body_word": {"enabled": True, "source": "text", "weight": 1.0, "analyzer": "word", "ngram": [1, 1], "min_df": 10, "max_features": 300000, "max_chars": 0},
        "body_char": {"enabled": True, "source": "text", "weight": 1.0, "analyzer": "char_wb", "ngram": [2, 4], "min_df": 20, "max_features": 300000, "max_chars": 1500},
        "filename": {"enabled": True, "source": "name", "weight": 1.0, "analyzer": "char_wb", "ngram": [2, 4], "min_df": 3, "max_features": 200000, "max_chars": 0},
        "title": {"enabled": True, "source": "title", "weight": 1.0, "analyzer": "char_wb", "ngram": [2, 4], "min_df": 3, "max_features": 100000, "max_chars": 0},
        "author": {"enabled": True, "source": "author", "weight": 1.0, "analyzer": "char_wb", "ngram": [2, 4], "min_df": 3, "max_features": 100000, "max_chars": 0},
        # 전집 판정의 결정적 증거. 을유문화사는 2_을유세계문학전집 의 98.6% 에 있고
        # 상위 장르 2_소설외국 에는 0.0% 다. 다른 필드에 묻히지 않게 가중치를 올려 둔다.
        "publisher": {"enabled": True, "source": "publisher", "weight": 3.0, "analyzer": "word", "ngram": [1, 2], "min_df": 2, "max_features": 50000, "max_chars": 0},
        "file_type": {"enabled": True, "source": "type", "weight": 0.5, "analyzer": "word", "ngram": [1, 1], "min_df": 1, "max_features": 100, "max_chars": 0},
    },
    "model": {
        # 규제 강도의 역수. 높이면 학습 데이터에 더 맞추고 과적합 위험이 는다.
        "C": 1.0,
        # 카테고리 크기가 78,520 대 30 까지 벌어진다. 보정하지 않으면 소수 카테고리를 못 맞힌다.
        "class_weight": "balanced",
        # OvR 을 몇 개까지 동시에 학습할 것인가. liblinear 자체는 단일 스레드다.
        # 워커마다 행렬 크기의 1.4배를 더 쓴다(training.py 의 실측표 참고).
        # 23만 건에서 8 로 두면 fit 에만 15.7GB 가 들어 죽는다. 2 는 2.2배 느리고 안전하다.
        "n_jobs": 2,
    },
    "decision": {
        # 이 확신도 미만이면 판정하지 않는다.
        #
        # None 이면 학습이 홀드아웃에서 임계값을 직접 고른다. 확신도는 마진을 softmax 로
        # 누른 값이라 척도가 데이터에 따라 달라진다. 실측에서 정답률 90% 지점의 임계값은
        # 0.056 이었고, 0.90 같은 값을 고정해 두면 아무것도 판정하지 못한다.
        # 숫자를 넣으면 그 값을 그대로 쓴다.
        "min_confidence": None,
        # 임계값을 고를 때 지킬 정답률. 0.90 이면 정답률 90% 를 유지하는 최대 판정률 지점.
        "target_accuracy": 0.90,
        # 마진을 확률로 바꿀 때 쓰는 온도. 크면 확률이 평탄해져 판정률이 준다.
        "temperature": 1.0,
    },
    "holdout": 0.2,
    "seed": 20260914,
}


def load_config(path: Path | str | None = None) -> Dict[str, Any]:
    """설정을 읽는다. 파일이 없거나 깨졌으면 기본값을 쓴다."""
    p = Path(path) if path else CONFIG_PATH
    cfg = copy.deepcopy(DEFAULT_CONFIG)
    if not p.exists():
        return cfg
    try:
        with p.open(encoding="utf-8") as f:
            user = json.load(f)
    except Exception as e:
        logger.warning("설정 파일을 읽지 못해 기본값을 쓴다 (%s): %s", p, e)
        return cfg
    return merge_config(cfg, user)


def merge_config(base: Dict[str, Any], override: Dict[str, Any]) -> Dict[str, Any]:
    """override 의 값으로 base 를 덮어쓴다. 딕셔너리는 한 단계씩 내려가며 합친다."""
    out = copy.deepcopy(base)
    for k, v in override.items():
        if isinstance(v, dict) and isinstance(out.get(k), dict):
            out[k] = merge_config(out[k], v)
        else:
            out[k] = copy.deepcopy(v)
    return out


def save_config(cfg: Dict[str, Any], path: Path | str | None = None) -> Path:
    p = Path(path) if path else CONFIG_PATH
    p.parent.mkdir(parents=True, exist_ok=True)
    with p.open("w", encoding="utf-8") as f:
        json.dump(cfg, f, ensure_ascii=False, indent=2)
    return p


def config_hash(cfg: Dict[str, Any]) -> str:
    """설정의 지문. 모델 파일에 같이 넣어 어떤 설정으로 학습했는지 되짚는다."""
    payload = json.dumps(cfg, ensure_ascii=False, sort_keys=True)
    return hashlib.sha256(payload.encode("utf-8")).hexdigest()[:16]


def enabled_fields(cfg: Dict[str, Any]) -> Dict[str, Dict[str, Any]]:
    return {k: v for k, v in cfg["fields"].items() if v.get("enabled")}


# 하위 카테고리를 상위 장르로 접는 표.
#
# 전집은 상위 장르의 부분집합이다. `2_을유세계문학전집`의 책은 모두 `2_소설외국`이기도 하다.
# 모델이 상위 장르를 내놓고 정답이 전집일 때 이를 오답으로 세면 분류 체계의 중복을
# 모델 결함으로 잘못 읽는다. 평가에서는 둘 다 정답으로 친다.
SERIES_TO_PARENT: Dict[str, str] = {
    "2_을유세계문학전집": "2_소설외국",
    "2_열린책들세계문학": "2_소설외국",
    "2_동서문화사월드북": "2_소설외국",
    "2_문예세계문학선": "2_소설외국",
    "2_소설Abe전집": "2_소설일본",
    "2_소설일본게이고": "2_소설일본",
    "2_소설일본하루키": "2_소설일본",
    "3_SF그리폰북스": "3_SF",
    "3_SF환상문학전집": "3_SF",
    "3_SF직지": "3_SF",
    "3_SF영문": "3_SF",
    "3_셜록홈즈": "3_스릴러",
    "4_살림지식총서": "4_인문일반논픽션",
    "4_시공디스커버리": "4_인문일반논픽션",
    "5_이지사이언스": "5_수학과학일반",
}


def resolve_parent(category: str | None) -> str | None:
    """하위 카테고리면 상위 장르를, 아니면 그대로 돌려준다."""
    if not category:
        return category
    return SERIES_TO_PARENT.get(category, category)
