#!/usr/bin/env python3
"""서점 신호 규칙 테스트

두 가지를 검증한다.
1. 매핑 우선순위 - 저자·시리즈 규칙 > DB 키워드 > 하드코딩 규칙
2. 결합 규칙 - 측정된 경계 아래에서는 서점을 모델보다 먼저 본다
"""

import json

import pytest

from backend.classifier.bookstore_policy import BookstorePolicy, keyword_map, map_bookstore_category, match_keyword, reset_keyword_cache

# 실제 DB 에 들어 있는 값에서 뽑았다. 하드코딩 규칙과 다른 4건이 포함돼 있다.
DB_KEYWORDS = {"한문": "1_동양고전한문", "영어전문교육": "7_영어교육", "자녀교육": "9_어린이육아", "반도체": "8_IT", "클래식": "5_음악"}


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


def test_ambiguous_keyword_is_dropped(monkeypatch):
    """같은 키워드가 두 카테고리에 있으면 쓰지 않는다.

    실제 DB 에 `환상문학`(3_SF, 3_판타지)과 `클래식`(1_서양고전, 5_음악)이 그렇다.
    하나를 고르는 규칙은 자의적이라 하드코딩 규칙에 넘기는 편이 낫다.
    """
    import backend.classifier.bookstore_policy as mod

    class FakeMapping:
        def get_all_mappings(self, content_type="book"):
            return {"3_SF": ["환상문학", "과학소설"], "3_판타지": ["환상문학"], "5_음악": ["클래식"]}

    monkeypatch.setattr(mod, "CategoryMapping", FakeMapping, raising=False)
    monkeypatch.setitem(__import__("sys").modules, "backend.category_mapping", type("M", (), {"CategoryMapping": FakeMapping}))

    loaded = mod._load_from_db()
    assert "환상문학" not in loaded
    assert loaded["과학소설"] == "3_SF"
    assert loaded["클래식"] == "5_음악"


def test_match_keyword_prefers_the_longer_word():
    # '영어전문교육' 이 '영어' 보다 구체적이다. 짧은 쪽이 먼저 맞으면 7_교육일반 으로 샌다.
    keywords = {"영어": "7_영어교육", "영어전문교육": "7_영어교육", "교육": "7_교육일반"}
    assert match_keyword("외국어 > 영어전문교육", keywords) == "7_영어교육"
    assert match_keyword("", keywords) is None
    assert match_keyword("소설 > 한국소설", {}) is None


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
