#!/usr/bin/env python

import sys
import os
import logging.config
from pathlib import Path
from typing import Dict, Any, Union, List
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
from backend.category_mapping import CategoryMapping
from urllib.parse import quote_plus

logging.config.fileConfig(Path(__file__).parent.parent / "logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger(__name__)

if "TM_FRONTEND_URL" not in os.environ:
    LOGGER.error("The environment variable TM_FRONTEND_URL is not set.")
    sys.exit(-1)

app = FastAPI()
LOGGER.info("app ready")
origins = [os.getenv("TM_FRONTEND_URL")]
app.add_middleware(CORSMiddleware, allow_origins=origins, allow_credentials=True, allow_methods=["*"], allow_headers=["*"],
                   expose_headers=["Accept-Ranges", "Content-Range", "Content-Length", "Content-Encoding"])
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

TM_GOOGLE_CLIENT_ID = os.getenv("TM_GOOGLE_CLIENT_ID")
TM_GOOGLE_CLIENT_SECRET = os.getenv("TM_GOOGLE_CLIENT_SECRET")

book_manager = BookManager()
print("book manager ready")

bookstore = Yes24Bookstore(base_dir=".", verbose=True)
print("bookstore ready")

category_mapping = CategoryMapping()
print("category mapping ready")


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
    result, message = await book_manager.delete_book(book_id)
    if result == "Ok":
        response_object["status"] = "success"
        response_object["result"] = result
    elif result == "Warning":
        response_object["status"] = "success"
        response_object["result"] = result
        response_object["warning"] = message
    else:
        response_object["error"] = message
    return response_object


# JSON 대신 파일 바이너리 다운로드를 위해 response_model를 None으로 지정
@app.get("/download/{book_id}/{path:path}", response_model=None)
@app.get("/download/{book_id}", response_model=None)
async def get_book_content(book_id: int) -> Union[str, FileResponse]:
    LOGGER.debug("# get_book(book_id=%d)", book_id)
    return await book_manager.get_book_content(book_id=book_id)


@app.get("/preview/{book_id}", response_model=None)
async def get_book_preview(book_id: int, pages: int = 5, chapters: int = 3):
    LOGGER.debug("# get_book_preview(book_id=%d, pages=%d, chapters=%d)", book_id, pages, chapters)
    return await book_manager.get_book_preview(book_id=book_id, pages=pages, chapters=chapters)


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


@app.get("/categories/{category:path}")
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
async def search_similar_books(book_id: int, offset: int = 0, limit: int = 10) -> Dict[str, Any]:
    LOGGER.debug("# search_similar_books(book_id=%d, offset=%d, limit=%d)", book_id, offset, limit)
    response_object: Dict[str, Any] = {"status": "failure"}
    # 우선 유사 도서 검색
    similar_list, total, error = await book_manager.search_similar_books_paged(book_id, size=limit, offset=offset)
    if similar_list and error is None:
        response_object["status"] = "success"
        response_object["result"] = [BookModel(**book.dict()) for book in similar_list]
        response_object["total"] = total
        return response_object
    # 유사 도서를 찾지 못한 경우 원본 도서 정보로 fallback
    book, err2 = await book_manager.get_book(book_id)
    if book and err2 is None:
        response_object["status"] = "success"
        response_object["result"] = [BookModel(**book.dict())]
        response_object["total"] = 1
    else:
        # 원본 도서도 조회 실패 시 error 반환
        response_object["error"] = error or err2
    return response_object


@app.get("/search/{keyword}")
async def search_by_keyword(keyword: str, offset: int = 0, limit: int = 10) -> Dict[str, Any]:
    LOGGER.debug("# search(keyword=%s, offset=%d, limit=%d)", keyword, offset, limit)
    response_object: Dict[str, Any] = {"status": "failure"}
    result, total, error = await book_manager.search_by_keyword_paged(keyword, size=limit, offset=offset)
    if error is None:
        response_object["status"] = "success"
        response_object["result"] = [BookModel(**book.dict()) for book in result]
        response_object["total"] = total
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


@app.post("/auth/google")
async def verify_google_token(request_body: dict):
    credential = request_body.get("credential")
    if not credential:
        raise HTTPException(status_code=400, detail="Credential is required")

    async with httpx.AsyncClient() as client:
        # Google ID Token 검증
        url = f"https://oauth2.googleapis.com/tokeninfo?id_token={credential}"
        response = await client.get(url)
        result = response.json()

    if "error" in result:
        LOGGER.error("Google token verification failed: %s", result.get("error_description", result.get("error")))
        raise HTTPException(status_code=401, detail="Invalid Google token")

    # Client ID 검증
    if result.get("aud") != TM_GOOGLE_CLIENT_ID:
        LOGGER.error("Google token audience mismatch: expected %s, got %s", TM_GOOGLE_CLIENT_ID, result.get("aud"))
        raise HTTPException(status_code=401, detail="Invalid token audience")

    return {
        "email": result.get("email"),
        "name": result.get("name"),
        "picture": result.get("picture"),
        "email_verified": result.get("email_verified")
    }


# === 카테고리 매핑 API ===

class CategoryKeywordsModel(BaseModel):
    keywords: List[str]


class CategoryMappingsModel(BaseModel):
    mappings: Dict[str, List[str]]


@app.get("/category-mappings")
async def get_all_category_mappings() -> Dict[str, Any]:
    """모든 카테고리-키워드 매핑 조회"""
    LOGGER.debug("# get_all_category_mappings()")
    try:
        mappings = category_mapping.get_all_mappings()
        return {"status": "success", "result": mappings}
    except Exception as e:
        LOGGER.error("get_all_category_mappings error: %s", e)
        raise HTTPException(status_code=500, detail=str(e))


@app.get("/category-mappings/{category}")
async def get_category_keywords(category: str) -> Dict[str, Any]:
    """특정 카테고리의 키워드 목록 조회"""
    LOGGER.debug("# get_category_keywords(category='%s')", category)
    try:
        keywords = category_mapping.get_keywords(category)
        return {"status": "success", "result": keywords}
    except Exception as e:
        LOGGER.error("get_category_keywords error: %s", e)
        raise HTTPException(status_code=500, detail=str(e))


@app.put("/category-mappings/{category}")
async def set_category_keywords(category: str, body: CategoryKeywordsModel) -> Dict[str, Any]:
    """카테고리의 키워드 목록 설정 (기존 대체)"""
    LOGGER.debug("# set_category_keywords(category='%s', keywords=%s)", category, body.keywords)
    try:
        success = category_mapping.set_keywords(category, body.keywords)
        if success:
            return {"status": "success", "result": category_mapping.get_keywords(category)}
        else:
            raise HTTPException(status_code=500, detail="Failed to set keywords")
    except HTTPException:
        raise
    except Exception as e:
        LOGGER.error("set_category_keywords error: %s", e)
        raise HTTPException(status_code=500, detail=str(e))


@app.post("/category-mappings/{category}/keywords")
async def add_category_keyword(category: str, body: Dict[str, str]) -> Dict[str, Any]:
    """카테고리에 키워드 추가"""
    keyword = body.get("keyword", "")
    LOGGER.debug("# add_category_keyword(category='%s', keyword='%s')", category, keyword)
    if not keyword:
        raise HTTPException(status_code=400, detail="Keyword is required")
    try:
        success = category_mapping.add_keyword(category, keyword)
        if success:
            return {"status": "success", "result": category_mapping.get_keywords(category)}
        else:
            return {"status": "duplicate", "message": "Keyword already exists", "result": category_mapping.get_keywords(category)}
    except Exception as e:
        LOGGER.error("add_category_keyword error: %s", e)
        raise HTTPException(status_code=500, detail=str(e))


@app.delete("/category-mappings/{category}/keywords/{keyword}")
async def remove_category_keyword(category: str, keyword: str) -> Dict[str, Any]:
    """카테고리에서 키워드 삭제"""
    LOGGER.debug("# remove_category_keyword(category='%s', keyword='%s')", category, keyword)
    try:
        success = category_mapping.remove_keyword(category, keyword)
        if success:
            return {"status": "success", "result": category_mapping.get_keywords(category)}
        else:
            raise HTTPException(status_code=404, detail="Keyword not found")
    except HTTPException:
        raise
    except Exception as e:
        LOGGER.error("remove_category_keyword error: %s", e)
        raise HTTPException(status_code=500, detail=str(e))


@app.delete("/category-mappings/{category}")
async def delete_category_mapping(category: str) -> Dict[str, Any]:
    """카테고리의 모든 키워드 삭제"""
    LOGGER.debug("# delete_category_mapping(category='%s')", category)
    try:
        success = category_mapping.delete_category(category)
        if success:
            return {"status": "success"}
        else:
            raise HTTPException(status_code=404, detail="Category not found")
    except HTTPException:
        raise
    except Exception as e:
        LOGGER.error("delete_category_mapping error: %s", e)
        raise HTTPException(status_code=500, detail=str(e))


@app.put("/category-mappings")
async def update_all_category_mappings(body: CategoryMappingsModel) -> Dict[str, Any]:
    """전체 매핑 일괄 업데이트"""
    LOGGER.debug("# update_all_category_mappings()")
    try:
        success = category_mapping.update_all_mappings(body.mappings)
        if success:
            return {"status": "success", "result": category_mapping.get_all_mappings()}
        else:
            raise HTTPException(status_code=500, detail="Failed to update mappings")
    except HTTPException:
        raise
    except Exception as e:
        LOGGER.error("update_all_category_mappings error: %s", e)
        raise HTTPException(status_code=500, detail=str(e))


# === 비노출 카테고리 API ===

class HiddenCategoryModel(BaseModel):
    hidden: bool


@app.get("/hidden-categories")
async def get_hidden_categories() -> Dict[str, Any]:
    """비노출 카테고리 목록 조회"""
    LOGGER.debug("# get_hidden_categories()")
    try:
        categories = category_mapping.get_hidden_categories()
        return {"status": "success", "result": categories}
    except Exception as e:
        LOGGER.error("get_hidden_categories error: %s", e)
        raise HTTPException(status_code=500, detail=str(e))


@app.get("/category-mismatches")
async def get_category_mismatches() -> Dict[str, Any]:
    """ES 카테고리별 문서 수와 파일시스템 파일 수의 불일치 검출"""
    LOGGER.debug("# get_category_mismatches()")
    response_object: Dict[str, Any] = {"status": "failure"}
    try:
        result = await book_manager.get_category_mismatches()
        response_object["status"] = "success"
        response_object["result"] = result
    except Exception as e:
        LOGGER.error("get_category_mismatches error: %s", e)
        response_object["error"] = str(e)
    return response_object


@app.post("/category-mismatches/index-file")
async def index_single_file(body: Dict[str, str]) -> Dict[str, Any]:
    """파일시스템의 파일을 ES에 적재"""
    LOGGER.debug("# index_single_file(body=%r)", body)
    file_path = body.get("file_path", "")
    if not file_path:
        raise HTTPException(status_code=400, detail="file_path is required")
    response_object: Dict[str, Any] = {"status": "failure"}
    book_id, error = await book_manager.index_single_file(file_path)
    if book_id is not None and error is None:
        response_object["status"] = "success"
        response_object["result"] = {"book_id": book_id}
    else:
        response_object["error"] = error
    return response_object


@app.post("/category-mismatches/delete-file")
async def delete_file(body: Dict[str, str]) -> Dict[str, Any]:
    """파일시스템에서 파일 삭제"""
    LOGGER.debug("# delete_file(body=%r)", body)
    file_path = body.get("file_path", "")
    if not file_path:
        raise HTTPException(status_code=400, detail="file_path is required")
    response_object: Dict[str, Any] = {"status": "failure"}
    result, error = await book_manager.delete_file(file_path)
    if result == "Ok":
        response_object["status"] = "success"
        response_object["result"] = result
    else:
        response_object["error"] = error
    return response_object


@app.get("/category-mismatches/{category:path}")
async def get_category_mismatch_details(category: str) -> Dict[str, Any]:
    """특정 카테고리의 책 수준 불일치 상세 조회"""
    LOGGER.debug("# get_category_mismatch_details(category='%s')", category)
    response_object: Dict[str, Any] = {"status": "failure"}
    try:
        result = await book_manager.get_category_mismatch_details(category)
        response_object["status"] = "success"
        response_object["result"] = result
    except Exception as e:
        LOGGER.error("get_category_mismatch_details error: %s", e)
        response_object["error"] = str(e)
    return response_object


@app.post("/hidden-categories/{category:path}")
async def set_hidden_category(category: str, body: HiddenCategoryModel) -> Dict[str, Any]:
    """카테고리 비노출 설정/해제"""
    LOGGER.debug("# set_hidden_category(category='%s', hidden=%s)", category, body.hidden)
    try:
        success = category_mapping.set_hidden(category, body.hidden)
        if success:
            return {"status": "success", "result": category_mapping.get_hidden_categories()}
        else:
            raise HTTPException(status_code=500, detail="Failed to update hidden category")
    except HTTPException:
        raise
    except Exception as e:
        LOGGER.error("set_hidden_category error: %s", e)
        raise HTTPException(status_code=500, detail=str(e))
