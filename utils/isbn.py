#!/usr/bin/env python

import re
import subprocess
import zipfile
from pathlib import Path
from typing import List, Optional

import pypdf
from bs4 import BeautifulSoup

# 앞뒤로 읽을 바이트 크기 (8KB)
HEAD_TAIL_SIZE = 8 * 1024


def read_head_tail_from_file(file_path: Path, size: int = HEAD_TAIL_SIZE) -> str:
    """파일의 앞/뒤 부분만 읽기 (바이트 기반)"""
    with file_path.open("rb") as f:
        f.seek(0, 2)  # SEEK_END
        file_size = f.tell()
        f.seek(0)  # 처음으로

        if file_size <= size * 2:
            # 작은 파일은 전체를 읽음
            content_bytes = f.read()
            return content_bytes.decode("utf-8", errors="ignore")
        else:
            # 큰 파일은 head + tail만 읽음
            head_bytes = f.read(size)
            f.seek(-size, 2)  # SEEK_END
            tail_bytes = f.read(size)
            head = head_bytes.decode("utf-8", errors="ignore")
            tail = tail_bytes.decode("utf-8", errors="ignore")
            return head + tail


def read_head_tail_from_content(content: str, size: int = HEAD_TAIL_SIZE) -> str:
    """문자열의 앞/뒤 부분만 추출"""
    if len(content) <= size * 2:
        return content
    return content[:size] + content[-size:]


def validate_isbn10(isbn: str) -> bool:
    if isbn in ("1111111111", "1100101101"):
        return False

    if len(isbn) != 10 or not all(c.isdigit() or c == 'X' for c in isbn):
        return False

    total = 0
    for i in range(9):
        if not isbn[i].isdigit():
            return False
        total += int(isbn[i]) * (10 - i)

    check_digit = 10 if isbn[-1] == 'X' else int(isbn[-1])
    total += check_digit
    return total % 11 == 0


def validate_isbn13(isbn: str) -> bool:
    if len(isbn) != 13 or not isbn.isdigit():
        return False

    total = 0
    for i, digit in enumerate(isbn):
        weight = 1 if i % 2 == 0 else 3
        total += int(digit) * weight
    return total % 10 == 0


def validate_isbn(isbn: str) -> bool:
    if len(isbn) == 10 and validate_isbn10(isbn) or len(isbn) == 13 and validate_isbn13(isbn):
        return True
    return False


def search_in_content(content: str) -> List[str]:
    # 구분자를 사용하는 패턴과 구분자가 명확하게 없는 패턴
    # 구분자가 존재하면 그룹 숫자간 공백이 존재할 수 있음
    isbn_pattern = r'''
                        (?:[:_]?|\b)
                        (
                            (?:
                                9\s?7\s?[8B9]
                                \s?[\-─–－‐]{1,3}\s?
                            )?
                            (?:[8B]\s?9|[1Il]\s?[1Il])
                            \s?[\-─–－‐]{1,3}\s?
                            (?:[\dIOBlsⅩ]\s?){1,7}
                            \s?[\-─–－‐]{1,3}\s?
                            (?:[\dIOBlsⅩ]\s?){1,6}
                            \s?[\-─–－‐]{1,3}\s?
                            [\dIOBlsⅩX]
                            |
                            (?:
                                97[8B9]
                                [\-\s─–－‐]{0,3}
                            )?
                            (?:[8B]9|11|[Il][Il])
                            [\-\s─–－‐]{0,3}
                            [\dIOBlsⅩ]{1,7}
                            [\-\s─–－‐]{0,3}
                            [\dIOBlsⅩ]{1,6}
                            [\-\s─–－‐]{0,3}
                            [\dIOBlsⅩ]{0,7}
                            [\-\s─–－‐]{0,3}
                            [\dIOBlsⅩ]{0,7}
                            [\-\s─–－‐]{0,3}
                            [\dIOBlsⅩX]
                        )
                        \b'''

    result_list: List[str] = []
    for match in re.finditer(isbn_pattern, content, re.VERBOSE):
        result = match.group(1)
        result = re.sub(r"(l|\bI\b)", "1", result)
        result = re.sub(r"s|B", "8", result)
        result = re.sub(r"O", "0", result)
        result = re.sub(r"Ⅹ", "X", result)
        result = re.sub(r"[^\dX]+", " ", result)
        result = re.sub(r"(^ |[^\dX ])", "", result)
        m = re.search(r'(?P<isbn>97[89] ?(\d ?){10})\b(?:\d+)?', result)
        if m:
            isbn = re.sub(r'[^\dX]', '', m.group("isbn"))
            if validate_isbn(isbn):
                result_list.append(isbn)
        else:
            m = re.search(r'^(?P<isbn>(8 ?9 ?(\d ?){7}|[01] ?(\d ?){8})[\dX])\b(?:\d)?', result)
            if m:
                isbn = re.sub(r'[^\dX]', '', m.group("isbn"))
                if validate_isbn(isbn):
                    result_list.append(isbn)
    return result_list


def extract_from_epub(file_path: Path) -> List[str]:
    """EPUB에서 ISBN 추출: OPF 메타데이터 먼저 확인 후 앞뒤 챕터 검색"""
    try:
        with zipfile.ZipFile(file_path, "r") as zf:
            # 1. OPF 파일 찾기
            opf_path = None
            for name in zf.namelist():
                if name.endswith(".opf"):
                    opf_path = name
                    break

            if not opf_path:
                return []

            # 2. OPF 메타데이터에서 ISBN 찾기
            opf_content = zf.read(opf_path).decode("utf-8", errors="ignore")
            isbn_list = search_in_content(opf_content)
            if isbn_list:
                return isbn_list

            # 3. 메타데이터에 없으면 spine에서 앞뒤 챕터 파싱
            soup = BeautifulSoup(opf_content, "html.parser")
            manifest = {}
            for item in soup.find_all("item"):
                item_id = item.get("id")
                href = item.get("href")
                media_type = item.get("media-type", "")
                if item_id and href and "html" in media_type:
                    manifest[item_id] = href

            spine_ids = [itemref.get("idref") for itemref in soup.find_all("itemref")]
            spine_hrefs = [manifest[sid] for sid in spine_ids if sid in manifest]

            if not spine_hrefs:
                return []

            # 앞 1개 + 뒤 1개 챕터만 읽기
            chapters_to_read = spine_hrefs[:1] + spine_hrefs[-1:]
            opf_dir = "/".join(opf_path.split("/")[:-1])

            content = ""
            for href in chapters_to_read:
                chapter_path = f"{opf_dir}/{href}" if opf_dir else href
                try:
                    chapter_content = zf.read(chapter_path).decode("utf-8", errors="ignore")
                    chapter_soup = BeautifulSoup(chapter_content, "html.parser")
                    content += chapter_soup.get_text() + "\n"
                except KeyError:
                    continue

            return search_in_content(content)
    except Exception:
        return []


def extract_from_djvu(file_path: Path) -> List[str]:
    """DJVU에서 ISBN 추출: 앞 5페이지 + 뒤 5페이지만 추출"""
    try:
        # 총 페이지 수 확인
        result = subprocess.run(
            ["djvused", str(file_path), "-e", "n"],
            capture_output=True, text=True, errors="ignore"
        )
        if result.returncode != 0:
            return []

        try:
            total_pages = int(result.stdout.strip())
        except ValueError:
            return []

        # 앞 5페이지
        head_pages = ",".join(str(i) for i in range(1, min(6, total_pages + 1)))
        # 뒤 5페이지
        tail_start = max(1, total_pages - 4)
        tail_pages = ",".join(str(i) for i in range(tail_start, total_pages + 1))

        content = ""
        for pages in [head_pages, tail_pages]:
            result = subprocess.run(
                ["djvutxt", f"--page={pages}", str(file_path)],
                capture_output=True, text=True, errors="ignore"
            )
            if result.returncode == 0:
                content += result.stdout

        return search_in_content(content) if content else []
    except FileNotFoundError:
        # djvused or djvutxt command not found
        return []


def extract_from_hwp(file_path: Path) -> List[str]:
    """HWP에서 ISBN 추출: head + tail로 앞뒤만 추출"""
    size = str(HEAD_TAIL_SIZE)

    # head -c 8192
    head_result = subprocess.run(
        f"strings '{file_path}' | head -c {size}",
        shell=True, capture_output=True, text=True, errors="ignore"
    )
    # tail -c 8192
    tail_result = subprocess.run(
        f"strings '{file_path}' | tail -c {size}",
        shell=True, capture_output=True, text=True, errors="ignore"
    )

    content = head_result.stdout + tail_result.stdout
    return search_in_content(content) if content else []


def extract(file_path: Path, content: Optional[str] = None) -> List[str]:
    """파일에서 ISBN을 추출하여 리스트로 반환

    Args:
        file_path: 파일 경로
        content: 이미 읽은 파일 내용 (TXT 파일의 경우 중복 I/O 방지용)
    """
    ext = file_path.suffix.lower()
    if ext == ".txt":
        if content:
            # 이미 읽은 content가 있으면 앞뒤만 추출하여 사용
            search_content = read_head_tail_from_content(content)
        else:
            search_content = read_head_tail_from_file(file_path)
        return search_in_content(search_content)
    elif ext == ".epub":
        return extract_from_epub(file_path)
    elif ext == ".pdf":
        content = ""
        try:
            with file_path.open("rb") as f:
                reader = pypdf.PdfReader(f)
                total_pages = len(reader.pages)
                # 앞 5페이지 추출
                for i in range(min(5, total_pages)):
                    text = reader.pages[i].extract_text()
                    if text:
                        content += text
                # 뒤 5페이지 추출
                for i in range(max(0, total_pages - 5), total_pages):
                    text = reader.pages[i].extract_text()
                    if text:
                        content += text
        except Exception:
            pass
        if content:
            return search_in_content(content)
    elif ext == ".djvu":
        return extract_from_djvu(file_path)
    elif ext == ".hwp":
        return extract_from_hwp(file_path)
    return []
