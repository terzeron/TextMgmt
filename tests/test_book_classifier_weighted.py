"""evaluate_category_decision 의 가중치 합 판정 계층에 대한 테스트"""

import math

import pytest

from backend.book_classifier import ACCEPT_MARGIN, ACCEPT_MIN_SCORE, LEXICON_MIN_WORDS, WEIGHT_EPUB_SUBJECT, SignalAccumulator, evaluate_category_decision, score_text_with_lexicon
from backend.category_lexicon import CategoryLexicon, get_kiwi

# 경제 어휘로만 구성한 중립 본문. 기존 5대 장르 스코어링이 반응하지 않아
# 어휘 사전 신호만 따로 관찰할 수 있다.
ECON_TEXT = (
    "금리 인상과 환율 변동이 물가에 미치는 영향을 살펴본다. "
    "통화정책 당국은 기준금리를 조정하여 총수요를 관리한다. "
    "재정정책은 조세와 정부지출을 통해 경기를 조절한다. "
    "실업률과 고용지표는 노동시장의 상태를 보여준다. "
    "무역수지와 경상수지는 대외거래의 결과를 기록한다. "
    "주식시장의 주가지수는 기업실적과 배당을 반영한다. "
    "채권수익률 곡선은 경기전망의 선행지표로 쓰인다. "
    "가계부채와 저축률은 소비여력을 결정하는 변수다. "
    "생산성 증가율은 잠재성장률의 핵심 요인이다. "
    "인플레이션 기대심리는 임금협상에도 작용한다. "
) * 3


@pytest.fixture(scope="module")
def kiwi():
    k = get_kiwi(num_workers=1)
    if k is None:
        pytest.skip("kiwipiepy 미설치")
    return k


def make_lexicon(categories):
    data = {"version": 1, "categories": {}}
    for cat, words in categories.items():
        norm = math.sqrt(sum(v * v for v in words.values())) or 1.0
        data["categories"][cat] = {"doc_count": 100, "norm": round(norm, 6), "words": words, "unique": []}
    return CategoryLexicon(data)


@pytest.fixture
def econ_lexicon(kiwi, monkeypatch):
    """ECON_TEXT 가 확실히 4_경제 로 판정되는 사전을 기본 사전 자리에 꽂는다"""
    from backend.category_lexicon import extract_word_set

    words = extract_word_set(ECON_TEXT, kiwi=kiwi)
    assert len(words) >= LEXICON_MIN_WORDS, f"테스트 본문의 어휘가 {len(words)}개로 부족하다"
    lex = make_lexicon({"4_경제": {w: 0.8 for w in sorted(words)}, "3_무협": {"무림": 0.9, "내공": 0.8}})
    monkeypatch.setattr("backend.book_classifier.get_default_lexicon", lambda: lex)
    return lex


# ---------------------------------------------------------------------------
# SignalAccumulator
# ---------------------------------------------------------------------------


def test_accumulator_with_no_signal_abstains():
    assert SignalAccumulator().decide() is None


def test_accumulator_ignores_empty_category_and_nonpositive_weight():
    acc = SignalAccumulator()
    acc.add(None, 3.0, "lexicon", "r")
    acc.add("", 3.0, "lexicon", "r")
    acc.add("3_무협", 0.0, "lexicon", "r")
    acc.add("3_무협", -1.0, "lexicon", "r")
    assert acc.decide() is None


def test_accumulator_abstains_below_minimum_score():
    acc = SignalAccumulator()
    acc.add("3_무협", ACCEPT_MIN_SCORE - 0.1, "store_vote", "r")
    assert acc.decide() is None


def test_accumulator_accepts_at_minimum_score():
    acc = SignalAccumulator()
    acc.add("3_무협", ACCEPT_MIN_SCORE, "content_metadata", "r")
    decision = acc.decide()
    assert decision is not None
    assert decision[0] == "3_무협"


def test_accumulator_abstains_when_margin_is_too_tight():
    acc = SignalAccumulator()
    acc.add("3_무협", 3.0, "content_metadata", "r")
    acc.add("3_판타지", 3.0 / ACCEPT_MARGIN + 0.01, "lexicon", "r")
    assert acc.decide() is None


def test_accumulator_accepts_when_margin_is_met():
    acc = SignalAccumulator()
    acc.add("3_무협", 3.0, "content_metadata", "r")
    acc.add("3_판타지", 3.0 / ACCEPT_MARGIN - 0.01, "lexicon", "r")
    decision = acc.decide()
    assert decision is not None
    assert decision[0] == "3_무협"


def test_weak_signals_accumulate_into_a_decision():
    """혼자서는 기준에 못 미치는 신호도 같은 방향으로 모이면 판정이 선다"""
    acc = SignalAccumulator()
    acc.add("3_판타지", 1.0, "store_vote", "Bookstore vote")
    acc.add("3_판타지", 1.2, "lexicon", "Corpus lexicon")
    decision = acc.decide()
    assert decision is not None
    assert decision[0] == "3_판타지"


def test_method_label_follows_cascade_priority_not_weight():
    """점수가 큰 신호가 아니라 캐스케이드 순서상 앞선 신호가 method 를 정한다"""
    acc = SignalAccumulator()
    acc.add("3_판타지", 2.0, "lexicon", "Corpus lexicon -> 3_판타지")
    acc.add("3_판타지", 1.2, "conflict_resolved", "Conflict resolved by keywords -> 3_판타지")
    cat, method, reason = acc.decide()
    assert cat == "3_판타지"
    assert method == "conflict_resolved"
    assert "Conflict resolved" in reason


def test_unknown_method_label_sorts_last():
    acc = SignalAccumulator()
    acc.add("3_판타지", 2.0, "미등록신호", "unknown source")
    acc.add("3_판타지", 0.5, "lexicon", "Corpus lexicon -> 3_판타지")
    _cat, method, _reason = acc.decide()
    assert method == "lexicon"


def test_reason_carries_score_breakdown():
    acc = SignalAccumulator()
    acc.add("3_무협", 2.6, "content_metadata", "EPUB dc:subject -> 3_무협")
    acc.add("3_판타지", 1.0, "store_vote", "Bookstore vote -> 3_판타지")
    _cat, _method, reason = acc.decide()
    assert "EPUB dc:subject" in reason
    assert "weighted 2.60 vs 1.00" in reason
    assert "content_metadata+2.6" in reason


# ---------------------------------------------------------------------------
# score_text_with_lexicon
# ---------------------------------------------------------------------------


def test_lexicon_scoring_without_dictionary_returns_empty(monkeypatch):
    monkeypatch.setattr("backend.book_classifier.get_default_lexicon", lambda: None)
    assert score_text_with_lexicon(ECON_TEXT) == []


def test_lexicon_scoring_skips_short_text(econ_lexicon):
    assert score_text_with_lexicon("금리 인상") == []
    assert score_text_with_lexicon("") == []


def test_lexicon_scoring_skips_text_with_too_few_words(econ_lexicon):
    # 200자는 넘지만 서로 다른 단어는 몇 개 안 되는 본문
    assert score_text_with_lexicon("금리 환율 물가 " * 40) == []


def test_lexicon_scoring_ranks_matching_category_first(econ_lexicon):
    ranked = score_text_with_lexicon(ECON_TEXT)
    assert ranked
    assert ranked[0][0] == "4_경제"
    assert ranked[0][1] == pytest.approx(1.0)


# ---------------------------------------------------------------------------
# evaluate_category_decision 통합
# ---------------------------------------------------------------------------


def write_txt(tmp_path, name, text):
    p = tmp_path / name
    p.write_text(text, encoding="utf-8")
    return p


def test_lexicon_breaks_a_two_store_tie(tmp_path, econ_lexicon):
    """서점 두 곳이 1표씩 갈릴 때 어휘 사전이 승부를 가른다"""
    fpath = write_txt(tmp_path, "경제서적.txt", ECON_TEXT)
    cat, method, reason = evaluate_category_decision("경제서적.txt", fpath, "경제서적", "", "경제서적", {"mapped": "4_경제"}, {"mapped": "3_무협"}, {"mapped": None})
    assert cat == "4_경제"
    assert "Corpus lexicon" in reason or "lexicon" in reason


def test_lexicon_alone_classifies_when_no_other_signal(tmp_path, econ_lexicon):
    """서점 결과가 전혀 없어도 본문 어휘만으로 판정한다"""
    fpath = write_txt(tmp_path, "무제.txt", ECON_TEXT)
    cat, method, _reason = evaluate_category_decision("무제.txt", fpath, "무제", "", "무제", {"mapped": None}, {"mapped": None}, {"mapped": None})
    assert cat == "4_경제"
    assert method == "lexicon"


def test_strong_metadata_signal_outranks_contradicting_lexicon(tmp_path, econ_lexicon, monkeypatch):
    """EPUB dc:subject(2.6)는 어휘 사전(2.0)보다 1.25배 이상 앞서므로 이긴다"""
    fpath = tmp_path / "메타.epub"
    fpath.write_bytes(b"dummy")
    monkeypatch.setattr("backend.book_classifier.inspect_epub_metadata", lambda _p: {"title": "", "subject": "무협소설", "description": ""})
    monkeypatch.setattr("backend.book_classifier.extract_content_first_1000_words", lambda _p: ECON_TEXT)

    cat, method, reason = evaluate_category_decision("메타.epub", fpath, "메타", "", "메타", {"mapped": None}, {"mapped": None}, {"mapped": None})
    assert cat == "3_무협"
    assert method == "content_metadata"
    assert "EPUB dc:subject" in reason
    assert WEIGHT_EPUB_SUBJECT >= ACCEPT_MIN_SCORE


def test_missing_lexicon_leaves_legacy_behaviour_intact(tmp_path, monkeypatch):
    """사전 파일이 없으면 기존 신호만으로 판정한다"""
    monkeypatch.setattr("backend.book_classifier.get_default_lexicon", lambda: None)
    fpath = write_txt(tmp_path, "무협지.txt", "소림사의 장로와 화산파 문도들이 강호의 평화를 위해 내공을 수련했다. 단전과 기경팔맥. " * 5)
    cat, method, _reason = evaluate_category_decision("무협지.txt", fpath, "무협지", "", "무협지", {"mapped": None}, {"mapped": None}, {"mapped": None})
    assert cat == "3_무협"
    assert method == "content_metadata"
