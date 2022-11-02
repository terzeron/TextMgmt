#!/usr/bin/env python
import os
import asyncio
import logging
from logging import config
from typing import Dict, Any, Union, Optional
from datetime import datetime
from threading import Thread
from text_manager import TextManager
from fastapi import FastAPI, Response, Header, status
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import FileResponse
from fswatch import libfswatch

logging.config.fileConfig("logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger(__name__)

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
HEADER_DATE_FORMAT = "%a, %d %b %Y %H:%M:%S GMT"
RENAMED_MASK = 0b0000000000010000
REMOVED_MASK = 0b0000000000001000
UPDATED_MASK = 0b0000000000000100
CREATED_MASK = 0b0000000000000010
last_modified_time = datetime.utcnow().strftime(HEADER_DATE_FORMAT)


def _handle_signal(signum, frame):
    LOGGER.debug(f"# _handle_signal(signum={signum}, frame={frame})")
    global fsw_session
    if libfswatch.fsw_is_running(fsw_session):
        libfswatch.fsw_stop_monitor(fsw_session)
    exit(0)


def callback(path, evt_time, flags_list):
    # LOGGER.debug(f"# callback(path={path})")
    global last_modified_time
    for flag in flags_list:
        if bool(flag & RENAMED_MASK) or bool(flag & REMOVED_MASK) or bool(flag & UPDATED_MASK) or bool(flag & CREATED_MASK):
            last_modified_time = datetime.utcnow().strftime(HEADER_DATE_FORMAT)
            LOGGER.debug(f"last_modified_time={last_modified_time}")


def _callback_wrapper(events, event_num):
    for i in range(event_num):
        flags_list = [events[i].flags[f_idx] for f_idx in range(events[i].flags_num)]
        callback(events[i].path, events[i].evt_time, flags_list)


fsw_session = libfswatch.fsw_init_session(0)
libfswatch.fsw_init_library()
libfswatch.fsw_add_path(fsw_session, str(text_manager.path_prefix).encode())
cevent_callback = libfswatch.cevent_callback(_callback_wrapper)
libfswatch.fsw_set_callback(fsw_session, cevent_callback)
thread = Thread(
    target=libfswatch.fsw_start_monitor,
    args=(fsw_session,),
    daemon=True,
)
thread.start()

asyncio.create_task(text_manager.load_initial_dir_entries())


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


@app.get("/dirs/{dir_name}/files/{file_name}")
async def get_file_info(response: Response, dir_name: str, file_name: str, if_modified_since: Optional[str] = Header(None), ) -> Dict[str, Any]:
    LOGGER.debug(f"# get_file(dir_name={dir_name}, file_name={file_name})")
    if last_modified_time and if_modified_since:
        if datetime.strptime(last_modified_time, HEADER_DATE_FORMAT) <= datetime.strptime(if_modified_since, HEADER_DATE_FORMAT):
            response.status_code = status.HTTP_304_NOT_MODIFIED
            return {}
    response.headers["Last-Modified"] = last_modified_time
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
    if last_modified_time and if_modified_since:
        if datetime.strptime(last_modified_time, HEADER_DATE_FORMAT) <= datetime.strptime(if_modified_since, HEADER_DATE_FORMAT):
            response.status_code = status.HTTP_304_NOT_MODIFIED
            return {}
    response.headers["Last-Modified"] = last_modified_time
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
    LOGGER.debug(f"# get_full_dirs(if_modified_since={str(if_modified_since)})")
    if last_modified_time and if_modified_since:
        if datetime.strptime(last_modified_time, HEADER_DATE_FORMAT) <= datetime.strptime(if_modified_since, HEADER_DATE_FORMAT):
            response.status_code = status.HTTP_304_NOT_MODIFIED
            return {}
    response.headers["Last-Modified"] = last_modified_time
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
    LOGGER.debug(f"# get_some_dirs(if_modified_since={str(if_modified_since)})")
    if last_modified_time and if_modified_since:
        if datetime.strptime(last_modified_time, HEADER_DATE_FORMAT) <= datetime.strptime(if_modified_since, HEADER_DATE_FORMAT):
            response.status_code = status.HTTP_304_NOT_MODIFIED
            return {}
    response.headers["Last-Modified"] = last_modified_time
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
    LOGGER.debug(f"# get_top_dir(if_modified_since={str(if_modified_since)})")
    if last_modified_time and if_modified_since:
        if datetime.strptime(last_modified_time, HEADER_DATE_FORMAT) <= datetime.strptime(if_modified_since, HEADER_DATE_FORMAT):
            response.status_code = status.HTTP_304_NOT_MODIFIED
            return {}
    response.headers["Last-Modified"] = last_modified_time
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
