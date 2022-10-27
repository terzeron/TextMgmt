#!/usr/bin/env python

import os
from typing import Dict, Any, Union
import logging
from logging import config
from text_manager import TextManager
from fastapi import FastAPI
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import FileResponse

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


@app.put("/dirs/{dir_name}/files/{file_name}/newdir/{new_dir_name}/newfile/{new_file_name}")
async def move_file(dir_name: str, file_name: str, new_dir_name: str, new_file_name: str) -> Dict[str, Any]:
    LOGGER.debug(f"# move_file(dir_name={dir_name}, file_name={file_name}, new_dir_name={new_dir_name}, new_file_name={new_file_name})")
    response_object: Dict[str, Any] = {"status": "failure"}
    result, error = text_manager.move_file(dir_name, file_name, new_dir_name, new_file_name)
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
    result, error = text_manager.delete_file(dir_name, file_name)
    if error is None:
        response_object["status"] = "success"
        response_object["result"] = result
    else:
        response_object["error"] = error
    return response_object


@app.get("/download/dirs/{dir_name}/files/{file_name}")
async def get_file_content(dir_name: str, file_name: str) -> Union[str, FileResponse]:
    LOGGER.debug(f"# get_file(dir_name={dir_name}, file_name={file_name})")
    return text_manager.get_file_content(dir_name, file_name)


@app.get("/dirs/{dir_name}/files/{file_name}")
async def get_file_info(dir_name: str, file_name: str) -> Dict[str, Any]:
    LOGGER.debug(f"# get_file(dir_name={dir_name}, file_name={file_name})")
    response_object: Dict[str, Any] = {"status": "failure"}
    result, error = text_manager.get_file_info(dir_name, file_name)
    if error is None:
        response_object["status"] = "success"
        response_object["result"] = result
    else:
        response_object["error"] = error
    # LOGGER.debug(response_object)
    return response_object


@app.get("/dirs/{dir_name}")
async def get_some_dirs(dir_name: str) -> Dict[str, Any]:
    LOGGER.debug(f"# get_some_dirs(dir_name={dir_name})")
    response_object: Dict[str, Any] = {"status": "failure"}
    result, error = text_manager.get_entries_from_dir(dir_name)
    if error is None:
        response_object["status"] = "success"
        response_object["result"] = result
    else:
        response_object["error"] = error
    LOGGER.debug(response_object)
    return response_object


@app.get("/dirs")
async def get_full_dirs() -> Dict[str, Any]:
    LOGGER.debug(f"# get_full_dirs()")
    response_object: Dict[str, Any] = {"status": "failure"}
    result, error = text_manager.get_full_dirs()
    if error is None:
        response_object["status"] = "success"
        response_object["result"] = result
    else:
        response_object["error"] = error
    return response_object


@app.get("/somedirs")
async def get_some_dirs() -> Dict[str, Any]:
    LOGGER.debug(f"# get_some_dirs()")
    response_object: Dict[str, Any] = {"status": "failure"}
    result, error = text_manager.get_some_entries_from_all_dirs(10)
    if error is None:
        response_object["status"] = "success"
        response_object["result"] = result
    else:
        response_object["error"] = error
    LOGGER.debug(response_object)
    return response_object


@app.get("/topdirs")
async def get_top_dirs() -> Dict[str, Any]:
    LOGGER.debug(f"# get_top_dir()")
    response_object: Dict[str, Any] = {"status": "failure"}
    result, error = text_manager.get_top_dirs()
    if error is None:
        response_object["status"] = "success"
        response_object["result"] = result
    else:
        response_object["error"] = error
    LOGGER.debug(response_object)
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
    result, error = text_manager.search(query)
    if error is None:
        response_object["status"] = "success"
        response_object["result"] = result
    else:
        response_object["error"] = error
    return response_object
