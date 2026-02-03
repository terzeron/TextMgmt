#!/usr/bin/env python

import sys
import os
import logging.config
from pathlib import Path
from typing import Dict, Any, Union
import httpx
from fastapi import FastAPI, HTTPException, Request
from fastapi.middleware.cors import CORSMiddleware
from fastapi.middleware.gzip import GZipMiddleware
from fastapi.responses import FileResponse, Response
from fastapi.exceptions import RequestValidationError
# 에러 및 미디어 타입 상수 정의
ERR_MISSING_INPUT = "제목 또는 저자를 입력해주세요"
JSON_MEDIA_TYPE = "application/json"
from pydantic import BaseModel
from backend.book_manager import BookManager
from backend.bookstore import Yes24Bookstore, AladinBookstore, RidibooksBookstore, NaverShoppingBookstore, NaverSeriesBookstore, MunpiaBookstore
from urllib.parse import quote_plus

logging.config.fileConfig(Path(__file__).parent.parent / "logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger(__name__)

if "TM_FRONTEND_URL" not in os.environ:
    LOGGER.error("The environment variable TM_FRONTEND_URL is not set.")
    sys.exit(-1)

app = FastAPI()
LOGGER.info("app ready")
origins = [os.getenv("TM_FRONTEND_URL")]
app.add_middleware(CORSMiddleware, allow_origins=origins, allow_credentials=True, allow_methods=["*"], allow_headers=["*"])
app.add_middleware(GZipMiddleware, minimum_size=1000)


@app.exception_handler(RequestValidationError)
async def validation_exception_handler(request: Request, exc: RequestValidationError):
    LOGGER.error("[422] %s %s", request.method, request.url.path)
    LOGGER.error("Validation error: %s", exc.errors())
    LOGGER.error("Request body: %s", exc.body)
    from fastapi.responses import JSONResponse
    return JSONResponse(
        status_code=422,
        content={"detail": exc.errors(), "body": str(exc.body)},
    )


@app.exception_handler(HTTPException)
async def http_exception_handler(request: Request, exc: HTTPException):
    LOGGER.error("[%d] %s %s - %s", exc.status_code, request.method, request.url.path, exc.detail)
    from fastapi.responses import JSONResponse
    return JSONResponse(
        status_code=exc.status_code,
        content={"detail": exc.detail},
    )


@app.exception_handler(Exception)
async def general_exception_handler(request: Request, exc: Exception):
    import traceback
    LOGGER.error("[500] %s %s", request.method, request.url.path)
    LOGGER.error("Exception: %s", str(exc))
    LOGGER.error("Traceback:\n%s", traceback.format_exc())
    from fastapi.responses import JSONResponse
    return JSONResponse(
        status_code=500,
        content={"detail": str(exc)},
    )


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

TM_FACEBOOK_APP_ID = os.getenv("TM_FACEBOOK_APP_ID")
TM_FACEBOOK_APP_SECRET = os.getenv("TM_FACEBOOK_APP_SECRET")

book_manager = BookManager()
print("book manager ready")

bookstore = Yes24Bookstore(base_dir=".", verbose=True)
print("bookstore ready")


class BookModel(BaseModel):
    book_id: int
    category: str
    title: str
    author: str
    file_path: str
    file_type: str
    file_size: int
    line_count: int = 0
    page_count: int = 0
    isbn: str = ""
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
    # 우선 유사 도서 검색
    similar_list, error = await book_manager.search_similar_books(book_id)
    if similar_list and error is None:
        response_object["status"] = "success"
        response_object["result"] = [BookModel(**book.dict()) for book in similar_list]
        return response_object
    # 유사 도서를 찾지 못한 경우 원본 도서 정보로 fallback
    book, err2 = await book_manager.get_book(book_id)
    if book and err2 is None:
        response_object["status"] = "success"
        response_object["result"] = [BookModel(**book.dict())]
    else:
        # 원본 도서도 조회 실패 시 error 반환
        response_object["error"] = error or err2
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
async def search_bookstore_api(store_name: str, title: str = "", author: str = "", isbn: str = ""):
    """
    지정된 온라인 서점에서 책을 검색하여 상위 결과의 메타데이터를 반환합니다.
    검색 우선순위: ISBN > 제목+저자 > 제목 > 저자
    """
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

    if not title and not author and not isbn:
        raise HTTPException(status_code=400, detail=ERR_MISSING_INPUT)

    bookstore = store_class()

    # 입력값 정리
    title = title.strip() if title else ""
    author = author.strip() if author else ""
    isbn = isbn.strip() if isbn else ""

    # 통합 검색 메서드 사용 - 실제 사용된 키워드와 검색 방법도 반환
    results, search_keyword, search_method = bookstore.search(isbn=isbn, title=title, author=author)

    # 결과가 튜플 리스트이므로 딕셔너리로 변환
    books_data = []
    from bs4 import BeautifulSoup
    # 상위 5개만 선택하여 isbn 필드도 포함
    for r in results[:5]:
        book_title, book_author, category, book_url, _ = r
        item = {
            "title": book_title,
            "author": book_author,
            "category": category,
            "book_url": book_url
        }
        # ISBN 추출: 캐시된 HTML에서 extract_book_info 사용
        html = bookstore._load_html_from_tmp(book_url)
        if html:
            soup = BeautifulSoup(html, 'html.parser')
            info = bookstore.extract_book_info(soup)
            if info.get('isbn'):
                item["isbn"] = info['isbn']
            # author 정보가 비어있으면 상세 페이지에서 재추출
            if not item['author'] and info.get('author'):
                item['author'] = info['author']
        books_data.append(item)

    if not books_data:
        return {
            "status": "not_found",
            "store": store_name,
            "search_keyword": search_keyword,
            "search_method": search_method,
            "search_url": bookstore.build_search_url(search_keyword) if search_keyword else "",
            "result": []
        }

    return {
        "status": "success",
        "store": store_name,
        "search_keyword": search_keyword,
        "search_method": search_method,
        "search_url": bookstore.build_search_url(search_keyword) if search_keyword else "",
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
            "client_id": TM_FACEBOOK_APP_ID,
            "client_secret": TM_FACEBOOK_APP_SECRET,
            "fb_exchange_token": access_token
        }
        response = await client.get(url, params=params)
        result = response.json()

    if "access_token" in result:
        return {"longLivedToken": result["access_token"]}
    raise HTTPException(status_code=500, detail="Failed to get long-lived token")
