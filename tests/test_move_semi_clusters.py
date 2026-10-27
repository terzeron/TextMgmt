import json
from pathlib import Path


def test_move_semi_clusters_main_uses_temporary_library_and_cleans_duplicates(tmp_path, monkeypatch, capsys):
    import utils.move_semi_clusters as module

    real_path = Path

    def mapped_path(value):
        return tmp_path if str(value) == "/mnt/data/text" else real_path(value)

    monkeypatch.setattr(module, "Path", mapped_path)
    source = tmp_path / "0_telegram"
    cases = {
        "준확실_3_무협": "무협.pdf",
        "준확실_3_판타지": "이윤기의 그리스 로마 신화 1.pdf",
        "준확실_3_여성향": "duplicate.epub",
        "준확실_9_BLGL": "bl.epub",
        "준확실_9_성인": "adult.pdf",
    }
    clusters = {}
    for cluster, filename in cases.items():
        source_file = source / cluster / filename
        source_file.parent.mkdir(parents=True, exist_ok=True)
        source_file.write_bytes(cluster.encode())
        clusters[cluster] = [{"file": str(source_file)}]
    clusters["준확실_9_BLGL"].append({"file": str(source / "missing.epub")})

    duplicate_source = Path(clusters["준확실_3_여성향"][0]["file"])
    duplicate_target = tmp_path / "3_여성향" / duplicate_source.name
    duplicate_target.parent.mkdir(parents=True)
    duplicate_target.write_bytes(b"keep existing")
    extra = source / "extra" / "이윤기의 그리스 로마 신화 2.pdf"
    extra.parent.mkdir(parents=True)
    extra.write_bytes(b"extra")
    (tmp_path / "hybrid_clustering_report.json").write_text(json.dumps({"clusters": clusters}), encoding="utf-8")

    module.main()

    assert (tmp_path / "3_무협" / "무협.pdf").is_file()
    assert (tmp_path / "1_서양고전" / "이윤기의 그리스 로마 신화 1.pdf").is_file()
    assert (tmp_path / "1_서양고전" / extra.name).is_file()
    assert (tmp_path / "3_여성향" / "duplicate.epub").read_bytes() == b"keep existing"
    assert not duplicate_source.exists()
    assert not (source / "missing.epub").exists()
    assert "총 처리: 신규 이동 5권, 중복 정리 1권" in capsys.readouterr().out


def test_move_semi_clusters_script_entrypoint_and_extra_duplicate_cleanup(tmp_path, monkeypatch, capsys):
    import pathlib
    import runpy
    import sys

    real_path = Path
    source = tmp_path / "0_telegram"
    source.mkdir()
    extra = source / "이윤기의 그리스 로마 신화 2.pdf"
    extra.write_bytes(b"duplicate")
    western = tmp_path / "1_서양고전"
    western.mkdir()
    (western / extra.name).write_bytes(b"keep")
    (tmp_path / "hybrid_clustering_report.json").write_text(json.dumps({"clusters": {}}), encoding="utf-8")

    def mapped_path(value):
        return tmp_path if str(value) == "/mnt/data/text" else real_path(value)

    monkeypatch.setattr(pathlib, "Path", mapped_path)
    monkeypatch.setattr(sys, "path", [entry for entry in sys.path if entry != str(real_path(__file__).resolve().parents[1])])
    runpy.run_module("utils.move_semi_clusters", run_name="__main__")

    assert not extra.exists()
    assert (western / extra.name).read_bytes() == b"keep"
    assert "중복 정리 1권" in capsys.readouterr().out
