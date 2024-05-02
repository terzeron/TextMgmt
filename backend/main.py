#!/usr/bin/env python

import sys
import os
import logging.config
from pathlib import Path
from typing import Dict, Any, Union
from fastapi import FastAPI
from fastapi.middleware.cors import CORSMiddleware
from fastapi.middleware.gzip import GZipMiddleware
from fastapi.responses import FileResponse
from pydantic import BaseModel
from backend.book_manager import BookManager
from utils.loader import Loader

logging.config.fileConfig(Path(__file__).parent.parent / "logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger(__name__)

if "TM_DOMAIN" not in os.environ:
    LOGGER.error("The environment variable TM_DOMAIN is not set.")
    sys.exit(-1)

app = FastAPI()
origins = [os.environ["TM_DOMAIN"]]
app.add_middleware(CORSMiddleware, allow_origins=origins, allow_credentials=True, allow_methods=["*"], allow_headers=["*"])
app.add_middleware(GZipMiddleware, minimum_size=1000)

book_manager = BookManager()
if book_manager.es_manager.es.count(index=book_manager.es_manager.index_name)["count"] == 0:
    print("loading data...")
    data = Loader.read_files(book_manager.path_prefix)
    book_manager.es_manager.insert(data)
print("ready!")


class BookModel(BaseModel):
    book_id: int
    category: str
    title: str
    author: str
    file_path: str
    file_type: str
    file_size: int
    updated_time: str


@app.put("/books/{book_id}")
async def update_book(book_id: int, book_item: BookModel) -> Dict[str, Any]:
    LOGGER.debug("# update_book(book_id=%d, book=%r)", book_id, book_item)
    response_object: Dict[str, Any] = {"status": "failure"}
    result, error = await book_manager.update_book(book_id, new_category=book_item.category, new_title=book_item.title, new_author=book_item.author, new_path=book_manager.path_prefix / book_item.file_path, new_type=book_item.file_type)
    if error is None:
        response_object["status"] = "success"
        response_object["result"] = result
    else:
        response_object["error"] = error
    return response_object


@app.delete("/books/{book_id}")
async def delete_book(book_id: int) -> Dict[str, Any]:
    LOGGER.debug("# delete_book(book_id=%d)", book_id)
    response_object: Dict[str, Any] = {"status": "failure"}
    result, error = await book_manager.delete_book(book_id)
    if error is None:
        response_object["status"] = "success"
        response_object["result"] = result
    else:
        response_object["error"] = error
    return response_object


# JSON 대신 파일 바이너리 다운로드를 위해 response_model를 None으로 지정
@app.get("/download/{book_id}/{dir_name}/{file_name}", response_model=None)
@app.get("/download/{book_id}", response_model=None)
async def get_book_content(book_id: int, dir_name: str = "", file_name: str = "") -> Union[str, FileResponse]:
    LOGGER.debug("# get_book(book_id=%d)", book_id)
    return await book_manager.get_book_content(book_id=book_id)


@app.get("/books/{book_id}")
async def get_book(book_id: int) -> Dict[str, Any]:
    LOGGER.debug("# get_book(book_id=%d)", book_id)
    response_object: Dict[str, Any] = {"status": "failure"}
    book, error = await book_manager.get_book(book_id)
    if book and error is None:
        response_object["status"] = "success"
        response_object["result"] = BookModel(**book.dict())
    else:
        response_object["error"] = error
    # LOGGER.debug(response_object)
    return response_object


@app.get("/categories/{category}")
async def get_books_in_category(category: str) -> Dict[str, Any]:
    LOGGER.debug("# get_books_in_category(category='%s')", category)
    response_object: Dict[str, Any] = {"status": "failure"}
    result, error = await book_manager.get_books_in_category(category)
    if error is None:
        response_object["status"] = "success"
        response_object["result"] = [BookModel(**book.dict()) for book in result]
    else:
        response_object["error"] = error
    #LOGGER.debug(response_object)
    return response_object


@app.get("/categories")
async def get_categories() -> Dict[str, Any]:
    LOGGER.debug("# get_categories()")
    response_object: Dict[str, Any] = {"status": "failure"}
    result, error = await book_manager.get_categories()
    if error is None:
        response_object["status"] = "success"
        response_object["result"] = result
    else:
        response_object["error"] = error
    # LOGGER.debug(response_object)
    return response_object


@app.get("/similar/{book_id}")
async def search_similar_books(book_id: int) -> Dict[str, Any]:
    LOGGER.debug("# search_similar_books(book_id=%d)", book_id)
    response_object: Dict[str, Any] = {"status": "failure"}
    result, error = await book_manager.search_similar_books(book_id)
    if error is None:
        response_object["status"] = "success"
        response_object["result"] = [BookModel(**book.dict()) for book in result]
    else:
        response_object["error"] = error
    return response_object


@app.get("/search/{keyword}")
async def search_by_keyword(keyword: str) -> Dict[str, Any]:
    LOGGER.debug("# search(keyword=%s)", keyword)
    response_object: Dict[str, Any] = {"status": "failure"}
    result, error = await book_manager.search_by_keyword(keyword)
    if error is None:
        response_object["status"] = "success"
        response_object["result"] = [BookModel(**book.dict()) for book in result]
    else:
        response_object["error"] = error
    return response_object
