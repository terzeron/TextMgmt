#!/usr/bin/env python3
"""서점 신호 규칙 테스트

두 가지를 검증한다.
1. 매핑 우선순위 - 저자·시리즈 규칙 > DB 키워드 > 하드코딩 규칙
2. 결합 규칙 - 측정된 경계 아래에서는 서점을 모델보다 먼저 본다
"""

import json

import pytest

from backend.classifier.bookstore_policy import BookstorePolicy, bands_from_cuts, keyword_map, map_bookstore_candidates, map_bookstore_category, match_keyword, quantile_bands, reset_keyword_cache

# 실제 DB 에 들어 있는 값에서 뽑았다. 하드코딩 규칙과 다른 4건이 포함돼 있다.
DB_KEYWORDS = {"한문": ["1_동양고전한문"], "영어전문교육": ["7_영어교육"], "자녀교육": ["9_어린이육아"], "반도체": ["8_IT"], "클래식": ["1_서양고전", "5_음악"], "환상문학": ["3_SF", "3_판타지"]}


@pytest.fixture(autouse=True)
def clean_cache():
    reset_keyword_cache()
    yield
    reset_keyword_cache()


# ---------------------------------------------------------------------------
# 키워드 맵
# ---------------------------------------------------------------------------


def test_keyword_map_caches_and_reloads_on_demand():
    calls = []

    def loader():
        calls.append(1)
        return dict(DB_KEYWORDS)

    assert keyword_map(loader) == DB_KEYWORDS
    keyword_map(loader)
    assert len(calls) == 1, "5분 안에는 DB 를 다시 읽지 않는다"

    keyword_map(loader, force=True)
    assert len(calls) == 2


def test_keyword_map_survives_a_dead_database():
    """DB 가 죽어도 판정은 계속돼야 한다. 빈 맵을 주고 하드코딩 규칙으로 넘어간다."""

    def broken():
        raise RuntimeError("MySQL 연결 실패")

    assert keyword_map(broken) == {}


def test_keyword_with_two_categories_keeps_both(monkeypatch):
    """한 키워드가 여러 카테고리에 붙는 것은 정상이다.

    테이블의 유일 키가 (category, keyword, content_type) 조합이고 search_by_keyword 도
    목록을 돌려준다. `환상문학`(3_SF, 3_판타지)과 `클래식`(1_서양고전, 5_음악)이 그렇다.
    어느 쪽인지는 서점 다수결이 가른다.
    """
    import backend.classifier.bookstore_policy as mod

    class FakeMapping:
        def get_all_mappings(self, content_type="book"):
            return {"3_SF": ["환상문학", "과학소설"], "3_판타지": ["환상문학"], "5_음악": ["클래식"]}

    monkeypatch.setitem(__import__("sys").modules, "backend.category_mapping", type("M", (), {"CategoryMapping": FakeMapping}))

    loaded = mod._load_from_db()
    assert sorted(loaded["환상문학"]) == ["3_SF", "3_판타지"]
    assert loaded["과학소설"] == ["3_SF"]


def test_multi_category_keyword_gives_no_single_answer():
    # 후보가 둘이면 한 서점만으로는 못 고른다. 다수결로 넘긴다.
    assert map_bookstore_category("예술 > 클래식", "", "", DB_KEYWORDS) is None
    assert sorted(map_bookstore_candidates("예술 > 클래식", "", "", DB_KEYWORDS)) == ["1_서양고전", "5_음악"]


def test_match_keyword_prefers_the_longer_word():
    # '영어전문교육' 이 '영어' 보다 구체적이다. 짧은 쪽이 먼저 맞으면 7_교육일반 으로 샌다.
    keywords = {"영어": ["7_영어교육"], "영어전문교육": ["7_영어교육"], "교육": ["7_교육일반"]}
    assert match_keyword("외국어 > 영어전문교육", keywords) == ["7_영어교육"]
    assert match_keyword("", keywords) == []
    assert match_keyword("소설 > 한국소설", {}) == []


# ---------------------------------------------------------------------------
# 매핑 우선순위
# ---------------------------------------------------------------------------


def test_series_rule_wins_over_db_keyword():
    """히가시노 게이고는 '일본소설' 이기 전에 2_소설일본게이고 다."""
    keywords = {"일본소설": "2_소설일본"}
    assert map_bookstore_category("소설 > 일본소설", "용의자 X의 헌신", "히가시노 게이고", keywords) == "2_소설일본게이고"


def test_db_keyword_wins_over_the_hardcoded_rule():
    """DB 가 더 구체적인 4건. 하드코딩은 한문을 1_동양고전 으로 보낸다."""
    assert map_bookstore_category("고전 > 한문", "", "", DB_KEYWORDS) == "1_동양고전한문"
    assert map_bookstore_category("외국어 > 영어전문교육", "", "", DB_KEYWORDS) == "7_영어교육"
    assert map_bookstore_category("가정 > 자녀교육", "", "", DB_KEYWORDS) == "9_어린이육아"


def test_db_keyword_fills_what_the_hardcoded_rule_misses():
    # '반도체' 는 하드코딩 규칙에 없다. DB 가 메운다.
    assert map_bookstore_category("공학 > 반도체", "", "", DB_KEYWORDS) == "8_IT"


def test_hardcoded_rule_covers_what_the_db_does_not():
    """DB 는 35개 카테고리뿐이다. 나머지 38개는 하드코딩 규칙이 맡는다."""
    assert map_bookstore_category("소설 > 무협소설", "", "", DB_KEYWORDS) == "3_무협"
    assert map_bookstore_category("소설 > 라이트노벨", "", "", DB_KEYWORDS) == "3_라이트노벨"


def test_unknown_category_maps_to_nothing():
    assert map_bookstore_category("듣도 보도 못한 분야", "", "", DB_KEYWORDS) is None


# ---------------------------------------------------------------------------
# 결합 규칙
# ---------------------------------------------------------------------------


def test_policy_without_a_file_never_prefers_the_bookstore():
    """규칙 파일이 없으면 예전 동작이다. 모델이 늘 먼저다."""
    policy = BookstorePolicy.load("/없는/경로/bookstore_policy.json")
    assert policy.override_below == 0.0
    assert policy.prefers_bookstore(0.001) is False


def test_policy_reads_the_measured_boundary(tmp_path):
    path = tmp_path / "bookstore_policy.json"
    path.write_text(json.dumps({"sample": 2000, "bookstore_override_below": 0.08, "bands": [{"low": 0.0, "high": 0.5}]}), encoding="utf-8")

    policy = BookstorePolicy.load(path)
    assert policy.override_below == 0.08
    assert policy.sample == 2000
    assert policy.prefers_bookstore(0.05) is True
    assert policy.prefers_bookstore(0.08) is False, "경계값은 모델 쪽이다"


def test_policy_tolerates_a_broken_file(tmp_path):
    path = tmp_path / "bookstore_policy.json"
    path.write_text("{ 이건 JSON 이 아니다", encoding="utf-8")
    assert BookstorePolicy.load(path).override_below == 0.0


# ---------------------------------------------------------------------------
# 판정 경로에 실제로 적용되는가
# ---------------------------------------------------------------------------


class StubPrediction:
    def __init__(self, category, confidence, reason="stub"):
        self.category = category
        self.confidence = confidence
        self.reason = reason
        self.ranked = [(category or "-", confidence)]


class StubClassifier:
    def __init__(self, pred):
        self.pred = pred

    def __bool__(self):
        return True

    def classify_path(self, fpath, min_confidence=None):
        return self.pred

    def classify_document(self, doc, min_confidence=None):
        return self.pred


def _service(tmp_path, classifier, policy):
    from backend.book_classifier import BookClassifierService

    return BookClassifierService(library_root=tmp_path, cache_file=tmp_path / "cache.json", classifier=classifier, bookstore_policy=policy)


def _entry(store_cat="3_무협", title="달빛조각사"):
    return {"search_title": title, "yes24": {"mapped": store_cat, "title": title}, "aladin": {"mapped": store_cat, "title": title}, "kyobo": {}}


def test_bookstore_wins_below_the_measured_boundary(tmp_path):
    """모델이 답을 내도 확신도가 경계 아래면 서점을 따른다."""
    model_answered = StubClassifier(StubPrediction("3_판타지", 0.05))
    service = _service(tmp_path, model_answered, BookstorePolicy(override_below=0.08))

    cat, method, reason = service._decide(None, "달빛조각사.txt", _entry())
    assert cat == "3_무협"
    assert method == "bookstore_majority"
    assert "서점 우선" in reason


def test_model_wins_above_the_boundary(tmp_path):
    model_answered = StubClassifier(StubPrediction("3_판타지", 0.20))
    service = _service(tmp_path, model_answered, BookstorePolicy(override_below=0.08))

    cat, method, _ = service._decide(None, "달빛조각사.txt", _entry())
    assert cat == "3_판타지"
    assert method == "model"


def test_bookstore_still_fills_the_gap_when_the_model_refuses(tmp_path):
    """경계가 0 이어도 모델이 판정을 거부하면 서점이 메운다. 예전 동작이다."""
    model_refused = StubClassifier(StubPrediction(None, 0.03, "확신도 부족"))
    service = _service(tmp_path, model_refused, BookstorePolicy(override_below=0.0))

    cat, method, _ = service._decide(None, "달빛조각사.txt", _entry())
    assert cat == "3_무협"
    assert method == "bookstore_majority"


def test_bookstore_is_queried_once_even_when_the_policy_prefers_it(tmp_path, monkeypatch):
    """서점 조회는 한 건에 3초 이상 걸린다. 규칙을 적용해도 두 번 부르면 안 된다."""
    model_refused = StubClassifier(StubPrediction(None, 0.01))
    service = _service(tmp_path, model_refused, BookstorePolicy(override_below=0.08))

    calls = []
    original = service._decide_by_bookstore

    def counting(entry, trust_single_match=True):
        calls.append(1)
        return original(entry, trust_single_match=trust_single_match)

    monkeypatch.setattr(service, "_decide_by_bookstore", counting)
    service._decide(None, "달빛조각사.txt", _entry())
    assert len(calls) == 1


# ---------------------------------------------------------------------------
# 확신도 구간
# ---------------------------------------------------------------------------


def test_quantile_bands_split_where_samples_are_sparse():
    """경계는 값이 촘촘한 곳이 아니라 벌어진 곳에 잡혀야 한다.

    고정 구간(0.5/0.7/0.8)으로 자르면 2,000건 중 1,884건이 한 칸에 몰렸다.
    """
    # 0.10 부근과 0.50 부근에 뭉쳐 있고 그 사이가 비어 있다
    dense_low = [0.10 + i * 0.0001 for i in range(50)]
    dense_high = [0.50 + i * 0.0001 for i in range(50)]
    cuts = quantile_bands(dense_low + dense_high, count=2)

    assert len(cuts) == 1
    # 빈 구간 안에서 잘라야 한다. 뭉친 값 사이를 가르면 안 된다.
    assert 0.1050 < cuts[0] < 0.50


def test_quantile_bands_handle_tiny_samples():
    assert quantile_bands([], count=4) == []
    assert quantile_bands([0.3], count=4) == []


def test_bands_from_cuts_covers_the_whole_range():
    bands = bands_from_cuts([0.05, 0.12])
    assert bands[0][0] == 0.0
    assert bands[-1][1] > 1.0
    # 구간이 끊기지 않고 이어져야 한다
    assert [hi for _, hi in bands[:-1]] == [lo for lo, _ in bands[1:]]


def test_overlapping_candidate_wins_the_vote(tmp_path):
    """서점마다 후보를 여럿 내도 겹치는 카테고리가 이긴다.

    `클래식` 은 1_서양고전 이자 5_음악 이다. 다른 서점이 5_음악 쪽을 가리키면 그쪽으로 굳는다.
    """
    model_refused = StubClassifier(StubPrediction(None, 0.01))
    service = _service(tmp_path, model_refused, BookstorePolicy())

    entry = {
        "search_title": "사계",
        "yes24": {"candidates": ["1_서양고전", "5_음악"], "title": "사계"},
        "aladin": {"candidates": ["5_음악"], "title": "사계"},
        "kyobo": {},
    }
    cat, method, _ = service._decide(None, "사계.epub", entry)
    assert cat == "5_음악"
    assert method == "bookstore_majority"


def test_single_store_with_two_candidates_refuses(tmp_path):
    """한 곳만 답했는데 후보가 둘이면 고를 근거가 없다. 수동으로 넘긴다."""
    model_refused = StubClassifier(StubPrediction(None, 0.01))
    service = _service(tmp_path, model_refused, BookstorePolicy())

    entry = {"search_title": "사계", "yes24": {"candidates": ["1_서양고전", "5_음악"], "title": "사계"}, "aladin": {}, "kyobo": {}}
    cat, method, _ = service._decide(None, "사계.epub", entry, trust_single_match=True)
    assert cat is None
    assert method == "conflict"
