#!/usr/bin/env python


import sys
import logging.config
from pathlib import Path
from backend.es_manager import ESManager

logging.config.fileConfig(Path(__file__).parent.parent / "logging.conf", disable_existing_loggers=False)
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

    es = ESManager()
    if title:
        result = es.search_by_title(title, ext, size, max_result_count=10)
        for _, item, _ in result:
            print("----------------------------------------")
            print("item:", item)
            print(item["summary"][:200].replace("\n", " "))
    if content:
        result = es.search_similar_docs(summary=content, max_result_count=max_result_count)
        for _, item, _ in result:
            print("item:", item)
            print(item["summary"][:200].replace("\n", " "))

    return 0


if __name__ == "__main__":
    sys.exit(main())
