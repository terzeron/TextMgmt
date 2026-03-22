import types


def test_get_top_frequent_words_handles_empty():
    from backend import test_api

    assert test_api.get_top_frequent_words("") == []


def test_get_top_frequent_words_filters_and_counts(monkeypatch):
    from backend import test_api

    class FakeOkt:
        def nouns(self, text):
            return ["가", "나", "테스트", "테스트", "데이터", "가", "나"]

    monkeypatch.setattr(test_api, "Okt", lambda: FakeOkt())

    result = test_api.get_top_frequent_words("dummy", top_n=10)
    assert result[0][0] == "테스트"
    assert result[0][1] == 2
    assert ("데이터", 1) in result
    assert all(len(word) > 1 for word, _ in result)


def test_run_api_exploration_test_success(monkeypatch):
    from backend import test_api

    class FakeOkt:
        def nouns(self, text):
            return ["테스트", "키워드", "키워드"]

    monkeypatch.setattr(test_api, "Okt", lambda: FakeOkt())

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

    monkeypatch.setattr(test_api.requests, "get", fake_get)

    test_api.run_api_exploration_test()


def test_run_api_exploration_test_no_categories(monkeypatch):
    from backend import test_api

    class FakeResponse:
        def __init__(self, data):
            self._data = data

        def raise_for_status(self):
            return None

        def json(self):
            return self._data

    monkeypatch.setattr(test_api.requests, "get", lambda *args, **kwargs: FakeResponse({"result": []}))

    test_api.run_api_exploration_test()


def test_get_top_frequent_words_returns_empty_when_no_meaningful_words(monkeypatch):
    from backend import test_api

    class FakeOkt:
        def nouns(self, text):
            return ["가", "나", "다"]

    monkeypatch.setattr(test_api, "Okt", lambda: FakeOkt())
    assert test_api.get_top_frequent_words("dummy") == []


def test_run_api_exploration_test_request_error(monkeypatch):
    from backend import test_api

    def raise_get(*args, **kwargs):
        raise test_api.requests.RequestException("boom")

    monkeypatch.setattr(test_api.requests, "get", raise_get)
    test_api.run_api_exploration_test()


def test_run_api_exploration_test_no_similar_books(monkeypatch):
    from backend import test_api

    class FakeOkt:
        def nouns(self, text):
            return ["키워드", "키워드"]

    monkeypatch.setattr(test_api, "Okt", lambda: FakeOkt())

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

    monkeypatch.setattr(test_api.requests, "get", fake_get)
    test_api.run_api_exploration_test()


def test_run_api_exploration_test_no_keywords(monkeypatch):
    from backend import test_api

    class FakeOkt:
        def nouns(self, text):
            return ["가", "나"]

    monkeypatch.setattr(test_api, "Okt", lambda: FakeOkt())

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

    monkeypatch.setattr(test_api.requests, "get", fake_get)
    test_api.run_api_exploration_test()


def test_run_api_exploration_test_search_empty(monkeypatch):
    from backend import test_api

    class FakeOkt:
        def nouns(self, text):
            return ["키워드", "키워드"]

    monkeypatch.setattr(test_api, "Okt", lambda: FakeOkt())

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

    monkeypatch.setattr(test_api.requests, "get", fake_get)
    test_api.run_api_exploration_test()
