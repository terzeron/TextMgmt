#!/usr/bin/env python

"""외부 파서(pdfminer, pypdf, pdfium)에 벽시계 상한을 거는 공용 유틸.

loader.py와 isbn.py가 함께 쓴다. loader.py가 isbn.py를 import하므로 반대 방향
import는 순환이 되어, 두 모듈이 함께 의존할 수 있도록 별도 모듈로 둔다.
"""

import signal
import threading
from contextlib import contextmanager
from typing import Any, Iterator


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
