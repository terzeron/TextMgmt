#!/usr/bin/env python

import sys
import os
import math
import warnings
import logging.config
from pathlib import Path
from typing import Dict, List, Any, Tuple, Union, Set
from itertools import islice
from elasticsearch import Elasticsearch


logging.config.fileConfig(
    Path(__file__).parent.parent / "logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger(__name__)
logging.getLogger("elasticsearch").setLevel(logging.CRITICAL)


class ESManager:
    DEFAULT_MAX_RESULT_COUNT = 10

    def __init__(self) -> None:
        for env in ["TM_ES_INDEX", "TM_ES_URL", "TM_ES_USER", "TM_ES_PASSWORD"]:
            print(f"{env}={os.environ[env]}")
            if env not in os.environ:
                LOGGER.error(f"The environment variable {env} is not set.")
                sys.exit(-1)

        self.index_name = os.environ["TM_ES_INDEX"]
        url = os.environ["TM_ES_URL"]
        user = os.environ["TM_ES_USER"]
        password = os.environ["TM_ES_PASSWORD"]

        self.es = Elasticsearch(
            hosts=[url],
            basic_auth=(user, password),
            request_timeout=10,
            retry_on_timeout=True,
        )

    def __del__(self) -> None:
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
                            "EC",    # 연결 어미
                            "EF",    # 종결 어미
                            "EP",    # 선어말 어미
                            "ETM",   # 관형형 전성 어미
                            "ETN",   # 명사형 전성 어미
                            # 조사 (Josa)
                            "JC",    # 접속 조사
                            "JKB",   # 부사격 조사
                            "JKC",   # 보격 조사
                            "JKG",   # 관형격 조사
                            "JKO",   # 목적격 조사
                            "JKQ",   # 인용격 조사
                            "JKS",   # 주격 조사
                            "JKV",   # 호격 조사
                            "JX",    # 보조사
                            # 기호
                            "SC",    # 구분자
                            "SE",    # 줄임표
                            "SF",    # 마침표, 물음표, 느낌표
                            "SP",    # 공백
                            "SSC",   # 닫는 괄호
                            "SSO",   # 여는 괄호
                            "SY",    # 기타 기호
                            # 접미사
                            "XSA",   # 형용사 파생 접미사
                            "XSN",   # 명사 파생 접미사
                            "XSV",   # 동사 파생 접미사
                            # 기타
                            "IC",    # 감탄사
                            "MAJ",   # 접속부사
                        ]
                    }
                },
                "analyzer": {
                    "nori_analyzer": {
                        "type": "custom",
                        "tokenizer": "nori_tokenizer",
                        "filter": ["nori_posfilter", "lowercase"]
                    }
                }
            }
        }
        mappings = {
            "properties": {
                "category": {
                    "type": "keyword",
                },
                "title": {"type": "text", "analyzer": "nori_analyzer", "fields": {"keyword": {"type": "keyword"}}},
                "author": {"type": "text", "analyzer": "nori_analyzer", "fields": {"keyword": {"type": "keyword"}}},
                "file_path": {"type": "keyword"},
                "file_type": {"type": "keyword"},
                "file_size": {"type": "unsigned_long"},
                "summary": {"type": "text", "analyzer": "nori_analyzer"},
                "updated_time": {
                    "type": "date",
                },
            }
        }

        if self.do_exist_index():
            return {"acknowledged": True}
        try:
            return self.es.indices.create(index=self.index_name, body={"settings": settings, "mappings": mappings})
        except BadRequestError as e:
            if "resource_already_exists_exception" in str(e):
                LOGGER.info("Index %s already exists, skipping creation", self.index_name)
                return {"acknowledged": True}
            raise

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

    def _search(self, query: Dict[str, Any], sort: Union[List[str], str, None] = None, max_result_count: int = -1) -> List[Tuple[int, Dict[str, Any], float]]:
        if max_result_count < 0:
            max_result_count = self.DEFAULT_MAX_RESULT_COUNT

        # Clamp the size to prevent ES 'max_result_window' error (default 10000)
        size = min(max_result_count, 10000)

        LOGGER.debug("_search(max_result_count=%d, size=%d, query='%s')",
                     max_result_count, size, query)
        result_count = 0
        result = []
        response = self.es.search(index=self.index_name, query=query,
                                  sort=sort, scroll="10m", track_scores=True, size=size)
        # total_hits = response['hits']['total']['value']
        max_score = response["hits"]["max_score"]
        if max_score is None:
            return []

        for hit in response["hits"]["hits"]:
            normalized_score = hit["_score"] * 100 / \
                max_score if max_score > 0 else 0
            # print(hit['_source'], normalized_score)
            result.append((int(hit["_id"]), hit["_source"], normalized_score))
            result_count += 1
            if result_count >= max_result_count:
                return result[:max_result_count]
        scroll_id = response["_scroll_id"]
        scroll_size = response["hits"]["total"]["value"]

        while scroll_size > 0:
            response = self.es.scroll(scroll_id=scroll_id, scroll="10m")
            # total_hits = response['hits']['total']['value']
            max_score = response["hits"]["max_score"]
            if max_score is None:
                return []
            for hit in response["hits"]["hits"]:
                normalized_score = hit["_score"] * 100 / max_score if max_score > 0 else 0
                # print(hit['_source'], normalized_score)
                result_item = (int(hit["_id"]), hit["_source"], normalized_score)
                result.append(result_item)
                result_count += 1
                if result_count >= max_result_count:
                    return result[:max_result_count]
            scroll_id = response["_scroll_id"]
            scroll_size = len(response["hits"]["hits"])
            # print(result_count)
            # print(len(result))

        return result[:max_result_count]

    def search_by_title(
        self, title: str, file_type: str = "", file_size: int = 0, max_result_count: int = -1
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
                    {"match": {"title": {"query": title, "boost": 1.2 +
                                         math.log2(len(title.split(" ")))}}},
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
            "search_by_summary(max_result_count=%d, summary='%s')", max_result_count, summary)
        query = {"match": {"summary": summary}}
        return self._search(query, max_result_count=max_result_count)

    def search_by_category(self, category: str, max_result_count: int = -1) -> List[Tuple[int, Dict[str, Any], float]]:
        if max_result_count < 0:
            max_result_count = self.DEFAULT_MAX_RESULT_COUNT
        LOGGER.debug("search_by_category(category='%s')", category)
        query = {"match": {"category": category}}
        sort = ["author.keyword", "title.keyword"]

        return self._search(query, sort=sort, max_result_count=max_result_count)

    def search_by_keyword(self, keyword: str, max_result_count: int = -1) -> List[Tuple[int, Dict[str, Any], float]]:
        if max_result_count < 0:
            max_result_count = self.DEFAULT_MAX_RESULT_COUNT
        LOGGER.debug(
            "search_by_keyword(keyword='%s', max_result_count=%d)", keyword, max_result_count)
        query = {
            "bool": {
                "should": [
                    {"match": {"title": {"query": keyword, "boost": 10}}},
                    {"match": {"author": {"query": keyword, "boost": 5}}},
                    {"match": {"summary": {"query": keyword, "boost": 1}}},
                ],
                "minimum_should_match": 1,
            }
        }
        return self._search(query, max_result_count=max_result_count)

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
                    {"range": {"file_size": {"gte": file_size *
                                             0.9, "lte": file_size * 1.1, "boost": 1}}},
                ],
                "minimum_should_match": 1,
            }
        }
        if exclude_id is not None:
            query["bool"]["must_not"] = [{"term": {"_id": str(exclude_id)}}]
        return self._search(query, max_result_count=max_result_count)

    def search_by_id(self, doc_id: int) -> Dict[str, Any]:
        LOGGER.debug("search_by_id(doc_id=%d)", doc_id)
        query = {"match": {"_id": str(doc_id)}}
        result_list = self._search(query, max_result_count=1)
        if result_list and len(result_list) > 0:
            return result_list[0][1]
        return {}

    def search_and_aggregate_by_category(self) -> List[str]:
        LOGGER.debug("search_and_aggregate_by_category()")
        field_name = "category"
        size = 100
        body = {"size": 1, "aggs": {"unique_values": {
            "terms": {"field": field_name, "size": size}}}}
        result = self.es.search(index=self.index_name, body=body)
        unique_values = [bucket["key"]
                         for bucket in result["aggregations"]["unique_values"]["buckets"]]
        return unique_values

    def insert(self, data: Dict[int, Dict[str, Any]], num_docs: int = sys.maxsize) -> List[int]:
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
                es_data.append(
                    {"index": {"_index": self.index_name, "_id": str(inode_num)}})
                es_data.append(path_and_size)
                doc_id_list.append(inode_num)
                data_count += 1
            LOGGER.info("%d items inserted", int(len(es_data) / 2))
            self.es.bulk(body=es_data, timeout="60s", refresh=True)
            es_data = []
            if data_count >= num_docs:
                break
        return doc_id_list

    def update(self, doc_id: int, category: str = "", title: str = "", author: str = "", file_path: str = "", file_type: str = "", file_size: int = 0, summary: str = "") -> bool:
        LOGGER.debug("update(doc_id=%d, title='%s', author='%s', file_path='%r', file_type='%s', file_size=%d, summary='%s', category='%s')", doc_id, title, author, file_path, file_type, file_size, summary, category)
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

    def delete(self, doc_id: int) -> bool:
        LOGGER.debug("delete(doc_id=%d)", doc_id)
        result = self.es.delete(index=self.index_name, id=str(doc_id), refresh=True)
        if "result" in result:
            if result["result"] == "deleted":
                return True
        return False
