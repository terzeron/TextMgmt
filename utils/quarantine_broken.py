#!/usr/bin/env python3
"""
quarantine_broken.py

인코딩이 손상된 문서를 `0_broken` 디렉토리로 격리한다.

되돌릴 수 있게 만든다. 옮긴 내역을 매니페스트에 남기고 `undo` 로 원위치시킨다.
판정에 오탐이 섞일 수 있으므로 이동은 항상 복구 가능해야 한다.

원래 카테고리를 하위 디렉토리로 유지한다. `0_broken/3_판타지/...` 형태라
어느 카테고리에서 왔는지 눈으로 확인할 수 있고 되돌리기도 쉽다.

사용 예:
    uv run python utils/quarantine_broken.py plan            # 무엇을 옮길지만 출력
    uv run python utils/quarantine_broken.py move --limit 2  # 표본 2건만 옮겨 확인
    uv run python utils/quarantine_broken.py move            # 전체 이동
    uv run python utils/quarantine_broken.py undo            # 매니페스트 기준 원위치
"""

import argparse
import json
import logging
import shutil
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Dict, List, Optional

REPO_ROOT = Path(__file__).resolve().parent.parent
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from utils.detect_mojibake import DEFAULT_LIBRARY_ROOT, VERDICT_CORRUPTED  # noqa: E402

LOGGER = logging.getLogger("quarantine_broken")

BROKEN_DIR_NAME = "0_broken"
DEFAULT_MANIFEST = REPO_ROOT / "tmp" / "quarantine_manifest.json"


# 되돌리면 알려진 바이너리가 되는 파일의 매직 바이트.
# 바이너리를 UTF-16LE 로 읽고 UTF-8 로 저장하면 이런 상태가 된다.
#   OLE 매직 D0 CF 11 E0 A1 B1 1A E1
#   -> UTF-16LE 로 읽으면 U+CFD0 U+E011 U+B1A1 U+E11A
#   -> UTF-8 로 저장하면 ec bf 90 ee 80 91 eb 86 a1 ee 84 9a
# 이 변환은 문자가 전부 BMP 안에 있으면 무손실이라 원본을 그대로 복원할 수 있다.
BINARY_MAGIC = {
    bytes.fromhex("d0cf11e0a1b11ae1"): ".hwp",
    b"HWP Document File": ".hwp",
    b"%PDF": ".pdf",
    b"\x89PNG\r\n\x1a\n": ".png",
}


def unmangle_utf16(raw: bytes) -> Optional[bytes]:
    """
    UTF-16LE 로 읽혀 UTF-8 로 저장된 바이너리를 원래 바이트로 되돌린다.
    되돌린 결과가 알려진 바이너리 매직으로 시작할 때만 성공으로 본다.

    표본을 바이트로 자르면 마지막 문자가 잘리므로 꼬리를 다듬어 가며 시도한다.
    """
    for trim in range(4):
        chunk = raw[: len(raw) - trim] if trim else raw
        try:
            restored = chunk.decode("utf-8").encode("utf-16-le")
        except UnicodeDecodeError:
            continue
        if any(restored.startswith(m) for m in BINARY_MAGIC):
            return restored
        return None
    return None


def true_extension(raw: bytes) -> Optional[str]:
    """되돌린 바이트가 가리키는 실제 확장자. 해당 없으면 None."""
    restored = unmangle_utf16(raw)
    if restored is None:
        return None
    for magic, ext in BINARY_MAGIC.items():
        if restored.startswith(magic):
            return ext
    return None


def fix_extension(path: Path, dry_run: bool = False) -> Optional[Dict[str, Any]]:
    """
    확장자가 틀린 바이너리를 원래 바이트로 되돌리고 올바른 확장자로 바꾼다.
    대상이 아니면 None.

    무손실 왕복을 확인한 뒤에만 쓴다. 확인에 실패하면 손대지 않는다.
    """
    try:
        raw = path.read_bytes()
    except OSError as e:
        return {"path": str(path), "error": str(e)}

    ext = true_extension(raw)
    if ext is None or path.suffix.lower() == ext:
        return None

    restored = unmangle_utf16(raw)
    if restored is None:
        return None
    if restored.decode("utf-16-le").encode("utf-8") != raw:
        # 무손실이 아니면 건드리지 않는다
        return {"path": str(path), "error": "무손실 왕복 실패"}

    target = path.with_suffix(ext)
    n = 1
    while target.exists():
        target = path.with_name(f"{path.stem}__{n}{ext}")
        n += 1

    result = {"path": str(path), "restored_to": str(target), "before": len(raw), "after": len(restored)}
    if not dry_run:
        target.write_bytes(restored)
        path.unlink()
    return result


def load_corrupted(report_path: Path) -> List[Dict[str, Any]]:
    """검사 리포트에서 손상 판정 항목만 뽑는다. scan 리포트와 recheck 목록을 모두 받는다."""
    data = json.loads(report_path.read_text(encoding="utf-8"))
    records = data["corrupted"] if isinstance(data, dict) else data
    return [r for r in records if r.get("verdict", VERDICT_CORRUPTED) == VERDICT_CORRUPTED]


def plan_moves(records: List[Dict[str, Any]], library_root: Path) -> List[Dict[str, Any]]:
    """원본 경로마다 격리 경로를 정한다. 원래 카테고리를 하위 디렉토리로 유지한다."""
    broken_root = library_root / BROKEN_DIR_NAME
    moves: List[Dict[str, Any]] = []
    taken = set()
    for rec in records:
        src = Path(rec["path"])
        try:
            rel = src.relative_to(library_root)
        except ValueError:
            LOGGER.warning(f"라이브러리 밖 경로는 건너뛴다: {src}")
            continue
        if rel.parts[0] == BROKEN_DIR_NAME:
            continue
        dst = broken_root / rel

        # 이름이 겹치면 덮어쓰지 않고 접미사를 붙인다
        candidate = dst
        n = 1
        while candidate in taken or candidate.exists():
            candidate = dst.with_name(f"{dst.stem}__{n}{dst.suffix}")
            n += 1
        taken.add(candidate)

        moves.append({"src": str(src), "dst": str(candidate), "likeness": rec.get("likeness"), "size": rec.get("size", 0)})
    return moves


def apply_moves(moves: List[Dict[str, Any]], manifest_path: Path, dry_run: bool = False) -> Dict[str, Any]:
    """이동을 실행하고 매니페스트를 남긴다. 한 건이라도 옮겼으면 매니페스트를 쓴다."""
    done: List[Dict[str, Any]] = []
    failed: List[Dict[str, Any]] = []

    for mv in moves:
        src, dst = Path(mv["src"]), Path(mv["dst"])
        if not src.exists():
            failed.append({**mv, "error": "원본 없음"})
            continue
        if dry_run:
            done.append(mv)
            continue
        try:
            dst.parent.mkdir(parents=True, exist_ok=True)
            shutil.move(str(src), str(dst))
            done.append(mv)
        except OSError as e:
            failed.append({**mv, "error": str(e)})
            LOGGER.warning(f"이동 실패: {src} -> {dst}: {e}")

    manifest = {"created_at": datetime.now(timezone.utc).isoformat(timespec="seconds"), "dry_run": dry_run, "moved": done, "failed": failed}
    if done and not dry_run:
        # 기존 매니페스트가 있으면 합쳐서 누적한다. 나눠 옮겨도 undo 가 전부를 되돌린다.
        if manifest_path.exists():
            try:
                prev = json.loads(manifest_path.read_text(encoding="utf-8"))
                manifest["moved"] = prev.get("moved", []) + done
            except Exception as e:
                LOGGER.warning(f"기존 매니페스트를 읽지 못해 새로 쓴다: {e}")
        manifest_path.parent.mkdir(parents=True, exist_ok=True)
        manifest_path.write_text(json.dumps(manifest, ensure_ascii=False, indent=2), encoding="utf-8")
    return manifest


def undo_moves(manifest_path: Path, dry_run: bool = False) -> Dict[str, Any]:
    """매니페스트에 적힌 이동을 되돌린다"""
    if not manifest_path.exists():
        raise FileNotFoundError(f"매니페스트가 없다: {manifest_path}")
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))

    restored: List[Dict[str, Any]] = []
    failed: List[Dict[str, Any]] = []
    for mv in manifest.get("moved", []):
        src, dst = Path(mv["src"]), Path(mv["dst"])
        if not dst.exists():
            failed.append({**mv, "error": "격리본 없음"})
            continue
        if src.exists():
            failed.append({**mv, "error": "원위치에 파일이 이미 있다"})
            continue
        if dry_run:
            restored.append(mv)
            continue
        try:
            src.parent.mkdir(parents=True, exist_ok=True)
            shutil.move(str(dst), str(src))
            restored.append(mv)
        except OSError as e:
            failed.append({**mv, "error": str(e)})

    if restored and not dry_run:
        remaining = [m for m in manifest.get("moved", []) if m not in restored]
        manifest["moved"] = remaining
        manifest_path.write_text(json.dumps(manifest, ensure_ascii=False, indent=2), encoding="utf-8")
    return {"restored": restored, "failed": failed}


def summarize(moves: List[Dict[str, Any]]) -> None:
    from collections import Counter  # noqa: PLC0415

    by_cat: Counter[str] = Counter()
    total_size = 0
    for mv in moves:
        parts = Path(mv["dst"]).parts
        idx = parts.index(BROKEN_DIR_NAME)
        by_cat[parts[idx + 1] if idx + 1 < len(parts) else "(최상위)"] += 1
        total_size += mv.get("size") or 0
    print(f"\n대상 {len(moves):,}건, {total_size / 1024 / 1024:.1f}MB")
    print(f"  {'원래 카테고리':<26}{'건수':>6}")
    for cat, n in by_cat.most_common():
        print(f"  {cat:<26}{n:>6}")


def main() -> int:
    parser = argparse.ArgumentParser(description="손상 문서를 0_broken 으로 격리")
    parser.add_argument("command", choices=["plan", "move", "undo", "fix-ext"])
    parser.add_argument("--report", type=Path, default=REPO_ROOT / "tmp" / "mojibake_recheck.json")
    parser.add_argument("--library-root", type=Path, default=DEFAULT_LIBRARY_ROOT)
    parser.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    parser.add_argument("--limit", type=int, default=0, help="앞에서 N건만 처리 (0 이면 전체)")
    parser.add_argument("--exclude", action="append", default=None, help="이 문자열이 파일명에 있으면 제외 (반복 지정 가능)")
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()

    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")

    if args.command == "fix-ext":
        records = load_corrupted(args.report)
        fixed: List[Dict[str, Any]] = []
        skipped: List[Dict[str, Any]] = []
        for rec in records:
            res = fix_extension(Path(rec["path"]), dry_run=args.dry_run)
            if res is None:
                continue
            (skipped if "error" in res else fixed).append(res)
        label = "교정 예정" if args.dry_run else "교정 완료"
        print(f"{label} {len(fixed)}건")
        for f in fixed:
            print(f"  {Path(f['path']).name[:48]}")
            print(f"    -> {Path(f['restored_to']).name[:48]}  ({f['before']:,} -> {f['after']:,} 바이트)")
        for s_ in skipped:
            print(f"  건너뜀: {Path(s_['path']).name[:44]} - {s_['error']}")
        return 0

    if args.command == "undo":
        res = undo_moves(args.manifest, dry_run=args.dry_run)
        print(f"원위치 {len(res['restored']):,}건, 실패 {len(res['failed']):,}건")
        for f in res["failed"][:10]:
            print(f"  실패: {Path(f['dst']).name[:50]} - {f['error']}")
        return 0

    records = load_corrupted(args.report)
    if args.exclude:
        before = len(records)
        records = [r for r in records if not any(k in Path(r["path"]).name for k in args.exclude)]
        LOGGER.info(f"제외 패턴으로 {before - len(records)}건 제외")

    moves = plan_moves(records, args.library_root)
    if args.limit > 0:
        moves = moves[: args.limit]

    if args.command == "plan":
        summarize(moves)
        print("\n예시 10건")
        for mv in moves[:10]:
            print(f"  {Path(mv['src']).name[:52]:<54} likeness={mv['likeness']}")
        return 0

    summarize(moves)
    result = apply_moves(moves, args.manifest, dry_run=args.dry_run)
    label = "이동 예정" if args.dry_run else "이동 완료"
    print(f"\n{label} {len(result['moved']):,}건, 실패 {len(result['failed']):,}건")
    for f in result["failed"][:10]:
        print(f"  실패: {Path(f['src']).name[:50]} - {f['error']}")
    if not args.dry_run and result["moved"]:
        print(f"매니페스트: {args.manifest}")
        print("되돌리려면: uv run python utils/quarantine_broken.py undo")
    return 0


if __name__ == "__main__":
    sys.exit(main())
