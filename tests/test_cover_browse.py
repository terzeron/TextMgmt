"""커버 목록 묶음 규칙(backend/cover_browse.py)을 ES 없이 확인한다."""

from backend.cover_browse import MISSING_TIME, CoverGroup, file_rank, latest_rank, merge_cover_page, pick_representative, plan_cover_scope


def _stats(counts: dict[str, int], updated: dict[str, float] | None = None, created: dict[str, float] | None = None) -> dict:
    """{카테고리: 문서 수}를 ES 집계 결과 모양 (수, 수정 시각, 생성 시각)으로 바꾼다."""
    return {c: (n, (updated or {}).get(c), (created or {}).get(c)) for c, n in counts.items()}


def _hit(book_id: int, title: str, updated: float | None, created: float | None = 0, path: str = "") -> tuple[int, dict, list]:
    path = path or f"{title}.epub"
    return book_id, {"title": title, "file_path": path}, [MISSING_TIME if updated is None else updated, MISSING_TIME if created is None else created, path]


class TestRank:
    def test_newer_comes_first_then_created_then_path(self):
        assert latest_rank(200, 1, "b") < latest_rank(100, 9, "a")
        assert latest_rank(100, 9, "b") < latest_rank(100, 1, "a")
        assert latest_rank(100, 1, "a") < latest_rank(100, 1, "b")

    def test_missing_times_sort_last(self):
        assert latest_rank(100, 1, "a") < latest_rank(None, 1, "a")
        assert latest_rank(100, 1, "a") < latest_rank(100, None, "a")

    def test_file_rank_matches_es_sort_values(self):
        assert file_rank([200, 5, "a.epub"]) == latest_rank(200, 5, "a.epub")
        assert file_rank([MISSING_TIME, MISSING_TIME, "a.epub"]) == latest_rank(None, None, "a.epub")


class TestPlanCoverScope:
    COUNTS = {
        "_root": 2,
        "5_수학과학일반": 100,
        "5_완결": 1,
        "5_완결/[삼촌] 이런 영웅은 싫어": 303,
        "5_완결/암살교실": 185,
        "5_완결/암살교실/외전": 4,
        "0_웹툰/9차원 소녀": 9,
    }

    def plan(self, scope: str = "", hidden: list[str] | None = None, **times):
        return plan_cover_scope(_stats(self.COUNTS, **times), scope, hidden)

    def test_root_scope_keeps_root_files_and_groups_top_level_directories(self):
        files, groups = self.plan()

        # 최상위 디렉토리(5_완결 등)의 직속 파일은 루트 화면에서 펼치지 않고 디렉토리 대표 한 권으로 묶는다.
        assert files == ["_root"]
        assert {group.key for group in groups} == {"5_수학과학일반", "5_완결", "0_웹툰"}

    def test_top_level_directory_direct_files_do_not_leak_into_root_files(self):
        files, _groups = plan_cover_scope(_stats({"3_판타지": 12, "3_무협": 1}))

        assert files == []

    def test_deeper_directories_collapse_into_their_top_level_group(self):
        _files, groups = self.plan()

        assert [group.key for group in groups].count("5_완결") == 1

    def test_groups_are_sorted_newest_first(self):
        _files, groups = self.plan(updated={"0_웹툰/9차원 소녀": 300, "5_완결/암살교실": 200, "5_수학과학일반": 100})

        assert [group.key for group in groups] == ["0_웹툰", "5_완결", "5_수학과학일반"]

    def test_group_time_is_the_newest_time_inside_it(self):
        # 5_완결 안에서 가장 최근 문서가 하위 디렉토리(외전)에 있어도 5_완결 묶음의 시각이 된다.
        _files, groups = self.plan(updated={"5_완결/암살교실/외전": 500, "5_완결/암살교실": 10, "0_웹툰/9차원 소녀": 100}, created={"5_완결": 7})

        by_key = {group.key: group for group in groups}
        assert by_key["5_완결"].updated == 500
        assert by_key["5_완결"].created == 7
        assert [group.key for group in groups][0] == "5_완결"

    def test_groups_without_times_go_last_and_ties_use_key(self):
        _files, groups = plan_cover_scope(_stats({"b/x": 1, "a/x": 1, "c/x": 1}, updated={"c/x": 5}))

        assert [group.key for group in groups] == ["c", "a", "b"]

    def test_selected_directory_keeps_direct_files_and_groups_children(self):
        files, groups = self.plan("5_완결", updated={"5_완결/암살교실": 9, "5_완결/[삼촌] 이런 영웅은 싫어": 1})

        assert files == ["5_완결"]
        assert [group.key for group in groups] == ["5_완결/암살교실", "5_완결/[삼촌] 이런 영웅은 싫어"]

    def test_selected_work_directory_groups_its_subdirectories(self):
        files, groups = self.plan("5_완결/암살교실")

        assert files == ["5_완결/암살교실"]
        assert [(group.name, group.key) for group in groups] == [("외전", "5_완결/암살교실/외전")]

    def test_selected_directory_without_direct_files_has_only_groups(self):
        files, groups = plan_cover_scope(_stats({"a/b": 1, "a/c": 1}), "a")

        assert files == []
        assert {group.name for group in groups} == {"b", "c"}

    def test_scope_does_not_match_sibling_with_same_prefix(self):
        files, groups = plan_cover_scope(_stats({"5_완결": 1, "5_완결2": 1, "5_완결2/x": 1}), "5_완결")

        assert files == ["5_완결"]
        assert groups == []

    def test_hidden_categories_are_dropped_with_their_subtrees(self):
        files, groups = self.plan(hidden=["5_완결/암살교실", "5_수학과학일반"])

        assert files == ["_root"]
        assert {group.key for group in groups} == {"5_완결", "0_웹툰"}

    def test_hidden_top_level_directory_hides_its_group(self):
        _files, groups = self.plan(hidden=["5_완결"])

        assert {group.key for group in groups} == {"5_수학과학일반", "0_웹툰"}

    def test_cache_and_extraction_directories_are_not_listed(self):
        counts = {".preview_cache": 48, "소설/.hidden": 1, "소설/OEBPS": 3, "소설/작품": 5, "0_trash": 52}

        _files, root_groups = plan_cover_scope(_stats(counts))
        _files, scoped = plan_cover_scope(_stats(counts), "소설")

        assert {group.key for group in root_groups} == {"소설", "0_trash"}
        assert [group.key for group in scoped] == ["소설/작품"]


class TestCoverGroup:
    def test_cursor_values_round_trip_into_the_same_rank(self):
        group = CoverGroup("작품", "a/작품", 123.0, 45.0)

        assert latest_rank(*group.cursor_values) == group.rank

    def test_cursor_values_keep_missing_times(self):
        group = CoverGroup("작품", "a/작품")

        assert group.cursor_values == [None, None, "a/작품"]
        assert latest_rank(*group.cursor_values) == group.rank


class TestPickRepresentative:
    def test_prefers_first_installment(self):
        hits = [(1, {"title": "작품 02권", "file_path": "d/작품 02권.pdf"}), (2, {"title": "작품 01권", "file_path": "d/작품 01권.pdf"})]

        assert pick_representative(hits)[0] == 2

    def test_does_not_confuse_numbers_that_merely_contain_one(self):
        hits = [(1, {"title": "작품 11권", "file_path": "d/작품 11권.pdf"}), (2, {"title": "작품 21화", "file_path": "d/작품 21화.pdf"})]

        assert pick_representative(hits)[0] == 1

    def test_falls_back_to_first_candidate(self):
        hits = [(7, {"title": "가", "file_path": "d/가.pdf"}), (8, {"title": "나", "file_path": "d/나.pdf"})]

        assert pick_representative(hits)[0] == 7

    def test_returns_none_without_candidates(self):
        assert pick_representative([]) is None

    def test_matches_on_file_name_when_title_lacks_installment(self):
        hits = [(1, {"title": "작품", "file_path": "d/작품 02화.pdf"}), (2, {"title": "작품", "file_path": "d/작품 1화.pdf"})]

        assert pick_representative(hits)[0] == 2


class TestMergeCoverPage:
    def test_merges_groups_and_files_newest_first(self):
        groups = [CoverGroup("나", "x/나", 400), CoverGroup("마", "x/마", 100)]
        files = [_hit(1, "가", 500), _hit(2, "다", 200)]

        entries, groups_used, files_used = merge_cover_page(groups, files, 10)

        assert [(kind, value.name if kind == "group" else value[1]["title"]) for kind, value in entries] == [("file", "가"), ("group", "나"), ("file", "다"), ("group", "마")]
        assert (groups_used, files_used) == (2, 2)

    def test_group_comes_first_when_rank_ties(self):
        entries, _g, _f = merge_cover_page([CoverGroup("x", "d", 100, 0)], [_hit(1, "x", 100, 0, path="d")], 10)

        assert [kind for kind, _value in entries] == ["group", "file"]

    def test_files_without_times_go_after_groups_with_times(self):
        entries, _g, _f = merge_cover_page([CoverGroup("작품", "x/작품", 100)], [_hit(1, "오래된 책", None)], 10)

        assert [kind for kind, _value in entries] == ["group", "file"]

    def test_stops_at_size_and_reports_how_many_were_used(self):
        groups = [CoverGroup("나", "x/나", 400), CoverGroup("라", "x/라", 100)]
        files = [_hit(1, "가", 500), _hit(2, "다", 200)]

        entries, groups_used, files_used = merge_cover_page(groups, files, 3)

        assert len(entries) == 3
        assert (groups_used, files_used) == (1, 2)

    def test_handles_one_empty_side(self):
        assert merge_cover_page([], [_hit(1, "가", 1)], 5)[1:] == (0, 1)
        assert merge_cover_page([CoverGroup("가", "x/가", 1)], [], 5)[1:] == (1, 0)
        assert merge_cover_page([], [], 5) == ([], 0, 0)
