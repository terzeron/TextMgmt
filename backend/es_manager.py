#!/usr/bin/env python

import sys
import os
import math
import logging.config
from pathlib import Path
from typing import Dict, List, Any, Optional, Tuple, Union, Set
from itertools import islice
import time
from elasticsearch import Elasticsearch
from elastic_transport import SerializationError, ConnectionError, ConnectionTimeout


logging.config.fileConfig(Path(__file__).parent.parent / "logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger(__name__)
logging.getLogger("elasticsearch").setLevel(logging.CRITICAL)


class ESManager:
    DEFAULT_MAX_RESULT_COUNT = 10

    def __init__(self, index_name: str = "") -> None:
        for env in ["TM_ES_BOOK_INDEX", "TM_ES_URL", "TM_ES_USER", "TM_ES_PASSWORD"]:
            if env not in os.environ:
                LOGGER.error(f"The environment variable {env} is not set.")
                sys.exit(-1)

        self.index_name = index_name or os.environ["TM_ES_BOOK_INDEX"]
        url = os.environ["TM_ES_URL"]
        user = os.environ["TM_ES_USER"]
        password = os.environ["TM_ES_PASSWORD"]

        self.es = Elasticsearch(
            hosts=[url],
            basic_auth=(user, password),
            request_timeout=10,
            retry_on_timeout=True,
        )

        max_retries = 5
        for attempt in range(max_retries):
            try:
                self.es.info()
                LOGGER.info("Elasticsearch 연결 성공")
                break
            except (ConnectionError, ConnectionTimeout) as e:
                if attempt < max_retries - 1:
                    wait = min(2 ** (attempt + 1), 10)
                    LOGGER.warning(
                        "ES 연결 실패 (시도 %d/%d): %s. %d초 후 재시도...",
                        attempt + 1,
                        max_retries,
                        e,
                        wait,
                    )
                    time.sleep(wait)
                else:
                    LOGGER.error("ES 연결 최종 실패: %s", e)
                    raise

    def __del__(self) -> None:
        if hasattr(self, "es"):
            del self.es

    def do_exist_index(self) -> bool:
        LOGGER.debug("do_exist_index()")
        return self.es.indices.exists(index=self.index_name)

    def get_existing_ids(self, doc_ids: List[int]) -> Set[int]:
        """주어진 ID 목록 중 ES에 존재하는 ID들을 반환"""
        if not doc_ids:
            return set()
        LOGGER.debug("get_existing_ids(%d ids)", len(doc_ids))
        docs = [{"_index": self.index_name, "_id": str(doc_id)} for doc_id in doc_ids]
        response = self.es.mget(docs=docs, source=False)
        return {int(doc["_id"]) for doc in response["docs"] if doc.get("found", False)}

    def get_existing_paths(self, doc_ids: List[int]) -> Dict[int, str]:
        """주어진 ID 목록의 기존 file_path를 조회. 반환: {inode: file_path}"""
        if not doc_ids:
            return {}
        LOGGER.debug("get_existing_paths(%d ids)", len(doc_ids))
        docs = [{"_index": self.index_name, "_id": str(doc_id)} for doc_id in doc_ids]
        response = self.es.mget(docs=docs, source=["file_path"])
        result: Dict[int, str] = {}
        for doc in response["docs"]:
            if doc.get("found", False):
                result[int(doc["_id"])] = doc["_source"]["file_path"]
        return result

    def bulk_update_paths(self, updates: Dict[int, Dict[str, str]], max_retries: int = 3) -> int:
        """inode → {"file_path": ..., "category": ...} 맵을 받아 bulk partial update 수행.
        반환: 업데이트된 문서 수"""
        if not updates:
            return 0
        LOGGER.debug("bulk_update_paths(%d items)", len(updates))
        updated_count = 0
        iter_items = iter(updates.items())
        chunk_size = 100

        while True:
            chunk = list(islice(iter_items, chunk_size))
            if not chunk:
                break
            es_data: List[Dict[str, Any]] = []
            for inode, fields in chunk:
                es_data.append({"update": {"_index": self.index_name, "_id": str(inode)}})
                es_data.append({"doc": fields})

            for attempt in range(max_retries):
                try:
                    self.es.bulk(body=es_data, timeout="60s", refresh=False)
                    break
                except (SerializationError, ConnectionError, ConnectionTimeout) as e:
                    if attempt < max_retries - 1:
                        wait_time = 2**attempt
                        LOGGER.warning(f"ES bulk_update_paths 실패 (시도 {attempt + 1}/{max_retries}): {e}. {wait_time}초 후 재시도...")
                        time.sleep(wait_time)
                    else:
                        LOGGER.error(f"ES bulk_update_paths 최종 실패: {e}")
                        raise

            updated_count += len(chunk)
        return updated_count

    def create_index(self) -> dict[str, Any]:
        LOGGER.debug("create_index()")
        from elasticsearch import BadRequestError

        settings = {
            "index": {
                "similarity": {"default": {"type": "BM25"}},
            },
            "analysis": {
                "tokenizer": {
                    "nori_tokenizer": {
                        "type": "nori_tokenizer",
                        "decompound_mode": "discard",
                    }
                },
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
                "analyzer": {
                    "nori_analyzer": {
                        "type": "custom",
                        "tokenizer": "nori_tokenizer",
                        "filter": ["nori_posfilter", "lowercase"],
                    }
                },
            },
        }
        mappings = {
            "properties": {
                "category": {
                    "type": "keyword",
                    "fields": {"nori": {"type": "text", "analyzer": "nori_analyzer"}},
                },
                "title": {
                    "type": "text",
                    "analyzer": "nori_analyzer",
                    "fields": {"keyword": {"type": "keyword"}},
                },
                "author": {
                    "type": "text",
                    "analyzer": "nori_analyzer",
                    "fields": {"keyword": {"type": "keyword"}},
                },
                "file_path": {"type": "keyword"},
                "file_type": {"type": "keyword"},
                "file_size": {"type": "unsigned_long"},
                "line_count": {"type": "integer"},
                "page_count": {"type": "integer"},
                "isbn": {"type": "keyword"},
                "summary": {"type": "text", "analyzer": "nori_analyzer"},
                "updated_time": {
                    "type": "date",
                },
            }
        }

        if self.do_exist_index():
            self._ensure_category_nori_subfield()
            return {"acknowledged": True}
        try:
            return self.es.indices.create(index=self.index_name, body={"settings": settings, "mappings": mappings})
        except BadRequestError as e:
            if "resource_already_exists_exception" in str(e):
                LOGGER.info("Index %s already exists, skipping creation", self.index_name)
                self._ensure_category_nori_subfield()
                return {"acknowledged": True}
            raise

    def _ensure_category_nori_subfield(self) -> None:
        """기존 인덱스에 category.nori 서브필드가 없으면 추가"""
        try:
            mapping = self.es.indices.get_mapping(index=self.index_name)
            cat_props = mapping[self.index_name]["mappings"]["properties"].get("category", {})
            if "fields" in cat_props and "nori" in cat_props["fields"]:
                return
            LOGGER.info("Adding category.nori sub-field to index %s", self.index_name)
            self.es.indices.put_mapping(
                index=self.index_name,
                properties={
                    "category": {
                        "type": "keyword",
                        "fields": {"nori": {"type": "text", "analyzer": "nori_analyzer"}},
                    }
                },
            )
            LOGGER.info("category.nori sub-field added successfully")
        except Exception as e:
            LOGGER.warning("Failed to add category.nori sub-field: %s", e)

    def delete_index(self) -> None:
        LOGGER.debug("delete_index()")
        if self.do_exist_index():
            self.es.indices.delete(index=self.index_name)

    def get_mappings(self) -> dict[str, Any]:
        LOGGER.debug("get_mappings()")
        if self.do_exist_index():
            return self.es.indices.get_mapping(index=self.index_name)[self.index_name]["mappings"]
        else:
            return {}

    def _search(
        self,
        query: Dict[str, Any],
        sort: Union[List[str], str, None] = None,
        max_result_count: int = -1,
    ) -> List[Tuple[int, Dict[str, Any], float]]:
        if max_result_count < 0:
            max_result_count = self.DEFAULT_MAX_RESULT_COUNT

        # Clamp the size to prevent ES 'max_result_window' error (default 10000)
        size = min(max_result_count, 10000)

        LOGGER.debug(
            "_search(max_result_count=%d, size=%d, query='%s')",
            max_result_count,
            size,
            query,
        )

        # 10,000개 이하면 scroll 없이 단순 검색 (scroll 컨텍스트 오버헤드 회피)
        if max_result_count <= 10000:
            response = self.es.search(
                index=self.index_name,
                query=query,
                sort=sort,
                size=size,
                track_scores=True,
            )
            max_score = response["hits"]["max_score"]
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
            response = self.es.search(
                index=self.index_name,
                query=query,
                sort=sort,
                scroll="10m",
                track_scores=True,
                size=size,
            )
            scroll_id = response.get("_scroll_id")
            max_score = response["hits"]["max_score"]
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
                max_score = response["hits"]["max_score"]
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

    def _search_paged(
        self,
        query: Dict[str, Any],
        sort: Union[List[str], str, None] = None,
        size: int = 10,
        offset: int = 0,
        ref_score: float = 0.0,
    ) -> Tuple[List[Tuple[int, Dict[str, Any], float]], int]:
        """(results, total_count) 튜플을 반환하는 페이지네이션 검색
        ref_score: 정규화 기준 점수. 0이면 결과 내 max_score를 사용."""
        size = min(size, 10000)
        LOGGER.debug("_search_paged(size=%d, offset=%d, query='%s')", size, offset, query)
        response = self.es.search(
            index=self.index_name,
            query=query,
            sort=sort,
            from_=offset,
            size=size,
            track_scores=True,
            track_total_hits=True,
        )
        total = response["hits"]["total"]["value"]
        base_score = ref_score if ref_score > 0 else (response["hits"]["max_score"] or 0)
        if base_score <= 0:
            return [], total
        result = []
        for hit in response["hits"]["hits"]:
            normalized_score = min(100.0, hit["_score"] * 100 / base_score) if base_score > 0 else 0
            result.append((int(hit["_id"]), hit["_source"], normalized_score))
        return result, total

    def search_by_title(
        self,
        title: str,
        file_type: str = "",
        file_size: int = 0,
        max_result_count: int = -1,
    ) -> List[Tuple[int, Dict[str, Any], float]]:
        if max_result_count < 0:
            max_result_count = self.DEFAULT_MAX_RESULT_COUNT
        LOGGER.debug(
            "search_by_title(max_result_count=%d, title='%s', file_type='%s', file_size=%d)",
            max_result_count,
            title,
            file_type,
            file_size,
        )
        query = {
            "bool": {
                "should": [
                    {
                        "match": {
                            "title": {
                                "query": title,
                                "boost": 1.2 + math.log2(len(title.split(" "))),
                            }
                        }
                    },
                    {"match": {"file_type": {"query": file_type, "boost": 1}}},
                    {"match": {"file_size": {"query": file_size, "boost": 1}}},
                ]
            }
        }
        return self._search(query, max_result_count=max_result_count)

    def search_by_summary(self, summary: str, max_result_count: int = -1) -> List[Tuple[int, Dict[str, Any], float]]:
        if max_result_count < 0:
            max_result_count = self.DEFAULT_MAX_RESULT_COUNT
        LOGGER.debug(
            "search_by_summary(max_result_count=%d, summary='%s')",
            max_result_count,
            summary,
        )
        query = {"match": {"summary": summary}}
        return self._search(query, max_result_count=max_result_count)

    def search_by_category(self, category: str, max_result_count: int = -1) -> List[Tuple[int, Dict[str, Any], float]]:
        if max_result_count < 0:
            max_result_count = self.DEFAULT_MAX_RESULT_COUNT
        LOGGER.debug("search_by_category(category='%s')", category)
        query = {"term": {"category": category}}
        sort = ["author.keyword", "title.keyword"]

        return self._search(query, sort=sort, max_result_count=max_result_count)

    def search_by_keyword(self, keyword: str, max_result_count: int = -1) -> List[Tuple[int, Dict[str, Any], float]]:
        if max_result_count < 0:
            max_result_count = self.DEFAULT_MAX_RESULT_COUNT
        LOGGER.debug(
            "search_by_keyword(keyword='%s', max_result_count=%d)",
            keyword,
            max_result_count,
        )
        query = {
            "bool": {
                "should": [
                    {"match": {"title": {"query": keyword, "boost": 10}}},
                    {"match": {"author": {"query": keyword, "boost": 5}}},
                    {"match": {"category.nori": {"query": keyword, "boost": 3}}},
                    {"match": {"summary": {"query": keyword, "boost": 1}}},
                ],
                "minimum_should_match": 1,
            }
        }
        return self._search(query, max_result_count=max_result_count)

    def search_by_keyword_paged(
        self,
        keyword: str,
        size: int = 10,
        offset: int = 0,
        exclude_categories: List[str] = None,
    ) -> Tuple[List[Tuple[int, Dict[str, Any], float]], int]:
        LOGGER.debug(
            "search_by_keyword_paged(keyword='%s', size=%d, offset=%d, exclude_categories=%s)",
            keyword,
            size,
            offset,
            exclude_categories,
        )
        query: Dict[str, Any] = {
            "bool": {
                "must": [
                    {
                        "bool": {
                            "should": [
                                {"match": {"title": {"query": keyword, "boost": 10}}},
                                {"match": {"author": {"query": keyword, "boost": 5}}},
                                {"match": {"category.nori": {"query": keyword, "boost": 3}}},
                                {"match": {"summary": {"query": keyword, "boost": 1}}},
                            ],
                            "minimum_should_match": 1,
                        }
                    }
                ],
            }
        }
        if exclude_categories:
            query["bool"]["must_not"] = [{"prefix": {"category": cat}} for cat in exclude_categories]
        return self._search_paged(query, size=size, offset=offset)

    def search_similar_docs(
        self,
        category: str = "",
        title: str = "",
        author: str = "",
        file_type: str = "",
        file_size: int = 0,
        summary: str = "",
        max_result_count: int = -1,
        exclude_id: int = None,
    ) -> List[Tuple[int, Dict[str, Any], float]]:
        if max_result_count < 0:
            max_result_count = self.DEFAULT_MAX_RESULT_COUNT
        LOGGER.debug(
            "search_similar_docs(category='%s', title='%s', author='%s', type='%s', size=%d, summary='%s', max_result_count=%d)",
            category,
            title,
            author,
            file_type,
            file_size,
            summary,
            max_result_count,
        )
        query = {
            "bool": {
                "should": [
                    {"match": {"title": {"query": title, "boost": 20}}},
                    {"match": {"author": {"query": author, "boost": 15}}},
                    {"match": {"summary": {"query": summary, "boost": 3}}},
                    {
                        "range": {
                            "file_size": {
                                "gte": file_size * 0.9,
                                "lte": file_size * 1.1,
                                "boost": 1,
                            }
                        }
                    },
                ],
                "minimum_should_match": 1,
            }
        }
        if exclude_id is not None:
            query["bool"]["must_not"] = [{"term": {"_id": str(exclude_id)}}]
        return self._search(query, max_result_count=max_result_count)

    def search_similar_docs_paged(
        self,
        category: str = "",
        title: str = "",
        author: str = "",
        file_type: str = "",
        file_size: int = 0,
        summary: str = "",
        exclude_id: int = None,
        size: int = 10,
        offset: int = 0,
    ) -> Tuple[List[Tuple[int, Dict[str, Any], float]], int]:
        LOGGER.debug(
            "search_similar_docs_paged(category='%s', title='%s', author='%s', type='%s', size=%d, offset=%d)",
            category,
            title,
            author,
            file_type,
            size,
            offset,
        )
        should_clauses = [
            {"match": {"title": {"query": title, "boost": 20}}},
            {"match": {"author": {"query": author, "boost": 15}}},
            {"match": {"summary": {"query": summary, "boost": 3}}},
            {
                "range": {
                    "file_size": {
                        "gte": file_size * 0.9,
                        "lte": file_size * 1.1,
                        "boost": 1,
                    }
                }
            },
        ]

        # exclude_id가 있으면 msearch로 self-score + 본 검색을 1 roundtrip으로 실행
        if exclude_id is not None:
            return self._search_paged_with_self_score(should_clauses, exclude_id, size=size, offset=offset)

        # exclude_id가 없으면 기존 경로
        query = {
            "bool": {
                "should": should_clauses,
                "minimum_should_match": 1,
            }
        }
        return self._search_paged(query, size=size, offset=offset)

    def _search_paged_with_self_score(self, should_clauses: list, exclude_id: int, size: int = 10, offset: int = 0) -> Tuple[List[Tuple[int, Dict[str, Any], float]], int]:
        """msearch로 self-score 쿼리와 본 검색을 한 번의 왕복으로 실행"""
        LOGGER.debug(
            "_search_paged_with_self_score(exclude_id=%s, size=%d, offset=%d)",
            exclude_id,
            size,
            offset,
        )
        size = min(size, 10000)

        # self-score 쿼리: 원본 문서만 매칭
        self_score_query = {
            "bool": {
                "should": should_clauses,
                "filter": [{"term": {"_id": str(exclude_id)}}],
            }
        }
        # 본 검색 쿼리: 원본 제외
        search_query = {
            "bool": {
                "should": should_clauses,
                "minimum_should_match": 1,
                "must_not": [{"term": {"_id": str(exclude_id)}}],
            }
        }

        searches: List[Dict[str, Any]] = [
            {"index": self.index_name},
            {"size": 1, "query": self_score_query},
            {"index": self.index_name},
            {
                "size": size,
                "from": offset,
                "query": search_query,
                "track_scores": True,
                "track_total_hits": True,
            },
        ]
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

    def _get_self_score(self, should_clauses: list, doc_id: int) -> float:
        """원본 문서가 동일 쿼리에서 받는 점수를 반환 (정규화 기준값)"""
        if doc_id is None:
            return 0.0
        query = {
            "bool": {
                "should": should_clauses,
                "filter": [{"term": {"_id": str(doc_id)}}],
            }
        }
        response = self.es.search(index=self.index_name, query=query, size=1)
        hits = response["hits"]["hits"]
        if hits:
            return hits[0]["_score"]
        return 0.0

    def search_by_id(self, doc_id: int) -> Dict[str, Any]:
        LOGGER.debug("search_by_id(doc_id=%d)", doc_id)
        from elasticsearch import NotFoundError

        try:
            response = self.es.get(index=self.index_name, id=str(doc_id))
            return response["_source"]
        except NotFoundError:
            return {}

    def search_and_aggregate_by_category(self) -> Dict[str, int]:
        LOGGER.debug("search_and_aggregate_by_category()")
        field_name = "category"
        size = 10000
        body = {
            "size": 1,
            "aggs": {"unique_values": {"terms": {"field": field_name, "size": size}}},
        }
        result = self.es.search(index=self.index_name, body=body)
        return {bucket["key"]: bucket["doc_count"] for bucket in result["aggregations"]["unique_values"]["buckets"]}

    def get_all_file_paths_grouped(self) -> Dict[str, Set[str]]:
        """scroll로 전체 인덱스를 순회하여 카테고리별 file_path 집합을 반환"""
        LOGGER.debug("get_all_file_paths_grouped()")
        result: Dict[str, Set[str]] = {}
        scroll_id = None
        try:
            response = self.es.search(
                index=self.index_name,
                body={"query": {"match_all": {}}, "_source": ["category", "file_path"]},
                scroll="10m",
                size=10000,
            )
            scroll_id = response.get("_scroll_id")

            while True:
                hits = response["hits"]["hits"]
                if not hits:
                    break
                for hit in hits:
                    src = hit["_source"]
                    cat = src.get("category", "")
                    fp = src.get("file_path", "")
                    if cat:
                        result.setdefault(cat, set()).add(fp)
                response = self.es.scroll(scroll_id=scroll_id, scroll="10m")
                scroll_id = response["_scroll_id"]
        finally:
            if scroll_id:
                try:
                    self.es.clear_scroll(scroll_id=scroll_id)
                except Exception as e:
                    LOGGER.debug("Failed to clear scroll: %s", e)
        return result

    def delete_by_file_paths(self, file_paths: List[str], exclude_ids: Optional[List[int]] = None) -> int:
        """주어진 file_path 목록에 해당하는 기존 문서를 삭제 (중복 방지용).
        exclude_ids가 지정되면 해당 ID는 삭제하지 않음.
        반환: 삭제된 문서 수"""
        if not file_paths:
            return 0
        must_clauses: List[Dict[str, Any]] = [{"terms": {"file_path.keyword": file_paths}}]
        must_not_clauses: List[Dict[str, Any]] = []
        if exclude_ids:
            must_not_clauses.append({"ids": {"values": [str(i) for i in exclude_ids]}})
        query: Dict[str, Any] = {"bool": {"must": must_clauses}}
        if must_not_clauses:
            query["bool"]["must_not"] = must_not_clauses
        try:
            result = self.es.delete_by_query(
                index=self.index_name,
                body={"query": query},
                conflicts="proceed",
                refresh=False,
            )
            deleted = result.get("deleted", 0)
            if deleted > 0:
                LOGGER.info(
                    "delete_by_file_paths: %d docs deleted for %d paths",
                    deleted,
                    len(file_paths),
                )
            return deleted
        except Exception as e:
            LOGGER.error("delete_by_file_paths error: %s", e)
            return 0

    def insert(
        self,
        data: Dict[int, Dict[str, Any]],
        num_docs: int = sys.maxsize,
        max_retries: int = 3,
    ) -> List[int]:
        LOGGER.debug("insert() %d items", len(data))
        es_data: List[Dict[str, Any]] = []
        data_count = 0
        chunk_size = 100
        iter_items = iter(data.items())
        doc_id_list: List[int] = []
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

    def update(
        self,
        doc_id: int,
        category: str = "",
        title: str = "",
        author: str = "",
        file_path: str = "",
        file_type: str = "",
        file_size: int = 0,
        summary: str = "",
    ) -> bool:
        LOGGER.debug(
            "update(doc_id=%d, title='%s', author='%s', file_path='%r', file_type='%s', file_size=%d, summary='%s', category='%s')",
            doc_id,
            title,
            author,
            file_path,
            file_type,
            file_size,
            summary,
            category,
        )
        doc: Dict[str, Any] = {}
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
            query = {
                "bool": {
                    "should": [
                        {"term": {"category": category}},
                        {"prefix": {"category": category + "/"}},
                    ],
                    "minimum_should_match": 1,
                }
            }
        else:
            query = {"term": {"category": category}}
        result = self.es.count(index=self.index_name, query=query)
        return result["count"]

    def count_by_categories(self, categories: List[str]) -> Dict[str, int]:
        """여러 카테고리의 문서 수를 msearch로 한 번에 조회"""
        if len(categories) == 1:
            return {categories[0]: self.count_by_category(categories[0])}
        LOGGER.debug("count_by_categories(categories=%s)", categories)
        searches: List[Dict[str, Any]] = []
        for cat in categories:
            searches.append({"index": self.index_name})
            searches.append(
                {
                    "size": 0,
                    "track_total_hits": True,
                    "query": {"term": {"category": cat}},
                }
            )
        response = self.es.msearch(searches=searches)
        result: Dict[str, int] = {}
        for cat, resp in zip(categories, response["responses"]):
            if "error" in resp:
                LOGGER.error("msearch error for category '%s': %s", cat, resp["error"])
                result[cat] = 0
            else:
                result[cat] = resp["hits"]["total"]["value"]
        return result

    def rename_category(self, old_category: str, new_category: str) -> Dict[str, Any]:
        """ES에서 특정 카테고리의 모든 문서를 새 카테고리로 변경

        category 필드와 file_path의 카테고리 prefix를 갱신한다.

        Returns:
            {"updated": int, "failures": list}
        """
        LOGGER.debug("rename_category(old='%s', new='%s')", old_category, new_category)
        old_prefix = old_category + "/"
        new_prefix = new_category + "/"
        script = {
            "source": """
                ctx._source.category = params.new_category;
                if (ctx._source.file_path.startsWith(params.old_prefix)) {
                    ctx._source.file_path = params.new_prefix + ctx._source.file_path.substring(params.old_prefix.length());
                }
            """,
            "lang": "painless",
            "params": {
                "new_category": new_category,
                "old_prefix": old_prefix,
                "new_prefix": new_prefix,
            },
        }
        result = self.es.update_by_query(
            index=self.index_name,
            query={"term": {"category": old_category}},
            script=script,
            conflicts="abort",
            refresh=True,
        )
        return {
            "updated": result.get("updated", 0),
            "failures": result.get("failures", []),
        }

    def delete_by_category(self, category: str, prefix: bool = False) -> Dict[str, Any]:
        """특정 카테고리의 모든 문서를 삭제

        Args:
            category: 카테고리명
            prefix: True이면 하위 카테고리(category/*)도 포함하여 삭제

        Returns:
            {"deleted": int, "failures": list}
        """
        LOGGER.debug("delete_by_category(category='%s', prefix=%s)", category, prefix)
        if prefix:
            query = {
                "bool": {
                    "should": [
                        {"term": {"category": category}},
                        {"prefix": {"category": category + "/"}},
                    ],
                    "minimum_should_match": 1,
                }
            }
        else:
            query = {"term": {"category": category}}
        result = self.es.delete_by_query(
            index=self.index_name,
            query=query,
            conflicts="abort",
            refresh=True,
        )
        return {
            "deleted": result.get("deleted", 0),
            "failures": result.get("failures", []),
        }

    def delete(self, doc_id: int) -> bool:
        LOGGER.debug("delete(doc_id=%d)", doc_id)
        result = self.es.delete(index=self.index_name, id=str(doc_id), refresh=True)
        if "result" in result:
            if result["result"] == "deleted":
                return True
        return False
