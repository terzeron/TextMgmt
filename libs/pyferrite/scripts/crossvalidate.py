#!/usr/bin/env python3
"""Check that pyferrite and Python agree, in both directions.

Python writes fixtures, Rust reads them and writes them back, and Python
verifies the result byte for byte. Any format whose Python library is missing
is skipped with a note rather than failing the run.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile

import numpy as np

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

try:
    import joblib
except ImportError:
    joblib = None
try:
    import h5py
except ImportError:
    h5py = None

GREEN, RED, YELLOW, DIM, RESET = "\033[32m", "\033[31m", "\033[33m", "\033[2m", "\033[0m"
passed = failed = skipped = 0


def ok(msg: str) -> None:
    global passed
    passed += 1
    print(f"  {GREEN}pass{RESET}  {msg}")


def bad(msg: str) -> None:
    global failed
    failed += 1
    print(f"  {RED}FAIL{RESET}  {msg}")


def skip(msg: str) -> None:
    global skipped
    skipped += 1
    print(f"  {YELLOW}skip{RESET}  {msg}")


def cargo(example: str, *args: str) -> None:
    subprocess.run(
        ["cargo", "run", "--quiet", "--example", example, *args],
        cwd=ROOT, check=True, stdout=subprocess.DEVNULL,
    )


SAMPLES = {
    "f64_2d": np.arange(12, dtype=np.float64).reshape(3, 4),
    "f32_2d": (np.arange(6, dtype=np.float32) * 1.5).reshape(2, 3),
    "i32_2d": np.array([[1, -2, 3], [4, 5, -6]], dtype=np.int32),
    "i64_1d": np.array([-1, 0, 7, 2 ** 40], dtype=np.int64),
    "u8_1d": np.array([0, 127, 255], dtype=np.uint8),
    "i16_1d": np.array([-32768, 0, 32767], dtype=np.int16),
    "bool_1d": np.array([True, False, True]),
    "f16_1d": np.array([1.5, -2.25, 65504.0], dtype=np.float16),
    "c128_1d": np.array([1 + 2j, -3 - 4j]),
    "empty": np.zeros((0,), dtype=np.float32),
    "scalar": np.array(42.0),
    "big": np.arange(10_000, dtype=np.float64).reshape(100, 100),
}
NUMERIC = {k: v for k, v in SAMPLES.items() if v.dtype.kind in "fiub"}


def compare(tag, got, want):
    got = np.asarray(got)
    if got.dtype != want.dtype:
        bad(f"{tag}: dtype {got.dtype} != {want.dtype}")
    elif got.shape != want.shape:
        bad(f"{tag}: shape {got.shape} != {want.shape}")
    elif not np.array_equal(got, want):
        n = int((got != want).sum())
        bad(f"{tag}: {n} of {want.size} values differ")
    else:
        ok(tag)


def main() -> int:
    print(f"{DIM}numpy {np.__version__}"
          f"{', joblib ' + joblib.__version__ if joblib else ''}"
          f"{', h5py ' + h5py.__version__ if h5py else ''}{RESET}")

    tmp = tempfile.mkdtemp(prefix="pyferrite-xval-")
    try:
        print("\nPython writes, pyferrite reads, pyferrite writes, Python verifies:")

        # --- .npy, one file per dtype -----------------------------------
        for name, a in SAMPLES.items():
            src, dst = f"{tmp}/{name}.npy", f"{tmp}/{name}.out.npy"
            np.save(src, a)
            cargo("conv", src, dst)
            compare(f".npy  {name:<8} {str(a.dtype):<10}", np.load(dst), a)

        # --- Fortran order ----------------------------------------------
        f = np.asfortranarray(np.arange(6, dtype=np.float64).reshape(2, 3))
        np.save(f"{tmp}/fort.npy", f)
        cargo("conv", f"{tmp}/fort.npy", f"{tmp}/fort.out.npy")
        compare(".npy  fortran order", np.load(f"{tmp}/fort.out.npy"), f)

        # --- .npz, plain and compressed ---------------------------------
        for tag, saver in (("plain", np.savez), ("compressed", np.savez_compressed)):
            src, dst = f"{tmp}/{tag}.npz", f"{tmp}/{tag}.out.npz"
            saver(src, **NUMERIC)
            cargo("conv", src, dst)
            back = np.load(dst)
            missing = set(NUMERIC) - set(back.files)
            if missing:
                bad(f".npz  {tag}: missing {sorted(missing)}")
            else:
                bads = [k for k in NUMERIC if not np.array_equal(back[k], NUMERIC[k])
                        or back[k].dtype != NUMERIC[k].dtype]
                (ok if not bads else bad)(
                    f".npz  {tag} ({len(NUMERIC)} arrays)"
                    + ("" if not bads else f": {bads}"))

        # --- pickle, every protocol -------------------------------------
        import pickle
        obj = {"name": "model", "n": 3, "lr": 0.01, "ok": True, "none": None,
               "lst": [1, 2, 3], "tup": (4.5, "s"), "big_int": 2 ** 80,
               "bytes": b"\x00\xff\x11", **NUMERIC}
        for proto in (2, 3, 4, 5):
            src, dst = f"{tmp}/p{proto}.pkl", f"{tmp}/p{proto}.out.pkl"
            with open(src, "wb") as fh:
                pickle.dump(obj, fh, protocol=proto)
            cargo("conv", src, dst)
            with open(dst, "rb") as fh:
                back = pickle.load(fh)
            bads = [k for k in NUMERIC
                    if not np.array_equal(np.asarray(back[k]), NUMERIC[k])]
            scalars = [k for k in ("name", "n", "lr", "ok", "none", "big_int", "bytes")
                       if back.get(k) != obj[k]]
            (ok if not bads and not scalars else bad)(
                f".pkl  protocol {proto}"
                + ("" if not bads and not scalars else f": arrays {bads}, scalars {scalars}"))

        # --- joblib -----------------------------------------------------
        if joblib is None:
            skip(".joblib (joblib is not installed)")
        else:
            for tag, kw in (("raw", {"compress": 0}), ("zlib", {"compress": ("zlib", 3)})):
                src, dst = f"{tmp}/j_{tag}.joblib", f"{tmp}/j_{tag}.out.joblib"
                joblib.dump(obj, src, **kw)
                cargo("conv", src, dst)
                back = joblib.load(dst)
                bads = [k for k in NUMERIC
                        if not np.array_equal(np.asarray(back[k]), NUMERIC[k])]
                (ok if not bads else bad)(f".joblib {tag}"
                                          + ("" if not bads else f": {bads}"))

        # --- HDF5 -------------------------------------------------------
        if h5py is None:
            skip(".h5 (h5py is not installed)")
        else:
            h5_ok = {k: v for k, v in NUMERIC.items() if v.dtype != np.bool_ and v.size}
            src, dst = f"{tmp}/plain.h5", f"{tmp}/plain.out.h5"
            with h5py.File(src, "w") as fh:
                for k, v in h5_ok.items():
                    fh[k] = v
                grp = fh.create_group("nested")
                grp["inner"] = h5_ok["f64_2d"]
            cargo("conv", src, dst)
            with h5py.File(dst, "r") as fh:
                bads = [k for k in h5_ok if k not in fh
                        or not np.array_equal(fh[k][...], h5_ok[k])]
                if "nested" not in fh or not np.array_equal(
                        fh["nested/inner"][...], h5_ok["f64_2d"]):
                    bads.append("nested/inner")
            (ok if not bads else bad)(f".h5   contiguous + groups"
                                      + ("" if not bads else f": {bads}"))

            # chunked, compressed and shuffled datasets, awkward geometries
            src, dst = f"{tmp}/chunk.h5", f"{tmp}/chunk.out.npz"
            geoms = [((13,), (5,), None), ((17, 11), (4, 3), "gzip"),
                     ((9, 9, 9), (2, 4, 5), None), ((5, 6, 7, 8), (2, 3, 4, 5), "gzip")]
            want = {}
            with h5py.File(src, "w") as fh:
                for i, (shape, chunks, comp) in enumerate(geoms):
                    a = np.arange(int(np.prod(shape)), dtype=np.float64).reshape(shape)
                    kw = {"chunks": chunks}
                    if comp:
                        kw["compression"] = comp
                    fh.create_dataset(f"d{i}", data=a, **kw)
                    want[f"d{i}"] = a
                fh.create_dataset("shuf", data=np.arange(400, dtype=np.float32).reshape(20, 20),
                                  chunks=(8, 8), compression="gzip", shuffle=True)
                want["shuf"] = np.arange(400, dtype=np.float32).reshape(20, 20)
            cargo("conv", src, dst)
            back = np.load(dst)
            bads = [k for k in want if not np.array_equal(back[k], want[k])]
            (ok if not bads else bad)(f".h5   chunked, gzip, shuffle ({len(want)} datasets)"
                                      + ("" if not bads else f": {bads}"))

        # --- structured arrays -------------------------------------------
        rec = np.array([(1, 0.5, 0), (2, -1.5, 1), (3, 2.25, 1)],
                       dtype=[("id", "<i8"), ("score", "<f8"), ("flag", "u1")])
        np.save(f"{tmp}/rec.npy", rec)
        cargo("conv", f"{tmp}/rec.npy", f"{tmp}/rec.out.npy")
        back = np.load(f"{tmp}/rec.out.npy")
        if back.dtype.names != rec.dtype.names:
            bad(f".npy  record array: fields {back.dtype.names} != {rec.dtype.names}")
        elif any(not np.array_equal(back[n], rec[n]) for n in rec.dtype.names):
            bad(".npy  record array: values differ")
        else:
            ok(".npy  record array (frame round-trip)")

        # --- torch, verified with an equivalent unpickler -----------------
        print("\npyferrite writes, Python verifies:")
        cargo("gen", tmp)
        verify_pt(f"{tmp}/w.pt")
        verify_pt_with_torch(f"{tmp}/w.pt")

    finally:
        shutil.rmtree(tmp, ignore_errors=True)

    print(f"\n{passed} passed, {failed} failed, {skipped} skipped")
    return 1 if failed else 0


def verify_pt(path: str) -> None:
    """Read a pyferrite-written .pt the way torch.load would."""
    import io
    import pickle
    import zipfile

    dtypes = {"FloatStorage": np.float32, "DoubleStorage": np.float64,
              "HalfStorage": np.float16, "LongStorage": np.int64,
              "IntStorage": np.int32, "ShortStorage": np.int16,
              "CharStorage": np.int8, "ByteStorage": np.uint8,
              "BoolStorage": np.bool_}
    z = zipfile.ZipFile(path)

    class Stub:
        def __init__(self, module, name):
            self.module, self.name = module, name

    def rebuild(storage, offset, size, stride, requires_grad, hooks):
        raw, dt = storage
        flat = np.frombuffer(raw, dtype=dt)
        if not size:
            return flat[offset]
        return np.lib.stride_tricks.as_strided(
            flat[offset:], size, tuple(s * flat.itemsize for s in stride)).copy()

    class Unpickler(pickle.Unpickler):
        def find_class(self, module, name):
            if (module, name) == ("torch._utils", "_rebuild_tensor_v2"):
                return rebuild
            if (module, name) == ("collections", "OrderedDict"):
                return dict
            return Stub(module, name)

        def persistent_load(self, pid):
            _tag, cls, key, _location, numel = pid
            dt = dtypes[cls.name]
            raw = z.read(f"archive/data/{key}")
            assert len(raw) == numel * np.dtype(dt).itemsize, f"storage {key} is the wrong size"
            return raw, dt

    obj = Unpickler(io.BytesIO(z.read("archive/data.pkl"))).load()
    expected = {
        "w": (np.arange(6, dtype=np.float32) * 1.5).reshape(2, 3),
        "b": np.array([-1, 0, 7, 2 ** 40], dtype=np.int64),
        "x": np.array([[0.5, -1.25], [3.0, 1e-300], [2.0, 9.75]]),
        "q": np.array([0, 1, 128, 254, 255], dtype=np.uint8),
    }
    bads = [k for k, v in expected.items()
            if k not in obj or not np.array_equal(obj[k], v) or obj[k].dtype != v.dtype]
    (ok if not bads else bad)(".pt   torch-equivalent unpickler"
                             + ("" if not bads else f": {bads}"))


def verify_pt_with_torch(path: str) -> None:
    """Load with the real torch.load and its default `weights_only=True`."""
    try:
        import torch
    except ImportError:
        skip(".pt   torch.load (torch not installed)")
        return
    try:
        obj = torch.load(path)
    except Exception as e:  # noqa: BLE001 - report whatever torch raised
        bad(f".pt   torch.load(weights_only=True): {str(e).splitlines()[0]}")
        return
    expected = {
        "w": (np.arange(6, dtype=np.float32) * 1.5).reshape(2, 3),
        "b": np.array([-1, 0, 7, 2 ** 40], dtype=np.int64),
        "x": np.array([[0.5, -1.25], [3.0, 1e-300], [2.0, 9.75]]),
        "q": np.array([0, 1, 128, 254, 255], dtype=np.uint8),
    }
    bads = [k for k, v in expected.items()
            if k not in obj or not np.array_equal(obj[k].numpy(), v) or obj[k].numpy().dtype != v.dtype]
    (ok if not bads else bad)(f".pt   torch.load(weights_only=True), torch {torch.__version__}"
                             + ("" if not bads else f": {bads}"))


if __name__ == "__main__":
    sys.exit(main())
