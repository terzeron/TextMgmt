#!/usr/bin/env python

import os
import logging
import chardet
from pathlib import Path
from typing import Any, Optional, Dict, Tuple, List

logging.config.fileConfig("logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger(__name__)


class TextManager:
    def __init__(self):
        self.path_prefix = Path(os.environ["HOME"]) / "workspace" / "TextMgmt"

    def change_encoding(self, dir_name: str, file_name: str, encoding: str) -> Tuple[Dict[str, Any], Optional[Any]]:
        pass

    def move_file(self, dir_name: str, file_name: str, new_dir_name: str) -> Tuple[Dict[str, Any], Optional[Any]]:
        pass

    def get_similar_file_list(self, dir_name: str, file_name: str) -> Tuple[Dict[str, Any], Optional[Any]]:
        pass

    def rename_file(self, dir_name: str, file_name: str, new_file_name: str) -> Tuple[Dict[str, Any], Optional[Any]]:
        pass

    def delete_file(self, dir_name: str, file_name: str) -> Tuple[Dict[str, Any], Optional[Any]]:
        pass

    def determine_file_content_and_encoding(self, path: Path) -> Tuple[str, str]:
        encoding = "utf-8"
        content = None
        with open(path, "r") as infile:
            content = infile.read()
        if content:
            encoding_metadata = chardet.detect(content.encode())
            if encoding_metadata["confidence"] > 0.99:
                encoding = encoding_metadata["encoding"]
        return content, encoding

    def get_file_info(self, dir_name: str, file_name: str) -> Tuple[Dict[str, Any], Optional[Any]]:
        LOGGER.debug(f"# get_file_info(dir_name={dir_name}, file_name={file_name})")
        path = self.path_prefix / dir_name / file_name
        if path.is_file():
            st = path.stat()
            content, encoding = self.determine_file_content_and_encoding(path)

            result = {
                "size": st.st_size,
                "encoding": encoding,
                "content": content
            }
            return result, None
        return None, "File not found"

    def get_file_list_from_dir(self, dir_name: str) -> Tuple[List[Dict[str, Any]], Optional[Any]]:
        LOGGER.debug(f"# get_file_list_from_dir(dir_name={dir_name})")
        path = self.path_prefix / dir_name
        result = []
        for entry in path.iterdir():
            if entry.is_dir():
                items = []
                for e in entry.iterdir():
                    if e.is_file():
                        items.append({"id": str(e.relative_to(path)), "title": e.name})
                items.sort(key=lambda x: x["id"])
                result.append({"id": str(entry.relative_to(path)), "title": entry.name, "items": items})
            elif entry.is_file():
                result.append({"id": str(entry.relative_to(path)), "title": entry.name})
        result.sort(key=lambda x: x["id"])
        return result, None

    def search(self, query: str) -> Tuple[Dict[str, Any], Optional[Any]]:
        pass
