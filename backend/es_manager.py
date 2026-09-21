#!/usr/bin/env python

import sys
import os
import math
import logging.config
from pathlib import Path
from typing import Any, cast
from collections.abc import Iterator
from itertools import islice
import time
from elasticsearch import Elasticsearch
from elastic_transport import SerializationError, ConnectionError, ConnectionTimeout
from utils.file_time import path_created_time_with_source


logging.config.fileConfig(Path(__file__).parent.parent / "logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger(__name__)
logging.getLogger("elasticsearch").setLevel(logging.CRITICAL)


class ESManager:
    DEFAULT_MAX_RESULT_COUNT = 10

    # 목록 응답(Book.dict()/Comics.dict())에 summary 가 없는데도 _source 를 통째로
    # 받아오면 500건 페이지가 3.67MB 가 되고 그 97.7%가 파싱 직후 버려지는 summary 다.
    # 여기 필드만 받으면 같은 페이지가 83KB 다 (2026-09-21 실측, 44배).
    #
    # summary 가 필요한 경로는 search_by_id(단건 get)뿐이므로 거기에는 적용하지 않는다.
    # 분류기(backend/classifier/corpus.py)는 self.es 를 직접 쓰고 자체 SOURCE_FIELDS 를
    # 넘기므로 이 상수의 영향을 받지 않는다.
    LIST_SOURCE_FIELDS = ["category", "title", "author", "file_path", "file_type", "file_size", "line_count", "page_count", "isbn", "created_time", "updated_time"]

    def __init__(self, index_name: str = "") -> None:
        for env in ["TM_ES_BOOK_INDEX", "TM_ES_URL", "TM_ES_USER", "TM_ES_PASSWORD"]:
            if env not in os.environ:
                LOGGER.error(f"The environment variable {env} is not set.")
                sys.exit(-1)

        self.index_name = index_name or os.environ["TM_ES_BOOK_INDEX"]
        url = os.environ["TM_ES_URL"]
        user = os.environ["TM_ES_USER"]
        password = os.environ["TM_ES_PASSWORD"]

        es_kwargs: dict[str, Any] = {"hosts": [url], "basic_auth": (user, password), "request_timeout": 10, "retry_on_timeout": True}
        if url.lower().startswith("https"):
            # ECK 자체서명 인증서: 전송 구간은 TLS로 암호화하되 인증서 검증은 생략한다.
            es_kwargs["verify_certs"] = False
            es_kwargs["ssl_show_warn"] = False
        self.es = Elasticsearch(**es_kwargs)

        max_retries = 5
        for attempt in range(max_retries):
            try:
                self.es.info()
                LOGGER.info("Elasticsearch 연결 성공")
                break
            except (ConnectionError, ConnectionTimeout, SerializationError) as e:
                if attempt < max_retries - 1:
                    wait = min(2 ** (attempt + 1), 10)
                    LOGGER.warning("ES 연결 실패 (시도 %d/%d): %s. %d초 후 재시도...", attempt + 1, max_retries, e, wait)
                    time.sleep(wait)
                else:
                    LOGGER.error("ES 연결 최종 실패: %s", e)
                    raise

    def __del__(self) -> None:
        if hasattr(self, "es"):
            del self.es

    def do_exist_index(self) -> bool:
        LOGGER.debug("do_exist_index()")
        return bool(self.es.indices.exists(index=self.index_name))

    def get_existing_paths(self, doc_ids: list[int]) -> dict[int, str]:
        """주어진 ID 목록의 기존 file_path를 조회. 반환: {inode: file_path}"""
        if not doc_ids:
            return {}
        LOGGER.debug("get_existing_paths(%d ids)", len(doc_ids))
        docs = [{"_index": self.index_name, "_id": str(doc_id)} for doc_id in doc_ids]
        response = self.es.mget(docs=docs, source=["file_path"])
        result: dict[int, str] = {}
        for doc in response["docs"]:
            if doc.get("found", False):
                result[int(doc["_id"])] = doc["_source"]["file_path"]
        return result

    def create_index(self) -> dict[str, Any]:
        LOGGER.debug("create_index()")
        from elasticsearch import BadRequestError

        settings = {
            "index": {"similarity": {"default": {"type": "BM25"}}},
            "analysis": {
                "tokenizer": {"nori_tokenizer": {"type": "nori_tokenizer", "decompound_mode": "discard"}},
                "filter": {
                    "nori_posfilter": {
                        "type": "nori_part_of_speech",
                        "stoptags": [
                            # 어미 (Ending)
                            "EC",  # 연결 어미
                            "EF",  # 종결 어미
                            "EP",  # 선어말 어미
                            "ETM",  # 관형형 전성 어미
                            "ETN",  # 명사형 전성 어미
                            # 조사 (Josa)
                            "JC",  # 접속 조사
                            "JKB",  # 부사격 조사
                            "JKC",  # 보격 조사
                            "JKG",  # 관형격 조사
                            "JKO",  # 목적격 조사
                            "JKQ",  # 인용격 조사
                            "JKS",  # 주격 조사
                            "JKV",  # 호격 조사
                            "JX",  # 보조사
                            # 기호
                            "SC",  # 구분자
                            "SE",  # 줄임표
                            "SF",  # 마침표, 물음표, 느낌표
                            "SP",  # 공백
                            "SSC",  # 닫는 괄호
                            "SSO",  # 여는 괄호
                            "SY",  # 기타 기호
                            # 접미사
                            "XSA",  # 형용사 파생 접미사
                            "XSN",  # 명사 파생 접미사
                            "XSV",  # 동사 파생 접미사
                            # 기타
                            "IC",  # 감탄사
                            "MAJ",  # 접속부사
                        ],
                    }
                },
                "analyzer": {"nori_analyzer": {"type": "custom", "tokenizer": "nori_tokenizer", "filter": ["nori_posfilter", "lowercase"]}},
            },
        }
        mappings = {
            "properties": {
                "category": {"type": "keyword", "fields": {"nori": {"type": "text", "analyzer": "nori_analyzer"}}},
                "title": {"type": "text", "analyzer": "nori_analyzer", "fields": {"keyword": {"type": "keyword"}}},
                "author": {"type": "text", "analyzer": "nori_analyzer", "fields": {"keyword": {"type": "keyword"}}},
                # EPUB 의 dc:publisher. 전집 판정의 결정적 증거라 자동분류가 쓴다.
                "publisher": {"type": "text", "analyzer": "nori_analyzer", "fields": {"keyword": {"type": "keyword"}}},
                "file_path": {"type": "keyword"},
                "file_type": {"type": "keyword"},
                "file_size": {"type": "unsigned_long"},
                "line_count": {"type": "integer"},
                "page_count": {"type": "integer"},
                "isbn": {"type": "keyword"},
                "summary": {"type": "text", "analyzer": "nori_analyzer"},
                "created_time": {"type": "date"},
                "created_time_source": {"type": "keyword"},
                "updated_time": {"type": "date"},
            }
        }

        if self.do_exist_index():
            self._ensure_existing_index_mappings()
            return {"acknowledged": True}
        try:
            return cast(dict[str, Any], self.es.indices.create(index=self.index_name, body={"settings": settings, "mappings": mappings}))
        except BadRequestError as e:
            if "resource_already_exists_exception" in str(e):
                LOGGER.info("Index %s already exists, skipping creation", self.index_name)
                self._ensure_existing_index_mappings()
                return {"acknowledged": True}
            raise

    def _ensure_existing_index_mappings(self) -> None:
        self._ensure_category_nori_subfield()
        self._ensure_created_time_field()
        self._ensure_publisher_field()

    def _ensure_category_nori_subfield(self) -> None:
        """기존 인덱스에 category.nori 서브필드가 없으면 추가"""
        try:
            mapping = self.es.indices.get_mapping(index=self.index_name)
            cat_props = mapping[self.index_name]["mappings"]["properties"].get("category", {})
            if "fields" in cat_props and "nori" in cat_props["fields"]:
                return
            LOGGER.info("Adding category.nori sub-field to index %s", self.index_name)
            self.es.indices.put_mapping(index=self.index_name, properties={"category": {"type": "keyword", "fields": {"nori": {"type": "text", "analyzer": "nori_analyzer"}}}})
            LOGGER.info("category.nori sub-field added successfully")
        except Exception as e:
            LOGGER.warning("Failed to add category.nori sub-field: %s", e)

    def _ensure_created_time_field(self) -> None:
        """기존 인덱스에 created_time 관련 필드가 없으면 추가"""
        try:
            mapping = self.es.indices.get_mapping(index=self.index_name)
            properties = mapping[self.index_name]["mappings"].get("properties", {})
            missing_properties = {}
            if "created_time" not in properties:
                missing_properties["created_time"] = {"type": "date"}
            if "created_time_source" not in properties:
                missing_properties["created_time_source"] = {"type": "keyword"}
            if not missing_properties:
                return
            LOGGER.info("Adding created_time fields to index %s: %s", self.index_name, sorted(missing_properties))
            self.es.indices.put_mapping(index=self.index_name, properties=missing_properties)
            LOGGER.info("created_time fields added successfully")
        except Exception as e:
            LOGGER.warning("Failed to add created_time fields: %s", e)

    def _ensure_publisher_field(self) -> None:
        """기존 인덱스에 publisher 필드가 없으면 추가"""
        try:
            mapping = self.es.indices.get_mapping(index=self.index_name)
            properties = mapping[self.index_name]["mappings"].get("properties", {})
            if "publisher" in properties:
                return
            LOGGER.info("Adding publisher field to index %s", self.index_name)
            self.es.indices.put_mapping(index=self.index_name, properties={"publisher": {"type": "text", "analyzer": "nori_analyzer", "fields": {"keyword": {"type": "keyword"}}}})
            LOGGER.info("publisher field added successfully")
        except Exception as e:
            LOGGER.warning("Failed to add publisher field: %s", e)

    def delete_index(self) -> None:
        LOGGER.debug("delete_index()")
        if self.do_exist_index():
            self.es.indices.delete(index=self.index_name)

    def _search(self, query: dict[str, Any], sort: list[str] | str | None = None, max_result_count: int = -1, source_fields: list[str] | None = None) -> list[tuple[int, dict[str, Any], float]]:
        if max_result_count < 0:
            max_result_count = self.DEFAULT_MAX_RESULT_COUNT

        # Clamp the size to prevent ES 'max_result_window' error (default 10000)
        size = min(max_result_count, 10000)

        LOGGER.debug("_search(max_result_count=%d, size=%d, query='%s')", max_result_count, size, query)

        # 10,000개 이하면 scroll 없이 단순 검색 (scroll 컨텍스트 오버헤드 회피)
        if max_result_count <= 10000:
            response = self.es.search(index=self.index_name, query=query, sort=sort, size=size, track_scores=True, source=source_fields)
            max_score = response["hits"].get("max_score")
            if max_score is None:
                return []
            result = []
            for hit in response["hits"]["hits"]:
                normalized_score = hit["_score"] * 100 / max_score if max_score > 0 else 0
                result.append((int(hit["_id"]), hit["_source"], normalized_score))
            return result[:max_result_count]

        # 10,000개 초과일 때만 scroll 사용
        result_count = 0
        result = []
        scroll_id = None
        try:
            response = self.es.search(index=self.index_name, query=query, sort=sort, scroll="10m", track_scores=True, size=size)
            scroll_id = response.get("_scroll_id")
            max_score = response["hits"].get("max_score")
            if max_score is None:
                return []

            for hit in response["hits"]["hits"]:
                normalized_score = hit["_score"] * 100 / max_score if max_score > 0 else 0
                result.append((int(hit["_id"]), hit["_source"], normalized_score))
                result_count += 1
                if result_count >= max_result_count:
                    return result[:max_result_count]

            while len(response["hits"]["hits"]) > 0:
                response = self.es.scroll(scroll_id=scroll_id, scroll="10m")
                scroll_id = response["_scroll_id"]
                max_score = response["hits"].get("max_score")
                if max_score is None:
                    return []
                for hit in response["hits"]["hits"]:
                    normalized_score = hit["_score"] * 100 / max_score if max_score > 0 else 0
                    result.append((int(hit["_id"]), hit["_source"], normalized_score))
                    result_count += 1
                    if result_count >= max_result_count:
                        return result[:max_result_count]

            return result[:max_result_count]
        finally:
            if scroll_id:
                try:
                    self.es.clear_scroll(scroll_id=scroll_id)
                except Exception as e:
                    LOGGER.debug("Failed to clear scroll: %s", e)

    def _search_paged(self, query: dict[str, Any], sort: list[str] | str | None = None, size: int = 10, offset: int = 0, ref_score: float = 0.0, source_fields: list[str] | None = None) -> tuple[list[tuple[int, dict[str, Any], float]], int]:
        """(results, total_count) 튜플을 반환하는 페이지네이션 검색
        ref_score: 정규화 기준 점수. 0이면 결과 내 max_score를 사용."""
        size = min(size, 10000)
        LOGGER.debug("_search_paged(size=%d, offset=%d, query='%s')", size, offset, query)
        response = self.es.search(index=self.index_name, query=query, sort=sort, from_=offset, size=size, track_scores=True, track_total_hits=True, source=source_fields)
        total = response["hits"]["total"]["value"]
        base_score = ref_score if ref_score > 0 else (response["hits"]["max_score"] or 0)
        if base_score <= 0:
            return [], total
        result = []
        for hit in response["hits"]["hits"]:
            normalized_score = min(100.0, hit["_score"] * 100 / base_score) if base_score > 0 else 0
            result.append((int(hit["_id"]), hit["_source"], normalized_score))
        return result, total

    def search_by_title(self, title: str, file_type: str = "", file_size: int = 0, max_result_count: int = -1) -> list[tuple[int, dict[str, Any], float]]:
        if max_result_count < 0:
            max_result_count = self.DEFAULT_MAX_RESULT_COUNT
        LOGGER.debug("search_by_title(max_result_count=%d, title='%s', file_type='%s', file_size=%d)", max_result_count, title, file_type, file_size)
        query = {"bool": {"should": [{"match": {"title": {"query": title, "boost": 1.2 + math.log2(len(title.split(" ")))}}}, {"match": {"file_type": {"query": file_type, "boost": 1}}}, {"match": {"file_size": {"query": file_size, "boost": 1}}}]}}
        return self._search(query, max_result_count=max_result_count, source_fields=self.LIST_SOURCE_FIELDS)

    def search_by_category(self, category: str, max_result_count: int = -1) -> list[tuple[int, dict[str, Any], float]]:
        if max_result_count < 0:
            max_result_count = self.DEFAULT_MAX_RESULT_COUNT
        LOGGER.debug("search_by_category(category='%s')", category)
        query = {"term": {"category": category}}
        sort = ["author.keyword", "title.keyword"]

        return self._search(query, sort=sort, max_result_count=max_result_count, source_fields=self.LIST_SOURCE_FIELDS)

    # 카테고리 목록의 전순서 정렬. 화면 표시 순서(제목)와 일치시켜야 페이지를
    # 이어붙여도 순서가 어긋나지 않는다. file_path를 tie-breaker로 써서 커서가 한
    # 문서를 가리키게 한다.
    #
    # _id를 tie-breaker로 덧붙이면 안 된다. 이 클러스터는 indices.id_field_data가
    # 꺼져 있어 "Fielddata access on the _id field is disallowed"로 질의가 통째로
    # 실패하고, 상세 조회와 카테고리 책 목록이 빈 결과가 된다.
    CATEGORY_SORT: list[dict[str, str]] = [{"title.keyword": "asc"}, {"file_path": "asc"}]

    def search_by_category_paged(self, category: str, size: int = 500, search_after: list[Any] | None = None) -> tuple[list[tuple[int, dict[str, Any], float]], int, list[Any] | None]:
        """카테고리 내 문서를 search_after 커서로 한 페이지 조회한다.

        from/size 페이징은 max_result_window(기본 10000) 때문에 깊은 페이지에서
        실패하지만, search_after는 깊이 제한이 없어 카테고리 전체를 순회할 수 있다.
        반환: (문서 목록, 전체 건수, 다음 커서 | None)
        """
        LOGGER.debug("search_by_category_paged(category='%s', size=%d, search_after=%s)", category, size, search_after)
        kwargs: dict[str, Any] = {"index": self.index_name, "query": {"term": {"category": category}}, "sort": self.CATEGORY_SORT, "size": size, "track_total_hits": True, "source": self.LIST_SOURCE_FIELDS}
        if search_after:
            kwargs["search_after"] = search_after
        try:
            response = self.es.search(**kwargs)
        except Exception as e:
            LOGGER.error("search_by_category_paged error: %s", e)
            return [], 0, None
        hits = response["hits"]["hits"]
        total = response["hits"]["total"]["value"]
        result = [(int(hit["_id"]), hit["_source"], 0.0) for hit in hits]
        # 마지막 페이지(요청 크기 미만)면 다음 커서를 주지 않는다.
        next_search_after = hits[-1]["sort"] if len(hits) == size else None
        return result, total, next_search_after

    def search_by_keyword(self, keyword: str, max_result_count: int = -1) -> list[tuple[int, dict[str, Any], float]]:
        if max_result_count < 0:
            max_result_count = self.DEFAULT_MAX_RESULT_COUNT
        LOGGER.debug("search_by_keyword(keyword='%s', max_result_count=%d)", keyword, max_result_count)
        query = {"bool": {"should": [{"match": {"title": {"query": keyword, "boost": 10}}}, {"match": {"author": {"query": keyword, "boost": 5}}}, {"match": {"category.nori": {"query": keyword, "boost": 3}}}, {"match": {"summary": {"query": keyword, "boost": 1}}}], "minimum_should_match": 1}}
        return self._search(query, max_result_count=max_result_count, source_fields=self.LIST_SOURCE_FIELDS)

    def search_by_keyword_paged(self, keyword: str, size: int = 10, offset: int = 0, exclude_categories: list[str] | None = None) -> tuple[list[tuple[int, dict[str, Any], float]], int]:
        LOGGER.debug("search_by_keyword_paged(keyword='%s', size=%d, offset=%d, exclude_categories=%s)", keyword, size, offset, exclude_categories)
        query: dict[str, Any] = {
            "bool": {"must": [{"bool": {"should": [{"match": {"title": {"query": keyword, "boost": 10}}}, {"match": {"author": {"query": keyword, "boost": 5}}}, {"match": {"category.nori": {"query": keyword, "boost": 3}}}, {"match": {"summary": {"query": keyword, "boost": 1}}}], "minimum_should_match": 1}}]}
        }
        if exclude_categories:
            query["bool"]["must_not"] = [{"prefix": {"category": cat}} for cat in exclude_categories]
        return self._search_paged(query, size=size, offset=offset, source_fields=self.LIST_SOURCE_FIELDS)

    LATEST_SORT: list[dict[str, Any]] = [{"created_time": {"order": "desc", "missing": "_last"}}, {"updated_time": {"order": "desc", "missing": "_last"}}, {"file_path": {"order": "asc"}}]

    @staticmethod
    def _exclude_category_queries(categories: list[str] | None) -> list[dict[str, Any]]:
        if not categories:
            return []
        result = []
        for category in categories:
            if not category:
                continue
            result.append({"bool": {"should": [{"term": {"category": category}}, {"prefix": {"category": category + "/"}}], "minimum_should_match": 1}})
        return result

    def search_latest_docs(self, max_result_count: int = 100, exclude_categories: list[str] | None = None) -> tuple[list[tuple[int, dict[str, Any], float]], int]:
        LOGGER.debug("search_latest_docs(max_result_count=%d, exclude_categories=%s)", max_result_count, exclude_categories)
        size = max(1, min(max_result_count, 10000))
        query: dict[str, Any] = {"match_all": {}}
        excluded = self._exclude_category_queries(exclude_categories)
        if excluded:
            query = {"bool": {"must": [{"match_all": {}}], "must_not": excluded}}
        try:
            response = self.es.search(index=self.index_name, query=query, sort=self.LATEST_SORT, size=size, track_total_hits=True, source=self.LIST_SOURCE_FIELDS)
        except Exception as e:
            LOGGER.error("search_latest_docs error: %s", e)
            return [], 0
        hits = response["hits"]["hits"]
        total = response["hits"]["total"]["value"]
        return [(int(hit["_id"]), hit["_source"], 0.0) for hit in hits], total

    def backfill_created_time(self, path_prefix: Path, batch_size: int = 500) -> dict[str, int]:
        """created_time이 없는 기존 ES 문서를 실제 파일 stat 기준으로 채운다."""
        LOGGER.info("backfill_created_time(index=%s) start", self.index_name)
        result = {"updated": 0, "skipped": 0, "failed": 0}
        query = {"bool": {"should": [{"bool": {"must_not": [{"exists": {"field": "created_time"}}]}}, {"bool": {"must_not": [{"exists": {"field": "created_time_source"}}]}}], "minimum_should_match": 1}}
        scroll_id = None
        resolved_prefix = path_prefix.resolve(strict=False)

        def resolve_path(stored_path: str) -> Path | None:
            if not stored_path:
                return None
            candidate = Path(stored_path)
            full_path = candidate if candidate.is_absolute() else path_prefix / stored_path
            try:
                resolved = full_path.resolve(strict=False)
                if not resolved.is_relative_to(resolved_prefix):
                    return None
            except OSError:
                return None
            return resolved

        def update_hits(hits: list[dict[str, Any]]) -> None:
            bulk_body: list[dict[str, Any]] = []
            pending = 0
            for hit in hits:
                source = hit.get("_source", {})
                file_path = resolve_path(str(source.get("file_path", "")))
                if file_path is None or not file_path.is_file():
                    result["skipped"] += 1
                    continue
                try:
                    created_dt, created_time_source = path_created_time_with_source(file_path)
                    created_time = created_dt.isoformat()
                except OSError:
                    result["failed"] += 1
                    continue
                bulk_body.append({"update": {"_index": self.index_name, "_id": hit["_id"]}})
                bulk_body.append({"doc": {"created_time": created_time, "created_time_source": created_time_source}})
                pending += 1
            if not bulk_body:
                return
            try:
                bulk_result = self.es.bulk(body=bulk_body, timeout="60s", refresh=False)
            except Exception as e:
                LOGGER.error("backfill_created_time bulk error: %s", e)
                result["failed"] += pending
                return
            if bulk_result.get("errors"):
                failed = 0
                for item in bulk_result.get("items", []):
                    if item.get("update", {}).get("error"):
                        failed += 1
                result["failed"] += failed
                result["updated"] += pending - failed
            else:
                result["updated"] += pending

        try:
            response = self.es.search(index=self.index_name, query=query, scroll="10m", size=batch_size, source=["file_path"])
            scroll_id = response.get("_scroll_id")
            hits = response["hits"]["hits"]
            while hits:
                update_hits(hits)
                response = self.es.scroll(scroll_id=scroll_id, scroll="10m")
                scroll_id = response.get("_scroll_id")
                hits = response["hits"]["hits"]
        except Exception as e:
            LOGGER.error("backfill_created_time error: %s", e)
            result["failed"] += 1
        finally:
            if scroll_id:
                try:
                    self.es.clear_scroll(scroll_id=scroll_id)
                except Exception as e:
                    LOGGER.debug("Failed to clear created_time backfill scroll: %s", e)
        if result["updated"] > 0:
            self.refresh()
        LOGGER.info("backfill_created_time(index=%s) done: %s", self.index_name, result)
        return result

    # summary 원문(최대 3,500자)을 match 쿼리에 그대로 넣으면 nori 가 토큰 1,073개로
    # 쪼개고, 그 OR 쿼리가 417,776건 중 324,049건(78%)을 스코어링해 한 번에 3.5초가
    # 걸렸다. more_like_this 는 TF-IDF 상위 max_query_terms 개만 남겨 유사도 의미를
    # 유지하면서 249ms 로 줄인다 (2026-09-21 실측, 3502ms → 249ms).
    #
    # max_query_terms 를 50 으로 올리면 424ms 로 다시 느려진다. min_doc_freq 3 은
    # 한두 문서에만 나오는 오탈자·OCR 노이즈를 유사도 근거에서 뺀다.
    SIMILAR_SUMMARY_MAX_QUERY_TERMS = 25
    SIMILAR_SUMMARY_MIN_DOC_FREQ = 3

    @classmethod
    def _similar_should_clauses(cls, title: str, author: str, file_size: int, summary: str) -> list[dict[str, Any]]:
        """유사 문서 검색의 should 절. search_similar_docs 와 _paged 가 공유한다."""
        return [
            {"match": {"title": {"query": title, "boost": 20}}},
            {"match": {"author": {"query": author, "boost": 15}}},
            {"more_like_this": {"fields": ["summary"], "like": summary, "max_query_terms": cls.SIMILAR_SUMMARY_MAX_QUERY_TERMS, "min_term_freq": 1, "min_doc_freq": cls.SIMILAR_SUMMARY_MIN_DOC_FREQ, "boost": 3}},
            {"range": {"file_size": {"gte": file_size * 0.9, "lte": file_size * 1.1, "boost": 1}}},
        ]

    def search_similar_docs(self, category: str = "", title: str = "", author: str = "", file_type: str = "", file_size: int = 0, summary: str = "", max_result_count: int = -1, exclude_id: int | None = None) -> list[tuple[int, dict[str, Any], float]]:
        if max_result_count < 0:
            max_result_count = self.DEFAULT_MAX_RESULT_COUNT
        LOGGER.debug("search_similar_docs(category='%s', title='%s', author='%s', type='%s', size=%d, summary='%s', max_result_count=%d)", category, title, author, file_type, file_size, summary, max_result_count)
        query = {"bool": {"should": self._similar_should_clauses(title, author, file_size, summary), "minimum_should_match": 1}}
        if exclude_id is not None:
            query["bool"]["must_not"] = [{"term": {"_id": str(exclude_id)}}]
        return self._search(query, max_result_count=max_result_count, source_fields=self.LIST_SOURCE_FIELDS)

    def search_similar_docs_paged(self, category: str = "", title: str = "", author: str = "", file_type: str = "", file_size: int = 0, summary: str = "", exclude_id: int | None = None, size: int = 10, offset: int = 0) -> tuple[list[tuple[int, dict[str, Any], float]], int]:
        LOGGER.debug("search_similar_docs_paged(category='%s', title='%s', author='%s', type='%s', size=%d, offset=%d)", category, title, author, file_type, size, offset)
        should_clauses = self._similar_should_clauses(title, author, file_size, summary)

        # exclude_id가 있으면 msearch로 self-score + 본 검색을 1 roundtrip으로 실행
        if exclude_id is not None:
            return self._search_paged_with_self_score(should_clauses, exclude_id, size=size, offset=offset)

        # exclude_id가 없으면 기존 경로
        query = {"bool": {"should": should_clauses, "minimum_should_match": 1}}
        return self._search_paged(query, size=size, offset=offset)

    def _search_paged_with_self_score(self, should_clauses: list, exclude_id: int, size: int = 10, offset: int = 0) -> tuple[list[tuple[int, dict[str, Any], float]], int]:
        """msearch로 self-score 쿼리와 본 검색을 한 번의 왕복으로 실행"""
        LOGGER.debug("_search_paged_with_self_score(exclude_id=%s, size=%d, offset=%d)", exclude_id, size, offset)
        size = min(size, 10000)

        # self-score 쿼리: 원본 문서만 매칭
        self_score_query = {"bool": {"should": should_clauses, "filter": [{"term": {"_id": str(exclude_id)}}]}}
        # 본 검색 쿼리: 원본 제외
        search_query = {"bool": {"should": should_clauses, "minimum_should_match": 1, "must_not": [{"term": {"_id": str(exclude_id)}}]}}

        # self-score 쿼리는 점수만 쓰고 본문을 버리므로 _source 를 통째로 끈다.
        searches: list[dict[str, Any]] = [{"index": self.index_name}, {"size": 1, "query": self_score_query, "_source": False}, {"index": self.index_name}, {"size": size, "from": offset, "query": search_query, "track_scores": True, "track_total_hits": True, "_source": self.LIST_SOURCE_FIELDS}]
        response = self.es.msearch(searches=searches)
        responses = response["responses"]

        # self-score 추출
        if "error" in responses[0]:
            LOGGER.error("msearch self-score query error: %s", responses[0]["error"])
            base_score = 0.0
        else:
            self_hits = responses[0]["hits"]["hits"]
            base_score = self_hits[0]["_score"] if self_hits else 0.0

        # 본 검색 결과
        if "error" in responses[1]:
            LOGGER.error("msearch search query error: %s", responses[1]["error"])
            return [], 0

        search_resp = responses[1]
        total = search_resp["hits"]["total"]["value"]
        if base_score <= 0:
            return [], total

        result = []
        for hit in search_resp["hits"]["hits"]:
            normalized_score = min(100.0, hit["_score"] * 100 / base_score)
            result.append((int(hit["_id"]), hit["_source"], normalized_score))
        return result, total

    def search_by_id(self, doc_id: int) -> dict[str, Any]:
        LOGGER.debug("search_by_id(doc_id=%d)", doc_id)
        from elasticsearch import NotFoundError

        try:
            response = self.es.get(index=self.index_name, id=str(doc_id))
            return response["_source"]
        except NotFoundError:
            return {}

    def search_and_aggregate_by_category(self) -> dict[str, int]:
        LOGGER.debug("search_and_aggregate_by_category()")
        field_name = "category"
        size = 10000
        body = {"size": 1, "aggs": {"unique_values": {"terms": {"field": field_name, "size": size}}}}
        result = self.es.search(index=self.index_name, body=body)
        return {bucket["key"]: bucket["doc_count"] for bucket in result["aggregations"]["unique_values"]["buckets"]}

    def iter_all_category_file_paths(self, batch_size: int = 10000) -> Iterator[tuple[str, str, int]]:
        """전체 문서를 (category, file_path, 문서 수)로 훑는다.

        검색이 아니라 열거다. 질의도 순위도 필요 없고 모든 문서의 경로만 있으면 된다.
        scroll은 히트 41만 개를 한 건씩 만들어 내보내느라 필드를 하나도 안 받아도 13초가
        걸린다. composite 집계는 doc_values를 그대로 접어 같은 일을 3초에 끝낸다.

        버킷은 (category, file_path) 조합별로 하나씩 나오고 doc_count가 그 조합의 문서
        수다. 즉 같은 경로를 가리키는 중복 문서도 이 값으로 그대로 센다. 버킷은 source
        순서대로 정렬돼 나오므로 카테고리별로 뭉쳐서 도착한다.
        """
        LOGGER.debug("iter_all_category_file_paths(batch_size=%d)", batch_size)
        after: dict[str, Any] | None = None
        while True:
            composite: dict[str, Any] = {
                "size": batch_size,
                # missing_bucket을 켜야 category나 file_path가 빠진 문서가 조용히 사라지지 않는다.
                "sources": [{"category": {"terms": {"field": "category", "missing_bucket": True}}}, {"file_path": {"terms": {"field": "file_path", "missing_bucket": True}}}],
            }
            if after:
                composite["after"] = after
            response = self.es.search(index=self.index_name, size=0, aggs={"paths": {"composite": composite}})
            aggregation = response["aggregations"]["paths"]
            buckets = aggregation["buckets"]
            if not buckets:
                return
            for bucket in buckets:
                key = bucket["key"]
                yield key.get("category") or "", key.get("file_path") or "", bucket["doc_count"]
            after = aggregation.get("after_key")
            if not after:
                return

    def delete_by_file_paths(self, file_paths: list[str], exclude_ids: list[int] | None = None) -> int:
        """주어진 file_path 목록에 해당하는 기존 문서를 삭제 (중복 방지용).
        exclude_ids가 지정되면 해당 ID는 삭제하지 않음.
        반환: 삭제된 문서 수"""
        if not file_paths:
            return 0
        must_clauses: list[dict[str, Any]] = [{"terms": {"file_path": file_paths}}]
        must_not_clauses: list[dict[str, Any]] = []
        if exclude_ids:
            must_not_clauses.append({"ids": {"values": [str(i) for i in exclude_ids]}})
        query: dict[str, Any] = {"bool": {"must": must_clauses}}
        if must_not_clauses:
            query["bool"]["must_not"] = must_not_clauses
        try:
            result = self.es.delete_by_query(index=self.index_name, body={"query": query}, conflicts="proceed", refresh=False)
            deleted = result.get("deleted", 0)
            if deleted > 0:
                LOGGER.info("delete_by_file_paths: %d docs deleted for %d paths", deleted, len(file_paths))
            return deleted
        except Exception as e:
            LOGGER.error("delete_by_file_paths error: %s", e)
            return 0

    @staticmethod
    def _path_prefix_query(path_prefix: str) -> dict[str, Any]:
        """경로 prefix 하위(자기 자신 포함) 문서를 매칭하는 쿼리.
        path_prefix가 ""/"."/"/"이면 전체(match_all)."""
        p = (path_prefix or "").strip("/")
        if p in ("", "."):
            return {"match_all": {}}
        return {"bool": {"should": [{"term": {"file_path": p}}, {"prefix": {"file_path": p + "/"}}], "minimum_should_match": 1}}

    def get_doc_ids_by_path_prefix(self, path_prefix: str) -> set[int]:
        """경로 prefix 하위 문서의 _id(inode) 집합을 반환 (scroll 사용, _source 미포함)."""
        query = self._path_prefix_query(path_prefix)
        ids: set[int] = set()
        scroll_id = None
        try:
            response = self.es.search(index=self.index_name, query=query, scroll="10m", size=5000, source=False)
            scroll_id = response.get("_scroll_id")
            hits = response["hits"]["hits"]
            while hits:
                for hit in hits:
                    ids.add(int(hit["_id"]))
                response = self.es.scroll(scroll_id=scroll_id, scroll="10m")
                scroll_id = response.get("_scroll_id")
                hits = response["hits"]["hits"]
            return ids
        except Exception as e:
            LOGGER.error("get_doc_ids_by_path_prefix error: %s", e)
            return ids
        finally:
            if scroll_id:
                try:
                    self.es.clear_scroll(scroll_id=scroll_id)
                except Exception:
                    pass

    def delete_by_ids(self, ids: list[int], chunk_size: int = 10000) -> int:
        """주어진 _id(inode) 목록의 문서를 삭제. 반환: 삭제된 문서 수."""
        if not ids:
            return 0
        deleted = 0
        try:
            for i in range(0, len(ids), chunk_size):
                chunk = ids[i : i + chunk_size]
                query = {"ids": {"values": [str(x) for x in chunk]}}
                result = self.es.delete_by_query(index=self.index_name, body={"query": query}, conflicts="proceed", refresh=False)
                deleted += result.get("deleted", 0)
            if deleted > 0:
                LOGGER.info("delete_by_ids: %d docs deleted for %d ids", deleted, len(ids))
            return deleted
        except Exception as e:
            LOGGER.error("delete_by_ids error: %s", e)
            return deleted

    def insert(self, data: dict[int, dict[str, Any]], num_docs: int = sys.maxsize, max_retries: int = 3) -> list[int]:
        LOGGER.debug("insert() %d items", len(data))
        es_data: list[dict[str, Any]] = []
        data_count = 0
        chunk_size = 100
        iter_items = iter(data.items())
        doc_id_list: list[int] = []
        while True:
            chunk = list(islice(iter_items, chunk_size))
            if not chunk:
                break
            for inode_num, path_and_size in chunk:
                es_data.append({"index": {"_index": self.index_name, "_id": str(inode_num)}})
                es_data.append(path_and_size)
                doc_id_list.append(inode_num)
                data_count += 1
            LOGGER.info("%d items inserted", int(len(es_data) / 2))

            # 재시도 로직
            for attempt in range(max_retries):
                try:
                    self.es.bulk(body=es_data, timeout="60s", refresh=False)
                    break
                except (SerializationError, ConnectionError, ConnectionTimeout) as e:
                    if attempt < max_retries - 1:
                        wait_time = 2**attempt  # 지수 백오프: 1, 2, 4초
                        LOGGER.warning(f"ES bulk 요청 실패 (시도 {attempt + 1}/{max_retries}): {e}. {wait_time}초 후 재시도...")
                        time.sleep(wait_time)
                    else:
                        LOGGER.error(f"ES bulk 요청 최종 실패: {e}")
                        raise

            es_data = []
            if data_count >= num_docs:
                break
        return doc_id_list

    def refresh(self) -> None:
        """인덱스를 refresh하여 최근 변경사항을 검색 가능하게 함"""
        LOGGER.debug("refresh()")
        self.es.indices.refresh(index=self.index_name)

    def update(self, doc_id: int, category: str = "", title: str = "", author: str = "", file_path: str = "", file_type: str = "", file_size: int = 0, summary: str = "") -> bool:
        LOGGER.debug("update(doc_id=%d, title='%s', author='%s', file_path='%r', file_type='%s', file_size=%d, summary='%s', category='%s')", doc_id, title, author, file_path, file_type, file_size, summary, category)
        doc: dict[str, Any] = {}
        if category:
            doc.update({"category": category})
        if title:
            doc.update({"title": title})
        if author:
            doc.update({"author": author})
        if file_path:
            doc.update({"file_path": str(file_path)})
        if file_type:
            doc.update({"file_type": file_type})
        if file_size:
            doc.update({"file_size": file_size})
        if summary:
            doc.update({"summary": summary})
        body = {"doc": doc}
        result = self.es.update(index=self.index_name, id=str(doc_id), body=body, refresh=True)
        if "_shards" in result:
            if "failed" in result["_shards"]:
                if result["_shards"]["failed"] > 0:
                    return False
        return True

    def count_by_category(self, category: str, prefix: bool = False) -> int:
        """특정 카테고리의 문서 수를 반환

        Args:
            category: 카테고리명
            prefix: True이면 하위 카테고리(category/*)도 포함하여 카운트
        """
        LOGGER.debug("count_by_category(category='%s', prefix=%s)", category, prefix)
        if prefix:
            query = {"bool": {"should": [{"term": {"category": category}}, {"prefix": {"category": category + "/"}}], "minimum_should_match": 1}}
        else:
            query = {"term": {"category": category}}
        result = self.es.count(index=self.index_name, query=query)
        return result["count"]

    def count_by_categories(self, categories: list[str], prefix: bool = False) -> dict[str, int]:
        """여러 카테고리의 문서 수를 msearch로 한 번에 조회

        prefix=True이면 하위 카테고리(category/*) 문서도 함께 센다.
        """
        if len(categories) == 1:
            return {categories[0]: self.count_by_category(categories[0], prefix=prefix)}
        LOGGER.debug("count_by_categories(categories=%s, prefix=%s)", categories, prefix)
        searches: list[dict[str, Any]] = []
        for cat in categories:
            if prefix:
                query: dict[str, Any] = {"bool": {"should": [{"term": {"category": cat}}, {"prefix": {"category": cat + "/"}}], "minimum_should_match": 1}}
            else:
                query = {"term": {"category": cat}}
            searches.append({"index": self.index_name})
            searches.append({"size": 0, "track_total_hits": True, "query": query})
        response = self.es.msearch(searches=searches)
        result: dict[str, int] = {}
        for cat, resp in zip(categories, response["responses"]):
            if "error" in resp:
                LOGGER.error("msearch error for category '%s': %s", cat, resp["error"])
                result[cat] = 0
            else:
                result[cat] = resp["hits"]["total"]["value"]
        return result

    def rename_category(self, old_category: str, new_category: str) -> dict[str, Any]:
        """ES에서 특정 카테고리의 모든 문서를 새 카테고리로 변경

        category 필드와 file_path의 카테고리 prefix를 갱신한다.

        Returns:
            {"updated": int, "failures": list}
        """
        LOGGER.debug("rename_category(old='%s', new='%s')", old_category, new_category)
        old_prefix = old_category + "/"
        new_prefix = new_category + "/"
        # 디렉토리 rename은 하위 카테고리까지 통째로 옮기므로, ES도 'A'와 'A/*'를 함께 갱신해야
        # 한다. term만 쓰면 하위 카테고리 문서가 예전 경로로 남아 FS와 조용히 어긋난다.
        script = {
            "source": """
                if (ctx._source.category == params.old_category) {
                    ctx._source.category = params.new_category;
                } else if (ctx._source.category.startsWith(params.old_prefix)) {
                    ctx._source.category = params.new_prefix + ctx._source.category.substring(params.old_prefix.length());
                }
                if (ctx._source.file_path != null && ctx._source.file_path.startsWith(params.old_prefix)) {
                    ctx._source.file_path = params.new_prefix + ctx._source.file_path.substring(params.old_prefix.length());
                }
            """,
            "lang": "painless",
            "params": {"old_category": old_category, "new_category": new_category, "old_prefix": old_prefix, "new_prefix": new_prefix},
        }
        query = {"bool": {"should": [{"term": {"category": old_category}}, {"prefix": {"category": old_prefix}}], "minimum_should_match": 1}}
        result = self.es.update_by_query(index=self.index_name, query=query, script=script, conflicts="abort", refresh=True)
        return {"updated": result.get("updated", 0), "failures": result.get("failures", [])}

    def delete_by_category(self, category: str, prefix: bool = False) -> dict[str, Any]:
        """특정 카테고리의 모든 문서를 삭제

        Args:
            category: 카테고리명
            prefix: True이면 하위 카테고리(category/*)도 포함하여 삭제

        Returns:
            {"deleted": int, "failures": list}
        """
        LOGGER.debug("delete_by_category(category='%s', prefix=%s)", category, prefix)
        if prefix:
            query = {"bool": {"should": [{"term": {"category": category}}, {"prefix": {"category": category + "/"}}], "minimum_should_match": 1}}
        else:
            query = {"term": {"category": category}}
        result = self.es.delete_by_query(index=self.index_name, query=query, conflicts="abort", refresh=True)
        return {"deleted": result.get("deleted", 0), "failures": result.get("failures", [])}

    def delete(self, doc_id: int) -> bool:
        LOGGER.debug("delete(doc_id=%d)", doc_id)
        result = self.es.delete(index=self.index_name, id=str(doc_id), refresh=True)
        if "result" in result:
            if result["result"] == "deleted":
                return True
        return False
