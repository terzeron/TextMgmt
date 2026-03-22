import pytest


def _set_es_env(monkeypatch):
    monkeypatch.setenv("TM_ES_BOOK_INDEX", "test_index")
    monkeypatch.setenv("TM_ES_URL", "http://localhost:9200")
    monkeypatch.setenv("TM_ES_USER", "elastic")
    monkeypatch.setenv("TM_ES_PASSWORD", "changeme")


def test_test_es_retry(monkeypatch):
    _set_es_env(monkeypatch)
    from backend import test_es_retry as ter
    from backend import es_manager as esm
    from unittest.mock import MagicMock

    class DummyES:
        def __init__(self, *args, **kwargs):
            return None

        def info(self):
            return mock_info()

    mock_info = MagicMock(return_value={"cluster_name": "test"})
    mock_sleep = MagicMock()
    monkeypatch.setattr(esm, "Elasticsearch", DummyES)
    monkeypatch.setattr(esm.time, "sleep", mock_sleep)

    wrapped_success = ter.test_init_success_first_try.__wrapped__
    wrapped_success(MagicMock(), mock_info, None)
    assert mock_info.call_count == 1

    mock_info.reset_mock()
    mock_info.side_effect = [esm.ConnectionError("connection refused"), esm.ConnectionError("connection refused"), {"cluster_name": "test"}]
    wrapped_retry = ter.test_init_retry_then_success.__wrapped__
    wrapped_retry(MagicMock(), mock_info, mock_sleep, None)
    assert mock_sleep.call_count == 2

    mock_info.reset_mock()
    mock_sleep.reset_mock()
    mock_info.side_effect = esm.ConnectionError("connection refused")
    wrapped_fail = ter.test_init_all_retries_fail.__wrapped__
    wrapped_fail(MagicMock(), mock_info, mock_sleep, None)
