#!/usr/bin/env python

"""외부 파서(pdfminer, pypdf, pdfium)에 벽시계 상한을 거는 공용 유틸.

loader.py와 isbn.py가 함께 쓴다. loader.py가 isbn.py를 import하므로 반대 방향
import는 순환이 되어, 두 모듈이 함께 의존할 수 있도록 별도 모듈로 둔다.
"""

import multiprocessing
import signal
import threading
from contextlib import contextmanager
from typing import Any, Callable, Iterator


class ParserTimeout(BaseException):
    """파서가 제한 시간 안에 끝나지 않음.

    Exception이 아니라 BaseException을 상속한다. pdfminer 등 파서 내부에는 예외를
    광범위하게 삼키는 지점이 많아, Exception을 상속하면 타임아웃이 그 자리에서
    잡아먹힌다. (실측: Exception 상속 시 30초 상한이 전혀 걸리지 않고 65초 소요)
    """


# 알람이 삼켜졌을 때 다시 울리기까지의 간격.
# BaseException으로도 pdfminer 깊은 곳의 bare `except:`는 뚫지 못한다. 일회성 알람은
# 그 지점에서 소비되고 두 번 다시 울리지 않으므로(실측: 30초 상한이 104.7초까지 밀림)
# 예외가 실제로 빠져나올 때까지 반복해서 울린다.
PARSER_TIMEOUT_RETRY_INTERVAL = 0.5


@contextmanager
def time_limit(seconds: float, message: str) -> Iterator[None]:
    """SIGALRM으로 블록 전체에 벽시계 제한 시간을 건다.

    손상 PDF는 파일을 다 읽은 뒤 메모리에서만 무한히 도는 경우가 있어(읽기 syscall이
    멈춘 채 CPU만 100%), 페이지 루프 안의 검사만으로는 빠져나올 수 없다. 열기 단계까지
    포함해 덮으려면 시그널이 필요하다.

    signal은 메인 스레드에서만 설치할 수 있다. 워커 스레드에서는 제한을 걸지 못하므로
    그대로 실행한다(FastAPI 동기 엔드포인트 등).
    """
    if threading.current_thread() is not threading.main_thread():
        yield
        return

    def _on_alarm(_signum: int, _frame: Any) -> None:
        raise ParserTimeout(message)

    previous_handler = signal.signal(signal.SIGALRM, _on_alarm)
    signal.setitimer(signal.ITIMER_REAL, seconds, PARSER_TIMEOUT_RETRY_INTERVAL)
    try:
        yield
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        signal.signal(signal.SIGALRM, previous_handler)


def _run_target(result_queue: Any, func: Callable[..., Any], args: tuple[Any, ...], kwargs: dict[str, Any]) -> None:
    try:
        result_queue.put(("ok", func(*args, **kwargs)))
    except BaseException as e:  # noqa: BLE001 - 자식 프로세스 예외를 부모로 그대로 전달
        result_queue.put(("error", e))


def run_with_hard_timeout(func: Callable[..., Any], args: tuple[Any, ...] = (), kwargs: dict[str, Any] | None = None, timeout: float = 150) -> Any:
    """`func(*args, **kwargs)`를 별도 프로세스에서 실행하고, 시간 초과 시 프로세스를 강제 종료한다.

    `time_limit()`의 SIGALRM 상한은 메인 스레드에서만 걸리므로 `asyncio.to_thread`로
    호출되는 워커 스레드 경로(카테고리 대량 재적재 등)에서는 무력화된다. 이 함수는
    스레드가 아니라 별도 OS 프로세스로 실행을 격리해 호출 스레드와 무관하게 동작하는
    타임아웃을 제공한다: 시간을 넘기면 `process.kill()`로 강제 종료하므로, 시그널조차
    빠져나오지 못하는 경우(손상 PDF가 C 확장 안에서 도는 등)까지 방어한다.

    `fork` 컨텍스트를 쓴다: 호출 시점의 부모 프로세스 메모리를 그대로 복사하므로
    `unittest.mock`/`monkeypatch`로 교체한 `func`도 자식에서 그대로 보인다(스레드
    동시 호출로 인한 pypdfium2 전역 상태 손상은 이 경로에서 발생하지 않는다 - 자식마다
    파싱을 전담하는 단일 스레드만 존재).

    멀티스레드 프로세스에서 fork()하면 부모의 다른 스레드가 쥐고 있던 락(logging,
    malloc 등)을 자식이 통째로 물려받아 그 락을 다시 잡으려 할 때 자식이 멈출 수
    있다는 일반적인 위험이 있다. 이 함수는 그런 경우에도 시간 초과 시 SIGKILL로
    자식을 강제 종료하므로(사용자 공간 락은 SIGKILL을 막지 못한다), 최악의 경우도
    "그 파일 하나가 timeout 만큼 걸리다 실패 처리"로 유계(bounded)된다 - 무한 정지가
    아니다. forkserver/spawn은 이 락 상속 문제 자체는 없지만 자식이 모듈을 새로
    import하므로 monkeypatch가 보이지 않아 테스트가 깨진다.
    """
    ctx = multiprocessing.get_context("fork")
    result_queue = ctx.SimpleQueue()
    process = ctx.Process(target=_run_target, args=(result_queue, func, args, kwargs or {}), daemon=True)
    process.start()
    process.join(timeout)
    if process.is_alive():
        process.kill()
        process.join()
        raise ParserTimeout(f"처리 시간 초과({timeout}초): {getattr(func, '__qualname__', func)}")
    if result_queue.empty():
        raise ParserTimeout(f"처리가 비정상 종료되었습니다(exit code {process.exitcode}): {getattr(func, '__qualname__', func)}")
    status, payload = result_queue.get()
    if status == "error":
        raise payload
    return payload
