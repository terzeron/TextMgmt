"""classify_proposal_status 를 실제 MySQL 8.0 에 대해 검증하는 테스트.

이 상태는 원래 corpus 디렉토리의 JSON 파일이었다. 파일로는 "읽고-판단하고-쓰기"에
잠금을 걸 수 없어, 프로세스가 여럿이면 두 작업이 동시에 시작할 수 있었다. 이 서비스는
uvicorn --workers 2 에 replicas 2 라 실제로 프로세스가 4개다.

여기서 확인하는 것은 SQL 문자열이 아니라 서버가 실제로 보장하는 성질이다:
동시에 들어온 두 시작 요청 중 하나만 성공하는가, JSON_MERGE_PATCH 가 status 를
건드리지 않고 필드만 합치는가, 하트비트가 끊긴 작업이 조회에서만 failed 로 보이고
행은 그대로인가. fake cursor 로는 셋 다 증명할 수 없다.
"""

import threading

import pytest

import backend.category_mapping as category_mapping_mod

STALE = 300


@pytest.fixture()
def cm(mysql_container):
    """실제 MySQL 컨테이너에 붙은 CategoryMapping. 테스트마다 상태 행을 비운다."""
    mapping = category_mapping_mod.CategoryMapping()
    with mapping._get_connection() as conn:
        with conn.cursor() as cursor:
            cursor.execute("TRUNCATE TABLE classify_proposal_status")
        conn.commit()
    return mapping


def _age_the_heartbeat(mapping, seconds: int, content_type: str = "book") -> None:
    """updated_at 을 과거로 밀어 '갱신이 끊긴' 작업을 만든다."""
    # DB 서버의 시계로 민다. 파이썬 지역 시간을 쓰면 앱(KST)과 DB(UTC)의 시간대 차이가
    # 그대로 섞여, 테스트가 의도한 것과 다른 나이를 만들어낸다.
    with mapping._get_connection() as conn:
        with conn.cursor() as cursor:
            cursor.execute("UPDATE classify_proposal_status SET updated_at = DATE_SUB(NOW(3), INTERVAL %s SECOND) WHERE content_type = %s", (seconds, content_type))
        conn.commit()


def test_status_starts_idle_when_nothing_ran_yet(cm):
    """작업 이력이 없으면 idle 이다. 행이 아예 없어도 예외 없이 답해야 한다."""
    assert cm.get_classify_proposal_status(stale_seconds=STALE)["status"] == "idle"


def test_set_and_get_round_trips_korean_and_nested_values(cm):
    """카운트, 한글 카테고리명, 실패 목록이 그대로 돌아온다."""
    cm.set_classify_proposal_status({"status": "ready", "source_category": "3_판타지", "total_count": 240, "processed_count": 240, "failures": [{"file_path": "3_판타지/책.epub", "error": "분류 제안에 실패했습니다"}]})

    status = cm.get_classify_proposal_status(stale_seconds=STALE)

    assert status["status"] == "ready"
    assert status["source_category"] == "3_판타지"
    assert status["total_count"] == 240
    assert status["failures"][0]["error"] == "분류 제안에 실패했습니다"


def test_merge_updates_counts_without_touching_status(cm):
    """진행률 갱신이 status 를 건드리면 안 된다.

    제안(running)과 적용(applying)이 같은 행을 쓴다. 진행률이 status 를 되돌리면
    적용 중인 작업이 화면에 '제안 생성 중'으로 보인다. 호출자가 status 를 실어
    보내더라도 무시해야 한다.
    """
    cm.set_classify_proposal_status({"status": "applying", "source_category": "3_판타지", "total_count": 240})

    cm.merge_classify_proposal_status({"processed_count": 52, "status": "running"})

    status = cm.get_classify_proposal_status(stale_seconds=STALE)
    assert status["status"] == "applying", "진행률 갱신이 status 를 되돌렸다"
    assert status["processed_count"] == 52
    assert status["source_category"] == "3_판타지", "합치는 대신 통째로 덮어썼다"
    assert status["total_count"] == 240


def test_stale_running_job_reads_as_failed_without_changing_the_row(cm):
    """하트비트가 끊긴 running 은 조회에서만 failed 로 보이고 행은 그대로다.

    조회가 행을 고치면 GET 요청이 쓰기가 되고, 여러 클라이언트가 동시에 폴링할 때
    서로의 쓰기와 경합한다. get_reload_status 와 같은 방식이다.
    """
    cm.set_classify_proposal_status({"status": "running", "source_category": "3_판타지"})
    _age_the_heartbeat(cm, STALE + 60)

    assert cm.get_classify_proposal_status(stale_seconds=STALE)["status"] == "failed"

    with cm._get_connection() as conn:
        with conn.cursor() as cursor:
            cursor.execute("SELECT status FROM classify_proposal_status WHERE content_type = 'book'")
            assert cursor.fetchone()["status"] == "running", "조회가 행을 고쳤다"


def test_start_is_rejected_while_another_job_runs(cm):
    """도는 작업이 있으면 새 제안을 시작하지 못하고, 도는 작업의 상태를 돌려준다."""
    started, _ = cm.try_start_classify_proposal({"status": "running", "source_category": "3_판타지"}, stale_seconds=STALE)
    assert started

    started_again, current = cm.try_start_classify_proposal({"status": "running", "source_category": "5_에세이"}, stale_seconds=STALE)

    assert not started_again
    assert current["source_category"] == "3_판타지"


def test_start_takes_over_a_job_whose_heartbeat_died(cm):
    """pod 가 죽어 하트비트가 끊긴 작업은 새 작업이 이어받을 수 있어야 한다.

    이게 없으면 한 번 죽은 작업이 그 카테고리를 영영 막는다.
    """
    cm.try_start_classify_proposal({"status": "running", "source_category": "3_판타지"}, stale_seconds=STALE)
    _age_the_heartbeat(cm, STALE + 60)

    started, current = cm.try_start_classify_proposal({"status": "running", "source_category": "5_에세이"}, stale_seconds=STALE)

    assert started
    assert current["source_category"] == "5_에세이"


@pytest.mark.parametrize("current_status,expected", [("ready", True), ("done", True), ("failed", True), ("idle", False), ("running", False), ("applying", False)])
def test_apply_is_accepted_only_when_no_job_is_running(cm, current_status, expected):
    """승인은 ready 뿐 아니라 done, failed 에서도 받는다.

    승인 도중 중단되면 상태가 applying 으로 남았다가 하트비트 만료로 failed 가 된다.
    그때도 남은 pending 행을 이어서 승인할 수 있어야 관리자가 반쯤 옮겨진 상태에서
    빠져나올 수 있다.
    """
    cm.set_classify_proposal_status({"status": current_status, "source_category": "3_판타지"})

    begun, _ = cm.try_begin_classify_apply("token-1", stale_seconds=STALE)

    assert begun is expected


def test_apply_keeps_the_proposal_counts_it_found(cm):
    """적용을 선점해도 제안 단계가 남긴 카운트는 지워지지 않는다.

    이걸 덮어쓰면 화면 헤더가 제안 진행률 대신 승인 건수를 보여준다.
    """
    cm.set_classify_proposal_status({"status": "ready", "source_category": "3_판타지", "total_count": 240, "processed_count": 240})

    begun, status = cm.try_begin_classify_apply("token-1", stale_seconds=STALE)

    assert begun
    assert status["status"] == "applying"
    assert status["apply_token"] == "token-1"
    assert status["total_count"] == 240
    assert status["source_category"] == "3_판타지"


def test_only_one_of_two_simultaneous_starts_wins(cm):
    """이 변경의 핵심이다. 두 프로세스가 같은 순간에 시작해도 하나만 성공한다.

    파일 방식에서는 둘 다 idle 을 읽고 둘 다 running 을 써서 제안 작업이 두 개
    돌았다. 각자 자기 커넥션을 쓰는 스레드 두 개로 실제 경합을 만들어 확인한다.
    """
    barrier = threading.Barrier(2)
    results: list[bool] = []
    lock = threading.Lock()

    def start(category: str) -> None:
        mapping = category_mapping_mod.CategoryMapping()
        barrier.wait()
        started, _ = mapping.try_start_classify_proposal({"status": "running", "source_category": category}, stale_seconds=STALE)
        with lock:
            results.append(started)

    threads = [threading.Thread(target=start, args=(name,)) for name in ("3_판타지", "5_에세이")]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()

    assert sorted(results) == [False, True], f"동시 시작이 둘 다 통과했다: {results}"


def test_only_one_of_two_simultaneous_applies_wins(cm):
    """승인도 마찬가지다. 둘 다 통과하면 같은 파일을 두 작업이 옮기려 든다."""
    cm.set_classify_proposal_status({"status": "ready", "source_category": "3_판타지"})
    barrier = threading.Barrier(2)
    results: list[bool] = []
    lock = threading.Lock()

    def begin(token: str) -> None:
        mapping = category_mapping_mod.CategoryMapping()
        barrier.wait()
        begun, _ = mapping.try_begin_classify_apply(token, stale_seconds=STALE)
        with lock:
            results.append(begun)

    threads = [threading.Thread(target=begin, args=(token,)) for token in ("token-a", "token-b")]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()

    assert sorted(results) == [False, True], f"동시 승인이 둘 다 통과했다: {results}"


def test_status_is_isolated_by_content_type(cm):
    """book 과 comic 은 서로의 상태를 건드리지 않는다."""
    cm.set_classify_proposal_status({"status": "running", "source_category": "3_판타지"}, content_type="book")
    cm.set_classify_proposal_status({"status": "ready", "source_category": "만화"}, content_type="comic")

    assert cm.get_classify_proposal_status(content_type="book", stale_seconds=STALE)["status"] == "running"
    assert cm.get_classify_proposal_status(content_type="comic", stale_seconds=STALE)["source_category"] == "만화"

