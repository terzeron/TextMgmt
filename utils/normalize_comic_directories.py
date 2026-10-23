#!/usr/bin/env python3
"""Normalize comic directory names from contained filenames and bookstore results."""

from __future__ import annotations

import argparse
import csv
import json
import re
import sys
import time
import unicodedata
from collections import Counter
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path
from uuid import uuid4

REPO_ROOT = Path(__file__).resolve().parent.parent
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from backend.bookstore import AbstractBookstore, AladinBookstore, KyoboBookstore, Yes24Bookstore  # noqa: E402

TARGET_CATEGORIES = ("3_미정리", "4_대기", "9_temp")
BOOKSTORES = (("yes24", Yes24Bookstore), ("aladin", AladinBookstore), ("kyobo", KyoboBookstore))
STORE_FIELDS = ("title", "author", "category", "url", "isbn")
BOOK_EXTENSIONS = {".pdf", ".epub"}
DEFAULT_NEGATIVE_CACHE_SECONDS = 7 * 24 * 60 * 60
AUTHOR_TAGS = {"만화", "comic", "코믹", "webtoon", "scan", "스캔", "스캔본", "한글", "영문", "일문", "정발", "번역"}
VOLUME_SUFFIX = re.compile(
    r"\s*(?:[-_ ]+)?(?:\d{1,4}(?:\s*[-~]\s*\d{1,4})?\s*(?:권|화|편|장|부)|"
    r"(?:vol(?:ume)?|ch(?:apter)?)\.?\s*\d+).*$",
    re.IGNORECASE,
)
CHAPTER_PREFIX = re.compile(r"^(?:제\s*)?\d{1,5}\s*(?:화|권|장|편)\s*[-_. ]*", re.IGNORECASE)
NUMBER_TAG = re.compile(r"^\[\d{2,}\]\s*")
DIRECTORY_METADATA = re.compile(
    r"(?:\s+|\s*\()(?="
    r"\d{1,4}\s*(?:[-~]\s*\d{1,4})\s*(?:(?:권|화|편|장)?(?:완결|완|미완|외전|외)?)|"
    r"\d{1,4}\s*(?:권|화|편|장)(?:완결|완|외전|외)?|"
    r"(?:완결|미완|연재\s*중?|누락|잘림|미수록)(?=$|[\s(\[~,-])"
    r")"
)
VOLUME_NUMBER = re.compile(r"(?:\bvol(?:ume)?\.?\s*(\d{1,4})|(?<!\d)(\d{1,4})\s*권)", re.IGNORECASE)
CHAPTER_NUMBER = re.compile(r"(?:\bch(?:apter)?\.?\s*(\d{1,5})|(?<!\d)(\d{1,5})\s*화)", re.IGNORECASE)
TRAILING_SEQUENCE_NUMBER = re.compile(r"(?<!\d)(\d{1,4})(?:\s*[-_.]?\s*(?:전|후|상|중|하))?\s*$")
EXTRA_MATERIAL = re.compile(r"(?:외전|번외|특전|특별편|보너스|\bextra\b|\bspecial\b|\bside[ _-]*story\b)", re.IGNORECASE)
INCOMPLETE_STATUS = re.compile(r"(?:미완|연재\s*중?|중단|누락|잘림|미수록)")
COMPLETE_STATUS = re.compile(r"(?:완결|(?<!미)완(?=외?(?:[\s,)\]]|$)))")
EXTRA_STATUS = re.compile(r"(?:외전|번외|특전|특별편|보너스|외(?=[\s,)\]]|$))")
AUTHOR_ROLE = re.compile(r"(?:글|그림|저|원작|스토리|만화)(?=$|[\s,/&·])")
NON_KOREAN_SCRIPT = re.compile(r"[A-Za-zぁ-ゟ゠-ヿ一-龯]")


@dataclass
class Proposal:
    source: Path
    target: Path | None
    confidence: float
    sample_count: int
    status: str
    evidence: str = ""


@dataclass
class IdentityResult:
    title: str | None
    author: str | None
    confidence: float
    file_count: int
    failure_status: str | None = None
    failure_reason: str = ""


def parse_filename(path: Path) -> tuple[str, str | None] | None:
    stem = unicodedata.normalize("NFKC", path.stem).strip()
    stem = NUMBER_TAG.sub("", stem)
    author = None
    match = re.match(r"^\[([^\]]+)\]\s*(.+)$", stem)
    if match and match.group(1).strip().casefold() not in AUTHOR_TAGS:
        tag = match.group(1).strip()
        if re.search(r"[A-Za-z가-힣]", tag):
            author, stem = tag, match.group(2).strip()
    stem = CHAPTER_PREFIX.sub("", stem)
    stem_before_volume = stem.strip(" _-.()[]")
    stem = VOLUME_SUFFIX.sub("", stem).strip(" _-.()[]")
    if stem == stem_before_volume:
        stem = TRAILING_SEQUENCE_NUMBER.sub("", stem).strip(" _-.()[]")
    if not re.sub(r"\W", "", stem):
        return None
    return stem, author


def title_key(value: str) -> str:
    value = unicodedata.normalize("NFKC", value).casefold()
    return "".join(char for char in value if char.isalnum())


def directory_author(name: str) -> str | None:
    match = re.match(r"^\[([^\]]+)\]\s*", name)
    if not match or match.group(1).strip().casefold() in AUTHOR_TAGS:
        return None
    tag = match.group(1).strip()
    return tag if re.search(r"[A-Za-z가-힣]", tag) else None


def korean_author(value: str | None) -> str | None:
    if not value:
        return None
    cleaned = AUTHOR_ROLE.sub("", unicodedata.normalize("NFKC", value))
    cleaned = re.sub(r"\s*[,/&·]+\s*$", "", cleaned).strip()
    if not re.search(r"[가-힣]", cleaned) or NON_KOREAN_SCRIPT.search(cleaned):
        return None
    return cleaned


def directory_title(name: str) -> str:
    title = re.sub(r"^\[[^\]]+\]\s*", "", name)
    metadata = DIRECTORY_METADATA.search(title)
    if metadata:
        metadata_start = metadata.start()
        prefix = title[:metadata_start]
        if prefix.rfind("(") > prefix.rfind(")"):
            metadata_start = prefix.rfind("(")
        title = title[:metadata_start]
    title = re.sub(r"\s*\[[^\]]*\]\s*$", "", title)
    title = re.sub(r"\s*\([^)]*(?:\d|완|미완|연재|권|화|부)[^)]*\)\s*$", "", title)
    title = re.sub(r"\s+\d{1,4}(?:\s*[-~]\s*\d{1,4})?\s*(?:권|화|편|장|부).*$", "", title)
    title = re.sub(r"\s+(?:완결|완|미완|연재중?).*$", "", title, flags=re.IGNORECASE)
    return title.strip(" _-.")


def local_author_references(root: Path) -> list[tuple[str, str]]:
    references = []
    for category in root.iterdir():
        if not category.is_dir() or category.is_symlink():
            continue
        for directory in category.iterdir():
            if not directory.is_dir() or directory.is_symlink():
                continue
            author = korean_author(directory_author(directory.name))
            title = title_key(directory_title(directory.name))
            if author and title:
                references.append((title, author))
    return references


def matching_local_author(title: str, references: list[tuple[str, str]]) -> str | None:
    key = title_key(title)
    exact = {author for candidate, author in references if candidate == key}
    if len(exact) == 1:
        return exact.pop()
    variants = {author for candidate, author in references if len(key) >= 2 and (candidate.startswith(key) or key.startswith(candidate))}
    return variants.pop() if len(variants) == 1 else None


def existing_metadata(name: str) -> str:
    match = DIRECTORY_METADATA.search(name)
    if not match:
        return ""
    return name[match.start() :].strip()


def matched_numbers(pattern: re.Pattern[str], value: str) -> list[int]:
    numbers = []
    for match in pattern.finditer(value):
        number = next(group for group in match.groups() if group is not None)
        parsed = int(number)
        if parsed > 0:
            numbers.append(parsed)
    return numbers


def normalized_directory_metadata(directory: Path) -> str | None:
    volumes: list[int] = []
    chapters: list[int] = []
    fallback_volumes: list[int] = []
    main_names: list[str] = []
    has_extra_entry = False

    for entry in directory.iterdir():
        if entry.is_symlink():
            continue
        if entry.is_file():
            if entry.suffix.lower() not in BOOK_EXTENSIONS:
                continue
            name = entry.stem
        elif entry.is_dir():
            name = entry.name
        else:
            continue

        normalized_name = unicodedata.normalize("NFKC", name)
        if EXTRA_MATERIAL.search(normalized_name):
            has_extra_entry = True
            continue

        main_names.append(normalized_name)
        found_volumes = matched_numbers(VOLUME_NUMBER, normalized_name)
        found_chapters = matched_numbers(CHAPTER_NUMBER, normalized_name)
        volumes.extend(found_volumes)
        chapters.extend(found_chapters)
        if not found_volumes and not found_chapters:
            trailing = TRAILING_SEQUENCE_NUMBER.search(normalized_name)
            if trailing and int(trailing.group(1)) > 0:
                fallback_volumes.append(int(trailing.group(1)))

    if chapters:
        first_number, last_number, unit = min(chapters), max(chapters), "화"
    elif volumes:
        first_number, last_number, unit = min(volumes), max(volumes), "권"
    elif len(set(fallback_volumes)) >= 2:
        first_number, last_number, unit = min(fallback_volumes), max(fallback_volumes), "권"
    else:
        return None

    metadata = existing_metadata(directory.name)
    if INCOMPLETE_STATUS.search(metadata):
        completion = "미완"
    elif COMPLETE_STATUS.search(metadata):
        completion = "완"
    elif any("완결" in name for name in main_names):
        completion = "완"
    else:
        completion = ""

    has_extras = EXTRA_STATUS.search(metadata) is not None or (has_extra_entry and bool(main_names))
    flags = completion + ("외" if has_extras else "")
    flag_suffix = f" {flags}" if flags else ""
    return f"({first_number}-{last_number}{unit}{flag_suffix})"


def infer_identity(directory: Path) -> IdentityResult:
    book_files = [path for path in sorted(directory.iterdir()) if path.is_file() and not path.is_symlink() and path.suffix.lower() in BOOK_EXTENSIONS]
    parsed = [parse_filename(path) for path in book_files]
    parsed = [item for item in parsed if item]
    if len(parsed) < 2:
        fallback_title = directory_title(directory.name)
        parsed_matches_directory = all(titles_equivalent(title, fallback_title) for title, _author in parsed)
        if len(book_files) >= 2 and normalized_directory_metadata(directory) and fallback_title and parsed_matches_directory:
            authors = Counter(author for _title, author in parsed if author)
            author = authors.most_common(1)[0][0] if authors else directory_author(directory.name)
            return IdentityResult(fallback_title, author, 0.8, len(book_files))
        status = "SKIP_INSUFFICIENT_FILES" if len(book_files) < 2 else "SKIP_UNPARSEABLE_FILENAMES"
        reason = f"book files: {len(book_files)}; parsed filenames: {len(parsed)}"
        return IdentityResult(None, None, 0.0, len(book_files), status, reason)

    groups: dict[str, list[tuple[str, str | None]]] = {}
    for title, author in parsed:
        groups.setdefault(title_key(title), []).append((title, author))
    key, matches = max(groups.items(), key=lambda item: len(item[1]))
    confidence = len(matches) / len(parsed)
    if not key:
        reason = f"book files: {len(book_files)}; parsed filenames: {len(parsed)}"
        return IdentityResult(None, None, confidence, len(book_files), "SKIP_UNPARSEABLE_FILENAMES", reason)
    if confidence < 0.6:
        reason = f"dominant filename title: {len(matches)}/{len(parsed)}"
        return IdentityResult(None, None, confidence, len(book_files), "SKIP_AMBIGUOUS_FILE_TITLES", reason)

    title = Counter(title for title, _ in matches).most_common(1)[0][0]
    current_title = directory_title(directory.name)
    if not titles_equivalent(title, current_title):
        reason = f"filename title: {title}; directory title: {current_title}"
        return IdentityResult(None, None, confidence, len(book_files), "SKIP_TITLE_MISMATCH", reason)
    authors = Counter(author for _, author in matches if author)
    author, author_count = authors.most_common(1)[0] if authors else (None, 0)
    if author and author_count / len(matches) < 0.6:
        author = None
    return IdentityResult(title, author, confidence, len(book_files))


def bookstore_title_base(found_title: str) -> str:
    found_title = unicodedata.normalize("NFKC", found_title).strip()
    found_title = re.sub(r"\s*(?:\(\s*|\[\s*)\d{1,4}(?:\s*[-~]\s*\d{1,4})?\s*(?:권|화|편|장)(?:\s*\)|\s*\])\s*$", "", found_title)
    found_title = re.sub(r"\s+\d{1,4}\s*(?:권|화|편|장)?\s*$", "", found_title)
    return found_title.strip().rstrip(".．。").rstrip()


def title_variants(value: str) -> set[str]:
    normalized = unicodedata.normalize("NFKC", value).strip()
    candidates = {normalized}
    candidates.add(re.sub(r"\s+[-:：]\s*[A-Za-z][A-Za-z0-9\s'.,!?&-]*$", "", normalized).strip())
    candidates.add(re.sub(r"\s*\([^()]*(?:판|편|본|판본|[A-Za-z가-힣])[^()]*\)\s*$", "", normalized).strip())
    return {key for candidate in candidates if (key := title_key(candidate))}


def titles_equivalent(left: str, right: str) -> bool:
    return bool(title_variants(left) & title_variants(right))


def bookstore_title_matches(found_title: str, candidate_title: str) -> bool:
    return titles_equivalent(bookstore_title_base(found_title), candidate_title)


def filesystem_safe_name(value: str) -> str | None:
    value = unicodedata.normalize("NFKC", value)
    replacements = {"/": "／", "\\": "＼", ":": "：", "*": "＊", "?": "？", '"': "＂", "<": "＜", ">": "＞", "|": "｜"}
    value = "".join(replacements.get(char, char) for char in value)
    value = re.sub(r"[\x00-\x1f\x7f]", "", value)
    value = re.sub(r"\s+", " ", value).strip().rstrip(" .")
    if not value or value in {".", ".."} or len(value.encode("utf-8")) > 255:
        return None
    return value


class BookstoreSearchCache:
    """Append raw store results and short-lived empty results by query and store."""

    def __init__(self, path: Path, refresh: bool = False, negative_ttl_seconds: int = DEFAULT_NEGATIVE_CACHE_SECONDS):
        self.path = path
        self.negative_ttl_seconds = max(0, negative_ttl_seconds)
        self.entries: dict[tuple[str, str], dict[str, object]] = {}
        self.hits = 0
        self.searches = 0
        self.search_seconds = 0.0
        self.last_request_completed_at: float | None = None
        if refresh:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("", encoding="utf-8")
        elif path.exists():
            self._load()

    def _load(self) -> None:
        try:
            with self.path.open(encoding="utf-8") as stream:
                for line in stream:
                    try:
                        item = json.loads(line)
                    except json.JSONDecodeError:
                        continue
                    key = (str(item.get("query", "")), str(item.get("store", "")))
                    results = item.get("results")
                    valid = isinstance(results, list) and all(isinstance(row, dict) and all(field in row for field in STORE_FIELDS) for row in results)
                    if key[0] and key[1] and valid:
                        cached_at = item.get("cached_at", 0)
                        if isinstance(cached_at, str):
                            try:
                                cached_at = datetime.fromisoformat(cached_at).timestamp()
                            except ValueError:
                                cached_at = 0
                        self.entries[key] = {"results": results, "cached_at": float(cached_at)}
        except OSError as exc:
            print(f"Cache read failed: {exc}", file=sys.stderr)

    def get(self, query: str, store: str) -> list[dict[str, str]] | None:
        key = (query, store)
        entry = self.entries.get(key)
        if entry is None:
            return None
        results = entry["results"]
        if not results and time.time() - float(entry["cached_at"]) > self.negative_ttl_seconds:
            del self.entries[key]
            return None
        self.hits += 1
        return results  # type: ignore[return-value]

    def put(self, query: str, store: str, results: list[dict[str, str]]) -> None:
        cached_at = time.time()
        item = {"query": query, "store": store, "cached_at": cached_at, "results": results}
        try:
            self.path.parent.mkdir(parents=True, exist_ok=True)
            with self.path.open("a", encoding="utf-8") as stream:
                stream.write(json.dumps(item, ensure_ascii=False) + "\n")
            self.entries[(query, store)] = {"results": results, "cached_at": cached_at}
        except OSError as exc:
            print(f"Cache write failed: {exc}", file=sys.stderr)

    def wait_for_request_slot(self, delay: float) -> None:
        if delay <= 0 or self.last_request_completed_at is None:
            return
        remaining = delay - (time.monotonic() - self.last_request_completed_at)
        if remaining > 0:
            time.sleep(remaining)


def canonical_book_title(title: str, author: str | None, stores: dict[str, AbstractBookstore], delay: float, query: str, cache: BookstoreSearchCache) -> tuple[str | None, str | None, str, str]:
    results: list[tuple[str, str, str]] = []
    rejected_candidates: list[str] = []
    store_items = list(stores.items())
    cached_by_store = {store_name: cache.get(query, store_name) for store_name, _store in store_items}
    store_status = ["+" if cached_by_store[store_name] is not None else "." for store_name, _store in store_items]

    for store_index, (store_name, store) in enumerate(store_items):
        cached_results = cached_by_store[store_name]
        if cached_results is None:
            store_status[store_index] = "-"
            cache.searches += 1
            cache.wait_for_request_slot(delay)
            search_started_at = time.monotonic()
            try:
                books, _, _ = store.search(title=title, author=author or "")
            except Exception as exc:
                print(f"{store_name} 검색 실패: {exc}", file=sys.stderr)
                books = []
            finally:
                cache.search_seconds += time.monotonic() - search_started_at
                cache.last_request_completed_at = time.monotonic()
            cached_results = []
            for found_title, found_author, category, url, _search_url, isbn in books:
                cached_results.append({"title": str(found_title).strip(), "author": str(found_author).strip(), "category": str(category), "url": str(url), "isbn": str(isbn)})
            cache.put(query, store_name, cached_results)

        matching_results = [row for row in cached_results if bookstore_title_matches(row["title"], title)]
        if matching_results:
            row = matching_results[0]
            results.append((store_name, bookstore_title_base(row["title"]), row["author"].strip()))
        elif cached_results:
            candidates = ", ".join(row["title"] for row in cached_results[:3])
            rejected_candidates.append(f"{store_name} candidates: {candidates}")

        title_votes = Counter(unicodedata.normalize("NFKC", row[1]).strip() for row in results)
        if title_votes and title_votes.most_common(1)[0][1] >= 1 and any(korean_author(row[2]) for row in results):
            break

    title_votes = Counter(unicodedata.normalize("NFKC", row[1]).strip() for row in results)
    if not title_votes or title_votes.most_common(1)[0][1] < 1:
        evidence = "; ".join(rejected_candidates)
        return None, None, evidence or "no bookstore result", "".join(store_status)

    canonical_title, _ = title_votes.most_common(1)[0]
    matching_rows = [row for row in results if unicodedata.normalize("NFKC", row[1]).strip() == canonical_title]
    author_votes = Counter(author for row in matching_rows if (author := korean_author(row[2])))
    canonical_author = None
    if author_votes:
        author_name, vote_count = author_votes.most_common(1)[0]
        if vote_count >= 1:
            canonical_author = author_name
    evidence = "; ".join(f"{store}: {found_title}" for store, found_title, _ in matching_rows)
    return canonical_title, canonical_author, evidence, "".join(store_status)


def build_proposals(root: Path, delay: float, max_searches: int, cache: BookstoreSearchCache) -> list[Proposal]:
    started_at = time.monotonic()
    proposals = []
    stores = {name: cls(verbose=False) for name, cls in BOOKSTORES}
    author_references = local_author_references(root)
    query_cache: dict[tuple[str, str], tuple[str | None, str | None, str, str]] = {}
    queries_processed = 0
    print("Store status [yes24/aladin/kyobo]: +=cache, -=request, .=skipped", file=sys.stderr)
    for category in TARGET_CATEGORIES:
        parent = root / category
        if not parent.is_dir():
            continue
        for directory in sorted(path for path in parent.iterdir() if path.is_dir() and not path.is_symlink()):
            identity = infer_identity(directory)
            title, author = identity.title, identity.author
            confidence, count = identity.confidence, identity.file_count
            if title is None:
                proposals.append(Proposal(directory, None, confidence, count, identity.failure_status or "SKIP_UNPARSEABLE_FILENAMES", identity.failure_reason))
                continue
            original_author = directory_author(directory.name)
            metadata = normalized_directory_metadata(directory)
            if metadata is None:
                proposals.append(Proposal(directory, None, confidence, count, "SKIP_NO_RANGE"))
                continue
            query_author = original_author or author
            query_key = (title_key(title), title_key(query_author or ""))
            cache_key = "|".join(query_key)
            if query_key not in query_cache:
                has_any_cached_store = any((cache_key, store_name) in cache.entries for store_name, _ in BOOKSTORES)
                if max_searches and queries_processed >= max_searches and not has_any_cached_store:
                    proposals.append(Proposal(directory, None, confidence, count, "SKIP_SEARCH_LIMIT"))
                    continue
                query_cache[query_key] = canonical_book_title(title, query_author, stores, delay, cache_key, cache)
                queries_processed += 1
                print(query_cache[query_key][3], end=" ", file=sys.stderr, flush=True)
                if queries_processed % 25 == 0:
                    print(f"\nTitle queries: {queries_processed}; store requests: {cache.searches}; store cache hits: {cache.hits}; bookstore time: {cache.search_seconds:.1f}s", file=sys.stderr)
            book_title, book_author, evidence, _store_status = query_cache[query_key]
            if book_title is None:
                proposals.append(Proposal(directory, None, confidence, count, "SKIP_NO_BOOKSTORE_MATCH", evidence))
                continue
            resolved_author = korean_author(original_author) or korean_author(author) or matching_local_author(book_title, author_references) or korean_author(book_author)
            if resolved_author is None:
                proposals.append(Proposal(directory, None, confidence, count, "SKIP_NO_AUTHOR", evidence))
                continue
            base_name = f"[{resolved_author}] {book_title} {metadata}"
            target_name = filesystem_safe_name(base_name)
            if target_name is None:
                proposals.append(Proposal(directory, None, confidence, count, "SKIP_INVALID_NAME", evidence))
                continue
            target = directory.with_name(target_name)
            status = "UNCHANGED" if target == directory else "RENAME"
            proposals.append(Proposal(directory, target, confidence, count, status, evidence))

    print(f"\nProposal scan: {time.monotonic() - started_at:.1f}s; directories: {len(proposals)}; title queries: {queries_processed}; store requests: {cache.searches}; store cache hits: {cache.hits}; bookstore time: {cache.search_seconds:.1f}s", file=sys.stderr)

    planned_sources = {p.source for p in proposals if p.status == "RENAME"}
    target_counts = Counter(p.target for p in proposals if p.status == "RENAME")
    for proposal in proposals:
        if proposal.status == "RENAME" and proposal.target:
            occupied = proposal.target.exists() and proposal.target not in planned_sources
            duplicate = target_counts[proposal.target] > 1
            if occupied or duplicate:
                proposal.status = "SKIP_CONFLICT"
    return proposals


def write_report(proposals: list[Proposal], path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8-sig", newline="") as stream:
        writer = csv.writer(stream, delimiter="\t")
        writer.writerow(("status", "confidence", "book_files", "source", "target", "evidence"))
        for item in proposals:
            writer.writerow((item.status, f"{item.confidence:.3f}", item.sample_count, item.source, item.target or "", item.evidence))


def apply_renames(proposals: list[Proposal]) -> int:
    renames = [item for item in proposals if item.status == "RENAME" and item.target]
    if any(item.target.exists() and item.target not in {row.source for row in renames} for item in renames):
        raise RuntimeError("A destination appeared after preflight; no rename was started.")

    staged: list[tuple[Path, Path, Path]] = []
    try:
        for item in renames:
            temporary = item.source.with_name(f".normalize-{uuid4().hex}")
            item.source.rename(temporary)
            staged.append((item.source, temporary, item.target))
        for _source, temporary, target in staged:
            if target.exists():
                raise FileExistsError(target)
            temporary.rename(target)
    except Exception:
        for source, temporary, target in reversed(staged):
            current = target if target.exists() else temporary
            if current.exists() and not source.exists():
                current.rename(source)
        raise
    return len(renames)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path("/mnt/data/만화"))
    parser.add_argument("--report", type=Path, default=Path("/tmp/comic-directory-normalization.tsv"))
    parser.add_argument("--delay", type=float, default=1.2, help="minimum idle seconds between live bookstore requests")
    parser.add_argument("--max-searches", type=int, default=0, help="limit distinct title/author searches; 0 means unlimited")
    parser.add_argument("--cache", type=Path, default=Path("/tmp/comic-bookstore-title-cache.jsonl"))
    parser.add_argument("--negative-cache-seconds", type=int, default=DEFAULT_NEGATIVE_CACHE_SECONDS, help="cache empty per-store results for this long; default: 7 days")
    parser.add_argument("--refresh-cache", action="store_true", help="clear the per-store result cache before searching")
    parser.add_argument("--apply", action="store_true", help="perform the planned directory renames")
    args = parser.parse_args()
    root = args.root.resolve()
    if not root.is_dir():
        parser.error(f"not a directory: {root}")

    cache = BookstoreSearchCache(args.cache, refresh=args.refresh_cache, negative_ttl_seconds=args.negative_cache_seconds)
    proposals = build_proposals(root, max(0.0, args.delay), max(0, args.max_searches), cache)
    write_report(proposals, args.report)
    counts = Counter(item.status for item in proposals)
    print(f"Directories scanned: {len(proposals)}")
    skipped = sum(count for status, count in counts.items() if status.startswith("SKIP_"))
    print(f"Rename proposals: {counts['RENAME']}; unchanged: {counts['UNCHANGED']}; skipped: {skipped}")
    print(f"Report: {args.report}")
    print(f"Bookstore cache: {args.cache}; store requests: {cache.searches}; store cache hits: {cache.hits}; bookstore time: {cache.search_seconds:.1f}s")
    if args.apply:
        changed = apply_renames(proposals)
        print(f"Renamed: {changed}")
        print("Rebuild or refresh the comics Elasticsearch index after renaming.")
    else:
        print("Dry run only. Pass --apply to rename directories.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
