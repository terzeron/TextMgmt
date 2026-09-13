"""Complement Naive Bayes 분류기 테스트

코사인 유사도(TF-IDF 중심점) 방식에서 옮겨왔다. 그 방식은 맞은 단어 전체의 겹침
비율을 보기 때문에 결정적 단어 하나가 흔한 단어 수백 개에 묻혔다.
NB 는 단어마다 로그 확률비를 더하므로 결정적 단어 하나가 판정을 뒤집을 수 있다.
"""

import gzip
import json

import pytest

from backend.category_lexicon import MIN_MARGIN, MIN_MATCHED_WORDS, SERIES_TO_PARENT, CategoryLexicon, extract_nouns, extract_word_set, get_kiwi, is_meaningful_word, resolve_parent


@pytest.fixture(scope="module")
def kiwi():
    k = get_kiwi(num_workers=1)
    if k is None:
        pytest.skip("kiwipiepy 미설치")
    return k


def make_model(categories, extra_vocab=()):
    """{카테고리: {단어: 문서수}} 로 모델을 만든다"""
    total = {}
    for words in categories.values():
        for w, n in words.items():
            total[w] = total.get(w, 0) + n
    for w in extra_vocab:
        total.setdefault(w, 1)
    return CategoryLexicon({"version": 2, "params": {"alpha": 1.0, "vocab_size": len(total)}, "total_df": total, "categories": {cat: {"doc_count": 100, "df": words} for cat, words in categories.items()}})


# 어느 카테고리에도 치우치지 않는 채움말. 점수를 움직이지 않아야 한다.
FILLER = {f"흔한말{i}": 50 for i in range(30)}

# 문서에 나오지 않는 어휘. 이걸 넣지 않으면 문서가 어휘 전체를 덮어버려
# 모든 카테고리 점수가 1.0 으로 같아진다(가중치 합이 1 로 정규화되므로).
# 실제 사전은 어휘 30,000 개에 문서가 1,000 개쯤 맞아 생기지 않는 상황이다.
UNUSED_VOCAB = [f"문서에없는말{i}" for i in range(200)]


def with_filler(words):
    return {**FILLER, **words}


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
    assert "는" not in words


def test_extract_word_set_deduplicates(kiwi):
    assert extract_word_set("무림 무림 무림 강호", kiwi=kiwi) == {"무림", "강호"}


def test_extract_nouns_without_kiwi_returns_empty(monkeypatch):
    monkeypatch.setattr("backend.category_lexicon.get_kiwi", lambda *a, **kw: None)
    assert extract_nouns("무림맹 장문인") == []


# ---------------------------------------------------------------------------
# 하위 카테고리 -> 상위 장르
# ---------------------------------------------------------------------------


def test_resolve_parent_maps_series_to_genre():
    assert resolve_parent("2_을유세계문학전집") == "2_소설외국"
    assert resolve_parent("3_셜록홈즈") == "3_스릴러"


def test_resolve_parent_passes_through_plain_categories():
    assert resolve_parent("3_무협") == "3_무협"
    assert resolve_parent(None) is None


def test_every_series_parent_is_itself_a_plain_category():
    """상위 장르가 다시 하위 카테고리면 사슬이 생겨 판정이 꼬인다"""
    for parent in SERIES_TO_PARENT.values():
        assert parent not in SERIES_TO_PARENT


# ---------------------------------------------------------------------------
# 점수화
# ---------------------------------------------------------------------------


def test_score_words_picks_matching_category():
    lex = make_model({"3_무협": with_filler({"무림": 90, "내공": 80}), "4_경제": with_filler({"금리": 90, "환율": 80})}, extra_vocab=UNUSED_VOCAB)
    scores = lex.score_words(set(FILLER) | {"무림", "내공"})
    assert scores["3_무협"] > scores["4_경제"]


def test_one_decisive_word_outweighs_many_shared_ones():
    """
    코사인 방식에서는 결정적 단어 하나가 흔한 단어 수백 개에 묻혔다.
    `40대 중년남의 하렘라이프` 가 학교 배경이라는 이유로 9_어린이육아 로 간 사례가 그것이다.

    여기서 `교실`·`엄마`·`학부모` 는 양쪽에 비슷하게 나오므로 변별력이 없고,
    `오우거` 는 판타지에만 나오므로 결정적이다. 결정적 단어 하나가 이겨야 한다.
    """
    shared = {"교실": 60, "엄마": 60, "학부모": 60}
    lex = make_model({"3_판타지": with_filler({**shared, "오우거": 80}), "9_어린이육아": with_filler({**shared})}, extra_vocab=UNUSED_VOCAB)
    scores = lex.score_words(set(FILLER) | set(shared) | {"오우거"})
    assert scores["3_판타지"] > scores["9_어린이육아"]


def test_shared_words_do_not_flip_the_verdict():
    """
    양쪽에 비슷하게 나오는 단어를 아무리 더해도 판정이 뒤집히면 안 된다.
    (가중치가 카테고리마다 따로 정규화되므로 기여가 완전히 같지는 않다.
     같아야 한다고 단정하지 말고, 판정이 유지되는지를 본다.)
    """
    shared = {f"양쪽말{i}": 60 for i in range(20)}
    lex = make_model({"3_판타지": with_filler({**shared, "오우거": 80}), "9_어린이육아": with_filler({**shared})}, extra_vocab=UNUSED_VOCAB)

    without = lex.score_words(set(FILLER) | {"오우거"})
    with_shared = lex.score_words(set(FILLER) | set(shared) | {"오우거"})
    assert without["3_판타지"] > without["9_어린이육아"]
    assert with_shared["3_판타지"] > with_shared["9_어린이육아"]


def test_score_words_needs_enough_matched_vocabulary():
    lex = make_model({"3_무협": with_filler({"무림": 90})})
    assert lex.score_words({"무림"}) == {}
    assert lex.score_words(set()) == {}


def test_unknown_words_are_ignored():
    lex = make_model({"3_무협": with_filler({"무림": 90}), "4_경제": with_filler({"금리": 90})}, extra_vocab=UNUSED_VOCAB)
    base = lex.score_words(set(FILLER) | {"무림"})
    with_noise = lex.score_words(set(FILLER) | {"무림", "사전에없는말"})
    assert base == with_noise


def test_scores_are_deterministic():
    lex = make_model({"3_무협": with_filler({"무림": 90}), "4_경제": with_filler({"금리": 90})}, extra_vocab=UNUSED_VOCAB)
    words = set(FILLER) | {"무림"}
    assert lex.score_words(words) == lex.score_words(words)


# ---------------------------------------------------------------------------
# 판정 기준
# ---------------------------------------------------------------------------


def test_rank_abstains_when_top_two_are_nearly_tied(kiwi, monkeypatch):
    lex = make_model({"3_판타지": with_filler({"공통어": 70}), "3_여성향": with_filler({"공통어": 69})}, extra_vocab=UNUSED_VOCAB)
    monkeypatch.setattr("backend.category_lexicon.extract_word_set", lambda *a, **kw: set(FILLER) | {"공통어"})
    assert lex.rank("아무 글이나", kiwi=kiwi) == []


def test_rank_decides_when_the_winner_is_clear(kiwi, monkeypatch):
    lex = make_model({"3_무협": with_filler({"무림": 95, "내공": 95, "강호": 95}), "4_경제": with_filler({"금리": 95})}, extra_vocab=UNUSED_VOCAB)
    monkeypatch.setattr("backend.category_lexicon.extract_word_set", lambda *a, **kw: set(FILLER) | {"무림", "내공", "강호"})
    ranked = lex.rank("아무 글이나", kiwi=kiwi, min_margin=1.0)
    assert ranked
    assert ranked[0][0] == "3_무협"
    assert ranked[0][1] == 1.0


def test_min_margin_is_a_ratio_just_above_one():
    """
    CNB 점수는 로그 확률비를 어휘 전체에 걸쳐 더한 값이라 1·2위 차이가 작은 비율로
    나타난다. 코사인 시절의 2.5배 같은 값을 그대로 쓰면 전부 걸러진다.
    """
    assert 1.0 < MIN_MARGIN < 1.05


def test_min_matched_words_is_positive():
    assert MIN_MATCHED_WORDS > 0


# ---------------------------------------------------------------------------
# 로드
# ---------------------------------------------------------------------------


def test_load_returns_none_for_missing_file(tmp_path):
    assert CategoryLexicon.load(tmp_path / "없는파일.json.gz") is None


def test_load_returns_none_for_corrupt_file(tmp_path):
    p = tmp_path / "broken.json.gz"
    p.write_bytes(b"not gzip at all")
    assert CategoryLexicon.load(p) is None


def test_load_returns_none_for_empty_model(tmp_path):
    p = tmp_path / "empty.json.gz"
    with gzip.open(p, "wt", encoding="utf-8") as f:
        json.dump({"version": 2, "total_df": {}, "categories": {}}, f)
    assert CategoryLexicon.load(p) is None


def test_load_roundtrip(tmp_path):
    p = tmp_path / "model.json.gz"
    payload = {"version": 2, "built_at": "2026-09-13T00:00:00+00:00", "params": {"alpha": 1.0, "vocab_size": 2}, "total_df": {"무림": 90, "금리": 90}, "categories": {"3_무협": {"doc_count": 42, "df": {"무림": 90}}, "4_경제": {"doc_count": 17, "df": {"금리": 90}}}}
    with gzip.open(p, "wt", encoding="utf-8") as f:
        json.dump(payload, f, ensure_ascii=False)

    lex = CategoryLexicon.load(p)
    assert lex is not None
    assert lex.categories == ["3_무협", "4_경제"]
    assert lex.doc_counts["3_무협"] == 42
    assert bool(lex) is True
