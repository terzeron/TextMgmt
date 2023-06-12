#!/usr/bin/env python


import sys
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
import PyPDF2 
from docx import Document
from pyth.plugins.rtf15.reader import Rtf15Reader
from bs4 import BeautifulSoup
from backend.es_manager import ESManager


logging.config.fileConfig("../logging.conf")
LOGGER = logging.getLogger()
TEXT_SIZE = 4096
TEMP_DIR_PREFIX_PATH = Path("/mnt/ramdisk")


def read_from_text(file_path: Path) -> str:
    with file_path.open("r", encoding="utf-8") as infile:
        data = infile.read(TEXT_SIZE)
        data = data.replace("\ufeff", "")
        data = re.sub(r"\W+", " ", data)
        return data


def read_from_epub_with_extracting_zip(file_path: Path) -> str:
    result = ""
    print("extracting epub file as zip file")
    try:
        with zipfile.ZipFile(file_path, "r") as zip_ref:
            temp_dir_name = hashlib.md5(str(file_path).encode("utf-8")).hexdigest()[:7]
            temp_dir_path = TEMP_DIR_PREFIX_PATH / temp_dir_name
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
                            LOGGER.error(f"can't file '{root_file_path}' in epub file '{file_path}'")
                            return ""
            #print(root_file_path)
            with root_file_path.open("r", encoding="utf-8") as infile:
                for line in infile:
                    matches = re.findall(r'<(?:opf:)?item\s[^>]*href="(?P<chapter_file>[^"]*\.x?html)"[^>]*media-type="application/xhtml\+xml"', line)
                    for match in matches:
                        chapter_file = match
                        chapter_file_path = root_file_path.parent / chapter_file
                        #print(chapter_file_path)
                        if not chapter_file_path.is_file():
                            continue
                        with chapter_file_path.open("r", encoding="utf-8") as file:
                            content = file.read()
                            soup = BeautifulSoup(content, "html.parser")
                            text = soup.get_text()
                            if len(result) < TEXT_SIZE:
                                result += text
                            else:
                                break
            shutil.rmtree(temp_dir_path)
    except zipfile.BadZipFile as e:
        LOGGER.error(file_path)
        LOGGER.error(e)

    return result


def read_from_epub(file_path: Path) -> str:
    try:
        result = ""
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
            #print(doc.get_body_content())
            soup = BeautifulSoup(doc.get_body_content(), "html.parser")
            text = soup.get_text()
            if len(result) < TEXT_SIZE:
                result += text
            else:
                break
    except Exception as e:
        LOGGER.error(file_path)
        LOGGER.error(e)
        try:
            result = read_from_epub_with_extracting_zip(file_path)
        except epub.EpubException as e2:
            LOGGER.error(file_path)
            LOGGER.error(e2)
    result = re.sub(r"\W+", " ", result)
    return result[:TEXT_SIZE]


def read_from_pdf(file_path: Path) -> str:
    result = ""
    with file_path.open("rb") as infile:
        try:
            reader = PyPDF2.PdfReader(infile)
            for page in reader.pages:
                text = page.extract_text()
                if len(result) < TEXT_SIZE:
                    result += text
                else:
                    break
        except Exception as e:
            LOGGER.error(file_path)
            LOGGER.error(e)
    result = re.sub(r"\W+", " ", result)
    return result[:TEXT_SIZE]


def read_from_html(file_path: Path) -> str:
    content = ""
    with file_path.open("rb") as infile:
        content = infile.read()

    soup = BeautifulSoup(content, "html.parser")
    result = soup.get_text()
    result = re.sub(r"\W+", " ", result)
    return result[:TEXT_SIZE]


def read_from_docx(file_path: Path) -> str:
    result = ""
    doc = Document(file_path)
    #print(doc)
    for paragraph in doc.paragraphs:
        text = paragraph.text
        if len(result) < TEXT_SIZE:
            result += text
        else:
            break
    result = re.sub(r"\W+", " ", result)
    return result[:TEXT_SIZE]


def read_from_rtf(file_path: Path) -> str:
    result = ""
    with file_path.open("rb") as infile:
        doc = Rtf15Reader.read(infile)
        for p in doc.content:
            for text in p.content:
                for t in text.content:
                    result += t
    result = re.sub(r"\W+", " ", result)
    return result[:TEXT_SIZE]


def read_data(path: Path) -> Dict[int, Dict[str, Any]]:
    data = {}
    if path.is_dir():
        file_path_list = path.rglob("*")
    else:
        file_path_list = [path] 

    for child_path in list(file_path_list):
        if child_path.is_file():
            #print(child_path)
            sys.stdout.flush()
            # read metadata of each file
            st = child_path.stat()
            inode_num = st.st_ino
            size = st.st_size
            dir_path_str = child_path.parent.name
            name = child_path.stem
            ext = child_path.suffix[1:]
            # read content of each file
            if ext == "txt":
                content = read_from_text(child_path)
            elif ext == "epub":
                content = read_from_epub(child_path)
            elif ext == "pdf":
                content = read_from_pdf(child_path)
            elif ext == "docx":
                content = read_from_docx(child_path)
            elif ext == "rtf":
                content = read_from_rtf(child_path)
            elif ext == "html":
                content = read_from_html(child_path)
            else:
                continue
            data[inode_num] = {"dir": dir_path_str, "name": name, "ext": ext, "size": int(size), "content": content, "updated_time": datetime.now().isoformat()}
            #print(data[inode_num])

    return data


def print_usage(program_name: str):
    print(f"Usage:\t{program_name}\t<file or directory path>\n")
    sys.exit(0)


def main() -> int:
    if len(sys.argv) < 2:
        print_usage(sys.argv[0])

    path = Path(sys.argv[1])
    if not path.exists():
        LOGGER.error("can't find such a file or directory '%s'", path)
        return 0

    data = read_data(path)
    es = ESManager("tm")
    try:
        es.create_index()
    except Exception as e:
        LOGGER.error(e)
    es.load(data)
    return 0


if __name__ == "__main__":
    sys.exit(main())
