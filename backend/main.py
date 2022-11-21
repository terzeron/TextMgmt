#!/usr/bin/env python


import os
import logging
import asyncio
import platform
from typing import Dict, Any, Union, Optional
from datetime import datetime
from fastapi import FastAPI, Response, Header, status
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import FileResponse
from text_manager import TextManager

logging.config.fileConfig("logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger(__name__)
# ignore debug logs from inotify.adapters
logging.getLogger("inotify.adapters").setLevel(logging.WARNING)

app = FastAPI()
origins = [
    os.environ["TM_DOMAIN"] if "TM_DOMAIN" in os.environ else "https://localhost:3000"
]
app.add_middleware(
    CORSMiddleware,
    allow_origins=origins,
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)

text_manager = TextManager()

if platform.system() == "Linux":
    import fs_monitor_in_linux
    fs_monitor_in_linux.start(text_manager)

asyncio.create_task(text_manager.get_full_dirs())


@app.put("/dirs/{dir_name}/files/{file_name}/newdir/{new_dir_name}/newfile/{new_file_name}")
async def move_file(dir_name: str, file_name: str, new_dir_name: str, new_file_name: str) -> Dict[str, Any]:
    LOGGER.debug(f"# move_file(dir_name={dir_name}, file_name={file_name}, new_dir_name={new_dir_name}, new_file_name={new_file_name})")
    response_object: Dict[str, Any] = {"status": "failure"}
    result, error = await text_manager.move_file(dir_name, file_name, new_dir_name, new_file_name)
    if error is None:
        response_object["status"] = "success"
        response_object["result"] = result
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
    else:
        response_object["error"] = error
    return response_object


@app.get("/download/dirs/{dir_name}/files/{file_name}")
async def get_file_content(dir_name: str, file_name: str) -> Union[str, FileResponse]:
    LOGGER.debug(f"# get_file(dir_name={dir_name}, file_name={file_name})")
    return await text_manager.get_file_content(dir_name, file_name)


def respond_with_304_not_modified(if_modified_since: Optional[str]) -> bool:
    if text_manager.last_modified_time and if_modified_since:
        if text_manager.last_modified_time <= datetime.strptime(if_modified_since, HEADER_DATE_FORMAT):
            return True
    return False


@app.get("/dirs/{dir_name}/files/{file_name}")
async def get_file_info(response: Response, dir_name: str, file_name: str, if_modified_since: Optional[str] = Header(None)) -> Dict[str, Any]:
    LOGGER.debug(f"# get_file(dir_name={dir_name}, file_name={file_name})")
    if respond_with_304_not_modified(if_modified_since):
        response.status_code = status.HTTP_304_NOT_MODIFIED
        return {}
    response.headers["Last-Modified"] = text_manager.last_modified_time.strftime(HEADER_DATE_FORMAT)
    response_object: Dict[str, Any] = {"status": "failure"}
    result, error = await text_manager.get_file_info(dir_name, file_name)
    if error is None:
        response_object["status"] = "success"
        response_object["result"] = result
    else:
        response_object["error"] = error
    # LOGGER.debug(response_object)
    return response_object


@app.get("/dirs/{dir_name}")
async def get_a_dir(response: Response, dir_name: str, if_modified_since: Optional[str] = Header(None)) -> Dict[str, Any]:
    LOGGER.debug(f"# get_a_dir(dir_name={dir_name})")
    if respond_with_304_not_modified(if_modified_since):
        response.status_code = status.HTTP_304_NOT_MODIFIED
        return {}
    response.headers["Last-Modified"] = text_manager.last_modified_time.strftime(HEADER_DATE_FORMAT)
    response_object: Dict[str, Any] = {"status": "failure"}
    result, error = await text_manager.get_entries_from_dir(dir_name)
    if error is None:
        response_object["status"] = "success"
        response_object["result"] = result
    else:
        response_object["error"] = error
    LOGGER.debug(response_object)
    return response_object


@app.get("/dirs")
async def get_full_dirs(response: Response, if_modified_since: Optional[str] = Header(None)) -> Dict[str, Any]:
    LOGGER.debug(f"# get_full_dirs(if_modified_since={if_modified_since})")
    if respond_with_304_not_modified(if_modified_since):
        response.status_code = status.HTTP_304_NOT_MODIFIED
        return {}
    response.headers["Last-Modified"] = text_manager.last_modified_time.strftime(HEADER_DATE_FORMAT)
    response_object: Dict[str, Any] = {"status": "failure"}
    result, error = await text_manager.get_full_dirs()
    if error is None:
        response_object["status"] = "success"
        response_object["result"] = result
    else:
        response_object["error"] = error
    return response_object


@app.get("/somedirs")
async def get_some_dirs(response: Response, if_modified_since: Optional[str] = Header(None)) -> Dict[str, Any]:
    LOGGER.debug(f"# get_some_dirs(if_modified_since={if_modified_since})")
    if respond_with_304_not_modified(if_modified_since):
        response.status_code = status.HTTP_304_NOT_MODIFIED
        return {}
    response.headers["Last-Modified"] = text_manager.last_modified_time.strftime(HEADER_DATE_FORMAT)
    response_object: Dict[str, Any] = {"status": "failure"}
    result, error = await text_manager.get_some_entries_from_all_dirs(10)
    if error is None:
        response_object["status"] = "success"
        response_object["result"] = result
    else:
        response_object["error"] = error
    # LOGGER.debug(response_object)
    return response_object


@app.get("/topdirs")
async def get_top_dirs(response: Response, if_modified_since: Optional[str] = Header(None)) -> Dict[str, Any]:
    LOGGER.debug(f"# get_top_dir(if_modified_since={if_modified_since})")
    if respond_with_304_not_modified(if_modified_since):
        response.status_code = status.HTTP_304_NOT_MODIFIED
        return {}
    response.headers["Last-Modified"] = text_manager.last_modified_time.strftime(HEADER_DATE_FORMAT)
    response_object: Dict[str, Any] = {"status": "failure"}
    result, error = await text_manager.get_top_dirs()
    if error is None:
        response_object["status"] = "success"
        response_object["result"] = result
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
    else:
        response_object["error"] = error
    return response_object
