#!/usr/bin/env pyhton

import sys
import os
import json
import logging.config
from pathlib import Path
from datetime import datetime
from typing import Any

logging.config.fileConfig(Path(__file__).parent.parent / "logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger(__name__)

if "TM_BOOK_DIR" not in os.environ:
    LOGGER.error("The environment variable TM_BOOK_DIR is not set.")
    sys.exit(-1)


class Book:
    path_prefix = Path(os.environ["TM_BOOK_DIR"])

    def __init__(self, book_id: int, info: dict[str, Any], score: float = 0.0, **kwargs) -> None:
        self.book_id: int = book_id
        self.category: str = info["category"]
        self.title: str = info["title"]
        self.author: str = info["author"]
        self.file_path: Path = self.path_prefix / info["file_path"]
        self.file_type: str = info["file_type"]
        self.file_size: int = info["file_size"]
        self.line_count: int = info.get("line_count", 0)
        self.page_count: int = info.get("page_count", 0)
        self.isbn: str = info.get("isbn", "")
        self.summary: str = info.get("summary", "")
        self.created_time: datetime = datetime.fromisoformat(info.get("created_time") or info["updated_time"])
        self.updated_time: datetime = datetime.fromisoformat(info["updated_time"])
        self.score: float = score

    def dict(self) -> dict[str, Any]:
        return {
            "book_id": self.book_id,
            "category": self.category,
            "title": self.title,
            "author": self.author,
            "file_path": str(self.file_path.relative_to(self.path_prefix)),
            "file_type": self.file_type,
            "file_size": self.file_size,
            "line_count": self.line_count,
            "page_count": self.page_count,
            "isbn": self.isbn,
            "created_time": self.created_time.strftime("%Y-%m-%dT%H:%M:%S.%f"),
            "updated_time": self.updated_time.strftime("%Y-%m-%dT%H:%M:%S.%f"),
            "score": self.score,
        }

    def json(self) -> str:
        return json.dumps(self.dict())

    def __str__(self) -> str:
        return f"{{category: {self.category}, title: {self.title}, author: {self.author}, file_path: {self.file_path}, file_type: {self.file_type}, file_size: {self.file_size}, updated_time: {self.updated_time}}}"
