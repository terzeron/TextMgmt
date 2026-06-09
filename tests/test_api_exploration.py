import requests
import sys
import types
from collections import Counter

try:
    # konlpy는 라이브 탐색 실행(run_api_exploration_test) 시에만 필요한 선택적 의존성.
    # 테스트는 Okt를 monkeypatch로 대체하므로 미설치 환경에서도 동작한다.
    from konlpy.tag import Okt
except ImportError:
    Okt = None


BASE_URL = "http://127.0.0.1:8000"


def get_top_frequent_words(text, top_n=100):
    if not text:
        return []

    if Okt is None:
        raise RuntimeError("konlpy가 설치되어 있지 않습니다. 라이브 탐색 실행에는 konlpy가 필요합니다.")

    okt = Okt()
    nouns = okt.nouns(text)
    meaningful_words = [word for word in nouns if len(word) > 1]
    if not meaningful_words:
        return []

    word_counts = Counter(meaningful_words)
    return word_counts.most_common(top_n)


def run_api_exploration_test():
    print("🚀 API 탐색 테스트를 시작합니다.")

    print("\n[1단계] 가장 책이 많은 카테고리 검색 중...")
    try:
        response = requests.get(f"{BASE_URL}/categories")
        response.raise_for_status()
        categories = response.json().get("result", [])
        if not categories:
            print("❌ 실패: 카테고리를 찾을 수 없습니다.")
            return
    except requests.RequestException as e:
        print(f"❌ 실패: 카테고리 목록 조회 중 오류 발생: {e}")
        return

    selected_category = None
    category_books = []
    max_book_count = 0

    for category in categories:
        try:
            cat_response = requests.get(f"{BASE_URL}/categories/{category}")
            cat_response.raise_for_status()
            books = cat_response.json().get("result", [])
            if len(books) > max_book_count:
                max_book_count = len(books)
                selected_category = category
                category_books = books
        except requests.RequestException:
            continue

    if selected_category:
        print(f"✅ 성공: 가장 책이 많은 '{selected_category}' 카테고리에서 책 {len(category_books)}권을 찾았습니다.")
    else:
        print("❌ 실패: 책이 있는 카테고리를 찾지 못했습니다.")
        return

    print("\n[2단계 & 3단계] 유사한 책이 있는 책 ID를 찾습니다...")
    similar_books = []
    selected_book_id = None

    for book in category_books:
        book_id = book.get("book_id")
        file_type = book.get("file_type", "")
        if not book_id or file_type in ("jpg", "jpeg", "png", "gif", "webp", "bmp", "tiff", "svg"):
            continue

        print(f"\n  - [2단계] 책 ID '{book_id}'(제목: {book.get('title')}, 타입: {file_type}) 선택.")
        print("  - [3단계] 해당 ID로 유사한 책 검색 시도...")
        try:
            similar_response = requests.get(f"{BASE_URL}/similar/{book_id}")
            similar_response.raise_for_status()

            result = similar_response.json().get("result", [])
            if result:
                selected_book_id = book_id
                similar_books = result
                print(f"✅ [3단계] 성공: 책 ID '{selected_book_id}'에서 유사한 책 {len(similar_books)}권을 찾았습니다.")
                print(f"  - 첫 번째 유사 책: {similar_books[0].get('title')}")
                break
            else:
                print("  - [3단계] 정보: 유사한 책이 없습니다. 다른 책으로 재시도합니다.")
        except requests.RequestException as e:
            print(f"  - [3단계] 오류: {e}. 다른 책으로 재시도합니다.")
            continue

    if not selected_book_id:
        print("❌ 실패: 유사한 책을 가진 책을 찾지 못했습니다.")
        return

    print("\n[4단계] 유사한 책 제목과 요약에서 키워드를 추출하여 검색 수행 중...")

    combined_text = " ".join(f"{book.get('title', '')} {book.get('summary', '')}".strip() for book in similar_books)
    print(f"  - 수집된 전체 텍스트의 길이: {len(combined_text)} 자")
    if len(combined_text) < 200:
        print(f'  - 수집된 텍스트 (일부): "{combined_text[:200]}..."')

    top_keywords = get_top_frequent_words(combined_text, 10)

    if not top_keywords:
        print("❌ 실패: 유사한 책들의 제목과 요약에서 유효한 키워드를 찾지 못했습니다.")
        return

    print("  - 유사 책 제목과 요약에서 찾은 명사 (빈도순):")
    for i, (word, count) in enumerate(top_keywords):
        print(f"    {i + 1:2d}. {word} ({count}회)")

    keyword = top_keywords[0][0]
    print(f"\n  - 검색에 사용할 키워드: '{keyword}'")

    try:
        search_response = requests.get(f"{BASE_URL}/search/{keyword}")
        search_response.raise_for_status()
        search_results = search_response.json().get("result", [])

        if search_results:
            print(f"✅ 성공: 키워드 '{keyword}'로 검색하여 {len(search_results)}개의 결과를 얻었습니다.")
            print(f"  - 첫 번째 결과: {search_results[0].get('title')}")
        else:
            print(f"❌ 실패: 키워드 '{keyword}'에 대한 검색 결과가 없습니다.")
            return

    except requests.RequestException as e:
        print(f"❌ 실패: 키워드 검색 중 오류 발생: {e}")
        return

    print("\n🎉 모든 테스트 시나리오를 성공적으로 완료했습니다.")


# ---- tests ----


def test_get_top_frequent_words_handles_empty():
    assert get_top_frequent_words("") == []


def test_get_top_frequent_words_filters_and_counts(monkeypatch):
    class FakeOkt:
        def nouns(self, text):
            return ["가", "나", "테스트", "테스트", "데이터", "가", "나"]

    mod = sys.modules[__name__]
    monkeypatch.setattr(mod, "Okt", lambda: FakeOkt())

    result = get_top_frequent_words("dummy", top_n=10)
    assert result[0][0] == "테스트"
    assert result[0][1] == 2
    assert ("데이터", 1) in result
    assert all(len(word) > 1 for word, _ in result)


def test_run_api_exploration_test_success(monkeypatch):
    class FakeOkt:
        def nouns(self, text):
            return ["테스트", "키워드", "키워드"]

    mod = sys.modules[__name__]
    monkeypatch.setattr(mod, "Okt", lambda: FakeOkt())

    class FakeResponse:
        def __init__(self, data):
            self._data = data

        def raise_for_status(self):
            return None

        def json(self):
            return self._data

    def fake_get(url, *args, **kwargs):
        if url.endswith("/categories"):
            return FakeResponse({"result": ["cat1"]})
        if "/categories/cat1" in url:
            return FakeResponse({"result": [{"book_id": 1, "title": "t1", "file_type": "epub"}]})
        if "/similar/1" in url:
            return FakeResponse({"result": [{"title": "t1", "summary": "s1"}]})
        if "/search/" in url:
            return FakeResponse({"result": [{"title": "t1"}]})
        return FakeResponse({"result": []})

    monkeypatch.setattr(mod, "requests", types.SimpleNamespace(get=fake_get, RequestException=requests.RequestException))

    run_api_exploration_test()


def test_run_api_exploration_test_no_categories(monkeypatch):
    class FakeResponse:
        def __init__(self, data):
            self._data = data

        def raise_for_status(self):
            return None

        def json(self):
            return self._data

    mod = sys.modules[__name__]
    monkeypatch.setattr(mod, "requests", types.SimpleNamespace(get=lambda *a, **k: FakeResponse({"result": []}), RequestException=requests.RequestException))

    run_api_exploration_test()


def test_get_top_frequent_words_returns_empty_when_no_meaningful_words(monkeypatch):
    class FakeOkt:
        def nouns(self, text):
            return ["가", "나", "다"]

    mod = sys.modules[__name__]
    monkeypatch.setattr(mod, "Okt", lambda: FakeOkt())
    assert get_top_frequent_words("dummy") == []


def test_run_api_exploration_test_request_error(monkeypatch):
    def raise_get(*args, **kwargs):
        raise requests.RequestException("boom")

    mod = sys.modules[__name__]
    monkeypatch.setattr(mod, "requests", types.SimpleNamespace(get=raise_get, RequestException=requests.RequestException))
    run_api_exploration_test()


def test_run_api_exploration_test_no_similar_books(monkeypatch):
    class FakeOkt:
        def nouns(self, text):
            return ["키워드", "키워드"]

    mod = sys.modules[__name__]
    monkeypatch.setattr(mod, "Okt", lambda: FakeOkt())

    class FakeResponse:
        def __init__(self, data):
            self._data = data

        def raise_for_status(self):
            return None

        def json(self):
            return self._data

    def fake_get(url, *args, **kwargs):
        if url.endswith("/categories"):
            return FakeResponse({"result": ["cat1"]})
        if "/categories/cat1" in url:
            return FakeResponse({"result": [{"book_id": 1, "title": "t1", "file_type": "epub"}]})
        if "/similar/1" in url:
            return FakeResponse({"result": []})
        return FakeResponse({"result": []})

    monkeypatch.setattr(mod, "requests", types.SimpleNamespace(get=fake_get, RequestException=requests.RequestException))
    run_api_exploration_test()


def test_run_api_exploration_test_no_keywords(monkeypatch):
    class FakeOkt:
        def nouns(self, text):
            return ["가", "나"]

    mod = sys.modules[__name__]
    monkeypatch.setattr(mod, "Okt", lambda: FakeOkt())

    class FakeResponse:
        def __init__(self, data):
            self._data = data

        def raise_for_status(self):
            return None

        def json(self):
            return self._data

    def fake_get(url, *args, **kwargs):
        if url.endswith("/categories"):
            return FakeResponse({"result": ["cat1"]})
        if "/categories/cat1" in url:
            return FakeResponse({"result": [{"book_id": 1, "title": "t1", "file_type": "epub"}]})
        if "/similar/1" in url:
            return FakeResponse({"result": [{"title": "t1", "summary": "s1"}]})
        return FakeResponse({"result": []})

    monkeypatch.setattr(mod, "requests", types.SimpleNamespace(get=fake_get, RequestException=requests.RequestException))
    run_api_exploration_test()


def test_run_api_exploration_test_search_empty(monkeypatch):
    class FakeOkt:
        def nouns(self, text):
            return ["키워드", "키워드"]

    mod = sys.modules[__name__]
    monkeypatch.setattr(mod, "Okt", lambda: FakeOkt())

    class FakeResponse:
        def __init__(self, data):
            self._data = data

        def raise_for_status(self):
            return None

        def json(self):
            return self._data

    def fake_get(url, *args, **kwargs):
        if url.endswith("/categories"):
            return FakeResponse({"result": ["cat1"]})
        if "/categories/cat1" in url:
            return FakeResponse({"result": [{"book_id": 1, "title": "t1", "file_type": "epub"}]})
        if "/similar/1" in url:
            return FakeResponse({"result": [{"title": "t1", "summary": "s1"}]})
        if "/search/" in url:
            return FakeResponse({"result": []})
        return FakeResponse({"result": []})

    monkeypatch.setattr(mod, "requests", types.SimpleNamespace(get=fake_get, RequestException=requests.RequestException))
    run_api_exploration_test()
