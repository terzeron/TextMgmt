#!/usr/bin/env python
"""auth.observation_hash 의 빈 입력과 Kyobo 링크 상한 분기를 확인한다."""

import os

from bs4 import BeautifulSoup

os.environ.setdefault("TM_JWT_SECRET", "test_jwt_secret_for_testing_minimum_32bytes")

import backend.auth as auth  # noqa: E402
from backend.bookstore import KyoboBookstore  # noqa: E402


def test_observation_hash_of_empty_value_is_empty_string():
    assert auth.observation_hash("") == ""


def test_kyobo_fallback_links_stop_at_max_results():
    store = KyoboBookstore(verbose=False)
    anchors = "".join(f'<a href="/detail/{n}">책 {n}</a>' for n in range(store.MAX_RESULTS + 5))
    soup = BeautifulSoup(f"<html><body>{anchors}</body></html>", "html.parser")

    links = store.extract_search_links(soup)

    assert links == [f"/detail/{n}" for n in range(store.MAX_RESULTS)]
