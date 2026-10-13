#!/usr/bin/env python
"""분류기/서점 판정/특징 공간의 드문 분기를 확인한다."""

import zipfile
from pathlib import Path
from unittest.mock import MagicMock

import pytest

from backend.book_classifier import BookClassifierService, clean_empty_parent_dirs, inspect_epub_metadata, is_title_match_reliable
from backend.classifier import BookCategoryClassifier
from backend.classifier.features import FeatureSpace


def _classifier(tmp_path: Path, model) -> BookCategoryClassifier:
    clf = BookCategoryClassifier(model=MagicMock(), library_root=tmp_path)
    clf.model = model
    return clf


def test_expected_accuracy_is_none_without_model(tmp_path: Path):
    assert _classifier(tmp_path, None).expected_accuracy(0.9) is None


def test_expected_accuracy_delegates_to_model(tmp_path: Path):
    model = MagicMock()
    model.expected_accuracy.return_value = 0.87
    assert _classifier(tmp_path, model).expected_accuracy(0.9) == 0.87
    model.expected_accuracy.assert_called_once_with(0.9)


def test_feature_space_names_follow_field_order():
    space = FeatureSpace({"title": {}, "body": {}})
    assert space.names == ["title", "body"]


def test_inspect_epub_metadata_returns_empty_meta_for_non_xml_opf(tmp_path: Path):
    epub = tmp_path / "bad.epub"
    with zipfile.ZipFile(epub, "w") as zf:
        zf.writestr("content.opf", "xml 이 아니다")
    assert inspect_epub_metadata(epub) == {"title": "", "author": "", "subject": "", "description": ""}


def test_title_match_accepts_reordered_words_with_high_bigram_similarity():
    assert is_title_match_reliable("마법사의 돌 해리 포터", "해리 포터 마법사의 돌") is True


def test_title_match_rejects_unrelated_titles():
    assert is_title_match_reliable("은하수를 여행하는 히치하이커", "조선시대 농업 경제사 연구") is False


def test_clean_empty_parent_dirs_ignores_unresolvable_path(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    child = tmp_path / "a"
    child.mkdir()

    def broken_resolve(self, *args, **kwargs):
        raise OSError("경로 해석 실패")

    monkeypatch.setattr(Path, "resolve", broken_resolve)

    clean_empty_parent_dirs(child, tmp_path)

    assert child.exists()


def test_bookstore_vote_skips_reliable_store_without_candidates(tmp_path: Path):
    service = BookClassifierService(library_root=tmp_path, cache_file=tmp_path / "cache.json", classifier=None)
    entry = {"search_title": "해리 포터와 마법사의 돌", "yes24": {"title": "해리 포터와 마법사의 돌"}, "aladin": {}, "kyobo": {}}

    result = service._decide_by_bookstore(entry)

    assert result[1] == "not_found"
    assert entry["bookstore_candidates"] == []
