#!/usr/bin/env python

import os
import logging
import chardet
from pathlib import Path
from typing import Any, Optional, Dict, Tuple, List, Union
from fastapi.responses import FileResponse


logging.config.fileConfig("logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger(__name__)


class TextManager:
    def __init__(self):
        work_dir = os.environ["TM_WORK_DIR"] if "TM_WORK_DIR" in os.environ else os.getcwd() + "/.."
        self.path_prefix = Path(work_dir)

    def change_encoding(self, dir_name: str, file_name: str, encoding: str) -> Tuple[Dict[str, Any], Optional[Any]]:
        LOGGER.debug(f"# change_encoding(dir_name={dir_name}, file_name={file_name}, encoding={encoding})")

    def move_file(self, dir_name: str, file_name: str, new_dir_name: str) -> Tuple[Dict[str, Any], Optional[Any]]:
        LOGGER.debug(f"# move_file(dir_name={dir_name}, file_name={file_name}, new_dir_name={new_dir_name})")

    def get_similar_file_list(self, dir_name: str, file_name: str) -> Tuple[Dict[str, Any], Optional[Any]]:
        LOGGER.debug(f"# get_similar_file_list(dir_name={dir_name}, file_name={file_name})")

    def rename_file(self, dir_name: str, file_name: str, new_file_name: str) -> Tuple[Dict[str, Any], Optional[Any]]:
        LOGGER.debug(f"# rename_file(dir_name={dir_name}, file_name={file_name}, new_file_name={new_file_name})")

    def delete_file(self, dir_name: str, file_name: str) -> Tuple[Dict[str, Any], Optional[Any]]:
        LOGGER.debug(f"# delete_file(dir_name={dir_name}, file_name={file_name})")

    def read_binary_file(self, path: Path) -> bytes:
        LOGGER.debug(f"# read_binary_file(path={path})")
        with open(path, "rb") as f:
            return f.read()

    def determine_file_content_and_encoding(self, path: Path) -> Tuple[str]:
        LOGGER.debug(f"# determine_file_content_and_encoding(path={path})")
        encoding = "utf-8"
        with open(path, "r") as infile:
            content = infile.read(1024 * 100)
            if content:
                encoding_metadata = chardet.detect(content.encode())
                if encoding_metadata["confidence"] > 0.99:
                    encoding = encoding_metadata["encoding"]
        return encoding

    def get_file_content(self, dir_name: str, file_name: str, size: int = 0) -> Union[str, bytes]:
        LOGGER.debug(f"# get_file_content(dir_name={dir_name}, file_name={file_name}, size={size})")
        path = self.path_prefix / dir_name / file_name
        if path.is_file():
            if path.suffix == ".txt":
                content = FileResponse(path=path, media_type="text/plain")
            elif path.suffix == ".pdf":
                content = FileResponse(path=path, media_type="application/pdf")
            elif path.suffix == ".epub":
                content = FileResponse(path=path, media_type="application/epub+zip")
            elif path.suffix == ".html":
                content = FileResponse(path=path, media_type="text/html")

            return content
        return None

    def get_file_info(self, dir_name: str, file_name: str) -> Tuple[Union[Dict[str, Any], bytes], Optional[Any]]:
        LOGGER.debug(f"# get_file_info(dir_name={dir_name}, file_name={file_name})")
        path = self.path_prefix / dir_name / file_name
        if path.is_file():
            st = path.stat()
            if path.suffix == ".txt":
                encoding = self.determine_file_content_and_encoding(path)
            else:
                encoding = "binary"

            result = {
                "size": st.st_size,
                "encoding": encoding,
            }
            return result, None
        return None, "File not found"

    def get_full_dirs(self) -> Tuple[List[Dict[str, Any]], Optional[Any]]:
        LOGGER.debug(f"# get_full_dirs()")
        path = self.path_prefix
        result = []
        for entry in path.iterdir():
            if entry.is_dir():
                nodes = []
                for e in entry.iterdir():
                    if e.is_file():
                        nodes.append({"key": e.name, "label": e.name})
                nodes.sort(key=lambda x: x["key"])
                result.append({"key": entry.name, "label": entry.name, "nodes": nodes})
            elif entry.is_file():
                result.append({"key": entry.name, "label": entry.name})
        result.sort(key=lambda x: x["key"])
        return result, None

    def get_some_dirs(self) -> Tuple[List[Dict[str, Any]], Optional[Any]]:
        LOGGER.debug(f"# get_some_dirs()")
        path = self.path_prefix
        result = []
        for entry in path.iterdir():
            if entry.is_dir():
                result.append({"key": entry.name, "label": entry.name})
        for entry in path.iterdir():
            if entry.is_dir():
                nodes = []
                count = 0
                for e in entry.iterdir():
                    if e.is_file():
                        nodes.append({"key": e.name, "label": e.name})
                        count = count + 1
                        if count >= 100:
                            break
                result.append({"key": entry.name, "label": entry.name, "nodes": nodes})
            elif entry.is_file():
                result.append({"key": entry.name, "label": entry.name})
        result.sort(key=lambda x: x["key"])
        return result, None

    def get_top_dirs(self) -> Tuple[List[Dict[str, Any]], Optional[Any]]:
        LOGGER.debug(f"# get_top_dirs()")
        path = self.path_prefix
        result = []
        for entry in path.iterdir():
            if entry.is_dir():
                result.append({"key": entry.name, "label": entry.name, "nodes": []})
        result.sort(key=lambda x: x["key"])
        return result, None

    def search(self, query: str) -> Tuple[Dict[str, Any], Optional[Any]]:
        pass
