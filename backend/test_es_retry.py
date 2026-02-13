#!/usr/bin/env python

from unittest.mock import patch
import pytest
from elasticsearch import Elasticsearch
from elastic_transport import ConnectionError as ESConnectionError


@pytest.fixture
def env_vars(monkeypatch):
    monkeypatch.setenv("TM_ES_INDEX", "test_index")
    monkeypatch.setenv("TM_ES_URL", "http://localhost:9200")
    monkeypatch.setenv("TM_ES_USER", "elastic")
    monkeypatch.setenv("TM_ES_PASSWORD", "changeme")


@patch.object(Elasticsearch, "info")
@patch.object(Elasticsearch, "__init__", return_value=None)
def test_init_success_first_try(mock_es_init, mock_info, env_vars):
    """ES 연결이 첫 시도에 성공하는 경우"""
    mock_info.return_value = {"cluster_name": "test"}

    from es_manager import ESManager
    ESManager()

    mock_info.assert_called_once()


@patch("es_manager.time.sleep")
@patch.object(Elasticsearch, "info")
@patch.object(Elasticsearch, "__init__", return_value=None)
def test_init_retry_then_success(mock_es_init, mock_info, mock_sleep, env_vars):
    """ES 연결이 2번 실패 후 3번째에 성공하는 경우"""
    mock_info.side_effect = [
        ESConnectionError("connection refused"),
        ESConnectionError("connection refused"),
        {"cluster_name": "test"},
    ]

    from es_manager import ESManager
    ESManager()

    assert mock_info.call_count == 3
    assert mock_sleep.call_count == 2
    mock_sleep.assert_any_call(2)   # min(2^1, 10)
    mock_sleep.assert_any_call(4)   # min(2^2, 10)


@patch("es_manager.time.sleep")
@patch.object(Elasticsearch, "info")
@patch.object(Elasticsearch, "__init__", return_value=None)
def test_init_all_retries_fail(mock_es_init, mock_info, mock_sleep, env_vars):
    """ES 연결이 5번 모두 실패하면 예외 발생"""
    mock_info.side_effect = ESConnectionError("connection refused")

    from es_manager import ESManager
    with pytest.raises(ESConnectionError):
        ESManager()

    assert mock_info.call_count == 5
    assert mock_sleep.call_count == 4  # 마지막 시도는 sleep 없이 raise
