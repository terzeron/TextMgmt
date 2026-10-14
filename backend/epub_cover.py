"""EPUB 표지 검사와 inode를 보존하는 표지 저장."""

import fcntl
import hashlib
import io
import os
import posixpath
import re
import shutil
import tempfile
import warnings
import zipfile
import uuid
from pathlib import Path
from typing import Any, BinaryIO

from lxml import etree
from PIL import Image

from backend.book_manager import BookManager, _safe_xml_parser

OPF_NS = "http://www.idpf.org/2007/opf"
MAX_XML_BYTES = 4 * 1024 * 1024
MAX_EPUB_BYTES = 1024 * 1024 * 1024
MAX_IMAGE_BYTES = 15 * 1024 * 1024
MAX_IMAGE_PIXELS = 80_000_000


class CoverError(ValueError):
    """사용자에게 표시할 수 있는 표지 처리 오류."""


def revision(path: Path) -> str:
    stat = path.stat()
    return f"{stat.st_ino}:{stat.st_size}:{stat.st_mtime_ns}:{stat.st_ctime_ns}"


def _read_package(archive: zipfile.ZipFile) -> tuple[str, Any]:
    if len(archive.infolist()) > 20000 or sum(item.file_size for item in archive.infolist()) > MAX_EPUB_BYTES:
        raise CoverError("EPUB이 검사 크기 제한을 초과했습니다")
    if archive.getinfo("META-INF/container.xml").file_size > MAX_XML_BYTES:
        raise CoverError("EPUB container가 너무 큽니다")
    opf_path = BookManager._find_opf_path(archive)
    if not opf_path or opf_path.startswith("/") or posixpath.normpath(opf_path).startswith("../") or archive.getinfo(opf_path).file_size > MAX_XML_BYTES:
        raise CoverError("EPUB package를 찾거나 읽을 수 없습니다")
    data = archive.read(opf_path)
    try:
        package = etree.fromstring(data, _safe_xml_parser())
    except etree.XMLSyntaxError:
        # 일부 EPUB은 opf:role 등에서 opf namespace 선언을 빠뜨린다.
        # 이 오류만 보완하며 XML 구조 오류는 복구해서 저장하지 않는다.
        parser = _safe_xml_parser(recover=True)
        package = etree.fromstring(data, parser)
        if package is None or any(error.type_name != "NS_ERR_UNDEFINED_NAMESPACE" for error in parser.error_log):
            raise
        for element in package.iter():
            for attribute in list(element.attrib):
                if attribute.startswith("opf:"):
                    value = element.attrib.pop(attribute)
                    element.set(f"{{{OPF_NS}}}{attribute[4:]}", value)
        # 알 수 없는 prefix는 여기서 실패한다. 저장할 때도 유효한 namespace를 쓴다.
        package = etree.fromstring(etree.tostring(package), _safe_xml_parser())
    if etree.QName(package).localname != "package":
        raise CoverError("올바른 EPUB package가 아닙니다")
    return opf_path, package


def normalize_image(data: bytes, *, max_width: int = 2000) -> bytes:
    if not data or len(data) > MAX_IMAGE_BYTES:
        raise CoverError("표지 이미지가 비어 있거나 너무 큽니다")
    try:
        with warnings.catch_warnings():
            warnings.simplefilter("ignore", Image.DecompressionBombWarning)
            with Image.open(io.BytesIO(data)) as image:
                # 큰 JPEG는 원본 전체를 메모리에 펼치기 전에 decoder에서 축소한다.
                target_height = min(3000, max(1, round(image.height * min(max_width, image.width) / image.width)))
                image.draft("RGB", (max_width, target_height))
                if image.width * image.height > MAX_IMAGE_PIXELS:
                    raise CoverError("표지 이미지 해상도가 너무 큽니다")
                image.load()
                image.thumbnail((max_width, 3000), Image.Resampling.LANCZOS)
                converted = image.convert("RGB")
                output = io.BytesIO()
                converted.save(output, "JPEG", quality=92)
                return output.getvalue()
    except CoverError:
        raise
    except Exception as exc:
        raise CoverError("표지 이미지 형식을 읽을 수 없습니다") from exc


def _inspect_archive(archive: zipfile.ZipFile) -> dict[str, Any]:
    _, package = _read_package(archive)
    member = BookManager._find_epub_cover_member(archive)
    has_cover = bool(member and member in archive.namelist())
    if has_cover and member is not None:
        if archive.getinfo(member).file_size > MAX_IMAGE_BYTES:
            raise CoverError("기존 표지가 검사 크기 제한을 초과했습니다")
        # SVG 표지를 읽지 못한다고 표지 없는 책으로 오인하지 않는다.
        if member.lower().endswith(".svg"):
            etree.fromstring(archive.read(member), _safe_xml_parser())
        else:
            normalize_image(archive.read(member))
    metadata = {}
    for key, localname in (("title", "title"), ("author", "creator"), ("isbn", "identifier")):
        values = package.xpath(f'//*[local-name()="{localname}"]/text()')
        metadata[key] = str(values[0]).strip() if values else ""
    if metadata["isbn"]:
        value = metadata["isbn"].removeprefix("urn:isbn:").replace("-", "")
        metadata["isbn"] = value if value.isdigit() and len(value) in (10, 13) else ""
    return {**metadata, "has_cover": has_cover}


def inspect_epub(path: Path) -> dict[str, Any]:
    try:
        with path.open("rb") as source:
            fcntl.flock(source, fcntl.LOCK_SH | fcntl.LOCK_NB)
            with zipfile.ZipFile(source) as archive:
                metadata = _inspect_archive(archive)
            return {**metadata, "title": metadata["title"] or path.stem, "revision": revision(path)}
    except CoverError:
        raise
    except BlockingIOError as exc:
        raise CoverError("이 EPUB은 현재 저장 중입니다") from exc
    except (OSError, KeyError, zipfile.BadZipFile, etree.XMLSyntaxError, ValueError) as exc:
        raise CoverError("EPUB 또는 기존 표지를 검사할 수 없습니다") from exc


def _stage_cover(source: BinaryIO, destination: Path, image: bytes) -> None:
    source.seek(0)
    with zipfile.ZipFile(source) as original:
        opf_path, package = _read_package(original)
        manifest = package.find(f"{{{OPF_NS}}}manifest")
        metadata = package.find(f"{{{OPF_NS}}}metadata")
        spine = package.find(f"{{{OPF_NS}}}spine")
        if manifest is None or metadata is None or spine is None:
            raise CoverError("EPUB manifest, metadata 또는 spine이 없습니다")
        obsolete = set()
        old_ids = {meta.get("content", "") for meta in metadata if meta.get("name") == "cover"}
        old_ids.update(item.get("id", "") for item in manifest if "cover-image" in item.get("properties", "").split())
        for old_id in old_ids:
            if not re.fullmatch(r"tm-cover-[a-f0-9]{16}", old_id):
                continue
            for identifier, suffix in ((old_id, ".jpg"), (old_id.replace("tm-cover-", "tm-cover-page-"), ".xhtml")):
                for item in list(manifest):
                    if item.get("id") == identifier and item.get("href") == identifier + suffix:
                        obsolete.add(posixpath.join(posixpath.dirname(opf_path), item.get("href")))
                        manifest.remove(item)
                        for reference in list(spine):
                            if reference.get("idref") == identifier:
                                spine.remove(reference)
        digest = hashlib.sha256(image).hexdigest()[:16]
        cover_id = f"tm-cover-{digest}"
        page_id = f"tm-cover-page-{digest}"
        cover_href = f"{cover_id}.jpg"
        page_href = f"{page_id}.xhtml"
        existing_ids = {item.get("id") for item in manifest}
        existing_names = set(original.namelist()) - obsolete
        if cover_id in existing_ids or page_id in existing_ids:
            raise CoverError("추가할 표지 ID가 이미 존재합니다")
        cover_member = posixpath.join(posixpath.dirname(opf_path), cover_href)
        page_member = posixpath.join(posixpath.dirname(opf_path), page_href)
        if cover_member in existing_names or page_member in existing_names:
            raise CoverError("추가할 표지 경로가 이미 존재합니다")
        for item in manifest:
            properties = item.get("properties", "").split()
            if "cover-image" in properties:
                properties.remove("cover-image")
                if properties:
                    item.set("properties", " ".join(properties))
                else:
                    item.attrib.pop("properties", None)
        for meta in metadata.findall(f"{{{OPF_NS}}}meta"):
            if meta.get("name") == "cover":
                metadata.remove(meta)
        etree.SubElement(metadata, f"{{{OPF_NS}}}meta", name="cover", content=cover_id)
        attributes = {"id": cover_id, "href": cover_href, "media-type": "image/jpeg"}
        if package.get("version", "").startswith("3"):
            attributes["properties"] = "cover-image"
        etree.SubElement(manifest, f"{{{OPF_NS}}}item", **attributes)
        etree.SubElement(manifest, f"{{{OPF_NS}}}item", id=page_id, href=page_href, **{"media-type": "application/xhtml+xml"})
        spine.insert(0, etree.Element(f"{{{OPF_NS}}}itemref", idref=page_id))
        page = (f'<?xml version="1.0" encoding="utf-8"?><html xmlns="http://www.w3.org/1999/xhtml">'
                f'<head><title>표지</title></head><body style="margin:0;text-align:center">'
                f'<img src="{cover_href}" alt="표지" style="max-width:100%;max-height:100vh"/></body></html>')
        with zipfile.ZipFile(destination, "w", compression=zipfile.ZIP_DEFLATED) as updated:
            updated.writestr("mimetype", "application/epub+zip", compress_type=zipfile.ZIP_STORED)
            updated.comment = original.comment
            for item in original.infolist():
                if item.filename == "mimetype" or item.filename in obsolete:
                    continue
                if item.filename == opf_path:
                    updated.writestr(item, etree.tostring(package, encoding="utf-8", xml_declaration=True))
                else:
                    with original.open(item) as reader, updated.open(item, "w") as writer:
                        while chunk := reader.read(1024 * 1024):
                            writer.write(chunk)
            updated.writestr(cover_member, image)
            updated.writestr(page_member, page)
    with destination.open("rb") as output:
        os.fsync(output.fileno())
    with zipfile.ZipFile(destination) as archive:
        if archive.testzip() is not None or not _inspect_archive(archive)["has_cover"]:
            raise CoverError("저장할 EPUB의 표지 검증에 실패했습니다")


def add_cover(path: Path, image: bytes, expected_revision: str, backup_dir: Path, *, replace_existing: bool = False) -> dict[str, Any]:
    image = normalize_image(image, max_width=600)
    with path.open("r+b") as source:
        try:
            fcntl.flock(source, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as exc:
            raise CoverError("이 EPUB은 현재 저장 중입니다") from exc
        if revision(path) != expected_revision:
            raise CoverError("EPUB이 변경되었습니다. 목록을 새로 불러오세요")
        with zipfile.ZipFile(source) as archive:
            if _inspect_archive(archive)["has_cover"] and not replace_existing:
                raise CoverError("EPUB에 이미 표지가 있습니다")
        original_stat = path.stat()
        backup_dir.mkdir(parents=True, exist_ok=True)
        backup_name = hashlib.sha256(f"{path}:{expected_revision}".encode()).hexdigest() + f"-{uuid.uuid4().hex}.epub"
        backup = backup_dir / backup_name
        source.seek(0)
        with backup.open("xb") as output:
            shutil.copyfileobj(source, output)
            output.flush()
            os.fsync(output.fileno())
        descriptor, filename = tempfile.mkstemp(prefix=".cover-", suffix=".tmp", dir=path.parent)
        os.close(descriptor)
        staged = Path(filename)
        try:
            _stage_cover(source, staged, image)
            if revision(path) != expected_revision:
                raise CoverError("EPUB이 변경되었습니다. 목록을 새로 불러오세요")
            try:
                with staged.open("rb") as reader:
                    source.seek(0)
                    shutil.copyfileobj(reader, source)
                    source.truncate()
                    source.flush()
                    os.fsync(source.fileno())
            except Exception:
                with backup.open("rb") as reader:
                    source.seek(0)
                    shutil.copyfileobj(reader, source)
                    source.truncate()
                    source.flush()
                    os.fsync(source.fileno())
                raise
            finally:
                os.utime(path, ns=(original_stat.st_atime_ns, original_stat.st_mtime_ns))
        finally:
            staged.unlink(missing_ok=True)
    return {"revision": revision(path), "backup_path": str(backup), "file_size": path.stat().st_size, "book_id": original_stat.st_ino}
