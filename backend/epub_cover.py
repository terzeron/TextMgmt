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
from urllib.parse import quote, unquote, urlparse

from lxml import etree
from PIL import Image

from backend.book_manager import BookManager, _safe_xml_parser
from backend.image_edit import ImageCropError, crop_pil

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


def _add_image_page(opf_path: str, manifest: Any, spine: Any, original_names: set[str], added: dict[str, bytes], image: bytes, anchor: Any, *, prefix: str, suffix: str = ".jpg", media_type: str = "image/jpeg", after: bool = False) -> str:
    """이미지 한 장짜리 쪽을 anchor(spine itemref) 앞(after면 뒤)에 끼우고 새 쪽의 idref를 돌려준다."""
    digest = hashlib.sha256(image).hexdigest()[:16]
    existing_ids = {item.get("id") for item in manifest}
    existing_names = original_names | set(added)
    base = posixpath.dirname(opf_path)
    # 같은 이미지를 여러 곳에 넣을 수 있도록 겹치면 번호를 붙인다.
    for serial in range(1, 1000):
        token = digest if serial == 1 else f"{digest}-{serial}"
        image_id, page_id = f"{prefix}-{token}", f"{prefix}-page-{token}"
        image_href, page_href = image_id + suffix, page_id + ".xhtml"
        image_member, page_member = posixpath.join(base, image_href), posixpath.join(base, page_href)
        if not {image_id, page_id} & existing_ids and not {image_member, page_member} & existing_names:
            break
    else:
        raise CoverError("추가할 삽화 ID를 만들 수 없습니다")
    etree.SubElement(manifest, f"{{{OPF_NS}}}item", id=image_id, href=image_href, **{"media-type": media_type})
    etree.SubElement(manifest, f"{{{OPF_NS}}}item", id=page_id, href=page_href, **{"media-type": "application/xhtml+xml"})
    spine.insert(spine.index(anchor) + (1 if after else 0), etree.Element(f"{{{OPF_NS}}}itemref", idref=page_id))
    added[image_member] = image
    added[page_member] = _illustration_page(image_href)
    return page_id


def _verify_staged(destination: Path) -> None:
    with destination.open("rb") as output:
        os.fsync(output.fileno())
    with zipfile.ZipFile(destination) as archive:
        if archive.testzip() is not None:
            raise CoverError("저장할 EPUB의 검증에 실패했습니다")
        _read_package(archive)


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
        added: dict[str, bytes] = {}
        created["idref"] = _add_image_page(opf_path, manifest, spine, set(original.namelist()), added, image, target, prefix="tm-plate")
        _write_epub(original, destination, opf_path, package, set(), added)
    _verify_staged(destination)


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


_IMAGE_FORMATS = {"JPEG": (".jpg", "image/jpeg"), "PNG": (".png", "image/png")}
_TOOL_COVER_PREFIX = "tm-cover-page-"
# crops 의 idref 에 이 값을 주면 쪽이 아니라 표지 이미지를 자른다(표지 화면은 spine 항목이 아닐 수 있다).
COVER_TARGET = "@cover"
_XLINK_HREF = "{http://www.w3.org/1999/xlink}href"


class _PageInfo:
    """이미지 편집 대상 이미지(쪽 spine 항목의 index 번째)의 구조.

    page_member: 쪽 XHTML 멤버, image_member: 대상 이미지 멤버, count: 쪽 안 이미지 수,
    image_refs: 쪽 안 모든 이미지가 가리키는 멤버(문서 순서, 못 찾으면 ""), image_only: 보이는 글이 없는지.
    """

    def __init__(self, page_member: str, image_member: str, image_only: bool, svg_image: bool = False, *, count: int = 1, index: int = 0, image_refs: list[str] | None = None) -> None:
        self.page_member = page_member
        self.image_member = image_member
        self.image_only = image_only
        self.svg_image = svg_image  # <img> 가 아니라 <svg><image> 로 이미지를 넣은 쪽
        self.count = count
        self.index = index
        self.image_refs = image_refs if image_refs is not None else [image_member]

    @property
    def page_level(self) -> bool:
        """이미지 한 장뿐이고 글이 없어 쪽 단위(새 쪽 추가·쪽 삭제)로 분할·삭제할 수 있는 쪽인가."""
        return self.count == 1 and self.image_only


def _page_info(original: zipfile.ZipFile, base: str, items: dict[str, Any], idref: str, index: int = 0) -> _PageInfo:
    item = items.get(idref)
    page_member = _member_path(base, item.get("href", "")) if item is not None else ""
    if not page_member or page_member not in original.NameToInfo:
        raise CoverError("편집할 쪽을 찾을 수 없습니다. 책을 다시 열어 보세요")
    if original.getinfo(page_member).file_size > MAX_XML_BYTES:
        raise CoverError("크롭할 수 없습니다 (쪽 문서가 너무 큼)")
    try:
        root = etree.fromstring(original.read(page_member), _safe_xml_parser(recover=True))
    except etree.XMLSyntaxError as exc:
        raise CoverError("크롭할 수 없습니다 (쪽 문서를 읽을 수 없음)") from exc
    images = [element for element in root.iter() if isinstance(element.tag, str) and etree.QName(element).localname in ("img", "image")]
    if not images:
        raise CoverError("크롭할 수 없습니다 (이미지가 0개인 쪽)")
    if not 0 <= index < len(images):
        raise CoverError(f"크롭할 수 없습니다 (이미지를 찾을 수 없음: 이 쪽의 {index + 1}번째 이미지, 전체 {len(images)}개)")
    refs = [_member_path(posixpath.dirname(page_member), element.get("src") or element.get("href") or element.get(_XLINK_HREF, "")) for element in images]
    image_member = refs[index]
    if not image_member or image_member not in original.NameToInfo:
        raise CoverError("크롭할 수 없습니다 (이미지를 찾을 수 없음)")
    svg_image = etree.QName(images[index]).localname == "image"
    body = next((element for element in root.iter() if isinstance(element.tag, str) and etree.QName(element).localname == "body"), root)
    return _PageInfo(page_member, image_member, not _visible_text(body), svg_image, count=len(images), index=index, image_refs=refs)


_IMAGE_TAG = re.compile(r"<(?:img|image)\b[^>]*>", re.IGNORECASE)
_MASKED = re.compile(r"<!--.*?-->|<!\[CDATA\[.*?\]\]>", re.DOTALL)
_URL_ATTRIBUTE = re.compile(r"""((?<![\w:-])(?:src|href|xlink:href)\s*=\s*)(["'])(.*?)\2""", re.IGNORECASE | re.DOTALL)


def _decode_page(raw: bytes) -> tuple[str, str]:
    """쪽 XHTML 바이트를 (글, 인코딩)으로 푼다. BOM·xml 선언·meta charset 을 보고 없으면 utf-8. 풀 수 없으면 CoverError."""
    if raw.startswith(b"\xef\xbb\xbf"):
        encoding = "utf-8-sig"
    elif raw.startswith((b"\xff\xfe", b"\xfe\xff")):
        encoding = "utf-16"
    else:
        head = raw[:1024].decode("ascii", "ignore")
        found = re.search(r"""<\?xml[^>]*encoding\s*=\s*["']([\w.-]+)["']""", head) or re.search(r"""<meta[^>]*charset\s*=\s*["']?([\w.-]+)""", head, re.IGNORECASE)
        encoding = found.group(1) if found else "utf-8"
    try:
        return raw.decode(encoding), encoding
    except (UnicodeDecodeError, LookupError) as exc:
        raise CoverError("쪽 구조를 해석할 수 없습니다 (문자 인코딩을 읽을 수 없음)") from exc


def _image_tags(text: str) -> list[re.Match[str]]:
    """text 에서 <img>/<image> 시작 태그를 문서 순서로 찾는다. 주석·CDATA 안의 것은 뺀다."""
    masked = [match.span() for match in _MASKED.finditer(text)]
    return [match for match in _IMAGE_TAG.finditer(text) if not any(start <= match.start() < end for start, end in masked)]


def _split_tag_copy(tag: str, new_href: str) -> str:
    """이미지 태그의 src/href/xlink:href 값을 new_href 로 바꾼 복사본. 다른 속성은 그대로 둔다."""
    quoted = quote(new_href)
    return _URL_ATTRIBUTE.sub(lambda match: f"{match.group(1)}{match.group(2)}{quoted}{match.group(2)}", tag)


def _edit_page_images(raw: bytes, count: int, edits: list[tuple[int, str, str | None]]) -> bytes:
    """쪽 XHTML 원문에서 이미지 태그를 문자열로 바꾼다(다시 직렬화하지 않아 나머지는 바이트 그대로다).

    edits: [(index, "split" | "delete", split 이면 새 이미지 멤버 상대경로)]. index 내림차순으로 적용해 번호가 밀리지 않는다.
    """
    text, encoding = _decode_page(raw)
    tags = _image_tags(text)
    if len(tags) != count:
        raise CoverError("쪽 구조를 해석할 수 없습니다 (이미지 태그 수가 맞지 않음)")
    for index, kind, new_href in sorted(edits, key=lambda edit: edit[0], reverse=True):
        match = tags[index]
        start, end = match.span()
        if kind == "split":
            tag = match.group(0)
            # 영역 둘이 위아래로 이어서 보이도록 줄을 바꾼다(SVG 안에서는 쓸 수 없다).
            separator = "<br/>" if tag[1:4].lower() == "img" else ""
            text = text[:end] + separator + _split_tag_copy(tag, new_href or "") + text[end:]
        else:
            wrapper = re.search(r"<(p|div)\b[^>]*>\s*$", text[:start], re.IGNORECASE)
            closing = re.match(r"\s*</(\w+)\s*>", text[end:])
            if wrapper and closing and closing.group(1).lower() == wrapper.group(1).lower():
                start, end = wrapper.start(), end + closing.end()
            text = text[:start] + text[end:]
    try:
        return text.encode(encoding)
    except UnicodeEncodeError as exc:
        raise CoverError("쪽 구조를 해석할 수 없습니다 (문자 인코딩을 쓸 수 없음)") from exc


def _add_split_image(opf_path: str, manifest: Any, original_names: set[str], added: dict[str, bytes], image: bytes, *, suffix: str, media_type: str) -> str:
    """쪽 안 분할의 둘째 영역 이미지를 새 멤버로 추가하고 그 멤버 경로를 돌려준다."""
    digest = hashlib.sha256(image).hexdigest()[:16]
    existing_ids = {item.get("id") for item in manifest}
    existing_names = original_names | set(added)
    base = posixpath.dirname(opf_path)
    for serial in range(1, 1000):
        token = digest if serial == 1 else f"{digest}-{serial}"
        image_id = f"tm-split-{token}"
        image_member = posixpath.join(base, image_id + suffix)
        if image_id not in existing_ids and image_member not in existing_names:
            break
    else:
        raise CoverError("추가할 이미지 ID를 만들 수 없습니다")
    etree.SubElement(manifest, f"{{{OPF_NS}}}item", id=image_id, href=image_id + suffix, **{"media-type": media_type})
    added[image_member] = image
    return image_member


def _visible_text(element: Any) -> str:
    """element 안의 보이는 글. SVG 의 title·desc 는 화면에 그려지지 않으므로 뺀다."""
    parts: list[str] = []

    def walk(node: Any) -> None:
        hidden = isinstance(node.tag, str) and etree.QName(node).localname in ("title", "desc") and etree.QName(node).namespace == "http://www.w3.org/2000/svg"
        if not hidden:
            parts.append(node.text or "")
            for child in node:
                walk(child)
        parts.append(node.tail or "")

    parts.append(element.text or "")
    for child in element:
        walk(child)
    return "".join(parts).strip()


def _fit_svg_page(page_xml: bytes, old_size: tuple[int, int], new_size: tuple[int, int]) -> bytes:
    """<svg><image> 쪽의 이미지 크기를 크롭한 이미지 크기에 맞춘다(찌그러짐 방지).

    image 의 width/height 가 원래 픽셀 크기와 같으면 새 크기로 바꾸고, svg 의 viewBox 가 그 크기를 감쌌으면 같이 바꾼다.
    구조가 예상과 다르거나 문서를 읽을 수 없으면 쪽을 건드리지 않는다.
    """
    try:
        root = etree.fromstring(page_xml, _safe_xml_parser())
    except etree.XMLSyntaxError:
        return page_xml
    image = next((element for element in root.iter() if isinstance(element.tag, str) and etree.QName(element).localname == "image"), None)
    if image is None:
        return page_xml
    old_w, old_h = old_size
    new_w, new_h = new_size
    changed = False
    if image.get("width") == str(old_w) and image.get("height") == str(old_h):
        image.set("width", str(new_w))
        image.set("height", str(new_h))
        changed = True
    for svg in image.iterancestors():
        if isinstance(svg.tag, str) and etree.QName(svg).localname == "svg" and svg.get("viewBox", "").split() == ["0", "0", str(old_w), str(old_h)]:
            svg.set("viewBox", f"0 0 {new_w} {new_h}")
            changed = True
            break
    if not changed:
        return page_xml
    return etree.tostring(root.getroottree(), xml_declaration=True, encoding="utf-8")


def _image_size(data: bytes) -> tuple[int, int]:
    with Image.open(io.BytesIO(data)) as image:
        return image.width, image.height


def _reference_variants(member: str) -> list[bytes]:
    name = posixpath.basename(member)
    return list({name.encode(), quote(name).encode()})


def _crop_image_bytes(data: bytes, rect: tuple[float, float, float, float, float]) -> tuple[bytes, str]:
    """JPEG/PNG 이미지를 rect 로 잘라 같은 형식으로 다시 인코딩한다: (바이트, 형식 이름)."""
    try:
        with warnings.catch_warnings():
            warnings.simplefilter("ignore", Image.DecompressionBombWarning)
            with Image.open(io.BytesIO(data)) as image:
                fmt = image.format or ""
                if fmt not in _IMAGE_FORMATS:
                    raise CoverError(f"크롭할 수 없습니다 (지원하지 않는 이미지 형식 {fmt or '알 수 없음'})")
                if image.width * image.height > MAX_IMAGE_PIXELS:
                    raise CoverError("크롭할 수 없습니다 (이미지 해상도가 너무 큼)")
                image.load()
                source: Image.Image = image
                if source.mode not in ("RGB", "L", "1"):
                    if "A" in source.mode or (source.mode == "P" and "transparency" in source.info):
                        rgba = source.convert("RGBA")
                        flattened = Image.new("RGB", rgba.size, (255, 255, 255))
                        flattened.paste(rgba, mask=rgba.getchannel("A"))
                        source = flattened
                    else:
                        source = source.convert("RGB")
                try:
                    cropped = crop_pil(source, rect)
                except ImageCropError as exc:
                    raise CoverError(f"크롭할 수 없습니다 ({exc})") from None
                output = io.BytesIO()
                if fmt == "JPEG":
                    cropped.save(output, "JPEG", quality=95)
                else:
                    cropped.save(output, "PNG")
                return output.getvalue(), fmt
    except CoverError:
        raise
    except Exception as exc:
        raise CoverError("크롭할 수 없습니다 (이미지를 읽을 수 없음)") from exc


def _stage_image_edits(source: BinaryIO, destination: Path, crops: list[dict[str, Any]], splits: list[dict[str, Any]], deletes: list[dict[str, Any]], inserts: list[tuple[str, bytes]], created: dict[str, Any]) -> None:
    source.seek(0)
    with zipfile.ZipFile(source) as original:
        opf_path, package = _read_package(original)
        manifest = package.find(f"{{{OPF_NS}}}manifest")
        spine = package.find(f"{{{OPF_NS}}}spine")
        if manifest is None or spine is None:
            raise CoverError("EPUB manifest 또는 spine이 없습니다")
        base = posixpath.dirname(opf_path)
        items = {item.get("id"): item for item in manifest}
        references = {reference.get("idref"): reference for reference in spine}
        original_names = set(original.namelist())
        added: dict[str, bytes] = {}
        obsolete: set[str] = set()
        spine_members = {idref: _member_path(base, items[idref].get("href", "")) for idref in references if idref in items}
        page_bytes: dict[str, bytes] = {}
        # 쪽 안에서 처리하는 분할·삭제: 쪽 멤버 → (이미지 수, [(index, 종류, 새 이미지 상대경로)]). 모두 모은 뒤 쪽마다 한 번에 적용한다.
        page_edits: dict[str, tuple[int, list[tuple[int, str, str | None]]]] = {}

        def read_page(member: str) -> bytes:
            if member not in page_bytes:
                info = original.getinfo(member) if member in original.NameToInfo else None
                page_bytes[member] = original.read(member) if info is not None and info.file_size <= MAX_XML_BYTES * 4 else b""
            return page_bytes[member]

        def used_elsewhere(image_member: str, own_pages: set[str], *, include_styles: bool) -> bool:
            variants = _reference_variants(image_member)
            members = {member for member in spine_members.values() if member and member not in own_pages}
            if include_styles:
                members |= {_member_path(base, item.get("href", "")) for item in manifest if item.get("media-type") == "text/css"}
            return any(variant in read_page(member) for member in members if member in original.NameToInfo for variant in variants)

        def info_for(entry: dict[str, Any]) -> _PageInfo:
            return _page_info(original, base, items, entry["idref"], entry.get("index", 0))

        def refuse_shared(info: _PageInfo) -> None:
            """대상 이미지 멤버를 다른 쪽이나 같은 쪽의 다른 이미지 태그도 쓰면 제자리 교체가 그쪽도 바꾸므로 거절한다."""
            if used_elsewhere(info.image_member, {info.page_member}, include_styles=False) or info.image_refs.count(info.image_member) > 1:
                raise CoverError("크롭할 수 없습니다 (이미지를 다른 쪽과 공유함)")

        def add_page_edit(info: _PageInfo, kind: str, new_href: str | None = None) -> None:
            count, edits = page_edits.setdefault(info.page_member, (info.count, []))
            edits.append((info.index, kind, new_href))

        for entry in crops:
            if entry["idref"] == COVER_TARGET:
                # 표지 화면의 이미지: 쪽(spine 항목)이 아니라 표지 이미지 멤버를 제자리에서 자른다.
                previous = _read_previous_cover(original, opf_path, manifest)
                if previous is None:
                    raise CoverError("크롭할 수 없습니다 (표지 이미지를 찾을 수 없음)")
                cover_member, cover_data, _, _ = previous
                data, _ = _crop_image_bytes(cover_data, entry["rect"])
                added[cover_member] = data
                # 표지 쪽이 <svg><image> 로 이 이미지를 그리면 크기를 새 이미지에 맞춘다.
                for page_member in {member for member in spine_members.values() if member in original.NameToInfo}:
                    if any(variant in read_page(page_member) for variant in _reference_variants(cover_member)):
                        added[page_member] = _fit_svg_page(read_page(page_member), _image_size(cover_data), _image_size(data))
                continue
            info = info_for(entry)
            refuse_shared(info)
            image_data = original.read(info.image_member)
            data, _ = _crop_image_bytes(image_data, entry["rect"])
            added[info.image_member] = data
            if info.svg_image and info.count == 1:
                added[info.page_member] = _fit_svg_page(original.read(info.page_member), _image_size(image_data), _image_size(data))
        for entry in splits:
            idref = entry["idref"]
            if idref.startswith(_TOOL_COVER_PREFIX):
                raise CoverError("표지 쪽은 분할할 수 없습니다")
            info = info_for(entry)
            refuse_shared(info)
            image_data = original.read(info.image_member)
            first, fmt = _crop_image_bytes(image_data, entry["first"])
            second, _ = _crop_image_bytes(image_data, entry["second"])
            suffix, media_type = _IMAGE_FORMATS[fmt]
            added[info.image_member] = first
            if info.page_level:
                if info.svg_image:
                    added[info.page_member] = _fit_svg_page(original.read(info.page_member), _image_size(image_data), _image_size(first))
                created["split"][idref] = _add_image_page(opf_path, manifest, spine, original_names, added, second, references[idref], prefix="tm-split", suffix=suffix, media_type=media_type, after=True)
            else:
                new_member = _add_split_image(opf_path, manifest, original_names, added, second, suffix=suffix, media_type=media_type)
                add_page_edit(info, "split", posixpath.relpath(new_member, posixpath.dirname(info.page_member)))
        for before_idref, image in inserts:
            target = references.get(before_idref)
            if target is None:
                raise CoverError("삽화를 넣을 위치를 찾을 수 없습니다. 책을 다시 열어 보세요")
            if before_idref.startswith(_TOOL_COVER_PREFIX):
                raise CoverError("표지 앞에는 삽화를 넣을 수 없습니다")
            created["inserted"].append(_add_image_page(opf_path, manifest, spine, original_names, added, image, target, prefix="tm-plate"))
        if deletes:
            toc_members = [_member_path(base, item.get("href", "")) for item in manifest if "nav" in (item.get("properties") or "").split() or item.get("media-type") == "application/x-dtbncx+xml"]
            toc_bytes = [original.read(member) for member in toc_members if member in original.NameToInfo and original.getinfo(member).file_size <= MAX_XML_BYTES]
            infos = []
            for entry in deletes:
                if entry["idref"].startswith(_TOOL_COVER_PREFIX):
                    raise CoverError("표지 쪽은 삭제할 수 없습니다")
                infos.append((entry["idref"], info_for(entry)))
            doomed_pages = {info.page_member for _, info in infos if info.page_level}
            page_deleted = False
            for idref, info in infos:
                if info.page_level:
                    if any(variant in data for data in toc_bytes for variant in _reference_variants(info.page_member)):
                        raise CoverError("목차가 가리키는 쪽은 삭제할 수 없습니다")
                    spine.remove(references[idref])
                    manifest.remove(items[idref])
                    obsolete.add(info.page_member)
                    page_deleted = True
                    own_pages = doomed_pages
                else:
                    add_page_edit(info, "delete")
                    # 같은 쪽의 다른 이미지 태그가 같은 멤버를 쓰면 이미지 파일은 남긴다.
                    if info.image_refs.count(info.image_member) > 1:
                        continue
                    own_pages = {info.page_member}
                if not used_elsewhere(info.image_member, own_pages, include_styles=True):
                    image_item = next((item for item in manifest if _member_path(base, item.get("href", "")) == info.image_member), None)
                    if image_item is not None and "cover-image" in (image_item.get("properties") or "").split():
                        continue
                    if image_item is not None:
                        manifest.remove(image_item)
                    obsolete.add(info.image_member)
            if page_deleted and not any(True for _ in spine):
                raise CoverError("모든 쪽을 삭제할 수는 없습니다")
        for page_member, (count, edits) in page_edits.items():
            added[page_member] = _edit_page_images(added.get(page_member) or original.read(page_member), count, edits)
        _write_epub(original, destination, opf_path, package, obsolete, added)
    _verify_staged(destination)


def edit_images(path: Path, expected_revision: str, backup_dir: Path, *, crops: list[dict[str, Any]], splits: list[dict[str, Any]], deletes: list[Any], inserts: list[tuple[str, bytes]], backup_name: str | None = None) -> dict[str, Any]:
    """EPUB 쪽의 이미지를 한 번에 크롭·분할·삭제·삽입한다(원자적).

    crops: [{"idref", "index"?, "rect": (x0, y0, x1, y1, angle)}], splits: [{"idref", "index"?, "first": rect, "second": rect}],
    deletes: [{"idref", "index"?}], inserts: [(before_idref, 이미지 바이트)]. index 는 쪽 안 이미지의 문서 순서(기본 0).
    같은 (idref, index) 는 크롭·분할·삭제 중 한 곳에만 나올 수 있다.
    """
    # deletes 는 [{"idref", "index"}] 이거나(구형) idref 문자열 목록이다.
    deletes = [{"idref": entry} if isinstance(entry, str) else entry for entry in deletes]
    targets = [(entry["idref"], 0 if entry["idref"] == COVER_TARGET else entry.get("index", 0)) for entry in (*crops, *splits, *deletes)]
    if not targets and not inserts:
        raise CoverError("저장할 이미지 편집이 없습니다")
    if len(set(targets)) != len(targets):
        raise CoverError("같은 쪽에 편집을 두 번 지정할 수 없습니다")
    if any(index < 0 for _, index in targets):
        raise CoverError("이미지 위치가 올바르지 않습니다")
    normalized = [(before, normalize_image(image, max_width=1200)) for before, image in inserts]
    created: dict[str, Any] = {"inserted": [], "split": {}}
    result = _rewrite_epub(path, expected_revision, backup_dir, backup_name, lambda source, staged: _stage_image_edits(source, staged, crops, splits, deletes, normalized, created))
    return {**result, "cover_image": None, "idrefs": created}
