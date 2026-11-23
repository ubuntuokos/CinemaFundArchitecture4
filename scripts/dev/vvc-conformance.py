#!/usr/bin/env python3
"""JVET VVC conformance corpus (FDIS_r1) for crates/codecs/tests/vvc_conformance.rs.

    python3 scripts/dev/vvc-conformance.py fetch [--out DIR]

Downloads FDIS_r1.zip (283 bitstreams, ~300 MB; never committed) into DIR (default
target/fixtures/vvc-conformance) and unpacks every stream into DIR/<name>/. The archive's
SHA-256 is checked against the value recorded here. Then:

    scripts/dev/vvc-decoder.py build
    cargo test -p cfa4-codecs --release --features vvc-external --test vvc_conformance -- --nocapture
"""
import argparse, hashlib, pathlib, sys, urllib.request, zipfile

URL = "https://www.itu.int/wftp3/av-arch/jvet-site/bitstream_exchange/VVC/FDIS_r1.zip"
SHA256 = "a09612a220ac2f61d9621df5ce2caa2b0af1d0a7856a5229240185c99d7efa50"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("cmd", choices=["fetch"])
    ap.add_argument("--out", default=str(pathlib.Path(__file__).resolve().parents[2] / "target/fixtures/vvc-conformance"))
    out = pathlib.Path(ap.parse_args().out)
    out.mkdir(parents=True, exist_ok=True)
    zp = out / "FDIS_r1.zip"
    if not zp.exists():
        with urllib.request.urlopen(URL, timeout=600) as r, open(zp, "wb") as f:
            while chunk := r.read(1 << 20):
                f.write(chunk)
    h = hashlib.sha256(zp.read_bytes()).hexdigest()
    if h != SHA256:
        zp.unlink()
        print(f"FDIS_r1.zip SHA-256 {h} differs from the recorded {SHA256}", file=sys.stderr)
        return 1
    with zipfile.ZipFile(zp) as z:
        inner = [n for n in z.namelist() if n.endswith(".zip")]
        for n in inner:
            name = pathlib.Path(n).stem
            dest = out / name
            if dest.exists():
                continue
            with z.open(n) as f, zipfile.ZipFile(f) as zz:
                zz.extractall(dest)
    print(f"{len(inner)} streams in {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
