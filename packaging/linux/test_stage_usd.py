#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""stage-usd.sh on a synthetic OpenUSD install and synthetic sources (SB-G5, offline).

Nothing is built or downloaded: the "libraries" are small text files named like the real ones.
"""

from __future__ import annotations

import shutil
import subprocess
import tarfile
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
STAGE = HERE / "stage-usd.sh"
PINS = HERE.parents[1] / "packaging" / "usd-version.env"
BUNDLED = (
    "pxr/base/js/rapidjson/rapidjson.h",
    "pxr/base/tf/pxrLZ4/lz4.cpp",
    "pxr/base/tf/pxrCLI11/CLI11.h",
    "pxr/base/tf/pxrTslRobinMap/robin_hash.h",
    "pxr/base/tf/pxrDoubleConversion/ieee.h",
    "pxr/base/gf/ilmbase_half.h",
    "pxr/base/pegtl/pegtl.hpp",
)


def pin(key: str) -> str:
    for line in PINS.read_text(encoding="utf-8").splitlines():
        name, sep, value = line.partition("=")
        if sep and name == key:
            return value
    raise KeyError(key)


def write(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


@unittest.skipUnless(shutil.which("bash") and shutil.which("tar"), "bash and tar are needed")
class StageUsdTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        tmp = Path(self.tmp.name)
        self.version = pin("CFA4_USD_VERSION")
        self.prefix, self.src = tmp / "prefix", tmp / "src"
        write(self.prefix / "lib" / "libusd_tf.so", "tf")
        write(self.prefix / "lib" / "libtbb.so.12", "tbb")
        write(self.prefix / "lib" / "usd" / "usdGeom" / "resources" / "plugInfo.json", "{}")
        write(self.prefix / "lib" / "cmake" / "pxr" / "x.cmake", "cmake")
        write(self.prefix / "include" / "pxr" / "pxr.h", "header")
        write(self.prefix / "plugin" / "usd" / "plugInfo.json", "{}")
        write(self.prefix / "BUILDINFO.json", "{}")
        write(self.prefix / "SHA256SUMS", "")
        usd, tbb = self.src / f"usd-{self.version}", self.src / f"onetbb-{pin('CFA4_ONETBB_VERSION')}"
        write(usd / "LICENSE.txt", "tost")
        write(usd / "NOTICE.txt", "notice")
        for rel in BUNDLED:
            write(usd / rel, f"// Copyright test\n// Licensed under test terms for {rel}\n\n#include <x>\n")
        write(tbb / "LICENSE.txt", "apache")
        write(tbb / "third-party-programs.txt", "programs")

    def run_stage(self, out: Path) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["bash", str(STAGE), "--prefix", str(self.prefix), "--src", str(self.src), "--out", str(out)],
            capture_output=True,
            text=True,
            check=False,
        )

    def test_runtime_files_and_licences_only(self) -> None:
        out = Path(self.tmp.name) / "out"
        result = self.run_stage(out)
        self.assertEqual(result.returncode, 0, result.stderr)
        tgz = out / f"cfa4-usd-worker-{self.version}-x86_64.tar.gz"
        with tarfile.open(tgz) as tar:
            names = tar.getnames()
        base = f"opt/cfa4/usd-{self.version}"
        for needed in (
            f"{base}/lib/libusd_tf.so",
            f"{base}/lib/libtbb.so.12",
            f"{base}/lib/usd/usdGeom/resources/plugInfo.json",
            f"{base}/plugin/usd/plugInfo.json",
            f"{base}/BUILDINFO.json",
            f"{base}/SHA256SUMS",
            f"{base}/LICENSES/OpenUSD-LICENSE.txt",
            f"{base}/LICENSES/OpenUSD-NOTICE.txt",
            f"{base}/LICENSES/oneTBB-LICENSE.txt",
            f"{base}/LICENSES/THIRD-PARTY-NOTICES.txt",
            "usr/share/doc/cfa4-usd-worker/copyright",
        ):
            self.assertIn(needed, names)
        self.assertFalse(
            [n for n in names if "/include/" in n or n.endswith(".cmake")], "headers or CMake files staged"
        )
        notices = (out / "stage" / base / "LICENSES" / "THIRD-PARTY-NOTICES.txt").read_text(encoding="utf-8")
        for rel in BUNDLED:
            self.assertIn(rel, notices)

    def test_two_runs_give_identical_archives(self) -> None:
        a, b = Path(self.tmp.name) / "a", Path(self.tmp.name) / "b"
        self.assertEqual(self.run_stage(a).returncode, 0)
        self.assertEqual(self.run_stage(b).returncode, 0)
        name = f"cfa4-usd-worker-{self.version}-x86_64.tar.gz"
        self.assertEqual((a / name).read_bytes(), (b / name).read_bytes())

    def test_existing_output_is_refused(self) -> None:
        out = Path(self.tmp.name) / "out"
        out.mkdir()
        result = self.run_stage(out)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("exists", result.stderr)

    def test_missing_licence_source_is_refused(self) -> None:
        (self.src / f"usd-{self.version}" / "NOTICE.txt").unlink()
        result = self.run_stage(Path(self.tmp.name) / "out")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("NOTICE.txt", result.stderr)


if __name__ == "__main__":
    unittest.main()
