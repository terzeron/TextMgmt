#!/usr/bin/env python

import sys
import math
from pathlib import Path
from typing import Dict, Any
from elasticsearch7 import Elasticsearch, RequestError
from pprint import pprint


def read_metadata(tsv_file_path: Path) -> Dict[int, str]:
    metadata = {}
    with tsv_file_path.open("r", encoding="utf-8") as infile:
        for line in infile:
            line = line.strip()
            if not line:
                continue
            inode_num, file_path_str, size = line.split("\t")
            dir_path_str = Path(file_path_str).parent.name
            name = Path(file_path_str).stem
            ext = Path(file_path_str).suffix[1:]
            metadata[int(inode_num)] = {"dir": dir_path_str, "name": name, "ext": ext, "size": int(size)}
    return metadata


class ESClient:

    def __init__(self, index_name):
        self.es = Elasticsearch()
        self.index_name = index_name

    def create_index(self):
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
                }
            }
        }
        self.es.indices.create(index=self.index_name, settings=settings, mappings=mappings)

    def load(self, metadata: Dict[int, str]) -> None:
        print("Loading metadata ...")
        es_data = []
        for inode_num, path_and_size in metadata.items():
            es_data.append({"index": {"_index": self.index_name, "_id": inode_num}})
            es_data.append(path_and_size)
        print(len(es_data))
        response = self.es.bulk(body=es_data, request_timeout=60, refresh=True)
        # print(response)

    def search(self, keyword: str, ext: str = "", size: int = 0) -> None:
        print(f"Searching with keyword '{keyword}' ...")
        # query = {
        #    "match": {
        #        "name": keyword
        #    }
        # }
        query = {
            "bool": {
                "should": [
                    {"match": {"name": {"query": keyword, "boost": 1.2 + math.log2(len(keyword.split(' ')))}}},
                    {"match": {"ext": {"query": ext, "boost": 1}}},
                    {"match": {"size": {"query": size, "boost": 1}}}
                ]
            }
        }
        response = self.es.search(index=self.index_name, query=query, scroll='10m')
        total_hits = response['hits']['total']['value']
        max_score = response['hits']['max_score']
        for hit in response['hits']['hits']:
            normalized_score = hit['_score'] * 100 / max_score
            print(hit['_source'], normalized_score)
        scroll_id = response['_scroll_id']
        scroll_size = response['hits']['total']['value']

        while scroll_size > 0:
            response = self.es.scroll(scroll_id=scroll_id, scroll='10m')
            total_hits = response['hits']['total']['value']
            max_score = response['hits']['max_score']
            for hit in response['hits']['hits']:
                normalized_score = hit['_score'] * 100 / max_score
                print(hit['_source'], normalized_score)
            scroll_id = response['_scroll_id']
            scroll_size = len(response['hits']['hits'])


def main() -> int:
    tsv_file = sys.argv[1]
    if len(sys.argv) > 2:
        keyword = sys.argv[2]
    else:
        return 0
    ext = ""
    if len(sys.argv) > 3:
        ext = sys.argv[3]
    size = 0
    if len(sys.argv) > 4:
        size = sys.argv[4]

    tsv_file_path = Path(tsv_file)
    metadata = read_metadata(tsv_file_path)
    es_client = ESClient("tm")
    try:
        es_client.create_index()
        es_client.load(metadata)
    except RequestError as e:
        print(e)
    es_client.search(keyword, ext, size)
    return 0


if __name__ == "__main__":
    sys.exit(main())

