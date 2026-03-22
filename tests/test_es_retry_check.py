import pytest
from unittest.mock import patch
from elasticsearch import Elasticsearch
from elastic_transport import ConnectionError as ESConnectionError


def _set_es_env(monkeypatch):
    monkeypatch.setenv("TM_ES_BOOK_INDEX", "test_index")
    monkeypatch.setenv("TM_ES_URL", "http://localhost:9200")
    monkeypatch.setenv("TM_ES_USER", "elastic")
    monkeypatch.setenv("TM_ES_PASSWORD", "changeme")


@patch.object(Elasticsearch, "info")
@patch.object(Elasticsearch, "__init__", return_value=None)
def test_init_success_first_try(mock_es_init, mock_info, monkeypatch):
    _set_es_env(monkeypatch)
    mock_info.return_value = {"cluster_name": "test"}

    from backend.es_manager import ESManager

    ESManager()

    mock_info.assert_called_once()


@patch("backend.es_manager.time.sleep")
@patch.object(Elasticsearch, "info")
@patch.object(Elasticsearch, "__init__", return_value=None)
def test_init_retry_then_success(mock_es_init, mock_info, mock_sleep, monkeypatch):
    _set_es_env(monkeypatch)
    mock_info.side_effect = [
        ESConnectionError("connection refused"),
        ESConnectionError("connection refused"),
        {"cluster_name": "test"},
    ]

    from backend.es_manager import ESManager

    ESManager()

    assert mock_info.call_count == 3
    assert mock_sleep.call_count == 2
    mock_sleep.assert_any_call(2)
    mock_sleep.assert_any_call(4)


@patch("backend.es_manager.time.sleep")
@patch.object(Elasticsearch, "info")
@patch.object(Elasticsearch, "__init__", return_value=None)
def test_init_all_retries_fail(mock_es_init, mock_info, mock_sleep, monkeypatch):
    _set_es_env(monkeypatch)
    mock_info.side_effect = ESConnectionError("connection refused")

    from backend.es_manager import ESManager

    with pytest.raises(ESConnectionError):
        ESManager()

    assert mock_info.call_count == 5
    assert mock_sleep.call_count == 4
