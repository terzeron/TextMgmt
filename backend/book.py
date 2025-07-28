#!/usr/bin/env pyhton

import sys
import os
import json
import logging.config
from pathlib import Path
from datetime import datetime
from typing import Dict, Any
from pydantic import BaseModel, Field

logging.config.fileConfig(Path(__file__).parent.parent / "logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger(__name__)

if "TM_WORK_DIR" not in os.environ:
    LOGGER.error("The environment variable TM_WORK_DIR is not set.")
    sys.exit(-1)


class Book(BaseModel):
    book_id: int
    category: str
    title: str
    author: str
    file_path: str
    file_type: str
    file_size: int
    summary: str = Field(default="")
    updated_time: str

    def json(self) -> str:
        return json.dumps(self.dict())

    def __str__(self) -> str:
        return f"{{category: {self.category}, title: {self.title}, author: {self.author}, file_path: {self.file_path}, file_type: {self.file_type}, file_size: {self.file_size}, updated_time: {self.updated_time}}}"
