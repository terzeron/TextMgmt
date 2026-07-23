#!/usr/bin/env python

import asyncio
import re
import sys
import os
import io
import posixpath

import logging.config
import shutil
import subprocess
import tempfile
import time
from collections import OrderedDict
from pathlib import Path
from typing import Any
from urllib.parse import quote, urlparse
from fastapi.responses import FileResponse, Response
from bs4 import BeautifulSoup
from backend.es_manager import ESManager
from backend.book import Book

logging.config.fileConfig(Path(__file__).parent.parent / "logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger(__name__)
logging.getLogger("elasticsearch").setLevel(logging.CRITICAL)

# 카테고리 단위 ES 조회 상한. ES max_result_window(기본 10000)와 일치시켜
# scroll 미사용 경로를 유지하고 요청당 메모리 사용을 제한한다 (CWE-770).
MAX_CATEGORY_RESULT_COUNT = 10000


def _safe_xml_parser(recover: bool = False) -> Any:
    """XXE 방지용 lxml 파서: 외부 엔티티/DTD/네트워크 접근을 모두 비활성화한다 (CWE-611)."""
    from lxml import etree  # type: ignore[attr-defined]

    return etree.XMLParser(recover=recover, resolve_entities=False, no_network=True, load_dtd=False)


class BookManager:
    ROOT_DIRECTORY = "$$rootdir$$"
    item_class = Book
    MEDIA_TYPES = {
        ".txt": "text/plain",
        ".pdf": "application/pdf",
        ".epub": "application/epub+zip",
        ".doc": "application/msword",
        ".docx": "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        ".hwp": "application/x-hwp",
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

    CACHE_MAX_AGE_SECONDS = 86400  # 1일
    PDF_READER_CACHE_MAX = 8  # book_id 기준 PdfReader LRU 캐시 최대 개수
    _pdf_reader_cache: "OrderedDict[int, tuple[float, Any, int]]" = OrderedDict()
    HTML_VIEWER_RESOURCE_EXTENSIONS = {".css", ".jpg", ".jpeg", ".png", ".gif", ".webp", ".bmp", ".svg", ".tiff", ".ico", ".avif", ".woff", ".woff2", ".ttf", ".otf", ".eot", ".mp3", ".ogg", ".wav", ".mp4", ".webm"}
    HTML_VIEWER_CSP = "sandbox; default-src 'none'; script-src 'none'; connect-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'; img-src 'self' data: blob:; style-src 'self' 'unsafe-inline'; font-src 'self' data:; media-src 'self' blob:;"

    @classmethod
    def _html_security_headers(cls) -> dict[str, str]:
        return {"Content-Security-Policy": cls.HTML_VIEWER_CSP, "X-Content-Type-Options": "nosniff", "Referrer-Policy": "no-referrer", "Permissions-Policy": "camera=(), microphone=(), geolocation=()", "Cache-Control": "no-transform"}

    @classmethod
    def _build_html_resource_url(cls, resource_base_url: str, raw_path: str) -> str | None:
        if not raw_path:
            return None
        trimmed = raw_path.strip()
        if not trimmed:
            return None
        if trimmed.startswith("#"):
            return trimmed

        parsed = urlparse(trimmed)
        scheme = parsed.scheme.lower()
        if scheme in {"data", "blob"}:
            return trimmed
        if scheme or trimmed.startswith("//"):
            return None
        return f"{resource_base_url}?path={quote(trimmed, safe='')}"

    @classmethod
    def _sanitize_html_for_viewer(cls, html_content: str, resource_base_url: str) -> str:
        soup = BeautifulSoup(html_content, "html.parser")

        for tag in soup.find_all(["script", "iframe", "frame", "object", "embed", "form", "base"]):
            tag.decompose()

        for meta in soup.find_all("meta"):
            if meta.get("http-equiv"):
                meta.decompose()

        for tag in soup.find_all(True):
            for attr in list(tag.attrs.keys()):
                lowered = attr.lower()
                if lowered.startswith("on") or lowered in {"srcdoc", "integrity", "crossorigin", "nonce", "formaction"}:
                    del tag.attrs[attr]

            if tag.name == "link":
                rel_values = [str(rel).lower() for rel in tag.get("rel", [])]
                if "stylesheet" not in rel_values:
                    tag.decompose()
                    continue

            if tag.name == "a":
                href = tag.get("href")
                if href:
                    safe_href = cls._build_html_resource_url(resource_base_url, href)
                    if safe_href is None or not safe_href.startswith("#"):
                        tag.attrs.pop("href", None)
                    else:
                        tag["href"] = safe_href
                tag["rel"] = ["nofollow", "noopener", "noreferrer"]
                continue

            for attr_name in ("src", "href", "poster", "xlink:href"):
                attr_value = tag.get(attr_name)
                if not attr_value:
                    continue
                safe_url = cls._build_html_resource_url(resource_base_url, attr_value)
                if safe_url is None:
                    tag.attrs.pop(attr_name, None)
                else:
                    tag[attr_name] = safe_url

        return str(soup)

    @staticmethod
    def _validate_preview_epub(cache_file: Path) -> tuple[bool, str | None]:
        """생성된 미리보기 EPUB의 구조적 유효성을 검증하고 경미한 문제는 자동 수정한다."""
        import zipfile
        from lxml import etree  # type: ignore[attr-defined]
        from posixpath import normpath, join as pjoin, dirname

        opf_ns = "http://www.idpf.org/2007/opf"

        try:
            with zipfile.ZipFile(str(cache_file), "r") as zin:
                names = set(zin.namelist())

                # 1) mimetype 검증
                if "mimetype" not in names:
                    return False, "mimetype file missing"
                mt = zin.read("mimetype").decode("ascii", errors="replace").strip()
                if mt != "application/epub+zip":
                    return False, f"invalid mimetype: {mt}"

                # 2) OPF 찾기 및 파싱
                opf_path = BookManager._find_opf_path(zin)
                if not opf_path:
                    return False, "OPF file not found"
                try:
                    opf_bytes = zin.read(opf_path)
                except KeyError:
                    return False, f"OPF file missing in archive: {opf_path}"
                try:
                    opf = etree.fromstring(opf_bytes, _safe_xml_parser(recover=True))
                except Exception as e:
                    return False, f"OPF parse error: {e}"

                opf_dir = dirname(opf_path)

                # 3) manifest / spine 존재 확인
                manifest_el = opf.find(f".//{{{opf_ns}}}manifest")
                spine_el = opf.find(f".//{{{opf_ns}}}spine")
                if manifest_el is None:
                    return False, "manifest element missing"
                if spine_el is None:
                    return False, "spine element missing"

                # manifest 맵 구축
                manifest: dict[str, str] = {}  # id → href
                for item in manifest_el.findall(f"{{{opf_ns}}}item"):
                    manifest[item.get("id", "")] = item.get("href", "")

                # 4) spine itemref 검증: manifest에 없는 항목 제거
                needs_rewrite = False
                spine_refs = list(spine_el.findall(f"{{{opf_ns}}}itemref"))
                for ref in spine_refs:
                    idref = ref.get("idref", "")
                    if idref not in manifest:
                        LOGGER.warning("EPUB validate: spine idref '%s' not in manifest, removing", idref)
                        spine_el.remove(ref)
                        needs_rewrite = True

                # 5) manifest 항목의 파일이 ZIP에 존재하는지 검증 (spine 항목만)
                for ref in list(spine_el.findall(f"{{{opf_ns}}}itemref")):
                    idref = ref.get("idref", "")
                    href = manifest.get(idref, "")
                    zp = normpath(pjoin(opf_dir, href)) if opf_dir else normpath(href)
                    if zp not in names:
                        LOGGER.warning("EPUB validate: spine item '%s' (href=%s) not in ZIP, removing", idref, href)
                        spine_el.remove(ref)
                        # manifest XML 및 dict에서도 제거
                        for item in list(manifest_el.findall(f"{{{opf_ns}}}item")):
                            if item.get("id") == idref:
                                manifest_el.remove(item)
                                break
                        manifest.pop(idref, None)
                        needs_rewrite = True

                # 6) 유효한 spine 챕터 수 확인
                remaining_refs = spine_el.findall(f"{{{opf_ns}}}itemref")
                if len(remaining_refs) == 0:
                    return False, "no valid spine chapters remain"

                # 7) toc 속성이 참조하는 NCX 검증
                toc_id = spine_el.get("toc", "")
                if toc_id:
                    if toc_id not in manifest:
                        LOGGER.warning("EPUB validate: toc='%s' not in manifest, removing toc attribute", toc_id)
                        del spine_el.attrib["toc"]
                        needs_rewrite = True
                    else:
                        toc_href = manifest[toc_id]
                        toc_zp = normpath(pjoin(opf_dir, toc_href)) if opf_dir else normpath(toc_href)
                        if toc_zp not in names:
                            LOGGER.warning("EPUB validate: toc NCX '%s' not in ZIP, removing toc attribute", toc_zp)
                            del spine_el.attrib["toc"]
                            needs_rewrite = True

                # 8) 필요 시 OPF 재작성
                if needs_rewrite:
                    LOGGER.info("EPUB validate: rewriting OPF in %s", cache_file.name)
                    modified_opf = '<?xml version="1.0" encoding="UTF-8"?>\n' + etree.tostring(opf, encoding="unicode")
                    # ZIP 내 OPF만 교체 (다른 파일 보존)
                    tmp_path = cache_file.with_suffix(".tmp")
                    with zipfile.ZipFile(str(cache_file), "r") as zin_r, zipfile.ZipFile(str(tmp_path), "w", zipfile.ZIP_DEFLATED) as zout:
                        for name in zin_r.namelist():
                            if name == opf_path:
                                zout.writestr(name, modified_opf)
                            elif name == "mimetype":
                                zout.writestr(name, zin_r.read(name), compress_type=zipfile.ZIP_STORED)
                            else:
                                zout.writestr(name, zin_r.read(name))
                    tmp_path.replace(cache_file)

        except zipfile.BadZipFile:
            return False, "corrupted ZIP file"
        except Exception as e:
            return False, f"validation error: {e}"

        return True, None

    @staticmethod
    def _evict_old_cache(cache_dir: Path) -> None:
        """cache_dir 내 1일 이상 된 파일을 삭제한다."""
        try:
            cutoff = time.time() - BookManager.CACHE_MAX_AGE_SECONDS
            for f in cache_dir.iterdir():
                try:
                    if f.is_file() and f.stat().st_mtime < cutoff:
                        f.unlink()
                        LOGGER.debug("Evicted old cache file: %s", f.name)
                except Exception as e:
                    LOGGER.warning("Failed to evict cache file %s: %s", f.name, e)
        except Exception as e:
            LOGGER.warning("Cache eviction failed: %s", e)

    @staticmethod
    def _get_cached_pdf_reader(book_id: int, file_path: Path) -> tuple[Any, int]:
        """book_id별 PdfReader와 총 페이지 수를 mtime 기준으로 캐시해서 반환한다.

        같은 PDF에 대한 청크 요청마다(그리고 disk-cache-hit 시에도) 전체 파일을
        재파싱하던 비용을 제거한다. 파일이 변경되면(mtime 불일치) 캐시를 무효화하고
        다시 읽는다. LRU로 PDF_READER_CACHE_MAX 개수만 유지한다.
        """
        from pypdf import PdfReader

        mtime = file_path.stat().st_mtime
        cache = BookManager._pdf_reader_cache
        cached = cache.get(book_id)
        if cached and cached[0] == mtime:
            cache.move_to_end(book_id)
            return cached[1], cached[2]

        reader = PdfReader(str(file_path))
        total_pages = len(reader.pages)
        cache[book_id] = (mtime, reader, total_pages)
        cache.move_to_end(book_id)
        while len(cache) > BookManager.PDF_READER_CACHE_MAX:
            cache.popitem(last=False)
        return reader, total_pages

    @staticmethod
    def _find_libreoffice() -> str:
        """LibreOffice 실행 파일 경로를 반환"""
        for cmd in ["libreoffice", "soffice"]:
            path = shutil.which(cmd)
            if path:
                return path
        mac_path = "/Applications/LibreOffice.app/Contents/MacOS/soffice"
        if Path(mac_path).exists():
            return mac_path
        return "libreoffice"

    @staticmethod
    def _find_opf_path(zin) -> str:
        """ZIP 내 OPF 파일 경로를 찾는다. container.xml → regex 폴백 → 직접 탐색 순으로 시도."""
        import re
        from lxml import etree  # type: ignore[attr-defined]

        cnt_ns = "urn:oasis:names:tc:opendocument:xmlns:container"
        # 1) container.xml에서 OPF 경로 추출
        try:
            container_xml = zin.read("META-INF/container.xml")
            try:
                container = etree.fromstring(container_xml, _safe_xml_parser(recover=True))
                rootfile = container.find(f".//{{{cnt_ns}}}rootfile")
                if rootfile is not None:
                    opf_path = rootfile.get("full-path", "")
                    if opf_path:
                        return opf_path
            except Exception as e:
                LOGGER.debug("XML parsing failed, falling back to regex: %s", e)
            # XML 파싱 실패 시 regex 폴백
            m = re.search(rb'full-path=["\']([^"\']+)', container_xml)
            if m:
                return m.group(1).decode("utf-8")
        except KeyError:
            pass

        # 2) container.xml이 없거나 파싱 실패 시 ZIP 내 .opf 파일 직접 탐색
        opf_candidates = [n for n in zin.namelist() if n.lower().endswith(".opf")]
        if opf_candidates:
            return opf_candidates[0]

        return ""

    @staticmethod
    def _get_epub_total_chapters(file_path: Path) -> int:
        """EPUB 파일의 총 챕터 수(spine itemref 수)를 반환"""
        import zipfile
        from lxml import etree  # type: ignore[attr-defined]

        try:
            with zipfile.ZipFile(str(file_path), "r") as zin:
                opf_path = BookManager._find_opf_path(zin)
                if not opf_path:
                    return 0
                opf_ns = "http://www.idpf.org/2007/opf"
                opf = etree.fromstring(zin.read(opf_path), _safe_xml_parser(recover=True))
                spine_el = opf.find(f".//{{{opf_ns}}}spine")
                if spine_el is None:
                    return 0
                return len(spine_el.findall(f"{{{opf_ns}}}itemref"))
        except Exception:
            LOGGER.exception("Failed to get EPUB total chapters for '%s'", file_path)
            return 0

    @staticmethod
    def _convert_with_libreoffice(file_path: Path, output_format: str) -> str:
        """LibreOffice를 사용하여 파일을 변환하고 결과 텍스트를 반환"""
        lo_bin = BookManager._find_libreoffice()
        with tempfile.TemporaryDirectory() as tmpdir:
            proc = subprocess.run([lo_bin, "--headless", "--convert-to", output_format, "--outdir", tmpdir, str(file_path)], capture_output=True, timeout=60)
            ext = output_format.split(":")[0]
            out_file = Path(tmpdir) / (file_path.stem + "." + ext)
            if out_file.exists():
                return out_file.read_text(encoding="utf-8", errors="replace")
            # stem 불일치 시 글로브로 검색
            out_files = list(Path(tmpdir).glob(f"*.{ext}"))
            if out_files:
                return out_files[0].read_text(encoding="utf-8", errors="replace")
            # 변환 결과 없음 — 진단 로그
            all_files = list(Path(tmpdir).iterdir())
            LOGGER.error("LibreOffice produced no output: file='%s', format='%s', returncode=%d, stderr=%s, tmpdir_files=%s", file_path, output_format, proc.returncode, proc.stderr.decode("utf-8", errors="replace")[:500], [f.name for f in all_files])
        return ""

    def __init__(self) -> None:
        if "TM_BOOK_DIR" not in os.environ:
            raise RuntimeError("The environment variable TM_BOOK_DIR is not set.")

        self.path_prefix = Path(os.environ["TM_BOOK_DIR"])
        LOGGER.debug(self.path_prefix)
        self.es_manager = ESManager()
        self.es_manager.create_index()
        self._mismatch_cache: dict[str, Any] | None = None
        self._mismatch_cache_time: float = 0.0

    def __del__(self) -> None:
        if hasattr(self, "es_manager"):
            del self.es_manager

    async def get_categories(self) -> tuple[dict[str, int], str | None]:
        LOGGER.debug("# get_categories()")
        categories = self.es_manager.search_and_aggregate_by_category()
        return categories, None

    async def get_books_in_category(self, category: str) -> tuple[list[Book], str | None]:
        doc_list = self.es_manager.search_by_category(category, max_result_count=MAX_CATEGORY_RESULT_COUNT)
        if len(doc_list) >= MAX_CATEGORY_RESULT_COUNT:
            LOGGER.warning("get_books_in_category: category '%s' 결과가 상한(%d)에 도달하여 잘렸습니다.", category, MAX_CATEGORY_RESULT_COUNT)
        if doc_list and len(doc_list) > 0:
            return [self.item_class(book_id=book_id, info=doc) for book_id, doc, _score in doc_list], None
        return [], f"No books found in '{category}'"

    async def get_book(self, book_id: int) -> tuple[Book | None, str | None]:
        LOGGER.debug("# get_book(book_id=%d)", book_id)
        doc = self.es_manager.search_by_id(book_id)
        if doc:
            return self.item_class(book_id=book_id, info=doc), None
        return None, f"No book found by '{book_id}'"

    async def validate_epub(self, book_id: int) -> tuple[dict[str, Any] | None, str | None]:
        """epubcheck를 실행하여 EPUB 파일의 구조적 유효성을 검증한다."""
        import asyncio
        import json as json_mod
        import tempfile
        import os

        LOGGER.debug("# validate_epub(book_id=%d)", book_id)
        book, _ = await self.get_book(book_id)
        if not book:
            return None, f"Book not found: {book_id}"
        if book.file_type != "epub":
            return None, f"Not an EPUB file (type: {book.file_type})"
        if not book.file_path.exists():
            return None, f"File not found: {book.file_path}"

        # 임시 파일에 JSON 출력 (stdout에 상태 메시지가 섞이는 문제 방지)
        fd, json_path = tempfile.mkstemp(suffix=".json")
        os.close(fd)
        proc = None
        try:
            try:
                proc = await asyncio.create_subprocess_exec("epubcheck", str(book.file_path), "--json", json_path, stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE)
                await asyncio.wait_for(proc.communicate(), timeout=60)
            except FileNotFoundError:
                return None, "epubcheck is not installed"
            except asyncio.TimeoutError:
                if proc is not None:
                    proc.kill()
                    await proc.wait()
                return None, "epubcheck timed out (60s)"

            try:
                with open(json_path, "r", encoding="utf-8") as f:
                    data = json_mod.load(f)
            except (json_mod.JSONDecodeError, OSError) as e:
                return (None, f"Failed to parse epubcheck output (exit_code={proc.returncode}): {e}")
        finally:
            try:
                os.unlink(json_path)
            except OSError:
                pass

        # 결과 구조화
        messages = []
        for msg in data.get("messages", []):
            locations = msg.get("locations", [])
            loc = locations[0] if locations else {}
            messages.append({"severity": msg.get("severity", ""), "id": msg.get("id", ""), "message": msg.get("message", ""), "location": {"path": loc.get("path", ""), "line": loc.get("line", -1), "column": loc.get("column", -1)} if loc else None})

        # publication 메타데이터
        pub_raw = data.get("publication", {})
        publication = None
        if pub_raw:
            publication = {"title": pub_raw.get("title", ""), "creator": pub_raw.get("creator", ""), "date": pub_raw.get("date", ""), "publisher": pub_raw.get("publisher", "")}

        # 요약 카운트
        checker = data.get("checker", {})

        rel_path = str(book.file_path.relative_to(self.path_prefix))
        result = {"valid": proc.returncode == 0, "file_path": rel_path, "messages": messages, "summary": {"fatal": checker.get("nFatal", 0), "error": checker.get("nError", 0), "warning": checker.get("nWarning", 0), "usage": checker.get("nUsage", 0), "info": checker.get("nInfo", 0)}}
        if publication:
            result["publication"] = publication

        return result, None

    async def validate_pdf(self, book_id: int) -> tuple[dict[str, Any] | None, str | None]:
        """pikepdf를 사용하여 PDF 파일의 구문 유효성을 검증하고 메타데이터를 추출한다."""
        import pikepdf

        LOGGER.debug("# validate_pdf(book_id=%d)", book_id)
        book, _ = await self.get_book(book_id)
        if not book:
            return None, f"Book not found: {book_id}"
        if book.file_type != "pdf":
            return None, f"Not a PDF file (type: {book.file_type})"
        if not book.file_path.exists():
            return None, f"File not found: {book.file_path}"

        try:
            pdf = pikepdf.open(book.file_path)
        except Exception as e:
            return None, f"Failed to open PDF: {e}"

        try:
            issues = pdf.check_pdf_syntax()
            messages = [{"severity": "WARNING", "message": msg} for msg in issues]

            # 메타데이터 추출
            publication = {}
            docinfo = pdf.docinfo
            if docinfo.get("/Title"):
                publication["title"] = str(docinfo["/Title"])
            if docinfo.get("/Author"):
                publication["creator"] = str(docinfo["/Author"])
            if docinfo.get("/Producer"):
                publication["producer"] = str(docinfo["/Producer"])
            if docinfo.get("/CreationDate"):
                publication["creation_date"] = str(docinfo["/CreationDate"])
            publication["page_count"] = len(pdf.pages)
            publication["pdf_version"] = pdf.pdf_version

            rel_path = str(book.file_path.relative_to(self.path_prefix))
            result = {"valid": len(issues) == 0, "file_path": rel_path, "messages": messages, "summary": {"error": 0, "warning": len(issues)}}
            if publication:
                result["publication"] = publication

            return result, None
        finally:
            pdf.close()

    async def get_book_content(self, book_id: int) -> str | FileResponse:
        LOGGER.debug("# get_book_content(book_id=%d)", book_id)
        doc = self.es_manager.search_by_id(book_id)
        if not doc:
            return ""
        book = self.item_class(book_id=book_id, info=doc)
        # book.file_path는 이미 path_prefix가 포함된 전체 경로
        if book.file_path.is_file():
            media_type = BookManager.MEDIA_TYPES.get(book.file_path.suffix, "application/octet-stream")
            # Content-Encoding: identity → GZipMiddleware 우회
            # Cache-Control: no-transform → 외부 프록시(Traefik 등)의 응답 변환(gzip 등) 방지
            headers = {"Content-Encoding": "identity", "Cache-Control": "no-transform"}
            if book.file_path.suffix.lower() == ".html":
                return FileResponse(path=book.file_path, media_type=media_type, filename=book.file_path.name, content_disposition_type="attachment", headers=headers)
            return FileResponse(path=book.file_path, media_type=media_type, headers=headers)
        return ""

    async def get_book_preview(self, book_id: int, pages: int = 5, chapters: int = 10, resource_base_url: str = "") -> Response | FileResponse:
        LOGGER.debug("# get_book_preview(book_id=%d, pages=%d, chapters=%d, resource_base_url='%s')", book_id, pages, chapters, resource_base_url)
        doc = self.es_manager.search_by_id(book_id)
        if not doc:
            LOGGER.warning("get_book_preview: book_id=%d not found in ES", book_id)
            return Response(status_code=404, content=f"Book not found: {book_id}")
        book = self.item_class(book_id=book_id, info=doc)
        if not book.file_path.is_file():
            LOGGER.warning("get_book_preview: file not found: '%s' (book_id=%d)", book.file_path, book_id)
            return Response(status_code=404, content=f"File not found: {book.file_path}")

        suffix = book.file_path.suffix.lower()
        cache_dir = self.path_prefix / ".preview_cache"
        cache_dir.mkdir(parents=True, exist_ok=True)
        original_mtime = book.file_path.stat().st_mtime

        if suffix == ".pdf":
            cache_file = cache_dir / f"{book_id}.pdf"
            if cache_file.exists() and cache_file.stat().st_mtime >= original_mtime:
                LOGGER.debug("Preview cache hit for book_id=%d (PDF)", book_id)
                return FileResponse(path=cache_file, media_type="application/pdf", headers={"Content-Encoding": "identity", "Cache-Control": "no-transform"})

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
                BookManager._evict_old_cache(cache_dir)
                LOGGER.debug("Preview generated for book_id=%d (PDF, %d pages)", book_id, pages_to_extract)
                return FileResponse(path=cache_file, media_type="application/pdf", headers={"Content-Encoding": "identity", "Cache-Control": "no-transform"})
            except Exception as e:
                LOGGER.error("PDF preview generation failed for book_id=%d: %s", book_id, e)
                return Response(status_code=500, content="PDF preview failed")

        elif suffix == ".epub":
            total_chapters = BookManager._get_epub_total_chapters(book.file_path)
            # chapters<=0: 전체 챕터 포함 (대용량 폰트만 제거)
            if chapters <= 0:
                chapters = total_chapters
            cache_file = cache_dir / f"{book_id}_ch{chapters}.epub"
            # 구 형식 캐시 정리 (book_id.epub, book_id.html)
            for old_name in [f"{book_id}.epub", f"{book_id}.html"]:
                old_cache = cache_dir / old_name
                if old_cache.exists():
                    old_cache.unlink()
            extra_headers = {"Content-Encoding": "identity", "Cache-Control": "no-transform", "X-Total-Chapters": str(total_chapters)}
            if cache_file.exists() and cache_file.stat().st_mtime >= original_mtime:
                LOGGER.debug("Preview cache hit for book_id=%d (EPUB, ch%d)", book_id, chapters)
                return FileResponse(path=cache_file, media_type="application/epub+zip", headers=extra_headers)

            import zipfile

            try:
                import re
                from lxml import etree  # type: ignore[attr-defined]
                from bs4 import BeautifulSoup
                from posixpath import normpath, join as pjoin, dirname

                with zipfile.ZipFile(str(book.file_path), "r") as zin:
                    # OPF 경로 찾기 (container.xml → regex → 직접 탐색)
                    opf_path = BookManager._find_opf_path(zin)
                    if not opf_path:
                        LOGGER.warning("EPUB preview: OPF file not found for book_id=%d", book_id)
                        return Response(status_code=422, content="EPUB structure error: OPF file not found")
                    opf_dir = dirname(opf_path)

                    # OPF 파싱 (recover=True: 선언되지 않은 네임스페이스 프리픽스 허용)
                    opf_ns = "http://www.idpf.org/2007/opf"
                    try:
                        opf_bytes = zin.read(opf_path)
                    except KeyError:
                        LOGGER.warning("EPUB preview: OPF file missing in archive: %s (book_id=%d)", opf_path, book_id)
                        return Response(status_code=422, content=f"EPUB structure error: OPF file missing: {opf_path}")
                    # opf: 프리픽스가 선언 없이 사용된 경우 추가
                    # (lxml recover가 보존하지만 재직렬화 시 xmlns:opf 누락 → 브라우저 파싱 실패)
                    opf_text = opf_bytes.decode("utf-8", errors="replace")
                    if "opf:" in opf_text and "xmlns:opf=" not in opf_text:
                        opf_text = opf_text.replace("<package ", f'<package xmlns:opf="{opf_ns}" ', 1)
                        opf_bytes = opf_text.encode("utf-8")
                    opf = etree.fromstring(opf_bytes, _safe_xml_parser(recover=True))

                    # manifest: id → href, media-type
                    manifest: dict[str, dict[str, str]] = {}
                    href_to_id: dict[str, str] = {}
                    for item in opf.findall(f".//{{{opf_ns}}}item"):
                        item_id = item.get("id", "")
                        href = item.get("href", "")
                        manifest[item_id] = {"href": href, "media-type": item.get("media-type", "")}
                        zip_path = normpath(pjoin(opf_dir, href)) if opf_dir else normpath(href)
                        href_to_id[zip_path] = item_id

                    # spine 순서
                    spine_el = opf.find(f".//{{{opf_ns}}}spine")
                    if spine_el is None:
                        LOGGER.warning("EPUB preview: spine not found for book_id=%d, trying manifest order", book_id)
                        chapter_idrefs = [mid for mid, info in manifest.items() if info.get("media-type") == "application/xhtml+xml"][:chapters]
                        # 출력 OPF에 spine 요소 생성 (검증 통과를 위해)
                        spine_el = etree.SubElement(opf, f"{{{opf_ns}}}spine")
                        for idref in chapter_idrefs:
                            itemref = etree.SubElement(spine_el, f"{{{opf_ns}}}itemref")
                            itemref.set("idref", idref)
                        spine_refs = list(spine_el.findall(f"{{{opf_ns}}}itemref"))
                    else:
                        spine_refs = list(spine_el.findall(f"{{{opf_ns}}}itemref"))
                        # 미리보기(0 < chapters < 전체)는 넘길 수 없는 비선형(linear="no")
                        # 항목(표지·목차 등)을 세지 않고 linear 항목만 앞 N개 선택한다.
                        # 그렇지 않으면 표지/목차가 미리보기 정원을 차지해 넘길 페이지가
                        # 거의 남지 않는다. 전체보기(chapters=전체)는 모든 항목을 유지한다.
                        if 0 < chapters < len(spine_refs):
                            selected = [ref for ref in spine_refs if (ref.get("linear") or "yes") != "no"][:chapters]
                        else:
                            selected = spine_refs[:chapters]
                        chapter_idrefs = [ref.get("idref") for ref in selected if ref.get("idref") in manifest]

                    # 포함할 zip 내 파일 경로
                    files_to_include = {opf_path}
                    if "META-INF/container.xml" in zin.namelist():
                        files_to_include.add("META-INF/container.xml")
                    manifest_ids_to_keep = set(chapter_idrefs)

                    # spine toc 속성이 참조하는 NCX 파일 포함
                    toc_id = spine_el.get("toc", "") if spine_el is not None else ""
                    if toc_id and toc_id in manifest:
                        manifest_ids_to_keep.add(toc_id)
                        toc_href = manifest[toc_id]["href"]
                        toc_zp = normpath(pjoin(opf_dir, toc_href)) if opf_dir else normpath(toc_href)
                        files_to_include.add(toc_zp)

                    # 챕터 파일의 zip 경로 계산
                    chapter_zip_paths = []
                    for idref in chapter_idrefs:
                        if idref in manifest:
                            href = manifest[idref]["href"]
                            zp = normpath(pjoin(opf_dir, href)) if opf_dir else normpath(href)
                            files_to_include.add(zp)
                            chapter_zip_paths.append(zp)

                    # 챕터 HTML에서 참조 리소스 수집
                    referenced = set()
                    for zp in chapter_zip_paths:
                        try:
                            content = zin.read(zp).decode("utf-8", errors="replace")
                        except KeyError:
                            LOGGER.warning("EPUB preview: chapter file missing in archive: %s", zp)
                            continue
                        item_dir = dirname(zp)
                        soup = BeautifulSoup(content, "html.parser")
                        for img in soup.find_all("img"):
                            src = img.get("src", "")
                            if src and not src.startswith("data:"):
                                referenced.add(normpath(pjoin(item_dir, src)))
                        # SVG <image> 태그 (커버 등에서 사용)
                        for image in soup.find_all("image"):
                            href = image.get("xlink:href") or image.get("href", "")
                            if href and not href.startswith("data:"):
                                referenced.add(normpath(pjoin(item_dir, href)))
                        for link in soup.find_all("link"):
                            href_attr = link.get("href", "")
                            if href_attr:
                                referenced.add(normpath(pjoin(item_dir, href_attr)))

                    # 챕터에서 참조된 CSS만 포함 및 CSS 내 url() 참조 수집
                    css_url_pattern = re.compile(r'url\(["\']?([^"\')\s]+)["\']?\)')
                    css_refs = [r for r in referenced if r in href_to_id and "css" in manifest[href_to_id[r]].get("media-type", "")]
                    referenced -= set(css_refs)  # CSS는 별도 처리
                    for zp in css_refs:
                        item_id = href_to_id[zp]
                        files_to_include.add(zp)
                        manifest_ids_to_keep.add(item_id)
                        try:
                            css_content = zin.read(zp).decode("utf-8", errors="replace")
                            css_dir = dirname(zp)
                            for m in css_url_pattern.findall(css_content):
                                if not m.startswith("data:"):
                                    referenced.add(normpath(pjoin(css_dir, m)))
                        except KeyError:
                            LOGGER.warning("EPUB preview: CSS file missing in archive: %s", zp)

                    # 참조된 이미지/폰트 추가 (대용량 폰트 제외)
                    FONT_SIZE_LIMIT = 500 * 1024  # 500KB
                    FONT_EXTENSIONS = {".ttf", ".otf", ".woff", ".woff2"}
                    FONT_MEDIA_TYPES = {"font/ttf", "font/otf", "font/woff", "font/woff2", "application/font-ttf", "application/font-woff", "application/font-woff2", "application/x-font-ttf"}
                    for ref_path in referenced:
                        if ref_path in href_to_id:
                            item_id = href_to_id[ref_path]
                            info = manifest[item_id]
                            ext = os.path.splitext(ref_path)[1].lower()
                            if ext in FONT_EXTENSIONS or info.get("media-type", "") in FONT_MEDIA_TYPES:
                                try:
                                    font_size = zin.getinfo(ref_path).file_size
                                    if font_size > FONT_SIZE_LIMIT:
                                        LOGGER.debug("EPUB preview: skipping large font %s (%d bytes)", ref_path, font_size)
                                        continue
                                except KeyError:
                                    LOGGER.warning("EPUB preview: font file missing in archive: %s", ref_path)
                                    continue
                            files_to_include.add(ref_path)
                            manifest_ids_to_keep.add(item_id)

                    # OPF 수정: manifest에서 불필요한 항목 제거
                    manifest_el = opf.find(f".//{{{opf_ns}}}manifest")
                    if manifest_el is not None:
                        for item in list(manifest_el.findall(f"{{{opf_ns}}}item")):
                            if item.get("id") not in manifest_ids_to_keep:
                                manifest_el.remove(item)

                    # spine에서 불필요한 항목 제거
                    if spine_el is not None:
                        for ref in list(spine_refs):
                            if ref.get("idref") not in chapter_idrefs:
                                spine_el.remove(ref)

                    # guide에서 존재하지 않는 파일 참조 제거
                    guide_el = opf.find(f".//{{{opf_ns}}}guide")
                    if guide_el is not None:
                        for ref in list(guide_el.findall(f"{{{opf_ns}}}reference")):
                            href = ref.get("href", "").split("#")[0]
                            ref_zp = normpath(pjoin(opf_dir, href)) if opf_dir else normpath(href)
                            if ref_zp not in files_to_include:
                                guide_el.remove(ref)

                    # 새 EPUB 작성
                    modified_opf = '<?xml version="1.0" encoding="UTF-8"?>\n' + etree.tostring(opf, encoding="unicode")

                    # @font-face 블록 제거용 패턴
                    font_face_pattern = re.compile(r"@font-face\s*\{[^}]*\}")

                    # NCX에서 미리보기에 포함되지 않은 파일 참조 제거
                    # (epub.js가 존재하지 않는 파일 참조로 인해 초기화 중단될 수 있음)
                    ncx_ns = "http://www.daisy.org/z3986/2005/ncx/"
                    ncx_zp = None
                    if toc_id and toc_id in manifest:
                        ncx_href = manifest[toc_id]["href"]
                        ncx_zp = normpath(pjoin(opf_dir, ncx_href)) if opf_dir else normpath(ncx_href)

                    with zipfile.ZipFile(str(cache_file), "w", zipfile.ZIP_DEFLATED) as zout:
                        zout.writestr("mimetype", "application/epub+zip", compress_type=zipfile.ZIP_STORED)
                        for zp in files_to_include:
                            if zp == opf_path:
                                zout.writestr(zp, modified_opf)
                            elif zp == ncx_zp:
                                # NCX 파일: 미리보기에 없는 파일 참조 navPoint 제거 (중첩 포함)
                                try:
                                    ncx_data = zin.read(zp)
                                    ncx_tree = etree.fromstring(ncx_data, _safe_xml_parser())
                                    ncx_dir = dirname(zp)
                                    # 모든 깊이의 navPoint를 순회하며 누락 파일 참조 제거
                                    for np in list(ncx_tree.iter(f"{{{ncx_ns}}}navPoint")):
                                        content_el = np.find(f"{{{ncx_ns}}}content")
                                        if content_el is not None:
                                            src = content_el.get("src", "")
                                            src_file = src.split("#")[0]
                                            if not src_file:
                                                continue  # fragment-only src는 유지
                                            src_zp = normpath(pjoin(ncx_dir, src_file)) if ncx_dir else normpath(src_file)
                                            if src_zp not in files_to_include:
                                                np.getparent().remove(np)
                                    ncx_out = '<?xml version="1.0" encoding="UTF-8"?>\n' + etree.tostring(ncx_tree, encoding="unicode")
                                    zout.writestr(zp, ncx_out)
                                except Exception as e:
                                    LOGGER.warning("EPUB preview: NCX filtering failed: %s", e)
                                    zout.writestr(zp, zin.read(zp))
                            else:
                                try:
                                    data = zin.read(zp)
                                    # CSS에서 제외된 폰트의 @font-face 제거
                                    if zp.endswith(".css"):
                                        css_text = data.decode("utf-8", errors="replace")
                                        css_dir = dirname(zp)

                                        def _strip_missing_font(m, _css_dir=css_dir):
                                            for url in css_url_pattern.findall(m.group()):
                                                if normpath(pjoin(_css_dir, url)) not in files_to_include:
                                                    return ""
                                            return m.group()

                                        css_text = font_face_pattern.sub(_strip_missing_font, css_text)
                                        data = css_text.encode("utf-8")
                                    zout.writestr(zp, data)
                                except KeyError:
                                    LOGGER.warning("EPUB preview: missing file in archive: %s", zp)

                # 생성된 EPUB 유효성 검증
                valid, err = BookManager._validate_preview_epub(cache_file)
                if not valid:
                    cache_file.unlink(missing_ok=True)
                    LOGGER.error("EPUB preview validation failed for book_id=%d: %s", book_id, err)
                    return Response(status_code=422, content=f"EPUB preview validation failed: {err}")

                BookManager._evict_old_cache(cache_dir)
                LOGGER.debug("Preview generated for book_id=%d (EPUB, %d chapters)", book_id, len(chapter_idrefs))
                return FileResponse(path=cache_file, media_type="application/epub+zip", headers=extra_headers)
            except zipfile.BadZipFile:
                LOGGER.exception("EPUB preview: corrupted ZIP for book_id=%d", book_id)
                return Response(status_code=422, content="EPUB file is corrupted or not a valid ZIP")
            except Exception:
                LOGGER.exception("EPUB preview generation failed for book_id=%d", book_id)
                return Response(status_code=500, content="EPUB preview failed")

        elif suffix == ".html":
            try:
                html_content = book.file_path.read_text(encoding="utf-8", errors="replace")
                sanitized = BookManager._sanitize_html_for_viewer(html_content, resource_base_url or f"/html-resource/{book_id}")
                return Response(content=sanitized, media_type="text/html", headers=BookManager._html_security_headers())
            except Exception as e:
                LOGGER.error("HTML preview generation failed for book_id=%d: %s", book_id, e)
                return Response(status_code=500, content="HTML preview failed")

        elif suffix in (".doc", ".hwp"):
            cache_file = cache_dir / f"{book_id}.html"
            if cache_file.exists() and cache_file.stat().st_mtime >= original_mtime and cache_file.stat().st_size > 0:
                LOGGER.debug("Preview cache hit for book_id=%d (%s)", book_id, suffix)
                return FileResponse(path=cache_file, media_type="text/html", headers=BookManager._html_security_headers())

            try:
                html_content = BookManager._convert_with_libreoffice(book.file_path, "html")
                if not html_content.strip() and suffix == ".hwp":
                    # LibreOffice 변환 실패 — 네이티브 HWP3 파서로 fallback
                    from utils.hwp3_parser import extract_text_from_hwp3

                    plain_text = extract_text_from_hwp3(book.file_path)
                    if plain_text:
                        import html as html_mod

                        escaped = html_mod.escape(plain_text)
                        paragraphs = "\n".join(f"<p>{line}</p>" for line in escaped.split("\n") if line.strip())
                        html_content = f'<html><body style="font-family:sans-serif;padding:1em;line-height:1.8">{paragraphs}</body></html>'
                if html_content:
                    cache_file.write_text(html_content, encoding="utf-8")
                    BookManager._evict_old_cache(cache_dir)
                    LOGGER.debug("Preview generated for book_id=%d (%s)", book_id, suffix)
                    return Response(content=html_content, media_type="text/html", headers=BookManager._html_security_headers())
                # 변환 실패 시 빈 HTML 반환 (Unsupported 에러 대신)
                return Response(content="<html><body><p>미리보기를 생성할 수 없습니다.</p></body></html>", media_type="text/html", headers=BookManager._html_security_headers())
            except Exception as e:
                LOGGER.error("%s preview generation failed for book_id=%d: %s", suffix.upper(), book_id, e)
                return Response(status_code=500, content=f"{suffix.upper()} preview failed")

        # 지원하지 않는 형식
        return Response(status_code=400, content=f"Unsupported file type: {suffix}")

    async def get_html_resource(self, book_id: int, resource_path: str) -> Response | FileResponse:
        LOGGER.debug("# get_html_resource(book_id=%d, resource_path='%s')", book_id, resource_path)
        doc = self.es_manager.search_by_id(book_id)
        if not doc:
            return Response(status_code=404, content=f"Book not found: {book_id}")
        book = self.item_class(book_id=book_id, info=doc)
        if book.file_path.suffix.lower() != ".html":
            return Response(status_code=400, content="HTML resource preview is only supported for HTML files")

        normalized = posixpath.normpath(resource_path or "")
        if normalized in {"", ".", ".."} or normalized.startswith("../"):
            return Response(status_code=400, content="Invalid resource path")

        html_dir = book.file_path.parent.resolve()
        target_path = (html_dir / normalized).resolve()
        try:
            if not target_path.is_relative_to(html_dir):
                return Response(status_code=400, content="Invalid resource path")
        except OSError:
            return Response(status_code=400, content="Invalid resource path")
        if not target_path.is_file():
            return Response(status_code=404, content=f"Resource not found: {resource_path}")
        if target_path.suffix.lower() not in BookManager.HTML_VIEWER_RESOURCE_EXTENSIONS:
            return Response(status_code=400, content="Unsupported resource type")

        media_type = BookManager.MEDIA_TYPES.get(target_path.suffix.lower(), "application/octet-stream")
        return FileResponse(path=target_path, media_type=media_type, headers=BookManager._html_security_headers())

    async def search_by_keyword(self, keyword: str, max_result_count: int = -1) -> tuple[list[Book], str | None]:
        LOGGER.debug("# search_by_keyword(keyword='%s')", keyword)
        result_list = self.es_manager.search_by_keyword(keyword, max_result_count=max_result_count)
        if result_list and len(result_list) > 0:
            return [self.item_class(book_id=book_id, info=doc) for book_id, doc, _score in result_list], None
        return [], "No books found"

    async def search_by_keyword_paged(self, keyword: str, size: int = 10, offset: int = 0, exclude_categories: list[str] | None = None) -> tuple[list[Book], int, str | None]:
        LOGGER.debug("# search_by_keyword_paged(keyword='%s', size=%d, offset=%d, exclude_categories=%s)", keyword, size, offset, exclude_categories)
        result_list, total = self.es_manager.search_by_keyword_paged(keyword, size=size, offset=offset, exclude_categories=exclude_categories)
        if result_list:
            return ([self.item_class(book_id=bid, info=doc) for bid, doc, _ in result_list], total, None)
        return [], total, None

    async def search_similar_books(self, book_id: int, max_result_count: int = -1) -> tuple[list[Book], str | None]:
        LOGGER.debug("# search_similar_books(book_id=%d)", book_id)
        doc = self.es_manager.search_by_id(book_id)
        if not doc:
            return [], f"No book found with id '{book_id}'"
        result_list = self.es_manager.search_similar_docs(doc["category"], doc["title"], doc["author"], doc["file_type"], doc["file_size"], doc["summary"][:3500], exclude_id=book_id, max_result_count=max_result_count)
        if result_list and len(result_list) > 0:
            return [self.item_class(book_id=doc_id, info=similar_doc) for doc_id, similar_doc, _score in result_list], None
        return [], "No similar books found"

    async def search_similar_books_paged(self, book_id: int, size: int = 10, offset: int = 0) -> tuple[list[Book], int, str | None]:
        LOGGER.debug("# search_similar_books_paged(book_id=%d, size=%d, offset=%d)", book_id, size, offset)
        doc = self.es_manager.search_by_id(book_id)
        if not doc:
            return [], 0, f"No book found with id '{book_id}'"
        result_list, total = self.es_manager.search_similar_docs_paged(doc["category"], doc["title"], doc["author"], doc["file_type"], doc["file_size"], doc["summary"][:3500], exclude_id=book_id, size=size, offset=offset)
        if result_list:
            return ([self.item_class(book_id=did, info=sdoc, score=score) for did, sdoc, score in result_list], total, None)
        return [], total, "No similar books found"

    async def add_book(self, data: dict[int, dict[str, Any]]) -> tuple[int | None, str | None]:
        LOGGER.debug("# add_book(data='%r')", data)
        doc_id_list = self.es_manager.insert(data)
        if doc_id_list and len(doc_id_list) == 1:
            self.es_manager.refresh()  # 단일 문서 추가 후 즉시 검색 가능하도록
            return doc_id_list[0], None
        return None, f"can't add book '{data}' to ElasticSearch"

    async def update_book(self, book_id: int, new_category: str, new_title: str, new_author: str, new_path: Path, new_type: str, force: bool = False) -> tuple[str, str | None]:
        LOGGER.debug("# update_book(book_id=%d, new_category='%s', new_title='%s', new_author='%s', new_path='%r', new_file_type='%s', force=%s)", book_id, new_category, new_title, new_author, new_path, new_type, force)
        # 경로 탈출 방지: path_prefix 외부 이동 금지
        try:
            if not new_path.resolve().is_relative_to(self.path_prefix.resolve()):
                return ("Error", "잘못된 경로입니다")
        except OSError:
            return ("Error", "잘못된 경로입니다")
        # rename file
        doc = self.es_manager.search_by_id(book_id)
        if doc:
            book = self.item_class(book_id=book_id, info=doc)
            file_path = book.file_path
            new_full_path = new_path

            # 대상 경로에 다른 파일이 이미 존재하는지 확인
            if new_full_path.exists():
                try:
                    is_same_file = file_path.exists() and file_path.samefile(new_full_path)
                except OSError:
                    is_same_file = False
                if not is_same_file:
                    if not force:
                        relative = new_full_path.relative_to(self.path_prefix)
                        return ("Error", f"CONFLICT:대상 경로에 파일이 이미 존재합니다: {relative}")
                    LOGGER.warning("update_book: force overwriting existing file '%s' (book_id=%d)", new_full_path, book_id)

            try:
                new_full_path.parent.mkdir(parents=True, exist_ok=True)
                file_path.rename(new_full_path)
            except IOError as e:
                return "Error", f"can't move '{file_path}' to '{new_full_path}', {e}"

            # update book info in ElasticSearch
            new_relative_path = new_full_path.relative_to(self.path_prefix)
            try:
                if self.es_manager.update(book_id, category=new_category, title=new_title, author=new_author, file_path=str(new_relative_path), file_type=new_type):
                    return "Ok", None
                LOGGER.error("update_book: ES update failed for book_id=%d, rolling back file move", book_id)
                try:
                    new_full_path.rename(file_path)
                except OSError as rollback_err:
                    LOGGER.error("update_book: rollback failed for book_id=%d: %s", book_id, rollback_err)
                    return ("Error", f"ES 업데이트 실패, 파일 롤백도 실패: {rollback_err}")
                return ("Error", f"ES 업데이트 실패, 파일 롤백 완료: book_id={book_id}")
            except Exception as e:
                LOGGER.error("update_book: ES update exception for book_id=%d: %s, rolling back file move", book_id, e)
                try:
                    new_full_path.rename(file_path)
                except OSError as rollback_err:
                    LOGGER.error("update_book: rollback failed for book_id=%d: %s", book_id, rollback_err)
                    return ("Error", f"ES 업데이트와 파일 롤백 모두 실패: ES={e}, rollback={rollback_err}")
                return ("Error", f"ES 업데이트 예외, 파일 롤백 완료: {e}")
        return ("Error", f"can't update book information of '{book_id}' in ElasticSearch, no such a book")

    def get_category_mismatches(self) -> dict[str, Any]:
        """파일시스템의 1레벨 디렉토리 기준으로 ES와 파일 경로 불일치를 검출"""
        import time as _time
        import os as _os
        from concurrent.futures import ThreadPoolExecutor

        # TTL 캐시 (5분)
        now = _time.monotonic()
        if self._mismatch_cache is not None and (now - self._mismatch_cache_time) < 300:
            return self._mismatch_cache

        # 1. ES: terms aggregation으로 카테고리별 문서 수 조회 (scroll 대비 수십 배 빠름)
        es_cats = self.es_manager.search_and_aggregate_by_category()

        # 2. 파일시스템: 1레벨 디렉토리 + 그 하위 2레벨 스캔 (파일 수만 카운트)
        base_str = str(self.path_prefix)
        fs_cats: dict[str, int] = {}

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
            # 최상위 디렉토리의 파일을 _root 카테고리로 카운트
            root_count = count_files(base_str)
            if root_count > 0:
                fs_cats["_root"] = root_count

            # L1/L2 디렉토리 목록 수집
            scan_tasks: list[tuple[str, str]] = []  # (dir_path, category)
            with _os.scandir(base_str) as l1_it:
                for l1 in l1_it:
                    if not l1.is_dir(follow_symlinks=False) or l1.name.startswith("."):
                        continue
                    rel1 = l1.name
                    scan_tasks.append((l1.path, rel1))
                    try:
                        with _os.scandir(l1.path) as l2_it:
                            for l2 in l2_it:
                                if not l2.is_dir(follow_symlinks=False) or l2.name.startswith("."):
                                    continue
                                scan_tasks.append((l2.path, f"{rel1}/{l2.name}"))
                    except (PermissionError, OSError):
                        pass

            # 병렬 FS 스캔
            with ThreadPoolExecutor() as executor:
                futures = {executor.submit(count_files, dp): cat for dp, cat in scan_tasks}
                for future in futures:
                    cat = futures[future]
                    count = future.result()
                    if count > 0:
                        fs_cats[cat] = count
        except (PermissionError, OSError):
            pass

        # 3. 비교 (건수 기반 비교 — 상세 경로 비교는 detail API에서 lazy 수행)
        all_keys = sorted(set(list(fs_cats.keys()) + [k for k in es_cats if k.count("/") <= 1 and not k.startswith(".")]))
        mismatches = []
        es_only = []
        fs_only = []
        for key in all_keys:
            es_count = es_cats.get(key)
            fs_count = fs_cats.get(key)
            if es_count is not None and fs_count is not None:
                diff = abs(es_count - fs_count)
                if diff > 0:
                    mismatches.append({"category": key, "es_count": es_count, "fs_count": fs_count, "diff": diff})
            elif es_count is not None:
                es_only.append({"category": key, "es_count": es_count})
            elif fs_count is not None:
                fs_only.append({"category": key, "fs_count": fs_count})

        result = {"mismatches": sorted(mismatches, key=lambda x: abs(x["diff"]), reverse=True), "es_only": es_only, "fs_only": fs_only}
        self._mismatch_cache = result
        self._mismatch_cache_time = now
        return result

    def get_category_mismatch_details(self, category: str) -> dict[str, Any]:
        """특정 카테고리의 ES 문서와 파일시스템 파일을 비교하여 불일치 항목을 반환"""
        import os as _os

        # 1. ES 문서 목록 (file_path 기준, 중복 감지 포함)
        doc_list = self.es_manager.search_by_category(category, max_result_count=MAX_CATEGORY_RESULT_COUNT)
        if len(doc_list) >= MAX_CATEGORY_RESULT_COUNT:
            LOGGER.warning("get_category_mismatch_details: category '%s' 결과가 상한(%d)에 도달하여 잘렸습니다.", category, MAX_CATEGORY_RESULT_COUNT)
        es_files: dict[str, dict[str, Any]] = {}
        path_docs: dict[str, list[dict[str, Any]]] = {}
        for book_id, doc, _score in doc_list:
            rel_path = doc.get("file_path", "")
            path_docs.setdefault(rel_path, []).append({"book_id": book_id, **doc})
            es_files[rel_path] = {"book_id": book_id, **doc}

        # 2. 파일시스템 파일 목록
        fs_files: set = set()
        if category == "_root":
            cat_dir = self.path_prefix
        else:
            cat_dir = self.path_prefix / category
        try:
            with _os.scandir(str(cat_dir)) as it:
                for entry in it:
                    if entry.is_file(follow_symlinks=False):
                        if category == "_root":
                            rel_path = entry.name
                        else:
                            rel_path = f"{category}/{entry.name}"
                        fs_files.add(rel_path)
        except (PermissionError, OSError):
            pass

        # 3. 비교
        es_paths = set(es_files.keys())
        es_only = []
        for path in sorted(es_paths - fs_files):
            info = es_files[path]
            es_only.append({"book_id": info["book_id"], "title": info.get("title", ""), "file_type": info.get("file_type", ""), "file_path": path})

        fs_only = []
        for path in sorted(fs_files - es_paths):
            name = path.rsplit("/", 1)[-1] if "/" in path else path
            fs_only.append({"file_name": name, "file_path": path})

        # 4. 중복 ES 문서 감지 (동일 file_path에 여러 book_id)
        duplicates = []
        for path in sorted(path_docs.keys()):
            docs = path_docs[path]
            if len(docs) <= 1:
                continue
            file_exists = path in fs_files
            # 파일의 실제 inode를 조회하여 ES doc _id(=inode)와 비교
            linked_id = None
            if file_exists:
                try:
                    abs_path = self.path_prefix / path
                    linked_id = _os.stat(str(abs_path)).st_ino
                except OSError:
                    pass
            dup_docs = []
            for d in docs:
                dup_docs.append({"book_id": d["book_id"], "title": d.get("title", ""), "author": d.get("author", ""), "file_type": d.get("file_type", ""), "file_linked": d["book_id"] == linked_id})
            duplicates.append({"file_path": path, "file_exists": file_exists, "docs": dup_docs})

        return {"es_only": es_only, "fs_only": fs_only, "duplicates": duplicates, "fs_count": len(fs_files)}

    async def index_single_file(self, file_path: str) -> tuple[int | None, str | None]:
        """파일시스템의 파일을 읽어 ES에 적재"""
        from utils.loader import Loader

        abs_path = (self.path_prefix / file_path).resolve()
        if not abs_path.is_relative_to(self.path_prefix.resolve()):
            return None, "잘못된 경로입니다"
        if not abs_path.is_file():
            return None, f"파일을 찾을 수 없습니다: {file_path}"
        data = Loader.read_file(abs_path)
        if not data:
            return None, f"지원하지 않는 파일 형식입니다: {file_path}"
        return await self.add_book(data)

    async def delete_file(self, file_path: str) -> tuple[str, str | None]:
        """파일시스템에서 파일을 삭제"""
        abs_path = (self.path_prefix / file_path).resolve()
        if not abs_path.is_relative_to(self.path_prefix.resolve()):
            return "Error", "잘못된 경로입니다"
        if not abs_path.is_file():
            return "Error", f"파일을 찾을 수 없습니다: {file_path}"
        try:
            abs_path.unlink()
            return "Ok", None
        except IOError as e:
            return "Error", f"파일 삭제 실패: {e}"

    async def reload_category(self, category: str, content_type: str = "book") -> tuple[dict[str, Any], str | None]:
        """카테고리 전체를 ES에 재적재 (loader.py --recursive --reload 호출)"""
        LOGGER.info("reload_category 시작: category='%s', content_type='%s'", category, content_type)

        if not category:
            return {}, "카테고리 이름이 비어있습니다"
        if ".." in category:
            return {}, "카테고리 이름에 '..'는 사용할 수 없습니다"

        abs_dir = (self.path_prefix / category).resolve()
        if not abs_dir.is_relative_to(self.path_prefix.resolve()):
            return {}, f"잘못된 경로입니다: {category}"
        if not abs_dir.is_dir():
            LOGGER.error("reload_category: 디렉토리 없음 — %s", abs_dir)
            return {}, f"디렉토리를 찾을 수 없습니다: {category}"

        index_name = "comics" if content_type == "comic" else "book"
        loader_path = str(Path(__file__).parent.parent / "utils" / "loader.py")
        LOGGER.info("reload_category: loader 실행 — index=%s, path=%s", index_name, abs_dir)

        try:
            proc = await asyncio.create_subprocess_exec(sys.executable, loader_path, "--recursive", "--reload", index_name, str(abs_dir), stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE)
            stdout, stderr = await asyncio.wait_for(proc.communicate(), timeout=600)
        except asyncio.TimeoutError:
            LOGGER.error("reload_category: 타임아웃 (10분 초과) — category='%s'", category)
            return {}, "재적재 시간이 초과되었습니다 (10분)"
        except Exception as e:
            LOGGER.error("reload_category: subprocess 실행 실패 — %s", e)
            return {}, f"재적재 실행 실패: {e}"

        stdout_text = stdout.decode("utf-8", errors="replace")
        stderr_text = stderr.decode("utf-8", errors="replace").strip()

        if proc.returncode != 0:
            LOGGER.error("reload_category: 실패 (exit %d) — stderr: %s", proc.returncode, stderr_text)
            return {}, f"재적재 실패 (exit {proc.returncode}): {stderr_text}"

        if stderr_text:
            LOGGER.warning("reload_category: stderr 출력 — %s", stderr_text)

        processed = 0
        match = re.search(r"총\s+(\d+)개\s+파일\s+처리됨", stdout_text)
        if match:
            processed = int(match.group(1))

        LOGGER.info("reload_category 완료: category='%s', processed=%d", category, processed)
        return {"category": category, "processed_count": processed}, None

    async def get_pdf_pages(self, book_id: int, start: int, end: int) -> Response:
        """PDF에서 start~end 페이지만 추출하여 Response로 반환 (1-based)"""
        LOGGER.debug("# get_pdf_pages(book_id=%d, start=%d, end=%d)", book_id, start, end)
        doc = self.es_manager.search_by_id(book_id)
        if not doc:
            return Response(status_code=404, content=f"Book not found: {book_id}")
        book = self.item_class(book_id=book_id, info=doc)
        if not book.file_path.is_file():
            return Response(status_code=404, content=f"File not found: {book.file_path}")
        if book.file_path.suffix.lower() != ".pdf":
            return Response(status_code=400, content=f"Not a PDF file: {book.file_path}")

        try:
            from pypdf import PdfWriter

            reader, total_pages = BookManager._get_cached_pdf_reader(book_id, book.file_path)

            # 범위 검증
            start = max(1, start)
            end = min(end, total_pages)
            if start > total_pages:
                return Response(status_code=400, content=f"Start page {start} exceeds total pages {total_pages}")

            # 캐시 확인
            cache_dir = self.path_prefix / ".preview_cache"
            cache_dir.mkdir(parents=True, exist_ok=True)
            cache_file = cache_dir / f"{book_id}_p{start}-{end}.pdf"
            original_mtime = book.file_path.stat().st_mtime

            if cache_file.exists() and cache_file.stat().st_mtime >= original_mtime:
                LOGGER.debug("PDF pages cache hit for book_id=%d (p%d-%d)", book_id, start, end)
                return Response(content=cache_file.read_bytes(), media_type="application/pdf", headers={"Content-Encoding": "identity", "Cache-Control": "no-transform", "X-Total-Pages": str(total_pages)})

            # 페이지 추출
            writer = PdfWriter()
            for i in range(start - 1, end):
                writer.add_page(reader.pages[i])
            buf = io.BytesIO()
            writer.write(buf)
            pdf_bytes = buf.getvalue()

            # 캐시 저장
            cache_file.write_bytes(pdf_bytes)
            BookManager._evict_old_cache(cache_dir)

            LOGGER.debug("PDF pages extracted for book_id=%d (p%d-%d, total=%d)", book_id, start, end, total_pages)
            return Response(content=pdf_bytes, media_type="application/pdf", headers={"Content-Encoding": "identity", "Cache-Control": "no-transform", "X-Total-Pages": str(total_pages)})
        except Exception as e:
            LOGGER.error("PDF pages extraction failed for book_id=%d: %s", book_id, e)
            return Response(status_code=500, content="PDF pages extraction failed")

    async def rename_category(self, old_category: str, new_category: str) -> tuple[dict[str, Any], str | None]:
        """카테고리 이름을 일괄 변경 (FS + ES, 실패 시 FS 롤백)

        Returns:
            (result_dict, error_message) 튜플
        """
        LOGGER.debug("# rename_category(old='%s', new='%s')", old_category, new_category)

        # 입력 검증
        if not old_category or not new_category:
            return {}, "카테고리 이름이 비어있습니다"
        if old_category == new_category:
            return {}, "이전 카테고리와 새 카테고리가 동일합니다"
        if ".." in old_category or ".." in new_category:
            return {}, "카테고리 이름에 '..'는 사용할 수 없습니다"

        # 경로 검증 (Path Traversal 방지)
        old_dir_check = (self.path_prefix / old_category).resolve()
        new_dir_check = (self.path_prefix / new_category).resolve()
        if not old_dir_check.is_relative_to(self.path_prefix.resolve()):
            return {}, f"잘못된 경로입니다: {old_category}"
        if not new_dir_check.is_relative_to(self.path_prefix.resolve()):
            return {}, f"잘못된 경로입니다: {new_category}"

        # ES에서 old/new 카테고리 문서 수를 msearch로 한 번에 확인
        counts = self.es_manager.count_by_categories([old_category, new_category])
        old_count = counts[old_category]
        if old_count == 0:
            return {}, f"카테고리 '{old_category}'에 문서가 없습니다"

        new_count = counts[new_category]
        if new_count > 0:
            return ({}, f"대상 카테고리 '{new_category}'에 이미 {new_count}개의 문서가 존재합니다")

        # 파일시스템 디렉토리 이름 변경
        old_dir = self.path_prefix / old_category
        new_dir = self.path_prefix / new_category
        fs_renamed = False

        if old_dir.is_dir():
            if new_dir.exists():
                return {}, f"대상 디렉토리가 이미 존재합니다: {new_dir}"
            try:
                new_dir.parent.mkdir(parents=True, exist_ok=True)
                old_dir.rename(new_dir)
                fs_renamed = True
            except OSError as e:
                return {}, f"디렉토리 이름 변경 실패: {e}"
        else:
            LOGGER.warning("rename_category: 디렉토리 없음 '%s', ES만 갱신", old_dir)

        # ES update_by_query 실행
        try:
            es_result = self.es_manager.rename_category(old_category, new_category)
        except Exception as e:
            # ES 실패 시 FS 롤백
            if fs_renamed:
                try:
                    new_dir.rename(old_dir)
                    LOGGER.info("rename_category: FS 롤백 성공")
                except OSError as rollback_err:
                    LOGGER.error("rename_category: FS 롤백 실패: %s", rollback_err)
                    return ({}, f"ES 업데이트 실패: {e}. 경고: FS 롤백도 실패하여 수동 복구 필요 ('{new_dir}' → '{old_dir}')")
            return {}, f"ES 업데이트 실패: {e}"

        if es_result.get("failures"):
            # 부분 실패 시 FS 롤백
            if fs_renamed:
                try:
                    new_dir.rename(old_dir)
                    LOGGER.info("rename_category: ES 부분 실패로 FS 롤백")
                except OSError as rollback_err:
                    LOGGER.error("rename_category: FS 롤백 실패: %s", rollback_err)
                    return ({}, f"ES 부분 실패: {es_result['failures']}. 경고: FS 롤백도 실패하여 수동 복구 필요")
            return {}, f"ES 업데이트 부분 실패: {es_result['failures']}"

        return {"old_category": old_category, "new_category": new_category, "updated_count": es_result["updated"], "fs_renamed": fs_renamed}, None

    async def delete_category(self, category: str) -> tuple[dict[str, Any], str | None]:
        """카테고리의 모든 문서를 일괄 삭제 (ES + FS)

        Returns:
            (result_dict, error_message) 튜플
        """
        LOGGER.debug("# delete_category(category='%s')", category)

        # 입력 검증
        if not category:
            return {}, "카테고리 이름이 비어있습니다"
        if ".." in category:
            return {}, "카테고리 이름에 '..'는 사용할 수 없습니다"

        # 경로 검증 (Path Traversal 방지)
        dir_check = (self.path_prefix / category).resolve()
        if not dir_check.is_relative_to(self.path_prefix.resolve()):
            return {}, f"잘못된 경로입니다: {category}"

        # ES에서 문서 수 확인 (하위 카테고리 포함)
        doc_count = self.es_manager.count_by_category(category, prefix=True)
        if doc_count == 0:
            return {}, f"카테고리 '{category}'에 문서가 없습니다"

        # ES delete_by_query 실행 (하위 카테고리 포함)
        try:
            es_result = self.es_manager.delete_by_category(category, prefix=True)
        except Exception as e:
            return {}, f"ES 삭제 실패: {e}"

        if es_result.get("failures"):
            return {}, f"ES 삭제 부분 실패: {es_result['failures']}"

        result: dict[str, Any] = {"category": category, "deleted_count": es_result["deleted"]}

        return result, None

    async def delete_book(self, book_id: int) -> tuple[str, str | None]:
        LOGGER.debug("# delete_book(book_id=%d)", book_id)
        doc = self.es_manager.search_by_id(book_id)
        if not doc:
            return "Ok", None

        warning_message = None

        # delete file
        try:
            book = self.item_class(book_id=book_id, info=doc)
            file_path = book.file_path
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
        return ("Error", f"can't delete book information of '{book_id}' from ElasticSearch")
