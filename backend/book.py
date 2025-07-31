#!/usr/bin/env pyhton

import sys
import os
import json
import logging.config
from pathlib import Path
from datetime import datetime
from typing import Dict, Any

logging.config.fileConfig(Path(__file__).parent.parent / "logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger(__name__)

if "TM_WORK_DIR" not in os.environ:
    LOGGER.error("The environment variable TM_WORK_DIR is not set.")
    sys.exit(-1)


class Book:
    path_prefix = Path(os.environ["TM_WORK_DIR"])

    def __init__(self, book_id: int, info: Dict[str, Any], **kwargs) -> None:
        self.book_id: int = book_id
        self.category: str = info["category"]
        self.title: str = info["title"]
        self.author: str = info["author"]
        self.file_path: Path = self.path_prefix / info["file_path"]
        self.file_type: str = info["file_type"]
        self.file_size: int = info["file_size"]
        self.summary: str = info.get("summary", "")
        self.updated_time: datetime = datetime.strptime(info["updated_time"], "%Y-%m-%dT%H:%M:%S.%f")

    def dict(self) -> Dict[str, Any]:
        return {
            "book_id": self.book_id,
            "category": self.category,
            "title": self.title,
            "author": self.author,
            "file_path": str(self.file_path.relative_to(self.path_prefix)),
            "file_type": self.file_type,
            "file_size": self.file_size,
            "updated_time": self.updated_time.strftime("%Y-%m-%dT%H:%M:%S.%f"),
        }

    def json(self) -> str:
        return json.dumps(self.dict())

    def __str__(self) -> str:
        return f"{{category: {self.category}, title: {self.title}, author: {self.author}, file_path: {self.file_path}, file_type: {self.file_type}, file_size: {self.file_size}, updated_time: {self.updated_time}}}"
