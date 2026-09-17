"""classify_proposal_items 테이블을 실제 MySQL 8.0에 대해 검증하는 테스트.

tests/test_category_mapping.py의 fake-cursor 테스트는 "어떤 SQL 문자열을
만들었는가"만 증명한다. CREATE TABLE의 JSON 컬럼/인덱스 문법, utf8mb4 한글
왕복, 100건 배치의 꼬리 유실 여부는 실제 서버에 실행해봐야만 드러난다.
이 파일은 그 네 가지와, clear의 content_type 격리, 빈 테이블 조회를
real MySQL(testcontainers)로 확인한다.
"""

import importlib

import pytest

import backend.category_mapping as category_mapping_mod


@pytest.fixture()
def cm(mysql_container):
    """실제 MySQL 컨테이너에 붙은 CategoryMapping. 테스트마다 대상 테이블을 비운다.

    tests/test_view_history_store.py의 store fixture, tests/test_refresh_token_store.py의
    mysql_store fixture와 같은 패턴이다: 생성자가 _init_db()로 테이블을 만들게 하고,
    본문 실행 전에 TRUNCATE로 이전 테스트의 잔여물을 지운다.

    tests/test_category_mapping.py의 build_cm()은 mock.patch.dict(sys.modules, ...) 블록
    "안"에서 importlib.reload(cm_mod)를 실행한다. 이 reload가 backend.category_mapping
    모듈의 공유 __dict__ 안 전역 이름 pymysql을 fake 객체로 바꿔놓는데, 모듈 __dict__는
    patch.dict가 끝나도 되돌아가지 않는다. 그래서 그 파일이 먼저 실행되면(알파벳 순서상
    이 파일보다 앞) 이후 이 파일에서 만드는 CategoryMapping도 실제 서버 대신 fake cursor에
    조용히 연결된다 — 실측: 이 오염 때문에 아래 5개 테스트가 단독 실행에서는 통과하고
    전체 스위트(tests/ -q)에서만 실패했다. 매번 진짜 pymysql로 다시 reload해서 오염을
    걷어낸 뒤 진행한다.
    """
    importlib.reload(category_mapping_mod)
    mapping = category_mapping_mod.CategoryMapping()
    with mapping._get_connection() as conn:
        with conn.cursor() as cursor:
            cursor.execute("TRUNCATE TABLE classify_proposal_items")
        conn.commit()
    return mapping


def test_create_table_statement_runs_on_real_mysql(cm):
    """CREATE TABLE IF NOT EXISTS classify_proposal_items가 MySQL 8.0에서 실제로
    성공했는지 확인한다. JSON 컬럼, 두 개의 복합/단일 인덱스, VARCHAR(1024)는
    fake cursor로는 문법 오류를 잡을 수 없다 — 여기서 처음 실행된다."""
    with cm._get_connection() as conn:
        with conn.cursor() as cursor:
            cursor.execute("SHOW CREATE TABLE classify_proposal_items")
            row = cursor.fetchone()

    ddl = row["Create Table"]
    assert "`payload` json" in ddl.lower() or "json" in ddl.lower()
    assert "varchar(1024)" in ddl.lower()
    assert "idx_content_source" in ddl.lower()
    assert "idx_content_type" in ddl.lower()


def test_payload_round_trips_korean_nested_null_and_long_path(cm):
    """한글, candidates 중첩 배열(최대 2개), null 값, 매우 긴 file_path가
    add -> get을 거쳐도 그대로 돌아오는지 확인한다. utf8mb4 왕복과 중첩 JSON
    직렬화/역직렬화는 fake cursor 테스트가 증명 못 하는 부분이다."""
    long_path = "/mnt/data/도서/" + "가" * 500 + ".epub"
    item = {"file_path": long_path, "title": "한국어 제목 테스트: 봄날의 기억", "target_category": None, "candidates": [{"category": "3_SF", "source": "bookstore", "detail": "서점 2곳 일치"}, {"category": "5_에세이", "source": "model", "detail": "모델 판정"}]}

    cm.add_classify_proposal_items([item], "0_inbox")
    fetched = cm.get_classify_proposal_items()

    assert len(fetched) == 1
    # apply_status/apply_error는 payload가 아니라 컬럼에서 실려온다. 새로 넣은
    # 행은 아직 아무것도 이동하지 않았으므로 기본값 pending/None이어야 한다.
    assert fetched[0] == {**item, "apply_status": "pending", "apply_error": None}


def test_get_returns_apply_status_and_apply_error_from_columns(cm):
    """get이 payload뿐 아니라 apply_status/apply_error 컬럼 값도 항목에 실어 돌려준다."""
    cm.add_classify_proposal_items([{"file_path": "a.epub"}], "0_inbox")

    cm.update_classify_proposal_item_status("a.epub", "failed", "파일을 찾을 수 없습니다")
    item = cm.get_classify_proposal_items()[0]

    assert item["apply_status"] == "failed"
    assert item["apply_error"] == "파일을 찾을 수 없습니다"


def test_update_status_changes_only_target_row(cm):
    """update가 file_path + content_type으로 지정한 행만 바꾸고 다른 행은 그대로 둔다."""
    cm.add_classify_proposal_items([{"file_path": "a.epub"}, {"file_path": "b.epub"}], "0_inbox")

    cm.update_classify_proposal_item_status("a.epub", "moved")

    items = {item["file_path"]: item for item in cm.get_classify_proposal_items()}
    assert items["a.epub"]["apply_status"] == "moved"
    assert items["a.epub"]["apply_error"] is None
    assert items["b.epub"]["apply_status"] == "pending"


def test_update_status_is_scoped_by_content_type(cm):
    """같은 file_path라도 content_type이 다르면 건드리지 않는다."""
    cm.add_classify_proposal_items([{"file_path": "same.epub"}], "0_inbox", content_type="book")
    cm.add_classify_proposal_items([{"file_path": "same.epub"}], "0_inbox", content_type="comic")

    cm.update_classify_proposal_item_status("same.epub", "moved", content_type="book")

    assert cm.get_classify_proposal_items(content_type="book")[0]["apply_status"] == "moved"
    assert cm.get_classify_proposal_items(content_type="comic")[0]["apply_status"] == "pending"

    # 다음 테스트를 위해 comic도 정리한다 (컨테이너는 세션 스코프로 공유된다)
    cm.clear_classify_proposal_items(content_type="comic")


def test_delete_applied_removes_only_moved_rows_and_returns_count(cm):
    """delete_applied는 apply_status='moved'인 행만 지우고, 지운 개수를 돌려준다."""
    cm.add_classify_proposal_items([{"file_path": "a.epub"}, {"file_path": "b.epub"}, {"file_path": "c.epub"}], "0_inbox")
    cm.update_classify_proposal_item_status("a.epub", "moved")
    cm.update_classify_proposal_item_status("b.epub", "failed", "실패")
    # c.epub은 pending으로 남겨둔다

    deleted_count = cm.delete_applied_classify_proposal_items()

    assert deleted_count == 1
    remaining = {item["file_path"] for item in cm.get_classify_proposal_items()}
    assert remaining == {"b.epub", "c.epub"}


def test_migration_adds_apply_status_columns_to_legacy_table(cm):
    """apply_status/apply_error 컬럼 없는 기존 테이블을 열어도 마이그레이션이 컬럼을 추가한다.

    tests/test_refresh_token_store.py의 test_mysql_migration_adds_columns_to_legacy_table과
    같은 패턴이다: 컬럼 없는 옛 스키마로 테이블을 다시 만들고, 새 인스턴스를 생성해
    _init_db가 마이그레이션을 수행하게 한 뒤 컬럼이 실제로 동작하는지 확인한다.
    """
    with cm._get_connection() as conn:
        with conn.cursor() as cursor:
            cursor.execute("DROP TABLE IF EXISTS classify_proposal_items")
            cursor.execute(
                "CREATE TABLE classify_proposal_items (id INT AUTO_INCREMENT PRIMARY KEY, content_type VARCHAR(10) NOT NULL DEFAULT 'book', source_category VARCHAR(255) NOT NULL, file_path VARCHAR(1024) NOT NULL, payload JSON NOT NULL, created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP, INDEX idx_content_source (content_type, source_category), INDEX idx_content_type (content_type)) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci"
            )
            cursor.execute("INSERT INTO classify_proposal_items (content_type, source_category, file_path, payload) VALUES ('book', '0_inbox', 'a.epub', '{}')")
        conn.commit()

    migrated = category_mapping_mod.CategoryMapping()  # _init_db가 마이그레이션을 수행한다

    items = migrated.get_classify_proposal_items()
    assert items[0]["apply_status"] == "pending"
    assert items[0]["apply_error"] is None
    migrated.update_classify_proposal_item_status("a.epub", "moved")
    assert migrated.get_classify_proposal_items()[0]["apply_status"] == "moved"


def test_batch_of_250_across_three_adds_keeps_full_tail_in_order(cm):
    """설계상 배치 크기인 100건씩(100+100+50) 나눠 넣어도 get이 250건 전부를
    삽입 순서대로 돌려주는지 확인한다. 마지막 잔여분(꼬리)이 누락되면 관리자가
    목록 끝의 책들을 못 보고 승인하게 된다."""
    cm.add_classify_proposal_items([{"file_path": f"{i}.epub"} for i in range(0, 100)], "0_inbox")
    cm.add_classify_proposal_items([{"file_path": f"{i}.epub"} for i in range(100, 200)], "0_inbox")
    cm.add_classify_proposal_items([{"file_path": f"{i}.epub"} for i in range(200, 250)], "0_inbox")

    items = cm.get_classify_proposal_items()

    assert len(items) == 250
    assert [item["file_path"] for item in items] == [f"{i}.epub" for i in range(250)]


def test_get_preserves_insertion_order_across_several_add_calls(cm):
    """화면은 분류가 끝난 순서 그대로 책을 보여줘야 하므로, 여러 번의 add 호출에
    걸쳐 들어간 항목도 삽입 순서를 유지한 채 조회되는지 별도로 확인한다."""
    cm.add_classify_proposal_items([{"file_path": "z.epub"}], "0_inbox")
    cm.add_classify_proposal_items([{"file_path": "a.epub"}], "0_inbox")
    cm.add_classify_proposal_items([{"file_path": "m.epub"}], "0_inbox")

    items = cm.get_classify_proposal_items()

    assert [item["file_path"] for item in items] == ["z.epub", "a.epub", "m.epub"]


def test_clear_only_removes_matching_content_type(cm):
    """clear는 지정한 content_type 행만 지우고 다른 content_type은 그대로 남겨야 한다."""
    cm.add_classify_proposal_items([{"file_path": "book1.epub"}], "0_inbox", content_type="book")
    cm.add_classify_proposal_items([{"file_path": "comic1.epub"}], "0_inbox", content_type="comic")

    cm.clear_classify_proposal_items(content_type="book")

    assert cm.get_classify_proposal_items(content_type="book") == []
    remaining = cm.get_classify_proposal_items(content_type="comic")
    assert [item["file_path"] for item in remaining] == ["comic1.epub"]

    # 다음 테스트를 위해 comic도 정리한다 (컨테이너는 세션 스코프로 공유된다)
    cm.clear_classify_proposal_items(content_type="comic")


def test_get_on_empty_table_returns_empty_list_not_error(cm):
    """아직 아무 제안도 만들지 않은 content_type을 조회하면 빈 리스트여야지,
    에러가 나거나 None이 나오면 안 된다."""
    items = cm.get_classify_proposal_items(content_type="book")

    assert items == []
