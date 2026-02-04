#!/usr/bin/env python


import sys
import os
import re
import getopt
import logging.config
import warnings
import zipfile
from datetime import datetime
from pathlib import Path
from itertools import islice
from typing import Dict, Any, List, Set, Tuple, Optional, Iterable

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


if "TM_WORK_DIR" not in os.environ:
    LOGGER.error("The environment variable TM_WORK_DIR is not set.")
    sys.exit(-1)


class Loader:
    TEXT_SIZE = 4096
    path_prefix = Path(os.environ["TM_WORK_DIR"])

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
                            warnings.filterwarnings("ignore", category=UserWarning, message=".*XML.*HTML.*")
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
                    warnings.filterwarnings("ignore", category=UserWarning, message=".*XML.*HTML.*")
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
            warnings.filterwarnings("ignore", category=UserWarning, message=".*XML.*HTML.*")
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
    def read_from_image(_file_path: Path) -> Tuple[str, int, int]:
        Stat.image_count += 1
        # 이미지는 line_count, page_count 해당 없음
        return "", 0, 0

    @staticmethod
    def read_file(file_path: Path, stat_result: Optional[os.stat_result] = None) -> Dict[int, Dict[str, Any]]:
        if file_path.is_file():
            sys.stdout.flush()
            # read metadata of each file (stat 결과 재사용)
            st = stat_result if stat_result else file_path.stat()
            inode_num = st.st_ino
            file_size = st.st_size
            category = str(file_path.parent.relative_to(Loader.path_prefix))
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
                summary, line_count, page_count = Loader.read_from_pdf(file_path)
            elif file_type == "docx":
                summary, line_count, page_count = Loader.read_from_docx(file_path)
            elif file_type == "rtf":
                summary, line_count, page_count = Loader.read_from_rtf(file_path)
            elif file_type == "html":
                summary, line_count, page_count = Loader.read_from_html(file_path)
            elif file_type in ("jpg", "jpeg", "png", "gif", "webp", "bmp", "tiff", "svg"):
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
                    "file_path": str(file_path.relative_to(Loader.path_prefix)),
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
                file_path_list = [p for p in path.rglob("*") if p.is_file()]
            else:
                file_path_list = []
                # 1. 하위 디렉토리 각각에서 첫 번째 파일 1개씩
                for subdir in sorted(path.iterdir()):
                    if subdir.is_dir():
                        subdir_files = sorted([p for p in subdir.iterdir() if p.is_file()])
                        if subdir_files:
                            file_path_list.append(subdir_files[0])
                # 2. 지정된 디렉토리에 바로 속한 파일들
                file_path_list.extend(sorted([p for p in path.iterdir() if p.is_file()]))
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
    print(f"Usage:\t{program_name}\t[ --delete ] [ --reload ] [ --recursive ] <file or directory path>")
    print("\t\t--delete: delete index and exit (no file path required)")
    print("\t\t--reload: delete and recreate index before loading")
    print("\t\t--recursive: scan subdirectories recursively")
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

    if not do_delete and len(args) < 1:
        print_usage(sys.argv[0])

    start_time: datetime = datetime.now()

    es_manager = ESManager()

    # ES 접속 테스트
    try:
        if not es_manager.es.ping():
            LOGGER.error("Elasticsearch 서버에 연결할 수 없습니다.")
            return -1
    except Exception as e:
        LOGGER.error(f"Elasticsearch 접속 실패: {e}")
        return -1

    try:
        if do_delete:
            if es_manager.do_exist_index():
                es_manager.delete_index()
                print(f"인덱스 '{es_manager.index_name}' 삭제 완료")
            else:
                print(f"인덱스 '{es_manager.index_name}'가 존재하지 않습니다")
            return 0
        if do_reload:
            es_manager.delete_index()
        es_manager.create_index()
    except Exception as e:
        LOGGER.error(e)
        return -1

    def process_file_iter(file_iter: Iterable[Path], skip_check: bool = False) -> Tuple[int, int]:
        """파일 iterator를 배치 처리하여 ES에 저장. 반환: (처리 수, 건너뜀 수)"""
        skipped_count = 0
        processed_count = 0
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

            # 존재 여부 검사 (skip_check가 False일 때만)
            existing_ids: Set[int] = set()
            if not skip_check:
                existing_ids = es_manager.get_existing_ids(list(file_stat_map.keys()))
                skipped_count += len(existing_ids)

            # 파일 파싱 (stat 결과 재사용)
            batch_data: Dict[int, Dict[str, Any]] = {}
            for inode, (file_path, st) in file_stat_map.items():
                if inode in existing_ids:
                    continue  # 이미 존재하면 건너뜀
                print(f"* {file_path}")
                data_item = Loader.read_file(file_path, stat_result=st)
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
                    print(f"  [배치 저장: {len(batch_data)}개, 건너뜀: {len(existing_ids)}개]")

        return processed_count, skipped_count

    for dir in args:
        dir_path = Path(dir)
        if not dir_path.exists():
            LOGGER.error("can't find such a file or directory '%s'", dir_path)
            return 0
        if not dir_path.is_relative_to(Loader.path_prefix):
            LOGGER.error(f"{dir_path} is not in $TM_WORK_DIR({Loader.path_prefix}).")
            continue

        print(f"====== {dir_path} ======")

        if do_recursive:
            # 전체 파일 등록 (generator 사용으로 메모리 효율화)
            file_iter = (p for p in dir_path.rglob("*") if p.is_file())
            processed, skipped_count = process_file_iter(file_iter, skip_check=do_reload)
            print(f"  총 {processed}개 파일 처리됨")
            if skipped_count > 0:
                print(f"  총 {skipped_count}개 중복 파일 건너뜀")
        else:
            # 1단계: 하위 디렉토리 각각에서 첫 번째 파일 1개씩 (즉시 저장)
            print("  [1단계] 하위 디렉토리별 샘플 파일 등록")
            sample_count = 0
            skipped1 = 0
            for subdir in sorted(dir_path.iterdir()):
                if subdir.is_dir():
                    print(f"    탐색 중: {subdir.name}/", end="", flush=True)
                    subdir_files = sorted([p for p in subdir.iterdir() if p.is_file()])
                    if subdir_files:
                        sample_file = subdir_files[0]
                        print(f" -> {sample_file.name}", end="", flush=True)
                        # 즉시 ES에 저장
                        processed, skipped = process_file_iter([sample_file], skip_check=do_reload)
                        if skipped > 0:
                            print(" (중복)")
                            skipped1 += skipped
                        else:
                            print(" (저장됨)")
                            sample_count += 1
                    else:
                        print(" -> (파일 없음)")
            print(f"    {sample_count}개 카테고리 샘플 저장, {skipped1}개 중복 건너뜀")

            # 2단계: 지정된 디렉토리에 바로 속한 파일들
            print("  [2단계] 현재 디렉토리 파일 등록")
            current_dir_files = sorted([p for p in dir_path.iterdir() if p.is_file()])
            print(f"    {len(current_dir_files)}개 파일 발견")
            _, skipped2 = process_file_iter(current_dir_files, skip_check=do_reload)
            if skipped2 > 0:
                print(f"    {skipped2}개 중복 파일 건너뜀")

        print("================================")

    # 모든 작업 완료 후 인덱스 refresh
    es_manager.refresh()

    end_time = datetime.now()
    Stat.index_total_time = (end_time - start_time).total_seconds()
    Stat.print()

    return 0


if __name__ == "__main__":
    sys.exit(main())
