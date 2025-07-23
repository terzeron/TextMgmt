#!/usr/bin/env python

import sys
import os
import math
import warnings
import time
import logging.config
from pathlib import Path
from typing import Dict, List, Any, Tuple, Union
from itertools import islice
from urllib.parse import urlparse, urlunparse
from elasticsearch import Elasticsearch, RequestError, NotFoundError
from elasticsearch.exceptions import ElasticsearchWarning

warnings.filterwarnings("ignore", category=DeprecationWarning)
warnings.filterwarnings("ignore", category=ElasticsearchWarning)
warnings.filterwarnings("ignore", category=UserWarning)
logging.config.fileConfig(Path(__file__).parent.parent / "logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger(__name__)
logging.getLogger("elasticsearch").setLevel(logging.CRITICAL)


class ESManager:
    def __init__(self, es_url: str = "https://localhost:9200", index_name: str = "tm") -> None:
        if es_url:
            parsed_url = urlparse(es_url)
            if not parsed_url.port:
                # If port is not specified, add the default port for the scheme
                port = 80 if parsed_url.scheme == "http" else 443
                parsed_url = parsed_url._replace(netloc=f"{parsed_url.hostname}:{port}")
            self.es_url = urlunparse(parsed_url)
        else:
            self.es_url = "https://localhost:9200"
        self.index_name = index_name
        self.es = None

    def connect(self) -> None:
        if self.es is None:
            self.es = Elasticsearch([self.es_url], verify_certs=False, ssl_show_warn=False)

    def _ensure_index_exists(self) -> None:
        if not self.do_exist_index():
            self.create_index()

    def is_healthy(self) -> bool:
        self.connect()
        self.es.info()
        return True

    def do_exist_index(self) -> bool:
        self.connect()
        LOGGER.debug("do_exist_index()")
        return self.es.indices.exists(index=self.index_name)

    def create_index(self) -> dict[str, Any]:
        self.connect()
        LOGGER.debug("create_index()")

        settings = {
            "index": {
                "analysis": {
                    "analyzer": {
                        "nori_analyzer": {
                            "tokenizer": "nori_tokenizer"
                        }
                    }
                },
                "similarity": {
                    "default": {
                        "type": "BM25"
                    }
                }
            }
        }
        mappings = {
            "properties": {
                "category": {
                    "type": "keyword",
                },
                "title": {
                    "type": "text",
                    "analyzer": "nori_analyzer",
                    "fields": {
                        "keyword": {
                            "type": "keyword"
                        }
                    }
                },
                "author": {
                    "type": "text",
                    "analyzer": "nori_analyzer",
                    "fields": {
                        "keyword": {
                            "type": "keyword"
                        }
                    }
                },
                "file_path": {
                    "type": "keyword"
                },
                "file_type": {
                    "type": "keyword"
                },
                "file_size": {
                    "type": "unsigned_long"
                },
                "summary": {
                    "type": "text",
                    "analyzer": "nori_analyzer"
                },
                "updated_time": {
                    "type": "date",
                }
            }
        }

        if self.do_exist_index():
            return {"acknowledged": True}
        return self.es.indices.create(index=self.index_name, settings=settings, mappings=mappings)

    def delete_index(self) -> None:
        self.connect()
        LOGGER.debug("delete_index()")
        if self.do_exist_index():
            self.es.indices.delete(index=self.index_name)

    def get_mapping(self) -> dict[str, Any]:
        self.connect()
        self._ensure_index_exists()
        LOGGER.debug("get_mapping()")
        while True:
            if self.es.indices.exists(index=self.index_name):
                break
            time.sleep(1)
        return self.es.indices.get_mapping(index=self.index_name)

    def _search(self, query: Dict[str, Any], sort: Union[List[str], str, None] = None, max_result_count: int = sys.maxsize) -> List[Tuple[int, Dict[str, Any], float]]:
        self.connect()
        self._ensure_index_exists()
        LOGGER.debug("_search(max_result_count=%d, query='%s')", max_result_count, query)
        result = []
        try:
            response = self.es.search(index=self.index_name, query=query, sort=sort, size=max_result_count)
            if not response or "hits" not in response:
                return []
        except Exception:
            return []

        max_score = response["hits"].get("max_score", 1.0)
        if not max_score:
            max_score = 1.0

        for hit in response['hits']['hits']:
            normalized_score = hit['_score'] * 100 / max_score if hit['_score'] else 0.0
            # print(hit['_source'], normalized_score)
            result_item = (int(hit['_id']), hit['_source'], normalized_score)
            result.append(result_item)
        return result

    def search_by_title(self, title: str, file_type: str = "", file_size: int = 0, max_result_count: int = sys.maxsize) -> List[Tuple[int, Dict[str, Any], float]]:
        self.connect()
        self._ensure_index_exists()
        LOGGER.debug("search_by_title(max_result_count=%d, title='%s', file_type='%s', file_size=%d)", max_result_count, title, file_type, file_size)
        query = {
            "bool": {
                "should": [
                    {"match": {"title": {"query": title, "boost": 1.2 + math.log2(len(title.split(' ')))}}},
                    {"match": {"file_type": {"query": file_type, "boost": 1}}},
                    {"match": {"file_size": {"query": file_size, "boost": 1}}}
                ]
            }
        }
        return self._search(query, max_result_count=max_result_count)

    def search_by_summary(self, summary: str, max_result_count: int = sys.maxsize) -> List[Tuple[int, Dict[str, Any], float]]:
        self.connect()
        self._ensure_index_exists()
        LOGGER.debug("search_by_summary(max_result_count=%d, summary='%s')", max_result_count, summary)
        query = {
            "match": {
                "summary": summary
            }
        }
        return self._search(query, max_result_count=max_result_count)

    def search_by_category(self, category: str, max_result_count: int = sys.maxsize) -> List[Tuple[int, Dict[str, Any], float]]:
        self.connect()
        self._ensure_index_exists()
        LOGGER.debug("search_by_category(category='%s')", category)
        query = {
            "match": {
                "category": category
            }
        }
        sort = ["author.keyword", "title.keyword"]

        return self._search(query, sort=sort, max_result_count=max_result_count)

    def search_by_keyword(self, keyword: str, max_result_count: int = sys.maxsize) -> List[Tuple[int, Dict[str, Any], float]]:
        self.connect()
        self._ensure_index_exists()
        LOGGER.debug("search_by_keyword(keyword='%s', max_result_count=%d)", keyword, max_result_count)
        query = {
            "bool": {
                "should": [
                    {"match": {"title": {"query": keyword, "boost": 10}}},
                    {"match": {"author": {"query": keyword, "boost": 5}}},
                    {"match": {"summary": {"query": keyword, "boost": 1}}}
                ],
                "minimum_should_match": 1
            }
        }
        return self._search(query, max_result_count=max_result_count)

    def search_similar_docs(self, category: str = "", title: str = "", author: str = "", file_type: str = "", file_size: int = 0, summary: str = "", max_result_count: int = sys.maxsize) -> List[Tuple[int, Dict[str, Any], float]]:
        self.connect()
        self._ensure_index_exists()
        LOGGER.debug("search_similar_docs(category='%s', title='%s', author='%s', type='%s', size=%d, summary='%s', max_result_count=%d)", category, title, author, file_type, file_size, summary, max_result_count)
        query = {
            "bool": {
                "should": [
                    {"match": {"summary": {"query": summary, "boost": 10}}},
                    {"match": {"title": {"query": title, "boost": 5}}},
                    {"match": {"author": {"query": author, "boost": 3}}},
                    {"range": {"file_size": {"gte": file_size * 0.9, "lte": file_size * 1.1, "boost": 2}}},
                ],
                "filter": [
                    {"match": {"file_type": {"query": file_type}}}
                ],
                "minimum_should_match": 1
            }
        }
        return self._search(query, max_result_count=max_result_count)

    def search_by_id(self, doc_id: int) -> Dict[str, Any]:
        self.connect()
        self._ensure_index_exists()
        LOGGER.debug("search_by_id(doc_id=%d)", doc_id)
        query = {
            "match": {
                "_id": str(doc_id)
            }
        }
        result_list = self._search(query, max_result_count=1)
        if result_list and len(result_list) > 0:
            return result_list[0][1]
        return {}

    def search_and_aggregate_by_category(self) -> List[str]:
        self.connect()
        self._ensure_index_exists()
        LOGGER.debug("search_and_aggregate_by_category()")
        field_name = "category"
        size = 100
        body = {
            "size": 1,
            "aggs": {
                "unique_values": {
                    "terms": {
                        "field": field_name,
                        "size": size
                    }
                }
            }
        }
        result = self.es.search(index=self.index_name, body=body)
        unique_values = [bucket['key'] for bucket in result['aggregations']['unique_values']['buckets']]
        return unique_values

    def insert(self, data: Dict[int, Dict[str, Any]], num_docs: int = sys.maxsize) -> List[int]:
        self.connect()
        self._ensure_index_exists()
        LOGGER.debug("insert() %d items", len(data))
        operations: List[Dict[str, Any]] = []
        data_count = 0
        iter_items = iter(data.items())
        doc_id_list: List[int] = []
        while True:
            chunk = list(islice(iter_items, 1000))
            if not chunk:
                break
            for inode_num, path_and_size in chunk:
                operations.append({"index": {"_index": self.index_name, "_id": str(inode_num)}})
                operations.append(path_and_size)
                doc_id_list.append(inode_num)
                data_count += 1
            LOGGER.info("%d items inserted", int(len(operations) / 2))
            self.es.bulk(operations=operations, timeout="60s", refresh=True)
            operations = []
            if data_count >= num_docs:
                break
        return doc_id_list

    def update(self, doc_id: int, category: str = "", title: str = "", author: str = "", file_path: str = "", file_type: str = "", file_size: int = 0, summary: str = "") -> bool:
        self.connect()
        self._ensure_index_exists()
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
        self.connect()
        self._ensure_index_exists()
        LOGGER.debug("delete(doc_id=%d)", doc_id)
        result = self.es.delete(index=self.index_name, id=str(doc_id), refresh=True)
        if "result" in result:
            if result["result"] == "deleted":
                return True
        return False
