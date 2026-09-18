#!/usr/bin/env python3
"""backend/classifier/reader.py 테스트.

이 모듈은 ES 에 없는 파일을 판정할 때만 쓰는 대비책이라 통합 테스트로는 거의 안 밟힌다.
인코딩 판별은 네 번 고쳐 잡은 로직이라 분기별로 직접 확인한다.
"""

import zipfile
from pathlib import Path

import pytest

from backend.classifier.reader import (
    MIN_SYLLABLES_FOR_LIKENESS,
    decode_best,
    korean_likeness,
    read_document_text,
    read_epub_publisher,
    read_epub_text,
    read_txt_text,
)

KOREAN = "그는 아무 말도 하지 않았다 그리고 다시 길을 걸었다 바람이 불었다 "


def test_korean_likeness_needs_enough_syllables():
    # 음절이 모자라면 비율이 흔들려 판정하지 않는다.
    assert korean_likeness("짧은 글") is None
    assert korean_likeness("") is None

    text = KOREAN * 10
    assert len([c for c in text if "가" <= c <= "힣"]) >= MIN_SYLLABLES_FOR_LIKENESS
    score = korean_likeness(text)
    assert score is not None and 0.0 < score <= 1.0


def test_korean_likeness_is_low_for_uncommon_syllables():
    # 흔한 음절이 없으면 정상 한국어로 안 읽힌다.
    assert korean_likeness("쀍" * 60) == 0.0


def test_decode_best_empty():
    assert decode_best(b"") == ""


@pytest.mark.parametrize("bom,enc", [(b"\xff\xfe", "utf-16-le"), (b"\xfe\xff", "utf-16-be")])
def test_decode_best_utf16_by_bom(bom: bytes, enc: str):
    # BOM 을 못 알아보면 UTF-16 한국어 텍스트를 통째로 깨진 것으로 읽는다(실측 97건).
    raw = bom + KOREAN.encode(enc)
    assert "바람이" in decode_best(raw)


@pytest.mark.parametrize("enc", ["utf-16-le", "utf-16-be"])
def test_decode_best_utf16_by_null_density(enc: str):
    # BOM 이 없어도 널 바이트 밀도로 알아본다. ASCII 가 많아야 널이 충분히 깔린다.
    raw = ("hello world " * 40).encode(enc)
    assert len(raw) >= 200 and raw.count(0) > len(raw) * 0.25
    assert "hello world" in decode_best(raw)


def test_decode_best_plain_utf8():
    assert decode_best(KOREAN.encode("utf-8")) == KOREAN


def test_decode_best_trims_truncated_utf8_tail():
    # 표본을 바이트 수로 자르면 멀티바이트 문자 중간에서 끊긴다. 꼬리를 다듬어 살린다.
    raw = (KOREAN * 5).encode("utf-8")[:-1]
    text = decode_best(raw)
    assert "바람이" in text


def test_decode_best_cp949():
    # CP949 한국어는 유효한 UTF-8 이 되는 일이 거의 없어 엄격 UTF-8 이 실패한다.
    raw = (KOREAN * 5).encode("cp949")
    with pytest.raises(UnicodeDecodeError):
        raw.decode("utf-8")
    assert "바람이" in decode_best(raw)


def test_decode_best_cp949_with_truncated_tail():
    raw = (KOREAN * 5).encode("cp949")[:-1]
    assert "바람이" in decode_best(raw)


def test_decode_best_falls_back_to_most_korean_reading():
    # 중간에 깨진 바이트가 있으면 어느 인코딩으로도 깨끗이 안 읽힌다.
    # 그때는 한국어로 가장 잘 읽히는 쪽을 쓴다 — 여기서는 cp949.
    encoded = (KOREAN * 5).encode("cp949")
    raw = encoded[:40] + b"\xff" + encoded[40:]
    for enc in ("utf-8", "cp949", "euc-kr"):
        with pytest.raises(UnicodeDecodeError):
            raw.decode(enc)
    assert "바람이" in decode_best(raw)


def test_decode_best_all_garbage_returns_something():
    # 한국어가 아예 없으면 likeness 가 None 이라 첫 후보를 그대로 쓴다.
    assert decode_best(b"\xff" * 300) == ""


def test_read_txt_text_missing_file(tmp_path: Path):
    assert read_txt_text(tmp_path / "없는파일.txt", 100, 10) == ""


def test_read_txt_text_head_only(tmp_path: Path):
    f = tmp_path / "a.txt"
    f.write_text(KOREAN, encoding="utf-8")
    # 파일이 head 표본보다 작으면 꼬리는 읽지 않는다.
    assert read_txt_text(f, 1000, 100).strip() == KOREAN.strip()


def test_read_txt_text_reads_head_and_tail(tmp_path: Path):
    f = tmp_path / "b.txt"
    f.write_text("시작" + "가" * 5000 + "끝부분", encoding="utf-8")
    text = read_txt_text(f, 50, 20)
    assert "시작" in text
    assert "끝부분" in text


def test_read_txt_text_tail_disabled(tmp_path: Path):
    f = tmp_path / "c.txt"
    f.write_text("시작" + "가" * 5000 + "끝부분", encoding="utf-8")
    assert "끝부분" not in read_txt_text(f, 50, 0)


def test_read_txt_text_open_failure(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    f = tmp_path / "d.txt"
    f.write_text("내용", encoding="utf-8")

    def boom(*args, **kwargs):
        raise OSError("열 수 없음")

    monkeypatch.setattr("builtins.open", boom)
    assert read_txt_text(f, 100, 10) == ""


def _make_epub(path: Path, docs: dict[str, str], opf: str | None = None) -> Path:
    with zipfile.ZipFile(path, "w") as z:
        for name, body in docs.items():
            z.writestr(name, body)
        if opf is not None:
            z.writestr("content.opf", opf)
    return path


def test_read_epub_text_without_html_documents(tmp_path: Path):
    epub = _make_epub(tmp_path / "a.epub", {"mimetype": "application/epub+zip"})
    assert read_epub_text(epub, 100, 10) == ""


def test_read_epub_text_strips_tags_and_skips_front_matter(tmp_path: Path):
    docs = {
        "cover.xhtml": "<html><body>" + "표지" * 100 + "</body></html>",
        "body1.xhtml": "<html><body>" + KOREAN * 5 + "</body></html>",
    }
    epub = _make_epub(tmp_path / "b.epub", docs)
    text = read_epub_text(epub, 2000, 0)
    # 표지·목차에서 표본을 뽑으면 본문을 못 본다.
    assert "표지" not in text
    assert "바람이" in text
    assert "<" not in text


def test_read_epub_text_rotates_when_many_documents(tmp_path: Path):
    # 문서가 5개 이상이면 앞쪽(판권지)을 건너뛰고 가운데부터 읽는다.
    # 한 문서가 50자 미만이면 gather 가 건너뛰므로 넉넉히 채운다.
    docs = {f"ch{i}.xhtml": f"<p>{'장면' + str(i)} {KOREAN * 3}</p>" for i in range(6)}
    epub = _make_epub(tmp_path / "c.epub", docs)
    text = read_epub_text(epub, 400, 100)
    assert "장면2" in text
    # 앞쪽 두 문서는 가운데부터 읽는 규칙 때문에 예산 안에서 밀려난다.
    assert "장면0" not in text[:400]


def test_read_epub_text_skips_short_documents(tmp_path: Path):
    docs = {
        "a.xhtml": "<p>짧음</p>",
        "b.xhtml": "<p>" + KOREAN * 5 + "</p>",
    }
    epub = _make_epub(tmp_path / "d.epub", docs)
    text = read_epub_text(epub, 2000, 0)
    assert "짧음" not in text


def test_read_epub_text_on_broken_zip(tmp_path: Path):
    broken = tmp_path / "broken.epub"
    broken.write_bytes(b"not a zip at all")
    assert read_epub_text(broken, 100, 10) == ""


def test_read_epub_text_ignores_unreadable_member(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    docs = {"a.xhtml": "<p>" + KOREAN * 5 + "</p>"}
    epub = _make_epub(tmp_path / "e.epub", docs)

    original_read = zipfile.ZipFile.read

    def flaky(self, name, *args, **kwargs):
        if str(name).endswith(".xhtml"):
            raise RuntimeError("멤버가 깨짐")
        return original_read(self, name, *args, **kwargs)

    monkeypatch.setattr(zipfile.ZipFile, "read", flaky)
    # 한 멤버가 깨져도 나머지로 계속한다.
    assert read_epub_text(epub, 2000, 0) == ""


def test_read_document_text_dispatches_by_extension(tmp_path: Path):
    txt = tmp_path / "a.txt"
    txt.write_text(KOREAN, encoding="utf-8")
    assert "바람이" in read_document_text(txt)

    epub = _make_epub(tmp_path / "a.epub", {"body.xhtml": "<p>" + KOREAN * 5 + "</p>"})
    assert "바람이" in read_document_text(epub)

    other = tmp_path / "a.pdf"
    other.write_bytes(b"%PDF-1.4")
    assert read_document_text(other) == ""


def test_read_epub_publisher(tmp_path: Path):
    opf = "<package><metadata><dc:publisher>민음사</dc:publisher></metadata></package>"
    epub = _make_epub(tmp_path / "p.epub", {"body.xhtml": "<p>x</p>"}, opf=opf)
    assert read_epub_publisher(epub) == "민음사"


def test_read_epub_publisher_joins_multiple_and_strips_tags(tmp_path: Path):
    opf = "<package><dc:publisher><b>민음사</b></dc:publisher><dc:publisher>창비</dc:publisher><dc:publisher>  </dc:publisher></package>"
    epub = _make_epub(tmp_path / "q.epub", {"body.xhtml": "<p>x</p>"}, opf=opf)
    assert read_epub_publisher(epub) == "민음사 창비"


def test_read_epub_publisher_without_opf(tmp_path: Path):
    epub = _make_epub(tmp_path / "r.epub", {"body.xhtml": "<p>x</p>"})
    assert read_epub_publisher(epub) == ""


def test_read_epub_publisher_on_broken_zip(tmp_path: Path):
    broken = tmp_path / "broken.epub"
    broken.write_bytes(b"not a zip")
    assert read_epub_publisher(broken) == ""
