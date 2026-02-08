#!/usr/bin/env python

import sys
import os
import io
import base64
import logging.config
from pathlib import Path
from typing import Tuple, Dict, List, Union, Optional, Any
import chardet
from fastapi.responses import FileResponse, Response
from backend.es_manager import ESManager
from backend.book import Book

logging.config.fileConfig(Path(__file__).parent.parent / "logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger(__name__)
logging.getLogger("elasticsearch").setLevel(logging.CRITICAL)


class BookManager:
    ROOT_DIRECTORY = "$$rootdir$$"
    MEDIA_TYPES = {
        ".txt": "text/plain",
        ".pdf": "application/pdf",
        ".epub": "application/epub+zip",
        ".doc": "application/msword",
        ".docx": "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        ".html": "text/html",
        ".jpg": "image/jpeg",
        ".jpeg": "image/jpeg",
        ".png": "image/png",
        ".gif": "image/gif",
        ".webp": "image/webp",
        ".bmp": "image/bmp",
        ".tiff": "image/tiff",
        ".svg": "image/svg+xml",
    }

    def __init__(self) -> None:
        if "TM_WORK_DIR" not in os.environ:
            LOGGER.error("The environment variable TM_WORK_DIR is not set.")
            sys.exit(-1)

        self.path_prefix = Path(os.environ["TM_WORK_DIR"])
        LOGGER.debug(self.path_prefix)
        self.es_manager = ESManager()
        self.es_manager.create_index()

    def __del__(self) -> None:
        del self.es_manager

    async def get_categories(self) -> Tuple[Dict[str, int], Optional[str]]:
        LOGGER.debug("# get_categories()")
        categories = self.es_manager.search_and_aggregate_by_category()
        return categories, None

    async def get_books_in_category(self, category: str) -> Tuple[List[Book], Optional[str]]:
        doc_list = self.es_manager.search_by_category(category, max_result_count=10000)
        if doc_list and len(doc_list) > 0:
            return [Book(book_id=book_id, info=doc) for book_id, doc, _score in doc_list], None
        return [], f"No books found in '{category}'"

    async def get_book(self, book_id: int) -> Tuple[Optional[Book], Optional[str]]:
        LOGGER.debug("# get_book(book_id=%d)", book_id)
        doc = self.es_manager.search_by_id(book_id)
        if doc:
            return Book(book_id=book_id, info=doc), None
        return None, f"No book found by '{book_id}'"

    @staticmethod
    def determine_file_content_and_encoding(file_path: Path) -> str:
        LOGGER.debug("# determine_file_content_and_encoding(file_path='%s')", file_path)
        if file_path.suffix != ".txt":
            return "binary"

        encoding = "utf-8"
        with file_path.open("r") as infile:
            content = infile.read(1024 * 100)
            if content:
                encoding_metadata = chardet.detect(content.encode())
                if encoding_metadata["confidence"] > 0.99:
                    encoding = encoding_metadata["encoding"] if encoding_metadata["encoding"] else "utf-8"
        return encoding

    async def get_book_content(self, book_id: int) -> Union[str, FileResponse]:
        LOGGER.debug("# get_book_content(book_id=%d)", book_id)
        doc = self.es_manager.search_by_id(book_id)
        if not doc:
            return ""
        book = Book(book_id=book_id, info=doc)
        # book.file_path는 이미 path_prefix가 포함된 전체 경로
        if book.file_path.is_file():
            media_type = BookManager.MEDIA_TYPES.get(book.file_path.suffix, "application/octet-stream")
            # Content-Encoding: identity → GZipMiddleware가 바이너리 파일을 압축하지 않도록 하여
            # pdf.js 등의 Range 요청이 정상 동작하게 함
            return FileResponse(path=book.file_path, media_type=media_type,
                                headers={"Content-Encoding": "identity"})
        return ""

    async def get_book_preview(self, book_id: int, pages: int = 5, chapters: int = 3) -> Union[str, Response, FileResponse]:
        LOGGER.debug("# get_book_preview(book_id=%d, pages=%d, chapters=%d)", book_id, pages, chapters)
        doc = self.es_manager.search_by_id(book_id)
        if not doc:
            return ""
        book = Book(book_id=book_id, info=doc)
        if not book.file_path.is_file():
            return ""

        suffix = book.file_path.suffix.lower()
        cache_dir = self.path_prefix / ".preview_cache"
        cache_dir.mkdir(parents=True, exist_ok=True)
        original_mtime = book.file_path.stat().st_mtime

        if suffix == ".pdf":
            cache_file = cache_dir / f"{book_id}.pdf"
            if cache_file.exists() and cache_file.stat().st_mtime >= original_mtime:
                LOGGER.debug("Preview cache hit for book_id=%d (PDF)", book_id)
                return FileResponse(path=cache_file, media_type="application/pdf",
                                    headers={"Content-Encoding": "identity"})

            try:
                from pypdf import PdfReader, PdfWriter
                reader = PdfReader(str(book.file_path))
                writer = PdfWriter()
                pages_to_extract = min(len(reader.pages), pages)
                for i in range(pages_to_extract):
                    writer.add_page(reader.pages[i])
                buf = io.BytesIO()
                writer.write(buf)
                preview_bytes = buf.getvalue()
                cache_file.write_bytes(preview_bytes)
                LOGGER.debug("Preview generated for book_id=%d (PDF, %d pages)", book_id, pages_to_extract)
                return Response(content=preview_bytes, media_type="application/pdf",
                                headers={"Content-Encoding": "identity"})
            except Exception as e:
                LOGGER.error("PDF preview generation failed for book_id=%d: %s", book_id, e)
                return ""

        elif suffix == ".epub":
            cache_file = cache_dir / f"{book_id}.html"
            if cache_file.exists() and cache_file.stat().st_mtime >= original_mtime:
                LOGGER.debug("Preview cache hit for book_id=%d (EPUB)", book_id)
                return FileResponse(path=cache_file, media_type="text/html")

            try:
                import ebooklib
                from ebooklib import epub
                from bs4 import BeautifulSoup

                epub_book = epub.read_epub(str(book.file_path), options={"ignore_ncx": True})

                # 이미지를 base64로 인코딩하여 딕셔너리에 저장
                images = {}
                for item in epub_book.get_items_of_type(ebooklib.ITEM_IMAGE):
                    images[item.get_name()] = (
                        item.get_type(),
                        base64.b64encode(item.get_content()).decode("ascii")
                    )

                # CSS 수집
                css_parts = []
                for item in epub_book.get_items_of_type(ebooklib.ITEM_STYLE):
                    css_parts.append(item.get_content().decode("utf-8", errors="replace"))

                # spine 순서대로 앞 N개 챕터 HTML 추출
                spine_items = []
                for item in epub_book.get_items_of_type(ebooklib.ITEM_DOCUMENT):
                    spine_items.append(item)
                chapters_to_extract = spine_items[:chapters]

                html_parts = []
                for item in chapters_to_extract:
                    content = item.get_content().decode("utf-8", errors="replace")
                    soup = BeautifulSoup(content, "html.parser")
                    # 이미지 src를 base64 data URI로 교체
                    for img in soup.find_all("img"):
                        src = img.get("src", "")
                        # 상대 경로 정규화
                        from posixpath import normpath, join, dirname
                        item_dir = dirname(item.get_name())
                        resolved = normpath(join(item_dir, src)) if src else src
                        for img_name, (img_type, img_b64) in images.items():
                            if img_name == resolved or img_name.endswith(src):
                                # MIME 타입 추정
                                ext = Path(img_name).suffix.lower()
                                mime = {".jpg": "image/jpeg", ".jpeg": "image/jpeg", ".png": "image/png", ".gif": "image/gif", ".svg": "image/svg+xml", ".webp": "image/webp"}.get(ext, "image/png")
                                img["src"] = f"data:{mime};base64,{img_b64}"
                                break
                    html_parts.append(str(soup))

                inline_css = "\n".join(css_parts)
                full_html = f"""<!DOCTYPE html>
<html>
<head><meta charset="utf-8"><style>{inline_css}</style></head>
<body>{"".join(html_parts)}</body>
</html>"""
                cache_file.write_text(full_html, encoding="utf-8")
                LOGGER.debug("Preview generated for book_id=%d (EPUB, %d chapters)", book_id, len(chapters_to_extract))
                return Response(content=full_html, media_type="text/html")
            except Exception as e:
                LOGGER.error("EPUB preview generation failed for book_id=%d: %s", book_id, e)
                return ""

        # 지원하지 않는 형식은 빈 문자열 반환
        return ""

    async def search_by_keyword(self, keyword: str, max_result_count: int = -1) -> Tuple[List[Book], Optional[str]]:
        LOGGER.debug("# search_by_keyword(keyword='%s')", keyword)
        result_list = self.es_manager.search_by_keyword(keyword, max_result_count=max_result_count)
        if result_list and len(result_list) > 0:
            return [Book(book_id=book_id, info=doc) for book_id, doc, _score in result_list], None
        return [], "No books found"

    async def search_by_keyword_paged(self, keyword: str, size: int = 10, offset: int = 0) -> Tuple[List[Book], int, Optional[str]]:
        LOGGER.debug("# search_by_keyword_paged(keyword='%s', size=%d, offset=%d)", keyword, size, offset)
        result_list, total = self.es_manager.search_by_keyword_paged(keyword, size=size, offset=offset)
        if result_list:
            return [Book(book_id=bid, info=doc) for bid, doc, _ in result_list], total, None
        return [], total, "No books found"

    async def search_similar_books(self, book_id: int, max_result_count: int = -1) -> Tuple[List[Book], Optional[str]]:
        LOGGER.debug("# search_similar_books(book_id=%d)", book_id)
        doc = self.es_manager.search_by_id(book_id)
        if not doc:
            return [], f"No book found with id '{book_id}'"
        result_list = self.es_manager.search_similar_docs(doc["category"], doc["title"], doc["author"], doc["file_type"], doc["file_size"], doc["summary"][:3500], exclude_id=book_id, max_result_count=max_result_count)
        if result_list and len(result_list) > 0:
            return [Book(book_id=doc_id, info=similar_doc) for doc_id, similar_doc, _score in result_list], None
        return [], "No similar books found"

    async def search_similar_books_paged(self, book_id: int, size: int = 10, offset: int = 0) -> Tuple[List[Book], int, Optional[str]]:
        LOGGER.debug("# search_similar_books_paged(book_id=%d, size=%d, offset=%d)", book_id, size, offset)
        doc = self.es_manager.search_by_id(book_id)
        if not doc:
            return [], 0, f"No book found with id '{book_id}'"
        result_list, total = self.es_manager.search_similar_docs_paged(
            doc["category"], doc["title"], doc["author"],
            doc["file_type"], doc["file_size"], doc["summary"][:3500],
            exclude_id=book_id, size=size, offset=offset
        )
        if result_list:
            return [Book(book_id=did, info=sdoc) for did, sdoc, _ in result_list], total, None
        return [], total, "No similar books found"

    async def add_book(self, data: Dict[int, Dict[str, Any]]) -> Tuple[Optional[int], Optional[str]]:
        LOGGER.debug("# add_book(data='%r')", data)
        doc_id_list = self.es_manager.insert(data)
        if doc_id_list and len(doc_id_list) == 1:
            self.es_manager.refresh()  # 단일 문서 추가 후 즉시 검색 가능하도록
            return doc_id_list[0], None
        return None, f"can't add book '{data}' to ElasticSearch"

    async def update_book(self, book_id: int, new_category: str, new_title: str, new_author: str, new_path: Path, new_type: str) -> Tuple[str, Optional[str]]:
        LOGGER.debug("# update_book(book_id=%d, new_category='%s', new_title='%s', new_author='%s', new_path='%r', new_file_type='%s')", book_id, new_category, new_title, new_author, new_path, new_type)
        # rename file
        doc = self.es_manager.search_by_id(book_id)
        if doc:
            book = Book(book_id=book_id, info=doc)
            file_path = self.path_prefix / book.file_path
            # _root 카테고리는 path_prefix 바로 아래
            if new_category == "_root":
                new_full_path = self.path_prefix / (new_title + "." + new_type)
            else:
                new_full_path = self.path_prefix / new_category / (new_title + "." + new_type)
            try:
                new_full_path.parent.mkdir(parents=True, exist_ok=True)
                file_path.rename(new_full_path)
            except IOError as e:
                return "Error", f"can't move '{file_path}' to '{new_full_path}', {e}"

            # update book info in ElasticSearch
            new_relative_path = new_full_path.relative_to(self.path_prefix)
            if self.es_manager.update(book_id, category=new_category, title=new_title, author=new_author, file_path=str(new_relative_path), file_type=new_type):
                return "Ok", None
        return "Error", f"can't update book information of '{book_id}' in ElasticSearch, no such a book"

    async def get_category_mismatches(self) -> Dict[str, Any]:
        """파일시스템의 숫자_ prefix 1레벨 디렉토리 기준으로 ES와 파일 수 불일치를 검출"""
        import os as _os
        import re

        # 1. ES 카테고리별 문서 수
        es_cats = self.es_manager.search_and_aggregate_by_category()

        # 2. 파일시스템: 숫자_ prefix 1레벨 디렉토리 + 그 하위 2레벨 스캔
        base_str = str(self.path_prefix)
        fs_cats: Dict[str, int] = {}
        num_prefix = re.compile(r"^\d+_")

        def count_files(dir_path: str) -> int:
            count = 0
            try:
                with _os.scandir(dir_path) as it:
                    for entry in it:
                        if entry.is_file(follow_symlinks=False):
                            count += 1
            except (PermissionError, OSError):
                pass
            return count

        try:
            with _os.scandir(base_str) as l1_it:
                for l1 in l1_it:
                    if not l1.is_dir(follow_symlinks=False) or not num_prefix.match(l1.name):
                        continue
                    rel1 = l1.name
                    c = count_files(l1.path)
                    if c > 0:
                        fs_cats[rel1] = c
                    # 2레벨 하위 디렉토리 스캔
                    try:
                        with _os.scandir(l1.path) as l2_it:
                            for l2 in l2_it:
                                if not l2.is_dir(follow_symlinks=False) or l2.name.startswith("."):
                                    continue
                                rel2 = f"{rel1}/{l2.name}"
                                c = count_files(l2.path)
                                if c > 0:
                                    fs_cats[rel2] = c
                    except (PermissionError, OSError):
                        pass
        except (PermissionError, OSError):
            pass

        # 3. 비교 (파일시스템 기준, ES에 없는 카테고리도 포함)
        all_keys = sorted(set(list(fs_cats.keys()) + [k for k in es_cats if k.count("/") <= 1 and num_prefix.match(k.split("/")[0])]))
        mismatches = []
        es_only = []
        fs_only = []
        for key in all_keys:
            es_count = es_cats.get(key)
            fs_count = fs_cats.get(key)
            if es_count is not None and fs_count is not None:
                if es_count != fs_count:
                    mismatches.append({"category": key, "es_count": es_count, "fs_count": fs_count, "diff": es_count - fs_count})
            elif es_count is not None:
                es_only.append({"category": key, "es_count": es_count})
            else:
                fs_only.append({"category": key, "fs_count": fs_count})

        return {
            "mismatches": sorted(mismatches, key=lambda x: abs(x["diff"]), reverse=True),
            "es_only": es_only,
            "fs_only": fs_only,
        }

    async def delete_book(self, book_id: int) -> Tuple[str, Optional[str]]:
        LOGGER.debug("# delete_book(book_id=%d)", book_id)
        doc = self.es_manager.search_by_id(book_id)
        if not doc:
            return "Ok", None

        warning_message = None

        # delete file
        try:
            book = Book(book_id=book_id, info=doc)
            file_path = self.path_prefix / book.file_path
            file_path.unlink()
        except FileNotFoundError as e:
            LOGGER.warning("File already deleted for book_id=%d: %s", book_id, e)
            warning_message = f"파일이 이미 삭제되었습니다: {e}"
        except IOError as e:
            return "Error", f"can't delete a book with '{book_id}', {e}"

        # delete book info from ElasticSearch
        if self.es_manager.delete(book_id):
            if warning_message:
                return "Warning", warning_message
            return "Ok", None
        return "Error", f"can't delete book information of '{book_id}' from ElasticSearch"
