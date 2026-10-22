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


# --- ctypes 콜백 안에서 난 타임아웃은 stderr 를 더럽히지 않는다 ---
#
# pypdfium2 는 파일 객체를 읽을 때마다 C 에서 Python 콜백(_buffer_reader.__call__)을 부른다.
# 알람이 그 콜백 안에서 울리면 ParserTimeout 이 ctypes 경계를 넘지 못하고
# "Exception ignored on calling ctypes callback function" 으로 찍힌다.
# 콜백 실패는 pdfium 에 읽기 실패(0)로 전달돼 오히려 빨리 빠져나오게 하고, 알람이 0.5초 간격으로
# 반복되므로 다음 알람이 Python 코드에서 정상 전파된다. 기능은 멀쩡하고 소음만 남는다.


def _fire_in_ctypes_callback(exc: BaseException) -> None:
    """실제 ctypes 콜백 안에서 예외를 일으킨다. ctypes 가 삼켜서 sys.unraisablehook 으로 보낸다."""
    import ctypes

    def raiser() -> int:
        raise exc

    ctypes.CFUNCTYPE(ctypes.c_int)(raiser)()


def test_time_limit_swallows_noise_of_timeout_raised_inside_ctypes_callback(monkeypatch):
    import sys

    seen = []
    monkeypatch.setattr(sys, "unraisablehook", lambda unraisable: seen.append(unraisable.exc_value))

    with time_limit(30, "test"):
        _fire_in_ctypes_callback(ParserTimeout("test"))

    assert seen == []


def test_time_limit_still_reports_other_exceptions_swallowed_by_ctypes(monkeypatch):
    """ParserTimeout 만 거르고, 그 밖의 삼켜진 예외는 원래 훅으로 그대로 보낸다."""
    import sys

    seen = []
    monkeypatch.setattr(sys, "unraisablehook", lambda unraisable: seen.append(unraisable.exc_value))

    with time_limit(30, "test"):
        _fire_in_ctypes_callback(ValueError("진짜 버그"))

    assert [type(e) for e in seen] == [ValueError]


def test_time_limit_restores_unraisablehook_on_exit_and_on_error(monkeypatch):
    import sys

    def original(unraisable):
        pass

    monkeypatch.setattr(sys, "unraisablehook", original)
    with time_limit(30, "test"):
        assert sys.unraisablehook is not original
    assert sys.unraisablehook is original

    with pytest.raises(RuntimeError):
        with time_limit(30, "test"):
            raise RuntimeError("boom")
    assert sys.unraisablehook is original


def test_time_limit_leaves_unraisablehook_alone_in_worker_thread(monkeypatch):
    """signal 을 못 거는 워커 스레드에서는 훅도 건드리지 않는다(전역 상태를 바꾸지 않는다)."""
    import sys

    def original(unraisable):
        pass

    monkeypatch.setattr(sys, "unraisablehook", original)
    result = {}

    def work():
        with time_limit(30, "test"):
            result["inside"] = sys.unraisablehook

    t = threading.Thread(target=work)
    t.start()
    t.join()
    assert result["inside"] is original

