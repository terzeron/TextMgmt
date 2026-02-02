#!/usr/bin/env python

import re
import subprocess
from pathlib import Path
from typing import List

import pypdf


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


def extract(file_path: Path) -> List[str]:
    """파일에서 ISBN을 추출하여 리스트로 반환"""
    ext = file_path.suffix.lower()
    if ext == ".txt":
        with file_path.open("r", encoding="utf-8", errors="ignore") as f:
            content = f.read()
            return search_in_content(content)
    elif ext == ".epub":
        result = subprocess.run(["unzip", "-p", str(file_path)], capture_output=True, text=True, errors="ignore")
        if result.returncode == 0:
            return search_in_content(result.stdout)
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
        result = subprocess.run(["djvutxt", str(file_path), "-"], capture_output=True, text=True, errors="ignore")
        if result.returncode == 0:
            return search_in_content(result.stdout)
    elif ext == ".hwp":
        result = subprocess.run(["strings", str(file_path)], capture_output=True, text=True, errors="ignore")
        if result.returncode == 0:
            return search_in_content(result.stdout)
    return []
