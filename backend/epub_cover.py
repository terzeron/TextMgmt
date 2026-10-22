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
from typing import Any, BinaryIO, Callable
from urllib.parse import unquote, urlparse

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
    image_info = None
    cover_error = ""
    if has_cover and member is not None:
        try:
            byte_size = archive.getinfo(member).file_size
            if byte_size > MAX_IMAGE_BYTES:
                raise CoverError("기존 표지가 검사 크기 제한을 초과했습니다")
            image_info = {"byte_size": byte_size}
            if member.lower().endswith(".svg"):
                etree.fromstring(archive.read(member), _safe_xml_parser())
            else:
                # 목록에서는 header만 읽는다. 픽셀 디코딩과 변환은 미리보기·저장에서 검증한다.
                with warnings.catch_warnings():
                    warnings.simplefilter("ignore", Image.DecompressionBombWarning)
                    with archive.open(member) as source, Image.open(source) as image:
                        if image.width * image.height > MAX_IMAGE_PIXELS:
                            raise CoverError("기존 표지 이미지 해상도가 너무 큽니다")
                        image_info.update(width=image.width, height=image.height)
        except (OSError, ValueError, zipfile.BadZipFile, etree.XMLSyntaxError, Image.DecompressionBombError) as exc:
            # 기존 이미지의 손상이 검색과 교체를 막지 않도록 EPUB 구조 오류와 구분한다.
            cover_error = str(exc) if isinstance(exc, CoverError) else "기존 표지 이미지를 읽을 수 없습니다"
            image_info = None
    metadata = {}
    for key, localname in (("title", "title"), ("author", "creator"), ("isbn", "identifier")):
        values = package.xpath(f'//*[local-name()="{localname}"]/text()')
        metadata[key] = str(values[0]).strip() if values else ""
    if metadata["isbn"]:
        value = metadata["isbn"].removeprefix("urn:isbn:").replace("-", "")
        metadata["isbn"] = value if value.isdigit() and len(value) in (10, 13) else ""
    return {**metadata, "has_cover": has_cover, "cover_image": image_info,
            **({"cover_error": cover_error} if cover_error else {})}


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


def _remove_tool_covers(opf_path: str, manifest: Any, metadata: Any, spine: Any) -> set[str]:
    """이 도구가 넣은 표지(tm-cover-*)의 manifest 항목과 spine 참조를 지우고, 파일에서 뺄 멤버 경로를 돌려준다.

    출판사 원본 표지는 건드리지 않는다. 그 구조는 책마다 달라 임의로 지우면 본문이 깨질 수 있다.
    """
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
    return obsolete


def _member_path(base: str, href: str) -> str:
    parsed = urlparse(href)
    if parsed.scheme or parsed.netloc or not parsed.path:
        return ""
    member = posixpath.normpath(posixpath.join(base, unquote(parsed.path)))
    return "" if member.startswith(("/", "../")) else member


def _detach_publisher_cover(archive: zipfile.ZipFile, opf_path: str, package: Any, *, removing: bool = False) -> tuple[set[str], dict[str, bytes]]:
    """표지 전용 항목을 읽기 순서와 탐색에서 뺀다. 본문과 공유하는 파일은 보존한다.

    본문 페이지의 목차 링크(<a href>)가 빠지는 표지 페이지를 가리키면 링크만 걷어 낸 텍스트로 바꾼다.
    새 표지 페이지로 다시 잇지 않는다. 도구가 넣은 표지는 삭제·교체로 사라지므로 다시 끊어진다.
    링크 말고 다른 방식의 참조는 지울 수 없어 거부한다.
    """
    member = BookManager._find_epub_cover_member(archive)
    if not member:
        return set(), {}
    manifest, metadata, spine = (package.find(f"{{{OPF_NS}}}{name}") for name in ("manifest", "metadata", "spine"))
    base = posixpath.dirname(opf_path)
    images = [item for item in manifest if _member_path(base, item.get("href", "")) == member]
    if not images:
        return set(), {}  # 도구 표지는 _remove_tool_covers가 이미 처리했다.

    documents = {}
    for item in manifest:
        if item.get("media-type") not in ("application/xhtml+xml", "image/svg+xml", "application/x-dtbncx+xml", "text/css"):
            continue
        name = _member_path(base, item.get("href", ""))
        if name not in archive.NameToInfo:
            continue
        if archive.getinfo(name).file_size > MAX_XML_BYTES:
            raise CoverError("표지와 본문 참조를 검사할 파일이 너무 큽니다")
        data = archive.read(name)
        if item.get("media-type") == "text/css":
            documents[name] = (item, None, data)
        else:
            documents[name] = (item, etree.fromstring(data, _safe_xml_parser()), data)

    def references(name: str, document: Any, data: bytes) -> set[str]:
        hrefs = [value for element in document.iter() for value in element.attrib.values()] if document is not None else []
        hrefs.extend(re.findall(r'url\(\s*[\'"]?([^\s\)\'\"]+)', data.decode("utf-8", errors="replace")))
        return {_member_path(posixpath.dirname(name), href) for href in hrefs}

    guide = package.find(f"{{{OPF_NS}}}guide")
    cover_pages = {_member_path(base, ref.get("href", "")) for ref in guide if "cover" in ref.get("type", "").split()} if guide is not None else set()
    cover_ids = {meta.get("content") for meta in metadata if meta.get("name") == "cover"}
    first_id = spine[0].get("idref") if len(spine) else None
    detached: set[str] = set()
    for name, (item, document, _) in documents.items():
        if item.get("media-type") != "application/xhtml+xml" or document is None:
            continue
        if name not in cover_pages and item.get("id") not in cover_ids | {first_id} and "cover" not in f"{item.get('id', '')} {item.get('href', '')}".lower():
            continue
        bodies = document.xpath('//*[local-name()="body"]')
        body = bodies[0] if bodies else document
        image_refs = [element.get("src") or element.get("href") or element.get("{http://www.w3.org/1999/xlink}href", "")
                      for element in body.iter() if isinstance(element.tag, str) and etree.QName(element).localname in ("img", "image")]
        if image_refs and all(_member_path(posixpath.dirname(name), href) == member for href in image_refs) and not "".join(body.itertext()).strip():
            detached.add(name)

    shared = any(member in references(name, document, data) for name, (_, document, data) in documents.items() if name not in detached and name != member)
    if shared and removing:
        raise CoverError("표지 이미지가 본문에서도 쓰여 자동으로 삭제할 수 없습니다")
    # 일반 본문의 목차 링크는 다시 잇거나 걷어 내고, 링크가 아닌 참조는 깨지므로 저장 전에 거부한다.
    relinked: dict[str, bytes] = {}
    for name, (item, document, data) in documents.items():
        if name in detached or item.get("media-type") == "application/x-dtbncx+xml" or "nav" in item.get("properties", "").split():
            continue
        if not detached & references(name, document, data):
            continue
        if document is None:
            raise CoverError("표지 페이지가 본문에서도 쓰여 자동으로 삭제할 수 없습니다")
        links = []
        for element in document.iter():
            if not isinstance(element.tag, str):
                continue
            for attribute, value in element.attrib.items():
                if _member_path(posixpath.dirname(name), value) in detached:
                    if etree.QName(element).localname != "a" or attribute != "href":
                        raise CoverError("표지 페이지가 본문에서도 쓰여 자동으로 삭제할 수 없습니다")
                    links.append(element)
        for link in links:
            namespace = etree.QName(link).namespace
            link.tag = f"{{{namespace}}}span" if namespace else "span"
            link.attrib.pop("href", None)
        if links:
            relinked[name] = etree.tostring(document, encoding="utf-8", xml_declaration=True)
    removed_ids = set()
    for item in list(manifest):
        if _member_path(base, item.get("href", "")) in detached or (item in images and not shared):
            removed_ids.add(item.get("id"))
            manifest.remove(item)
    for reference in list(spine):
        if reference.get("idref") in removed_ids:
            spine.remove(reference)
    for meta in list(metadata):
        if meta.get("name") == "cover":
            metadata.remove(meta)
    if guide is not None:
        for reference in list(guide):
            if "cover" in reference.get("type", "").split():
                guide.remove(reference)

    updates: dict[str, bytes] = dict(relinked)
    for name, (item, document, _) in documents.items():
        is_ncx = item.get("media-type") == "application/x-dtbncx+xml"
        is_nav = "nav" in item.get("properties", "").split()
        if document is None or (not is_ncx and not is_nav):
            continue
        changed = False
        for content in document.xpath('//*[local-name()="content"]' if is_ncx else '//*[local-name()="a"]'):
            if _member_path(posixpath.dirname(name), content.get("src" if is_ncx else "href", "")) not in detached:
                continue
            point = content.getparent()
            # 하위 목차가 있으면 그 항목을 부모로 올려 본문 탐색 경로를 보존한다.
            if is_ncx and etree.QName(point).localname == "navPoint":
                parent = point.getparent()
                for child in point.xpath('./*[local-name()="navPoint"]'):
                    parent.insert(parent.index(point), child)
                parent.remove(point)
            elif is_nav:
                if etree.QName(point).localname == "li" and not point.xpath('./*[local-name()="ol"]'):
                    point.getparent().remove(point)
                else:
                    content.tag = "{http://www.w3.org/1999/xhtml}span"
                    content.attrib.pop("href", None)
            else:
                raise CoverError("표지의 목차 참조를 안전하게 정리할 수 없습니다")
            changed = True
        if changed:
            orders = document.xpath('//*[@playOrder]')
            numbering = {old: str(index) for index, old in enumerate(sorted({int(node.get("playOrder")) for node in orders}), 1)}
            for node in orders:
                node.set("playOrder", numbering[int(node.get("playOrder"))])
            updates[name] = etree.tostring(document, encoding="utf-8", xml_declaration=True)
    return detached | ({member} if not shared else set()), updates


def _write_epub(original: zipfile.ZipFile, destination: Path, opf_path: str, package: Any, obsolete: set[str], added: dict[str, bytes]) -> None:
    """OPF와 added의 멤버를 갱신하고, obsolete를 제외한 나머지 멤버는 그대로 복사한다."""
    with zipfile.ZipFile(destination, "w", compression=zipfile.ZIP_DEFLATED) as updated:
        updated.writestr("mimetype", "application/epub+zip", compress_type=zipfile.ZIP_STORED)
        updated.comment = original.comment
        for item in original.infolist():
            if item.filename == "mimetype" or item.filename in obsolete:
                continue
            if item.filename == opf_path:
                updated.writestr(item, etree.tostring(package, encoding="utf-8", xml_declaration=True))
            elif item.filename in added:
                updated.writestr(item, added[item.filename])
            else:
                with original.open(item) as reader, updated.open(item, "w") as writer:
                    while chunk := reader.read(1024 * 1024):
                        writer.write(chunk)
        for member, data in added.items():
            if member not in original.NameToInfo or member in obsolete:
                updated.writestr(member, data)


def _read_previous_cover(archive: zipfile.ZipFile, opf_path: str, manifest: Any) -> tuple[str, bytes, str, str] | None:
    """교체 전 표지 이미지를 (멤버 경로, 바이트, media-type, 확장자)로 읽는다. 도판으로 남길 수 없으면 None.

    manifest 항목을 지우기 전에 불러야 한다. 이미지가 아니거나 너무 크면 남기지 않는다.
    """
    member = BookManager._find_epub_cover_member(archive)
    if not member or member not in archive.NameToInfo or archive.getinfo(member).file_size > MAX_IMAGE_BYTES:
        return None
    base = posixpath.dirname(opf_path)
    item = next((entry for entry in manifest if _member_path(base, entry.get("href", "")) == member), None)
    media_type = item.get("media-type", "") if item is not None else ""
    if not media_type.startswith("image/"):
        return None
    suffix = posixpath.splitext(member)[1].lower()
    return member, archive.read(member), media_type, suffix if re.fullmatch(r"\.[a-z0-9]{1,5}", suffix) else ".jpg"


def _illustration_page(image_href: str) -> bytes:
    return (f'<?xml version="1.0" encoding="utf-8"?><html xmlns="http://www.w3.org/1999/xhtml">'
            f'<head><title>도판</title></head><body style="margin:0;text-align:center">'
            f'<div><img src="{image_href}" alt="도판" style="max-width:100%;max-height:100vh"/></div></body></html>').encode()


def _stage_cover(source: BinaryIO, destination: Path, image: bytes) -> None:
    source.seek(0)
    with zipfile.ZipFile(source) as original:
        opf_path, package = _read_package(original)
        manifest = package.find(f"{{{OPF_NS}}}manifest")
        metadata = package.find(f"{{{OPF_NS}}}metadata")
        spine = package.find(f"{{{OPF_NS}}}spine")
        if manifest is None or metadata is None or spine is None:
            raise CoverError("EPUB manifest, metadata 또는 spine이 없습니다")
        previous = _read_previous_cover(original, opf_path, manifest)
        obsolete = _remove_tool_covers(opf_path, manifest, metadata, spine)
        detached, updates = _detach_publisher_cover(original, opf_path, package)
        obsolete.update(detached)
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
                f'<div><img src="{cover_href}" alt="표지" style="max-width:100%;max-height:100vh"/></div></body></html>')
        added = {**updates, cover_member: image, page_member: page.encode()}
        # 교체로 빠지는 기존 표지는 새 표지 바로 뒤의 도판으로 남긴다. 본문에서도 쓰여 파일이 그대로 남는 표지는 이미 본문에 있다.
        # 이름에 cover를 넣지 않는다. 표지 탐색이 id·href의 cover로 이미지를 고르므로 표지로 오인된다.
        if previous and previous[0] in obsolete:
            _, previous_data, media_type, suffix = previous
            plate_digest = hashlib.sha256(previous_data).hexdigest()[:16]
            plate_id, plate_page_id = f"tm-plate-{plate_digest}", f"tm-plate-page-{plate_digest}"
            plate_href, plate_page_href = plate_id + suffix, plate_page_id + ".xhtml"
            plate_member = posixpath.join(posixpath.dirname(opf_path), plate_href)
            plate_page_member = posixpath.join(posixpath.dirname(opf_path), plate_page_href)
            taken = existing_ids | existing_names | {cover_id, page_id, cover_member, page_member}
            if plate_digest != digest and not {plate_id, plate_page_id, plate_member, plate_page_member} & taken:
                etree.SubElement(manifest, f"{{{OPF_NS}}}item", id=plate_id, href=plate_href, **{"media-type": media_type})
                etree.SubElement(manifest, f"{{{OPF_NS}}}item", id=plate_page_id, href=plate_page_href, **{"media-type": "application/xhtml+xml"})
                spine.insert(1, etree.Element(f"{{{OPF_NS}}}itemref", idref=plate_page_id))
                added.update({plate_member: previous_data, plate_page_member: _illustration_page(plate_href)})
        _write_epub(original, destination, opf_path, package, obsolete, added)
    with destination.open("rb") as output:
        os.fsync(output.fileno())
    with zipfile.ZipFile(destination) as archive:
        if archive.testzip() is not None or not _inspect_archive(archive)["has_cover"]:
            raise CoverError("저장할 EPUB의 표지 검증에 실패했습니다")


def _stage_illustration(source: BinaryIO, destination: Path, image: bytes, before_idref: str, created: dict[str, str]) -> None:
    """before_idref 항목 바로 앞에 이미지 한 장짜리 도판 페이지를 끼운다. 새 페이지의 idref를 created에 적는다."""
    source.seek(0)
    with zipfile.ZipFile(source) as original:
        opf_path, package = _read_package(original)
        manifest = package.find(f"{{{OPF_NS}}}manifest")
        spine = package.find(f"{{{OPF_NS}}}spine")
        if manifest is None or spine is None:
            raise CoverError("EPUB manifest 또는 spine이 없습니다")
        target = next((reference for reference in spine if reference.get("idref") == before_idref), None)
        if target is None:
            raise CoverError("삽화를 넣을 위치를 찾을 수 없습니다. 책을 다시 열어 보세요")
        if before_idref.startswith("tm-cover-page-"):
            raise CoverError("표지 앞에는 삽화를 넣을 수 없습니다")
        digest = hashlib.sha256(image).hexdigest()[:16]
        existing_ids = {item.get("id") for item in manifest}
        existing_names = set(original.namelist())
        base = posixpath.dirname(opf_path)
        # 같은 이미지를 여러 곳에 넣을 수 있도록 겹치면 번호를 붙인다.
        for serial in range(1, 1000):
            token = digest if serial == 1 else f"{digest}-{serial}"
            image_id, page_id = f"tm-plate-{token}", f"tm-plate-page-{token}"
            image_href, page_href = image_id + ".jpg", page_id + ".xhtml"
            image_member, page_member = posixpath.join(base, image_href), posixpath.join(base, page_href)
            if not {image_id, page_id} & existing_ids and not {image_member, page_member} & existing_names:
                break
        else:
            raise CoverError("추가할 삽화 ID를 만들 수 없습니다")
        etree.SubElement(manifest, f"{{{OPF_NS}}}item", id=image_id, href=image_href, **{"media-type": "image/jpeg"})
        etree.SubElement(manifest, f"{{{OPF_NS}}}item", id=page_id, href=page_href, **{"media-type": "application/xhtml+xml"})
        spine.insert(spine.index(target), etree.Element(f"{{{OPF_NS}}}itemref", idref=page_id))
        _write_epub(original, destination, opf_path, package, set(), {image_member: image, page_member: _illustration_page(image_href)})
        created["idref"] = page_id
    with destination.open("rb") as output:
        os.fsync(output.fileno())
    with zipfile.ZipFile(destination) as archive:
        if archive.testzip() is not None:
            raise CoverError("저장할 EPUB의 검증에 실패했습니다")
        _read_package(archive)


def _stage_cover_removal(source: BinaryIO, destination: Path) -> None:
    source.seek(0)
    with zipfile.ZipFile(source) as original:
        opf_path, package = _read_package(original)
        manifest = package.find(f"{{{OPF_NS}}}manifest")
        metadata = package.find(f"{{{OPF_NS}}}metadata")
        spine = package.find(f"{{{OPF_NS}}}spine")
        if manifest is None or metadata is None or spine is None:
            raise CoverError("EPUB manifest, metadata 또는 spine이 없습니다")
        obsolete = _remove_tool_covers(opf_path, manifest, metadata, spine)
        detached, updates = _detach_publisher_cover(original, opf_path, package, removing=True)
        obsolete.update(detached)
        for meta in metadata.findall(f"{{{OPF_NS}}}meta"):
            if meta.get("name") == "cover" and re.fullmatch(r"tm-cover-[a-f0-9]{16}", meta.get("content", "")):
                metadata.remove(meta)
        _write_epub(original, destination, opf_path, package, obsolete, updates)
    with destination.open("rb") as output:
        os.fsync(output.fileno())
    with zipfile.ZipFile(destination) as archive:
        if archive.testzip() is not None:
            raise CoverError("저장할 EPUB의 검증에 실패했습니다")
        # 표지를 지워도 책 자체는 읽혀야 한다.
        _read_package(archive)
        if _inspect_archive(archive)["has_cover"]:
            raise CoverError("표지를 삭제한 EPUB의 검증에 실패했습니다")


def _rewrite_epub(path: Path, expected_revision: str, backup_dir: Path, backup_name: str | None, stage: Callable[[BinaryIO, Path], None], precheck: Callable[[zipfile.ZipFile], None] | None = None) -> dict[str, Any]:
    """EPUB 를 잠그고 원본을 보존한 뒤 stage 가 만든 새 파일로 제자리에서 바꾼다. 쓰다 실패하면 원본으로 되돌린다.

    backup_name 을 주면(휴지통 보존) 그 이름으로 두고, 같은 이름이 이미 있으면 덮어쓰지 않고 처음 원본을 지킨다(PDF 편집과 같다).
    주지 않으면 편집마다 고유한 이름으로 둔다(표지 관리).
    """
    with path.open("r+b") as source:
        try:
            fcntl.flock(source, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as exc:
            raise CoverError("이 EPUB은 현재 저장 중입니다") from exc
        if revision(path) != expected_revision:
            raise CoverError("EPUB이 변경되었습니다. 목록을 새로 불러오세요")
        if precheck is not None:
            with zipfile.ZipFile(source) as archive:
                precheck(archive)
        original_stat = path.stat()
        backup_dir.mkdir(parents=True, exist_ok=True)
        named = backup_name is not None
        backup = backup_dir / (backup_name or hashlib.sha256(f"{path}:{expected_revision}".encode()).hexdigest() + f"-{uuid.uuid4().hex}.epub")
        backup_created = True
        source.seek(0)
        try:
            with backup.open("xb") as output:
                shutil.copyfileobj(source, output)
                output.flush()
                os.fsync(output.fileno())
        except FileExistsError:
            if not named:
                raise
            backup_created = False
        descriptor, filename = tempfile.mkstemp(prefix=".cover-", suffix=".tmp", dir=path.parent)
        os.close(descriptor)
        staged = Path(filename)
        # 이미 있던 백업은 처음 원본이라 지금 원본이 아니다. 실패 때 되돌릴 지금 원본은 따로 복사해 둔다.
        rollback = backup
        try:
            if not backup_created:
                descriptor, rollback_name = tempfile.mkstemp(prefix=".cover-rollback-", suffix=".tmp", dir=path.parent)
                os.close(descriptor)
                rollback = Path(rollback_name)
                source.seek(0)
                with rollback.open("wb") as output:
                    shutil.copyfileobj(source, output)
                    output.flush()
                    os.fsync(output.fileno())
            stage(source, staged)
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
                with rollback.open("rb") as reader:
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
            if rollback is not backup:
                rollback.unlink(missing_ok=True)
    return {"revision": revision(path), "backup_path": str(backup), "backup_created": backup_created, "file_size": path.stat().st_size, "book_id": original_stat.st_ino}


def add_cover(path: Path, image: bytes, expected_revision: str, backup_dir: Path, *, replace_existing: bool = False, backup_name: str | None = None) -> dict[str, Any]:
    image = normalize_image(image, max_width=600)
    with Image.open(io.BytesIO(image)) as cover:
        image_info = {"width": cover.width, "height": cover.height, "byte_size": len(image)}

    def refuse_existing(archive: zipfile.ZipFile) -> None:
        if _inspect_archive(archive)["has_cover"] and not replace_existing:
            raise CoverError("EPUB에 이미 표지가 있습니다")

    result = _rewrite_epub(path, expected_revision, backup_dir, backup_name, lambda source, staged: _stage_cover(source, staged, image), refuse_existing)
    return {**result, "cover_image": image_info}


def remove_cover(path: Path, expected_revision: str, backup_dir: Path, *, backup_name: str | None = None) -> dict[str, Any]:
    """EPUB 표지를 뺀다. 표지가 없거나 본문과 공유하면 원본과 백업을 건드리지 않는다."""

    def require_cover(archive: zipfile.ZipFile) -> None:
        opf_path, package = _read_package(archive)
        manifest, metadata, spine = (package.find(f"{{{OPF_NS}}}{name}") for name in ("manifest", "metadata", "spine"))
        if manifest is None or metadata is None or spine is None:
            raise CoverError("EPUB manifest, metadata 또는 spine이 없습니다")
        if not _inspect_archive(archive)["has_cover"]:
            raise CoverError("EPUB에 표지가 없습니다")
        _remove_tool_covers(opf_path, manifest, metadata, spine)
        _detach_publisher_cover(archive, opf_path, package, removing=True)

    result = _rewrite_epub(path, expected_revision, backup_dir, backup_name, _stage_cover_removal, require_cover)
    return {**result, "cover_image": None}


def add_illustration(path: Path, image: bytes, expected_revision: str, backup_dir: Path, before_idref: str, *, backup_name: str | None = None) -> dict[str, Any]:
    """before_idref 페이지 앞에 삽화 한 장을 넣는다. 표지는 건드리지 않는다."""
    image = normalize_image(image, max_width=1200)
    created: dict[str, str] = {}
    result = _rewrite_epub(path, expected_revision, backup_dir, backup_name, lambda source, staged: _stage_illustration(source, staged, image, before_idref, created))
    return {**result, "cover_image": None, "idref": created["idref"]}
