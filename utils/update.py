#!/usr/bin/env python


import sys
import logging.config
from pathlib import Path
from backend.es_manager import ESManager

logging.config.fileConfig(Path(__file__).parent.parent / "logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger()


def main() -> int:
    es = ESManager()
    result = es.update(doc_id=3384, category="2_utf8", title="test title2", author="test author", file_path="/test/path", file_type="test_ext2", file_size=10000000, summary="test content2")
    if not result:
        LOGGER.error("Failed to update")
        return -1
    book = es.search_by_id(3384)
    print(book)
    result = es.delete(3384)
    if not result:
        LOGGER.error("Failed to delete")
        return -1
    return 0


if __name__ == "__main__":
    sys.exit(main())
