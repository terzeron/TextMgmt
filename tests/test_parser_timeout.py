import os
import threading
import time
import pytest
from utils.parser_timeout import (
    ParserTimeout,
    time_limit,
    _run_target,
    run_with_hard_timeout,
)


def test_time_limit_normal():
    with time_limit(1.0, "Timeout"):
        x = 1 + 1
    assert x == 2


def test_time_limit_timeout():
    with pytest.raises(ParserTimeout) as exc:
        with time_limit(0.1, "Exceeded limit"):
            time.sleep(0.3)
    assert "Exceeded limit" in str(exc.value)


def test_time_limit_worker_thread():
    result = []
    def worker():
        with time_limit(0.1, "Worker timeout"):
            time.sleep(0.05)
            result.append(42)
    t = threading.Thread(target=worker)
    t.start()
    t.join()
    assert result == [42]


def test_run_target_success_and_error():
    class DummyQueue:
        def __init__(self):
            self.items = []
        def put(self, val):
            self.items.append(val)

    q = DummyQueue()
    _run_target(q, lambda a, b: a + b, (1, 2), {})
    assert q.items == [("ok", 3)]

    q_err = DummyQueue()
    def fail_fn():
        raise ValueError("Something went wrong")
    _run_target(q_err, fail_fn, (), {})
    assert q_err.items[0][0] == "error"
    assert isinstance(q_err.items[0][1], ValueError)


def test_run_with_hard_timeout_success():
    res = run_with_hard_timeout(lambda x, y: x * y, args=(3, 4), timeout=2.0)
    assert res == 12


def test_run_with_hard_timeout_error():
    def raise_err():
        raise KeyError("missing")
    with pytest.raises(KeyError):
        run_with_hard_timeout(raise_err, timeout=2.0)


def test_run_with_hard_timeout_timeout():
    def slow_fn():
        time.sleep(1.0)
    with pytest.raises(ParserTimeout) as exc:
        run_with_hard_timeout(slow_fn, timeout=0.1)
    assert "처리 시간 초과" in str(exc.value)


def test_run_with_hard_timeout_empty_queue_crash():
    def crash_fn():
        os._exit(1)
    with pytest.raises(ParserTimeout) as exc:
        run_with_hard_timeout(crash_fn, timeout=2.0)
    assert "처리가 비정상 종료되었습니다" in str(exc.value)
