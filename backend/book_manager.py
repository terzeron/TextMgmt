#!/usr/bin/env python

import sys
import os
import io

import logging.config
import shutil
import subprocess
import tempfile
import time
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

    @staticmethod
    def _validate_preview_epub(cache_file: Path) -> Tuple[bool, Optional[str]]:
        """생성된 미리보기 EPUB의 구조적 유효성을 검증하고 경미한 문제는 자동 수정한다."""
        import zipfile
        from lxml import etree
        from posixpath import normpath, join as pjoin, dirname

        opf_ns = 'http://www.idpf.org/2007/opf'

        try:
            with zipfile.ZipFile(str(cache_file), 'r') as zin:
                names = set(zin.namelist())

                # 1) mimetype 검증
                if 'mimetype' not in names:
                    return False, "mimetype file missing"
                mt = zin.read('mimetype').decode('ascii', errors='replace').strip()
                if mt != 'application/epub+zip':
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
                    opf = etree.fromstring(opf_bytes, etree.XMLParser(recover=True))
                except Exception as e:
                    return False, f"OPF parse error: {e}"

                opf_dir = dirname(opf_path)

                # 3) manifest / spine 존재 확인
                manifest_el = opf.find(f'.//{{{opf_ns}}}manifest')
                spine_el = opf.find(f'.//{{{opf_ns}}}spine')
                if manifest_el is None:
                    return False, "manifest element missing"
                if spine_el is None:
                    return False, "spine element missing"

                # manifest 맵 구축
                manifest: Dict[str, str] = {}  # id → href
                for item in manifest_el.findall(f'{{{opf_ns}}}item'):
                    manifest[item.get('id', '')] = item.get('href', '')

                # 4) spine itemref 검증: manifest에 없는 항목 제거
                needs_rewrite = False
                spine_refs = list(spine_el.findall(f'{{{opf_ns}}}itemref'))
                for ref in spine_refs:
                    idref = ref.get('idref', '')
                    if idref not in manifest:
                        LOGGER.warning("EPUB validate: spine idref '%s' not in manifest, removing", idref)
                        spine_el.remove(ref)
                        needs_rewrite = True

                # 5) manifest 항목의 파일이 ZIP에 존재하는지 검증 (spine 항목만)
                for ref in list(spine_el.findall(f'{{{opf_ns}}}itemref')):
                    idref = ref.get('idref', '')
                    href = manifest.get(idref, '')
                    zp = normpath(pjoin(opf_dir, href)) if opf_dir else normpath(href)
                    if zp not in names:
                        LOGGER.warning("EPUB validate: spine item '%s' (href=%s) not in ZIP, removing", idref, href)
                        spine_el.remove(ref)
                        # manifest XML 및 dict에서도 제거
                        for item in list(manifest_el.findall(f'{{{opf_ns}}}item')):
                            if item.get('id') == idref:
                                manifest_el.remove(item)
                                break
                        manifest.pop(idref, None)
                        needs_rewrite = True

                # 6) 유효한 spine 챕터 수 확인
                remaining_refs = spine_el.findall(f'{{{opf_ns}}}itemref')
                if len(remaining_refs) == 0:
                    return False, "no valid spine chapters remain"

                # 7) toc 속성이 참조하는 NCX 검증
                toc_id = spine_el.get('toc', '')
                if toc_id:
                    if toc_id not in manifest:
                        LOGGER.warning("EPUB validate: toc='%s' not in manifest, removing toc attribute", toc_id)
                        del spine_el.attrib['toc']
                        needs_rewrite = True
                    else:
                        toc_href = manifest[toc_id]
                        toc_zp = normpath(pjoin(opf_dir, toc_href)) if opf_dir else normpath(toc_href)
                        if toc_zp not in names:
                            LOGGER.warning("EPUB validate: toc NCX '%s' not in ZIP, removing toc attribute", toc_zp)
                            del spine_el.attrib['toc']
                            needs_rewrite = True

                # 8) 필요 시 OPF 재작성
                if needs_rewrite:
                    LOGGER.info("EPUB validate: rewriting OPF in %s", cache_file.name)
                    modified_opf = '<?xml version="1.0" encoding="UTF-8"?>\n' + etree.tostring(opf, encoding='unicode')
                    # ZIP 내 OPF만 교체 (다른 파일 보존)
                    tmp_path = cache_file.with_suffix('.tmp')
                    with zipfile.ZipFile(str(cache_file), 'r') as zin_r, \
                         zipfile.ZipFile(str(tmp_path), 'w', zipfile.ZIP_DEFLATED) as zout:
                        for name in zin_r.namelist():
                            if name == opf_path:
                                zout.writestr(name, modified_opf)
                            elif name == 'mimetype':
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
        from lxml import etree

        cnt_ns = 'urn:oasis:names:tc:opendocument:xmlns:container'
        # 1) container.xml에서 OPF 경로 추출
        try:
            container_xml = zin.read('META-INF/container.xml')
            try:
                container = etree.fromstring(container_xml, etree.XMLParser(recover=True))
                rootfile = container.find(f'.//{{{cnt_ns}}}rootfile')
                if rootfile is not None:
                    opf_path = rootfile.get('full-path', '')
                    if opf_path:
                        return opf_path
            except Exception:
                pass
            # XML 파싱 실패 시 regex 폴백
            m = re.search(rb'full-path=["\']([^"\']+)', container_xml)
            if m:
                return m.group(1).decode('utf-8')
        except KeyError:
            pass

        # 2) container.xml이 없거나 파싱 실패 시 ZIP 내 .opf 파일 직접 탐색
        opf_candidates = [n for n in zin.namelist() if n.lower().endswith('.opf')]
        if opf_candidates:
            return opf_candidates[0]

        return ''

    @staticmethod
    def _get_epub_total_chapters(file_path: Path) -> int:
        """EPUB 파일의 총 챕터 수(spine itemref 수)를 반환"""
        import zipfile
        from lxml import etree

        try:
            with zipfile.ZipFile(str(file_path), 'r') as zin:
                opf_path = BookManager._find_opf_path(zin)
                if not opf_path:
                    return 0
                opf_ns = 'http://www.idpf.org/2007/opf'
                opf = etree.fromstring(zin.read(opf_path), etree.XMLParser(recover=True))
                spine_el = opf.find(f'.//{{{opf_ns}}}spine')
                if spine_el is None:
                    return 0
                return len(spine_el.findall(f'{{{opf_ns}}}itemref'))
        except Exception:
            LOGGER.exception("Failed to get EPUB total chapters for '%s'", file_path)
            return 0

    @staticmethod
    def _convert_with_libreoffice(file_path: Path, output_format: str) -> str:
        """LibreOffice를 사용하여 파일을 변환하고 결과 텍스트를 반환"""
        lo_bin = BookManager._find_libreoffice()
        with tempfile.TemporaryDirectory() as tmpdir:
            proc = subprocess.run(
                [lo_bin, "--headless", "--convert-to", output_format,
                 "--outdir", tmpdir, str(file_path)],
                capture_output=True, timeout=60
            )
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
            LOGGER.error(
                "LibreOffice produced no output: file='%s', format='%s', "
                "returncode=%d, stderr=%s, tmpdir_files=%s",
                file_path, output_format, proc.returncode,
                proc.stderr.decode("utf-8", errors="replace")[:500],
                [f.name for f in all_files]
            )
        return ""

    def __init__(self) -> None:
        if "TM_BOOK_DIR" not in os.environ:
            LOGGER.error("The environment variable TM_BOOK_DIR is not set.")
            sys.exit(-1)

        self.path_prefix = Path(os.environ["TM_BOOK_DIR"])
        LOGGER.debug(self.path_prefix)
        self.es_manager = ESManager()
        self.es_manager.create_index()
        # FS 캐시 (get_category_mismatches → get_category_mismatch_details 재사용)
        self._fs_cats_cache: Optional[Dict[str, set]] = None
        self._fs_cats_cache_time: float = 0.0

    def __del__(self) -> None:
        if hasattr(self, 'es_manager'):
            del self.es_manager

    async def get_categories(self) -> Tuple[Dict[str, int], Optional[str]]:
        LOGGER.debug("# get_categories()")
        categories = self.es_manager.search_and_aggregate_by_category()
        return categories, None

    async def get_books_in_category(self, category: str) -> Tuple[List[Book], Optional[str]]:
        doc_list = self.es_manager.search_by_category(category, max_result_count=sys.maxsize)
        if doc_list and len(doc_list) > 0:
            return [self.item_class(book_id=book_id, info=doc) for book_id, doc, _score in doc_list], None
        return [], f"No books found in '{category}'"

    async def get_book(self, book_id: int) -> Tuple[Optional[Book], Optional[str]]:
        LOGGER.debug("# get_book(book_id=%d)", book_id)
        doc = self.es_manager.search_by_id(book_id)
        if doc:
            return self.item_class(book_id=book_id, info=doc), None
        return None, f"No book found by '{book_id}'"

    async def validate_epub(self, book_id: int) -> Tuple[Optional[Dict[str, Any]], Optional[str]]:
        """epubcheck를 실행하여 EPUB 파일의 구조적 유효성을 검증한다."""
        import asyncio
        import json as json_mod
        import tempfile
        import os

        LOGGER.debug("# validate_epub(book_id=%d)", book_id)
        book, error = await self.get_book(book_id)
        if not book:
            return None, f"Book not found: {book_id}"
        if book.file_type != "epub":
            return None, f"Not an EPUB file (type: {book.file_type})"
        if not book.file_path.exists():
            return None, f"File not found: {book.file_path}"

        # 임시 파일에 JSON 출력 (stdout에 상태 메시지가 섞이는 문제 방지)
        fd, json_path = tempfile.mkstemp(suffix=".json")
        os.close(fd)
        try:
            try:
                proc = await asyncio.create_subprocess_exec(
                    "epubcheck", str(book.file_path), "--json", json_path,
                    stdout=asyncio.subprocess.PIPE,
                    stderr=asyncio.subprocess.PIPE,
                )
                await asyncio.wait_for(proc.communicate(), timeout=60)
            except FileNotFoundError:
                return None, "epubcheck is not installed"
            except asyncio.TimeoutError:
                proc.kill()
                await proc.wait()
                return None, "epubcheck timed out (60s)"

            try:
                with open(json_path, "r", encoding="utf-8") as f:
                    data = json_mod.load(f)
            except (json_mod.JSONDecodeError, OSError) as e:
                return None, f"Failed to parse epubcheck output (exit_code={proc.returncode}): {e}"
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
            messages.append({
                "severity": msg.get("severity", ""),
                "id": msg.get("id", ""),
                "message": msg.get("message", ""),
                "location": {
                    "path": loc.get("path", ""),
                    "line": loc.get("line", -1),
                    "column": loc.get("column", -1),
                } if loc else None,
            })

        # publication 메타데이터
        pub_raw = data.get("publication", {})
        publication = None
        if pub_raw:
            publication = {
                "title": pub_raw.get("title", ""),
                "creator": pub_raw.get("creator", ""),
                "date": pub_raw.get("date", ""),
                "publisher": pub_raw.get("publisher", ""),
            }

        # 요약 카운트
        checker = data.get("checker", {})

        rel_path = str(book.file_path.relative_to(self.path_prefix))
        result = {
            "valid": proc.returncode == 0,
            "file_path": rel_path,
            "messages": messages,
            "summary": {
                "fatal": checker.get("nFatal", 0),
                "error": checker.get("nError", 0),
                "warning": checker.get("nWarning", 0),
                "usage": checker.get("nUsage", 0),
                "info": checker.get("nInfo", 0),
            },
        }
        if publication:
            result["publication"] = publication

        return result, None

    async def validate_pdf(self, book_id: int) -> Tuple[Optional[Dict[str, Any]], Optional[str]]:
        """pikepdf를 사용하여 PDF 파일의 구문 유효성을 검증하고 메타데이터를 추출한다."""
        import pikepdf

        LOGGER.debug("# validate_pdf(book_id=%d)", book_id)
        book, error = await self.get_book(book_id)
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
            result = {
                "valid": len(issues) == 0,
                "file_path": rel_path,
                "messages": messages,
                "summary": {
                    "error": 0,
                    "warning": len(issues),
                },
            }
            if publication:
                result["publication"] = publication

            return result, None
        finally:
            pdf.close()

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
        book = self.item_class(book_id=book_id, info=doc)
        # book.file_path는 이미 path_prefix가 포함된 전체 경로
        if book.file_path.is_file():
            media_type = BookManager.MEDIA_TYPES.get(book.file_path.suffix, "application/octet-stream")
            # Content-Encoding: identity → GZipMiddleware 우회
            # Cache-Control: no-transform → 외부 프록시(Traefik 등)의 응답 변환(gzip 등) 방지
            return FileResponse(path=book.file_path, media_type=media_type,
                                headers={"Content-Encoding": "identity",
                                         "Cache-Control": "no-transform"})
        return ""

    async def get_book_preview(self, book_id: int, pages: int = 5, chapters: int = 3) -> Union[Response, FileResponse]:
        LOGGER.debug("# get_book_preview(book_id=%d, pages=%d, chapters=%d)", book_id, pages, chapters)
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
                return FileResponse(path=cache_file, media_type="application/pdf",
                                    headers={"Content-Encoding": "identity",
                                             "Cache-Control": "no-transform"})

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
                return FileResponse(path=cache_file, media_type="application/pdf",
                                    headers={"Content-Encoding": "identity",
                                             "Cache-Control": "no-transform"})
            except Exception as e:
                LOGGER.error("PDF preview generation failed for book_id=%d: %s", book_id, e)
                return Response(status_code=500, content=f"PDF preview failed: {e}")

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
            extra_headers = {
                "Content-Encoding": "identity",
                "Cache-Control": "no-transform",
                "X-Total-Chapters": str(total_chapters),
            }
            if cache_file.exists() and cache_file.stat().st_mtime >= original_mtime:
                LOGGER.debug("Preview cache hit for book_id=%d (EPUB, ch%d)", book_id, chapters)
                return FileResponse(path=cache_file, media_type="application/epub+zip",
                                    headers=extra_headers)

            import zipfile
            try:
                import re
                from lxml import etree
                from bs4 import BeautifulSoup
                from posixpath import normpath, join as pjoin, dirname

                with zipfile.ZipFile(str(book.file_path), 'r') as zin:
                    # OPF 경로 찾기 (container.xml → regex → 직접 탐색)
                    opf_path = BookManager._find_opf_path(zin)
                    if not opf_path:
                        LOGGER.warning("EPUB preview: OPF file not found for book_id=%d", book_id)
                        return Response(status_code=422,
                                        content="EPUB structure error: OPF file not found")
                    opf_dir = dirname(opf_path)

                    # OPF 파싱 (recover=True: 선언되지 않은 네임스페이스 프리픽스 허용)
                    opf_ns = 'http://www.idpf.org/2007/opf'
                    try:
                        opf_bytes = zin.read(opf_path)
                    except KeyError:
                        LOGGER.warning("EPUB preview: OPF file missing in archive: %s (book_id=%d)", opf_path, book_id)
                        return Response(status_code=422,
                                        content=f"EPUB structure error: OPF file missing: {opf_path}")
                    # opf: 프리픽스가 선언 없이 사용된 경우 추가
                    # (lxml recover가 보존하지만 재직렬화 시 xmlns:opf 누락 → 브라우저 파싱 실패)
                    opf_text = opf_bytes.decode('utf-8', errors='replace')
                    if 'opf:' in opf_text and 'xmlns:opf=' not in opf_text:
                        opf_text = opf_text.replace('<package ', f'<package xmlns:opf="{opf_ns}" ', 1)
                        opf_bytes = opf_text.encode('utf-8')
                    opf = etree.fromstring(opf_bytes, etree.XMLParser(recover=True))

                    # manifest: id → href, media-type
                    manifest: Dict[str, Dict[str, str]] = {}
                    href_to_id: Dict[str, str] = {}
                    for item in opf.findall(f'.//{{{opf_ns}}}item'):
                        item_id = item.get('id', '')
                        href = item.get('href', '')
                        manifest[item_id] = {'href': href, 'media-type': item.get('media-type', '')}
                        zip_path = normpath(pjoin(opf_dir, href)) if opf_dir else normpath(href)
                        href_to_id[zip_path] = item_id

                    # spine 순서
                    spine_el = opf.find(f'.//{{{opf_ns}}}spine')
                    if spine_el is None:
                        LOGGER.warning("EPUB preview: spine not found for book_id=%d, trying manifest order", book_id)
                        chapter_idrefs = [mid for mid, info in manifest.items()
                                          if info.get('media-type') == 'application/xhtml+xml'][:chapters]
                        # 출력 OPF에 spine 요소 생성 (검증 통과를 위해)
                        spine_el = etree.SubElement(opf, f'{{{opf_ns}}}spine')
                        for idref in chapter_idrefs:
                            itemref = etree.SubElement(spine_el, f'{{{opf_ns}}}itemref')
                            itemref.set('idref', idref)
                        spine_refs = list(spine_el.findall(f'{{{opf_ns}}}itemref'))
                    else:
                        spine_refs = list(spine_el.findall(f'{{{opf_ns}}}itemref'))
                        chapter_idrefs = [ref.get('idref') for ref in spine_refs[:chapters]
                                          if ref.get('idref') in manifest]

                    # 포함할 zip 내 파일 경로
                    files_to_include = {opf_path}
                    if 'META-INF/container.xml' in zin.namelist():
                        files_to_include.add('META-INF/container.xml')
                    manifest_ids_to_keep = set(chapter_idrefs)

                    # spine toc 속성이 참조하는 NCX 파일 포함
                    toc_id = spine_el.get('toc', '') if spine_el is not None else ''
                    if toc_id and toc_id in manifest:
                        manifest_ids_to_keep.add(toc_id)
                        toc_href = manifest[toc_id]['href']
                        toc_zp = normpath(pjoin(opf_dir, toc_href)) if opf_dir else normpath(toc_href)
                        files_to_include.add(toc_zp)

                    # 챕터 파일의 zip 경로 계산
                    chapter_zip_paths = []
                    for idref in chapter_idrefs:
                        if idref in manifest:
                            href = manifest[idref]['href']
                            zp = normpath(pjoin(opf_dir, href)) if opf_dir else normpath(href)
                            files_to_include.add(zp)
                            chapter_zip_paths.append(zp)

                    # 챕터 HTML에서 참조 리소스 수집
                    referenced = set()
                    for zp in chapter_zip_paths:
                        try:
                            content = zin.read(zp).decode('utf-8', errors='replace')
                        except KeyError:
                            LOGGER.warning("EPUB preview: chapter file missing in archive: %s", zp)
                            continue
                        item_dir = dirname(zp)
                        soup = BeautifulSoup(content, 'html.parser')
                        for img in soup.find_all('img'):
                            src = img.get('src', '')
                            if src and not src.startswith('data:'):
                                referenced.add(normpath(pjoin(item_dir, src)))
                        # SVG <image> 태그 (커버 등에서 사용)
                        for image in soup.find_all('image'):
                            href = image.get('xlink:href') or image.get('href', '')
                            if href and not href.startswith('data:'):
                                referenced.add(normpath(pjoin(item_dir, href)))
                        for link in soup.find_all('link'):
                            href_attr = link.get('href', '')
                            if href_attr:
                                referenced.add(normpath(pjoin(item_dir, href_attr)))

                    # 챕터에서 참조된 CSS만 포함 및 CSS 내 url() 참조 수집
                    css_url_pattern = re.compile(r'url\(["\']?([^"\')\s]+)["\']?\)')
                    css_refs = [r for r in referenced if r in href_to_id
                                and 'css' in manifest[href_to_id[r]].get('media-type', '')]
                    referenced -= set(css_refs)  # CSS는 별도 처리
                    for zp in css_refs:
                        item_id = href_to_id[zp]
                        files_to_include.add(zp)
                        manifest_ids_to_keep.add(item_id)
                        try:
                            css_content = zin.read(zp).decode('utf-8', errors='replace')
                            css_dir = dirname(zp)
                            for m in css_url_pattern.findall(css_content):
                                if not m.startswith('data:'):
                                    referenced.add(normpath(pjoin(css_dir, m)))
                        except KeyError:
                            LOGGER.warning("EPUB preview: CSS file missing in archive: %s", zp)

                    # 참조된 이미지/폰트 추가 (대용량 폰트 제외)
                    FONT_SIZE_LIMIT = 500 * 1024  # 500KB
                    FONT_EXTENSIONS = {'.ttf', '.otf', '.woff', '.woff2'}
                    FONT_MEDIA_TYPES = {'font/ttf', 'font/otf', 'font/woff', 'font/woff2',
                                        'application/font-ttf', 'application/font-woff',
                                        'application/font-woff2', 'application/x-font-ttf'}
                    for ref_path in referenced:
                        if ref_path in href_to_id:
                            item_id = href_to_id[ref_path]
                            info = manifest[item_id]
                            ext = os.path.splitext(ref_path)[1].lower()
                            if ext in FONT_EXTENSIONS or info.get('media-type', '') in FONT_MEDIA_TYPES:
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
                    manifest_el = opf.find(f'.//{{{opf_ns}}}manifest')
                    if manifest_el is not None:
                        for item in list(manifest_el.findall(f'{{{opf_ns}}}item')):
                            if item.get('id') not in manifest_ids_to_keep:
                                manifest_el.remove(item)

                    # spine에서 불필요한 항목 제거
                    if spine_el is not None:
                        for ref in list(spine_refs):
                            if ref.get('idref') not in chapter_idrefs:
                                spine_el.remove(ref)

                    # guide에서 존재하지 않는 파일 참조 제거
                    guide_el = opf.find(f'.//{{{opf_ns}}}guide')
                    if guide_el is not None:
                        for ref in list(guide_el.findall(f'{{{opf_ns}}}reference')):
                            href = ref.get('href', '').split('#')[0]
                            ref_zp = normpath(pjoin(opf_dir, href)) if opf_dir else normpath(href)
                            if ref_zp not in files_to_include:
                                guide_el.remove(ref)

                    # 새 EPUB 작성
                    modified_opf = '<?xml version="1.0" encoding="UTF-8"?>\n' + etree.tostring(opf, encoding='unicode')

                    # @font-face 블록 제거용 패턴
                    font_face_pattern = re.compile(r'@font-face\s*\{[^}]*\}')

                    # NCX에서 미리보기에 포함되지 않은 파일 참조 제거
                    # (epub.js가 존재하지 않는 파일 참조로 인해 초기화 중단될 수 있음)
                    ncx_ns = 'http://www.daisy.org/z3986/2005/ncx/'
                    ncx_zp = None
                    if toc_id and toc_id in manifest:
                        ncx_href = manifest[toc_id]['href']
                        ncx_zp = normpath(pjoin(opf_dir, ncx_href)) if opf_dir else normpath(ncx_href)

                    with zipfile.ZipFile(str(cache_file), 'w', zipfile.ZIP_DEFLATED) as zout:
                        zout.writestr('mimetype', 'application/epub+zip', compress_type=zipfile.ZIP_STORED)
                        for zp in files_to_include:
                            if zp == opf_path:
                                zout.writestr(zp, modified_opf)
                            elif zp == ncx_zp:
                                # NCX 파일: 미리보기에 없는 파일 참조 navPoint 제거 (중첩 포함)
                                try:
                                    ncx_data = zin.read(zp)
                                    ncx_tree = etree.fromstring(ncx_data)
                                    ncx_dir = dirname(zp)
                                    # 모든 깊이의 navPoint를 순회하며 누락 파일 참조 제거
                                    for np in list(ncx_tree.iter(f'{{{ncx_ns}}}navPoint')):
                                        content_el = np.find(f'{{{ncx_ns}}}content')
                                        if content_el is not None:
                                            src = content_el.get('src', '')
                                            src_file = src.split('#')[0]
                                            if not src_file:
                                                continue  # fragment-only src는 유지
                                            src_zp = normpath(pjoin(ncx_dir, src_file)) if ncx_dir else normpath(src_file)
                                            if src_zp not in files_to_include:
                                                np.getparent().remove(np)
                                    ncx_out = '<?xml version="1.0" encoding="UTF-8"?>\n' + etree.tostring(ncx_tree, encoding='unicode')
                                    zout.writestr(zp, ncx_out)
                                except Exception as e:
                                    LOGGER.warning("EPUB preview: NCX filtering failed: %s", e)
                                    zout.writestr(zp, zin.read(zp))
                            else:
                                try:
                                    data = zin.read(zp)
                                    # CSS에서 제외된 폰트의 @font-face 제거
                                    if zp.endswith('.css'):
                                        css_text = data.decode('utf-8', errors='replace')
                                        css_dir = dirname(zp)

                                        def _strip_missing_font(m):
                                            for url in css_url_pattern.findall(m.group()):
                                                if normpath(pjoin(css_dir, url)) not in files_to_include:
                                                    return ''
                                            return m.group()

                                        css_text = font_face_pattern.sub(_strip_missing_font, css_text)
                                        data = css_text.encode('utf-8')
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
                return FileResponse(path=cache_file, media_type="application/epub+zip",
                                    headers=extra_headers)
            except zipfile.BadZipFile:
                LOGGER.exception("EPUB preview: corrupted ZIP for book_id=%d", book_id)
                return Response(status_code=422, content="EPUB file is corrupted or not a valid ZIP")
            except Exception as e:
                LOGGER.exception("EPUB preview generation failed for book_id=%d", book_id)
                return Response(status_code=500, content=f"EPUB preview failed: {e}")

        elif suffix in (".doc", ".hwp"):
            cache_file = cache_dir / f"{book_id}.html"
            if cache_file.exists() and cache_file.stat().st_mtime >= original_mtime:
                LOGGER.debug("Preview cache hit for book_id=%d (%s)", book_id, suffix)
                return FileResponse(path=cache_file, media_type="text/html")

            try:
                html_content = BookManager._convert_with_libreoffice(book.file_path, "html")
                if html_content:
                    cache_file.write_text(html_content, encoding="utf-8")
                    BookManager._evict_old_cache(cache_dir)
                    LOGGER.debug("Preview generated for book_id=%d (%s)", book_id, suffix)
                    return Response(content=html_content, media_type="text/html")
            except Exception as e:
                LOGGER.error("%s preview generation failed for book_id=%d: %s", suffix.upper(), book_id, e)
                return Response(status_code=500, content=f"{suffix.upper()} preview failed: {e}")

        # 지원하지 않는 형식
        return Response(status_code=400, content=f"Unsupported file type: {suffix}")

    async def search_by_keyword(self, keyword: str, max_result_count: int = -1) -> Tuple[List[Book], Optional[str]]:
        LOGGER.debug("# search_by_keyword(keyword='%s')", keyword)
        result_list = self.es_manager.search_by_keyword(keyword, max_result_count=max_result_count)
        if result_list and len(result_list) > 0:
            return [self.item_class(book_id=book_id, info=doc) for book_id, doc, _score in result_list], None
        return [], "No books found"

    async def search_by_keyword_paged(self, keyword: str, size: int = 10, offset: int = 0, exclude_categories: List[str] = None) -> Tuple[List[Book], int, Optional[str]]:
        LOGGER.debug("# search_by_keyword_paged(keyword='%s', size=%d, offset=%d, exclude_categories=%s)", keyword, size, offset, exclude_categories)
        result_list, total = self.es_manager.search_by_keyword_paged(keyword, size=size, offset=offset, exclude_categories=exclude_categories)
        if result_list:
            return [self.item_class(book_id=bid, info=doc) for bid, doc, _ in result_list], total, None
        return [], total, "No books found"

    async def search_similar_books(self, book_id: int, max_result_count: int = -1) -> Tuple[List[Book], Optional[str]]:
        LOGGER.debug("# search_similar_books(book_id=%d)", book_id)
        doc = self.es_manager.search_by_id(book_id)
        if not doc:
            return [], f"No book found with id '{book_id}'"
        result_list = self.es_manager.search_similar_docs(doc["category"], doc["title"], doc["author"], doc["file_type"], doc["file_size"], doc["summary"][:3500], exclude_id=book_id, max_result_count=max_result_count)
        if result_list and len(result_list) > 0:
            return [self.item_class(book_id=doc_id, info=similar_doc) for doc_id, similar_doc, _score in result_list], None
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
            return [self.item_class(book_id=did, info=sdoc, score=score) for did, sdoc, score in result_list], total, None
        return [], total, "No similar books found"

    async def add_book(self, data: Dict[int, Dict[str, Any]]) -> Tuple[Optional[int], Optional[str]]:
        LOGGER.debug("# add_book(data='%r')", data)
        doc_id_list = self.es_manager.insert(data)
        if doc_id_list and len(doc_id_list) == 1:
            self.es_manager.refresh()  # 단일 문서 추가 후 즉시 검색 가능하도록
            return doc_id_list[0], None
        return None, f"can't add book '{data}' to ElasticSearch"

    async def update_book(self, book_id: int, new_category: str, new_title: str, new_author: str, new_path: Path, new_type: str, force: bool = False) -> Tuple[str, Optional[str]]:
        LOGGER.debug("# update_book(book_id=%d, new_category='%s', new_title='%s', new_author='%s', new_path='%r', new_file_type='%s', force=%s)", book_id, new_category, new_title, new_author, new_path, new_type, force)
        # rename file
        doc = self.es_manager.search_by_id(book_id)
        if doc:
            book = self.item_class(book_id=book_id, info=doc)
            file_path = self.path_prefix / book.file_path
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
                        return "Error", f"CONFLICT:대상 경로에 파일이 이미 존재합니다: {relative}"
                    LOGGER.warning("update_book: force overwriting existing file '%s' (book_id=%d)", new_full_path, book_id)

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
        """파일시스템의 1레벨 디렉토리 기준으로 ES와 파일 경로 불일치를 검출"""
        import os as _os
        from concurrent.futures import ThreadPoolExecutor

        # 1. ES: 단일 scroll로 카테고리별 file_path 집합 조회
        es_file_paths = self.es_manager.get_all_file_paths_grouped()
        es_cats = {cat: len(paths) for cat, paths in es_file_paths.items()}

        # 2. 파일시스템: 1레벨 디렉토리 + 그 하위 2레벨 스캔 (경로 집합)
        base_str = str(self.path_prefix)
        fs_cats: Dict[str, set] = {}

        def collect_files(dir_path: str, category: str) -> set:
            paths: set = set()
            try:
                with _os.scandir(dir_path) as it:
                    for entry in it:
                        if entry.is_file(follow_symlinks=False):
                            paths.add(f"{category}/{entry.name}")
            except (PermissionError, OSError):
                pass
            return paths

        try:
            # 최상위 디렉토리의 파일을 _root 카테고리로 수집
            root_paths: set = set()
            with _os.scandir(base_str) as l1_it:
                for l1 in l1_it:
                    if l1.is_file(follow_symlinks=False):
                        root_paths.add(l1.name)
            if root_paths:
                fs_cats["_root"] = root_paths

            # L1/L2 디렉토리 목록 수집 (빠름 — 디렉토리 이름만 읽기)
            scan_tasks: List[Tuple[str, str]] = []  # (dir_path, category)
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
                futures = {executor.submit(collect_files, dp, cat): cat for dp, cat in scan_tasks}
                for future in futures:
                    cat = futures[future]
                    paths = future.result()
                    if paths:
                        fs_cats[cat] = paths
        except (PermissionError, OSError):
            pass

        # FS 캐시 저장 (get_category_mismatch_details에서 재사용, TTL 60초)
        # time을 먼저 설정하여 캐시 읽기 측에서 stale 시간을 과대평가하지 않도록 함
        self._fs_cats_cache_time = time.time()
        self._fs_cats_cache = fs_cats

        # 3. 비교 (경로 기반 집합 비교)
        all_keys = sorted(set(list(fs_cats.keys()) + [k for k in es_cats if k.count("/") <= 1 and not k.startswith(".")]))
        mismatches = []
        es_only = []
        fs_only = []
        for key in all_keys:
            es_count = es_cats.get(key)
            fs_paths = fs_cats.get(key)
            if es_count is not None and fs_paths is not None:
                # 양쪽 다 존재 → 경로 기반 비교 (이미 메모리에 있음)
                es_paths = es_file_paths.get(key, set())
                diff = len(es_paths - fs_paths) + len(fs_paths - es_paths)
                if diff > 0:
                    mismatches.append({"category": key, "es_count": es_count, "fs_count": len(fs_paths), "diff": diff})
            elif es_count is not None:
                es_only.append({"category": key, "es_count": es_count})
            elif fs_paths is not None:
                fs_only.append({"category": key, "fs_count": len(fs_paths)})

        return {
            "mismatches": sorted(mismatches, key=lambda x: abs(x["diff"]), reverse=True),
            "es_only": es_only,
            "fs_only": fs_only,
        }

    async def get_category_mismatch_details(self, category: str) -> Dict[str, Any]:
        """특정 카테고리의 ES 문서와 파일시스템 파일을 비교하여 불일치 항목을 반환"""
        import os as _os

        # 1. ES 문서 목록 (file_path 기준)
        doc_list = self.es_manager.search_by_category(category, max_result_count=sys.maxsize)
        es_files: Dict[str, Dict[str, Any]] = {}
        for book_id, doc, _score in doc_list:
            rel_path = doc.get("file_path", "")
            es_files[rel_path] = {"book_id": book_id, **doc}

        # 2. 파일시스템 파일 목록 (캐시 활용, TTL 60초)
        fs_files: set = set()
        if self._fs_cats_cache is not None and (time.time() - self._fs_cats_cache_time) < 60:
            fs_files = self._fs_cats_cache.get(category, set())
        else:
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
            es_only.append({
                "book_id": info["book_id"],
                "title": info.get("title", ""),
                "file_type": info.get("file_type", ""),
                "file_path": path,
            })

        fs_only = []
        for path in sorted(fs_files - es_paths):
            name = path.rsplit("/", 1)[-1] if "/" in path else path
            fs_only.append({
                "file_name": name,
                "file_path": path,
            })

        return {"es_only": es_only, "fs_only": fs_only}

    async def index_single_file(self, file_path: str) -> Tuple[Optional[int], Optional[str]]:
        """파일시스템의 파일을 읽어 ES에 적재"""
        from utils.loader import Loader
        abs_path = self.path_prefix / file_path
        if not abs_path.is_file():
            return None, f"파일을 찾을 수 없습니다: {file_path}"
        data = Loader.read_file(abs_path)
        if not data:
            return None, f"지원하지 않는 파일 형식입니다: {file_path}"
        return await self.add_book(data)

    async def delete_file(self, file_path: str) -> Tuple[str, Optional[str]]:
        """파일시스템에서 파일을 삭제"""
        abs_path = self.path_prefix / file_path
        if not abs_path.is_relative_to(self.path_prefix):
            return "Error", f"잘못된 경로입니다: {file_path}"
        if not abs_path.is_file():
            return "Error", f"파일을 찾을 수 없습니다: {file_path}"
        try:
            abs_path.unlink()
            return "Ok", None
        except IOError as e:
            return "Error", f"파일 삭제 실패: {e}"

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
            from pypdf import PdfReader, PdfWriter
            reader = PdfReader(str(book.file_path))
            total_pages = len(reader.pages)

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
                return Response(
                    content=cache_file.read_bytes(),
                    media_type="application/pdf",
                    headers={
                        "Content-Encoding": "identity",
                        "Cache-Control": "no-transform",
                        "X-Total-Pages": str(total_pages),
                    },
                )

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
            return Response(
                content=pdf_bytes,
                media_type="application/pdf",
                headers={
                    "Content-Encoding": "identity",
                    "Cache-Control": "no-transform",
                    "X-Total-Pages": str(total_pages),
                },
            )
        except Exception as e:
            LOGGER.error("PDF pages extraction failed for book_id=%d: %s", book_id, e)
            return Response(status_code=500, content=f"PDF pages extraction failed: {e}")

    async def rename_category(self, old_category: str, new_category: str) -> Tuple[Dict[str, Any], Optional[str]]:
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
        if '..' in old_category or '..' in new_category:
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
            return {}, f"대상 카테고리 '{new_category}'에 이미 {new_count}개의 문서가 존재합니다"

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
                    return {}, f"ES 업데이트 실패: {e}. 경고: FS 롤백도 실패하여 수동 복구 필요 ('{new_dir}' → '{old_dir}')"
            return {}, f"ES 업데이트 실패: {e}"

        if es_result.get("failures"):
            # 부분 실패 시 FS 롤백
            if fs_renamed:
                try:
                    new_dir.rename(old_dir)
                    LOGGER.info("rename_category: ES 부분 실패로 FS 롤백")
                except OSError as rollback_err:
                    LOGGER.error("rename_category: FS 롤백 실패: %s", rollback_err)
                    return {}, f"ES 부분 실패: {es_result['failures']}. 경고: FS 롤백도 실패하여 수동 복구 필요"
            return {}, f"ES 업데이트 부분 실패: {es_result['failures']}"

        return {
            "old_category": old_category,
            "new_category": new_category,
            "updated_count": es_result["updated"],
            "fs_renamed": fs_renamed,
        }, None

    async def delete_category(self, category: str) -> Tuple[Dict[str, Any], Optional[str]]:
        """카테고리의 모든 문서를 일괄 삭제 (ES + FS)

        Returns:
            (result_dict, error_message) 튜플
        """
        LOGGER.debug("# delete_category(category='%s')", category)

        # 입력 검증
        if not category:
            return {}, "카테고리 이름이 비어있습니다"
        if '..' in category:
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

        result: Dict[str, Any] = {
            "category": category,
            "deleted_count": es_result["deleted"],
        }

        return result, None

    async def delete_book(self, book_id: int) -> Tuple[str, Optional[str]]:
        LOGGER.debug("# delete_book(book_id=%d)", book_id)
        doc = self.es_manager.search_by_id(book_id)
        if not doc:
            return "Ok", None

        warning_message = None

        # delete file
        try:
            book = self.item_class(book_id=book_id, info=doc)
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
