#!/usr/bin/env python

import os
import re
import chardet
import logging
from logging import config
from pathlib import Path
from typing import Any, Optional, Dict, Tuple, List, Union
from fastapi.responses import FileResponse

logging.config.fileConfig("logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger(__name__)


class TextManager:
    ROOT_DIRECTORY = "$$rootdir$$"

    def __init__(self):
        LOGGER.debug("# TextManager()")
        work_dir = os.environ["TM_WORK_DIR"] if "TM_WORK_DIR" in os.environ else os.getcwd() + "/../text"
        LOGGER.debug(f"work_dir={work_dir}")
        self.path_prefix = Path(work_dir).resolve()

    def determine_file_path(self, dir_name: str, file_name: str) -> Path:
        if dir_name == TextManager.ROOT_DIRECTORY:
            path = self.path_prefix / file_name
        else:
            path = self.path_prefix / dir_name / file_name
        return path

    def change_encoding(self, dir_name: str, file_name: str) -> Tuple[Optional[str], Optional[Any]]:
        LOGGER.debug(f"# change_encoding(dir_name={dir_name}, file_name={file_name})")
        path = self.determine_file_path(dir_name, file_name)
        common_expression = r"(있었다|것이다|말했다|없었다|않았다|물었다|열었다|같았다|보였다|그러나|그리고|하지만|그렇게|그런데|때문에|갑자기|어떻게|앞으로|천천히|동시에|아니라|아닌가|있다는|말인가|못하고|그녀는|그녀의|자신의|그것은|사람은|그들은|사람이|그들의|자신이|사람의|이렇게|소리가|그래서|소리를|생각이|하였다|우리는|되었다)"
        with open(path, "rb") as infile:
            text = infile.read()
            for encoding in ["cp949", "johab", "utf-16"]:
                try:
                    new_text = text.decode(encoding)
                    m = re.search(common_expression, new_text)
                    if m:
                        return "Ok", None
                except UnicodeDecodeError:
                    continue
        return None, f"can't find appropriate encoding for file '{dir_name}/{file_name}'"

    async def get_similar_file_list(self, dir_name: str, file_name: str) -> Tuple[List[str], Optional[Any]]:
        LOGGER.debug(f"# get_similar_file_list(dir_name={dir_name}, file_name={file_name})")
        path = self.determine_file_path(dir_name, file_name)
        result = [str(path)]
        return result, None

    async def move_file(self, dir_name: str, file_name: str, new_dir_name: str, new_file_name: str) -> Tuple[str, Optional[Any]]:
        LOGGER.debug(f"# move_file(dir_name={dir_name}, file_name={file_name}, new_dir_name={new_dir_name}, new_file_name={new_file_name})")
        path = self.determine_file_path(dir_name, file_name)
        if new_dir_name == TextManager.ROOT_DIRECTORY or new_dir_name.startswith("..") or new_dir_name.startswith("/"):
            new_dir_name = "."
        if not (self.path_prefix / new_dir_name).exists():
            return "Error", f"directory '{new_dir_name}' doesn't exist"
        if new_file_name == "":
            new_file_name = file_name
        new_path = self.path_prefix / new_dir_name / new_file_name
        try:
            path.rename(new_path)
        except IOError as e:
            return "Error", f"can't rename '{dir_name}/{file_name}' to '{new_dir_name}/{new_file_name}', {e}"
        return "Ok", None

    async def delete_file(self, dir_name: str, file_name: str) -> Tuple[str, Optional[Any]]:
        LOGGER.debug(f"# delete_file(dir_name={dir_name}, file_name={file_name})")
        path = self.determine_file_path(dir_name, file_name)
        try:
            path.unlink()
        except IOError as e:
            return "Error", f"can't delete '{dir_name}/{file_name}', {e}"
        return "Ok", None

    @staticmethod
    def determine_file_content_and_encoding(path: Path) -> Tuple[str]:
        LOGGER.debug(f"# determine_file_content_and_encoding(path={path})")
        encoding = "utf-8"
        with open(path, "r") as infile:
            content = infile.read(1024 * 100)
            if content:
                encoding_metadata = chardet.detect(content.encode())
                if encoding_metadata["confidence"] > 0.99:
                    encoding = encoding_metadata["encoding"]
        return encoding

    async def get_file_content(self, dir_name: str, file_name: str, size: int = 0) -> Union[str, FileResponse]:
        LOGGER.debug(f"# get_file_content(dir_name={dir_name}, file_name={file_name}, size={size})")
        content = ""
        path = self.determine_file_path(dir_name, file_name)
        if path.is_file():
            if path.suffix == ".txt":
                content = FileResponse(path=path, media_type="text/plain")
            elif path.suffix == ".pdf":
                content = FileResponse(path=path, media_type="application/pdf")
            elif path.suffix == ".epub":
                content = FileResponse(path=path, media_type="application/epub+zip")
            elif path.suffix == ".html":
                content = FileResponse(path=path, media_type="text/html")
            else:
                content = FileResponse(path=path, media_type="application/octet-stream")

            return content
        return content

    async def get_file_info(self, dir_name: str, file_name: str) -> Tuple[Union[Dict[str, Any], bytes], Optional[Any]]:
        LOGGER.debug(f"# get_file_info(dir_name={dir_name}, file_name={file_name})")
        result = {}
        path = self.determine_file_path(dir_name, file_name)
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
        return result, "File not found"

    async def get_full_dirs(self) -> Tuple[List[Dict[str, Any]], Optional[Any]]:
        #LOGGER.debug(f"# get_full_dirs()")
        path = self.path_prefix
        result = []
        for entry in path.iterdir():
            if entry.is_dir():
                nodes, error = await self.get_entries_from_dir(entry.name)
                result.append({"key": entry.name, "label": entry.name, "nodes": nodes})
            elif entry.is_file():
                result.append({"key": entry.name, "label": entry.name})
        result.sort(key=lambda x: x["key"])
        return result, None

    async def get_entries_from_dir(self, dir_name: str = "", size: int = 0) -> Tuple[List[Dict[str, Any]], Optional[Any]]:
        #LOGGER.debug(f"# get_full_entries_from_dir(dir_name={dir_name}, size={size})")
        # 특정 디렉토리 하위만 조회
        result = []
        path = self.path_prefix / dir_name
        if path.is_dir():
            count = 0
            for entry in path.iterdir():
                if entry.is_file():
                    result.append({"key": entry.name, "label": entry.name})
                    count = count + 1
                    if 0 < size <= count:
                        break
            result.sort(key=lambda x: x["key"])
        else:
            return result, f"can't find '{dir_name}'"
        return result, None

    async def get_some_entries_from_all_dirs(self, size: int = 0) -> Tuple[List[Dict[str, Any]], Optional[Any]]:
        #LOGGER.debug(f"# get_some_entries_from_all_dirs(size={size})")
        # 전체 디렉토리의 일부 몇 개만 조회
        result = []
        path = self.path_prefix
        for entry in path.iterdir():
            if entry.is_dir():
                nodes, error = await self.get_entries_from_dir(entry.name, size)
                result.append({"key": entry.name, "label": entry.name, "nodes": nodes})
            elif entry.is_file():
                result.append({"key": entry.name, "label": entry.name})
        result.sort(key=lambda x: x["key"])
        return result, None

    async def get_top_dirs(self) -> Tuple[List[Dict[str, Any]], Optional[Any]]:
        #LOGGER.debug(f"# get_top_dirs()")
        path = self.path_prefix
        result = []
        for entry in path.iterdir():
            if entry.is_dir():
                result.append({"key": entry.name, "label": entry.name, "nodes": []})
        result.sort(key=lambda x: x["key"])
        return result, None

    async def search(self, query: str) -> Tuple[List[str], Optional[Any]]:
        LOGGER.debug(f"# search(query={query})")
        path = self.path_prefix
        result = [str(path)]
        return result, None
