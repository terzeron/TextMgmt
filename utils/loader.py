#!/usr/bin/env python


import sys
import os
import re
import shutil
import logging.config
from datetime import datetime
from pathlib import Path
from typing import Dict, Any
import zipfile
import hashlib
import ebooklib
from ebooklib import epub
import pypdf
from docx import Document
from striprtf.striprtf import rtf_to_text
from bs4 import BeautifulSoup
from backend.es_manager import ESManager
from utils.stat import Stat

logging.config.fileConfig(Path(__file__).parent.parent / "logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger()
TEMP_DIR_PREFIX_PATH = Path("/mnt/ramdisk")


if "TM_WORK_DIR" not in os.environ:
    LOGGER.error("The environment variable TM_WORK_DIR is not set.")
    sys.exit(-1)


class Loader:
    TEXT_SIZE = 4096
    path_prefix = Path(os.environ["TM_WORK_DIR"])

    @staticmethod
    def read_from_text(file_path: Path) -> str:
        Stat.text_count += 1
        start_time = datetime.now()

        try:
            with file_path.open("r", encoding="utf-8") as infile:
                data = infile.read(Loader.TEXT_SIZE)
                data = data.replace("\ufeff", "")
                data = re.sub(r"\W+", " ", data)
        except UnicodeDecodeError as e:
            LOGGER.error(f"can't read unicode text from file '{file_path}', {e}")
            data = ""

        end_time = datetime.now()
        Stat.text_total_time += (end_time - start_time).total_seconds()

        return data

    @staticmethod
    def read_from_epub_with_extracting_zip(file_path: Path) -> str:
        result = ""
        print("extracting epub file as zip file")
        try:
            with zipfile.ZipFile(file_path, "r") as zip_ref:
                temp_dir_name = hashlib.md5(str(file_path).encode("utf-8")).hexdigest()[:7]
                if TEMP_DIR_PREFIX_PATH.is_dir():
                    temp_dir_path = TEMP_DIR_PREFIX_PATH / temp_dir_name
                else:
                    temp_dir_path = Path(temp_dir_name)
                zip_ref.extractall(temp_dir_path)
                root_file_path = temp_dir_path / "OEBPS" / "content.opf"
                metainf_file_path = temp_dir_path / "META-INF" / "container.xml"
                with metainf_file_path.open("r", encoding="utf-8") as metainf_file:
                    for line in metainf_file:
                        m = re.search(r'<rootfile[^>]*full-path="(?P<root_file>[^"]+)"', line)
                        if m:
                            root_file = m.group("root_file")
                            root_file_path = temp_dir_path / Path(root_file)
                            if not root_file_path.is_file():
                                LOGGER.error("can't file '%s' in epub file '%s'", root_file_path, file_path)
                                return ""
                #print(root_file_path)
                with root_file_path.open("r", encoding="utf-8") as infile:
                    for line in infile:
                        matches = re.findall(r'<(?:opf:)?item\s[^>]*href="(?P<chapter_file>[^"]*\.x?html)"[^>]*media-type="application/xhtml\+xml"', line)
                        for match in matches:
                            chapter_file = match
                            chapter_file_path = root_file_path.parent / chapter_file
                            # print(chapter_file_path)
                            if not chapter_file_path.is_file():
                                continue
                            with chapter_file_path.open("r", encoding="utf-8") as file:
                                content = file.read()
                                soup = BeautifulSoup(content, "html.parser")
                                text = soup.get_text()
                                if len(result) < Loader.TEXT_SIZE:
                                    result += text
                                else:
                                    break
                shutil.rmtree(temp_dir_path)
        except zipfile.BadZipFile as e:
            LOGGER.error(file_path)
            LOGGER.error(e)

        return result[:Loader.TEXT_SIZE]

    @staticmethod
    def read_from_epub(file_path: Path) -> str:
        Stat.normal_epub_count += 1
        start_time = datetime.now()

        result = ""
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
                # print(doc.get_body_content())
                soup = BeautifulSoup(doc.get_body_content(), "html.parser")
                text = soup.get_text()
                if len(result) < Loader.TEXT_SIZE:
                    result += text
                else:
                    break

            end_time = datetime.now()
            Stat.normal_epub_total_time += (end_time - start_time).total_seconds()

        except Exception as e:
            LOGGER.error(file_path)
            LOGGER.error(e)

            Stat.normal_epub_count -= 1
            Stat.zipped_epub_count += 1
            start_time = datetime.now()

            try:
                result = Loader.read_from_epub_with_extracting_zip(file_path)
            except epub.EpubException as e2:
                LOGGER.error(file_path)
                LOGGER.error(e2)

            end_time = datetime.now()
            Stat.zipped_epub_total_time += (end_time - start_time).total_seconds()

        result = re.sub(r"\W+", " ", result)

        return result[:Loader.TEXT_SIZE]

    @staticmethod
    def read_from_pdf(file_path: Path) -> str:
        Stat.pdf_count += 1
        start_time = datetime.now()

        result = ""
        with file_path.open("rb") as infile:
            try:
                reader = pypdf.PdfReader(infile)
                for page in reader.pages:
                    text = page.extract_text()
                    if len(result) < Loader.TEXT_SIZE:
                        result += text
                    else:
                        break
            except Exception as e:
                LOGGER.error(file_path)
                LOGGER.error(e)
        result = re.sub(r"\W+", " ", result)

        end_time = datetime.now()
        Stat.pdf_total_time += (end_time - start_time).total_seconds()

        return result[:Loader.TEXT_SIZE]

    @staticmethod
    def read_from_html(file_path: Path) -> str:
        Stat.html_count += 1
        start_time = datetime.now()

        content = ""
        with file_path.open("r") as infile:
            content = infile.read()

        soup = BeautifulSoup(content, "html.parser")
        result = soup.get_text()
        result = re.sub(r"\W+", " ", result)

        end_time = datetime.now()
        Stat.html_total_time += (end_time - start_time).total_seconds()

        return result[:Loader.TEXT_SIZE]

    @staticmethod
    def read_from_docx(file_path: Path) -> str:
        Stat.docx_count += 1
        start_time = datetime.now()

        result = ""
        doc = Document(str(file_path))
        # print(doc)
        for paragraph in doc.paragraphs:
            text = paragraph.text
            if len(result) < Loader.TEXT_SIZE:
                result += text
            else:
                break
        result = re.sub(r"\W+", " ", result)

        end_time = datetime.now()
        Stat.docx_total_time += (end_time - start_time).total_seconds()

        return result[:Loader.TEXT_SIZE]

    @staticmethod
    def read_from_rtf(file_path: Path) -> str:
        Stat.rtf_count += 1
        start_time = datetime.now()

        result = ""
        # print(file_path)
        try:
            with file_path.open("r", encoding="utf-8") as infile:
                doc = infile.read()
                result = rtf_to_text(doc)
        except UnicodeDecodeError as e:
            LOGGER.error(file_path)
            LOGGER.error(e)
        result = re.sub(r"\W+", " ", result)
        # print(result[:TEXT_SIZE])

        end_time = datetime.now()
        Stat.rtf_total_time += (end_time - start_time).total_seconds()

        return result[:Loader.TEXT_SIZE]

    @staticmethod
    def read_from_image(file_path: Path) -> str:
        Stat.image_count += 1

        return ""

    @staticmethod
    def read_file(file_path: Path) -> Dict[int, Dict[str, Any]]:
        if file_path.is_file():
            # print(child_path)
            sys.stdout.flush()
            # read metadata of each file
            st = file_path.stat()
            inode_num = st.st_ino
            file_size = st.st_size
            category = file_path.parent.name
            m = re.search(r"^\[(?P<author>[\]]+)\]\s*(?P<title>.+)$", file_path.stem)
            if m:
                author = m.group("author")
                title = m.group("title")
            else:
                author = ""
                title = file_path.stem

            file_type = file_path.suffix[1:]
            # read content of each file
            if file_type == "txt":
                summary = Loader.read_from_text(file_path)
            elif file_type == "epub":
                summary = Loader.read_from_epub(file_path)
            elif file_type == "pdf":
                summary = Loader.read_from_pdf(file_path)
            elif file_type == "docx":
                summary = Loader.read_from_docx(file_path)
            elif file_type == "rtf":
                summary = Loader.read_from_rtf(file_path)
            elif file_type == "html":
                summary = Loader.read_from_html(file_path)
            elif file_type in ("jpg", "jpeg", "png", "gif", "webp", "bmp", "tiff", "svg"):
                summary = Loader.read_from_image(file_path)
            else:
                return {}

            return {
                inode_num: {
                    "category": category,
                    "title": title,
                    "author": author,
                    "file_path": str(file_path.relative_to(Loader.path_prefix)),
                    "file_type": file_type,
                    "file_size": int(file_size),
                    "summary": summary,
                    "updated_time": datetime.now().isoformat()
                }
            }

        return {}

    @staticmethod
    def read_files(path: Path, num_files: int = sys.maxsize) -> Dict[int, Dict[str, Any]]:
        data: Dict[int, Dict[str, Any]] = {}
        if path.is_dir():
            file_path_list = list(path.rglob("*"))
        else:
            file_path_list = [path]

        file_count = 0
        for child_path in file_path_list[:num_files]:
            data_item = Loader.read_file(child_path)
            data.update(data_item)
            file_count += 1
            if file_count >= num_files:
                break

        return data


def print_usage(program_name: str):
    print(f"Usage:\t{program_name}\t<index name>\t<file or directory path>\n")
    sys.exit(0)


def main() -> int:
    if len(sys.argv) < 3:
        print_usage(sys.argv[0])

    path = Path(sys.argv[2])
    if not path.exists():
        LOGGER.error("can't find such a file or directory '%s'", path)
        return 0

    data = Loader.read_files(path)

    start_time = datetime.now()
    es_manager = ESManager()
    try:
        es_manager.create_index()
    except Exception as e:
        LOGGER.error(e)
    es_manager.insert(data)

    end_time = datetime.now()
    Stat.index_count = len(data)
    Stat.index_total_time = (end_time - start_time).total_seconds()

    Stat.print()

    return 0


if __name__ == "__main__":
    sys.exit(main())
