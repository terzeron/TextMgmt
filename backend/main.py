#!/usr/bin/env python

import sys
import os
import logging.config
from pathlib import Path
from typing import Dict, Any, Union
import httpx
from fastapi import FastAPI, HTTPException
from fastapi.middleware.cors import CORSMiddleware
from fastapi.middleware.gzip import GZipMiddleware
from fastapi.responses import FileResponse, Response
# 에러 및 미디어 타입 상수 정의
ERR_MISSING_INPUT = "제목 또는 저자를 입력해주세요"
JSON_MEDIA_TYPE = "application/json"
from pydantic import BaseModel
from backend.book_manager import BookManager
from backend.bookstore import Bookstore, Yes24Bookstore, AladinBookstore, RidibooksBookstore, NaverShoppingBookstore, NaverSeriesBookstore, MunpiaBookstore
from urllib.parse import quote_plus
from utils.loader import Loader

logging.config.fileConfig(Path(__file__).parent.parent / "logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger(__name__)

if "TM_FRONTEND_URL" not in os.environ:
    LOGGER.error("The environment variable TM_FRONTEND_URL is not set.")
    sys.exit(-1)
if "VITE_FACEBOOK_APP_ID" not in os.environ:
    LOGGER.error("The environment variable VITE_FACEBOOK_APP_ID is not set.")
    sys.exit(-1)
if "VITE_FACEBOOK_APP_SECRET" not in os.environ:
    LOGGER.error("The environment variable VITE_FACEBOOK_APP_SECRET is not set.")
    sys.exit(-1)

app = FastAPI()
LOGGER.info("app ready")
origins = [os.getenv("TM_FRONTEND_URL")]
app.add_middleware(CORSMiddleware, allow_origins=origins, allow_credentials=True, allow_methods=["*"], allow_headers=["*"])
app.add_middleware(GZipMiddleware, minimum_size=1000)

# JSON 응답에서 한글이 유니코드 이스케이프로 인코딩되지 않도록 설정
import json
from fastapi.responses import JSONResponse

class CustomJSONResponse(JSONResponse):
    def render(self, content) -> bytes:
        return json.dumps(content, ensure_ascii=False, separators=(',', ':'), indent=2).encode('utf-8')

# FastAPI의 기본 JSON 인코더 설정
import json
from fastapi.encoders import jsonable_encoder

# 원본 jsonable_encoder를 백업
_original_jsonable_encoder = jsonable_encoder

def custom_jsonable_encoder(obj, **kwargs):
    """한글이 유니코드 이스케이프로 인코딩되지 않도록 하는 커스텀 인코더"""
    if isinstance(obj, dict):
        return {k: custom_jsonable_encoder(v, **kwargs) for k, v in obj.items()}
    elif isinstance(obj, list):
        return [custom_jsonable_encoder(item, **kwargs) for item in obj]
    elif isinstance(obj, str):
        return obj
    else:
        return _original_jsonable_encoder(obj, **kwargs)

# FastAPI 앱에 커스텀 JSON 인코더 설정
app.json_encoder = custom_jsonable_encoder

VITE_FACEBOOK_APP_ID = os.getenv("VITE_FACEBOOK_APP_ID")
VITE_FACEBOOK_APP_SECRET = os.getenv("VITE_FACEBOOK_APP_SECRET")

book_manager = BookManager()
if book_manager.es_manager.es.count(index=book_manager.es_manager.index_name)["count"] == 0:
    print("loading data...")
    data = Loader.read_files(book_manager.path_prefix)
    book_manager.es_manager.insert(data)
print("book manager ready")

bookstore = Bookstore(base_dir=".", verbose=True)
print("bookstore ready")


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
@app.get("/download/{book_id}/{path:path}", response_model=None)
@app.get("/download/{book_id}", response_model=None)
async def get_book_content(book_id: int) -> Union[str, FileResponse]:
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


@app.get("/search/bookstore/{store_name}")
async def search_bookstore_api(store_name: str, title: str):
    """지정된 온라인 서점에서 책을 검색하여 상위 2개의 메타데이터를 반환합니다."""
    store_class = None
    if store_name.lower() == 'yes24':
        store_class = Yes24Bookstore
    elif store_name.lower() == 'aladin':
        store_class = AladinBookstore
    elif store_name.lower() == 'ridi':
        store_class = RidibooksBookstore
    elif store_name.lower() == 'naver':
        store_class = NaverShoppingBookstore
    elif store_name.lower() == 'naverseries':
        store_class = NaverSeriesBookstore
    elif store_name.lower() == 'munpia':
        store_class = MunpiaBookstore
    else:
        raise HTTPException(status_code=404, detail="Bookstore not found")

    bookstore = store_class()
    results = bookstore.search_by_keyword(title)

    # 결과가 튜플 리스트이므로 딕셔너리로 변환
    books_data = [
        {
            "title": r[0],
            "author": r[1],
            "category": r[2],
            "book_url": r[3]
        } for r in results[:5] # 상위 5개만 선택
    ]

    if not books_data:
        return {
            "status": "not_found",
            "store": store_name,
            "search_keyword": title,
            "search_url": bookstore.build_search_url(title),
            "result": []
        }

    return {
        "status": "success",
        "store": store_name,
        "search_keyword": title,
        "search_url": bookstore.build_search_url(title),
        "search_method": f"{store_name.lower()}_keyword",
        "result": books_data
    }


@app.post("/auth/facebook")
async def exchange_facebook_token(exchange_request_body: dict):
    access_token = exchange_request_body.get("accessToken")
    if not access_token:
        raise HTTPException(status_code=400, detail="Access token is required")

    async with httpx.AsyncClient() as client:
        url = "https://graph.facebook.com/v12.0/oauth/access_token"
        params = {
            "grant_type": "fb_exchange_token",
            "client_id": VITE_FACEBOOK_APP_ID,
            "client_secret": VITE_FACEBOOK_APP_SECRET,
            "fb_exchange_token": access_token
        }
        response = await client.get(url, params=params)
        result = response.json()

    if "access_token" in result:
        return {"longLivedToken": result["access_token"]}
    raise HTTPException(status_code=500, detail="Failed to get long-lived token")
