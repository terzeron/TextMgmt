#!/usr/bin/env python

import sys
import os
import logging.config
from pathlib import Path
from typing import Dict, Any, Union, List
from fastapi import FastAPI, UploadFile, File, Form, HTTPException, Depends
from fastapi.middleware.cors import CORSMiddleware
from fastapi.middleware.gzip import GZipMiddleware
from fastapi.responses import FileResponse
from pydantic import BaseModel
from backend.book_manager import BookManager
from backend.book import Book
from utils.loader import Loader
from contextlib import contextmanager

logging.config.fileConfig(Path(__file__).parent.parent / "logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger(__name__)

if "TM_FRONTEND_URL" not in os.environ:
    LOGGER.error("The environment variable TM_FRONTEND_URL is not set.")
    sys.exit(-1)


@contextmanager
def lifespan(app: FastAPI):
    book_manager = BookManager()
    try:
        book_manager.is_healthy()
    except Exception as e:
        LOGGER.error(f"can't connect to es server")
        LOGGER.exception(e)
    yield


app = FastAPI(lifespan=lifespan)

# For test purpose
# book_manager: BookManager = BookManager()

# CORS 미들웨어 추가
app.add_middleware(
    CORSMiddleware,
    allow_origins=[os.environ["TM_FRONTEND_URL"]],
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)

class BookModel(BaseModel):
    book_id: int
    category: str
    title: str
    author: str
    file_path: str
    file_type: str
    file_size: int
    summary: str
    updated_time: str


def get_book_manager():
    return BookManager()


@app.put("/books/{book_id}")
def update_book(book_id: int, book: BookModel, book_manager: BookManager = Depends(get_book_manager)):
    result, error = book_manager.update_book(book_id, book.model_dump())
    if error:
        return {"status": "fail", "result": error}
    return {"status": "success", "result": result}


@app.delete("/books/{book_id}")
def delete_book(book_id: int, book_manager: BookManager = Depends(get_book_manager)):
    result, error = book_manager.delete_book(book_id)
    if error:
        return {"status": "fail", "result": error}
    return {"status": "success", "result": result}

@app.get("/books/{book_id}")
def get_book(book_id: int, book_manager: BookManager = Depends(get_book_manager)):
    book, error = book_manager.get_book(book_id)
    if error:
        return {"status": "fail", "result": error}
    return {"status": "success", "result": book.dict() if book else None}

@app.get("/categories")
def get_categories(book_manager: BookManager = Depends(get_book_manager)):
    categories, error = book_manager.get_categories()
    if error:
        return {"status": "fail", "result": error}
    return {"status": "success", "result": categories}

@app.get("/categories/{category_name}")
def get_books_in_category(category_name: str, page: int = 1, page_size: int = 10, book_manager: BookManager = Depends(get_book_manager)):
    books, error = book_manager.get_books_in_category(category_name, page, page_size)
    if error:
        return {"status": "fail", "result": error}
    return {"status": "success", "result": [book.dict() for book in books]}

@app.get("/search/{keyword}")
def search_by_keyword(keyword: str, page: int = 1, page_size: int = 10, book_manager: BookManager = Depends(get_book_manager)):
    books, error = book_manager.search_by_keyword(keyword, max_result_count=page * page_size)
    if error:
        return {"status": "fail", "result": error}
    return {"status": "success", "result": [book.dict() for book in books]}

@app.get("/similar/{book_id}")
def search_similar_books(book_id: int, page: int = 1, page_size: int = 10, book_manager: BookManager = Depends(get_book_manager)):
    books, error = book_manager.search_similar_books(str(book_id), max_result_count=page * page_size)
    if error:
        return {"status": "fail", "result": error}
    return {"status": "success", "result": [book.dict() for book in books]}

@app.get("/download/{book_id}")
def download_book(book_id: int, book_manager: BookManager = Depends(get_book_manager)):
    return book_manager.get_book_content(book_id)
