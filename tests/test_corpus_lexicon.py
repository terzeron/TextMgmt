import zipfile

import pytest

from backend.category_lexicon import CategoryLexicon, get_kiwi
from utils.corpus_lexicon import build_lexicon, collect_all, explain_file, is_mojibake, collect_category, list_category_dirs, list_category_files, load_collected, read_document_text, read_epub_text, read_txt_text, sample_files, split_holdout, write_lexicon

WUXIA_SENTENCES = ["소림사 장로가 강호에서 내공을 운기조식하며 무림맹 검법을 수련했다.", "화산파 장문인은 단전에 진기를 모아 주화입마를 피했다.", "마교 천마가 사파 무인들을 이끌고 정파 문파를 습격했다."]
FANTASY_SENTENCES = ["던전에서 몬스터를 사냥하는 헌터가 각성하여 레이드에 참가했다.", "마법사가 마나를 모아 마법진을 그리고 드래곤을 소환했다.", "상태창을 열어 스킬과 스탯을 확인한 플레이어가 길드에 가입했다."]
ECON_SENTENCES = ["거시경제학의 총수요곡선과 통화정책 전달경로를 분석한 논문이다.", "금리 인상이 물가와 실업률에 미치는 영향을 계량경제학으로 추정했다.", "환율 변동과 무역수지의 상관관계를 시계열 분석으로 검증한다."]
# 모든 카테고리에 공통으로 섞는 범용 문장. idf 로 자동 소거되어야 한다.
FILLER = " 그는 그것을 보았다. 사람들이 생각했다. 그때 일이 있었다."


@pytest.fixture(scope="module")
def kiwi():
    k = get_kiwi(num_workers=1)
    if k is None:
        pytest.skip("kiwipiepy 미설치")
    return k


@pytest.fixture(scope="module")
def synth_corpus(tmp_path_factory):
    root = tmp_path_factory.mktemp("corpus")
    for cat, sentences in [("3_무협", WUXIA_SENTENCES), ("3_판타지", FANTASY_SENTENCES), ("4_경제", ECON_SENTENCES)]:
        d = root / cat
        d.mkdir()
        body = (" ".join(sentences) + FILLER) * 20
        for i in range(25):
            (d / f"{cat}_{i:03d}.txt").write_text(body, encoding="utf-8")
    # 카테고리가 아닌 스테이징 디렉토리
    staging = root / "0_telegram"
    staging.mkdir()
    (staging / "미분류.txt").write_text("아무 내용", encoding="utf-8")
    (root / "trash").mkdir()
    return root


@pytest.fixture(scope="module")
def built(synth_corpus, tmp_path_factory, kiwi):
    work = tmp_path_factory.mktemp("work")
    collect_all(synth_corpus, work, min_files=5, max_files=25, holdout_ratio=0.2, num_workers=1)
    collected = load_collected(work)
    lexicon = build_lexicon(collected, top_n=50)
    path = write_lexicon(lexicon, work / "lex.json.gz")
    return collected, lexicon, CategoryLexicon.load(path)


# ---------------------------------------------------------------------------
# 코퍼스 탐색
# ---------------------------------------------------------------------------


def test_list_category_dirs_excludes_zero_prefix_and_unnumbered(synth_corpus):
    assert list_category_dirs(synth_corpus) == ["3_무협", "3_판타지", "4_경제"]


def test_list_category_dirs_on_missing_root(tmp_path):
    assert list_category_dirs(tmp_path / "없음") == []


def test_list_category_files_finds_txt_and_epub(tmp_path):
    (tmp_path / "a.txt").write_text("x", encoding="utf-8")
    (tmp_path / "b.epub").write_bytes(b"x")
    (tmp_path / "c.pdf").write_bytes(b"x")
    (tmp_path / ".hidden.txt").write_text("x", encoding="utf-8")
    names = [p.name for p in list_category_files(tmp_path)]
    assert names == ["a.txt", "b.epub"]


# ---------------------------------------------------------------------------
# 본문 읽기
# ---------------------------------------------------------------------------


def test_read_txt_text_handles_cp949(tmp_path):
    p = tmp_path / "cp949.txt"
    p.write_bytes("소림사 장로가 강호에서 내공을 수련했다.".encode("cp949"))
    assert "소림사" in read_txt_text(p, 1000, 0)


def test_read_txt_text_reads_head_and_tail(tmp_path):
    p = tmp_path / "long.txt"
    p.write_text("머리말 " + ("가" * 50000) + " 꼬리말", encoding="utf-8")
    text = read_txt_text(p, 200, 200)
    assert "머리말" in text
    assert "꼬리말" in text


def test_read_txt_text_on_missing_file(tmp_path):
    assert read_txt_text(tmp_path / "없음.txt", 100, 100) == ""


def test_read_epub_text_strips_tags_and_skips_cover(tmp_path):
    p = tmp_path / "book.epub"
    with zipfile.ZipFile(p, "w") as z:
        z.writestr("OPS/cover.xhtml", "<html><body><p>" + ("표지" * 50) + "</p></body></html>")
        z.writestr("OPS/ch1.xhtml", "<html><body><p>" + ("소림사 장로가 강호에서 " * 20) + "</p></body></html>")
    text = read_epub_text(p, 5000, 0)
    assert "소림사" in text
    assert "<p>" not in text
    assert "표지" not in text


def test_read_epub_text_on_corrupt_zip(tmp_path):
    p = tmp_path / "broken.epub"
    p.write_bytes(b"PK\x03\x04 garbage")
    assert read_epub_text(p, 1000, 0) == ""


def test_read_document_text_ignores_unknown_extension(tmp_path):
    p = tmp_path / "x.pdf"
    p.write_bytes(b"%PDF")
    assert read_document_text(p) == ""


# ---------------------------------------------------------------------------
# 샘플링
# ---------------------------------------------------------------------------


def test_sample_files_is_deterministic_and_capped():
    files = [f"f{i}" for i in range(100)]
    a = sample_files(files, 10, seed=1, salt="3_무협")
    b = sample_files(files, 10, seed=1, salt="3_무협")
    c = sample_files(files, 10, seed=1, salt="3_판타지")
    assert a == b
    assert len(a) == 10
    assert a != c


def test_sample_files_returns_all_when_under_cap():
    files = ["a", "b", "c"]
    assert sample_files(files, 10, seed=1, salt="x") == files


def test_split_holdout_is_disjoint_and_covers_all():
    files = [f"f{i}" for i in range(50)]
    train, hold = split_holdout(files, 0.2, seed=1, salt="c")
    assert set(train) & set(hold) == set()
    assert sorted(train + hold) == sorted(files)
    assert len(hold) == 10


def test_split_holdout_disabled_for_tiny_sets():
    files = ["a", "b", "c"]
    train, hold = split_holdout(files, 0.2, seed=1, salt="c")
    assert train == files
    assert hold == []


# ---------------------------------------------------------------------------
# 수집
# ---------------------------------------------------------------------------


def test_collect_category_skips_when_below_min_files(tmp_path, kiwi):
    d = tmp_path / "3_무협"
    d.mkdir()
    (d / "only.txt").write_text("소림사 장로", encoding="utf-8")
    assert collect_category("3_무협", d, tmp_path / "work", min_files=20, kiwi=kiwi) is None


def test_collect_category_writes_cache_and_holdout(synth_corpus, tmp_path, kiwi):
    res = collect_category("3_무협", synth_corpus / "3_무협", tmp_path / "work", min_files=5, max_files=25, holdout_ratio=0.2, kiwi=kiwi)
    assert res is not None
    assert res["doc_count"] == 20
    assert len(res["holdout"]) == 5
    assert "무림맹" in res["df"]
    assert (tmp_path / "work" / "3_무협.json.gz").exists()


def test_collect_all_resumes_from_cache(synth_corpus, tmp_path, kiwi, caplog):
    work = tmp_path / "work"
    collect_all(synth_corpus, work, min_files=5, max_files=25, num_workers=1)
    with caplog.at_level("INFO"):
        done = collect_all(synth_corpus, work, min_files=5, max_files=25, num_workers=1)
    assert sorted(done) == ["3_무협", "3_판타지", "4_경제"]
    assert "캐시 존재" in caplog.text


def test_load_collected_ignores_corrupt_cache(tmp_path):
    (tmp_path / "bad.json.gz").write_bytes(b"not gzip")
    assert load_collected(tmp_path) == {}


# ---------------------------------------------------------------------------
# 사전 빌드
# ---------------------------------------------------------------------------


def test_build_lexicon_rejects_empty_input():
    with pytest.raises(ValueError):
        build_lexicon({})


def test_build_lexicon_drops_words_common_to_every_category(built):
    _collected, lexicon, _lex = built
    # '사람', '생각' 은 세 카테고리 전부에 등장하므로 log(C/C)=0 으로 소거된다
    for cat, entry in lexicon["categories"].items():
        assert "사람" not in entry["words"], cat
        assert "생각" not in entry["words"], cat


def test_build_lexicon_keeps_domain_terms(built):
    _collected, lexicon, _lex = built
    assert "무림맹" in lexicon["categories"]["3_무협"]["words"]
    assert "몬스터" in lexicon["categories"]["3_판타지"]["words"]
    assert "거시경제학" in lexicon["categories"]["4_경제"]["words"]


def test_unique_sets_are_category_exclusive(built):
    _collected, lexicon, _lex = built
    uniques = {cat: set(entry["unique"]) for cat, entry in lexicon["categories"].items()}
    assert "무림맹" in uniques["3_무협"]
    assert "던전" in uniques["3_판타지"]
    for cat_a, words_a in uniques.items():
        for cat_b, words_b in uniques.items():
            if cat_a != cat_b:
                assert words_a & words_b == set(), f"{cat_a} 와 {cat_b} 의 고유 단어가 겹친다"


def test_build_lexicon_records_params_and_norm(built):
    _collected, lexicon, _lex = built
    assert lexicon["params"]["top_n"] == 50
    assert lexicon["params"]["category_count"] == 3
    for entry in lexicon["categories"].values():
        assert entry["norm"] > 0
        assert entry["doc_count"] == 20


@pytest.mark.parametrize("probe,expected", [("마교 천마가 무림맹 장문인과 강호에서 검법으로 싸웠다", "3_무협"), ("헌터가 던전에서 몬스터를 잡고 레이드 길드에 가입했다", "3_판타지"), ("통화정책과 환율이 무역수지와 물가에 미치는 영향", "4_경제")])
def test_end_to_end_lexicon_classifies_probe_text(built, kiwi, probe, expected):
    _collected, _lexicon, lex = built
    ranked = lex.rank(probe, top_k=3, kiwi=kiwi)
    assert ranked, f"{expected}: 판정 없음"
    assert ranked[0][0] == expected


def test_end_to_end_lexicon_abstains_on_unrelated_text(built, kiwi):
    _collected, _lexicon, lex = built
    assert lex.rank("The quick brown fox jumps over the lazy dog.", kiwi=kiwi) == []


# ---------------------------------------------------------------------------
# 파일 한 건 판정 설명
# ---------------------------------------------------------------------------

def test_explain_file_reports_lexicon_legacy_and_weighted(built, synth_corpus, kiwi):
    _collected, _lexicon, lex = built
    target = next((synth_corpus / "3_무협").glob("*.txt"))
    out = explain_file(target, lex, kiwi=kiwi)

    assert out["file"] == str(target)
    assert out["word_count"] > 0
    assert out["lexicon"][0]["category"] == "3_무협"
    assert out["lexicon"][0]["normalized"] == 1.0
    assert "무림맹" in out["lexicon"][0]["matched_unique"]
    # 기존 5대 장르 스코어링과 가중치 합 판정도 함께 보고한다
    assert out["legacy"]["category"] == "3_무협"
    assert out["weighted"]["category"] == "3_무협"


def test_explain_file_reports_error_for_unreadable_file(built, tmp_path, kiwi):
    _collected, _lexicon, lex = built
    p = tmp_path / "tiny.txt"
    p.write_text("짧다", encoding="utf-8")
    assert "error" in explain_file(p, lex, kiwi=kiwi)


# ---------------------------------------------------------------------------
# 꼬리 카테고리 (문서 수가 적은 카테고리)
# ---------------------------------------------------------------------------

def test_tiny_category_still_gets_a_lexicon(tmp_path, kiwi):
    """문서가 몇 건뿐이어도 카테고리는 사전을 갖는다"""
    root = tmp_path / "corpus"
    for cat, sentences, n in [("9_격언명언", ECON_SENTENCES, 7), ("3_무협", WUXIA_SENTENCES, 25)]:
        d = root / cat
        d.mkdir(parents=True)
        for i in range(n):
            (d / f"{i}.txt").write_text((" ".join(sentences) + FILLER) * 20, encoding="utf-8")

    work = tmp_path / "work"
    collect_all(root, work, holdout_ratio=0.0, num_workers=1)
    lexicon = build_lexicon(load_collected(work), top_n=50)

    assert set(lexicon["categories"]) == {"9_격언명언", "3_무협"}
    assert lexicon["categories"]["9_격언명언"]["doc_count"] == 7


def test_zero_file_category_is_skipped(tmp_path, kiwi):
    """파일이 0건이면 사전을 만들 근거가 없으므로 건너뛴다"""
    d = tmp_path / "1_올재"
    d.mkdir()
    assert collect_category("1_올재", d, tmp_path / "work", kiwi=kiwi) is None


def test_unique_words_require_a_minimum_document_count(kiwi):
    """1개 문서에만 나온 단어는 출현률이 높아도 고유 단어로 인정하지 않는다"""
    collected = {
        "9_격언명언": {"category": "9_격언명언", "doc_count": 4, "df": {"흔한말": 4, "한번만나온말": 1}},
        "3_무협": {"category": "3_무협", "doc_count": 100, "df": {"무림": 90}},
    }
    lexicon = build_lexicon(collected, top_n=50, unique_min_docs=3)
    unique = set(lexicon["categories"]["9_격언명언"]["unique"])
    assert "흔한말" in unique
    assert "한번만나온말" not in unique


def test_unique_min_docs_guard_can_be_relaxed():
    collected = {
        "9_격언명언": {"category": "9_격언명언", "doc_count": 4, "df": {"한번만나온말": 1}},
        "3_무협": {"category": "3_무협", "doc_count": 100, "df": {"무림": 90}},
    }
    lexicon = build_lexicon(collected, top_n=50, unique_min_docs=1)
    assert "한번만나온말" in set(lexicon["categories"]["9_격언명언"]["unique"])


# ---------------------------------------------------------------------------
# 인코딩 손상 탐지
# ---------------------------------------------------------------------------

# 중국어 텍스트를 잘못된 코드페이지로 읽어 저장한 실제 코퍼스 파일에서 발췌
MOJIBAKE_TEXT = "쒎똿耶먨ㄷ뱿竊뚧닊룵쐣訝鰲믧뙝╈ 쒍鰲믧뙝弱긷룵꺗縕룝뗥똿耶먲펯앲끽똿耶먪쉪鴉멩뎸떯걥弱뤸툍뵥쑉뎸恙껆쉪訝鰲믧뙝竊뚧룢雅녵 " * 6
CLEAN_TEXT = "소림사 장로가 강호에서 내공을 운기조식하며 화산파 검법을 수련했다. 그는 무림맹의 장문인으로서 제자들을 이끌었다. " * 6
# 국한문 혼용 정상 문헌. 한자가 한글 조사에 그대로 붙지만 손상이 아니다.
HANJA_MIXED_TEXT = "孟子의 德을 논하자면 仁義禮智가 그 바탕이 된다고 하였다. 朱子는 集註에서 이를 자세히 풀이하였다. " * 8


def test_is_mojibake_flags_corrupted_text():
    assert is_mojibake(MOJIBAKE_TEXT) is True


def test_is_mojibake_accepts_clean_korean():
    assert is_mojibake(CLEAN_TEXT) is False


def test_is_mojibake_accepts_hanja_mixed_korean():
    """국한문 혼용은 한자가 한글에 붙어도 손상이 아니다"""
    assert is_mojibake(HANJA_MIXED_TEXT) is False


def test_is_mojibake_ignores_non_korean_documents():
    """한글이 거의 없는 문서(한문 원전, 중국어, 일본어, 영문)는 판정 대상이 아니다"""
    assert is_mojibake("The quick brown fox jumps over the lazy dog. " * 10) is False
    assert is_mojibake("孟子曰 仁義禮智 信也 天下之達道也 君子之道 費而隱 " * 20) is False


def test_is_mojibake_skips_short_text():
    assert is_mojibake("짧은 글") is False


def test_collection_excludes_corrupted_documents(tmp_path, kiwi):
    d = tmp_path / "9_격언명언"
    d.mkdir()
    for i in range(6):
        (d / f"clean_{i}.txt").write_text(CLEAN_TEXT * 3, encoding="utf-8")
    for i in range(2):
        (d / f"broken_{i}.txt").write_text(MOJIBAKE_TEXT * 3, encoding="utf-8")

    res = collect_category("9_격언명언", d, tmp_path / "work", holdout_ratio=0.0, kiwi=kiwi)
    assert res is not None
    assert res["doc_count"] == 6
    assert res["corrupted_docs"] == 2
    # 깨진 토큰이 사전 원재료에 들어가지 않는다
    assert "무림맹" in res["df"]
    assert not any("쒎똿耶" in w for w in res["df"])
