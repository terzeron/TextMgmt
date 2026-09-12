import gzip
import json
import math

import pytest

from backend.category_lexicon import MIN_SIMILARITY, UNIQUE_BONUS, CategoryLexicon, extract_nouns, extract_word_set, get_kiwi, is_meaningful_word


@pytest.fixture(scope="module")
def kiwi():
    k = get_kiwi(num_workers=1)
    if k is None:
        pytest.skip("kiwipiepy 미설치")
    return k


def make_lexicon(categories):
    data = {"version": 1, "categories": {}}
    for cat, (words, unique) in categories.items():
        norm = math.sqrt(sum(v * v for v in words.values())) or 1.0
        data["categories"][cat] = {"doc_count": 100, "norm": round(norm, 6), "words": words, "unique": unique}
    return CategoryLexicon(data)


# ---------------------------------------------------------------------------
# 단어 추출
# ---------------------------------------------------------------------------


def test_is_meaningful_word_filters_short_and_symbolic():
    assert is_meaningful_word("무림")
    assert is_meaningful_word("가") is False
    assert is_meaningful_word("dungeon")
    assert is_meaningful_word("ab") is False
    assert is_meaningful_word("12") is False


def test_extract_nouns_merges_contiguous_compound_nouns(kiwi):
    words = extract_nouns("그는 무림맹의 장문인으로서 내공을 운기조식하며 화산파 검법을 익혔다.", kiwi=kiwi)
    # 병합하지 않으면 '운기/조/식', '장/문인' 으로 부서져 도메인 용어가 사라진다
    assert "운기조식" in words
    assert "장문인" in words
    assert "무림맹" in words
    assert "화산파" in words
    # 조사와 용언은 남지 않는다
    assert "는" not in words
    assert "익히" not in words


def test_extract_nouns_keeps_domain_terms_across_genres(kiwi):
    assert "거시경제학" in extract_nouns("이 논문은 거시경제학의 통화정책을 분석한다.", kiwi=kiwi)
    assert "몬스터" in extract_nouns("던전에서 몬스터를 사냥하는 헌터.", kiwi=kiwi)


def test_extract_nouns_on_empty_text_returns_empty(kiwi):
    assert extract_nouns("", kiwi=kiwi) == []
    assert extract_nouns("   ", kiwi=kiwi) == []


def test_extract_word_set_deduplicates(kiwi):
    words = extract_word_set("무림 무림 무림 강호", kiwi=kiwi)
    assert words == {"무림", "강호"}


def test_extract_nouns_without_kiwi_returns_empty(monkeypatch):
    monkeypatch.setattr("backend.category_lexicon.get_kiwi", lambda *a, **kw: None)
    assert extract_nouns("무림맹 장문인") == []


# ---------------------------------------------------------------------------
# 점수화
# ---------------------------------------------------------------------------


def test_score_words_picks_matching_category():
    lex = make_lexicon({"3_무협": ({"무림": 0.9, "내공": 0.8, "강호": 0.7}, []), "4_경제": ({"금리": 0.9, "환율": 0.8, "물가": 0.7}, [])})
    sims = lex.score_words({"무림", "내공"})
    assert sims["3_무협"] > 0
    assert "4_경제" not in sims


def test_unique_words_get_bonus_multiplier():
    plain = make_lexicon({"c": ({"가나다": 1.0}, [])})
    boosted = make_lexicon({"c": ({"가나다": 1.0}, ["가나다"])})
    assert boosted.score_words({"가나다"})["c"] == pytest.approx(plain.score_words({"가나다"})["c"] * UNIQUE_BONUS)


def test_l2_normalization_removes_vocabulary_scale_advantage():
    """가중치 총량이 큰 카테고리가 자동으로 이기면 안 된다"""
    lex = make_lexicon({"큰어휘": ({f"w{i}": 0.9 for i in range(200)}, []), "작은어휘": ({"작은단어": 0.9}, [])})
    # 각 카테고리의 단어를 정확히 1개씩 맞힌 문서
    sims = lex.score_words({"w0", "작은단어"})
    assert sims["작은어휘"] > sims["큰어휘"]


def test_rank_returns_normalized_scores_sorted():
    lex = make_lexicon({"3_무협": ({"무림": 0.9, "내공": 0.8}, []), "3_판타지": ({"던전": 0.9, "무림": 0.1}, [])})
    ranked = lex.score_words({"무림", "내공"})
    assert ranked["3_무협"] > ranked["3_판타지"]


def test_rank_abstains_when_similarity_is_below_threshold(kiwi):
    lex = make_lexicon({"3_무협": ({"무림": 1.0}, [])})
    assert lex.rank("금리 인상과 환율 변동이 물가에 미치는 영향", kiwi=kiwi) == []


def test_rank_top_result_is_normalized_to_one(kiwi):
    lex = make_lexicon({"3_무협": ({"무림": 0.9, "내공": 0.8, "강호": 0.7}, ["무림"]), "4_경제": ({"금리": 0.9}, [])})
    ranked = lex.rank("무림 강호의 내공", kiwi=kiwi)
    assert ranked
    assert ranked[0][0] == "3_무협"
    assert ranked[0][1] == 1.0
    assert ranked[0][2] >= MIN_SIMILARITY


def test_score_words_on_empty_input():
    lex = make_lexicon({"c": ({"가나다": 1.0}, [])})
    assert lex.score_words(set()) == {}


# ---------------------------------------------------------------------------
# 로드
# ---------------------------------------------------------------------------


def test_load_returns_none_for_missing_file(tmp_path):
    assert CategoryLexicon.load(tmp_path / "없는파일.json.gz") is None


def test_load_returns_none_for_corrupt_file(tmp_path):
    p = tmp_path / "broken.json.gz"
    p.write_bytes(b"not gzip at all")
    assert CategoryLexicon.load(p) is None


def test_load_returns_none_for_empty_lexicon(tmp_path):
    p = tmp_path / "empty.json.gz"
    with gzip.open(p, "wt", encoding="utf-8") as f:
        json.dump({"version": 1, "categories": {}}, f)
    assert CategoryLexicon.load(p) is None


def test_load_roundtrip(tmp_path):
    p = tmp_path / "lex.json.gz"
    payload = {"version": 1, "built_at": "2026-09-12T00:00:00+00:00", "params": {"top_n": 2}, "categories": {"3_무협": {"doc_count": 42, "norm": 1.2, "words": {"무림": 0.9, "내공": 0.8}, "unique": ["무림"]}}}
    with gzip.open(p, "wt", encoding="utf-8") as f:
        json.dump(payload, f, ensure_ascii=False)

    lex = CategoryLexicon.load(p)
    assert lex is not None
    assert lex.categories == ["3_무협"]
    assert lex.doc_counts["3_무협"] == 42
    assert lex.params["top_n"] == 2
    assert bool(lex) is True


def test_missing_norm_is_computed_from_weights():
    lex = CategoryLexicon({"version": 1, "categories": {"c": {"words": {"a": 3.0, "b": 4.0}}}})
    assert lex._norms["c"] == pytest.approx(5.0)
