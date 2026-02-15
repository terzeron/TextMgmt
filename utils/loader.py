#!/usr/bin/env python


import sys
import os
import re
import getopt
import logging.config
import shutil
import subprocess
import tempfile
import warnings
import zipfile
from datetime import datetime
from pathlib import Path
from itertools import islice
from typing import Dict, Any, List, Tuple, Optional, Iterable

import ebooklib
from ebooklib import epub
import pypdf
from docx import Document
from striprtf.striprtf import rtf_to_text
from bs4 import BeautifulSoup

from backend.es_manager import ESManager
from utils.stat import Stat
from utils.isbn import extract as extract_isbn

logging.config.fileConfig(Path(__file__).parent.parent / "logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger()

# ebooklib 내부에서 XHTML을 HTML 파서로 읽을 때 발생하는 경고 억제
warnings.filterwarnings("ignore", message=".*XML.*HTML.*")

if "TM_BOOK_DIR" not in os.environ:
    LOGGER.error("The environment variable TM_BOOK_DIR is not set.")
    sys.exit(-1)

if "TM_COMICS_DIR" not in os.environ:
    LOGGER.error("The environment variable TM_COMICS_DIR is not set.")
    sys.exit(-1)


class Loader:
    TEXT_SIZE = 4096
    path_prefix = Path(os.environ["TM_BOOK_DIR"])
    comics_path_prefix = Path(os.environ["TM_COMICS_DIR"])

    @staticmethod
    def get_path_prefix(file_path: Path) -> Path:
        """파일 경로에 해당하는 path_prefix를 반환"""
        if file_path.is_relative_to(Loader.comics_path_prefix):
            return Loader.comics_path_prefix
        return Loader.path_prefix

    @staticmethod
    def read_from_text(file_path: Path) -> Tuple[str, int, int, str]:
        """TXT 파일 읽기. 반환: (summary, line_count, page_count, raw_content)"""
        Stat.text_count += 1
        start_time = datetime.now()

        line_count = 0
        data = ""
        raw_content = ""
        try:
            with file_path.open("r", encoding="utf-8") as infile:
                # 한 번에 읽어서 처리
                raw_content = infile.read()
                line_count = raw_content.count('\n') + 1 if raw_content else 0
                data = raw_content[:Loader.TEXT_SIZE]
                data = data.replace("\ufeff", "")
                data = re.sub(r'[^\w\sㄱ-힣]', ' ', data)
        except UnicodeDecodeError as e:
            LOGGER.error(f"can't read unicode text from file '{file_path}', {e}")
            data = ""

        end_time = datetime.now()
        Stat.text_total_time += (end_time - start_time).total_seconds()

        return data, line_count, 0, raw_content

    @staticmethod
    def read_from_epub_with_extracting_zip(file_path: Path) -> Tuple[str, int]:
        """메모리에서 직접 zip 내용을 읽어 처리 (임시 디렉토리 사용 안 함)"""
        result = ""
        total_text = ""
        try:
            with zipfile.ZipFile(file_path, "r") as zf:
                # 1. container.xml에서 rootfile 찾기
                container_content = zf.read("META-INF/container.xml").decode("utf-8", errors="ignore")
                m = re.search(r'<rootfile[^>]*full-path="(?P<root_file>[^"]+)"', container_content)
                if not m:
                    return "", 0
                root_file = m.group("root_file")
                root_dir = "/".join(root_file.split("/")[:-1])

                # 2. OPF 파일에서 chapter 파일 목록 찾기
                opf_content = zf.read(root_file).decode("utf-8", errors="ignore")
                matches = re.findall(
                    r'<(?:opf:)?item\s[^>]*href="(?P<chapter_file>[^"]*\.x?html)"[^>]*media-type="application/xhtml\+xml"',
                    opf_content
                )

                # 3. 각 chapter 파일 읽기
                for chapter_file in matches:
                    chapter_path = f"{root_dir}/{chapter_file}" if root_dir else chapter_file
                    try:
                        content = zf.read(chapter_path).decode("utf-8", errors="ignore")
                        with warnings.catch_warnings():
                            warnings.filterwarnings("ignore", message=".*XML.*HTML.*")
                            soup = BeautifulSoup(content, "lxml")
                        text = soup.get_text()
                        total_text += text
                        if len(result) < Loader.TEXT_SIZE:
                            result += text
                    except KeyError:
                        continue
        except zipfile.BadZipFile as e:
            LOGGER.error(file_path)
            LOGGER.error(e)

        line_count = total_text.count('\n') + 1 if total_text else 0
        return result[:Loader.TEXT_SIZE], line_count

    @staticmethod
    def read_from_epub(file_path: Path) -> Tuple[str, int, int]:
        Stat.normal_epub_count += 1
        start_time = datetime.now()

        result = ""
        total_text = ""
        line_count = 0
        try:
            book = epub.read_epub(file_path)
            titles = book.get_metadata("DC", "title")
            if titles:
                for title in titles:
                    if title[0]:
                        result += " " + title[0]
            creators = book.get_metadata("DC", "creator")
            if creators:
                for creator in creators:
                    if creator[0]:
                        result += " " + creator[0]

            for doc in book.get_items_of_type(ebooklib.ITEM_DOCUMENT):
                with warnings.catch_warnings():
                    warnings.filterwarnings("ignore", message=".*XML.*HTML.*")
                    soup = BeautifulSoup(doc.get_body_content(), "lxml")
                text = soup.get_text()
                total_text += text
                if len(result) < Loader.TEXT_SIZE:
                    result += text

            line_count = total_text.count('\n') + 1 if total_text else 0

            end_time = datetime.now()
            Stat.normal_epub_total_time += (end_time - start_time).total_seconds()
        except Exception as e:
            LOGGER.error(file_path)
            LOGGER.error(e)

            Stat.normal_epub_count -= 1
            Stat.zipped_epub_count += 1
            start_time = datetime.now()

            try:
                result, line_count = Loader.read_from_epub_with_extracting_zip(file_path)
            except epub.EpubException as e2:
                LOGGER.error(file_path)
                LOGGER.error(e2)

            end_time = datetime.now()
            Stat.zipped_epub_total_time += (end_time - start_time).total_seconds()

        result = re.sub(r'[^\w\sㄱ-힣]', ' ', result)

        return result[:Loader.TEXT_SIZE], line_count, 0

    @staticmethod
    def read_from_pdf(file_path: Path) -> Tuple[str, int, int]:
        Stat.pdf_count += 1
        start_time = datetime.now()

        result = ""
        page_count = 0
        with file_path.open("rb") as infile:
            try:
                reader = pypdf.PdfReader(infile)
                page_count = len(reader.pages)
                for page in reader.pages:
                    text = page.extract_text()
                    if len(result) < Loader.TEXT_SIZE:
                        result += text
                    else:
                        break
            except Exception as e:
                LOGGER.error(file_path)
                LOGGER.error(e)
        result = re.sub(r'[^\w\sㄱ-힣]', ' ', result)

        end_time = datetime.now()
        Stat.pdf_total_time += (end_time - start_time).total_seconds()

        return result[:Loader.TEXT_SIZE], 0, page_count

    @staticmethod
    def read_from_html(file_path: Path) -> Tuple[str, int, int]:
        Stat.html_count += 1
        start_time = datetime.now()

        content = ""
        line_count = 0
        with file_path.open("r") as infile:
            content = infile.read()
            line_count = content.count('\n') + 1

        # XMLParsedAsHTMLWarning 억제하고 lxml 파서 사용
        with warnings.catch_warnings():
            warnings.filterwarnings("ignore", message=".*XML.*HTML.*")
            soup = BeautifulSoup(content, "lxml")
        result = soup.get_text()
        result = re.sub(r'[^\w\sㄱ-힣]', ' ', result)

        end_time = datetime.now()
        Stat.html_total_time += (end_time - start_time).total_seconds()

        return result[:Loader.TEXT_SIZE], line_count, 0

    @staticmethod
    def read_from_docx(file_path: Path) -> Tuple[str, int, int]:
        Stat.docx_count += 1
        start_time = datetime.now()

        result = ""
        total_text = ""
        doc = Document(str(file_path))
        for paragraph in doc.paragraphs:
            text = paragraph.text
            total_text += text + "\n"
            if len(result) < Loader.TEXT_SIZE:
                result += text
        result = re.sub(r'[^\w\sㄱ-힣]', ' ', result)
        line_count = total_text.count('\n') if total_text else 0

        end_time = datetime.now()
        Stat.docx_total_time += (end_time - start_time).total_seconds()

        return result[:Loader.TEXT_SIZE], line_count, 0

    @staticmethod
    def read_from_rtf(file_path: Path) -> Tuple[str, int, int]:
        Stat.rtf_count += 1
        start_time = datetime.now()

        result = ""
        line_count = 0
        try:
            with file_path.open("rb") as infile:
                raw_data = infile.read()
                doc = raw_data.decode('utf-8')
                result = rtf_to_text(doc, errors="ignore")
                line_count = result.count('\n') + 1 if result else 0
        except Exception as e:
            LOGGER.error(file_path)
            LOGGER.error(e)
        result = re.sub(r'[^\w\sㄱ-힣]', ' ', result)

        end_time = datetime.now()
        Stat.rtf_total_time += (end_time - start_time).total_seconds()

        return result[:Loader.TEXT_SIZE], line_count, 0

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
    def _convert_with_libreoffice(file_path: Path, output_format: str) -> str:
        """LibreOffice를 사용하여 파일을 변환하고 결과 텍스트를 반환"""
        lo_bin = Loader._find_libreoffice()
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

    @staticmethod
    def read_from_doc(file_path: Path) -> Tuple[str, int, int]:
        Stat.doc_count += 1
        start_time = datetime.now()
        result = ""
        line_count = 0
        try:
            raw_text = Loader._convert_with_libreoffice(file_path, "txt:Text")
            line_count = raw_text.count('\n') + 1 if raw_text else 0
            result = raw_text[:Loader.TEXT_SIZE]
            result = re.sub(r'[^\w\sㄱ-힣]', ' ', result)
        except Exception as e:
            LOGGER.error(f"can't read doc file '{file_path}': {e}")
        end_time = datetime.now()
        Stat.doc_total_time += (end_time - start_time).total_seconds()
        return result[:Loader.TEXT_SIZE], line_count, 0

    @staticmethod
    def read_from_hwp(file_path: Path) -> Tuple[str, int, int]:
        Stat.hwp_count += 1
        start_time = datetime.now()
        result = ""
        line_count = 0
        try:
            raw_text = Loader._convert_with_libreoffice(file_path, "txt:Text")
            line_count = raw_text.count('\n') + 1 if raw_text else 0
            result = raw_text[:Loader.TEXT_SIZE]
            result = re.sub(r'[^\w\sㄱ-힣]', ' ', result)
        except Exception as e:
            LOGGER.error(f"can't read hwp file '{file_path}': {e}")
        end_time = datetime.now()
        Stat.hwp_total_time += (end_time - start_time).total_seconds()
        return result[:Loader.TEXT_SIZE], line_count, 0

    @staticmethod
    def read_from_image(_file_path: Path) -> Tuple[str, int, int]:
        Stat.image_count += 1
        # 이미지는 line_count, page_count 해당 없음
        return "", 0, 0

    @staticmethod
    def read_file(file_path: Path, stat_result: Optional[os.stat_result] = None, skip_text: bool = False) -> Dict[int, Dict[str, Any]]:
        if file_path.is_file():
            sys.stdout.flush()
            # read metadata of each file (stat 결과 재사용)
            st = stat_result if stat_result else file_path.stat()
            inode_num = st.st_ino
            file_size = st.st_size
            prefix = Loader.get_path_prefix(file_path)
            category = str(file_path.parent.relative_to(prefix))
            if category == ".":
                category = "_root"
            m = re.search(r"^\[(?P<author>[^\]]+)\]\s*(?P<title>.+)$", file_path.stem)
            if m:
                author = m.group("author")
                title = m.group("title")
            else:
                author = ""
                title = file_path.stem

            file_type = file_path.suffix[1:]

            # skip_text: 텍스트 추출 건너뛰기 (만화 등 이미지 기반 파일)
            if skip_text:
                summary = ""
                line_count = 0
                page_count = 0
                isbn_list = []
                # PDF만 예외: page_count 추출 유지
                if file_type == "pdf":
                    try:
                        with file_path.open("rb") as f:
                            page_count = len(pypdf.PdfReader(f).pages)
                    except Exception as e:
                        LOGGER.error(file_path)
                        LOGGER.error(e)
                    Stat.pdf_count += 1
                # 지원하지 않는 확장자는 기존 동작 유지 (빈 dict 반환)
                supported_types = {"txt", "epub", "pdf", "docx", "doc", "hwp", "rtf", "html",
                                   "jpg", "jpeg", "png", "gif", "webp", "bmp", "tiff", "svg", "cbz"}
                if file_type not in supported_types:
                    return {}

                return {
                    inode_num: {
                        "category": category,
                        "title": title,
                        "author": author,
                        "file_path": str(file_path.relative_to(prefix)),
                        "file_type": file_type,
                        "file_size": int(file_size),
                        "line_count": line_count,
                        "page_count": page_count,
                        "isbn": "",
                        "summary": summary,
                        "updated_time": datetime.now().isoformat()
                    }
                }

            # read content of each file
            summary = ""
            line_count = 0
            page_count = 0
            isbn_list = []
            raw_content = ""  # TXT 파일용 원본 content
            if file_type == "txt":
                summary, line_count, page_count, raw_content = Loader.read_from_text(file_path)
            elif file_type == "epub":
                summary, line_count, page_count = Loader.read_from_epub(file_path)
            elif file_type == "pdf":
                if file_path.is_relative_to(Loader.comics_path_prefix):
                    # comics: 텍스트 추출은 생략하되 페이지 수만 추출
                    page_count = 0
                    try:
                        with file_path.open("rb") as f:
                            page_count = len(pypdf.PdfReader(f).pages)
                    except Exception as e:
                        LOGGER.error(file_path)
                        LOGGER.error(e)
                    summary, line_count = "", 0
                    Stat.pdf_count += 1
                else:
                    summary, line_count, page_count = Loader.read_from_pdf(file_path)
            elif file_type == "docx":
                summary, line_count, page_count = Loader.read_from_docx(file_path)
            elif file_type == "doc":
                summary, line_count, page_count = Loader.read_from_doc(file_path)
            elif file_type == "hwp":
                summary, line_count, page_count = Loader.read_from_hwp(file_path)
            elif file_type == "rtf":
                summary, line_count, page_count = Loader.read_from_rtf(file_path)
            elif file_type == "html":
                summary, line_count, page_count = Loader.read_from_html(file_path)
            elif file_type in ("jpg", "jpeg", "png", "gif", "webp", "bmp", "tiff", "svg"):
                summary, line_count, page_count = Loader.read_from_image(file_path)
            elif file_type == "cbz":
                summary, line_count, page_count = Loader.read_from_image(file_path)
            else:
                return {}

            # ISBN 추출 (TXT는 이미 읽은 content 재사용)
            if file_type in ("txt", "epub", "pdf", "djvu", "hwp"):
                isbn_list = extract_isbn(file_path, content=raw_content if file_type == "txt" else None)

            return {
                inode_num: {
                    "category": category,
                    "title": title,
                    "author": author,
                    "file_path": str(file_path.relative_to(prefix)),
                    "file_type": file_type,
                    "file_size": int(file_size),
                    "line_count": line_count,
                    "page_count": page_count,
                    "isbn": isbn_list[0] if isbn_list else "",
                    "summary": summary,
                    "updated_time": datetime.now().isoformat()
                }
            }

        return {}

    @staticmethod
    def get_file_list(path: Path, num_files: int = sys.maxsize, recursive: bool = False) -> List[Path]:
        """파일 목록을 반환 (파싱 없이)

        recursive=False인 경우:
        1. 하위 디렉토리 각각에서 첫 번째 파일 1개씩
        2. 지정된 디렉토리에 바로 속한 파일들
        """
        if path.is_dir():
            if recursive:
                file_path_list = [
                    p for p in path.rglob("*")
                    if p.is_file() and not any(part.startswith('.') for part in p.relative_to(path).parts)
                ]
            else:
                file_path_list = []
                # 1. 하위 디렉토리 각각에서 첫 번째 파일 1개씩
                for subdir in path.iterdir():
                    if subdir.is_dir() and not subdir.name.startswith('.'):
                        first_file = next((p for p in subdir.iterdir() if p.is_file() and not p.name.startswith('.')), None)
                        if first_file:
                            file_path_list.append(first_file)
                # 2. 지정된 디렉토리에 바로 속한 파일들
                file_path_list.extend(p for p in path.iterdir() if p.is_file() and not p.name.startswith('.'))
        else:
            file_path_list = [path]
        return file_path_list[:num_files]

    @staticmethod
    def get_stat(file_path: Path) -> os.stat_result:
        """파일의 stat 결과를 반환"""
        return file_path.stat()

    @staticmethod
    def read_files(path: Path, num_files: int = sys.maxsize, recursive: bool = False) -> Dict[int, Dict[str, Any]]:
        """파일들을 읽어서 데이터 딕셔너리로 반환 (테스트 및 하위 호환성용)"""
        data: Dict[int, Dict[str, Any]] = {}
        file_list = Loader.get_file_list(path, num_files, recursive)

        for child_path in file_list:
            print(f"* {child_path}")
            data_item = Loader.read_file(child_path)
            if data_item:
                data.update(data_item)

        return data


def print_usage(program_name: str):
    print(f"Usage:\t{program_name}\t[ --delete ] [ --reload ] [ --recursive ] <index_name> [file or directory path ...]")
    print("\t\tindex_name: book | comics")
    print("\t\t--delete: delete index and exit (no file path required)")
    print("\t\t--reload: force reload even if file already exists in ES")
    print("\t\t--recursive: scan subdirectories recursively")
    print()
    print("\t\tNote: When a file (not directory) is specified, it will be force-reloaded automatically.")
    sys.exit(0)


def main() -> int:
    BATCH_SIZE = 100

    do_delete = False
    do_reload = False
    do_recursive = False
    args: List[str] = []
    try:
        opts, args = getopt.getopt(sys.argv[1:], "", ["delete", "reload", "recursive"])
        for opt, _ in opts:
            if opt == "--delete":
                do_delete = True
            elif opt == "--reload":
                do_reload = True
            elif opt == "--recursive":
                do_recursive = True
    except getopt.GetoptError as e:
        LOGGER.error(e)
        print_usage(sys.argv[0])

    # 인덱스명 파싱
    INDEX_MAP = {
        "book": os.environ.get("TM_ES_BOOK_INDEX", "book"),
        "comics": os.environ.get("TM_ES_COMICS_INDEX", "comics"),
    }

    if len(args) < 1:
        print_usage(sys.argv[0])

    index_name_arg = args[0]
    if index_name_arg not in INDEX_MAP:
        LOGGER.error(f"유효하지 않은 인덱스명: '{index_name_arg}' (book 또는 comics만 가능)")
        print_usage(sys.argv[0])

    file_args = args[1:]

    if not do_delete and len(file_args) < 1:
        print_usage(sys.argv[0])

    start_time: datetime = datetime.now()

    # ESManager 초기화 (한 번의 실행에서 하나의 인덱스만 사용)
    idx = INDEX_MAP[index_name_arg]
    es_manager = ESManager(index_name=idx)
    try:
        if not es_manager.es.ping():
            LOGGER.error("Elasticsearch 서버에 연결할 수 없습니다.")
            sys.exit(-1)
    except Exception as e:
        LOGGER.error(f"Elasticsearch 접속 실패: {e}")
        sys.exit(-1)
    es_manager.create_index()
    print(f"ES 인덱스: {idx}")

    if do_delete:
        try:
            if es_manager.do_exist_index():
                es_manager.delete_index()
                print(f"인덱스 '{es_manager.index_name}' 삭제 완료")
            else:
                print(f"인덱스 '{es_manager.index_name}'가 존재하지 않습니다")
        except Exception as e:
            LOGGER.error(e)
            return -1
        return 0

    def process_file_iter(file_iter: Iterable[Path], skip_check: bool = False, skip_text: bool = False) -> Tuple[int, int, int]:
        """파일 iterator를 배치 처리하여 ES에 저장. 반환: (처리 수, 건너뜀 수, 경로동기화 수)"""
        skipped_count = 0
        processed_count = 0
        synced_count = 0
        file_iterator = iter(file_iter)

        while True:
            # generator에서 배치 단위로 가져오기
            batch_files = list(islice(file_iterator, BATCH_SIZE))
            if not batch_files:
                break

            # stat 수집 (한 번만 호출하여 inode와 함께 저장)
            file_stat_map: Dict[int, Tuple[Path, os.stat_result]] = {}
            for file_path in batch_files:
                try:
                    st = Loader.get_stat(file_path)
                    file_stat_map[st.st_ino] = (file_path, st)
                except OSError:
                    continue

            if skip_check:
                # --reload 모드: 존재 여부 무시, 전부 파싱
                new_inodes = set(file_stat_map.keys())
                path_updates: Dict[int, Dict[str, str]] = {}
            else:
                # 기존 경로 조회 (inode → file_path)
                existing_paths = es_manager.get_existing_paths(list(file_stat_map.keys()))

                # 경로 동기화: ES에 있고 file_path가 다른 경우
                path_updates: Dict[int, Dict[str, str]] = {}
                for inode, es_file_path in existing_paths.items():
                    if inode not in file_stat_map:
                        continue
                    file_path, _ = file_stat_map[inode]
                    prefix = Loader.get_path_prefix(file_path)
                    current_file_path = str(file_path.relative_to(prefix))
                    if current_file_path != es_file_path:
                        category = str(file_path.parent.relative_to(prefix))
                        if category == ".":
                            category = "_root"
                        path_updates[inode] = {"file_path": current_file_path, "category": category}

                # 신규 파일: ES에 없는 inode
                new_inodes = set(file_stat_map.keys()) - set(existing_paths.keys())
                skipped_count += len(existing_paths) - len(path_updates)

            # 경로 동기화 실행
            if path_updates:
                es_manager.bulk_update_paths(path_updates)
                synced_count += len(path_updates)
                for inode, fields in path_updates.items():
                    print(f"  [경로 동기화] inode={inode}: {fields['file_path']}")

            # 파일 파싱 (stat 결과 재사용)
            batch_data: Dict[int, Dict[str, Any]] = {}
            for inode in new_inodes:
                file_path, st = file_stat_map[inode]
                print(f"* {file_path}")
                data_item = Loader.read_file(file_path, stat_result=st, skip_text=skip_text)
                if data_item:
                    batch_data.update(data_item)

            # 데이터 저장
            if batch_data:
                es_manager.insert(batch_data)
                processed_count += len(batch_data)
                Stat.index_count += len(batch_data)
                if skip_check:
                    print(f"  [배치 저장: {len(batch_data)}개]")
                else:
                    synced_msg = f", 경로동기화: {len(path_updates)}개" if path_updates else ""
                    skip_batch = len(set(existing_paths.keys()) - set(path_updates.keys()))
                    print(f"  [배치 저장: {len(batch_data)}개, 건너뜀: {skip_batch}개{synced_msg}]")

        return processed_count, skipped_count, synced_count

    for arg in file_args:
        target_path = Path(arg)
        if not target_path.exists():
            LOGGER.error("can't find such a file or directory '%s'", target_path)
            return 0
        if not target_path.is_relative_to(Loader.path_prefix) and not target_path.is_relative_to(Loader.comics_path_prefix):
            LOGGER.error(f"{target_path} is not in $TM_BOOK_DIR({Loader.path_prefix}) or $TM_COMICS_DIR({Loader.comics_path_prefix}).")
            continue
        print(f"====== {target_path} ======")

        # comics 인덱스는 텍스트 추출 건너뛰기
        skip_text = index_name_arg == "comics"

        # 파일이 지정된 경우: 강제 재적재 (skip_check=True)
        if target_path.is_file():
            print(f"  [파일 강제 재적재] {target_path.name}")
            processed, _, _ = process_file_iter([target_path], skip_check=True, skip_text=skip_text)
            if processed > 0:
                print(f"  파일 재적재 완료")
            else:
                print(f"  파일 적재 실패 (지원하지 않는 형식일 수 있음)")
        elif do_recursive:
            # 전체 파일 등록 (generator 사용으로 메모리 효율화, hidden directory 제외)
            file_iter = (
                p for p in target_path.rglob("*")
                if p.is_file() and not any(part.startswith('.') for part in p.relative_to(target_path).parts)
            )
            skip_check = do_reload
            processed, skipped_count, synced_count = process_file_iter(file_iter, skip_check=skip_check, skip_text=skip_text)
            print(f"  총 {processed}개 파일 처리됨")
            if skipped_count > 0:
                print(f"  총 {skipped_count}개 중복 파일 건너뜀")
            if synced_count > 0:
                print(f"  총 {synced_count}개 경로 동기화")
        else:
            skip_check = do_reload

            # 1단계: 하위 디렉토리 각각에서 첫 번째 파일 1개씩 (모아서 한꺼번에 저장)
            print("  [1단계] 하위 디렉토리별 샘플 파일 등록")
            sample_files: List[Tuple[str, Path]] = []  # (subdir_name, file_path)
            for subdir in target_path.iterdir():
                if subdir.is_dir() and not subdir.name.startswith('.'):
                    # 첫 번째 파일만 가져옴 (정렬 불필요, iterator 사용)
                    first_file = next((p for p in subdir.iterdir() if p.is_file() and not p.name.startswith('.')), None)
                    if first_file:
                        sample_files.append((subdir.name, first_file))
                    else:
                        print(f"    {subdir.name}/ -> (파일 없음)")

            if sample_files:
                print(f"    {len(sample_files)}개 디렉토리에서 샘플 파일 선정 완료")
                for subdir_name, sample_file in sample_files:
                    print(f"      {subdir_name}/ -> {sample_file.name}")

                # 모아서 한꺼번에 ES에 저장
                sample_file_paths = [f for _, f in sample_files]
                processed, skipped1, synced1 = process_file_iter(sample_file_paths, skip_check=skip_check, skip_text=skip_text)
                print(f"    {processed}개 카테고리 샘플 저장, {skipped1}개 중복 건너뜀")
                if synced1 > 0:
                    print(f"    {synced1}개 경로 동기화")

            # 2단계: 지정된 디렉토리에 바로 속한 파일들
            print("  [2단계] 현재 디렉토리 파일 등록")
            current_dir_files = [p for p in target_path.iterdir() if p.is_file() and not p.name.startswith('.')]
            print(f"    {len(current_dir_files)}개 파일 발견")
            _, skipped2, synced2 = process_file_iter(current_dir_files, skip_check=skip_check, skip_text=skip_text)
            if skipped2 > 0:
                print(f"    {skipped2}개 중복 파일 건너뜀")
            if synced2 > 0:
                print(f"    {synced2}개 경로 동기화")

        print("================================")

    # 모든 작업 완료 후 인덱스 refresh
    es_manager.refresh()

    end_time = datetime.now()
    Stat.index_total_time = (end_time - start_time).total_seconds()
    Stat.print()

    return 0


if __name__ == "__main__":
    sys.exit(main())
