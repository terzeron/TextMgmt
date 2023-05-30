#!/usr/bin/env python


import math
import logging.config
from pathlib import Path
from typing import Dict, List, Any, Tuple
from elasticsearch7 import Elasticsearch


logging.config.fileConfig("../logging.conf")
LOGGER = logging.getLogger()
TEXT_SIZE = 4096
TEMP_DIR_PREFIX_PATH = Path("/mnt/ramdisk")


class ESClient:
    def __init__(self, index_name) -> None:
        self.es = Elasticsearch()
        self.index_name = index_name

    def __del__(self) -> None:
        del self.es

    def create_index(self) -> None:
        print("Creating index ...")
        self.es = Elasticsearch()
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
                "dir": {
                    "type": "keyword",
                },
                "name": {
                    "type": "text",
                    "analyzer": "nori_analyzer"
                },
                "ext": {
                    "type": "keyword"
                },
                "size": {
                    "type": "unsigned_long"
                },
                "content": {
                    "type": "text",
                    "analyzer": "nori_analyzer"
                },
                "updated_time": {
                    "type": "date",
                }
            }
        }
        self.es.indices.create(index=self.index_name, settings=settings, mappings=mappings)

    def load(self, data: Dict[int, Dict[str, Any]]) -> None:
        print("Loading data ...")
        es_data: List[Dict[str, Any]] = []
        data_count = 0
        from itertools import islice
        iter_items = iter(data.items())
        while True:
            chunk = list(islice(iter_items, 1000))
            if not chunk:
                break
            for inode_num, path_and_size in chunk:
                es_data.append({"index": {"_index": self.index_name, "_id": inode_num}})
                es_data.append(path_and_size)
                data_count += 1
            print(int(len(es_data) / 2))
            self.es.bulk(body=es_data, request_timeout=60, refresh=True)
            es_data = []

    def _search(self, max_result_count: int, query: Dict[str, Any]) -> List[Tuple[Dict[str, Any], float]]:
        result_count = 0
        result = []
        response = self.es.search(index=self.index_name, query=query, scroll='10m')
        #total_hits = response['hits']['total']['value']
        max_score = response['hits']['max_score']
        for hit in response['hits']['hits']:
            normalized_score = hit['_score'] * 100 / max_score
            #print(hit['_source'], normalized_score)
            result.append((hit['_source'], normalized_score))
            result_count += 1
            if result_count >= max_result_count:
                return result[:max_result_count]
        scroll_id = response['_scroll_id']
        scroll_size = response['hits']['total']['value']

        while scroll_size > 0:
            response = self.es.scroll(scroll_id=scroll_id, scroll='10m')
            #total_hits = response['hits']['total']['value']
            max_score = response['hits']['max_score']
            for hit in response['hits']['hits']:
                normalized_score = hit['_score'] * 100 / max_score
                #print(hit['_source'], normalized_score)
                result.append((hit['_source'], normalized_score))
                result_count += 1
                if result_count >= max_result_count:
                    return result[:max_result_count]
            scroll_id = response['_scroll_id']
            scroll_size = len(response['hits']['hits'])
            #print(result_count)
            #print(len(result))

        return result[:max_result_count]

    def search_by_title(self, max_result_count: int, title: str, ext: str = "", size: int = 0) -> List[Tuple[Dict[str, Any]]]:  
        print(f"Searching by title and metata with '{title}', '{ext}', '{size}' ...")
        query = {
            "bool": {
                "should": [
                    {"match": {"name": {"query": title, "boost": 1.2 + math.log2(len(title.split(' ')))}}},
                    {"match": {"ext": {"query": ext, "boost": 1}}},
                    {"match": {"size": {"query": size, "boost": 1}}}
                ]
            }
        }
        return self._search(max_result_count, query)

    def search_by_content(self, max_result_count: int, content: str) -> List[Tuple[Dict[str, Any]]]:
        print(f"Searching by content with '{content}' ...")
        query = {
            "match": {
                "content": content
            }
        }
        return self._search(max_result_count, query)
