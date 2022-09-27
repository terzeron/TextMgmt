#!/usr/bin/env python

from fastapi import FastAPI
from fastapi.middleware.cors import CORSMiddleware
from typing import Dict, Any, Optional
import logging
from text_manager import TextManager

logging.config.fileConfig("logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger(__name__)

app = FastAPI()
origins = [
    "http://localhost:3000",
]
app.add_middleware(
    CORSMiddleware,
    allow_origins=origins,
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)

text_manager = TextManager()


@app.put("/dirs/{dir_name}/files/{file_name}/encoding/{encoding}")
async def change_encoding(dir_name: str, file_name: str, encoding: str) -> Dict[str, Any]:
    response_object: Dict[str, Any] = {"status": "failure"}
    return response_object


@app.put("/dirs/{dir_name}/files/{file_name}/move/{new_dir_name}")
async def move_file(dir_name: str, file_name: str) -> Dict[str, Any]:
    response_object: Dict[str, Any] = {"status": "failure"}
    return response_object


@app.get("/dirs/{dir_name}/files/{file_name}/similar")
async def get_similar_file_list(dir_name: str, file_name: str) -> Dict[str, Any]:
    response_object: Dict[str, Any] = {"status": "failure"}
    return response_object


@app.put("/dirs/{dir_name}/files/{file_name}")
async def rename_file(dir_name: str, file_name: str) -> Dict[str, Any]:
    response_object: Dict[str, Any] = {"status": "failure"}
    return response_object


@app.delete("/dirs/{dir_name}/files/{file_name}")
async def delete_file(dir_name: str, file_name: str) -> Dict[str, Any]:
    response_object: Dict[str, Any] = {"status": "failure"}
    return response_object


@app.get("/dirs/{dir_name}/files/{file_name}")
async def get_file(dir_name: str, file_name: str) -> Dict[str, Any]:
    LOGGER.debug(f"# get_file(dir_name={dir_name}, file_name={file_name})")
    response_object: Dict[str, Any] = {"status": "failure"}
    result, error = text_manager.get_file_info(dir_name, file_name)
    if error is None:
        response_object["status"] = "success"
        response_object["result"] = result
    else:
        response_object["error"] = error
    LOGGER.debug(response_object)
    return response_object


@app.get("/dirs")
async def get_dir(dir_name: Optional[str] = "") -> Dict[str, Any]:
    LOGGER.debug(f"# get_dir(dir_name={dir_name})")
    response_object: Dict[str, Any] = {"status": "failure"}
    result, error = text_manager.get_file_list_from_dir(dir_name)
    if error is None:
        response_object["status"] = "success"
        response_object["result"] = result
    else:
        response_object["error"] = error
    LOGGER.debug(response_object)
    return response_object


@app.post("/search/{query}")
async def search(query: str) -> Dict[str, Any]:
    response_object: Dict[str, Any] = {"status": "failure"}
    return response_object
