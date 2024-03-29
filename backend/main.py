#!/usr/bin/env python


import os
import logging.config
import asyncio
import platform
from typing import Dict, Any, Union, Optional
from datetime import datetime
from fastapi import FastAPI, Response, Header, status
from fastapi.middleware.cors import CORSMiddleware
from fastapi.middleware.gzip import GZipMiddleware
from fastapi.responses import FileResponse
from fastapi_utils.tasks import repeat_every
from text_manager import TextManager

logging.config.fileConfig("logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger(__name__)
# ignore debug logs from inotify.adapters
logging.getLogger("inotify.adapters").setLevel(logging.WARNING)

app = FastAPI()
origins = [
    os.environ["TM_DOMAIN"] if "TM_DOMAIN" in os.environ else "https://localhost:3000"
]
app.add_middleware(CORSMiddleware, allow_origins=origins, allow_credentials=True, allow_methods=["*"], allow_headers=["*"])
app.add_middleware(GZipMiddleware, minimum_size=1000)

text_manager = TextManager()
asyncio.create_task(text_manager.get_full_dirs())


def respond_with_304_not_modified(if_modified_since: Optional[str]) -> bool:
    LOGGER.debug("# respond_with_304_not_modified(%r, %r)", if_modified_since, text_manager.get_last_modified_time_str())
    if if_modified_since:
        LOGGER.debug("if_modified_since=%r", text_manager.get_if_modified_since_time_str(if_modified_since))
    if if_modified_since and text_manager.get_last_modified_time_str():
        if text_manager.get_last_modified_time_str() <= text_manager.get_if_modified_since_time_str(if_modified_since):
            return True
    return False


@app.on_event("startup")
@repeat_every(seconds=50)
async def check_recent_changes_in_fs() -> None:
    LOGGER.debug("# check_recent_changes_in_fs()")
    if platform.system() == "Linux":
        rs = text_manager.conn.cursor().execute("SELECT last_modified_time FROM fs_modification")
        last_modified_time = rs.fetchone()["last_modified_time"]
        if datetime.now() - datetime.timedelta(minutes=2) < last_modified_time < datetime.now() - datetime.timedelta(minutes=1):
            text_manager.conn.cursor().execute("UPDATE cache_modification SET last_modified_time = NOW()")
            text_manager.conn.commit()
            LOGGER.debug("updated last modified time of cache")
            await asyncio.create_task(text_manager.get_full_dirs())
    else:
        text_manager.conn.cursor().execute("UPDATE cache_modification SET last_modified_time = NOW()")
        text_manager.conn.commit()
        LOGGER.debug("updated last modified time of cache")
        await asyncio.create_task(text_manager.get_full_dirs())


@app.put("/dirs/{dir_name}/files/{file_name}/newdir/{new_dir_name}/newfile/{new_file_name}")
async def move_file(dir_name: str, file_name: str, new_dir_name: str, new_file_name: str) -> Dict[str, Any]:
    LOGGER.debug(
        f"# move_file(dir_name={dir_name}, file_name={file_name}, new_dir_name={new_dir_name}, new_file_name={new_file_name})")
    response_object: Dict[str, Any] = {"status": "failure"}
    result, error = await text_manager.move_file(dir_name, file_name, new_dir_name, new_file_name)
    if error is None:
        response_object["status"] = "success"
        response_object["result"] = result
        response_object["last_modified_time"] = text_manager.get_last_modified_time_str()
        response_object["last_responded_time"] = text_manager.get_last_responded_time_str()
    else:
        response_object["error"] = error
    return response_object


@app.delete("/dirs/{dir_name}/files/{file_name}")
async def delete_file(dir_name: str, file_name: str) -> Dict[str, Any]:
    LOGGER.debug(f"# delete_file(dir_name={dir_name}, file_name={file_name})")
    response_object: Dict[str, Any] = {"status": "failure"}
    result, error = await text_manager.delete_file(dir_name, file_name)
    if error is None:
        response_object["status"] = "success"
        response_object["result"] = result
        response_object["last_modified_time"] = text_manager.get_last_modified_time_str()
        response_object["last_responded_time"] = text_manager.get_last_responded_time_str()
    else:
        response_object["error"] = error
    return response_object


@app.get("/download/dirs/{dir_name}/files/{file_name}", response_model=None)
async def get_file_content(dir_name: str, file_name: str) -> Union[str, FileResponse]:
    LOGGER.debug(f"# get_file(dir_name={dir_name}, file_name={file_name})")
    return await text_manager.get_file_content(dir_name, file_name)


@app.get("/dirs/{dir_name}/files/{file_name}")
async def get_file_info(response: Response, dir_name: str, file_name: str, if_modified_since: Optional[str] = Header(None)) -> Dict[str, Any]:
    LOGGER.debug(f"# get_file(dir_name={dir_name}, file_name={file_name})")
    response_object: Dict[str, Any] = {"status": "failure"}
    result, error = await text_manager.get_file_info(dir_name, file_name)
    if error is None:
        response_object["status"] = "success"
        response_object["result"] = result
        response_object["last_modified_time"] = text_manager.get_last_modified_time_str()
        response_object["last_responded_time"] = text_manager.get_last_responded_time_str()
    else:
        response_object["error"] = error
    # LOGGER.debug(response_object)
    return response_object


@app.get("/dirs/{dir_name}")
async def get_a_dir(response: Response, dir_name: str, if_modified_since: Optional[str] = Header(None)) -> Dict[
    str, Any]:
    LOGGER.debug(f"# get_a_dir(dir_name={dir_name})")
    response_object: Dict[str, Any] = {"status": "failure"}
    result, error = await text_manager.get_entries_from_dir(dir_name)
    if error is None:
        response_object["status"] = "success"
        response_object["result"] = result
        response_object["last_modified_time"] = text_manager.get_last_modified_time_str()
        response_object["last_responded_time"] = text_manager.get_last_responded_time_str()
    else:
        response_object["error"] = error
    LOGGER.debug(response_object)
    return response_object


@app.get("/dirs")
async def get_full_dirs(response: Response, if_modified_since: Optional[str] = Header(None)) -> Dict[str, Any]:
    LOGGER.debug(f"# get_full_dirs(if_modified_since={if_modified_since})")
    if respond_with_304_not_modified(if_modified_since):
        LOGGER.debug(f"respond empty reponse body with 304 status code")
        response.status_code = status.HTTP_304_NOT_MODIFIED
        return {}
    response.headers["Last-Modified"] = text_manager.get_last_modified_header_str()
    response_object: Dict[str, Any] = {"status": "failure"}
    result, error = await text_manager.get_full_dirs()
    if error is None:
        response_object["status"] = "success"
        response_object["result"] = result
        response_object["last_modified_time"] = text_manager.get_last_modified_time_str()
        response_object["last_responded_time"] = text_manager.get_last_responded_time_str()
    else:
        response_object["error"] = error
    return response_object


@app.get("/topdirs")
async def get_top_dirs(response: Response, if_modified_since: Optional[str] = Header(None)) -> Dict[str, Any]:
    LOGGER.debug(f"# get_top_dir(if_modified_since={if_modified_since})")
    response_object: Dict[str, Any] = {"status": "failure"}
    result, error = await text_manager.get_top_dirs()
    if error is None:
        response_object["status"] = "success"
        response_object["result"] = result
        response_object["last_modified_time"] = text_manager.get_last_modified_time_str()
        response_object["last_responded_time"] = text_manager.get_last_responded_time_str()
    else:
        response_object["error"] = error
    # LOGGER.debug(response_object)
    return response_object


@app.put("/encoding/dirs/{dir_name}/files/{file_name}}")
async def change_encoding(dir_name: str, file_name: str) -> Dict[str, Any]:
    LOGGER.debug(f"# change_encoding(dir_name={dir_name}, file_name={file_name})")
    response_object: Dict[str, Any] = {"status": "failure"}
    result, error = text_manager.change_encoding(dir_name, file_name)
    if error is None:
        response_object["status"] = "success"
        response_object["result"] = result
        response_object["last_modified_time"] = text_manager.get_last_modified_time_str()
        response_object["last_responded_time"] = text_manager.get_last_responded_time_str()
    else:
        response_object["error"] = error
    LOGGER.debug(response_object)
    return response_object


@app.get("/dirs/{dir_name}/files/{file_name}/similar")
async def get_similar_file_list(dir_name: str, file_name: str) -> Dict[str, Any]:
    LOGGER.debug(f"# get_similar_file_list(dir_name={dir_name}, file_name={file_name})")
    response_object: Dict[str, Any] = {"status": "failure"}
    return response_object


@app.post("/search/{query}")
async def search(query: str) -> Dict[str, Any]:
    response_object: Dict[str, Any] = {"status": "failure"}
    result, error = await text_manager.search(query)
    if error is None:
        response_object["status"] = "success"
        response_object["result"] = result
        response_object["last_modified_time"] = text_manager.get_last_modified_time_str()
        response_object["last_responded_time"] = text_manager.get_last_responded_time_str()
    else:
        response_object["error"] = error
    return response_object
