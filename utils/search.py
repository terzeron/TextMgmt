#!/usr/bin/env python


import sys
import logging.config
from backend.es_manager import ESManager


logging.config.fileConfig("../logging.conf")
LOGGER = logging.getLogger()


def print_usage(program_name: str):
    print(f"Usage:\t{program_name}\t<title> [ <content> [ <ext> [ <size> ] ] ]\n")
    print("\tIf you specify <title> argument, this program will do searching with <title>, <ext>, and <size>.")
    print("\tIf you specify <content> argument, this program will do searching with <content> only.")
    sys.exit(0)


def main() -> int:
    title = ""
    content = ""
    ext = ""
    size = 0
    max_result_count = 10

    if len(sys.argv) > 1:
        title = sys.argv[1]
    else:
        print_usage(sys.argv[0])
    if len(sys.argv) > 2:
        content = sys.argv[2]
    if len(sys.argv) > 3:
        ext = sys.argv[3]
    if len(sys.argv) > 4:
        size = int(sys.argv[4])

    es = ESManager("tm")
    if title:
        result = es.search_by_title(max_result_count, title, ext, size)
        for item, score in result:
            print("----------------------------------------")
            print("item:", item["name"], "ext:", item["ext"], "size:", item["size"], "score:", score)
            print(item["content"][:200].replace("\n", " "))
    if content:
        result = es.search_by_content(max_result_count, content)
        for item, score in result:
            print("item:", item["name"], "ext:", item["ext"], "size:", item["size"], "score:", score)
            print(item["content"][:200].replace("\n", " "))

    return 0


if __name__ == "__main__":
    sys.exit(main())
