#!/usr/bin/env python3
"""
도서 자동분류 모델 관리 도구

수집 -> 학습 -> 평가 -> 적용 순서로 쓴다. 모든 명령에 `-h` 가 있고,
파라미터마다 범위와 "높이면/낮추면 어떻게 되는지" 를 적어 두었다.
"""

import argparse
import json
import logging
import shutil
import sys
import textwrap
import time
from collections import Counter
from pathlib import Path
from typing import Any, Dict, List, Optional

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from backend.classifier.config import CONFIG_PATH, load_config, merge_config, save_config
from backend.classifier.bookstore_policy import bands_from_cuts, quantile_bands
from backend.classifier.model import MODEL_PATH, CategoryModel

logger = logging.getLogger("classify_cli")

DEFAULT_CORPUS = Path("tmp/classifier/corpus.jsonl")

# 거부 구간 표본을 모으려면 그보다 넓은 풀에서 확신도를 재야 한다. 거부율이 8.5% 라
# 표본의 20배를 훑으면 충분하다.
POOL_FACTOR = 20
# 'boundary' 에서 임계값의 몇 배까지 볼 것인가.
BOUNDARY_FACTOR = 2.0


# ---------------------------------------------------------------------------
# 도움말 문안
# ---------------------------------------------------------------------------

TOP_DESCRIPTION = """\
도서 자동분류 모델 관리 도구

  collect     Elasticsearch 에서 학습 데이터를 모은다
  train       모델을 학습하고 저장한다
  evaluate    저장된 모델의 판정률-정답률 곡선을 본다
  bookstore-policy
              서점 신호를 언제 믿을지 정하는 규칙을 만든다
  classify    파일 하나를 판정하고 근거를 보여준다
  reclassify  디렉토리를 일괄 재분류한다
  info        저장된 모델의 학습 정보를 보여준다

각 명령의 상세 설명은 '<명령> -h' 로 본다.

전형적인 순서:
  1) collect                     ES 에서 23만여 건을 45초에 모은다
  2) train                       학습하고 홀드아웃 성적을 출력한다
  3) reclassify 0_nf             미리보기. 파일을 옮기지 않는다
  4) reclassify 0_nf --apply     실제로 파일을 옮긴다
"""

COLLECT_DESC = """\
Elasticsearch 에서 학습 데이터를 모아 JSONL 로 저장한다.

ES 는 inode 를 _id 로 써서 /mnt/data/text 의 모든 문서를 인덱싱해 두었고,
본문 앞부분(최대 4,096자)·제목·저자·출판사·확장자가 들어 있다. 디스크에서 직접
읽으면 회전 디스크라 몇 시간이 걸리지만 ES 에서는 45초다.

EPUB 만 모으지 않는다. PDF, TXT, HWP 등 ES 가 본문을 가진 포맷을 모두 쓴다.
문서가 몇십 건뿐인 카테고리는 EPUB 으로 제한하면 학습 표본이 남지 않는다.
확장자는 그 자체로 특징 필드(file_type)이기도 하다.
"""

TRAIN_DESC = """\
모델을 학습하고 backend/classifier/model.joblib 에 저장한다.

학습 데이터는 collect 로 미리 모아둔 파일을 쓴다. 16코어에서 10~30분 걸린다.
학습이 끝나면 홀드아웃 성적(판정률-정답률 곡선)을 출력한다.
"""

EVALUATE_DESC = """\
저장된 모델을 홀드아웃으로 다시 재고 판정률-정답률 곡선을 출력한다.

학습 없이 곡선만 보고 싶을 때, 또는 임계값을 어디에 둘지 고를 때 쓴다.
"""

SCORE_CALIBRATION_DESC = """\
화면에 쓸 점수 눈금을 만든다. 모델은 다시 학습하지 않는다.

확신도는 79개 클래스 softmax 라 실제 범위가 0.013~0.13 이다. 화면에서 0.05 와
0.07 중 어느 쪽이 얼마나 나은지 읽을 수가 없다. 홀드아웃에서 (확신도 -> 정답률)
계단을 isotonic 회귀로 재서 모델 파일의 meta 에 넣는다. 그러면 0.076 을
'예상 정답률 0.93' 으로 옮길 수 있다.

판정은 안 바뀐다. 임계값(min_confidence)과 서점 경계는 원래 확신도 눈금 위에
그대로 있고, 이 계단은 표시에만 쓴다. train 은 같은 일을 학습 끝에 자동으로 한다.
이 명령은 이미 학습해 둔 모델에 눈금만 덧붙일 때 쓴다.
"""

BOOKSTORE_POLICY_DESC = """\
서점 신호를 언제 믿을지 정하는 규칙을 만든다.

이름이 'calibrate' 였다. 확률 보정(calibration)을 하는 줄 알기 쉬워 바꿨다.
확률 보정은 train 안에서 한다. 이 명령은 모델과 서점 중 어느 쪽을 따를지
확신도 구간별로 재서 경계 하나를 고른다.

서점 조회는 한 건에 3초 이상 걸려(서점 3곳 × 1초 대기) 23만 건에는 못 쓴다.
그래서 작은 표본에만 조회한다.
"""

CLASSIFY_DESC = """\
파일 하나를 판정하고 근거를 보여준다.

모델이 왜 그렇게 판정했는지 확인할 때 쓴다. 파일을 옮기지 않는다.
"""

RECLASSIFY_DESC = """\
디렉토리 안의 파일을 일괄 재분류한다. 기본은 미리보기이고 파일을 옮기지 않는다.
"""


def _fmt(text: str) -> str:
    return textwrap.dedent(text)


def _help(text: str) -> str:
    """인자 도움말. argparse 가 help 문자열에 %% 치환을 걸므로 백분율 기호를 지킨다."""
    return textwrap.dedent(text).replace("%", "%%")


# ---------------------------------------------------------------------------
# 인자 정의
# ---------------------------------------------------------------------------


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(prog="classify_cli", description=_fmt(TOP_DESCRIPTION), formatter_class=argparse.RawTextHelpFormatter)
    p.add_argument("--verbose", "-v", action="store_true", help="진행 로그를 자세히 출력한다")
    sub = p.add_subparsers(dest="command", metavar="<명령>")

    # -- collect ------------------------------------------------------------
    c = sub.add_parser("collect", help="ES 에서 학습 데이터를 모은다", description=_fmt(COLLECT_DESC), formatter_class=argparse.RawTextHelpFormatter)
    c.add_argument("--out", type=Path, default=DEFAULT_CORPUS, metavar="PATH", help=f"저장할 파일 (기본: {DEFAULT_CORPUS})")
    c.add_argument(
        "--min-chars",
        type=int,
        default=200,
        metavar="N",
        help=_help("""\
            본문이 이 글자 수보다 짧으면 학습에 쓰지 않는다
              기본 200 / 범위 0~4000
              높이면: 본문이 충실한 문서만 배워 정답률이 오른다. 학습 문서가 준다.
              낮추면: 문서가 늘지만 본문이 거의 없는 문서가 잡음으로 들어온다."""),
    )
    c.add_argument(
        "--exclude-prefix",
        action="append",
        default=None,
        metavar="PREFIX",
        help=_help("""\
            이 접두어로 시작하는 카테고리를 뺀다 (여러 번 지정 가능)
              기본 '0_' 와 '9_북스캔OCR'
              0_* 는 미분류/격리 디렉토리라 레이블이 아니고,
              9_북스캔OCR 은 책이 아니라 페이지 이미지라 본문이 없다."""),
    )
    c.add_argument("--index", default=None, metavar="NAME", help="ES 인덱스 이름 (기본: 환경변수 TM_ES_BOOK_INDEX)")
    c.add_argument("--library-root", type=Path, default=None, metavar="PATH", help="라이브러리 최상위. ES 의 file_path 가 이 경로 기준 상대 경로다 (기본: 설정값)")
    c.add_argument(
        "--with-publisher",
        action="store_true",
        help=_help("""\
            ES 에 publisher 가 비어 있는 EPUB 을 파일에서 직접 읽어 메운다
              loader 가 색인할 때 publisher 를 넣으므로, 다시 색인한 뒤에는 필요 없다.
              아직 안 한 문서가 많으면 켤 것. 전집 판정의 결정적 증거다.
              /mnt/data 가 회전 디스크라 6만 건이면 30분 넘게 걸린다.
              읽은 값은 '<출력파일>.publisher.json' 에 남아 다음 실행에서 재사용된다."""),
    )
    c.add_argument(
        "--publisher-workers",
        type=int,
        default=2,
        metavar="N",
        help=_help("""\
            publisher 를 읽을 동시 워커 수
              기본 2 / 범위 1~8
              높이면: 회전 디스크에서는 오히려 느려진다. 실측에서 8개는 1개보다 느렸다.
              낮추면: 1개면 약 30% 느리다."""),
    )

    # -- train --------------------------------------------------------------
    t = sub.add_parser("train", help="모델을 학습하고 저장한다", description=_fmt(TRAIN_DESC), formatter_class=argparse.RawTextHelpFormatter)
    t.add_argument("--corpus", type=Path, default=DEFAULT_CORPUS, metavar="PATH", help=f"학습 데이터 파일 (기본: {DEFAULT_CORPUS})")
    t.add_argument("--out", type=Path, default=MODEL_PATH, metavar="PATH", help=f"저장할 모델 파일 (기본: {MODEL_PATH})")
    t.add_argument(
        "--spill-dir",
        type=Path,
        default=None,
        metavar="PATH",
        help=_help("""\
            벡터화 블록을 이 디렉토리로 흘려보낸다 (기본: 안 함)
              메모리가 좁은 기계에서 쓴다. 필드 블록을 만드는 즉시 디스크에 쓰고
              메모리에서 지운 뒤, 마지막에 한 블록씩 다시 읽어 합친다.
              기준 없이 쓰면 23만 건에서 피크가 9.7GB 라 여유 10GB 기계에서
              죽는다. 대신 디스크를 약 4GB 쓰고 합치는 시간이 더 걸린다."""),
    )
    t.add_argument(
        "--min-per-category",
        type=int,
        default=None,
        metavar="N",
        help=_help("""\
            이 건수 미만인 카테고리는 학습에서 뺀다
              기본 30 / 범위 2~1000
              높이면: 표본이 적어 불안정한 카테고리가 빠져 정답률이 오른다.
                      대신 그 카테고리는 영영 판정 대상에서 빠진다.
              낮추면: 모든 카테고리를 다루지만 소수 카테고리가 오답을 늘린다."""),
    )
    t.add_argument(
        "--body-max-features",
        type=int,
        default=None,
        metavar="N",
        help=_help("""\
            본문 어휘 상한
              기본 300000 / 범위 10000~1000000
              높이면: 드문 단어까지 써서 정답률이 조금 오른다. 메모리와 시간이 는다.
                      500000 이상은 메모리 9GB 환경에서 스왑이 발생한다.
              낮추면: 빠르고 가볍지만 변별력이 떨어진다."""),
    )
    t.add_argument(
        "--body-char-max-chars",
        type=int,
        default=None,
        metavar="N",
        help=_help("""\
            본문 문자 n-gram 이 앞 몇 글자까지 볼 것인가
              기본 1000 / 범위 200~5000
              이 필드가 학습에서 메모리를 가장 많이 쓴다. 글자 수에 비례한다.
              실측(23만 건): 1500 이면 피크 9.7GB 라 여유 10GB 기계에서 죽는다.
              높이면: 본문을 더 넓게 보지만 메모리와 시간이 그만큼 는다.
              낮추면: 가볍지만 뒷부분에만 나오는 단서를 놓친다."""),
    )
    t.add_argument(
        "--body-min-df",
        type=int,
        default=None,
        metavar="N",
        help=_help("""\
            이 문서 수 미만으로 나오는 단어는 버린다
              기본 10 / 범위 1~100
              높이면: 오타와 일회성 고유명사가 걸러져 일반화가 좋아진다.
              낮추면: 시리즈 고유명사를 잡아 전집 판정이 좋아지지만 과적합 위험이 는다."""),
    )
    t.add_argument(
        "--C",
        type=float,
        default=None,
        metavar="FLOAT",
        help=_help("""\
            선형 모델 규제 강도의 역수
              기본 1.0 / 범위 0.01~100
              높이면: 학습 데이터에 더 맞춘다. 과적합해 홀드아웃 정답률이 떨어질 수 있다.
              낮추면: 규제가 세져 모델이 단순해진다. 표본이 적은 카테고리에 유리할 수 있다."""),
    )
    t.add_argument(
        "--class-weight",
        choices=["balanced", "none"],
        default=None,
        help=_help("""\
            카테고리 크기 불균형 보정 (기본 balanced)
              balanced: 크기에 반비례해 가중한다. 3_판타지(78,520건)가
                        소수 카테고리(30건)를 덮는 것을 막는다.
              none:     전체 정답률은 조금 오르지만 소수 카테고리를 거의 못 맞힌다."""),
    )
    t.add_argument(
        "--char-ngram",
        dest="char_ngram",
        action="store_true",
        default=None,
        help=_help("""\
            본문 문자 n-gram 을 쓴다 (기본: 설정값)
              켜면: 조사가 붙어 단어 매칭이 깨지는 한국어 특성을 보완한다.
                    메모리가 2배로 늘고 벡터화가 20분 이상 걸린다.
                    23만 건 학습에서는 --n-jobs 를 3 이하로 같이 낮춰야 한다."""),
    )
    t.add_argument("--no-char-ngram", dest="char_ngram", action="store_false", help="본문 문자 n-gram 을 쓰지 않는다 (빠르고 가볍다)")
    t.add_argument(
        "--holdout",
        type=float,
        default=None,
        metavar="FLOAT",
        help=_help("""\
            평가용으로 떼어둘 비율
              기본 0.2 / 범위 0.05~0.5
              높이면: 평가가 안정되지만 학습 데이터가 준다.
              낮추면: 학습 데이터가 늘지만 측정값의 오차가 커진다."""),
    )
    t.add_argument(
        "--max-iter",
        type=int,
        default=None,
        metavar="N",
        help=_help("""\
            liblinear 이 몇 번까지 반복할 것인가
              기본 5000 / 범위 1000~50000
              높이면: 수렴하지 못한 클래스가 준다. 학습이 그만큼 길어진다.
              낮추면: 빠르지만 마진이 제자리가 아니라 정답률이 깎일 수 있다."""),
    )
    t.add_argument(
        "--n-jobs",
        type=int,
        default=None,
        metavar="N",
        help=_help("""\
            동시에 학습할 카테고리 수
              기본 2 / 범위 1~코어수
              높이면: 빨라지지만 조금만 빨라진다. 워커마다 행렬 크기의 1.4배를 더 쓴다.
                      4만 건 실측: 1개 266초, 2개 176초, 4개 131초, 8개 122초.
                      23만 건에서 8개로 두면 fit 에만 약 15.7GB 가 들어 죽는다.
              낮추면: 메모리가 준다. 1개로 두면 가장 안전하고 2개보다 1.5배 느리다."""),
    )
    t.add_argument("--config", type=Path, default=None, metavar="PATH", help=f"설정 파일 (기본: {CONFIG_PATH})")
    t.add_argument("--save-config", action="store_true", help="이번에 쓴 설정을 config.json 에 저장한다")

    # -- evaluate -----------------------------------------------------------
    e = sub.add_parser("evaluate", help="저장된 모델의 판정률-정답률 곡선을 본다", description=_fmt(EVALUATE_DESC), formatter_class=argparse.RawTextHelpFormatter)
    e.add_argument("--corpus", type=Path, default=DEFAULT_CORPUS, metavar="PATH", help=f"평가 데이터 (기본: {DEFAULT_CORPUS})")
    e.add_argument("--model", type=Path, default=MODEL_PATH, metavar="PATH", help="모델 파일")

    # -- score-calibration --------------------------------------------------
    sc = sub.add_parser("score-calibration", help="화면에 쓸 예상 정답률 눈금을 모델에 넣는다", description=_fmt(SCORE_CALIBRATION_DESC), formatter_class=argparse.RawTextHelpFormatter)
    sc.add_argument("--corpus", type=Path, default=DEFAULT_CORPUS, metavar="PATH", help=f"홀드아웃을 뽑을 데이터 (기본: {DEFAULT_CORPUS})")
    sc.add_argument("--model", type=Path, default=MODEL_PATH, metavar="PATH", help="모델 파일")
    sc.add_argument("--dry-run", action="store_true", help="계단만 보여주고 모델 파일은 건드리지 않는다")

    # -- bookstore-policy ---------------------------------------------------
    cal = sub.add_parser("bookstore-policy", help="서점 신호를 언제 믿을지 정하는 규칙을 만든다", description=_fmt(BOOKSTORE_POLICY_DESC), formatter_class=argparse.RawTextHelpFormatter)
    cal.add_argument("--corpus", type=Path, default=DEFAULT_CORPUS, metavar="PATH", help="표본을 뽑을 데이터")
    cal.add_argument("--model", type=Path, default=MODEL_PATH, metavar="PATH", help="모델 파일")
    cal.add_argument(
        "--sample",
        type=int,
        default=2000,
        metavar="N",
        help=_help("""\
            서점에 조회할 표본 수
              기본 2000 / 범위 200~10000
              높이면: 결합 가중치가 정확해진다. 한 건에 3초 이상 걸려
                      2000건이면 약 100분, 5000건이면 약 250분이 든다.
              낮추면: 빨리 끝나지만 가중치가 표본 오차에 흔들린다."""),
    )
    cal.add_argument(
        "--delay",
        type=float,
        default=1.0,
        metavar="SEC",
        help=_help("""\
            서점 조회 사이에 쉬는 시간
              기본 1.0 / 범위 1.0~5.0
              1.0 미만으로 낮추지 말 것. 서점이 차단할 수 있다.
              높이면: 안전하지만 전체 시간이 비례해 는다."""),
    )
    cal.add_argument(
        "--reband",
        type=Path,
        default=None,
        metavar="PATH",
        help=_help("""\
            저장된 측정 결과의 구간만 다시 나눈다 (서점을 다시 조회하지 않는다)
              건별 결과가 파일에 남아 있어 구간 수를 바꿔 볼 수 있다.
              표본 2,000건 조회에 4시간 35분이 걸렸으므로 재조회는 피한다."""),
    )
    cal.add_argument(
        "--sample-from",
        choices=["refused", "boundary", "all"],
        default="refused",
        help=_help("""\
            표본을 어디서 뽑을 것인가 (기본 refused)
              refused:  모델이 판정을 거부한 구간. 서점을 실제로 쓰는 자리다.
              boundary: 거부 구간과 그 바로 위. 서점이 낮은 확신도의 답을
                        가로챌 만한지도 함께 본다.
              all:      홀드아웃 전체. 52%가 웹소설이라 서점이 불리하게 나온다."""),
    )
    cal.add_argument(
        "--bands",
        type=int,
        default=6,
        metavar="N",
        help=_help("""\
            확신도를 몇 구간으로 나눌 것인가
              기본 6 / 범위 2~20
              경계는 분위수 자리 근처에서 값이 가장 벌어진 곳으로 잡는다.
              높이면: 구간이 촘촘해 경계를 세밀하게 잡지만 구간마다 건수가 줄어 흔들린다."""),
    )
    cal.add_argument("--out", type=Path, default=None, metavar="PATH", help="측정 결과를 저장할 파일")

    # -- classify -----------------------------------------------------------
    cl = sub.add_parser("classify", help="파일 하나를 판정하고 근거를 보여준다", description=_fmt(CLASSIFY_DESC), formatter_class=argparse.RawTextHelpFormatter)
    cl.add_argument("path", type=Path, nargs="+", metavar="PATH", help="판정할 파일 경로")
    cl.add_argument("--model", type=Path, default=MODEL_PATH, metavar="PATH", help="모델 파일")
    cl.add_argument(
        "--min-confidence",
        type=float,
        default=None,
        metavar="F",
        help=_help("""\
            이 확신도 미만이면 판정하지 않는다
              기본: 모델이 학습 때 홀드아웃에서 고른 값 (info 로 확인)
              0.00 으로 두면 확신도와 무관하게 1위 후보를 보여준다."""),
    )
    cl.add_argument("--top", type=int, default=3, metavar="N", help="후보를 몇 개까지 보여줄 것인가 (기본 3)")
    cl.add_argument("--no-es", action="store_true", help="ES 를 쓰지 않고 파일에서 직접 읽는다")
    cl.add_argument("--library-root", type=Path, default=None, metavar="PATH", help="라이브러리 최상위 (기본: 설정값)")

    # -- reclassify ---------------------------------------------------------
    r = sub.add_parser("reclassify", help="디렉토리를 일괄 재분류한다", description=_fmt(RECLASSIFY_DESC), formatter_class=argparse.RawTextHelpFormatter)
    r.add_argument("category", metavar="CATEGORY", help="대상 디렉토리 이름 (예: 0_nf_completed)")
    r.add_argument("--library-root", type=Path, default=Path("/mnt/data/text"), metavar="PATH", help="라이브러리 최상위 (기본: /mnt/data/text)")
    r.add_argument("--model", type=Path, default=MODEL_PATH, metavar="PATH", help="모델 파일")
    r.add_argument(
        "--min-confidence",
        type=float,
        default=None,
        metavar="F",
        help=_help("""\
            이 확신도 미만이면 판정하지 않고 파일을 그대로 둔다
              기본: 모델이 학습 때 홀드아웃에서 고른 값 (info 로 확인)
              높이면: 틀린 이동이 줄지만 손대지 않는 파일이 는다.
              낮추면: 더 많이 옮기지만 오분류가 는다.
              확신도 척도는 데이터마다 다르다. 0.90 같은 값을 임의로 넣으면
              아무것도 안 옮긴다. 바꾸려면 evaluate 의 곡선을 보고 정할 것."""),
    )
    r.add_argument("--apply", action="store_true", help="실제로 파일을 옮긴다 (기본은 미리보기)")
    r.add_argument(
        "--limit",
        type=int,
        default=0,
        metavar="N",
        help=_help("""\
            처음 N 개만 처리한다
              기본 0 (전부) / 범위 0~
              큰 디렉토리에 처음 적용할 때 100 정도로 두고 결과를 확인할 것."""),
    )
    r.add_argument("--no-es", action="store_true", help="ES 를 쓰지 않고 파일에서 직접 읽는다")

    # -- info ---------------------------------------------------------------
    i = sub.add_parser("info", help="저장된 모델의 학습 정보를 보여준다")
    i.add_argument("--model", type=Path, default=MODEL_PATH, metavar="PATH", help="모델 파일")

    return p


# ---------------------------------------------------------------------------
# 공통 도우미
# ---------------------------------------------------------------------------

# ES 가 본문 앞부분을 갖고 있는 포맷 전부. EPUB 만 다루면 문서가 적은 카테고리에서
# 학습 표본이 더 줄어든다. 그림만 있는 포맷(cbz, 이미지)은 본문이 없어 뺀다.
TEXT_EXTS = (".txt", ".epub", ".pdf", ".docx", ".doc", ".hwp", ".rtf", ".html")


def _es_manager(index: Optional[str] = None) -> Any:
    """ESManager 를 만든다. 인덱스 이름을 안 주면 환경변수를 쓴다."""
    from backend.es_manager import ESManager

    return ESManager(index_name=index or "")


def _load_model(path: Path) -> CategoryModel:
    model = CategoryModel.load(path)
    if model is None:
        raise SystemExit(f"모델을 읽지 못했다: {path}\n먼저 'collect' 와 'train' 을 돌릴 것.")
    return model


def _load_corpus(path: Path) -> List[Dict[str, Any]]:
    from backend.classifier.corpus import read_jsonl

    if not path.exists():
        raise SystemExit(f"학습 데이터가 없다: {path}\n먼저 'collect' 를 돌릴 것.")
    docs = read_jsonl(path)
    if not docs:
        raise SystemExit(f"학습 데이터가 비었다: {path}")
    return docs


def _publisher_cache_path(corpus_path: Path) -> Path:
    return corpus_path.with_suffix(corpus_path.suffix + ".publisher.json")


def _overrides(args: argparse.Namespace) -> Dict[str, Any]:
    """CLI 로 준 값만 설정에 덮어쓴다. 안 준 값은 config.json 의 값을 그대로 둔다."""
    over: Dict[str, Any] = {}

    def put(path: List[str], value: Any) -> None:
        if value is None:
            return
        node = over
        for key in path[:-1]:
            node = node.setdefault(key, {})
        node[path[-1]] = value

    put(["corpus", "min_per_category"], getattr(args, "min_per_category", None))
    put(["model", "C"], getattr(args, "C", None))
    put(["model", "n_jobs"], getattr(args, "n_jobs", None))
    put(["model", "max_iter"], getattr(args, "max_iter", None))
    put(["holdout"], getattr(args, "holdout", None))
    if getattr(args, "class_weight", None) is not None:
        put(["model", "class_weight"], None if args.class_weight == "none" else args.class_weight)
    for field in ("body_word", "body_char"):
        put(["fields", field, "max_features"], getattr(args, "body_max_features", None))
        put(["fields", field, "min_df"], getattr(args, "body_min_df", None))
    put(["fields", "body_char", "max_chars"], getattr(args, "body_char_max_chars", None))
    if getattr(args, "char_ngram", None) is not None:
        put(["fields", "body_char", "enabled"], bool(args.char_ngram))
    return over


def _holdout_docs(docs: List[Dict[str, Any]], config: Dict[str, Any]) -> List[Dict[str, Any]]:
    """학습 때와 같은 규칙으로 홀드아웃을 다시 떼어낸다.

    문서 키의 해시로 나누므로, 코퍼스에 문서가 들고 나도 각 문서의 소속은 안 바뀐다.
    예전처럼 위치 인덱스로 나누면 한 건만 바뀌어도 전혀 다른 집합이 떨어져,
    학습에 쓴 문서가 홀드아웃에 섞이고 정답률이 부풀려진다.
    """
    from backend.classifier.training import filter_by_size, split_holdout

    kept, _ = filter_by_size(docs, config["corpus"]["min_per_category"])
    _, hold = split_holdout(kept, float(config["holdout"]), int(config["seed"]))
    return hold


def _iter_files(root: Path) -> List[Path]:
    return sorted(p for p in root.rglob("*") if p.is_file() and p.suffix.lower() in TEXT_EXTS)


# ---------------------------------------------------------------------------
# 명령 구현
# ---------------------------------------------------------------------------


def cmd_collect(args: argparse.Namespace) -> int:
    from backend.classifier.corpus import attach_publishers, collect, read_jsonl, write_jsonl

    cfg = load_config()
    prefixes = args.exclude_prefix if args.exclude_prefix is not None else cfg["corpus"]["excluded_prefixes"]
    library_root = args.library_root or cfg["corpus"]["library_root"]
    t0 = time.time()
    stats = collect(_es_manager(args.index), args.out, min_chars=args.min_chars, excluded_prefixes=prefixes, index=args.index, label_pattern=cfg["corpus"]["label_pattern"])
    print(f"수집 {stats['kept']:,}건 / 제외 {stats['skipped']:,}건 / 카테고리 {stats['categories']}개  ({time.time() - t0:.0f}초)")

    if args.with_publisher:
        docs = read_jsonl(args.out)
        t0 = time.time()
        pstats = attach_publishers(docs, _publisher_cache_path(args.out), library_root, workers=args.publisher_workers)
        write_jsonl(docs, args.out)
        print(f"publisher: 새로 읽음 {pstats['read']:,}건, 값이 있는 문서 {pstats['with_publisher']:,}건  ({time.time() - t0:.0f}초)")

    top = list(stats["per_category"].items())
    print("\n  상위 10개 카테고리")
    for cat, n in top[:10]:
        print(f"  {n:>8,}  {cat}")
    if len(top) > 10:
        print("  ... 하위 10개")
        for cat, n in top[-10:]:
            print(f"  {n:>8,}  {cat}")
    print(f"\n저장: {args.out}")
    return 0


def cmd_train(args: argparse.Namespace) -> int:
    from backend.classifier.config import SERIES_TO_PARENT
    from backend.classifier.training import format_report, train

    cfg = merge_config(load_config(args.config), _overrides(args))
    docs = _load_corpus(args.corpus)
    if not any(d.get("publisher") for d in docs):
        logger.warning("학습 데이터에 publisher 가 하나도 없다. 'collect --with-publisher' 로 다시 모으면 전집 판정이 좋아진다.")

    model, report = train(docs, cfg, accept_parent=SERIES_TO_PARENT, spill_dir=args.spill_dir)
    path = model.save(args.out)
    size_mb = path.stat().st_size / 1024 / 1024
    if args.spill_dir:
        # 합쳐 둔 행렬은 5GB 를 넘는다. 모델을 저장한 뒤에는 쓸 데가 없다.
        shutil.rmtree(args.spill_dir, ignore_errors=True)

    print(format_report(report))
    print()
    print(f"모델 저장: {path}  ({size_mb:.0f}MB, 특징 {model.meta['features']:,}개, 학습 {model.meta['train_seconds']:.0f}초)")
    for name, info in model.meta["blocks"].items():
        print(f"  {name:<10} 특징 {info['features']:>8,}  가중치 {info['weight']}")
    if args.save_config:
        print(f"설정 저장: {save_config(cfg)}")
    return 0


def cmd_evaluate(args: argparse.Namespace) -> int:
    from backend.classifier.config import SERIES_TO_PARENT
    from backend.classifier.training import evaluate, format_report

    model = _load_model(args.model)
    docs = _load_corpus(args.corpus)
    hold = _holdout_docs(docs, model.config)
    print(format_report(evaluate(model, hold, accept_parent=SERIES_TO_PARENT)))
    return 0


def cmd_score_calibration(args: argparse.Namespace) -> int:
    """이미 학습한 모델에 '확신도 -> 예상 정답률' 눈금을 넣는다.

    재학습이 아니다. 판정만 다시 돌려 계단을 재고 meta 에 붙인 뒤 같은 경로에 저장한다.
    """
    from backend.classifier.calibration import expected_accuracy
    from backend.classifier.config import SERIES_TO_PARENT
    from backend.classifier.training import fit_confidence_calibration

    model = _load_model(args.model)
    docs = _load_corpus(args.corpus)
    hold = _holdout_docs(docs, model.config)
    print(f"홀드아웃 {len(hold):,}건으로 눈금을 잰다")

    calibration = fit_confidence_calibration(model, hold, accept_parent=SERIES_TO_PARENT)
    if calibration is None:
        print("표본이 모자라 눈금을 만들지 못했다")
        return 1

    print(f"  계단 {len(calibration['x'])}칸, 표본 {calibration['n']:,}건")
    print("  확신도   예상 정답률")
    for point in (0.02, 0.04, 0.056, 0.076, 0.10, 0.13):
        value = expected_accuracy(calibration, point)
        print(f"  {point:>6.3f}   {value:.3f}")

    if args.dry_run:
        print("--dry-run 이라 모델 파일은 그대로 둔다")
        return 0

    model.meta["confidence_calibration"] = calibration
    model.save(args.model)
    print(f"모델에 눈금을 넣었다: {args.model}")
    return 0


def cmd_bookstore_policy(args: argparse.Namespace) -> int:
    """
    서점 신호를 언제 믿을지 잰다.

    모델 확신도 구간마다 "모델이 맞은 비율"과 "서점이 맞은 비율"을 따로 세어,
    서점 쪽이 더 나은 구간의 상한을 찾는다. 결합 규칙은 그 상한 하나다.
    """
    import random

    from backend.book_classifier import BookClassifierService, clean_filename_to_author_title
    from backend.classifier.config import resolve_parent

    if args.reband:
        return _reband(args)

    model = _load_model(args.model)
    docs = _load_corpus(args.corpus)
    hold = _holdout_docs(docs, model.config)
    random.Random(model.config["seed"]).shuffle(hold)

    # 서점을 실제로 쓰는 자리에서 재야 한다.
    #
    # 홀드아웃 전체에서 고르게 뽑으면 52%가 웹소설이 된다. 서점은 웹소설을 거의 안
    # 다뤄 정답률이 낮게 나오는데, 정작 서점이 필요한 자리는 모델이 답을 못 하는
    # 8.5%(논픽션에 몰려 있다)다. 실측에서 그 8.5%의 판정률을 서점이 45% -> 95%로
    # 끌어올렸다. 균등 표본은 그 효과를 못 본다.
    pool_size = max(args.sample * POOL_FACTOR, args.sample)
    pool = hold[:pool_size]
    pool_preds = model.predict(pool, min_confidence=0.0)
    floor = model.min_confidence
    if args.sample_from == "refused":
        picked = [(d, p) for d, p in zip(pool, pool_preds) if p.confidence < floor]
        where = f"모델이 판정을 거부한 구간(확신도 < {floor:.3f})"
    elif args.sample_from == "boundary":
        # 거부 구간과 그 바로 위. 서점이 낮은 확신도의 모델 답을 가로챌 만한지도 함께 본다.
        picked = [(d, p) for d, p in zip(pool, pool_preds) if p.confidence < floor * BOUNDARY_FACTOR]
        where = f"거부 구간과 그 위 (확신도 < {floor * BOUNDARY_FACTOR:.3f})"
    else:
        picked = list(zip(pool, pool_preds))
        where = "홀드아웃 전체"

    if not picked:
        raise SystemExit(f"{where} 에 해당하는 문서가 없다. --sample-from 을 바꿀 것")
    sample = [d for d, _ in picked[: args.sample]]
    preds = [p for _, p in picked[: args.sample]]
    print(f"표본 {len(sample):,}건을 {where} 에서 뽑았다 (후보 {len(picked):,}건 / 풀 {len(pool):,}건)", flush=True)

    service = BookClassifierService(delay=args.delay)

    rows: List[Dict[str, Any]] = []
    t0 = time.time()
    for i, (doc, pred) in enumerate(zip(sample, preds), 1):
        raw_author, raw_title, search_title = clean_filename_to_author_title(doc.get("name") or "")
        y, a, k = service.query_bookstores(search_title, raw_author, raw_title)
        votes = [v for v in (y.get("mapped"), a.get("mapped"), k.get("mapped")) if v]
        store_cat = None
        if votes:
            top, n = Counter(votes).most_common(1)[0]
            store_cat = top if n >= 2 or len(votes) == 1 else None
        truth = doc["cat"]
        rows.append({"confidence": pred.confidence, "model_ok": resolve_parent(pred.ranked[0][0]) == resolve_parent(truth), "store_cat": store_cat, "store_ok": bool(store_cat) and resolve_parent(store_cat) == resolve_parent(truth), "store_answered": bool(store_cat)})
        if i % 100 == 0:
            rate = i / (time.time() - t0)
            print(f"  {i}/{len(sample)}  {rate * 60:.0f}건/분  남은 시간 약 {(len(sample) - i) / rate / 60:.0f}분", flush=True)

    # 구간은 고정값이 아니라 확신도 분포에서 고른다. 고정값으로 자르면 2,000건 중
    # 1,884건이 한 구간에 몰렸다(실측). 분위수 자리 근처에서 값이 가장 벌어진 곳을
    # 경계로 삼아, 사실상 같은 문서가 두 구간으로 갈리지 않게 한다.
    cuts = quantile_bands([r["confidence"] for r in rows], count=args.bands)
    report: List[Dict[str, Any]] = []
    print("\n  확신도 구간   건수   모델 정답률   서점 응답률   서점 정답률")
    for lo, hi in bands_from_cuts(cuts):
        band = [r for r in rows if lo <= r["confidence"] < hi]
        if not band:
            continue
        answered = [r for r in band if r["store_answered"]]
        m_acc = sum(r["model_ok"] for r in band) / len(band)
        s_rate = len(answered) / len(band)
        s_acc = (sum(r["store_ok"] for r in answered) / len(answered)) if answered else 0.0
        report.append({"low": lo, "high": hi, "n": len(band), "model_accuracy": m_acc, "store_coverage": s_rate, "store_accuracy": s_acc})
        print(f"  {lo:.3f}~{hi:.3f} {len(band):>5}   {m_acc:>9.1%}   {s_rate:>9.1%}   {s_acc:>9.1%}")

    # 경계는 실제로 잰 범위를 넘으면 안 된다.
    #
    # 거부 구간에서만 뽑으면 마지막 구간의 상한이 1.01(열린 위쪽 끝)이라, 모든 구간에서
    # 서점이 이겼을 때 경계가 1.01 로 나온다. 그대로 쓰면 재지도 않은 고확신 구간까지
    # 서점이 가로챈다. 표본에서 본 가장 높은 확신도로 자른다.
    measured_max = max((r["confidence"] for r in rows), default=0.0)
    better = [b for b in report if b["store_accuracy"] > b["model_accuracy"]]
    threshold = min(max((b["high"] for b in better), default=0.0), measured_max)
    out = {
        "sample": len(rows),
        "sample_from": args.sample_from,
        # 이 확신도까지만 쟀다. 경계는 여기를 못 넘는다.
        "measured_max_confidence": measured_max,
        "bands": report,
        # 이 확신도 미만일 때만 서점 판정으로 갈아탄다. 0 이면 서점이 이긴 구간이 없다는 뜻이다.
        "bookstore_override_below": threshold,
        # 건별 결과를 남긴다. 구간을 다시 나눌 때 서점을 다시 조회하지 않아도 된다.
        # 실측에서 표본 2,000건 조회에 4시간 35분이 걸렸다.
        "rows": rows,
    }
    print(f"\n  서점으로 갈아탈 확신도 상한: {threshold:.3f}" + ("  (서점이 모델을 이긴 구간이 없다)" if threshold == 0.0 else ""))
    dest = args.out or args.model.parent / "bookstore_policy.json"
    with Path(dest).open("w", encoding="utf-8") as f:
        json.dump(out, f, ensure_ascii=False, indent=2)
    print(f"저장: {dest}")
    return 0


def _summarize_policy(rows: List[Dict[str, Any]], band_count: int) -> Dict[str, Any]:
    """건별 결과를 구간으로 묶어 서점 경계를 고른다. 서점을 조회하지 않는다."""
    cuts = quantile_bands([r["confidence"] for r in rows], count=band_count)
    report: List[Dict[str, Any]] = []
    print("\n  확신도 구간   건수   모델 정답률   서점 응답률   서점 정답률")
    for lo, hi in bands_from_cuts(cuts):
        band = [r for r in rows if lo <= r["confidence"] < hi]
        if not band:
            continue
        answered = [r for r in band if r["store_answered"]]
        m_acc = sum(r["model_ok"] for r in band) / len(band)
        s_rate = len(answered) / len(band)
        s_acc = (sum(r["store_ok"] for r in answered) / len(answered)) if answered else 0.0
        report.append({"low": lo, "high": hi, "n": len(band), "model_accuracy": m_acc, "store_coverage": s_rate, "store_accuracy": s_acc})
        print(f"  {lo:.3f}~{hi:.3f} {len(band):>5}   {m_acc:>9.1%}   {s_rate:>9.1%}   {s_acc:>9.1%}")

    # 경계는 실제로 잰 범위를 넘으면 안 된다. 거부 구간만 뽑으면 마지막 구간의 상한이
    # 1.01(열린 위쪽 끝)이라, 모든 구간에서 서점이 이길 때 경계가 1.01 로 나온다.
    # 그대로 쓰면 재지도 않은 고확신 구간까지 서점이 가로챈다.
    measured_max = max((r["confidence"] for r in rows), default=0.0)
    better = [b for b in report if b["store_accuracy"] > b["model_accuracy"]]
    threshold = min(max((b["high"] for b in better), default=0.0), measured_max)
    print(f"\n  서점으로 갈아탈 확신도 상한: {threshold:.3f}" + ("  (서점이 모델을 이긴 구간이 없다)" if threshold == 0.0 else f"  (측정 범위 {measured_max:.3f} 까지)"))
    return {"sample": len(rows), "measured_max_confidence": measured_max, "bands": report, "bookstore_override_below": threshold, "rows": rows}


def _reband(args: argparse.Namespace) -> int:
    """저장된 건별 결과로 구간만 다시 나눈다."""
    path = Path(args.reband)
    if not path.exists():
        raise SystemExit(f"측정 결과 파일이 없다: {path}")
    with path.open(encoding="utf-8") as f:
        saved = json.load(f)
    rows = saved.get("rows") or []
    if not rows:
        raise SystemExit(f"건별 결과가 없어 다시 나눌 수 없다: {path}\n서점 조회부터 다시 해야 한다.")

    out = _summarize_policy(rows, args.bands)
    out["sample_from"] = saved.get("sample_from", "unknown")
    dest = args.out or path
    with Path(dest).open("w", encoding="utf-8") as f:
        json.dump(out, f, ensure_ascii=False, indent=2)
    print(f"저장: {dest}")
    return 0


def cmd_classify(args: argparse.Namespace) -> int:
    from backend.classifier import BookCategoryClassifier

    model = _load_model(args.model)
    es = None if args.no_es else _es_manager()
    clf = BookCategoryClassifier(model=model, es_manager=es, library_root=args.library_root)

    for path in args.path:
        if not path.exists():
            print(f"{path}: 파일이 없다")
            continue
        doc = clf.build_document(path)
        pred = clf.classify_document(doc, min_confidence=args.min_confidence)
        source = "ES" if doc.get("id") else "파일"
        print(f"\n{path}")
        print(f"  특징 출처: {source}  본문 {len(doc.get('text') or ''):,}자  제목 '{doc.get('title') or '-'}'  저자 '{doc.get('author') or '-'}'  publisher '{doc.get('publisher') or '-'}'")
        print(f"  판정: {pred.category or '판정 안 함'}")
        print(f"  근거: {pred.reason}")
        for rank, (cat, prob) in enumerate(pred.ranked[: args.top], 1):
            print(f"    {rank}. {cat:<24} {prob:.3f}")
    return 0


def cmd_reclassify(args: argparse.Namespace) -> int:
    import shutil

    from backend.classifier import BookCategoryClassifier

    source_dir = args.library_root / args.category
    if not source_dir.is_dir():
        raise SystemExit(f"디렉토리가 없다: {source_dir}")

    model = _load_model(args.model)
    clf = BookCategoryClassifier(model=model, es_manager=None if args.no_es else _es_manager(), library_root=args.library_root)

    files = _iter_files(source_dir)
    if args.limit:
        files = files[: args.limit]
    print(f"대상 {len(files):,}건  ({source_dir})")
    print("미리보기다. 파일을 옮기지 않는다." if not args.apply else "실제로 파일을 옮긴다.")

    moved = skipped = failed = 0
    targets: Counter = Counter()
    for i, fpath in enumerate(files, 1):
        pred = clf.classify_path(fpath, min_confidence=args.min_confidence)
        if not pred.category:
            skipped += 1
        else:
            targets[pred.category] += 1
            dest = args.library_root / pred.category / fpath.name
            if args.apply:
                if dest.exists():
                    skipped += 1
                    continue
                try:
                    dest.parent.mkdir(parents=True, exist_ok=True)
                    shutil.move(str(fpath), str(dest))
                    moved += 1
                except OSError as e:
                    logger.warning("옮기지 못했다 (%s): %s", fpath, e)
                    failed += 1
            else:
                moved += 1
                if moved <= 20:
                    print(f"  {pred.confidence:.3f}  {pred.category:<22} <- {fpath.name}")
        if i % 500 == 0:
            print(f"  {i:,}/{len(files):,} 처리", flush=True)

    verb = "옮김" if args.apply else "옮길 예정"
    print(f"\n{verb} {moved:,}건 / 판정 안 함 {skipped:,}건" + (f" / 실패 {failed:,}건" if failed else ""))
    print("\n  카테고리별 건수")
    for cat, n in targets.most_common(15):
        print(f"  {n:>6,}  {cat}")
    if not args.apply:
        print("\n실제로 옮기려면 --apply 를 붙일 것.")
    return 0


def cmd_info(args: argparse.Namespace) -> int:
    from backend.classifier.training import format_report

    model = _load_model(args.model)
    meta = model.meta
    print(f"모델 파일  {args.model}  ({args.model.stat().st_size / 1024 / 1024:.0f}MB)")
    print(f"학습 시각  {meta.get('trained_at', '-')}")
    print(f"설정 지문  {meta.get('config_hash', '-')}")
    print(f"학습 문서  {meta.get('documents', 0):,}건 / 카테고리 {len(meta.get('categories', []))}개 / 특징 {meta.get('features', 0):,}개")
    dropped = meta.get("dropped_categories") or []
    if dropped:
        print(f"제외 카테고리 {len(dropped)}개: {', '.join(dropped[:10])}" + (" ..." if len(dropped) > 10 else ""))
    print(f"판정 임계값 {model.min_confidence:.2f}  온도 {model.temperature}")
    print("\n  필드          특징      가중치")
    for name, info in (meta.get("blocks") or {}).items():
        print(f"  {name:<12} {info['features']:>8,}  {info['weight']}")
    if meta.get("holdout"):
        print()
        print(format_report(meta["holdout"]))
    return 0


COMMANDS = {"collect": cmd_collect, "train": cmd_train, "evaluate": cmd_evaluate, "score-calibration": cmd_score_calibration, "bookstore-policy": cmd_bookstore_policy, "classify": cmd_classify, "reclassify": cmd_reclassify, "info": cmd_info}


def main(argv: Optional[List[str]] = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    if not args.command:
        parser.print_help()
        return 1
    logging.basicConfig(level=logging.DEBUG if args.verbose else logging.INFO, format="%(asctime)s %(levelname)s %(message)s", datefmt="%H:%M:%S")
    return COMMANDS[args.command](args)


if __name__ == "__main__":
    sys.exit(main())
