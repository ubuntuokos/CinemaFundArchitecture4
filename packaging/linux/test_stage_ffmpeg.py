#!/usr/bin/env python3
"""stage-ffmpeg.sh with a synthetic FFmpeg 9.0.2 prefix (small shared libraries built with cc).

No FFmpeg is built or downloaded: the libraries only carry the SONAMEs and NEEDED entries the
script reads, and bin/ffmpeg is a shell script answering -version and -buildconf.
"""

import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
STAGE = HERE / "stage-ffmpeg.sh"


def system_library(soname: str) -> str | None:
    """Path of an installed system library (from ldconfig's cache)."""
    try:
        cache = subprocess.run(["ldconfig", "-p"], capture_output=True, text=True, check=True).stdout
    except (OSError, subprocess.CalledProcessError):
        return None
    for line in cache.splitlines():
        parts = line.split()
        if parts and parts[0] == soname and "x86-64" in line:
            return parts[-1]
    return None


# libzstd1 is required by dpkg itself, so every Ubuntu host has it, from a package with a copyright
# file: it stands in for the codec libraries (dav1d, opus, …) that the real build loads.
ZSTD = system_library("libzstd.so.1")
TOOLS = ("cc", "readelf", "patchelf", "ldd", "dpkg-query")


@unittest.skipUnless(all(shutil.which(t) for t in TOOLS) and ZSTD, f"needs {', '.join(TOOLS)} and libzstd1")
class StageFfmpegTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="cfa4-stage-ffmpeg-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def library(self, directory: Path, soname: str, *needed: str, rpath: str = "") -> None:
        source = self.root / "empty.c"
        source.write_text("int cfa4_fixture;\n")
        directory.mkdir(parents=True, exist_ok=True)
        real = directory / f"{soname}.1.0"
        args = ["cc", "-shared", "-fPIC", "-o", str(real), str(source), f"-Wl,-soname,{soname}",
                "-Wl,--no-as-needed", *needed]
        if rpath:
            args.append(f"-Wl,-rpath,{rpath}")
        subprocess.run(args, check=True)
        (directory / soname).symlink_to(real.name)
        # the development symlink the real install has too
        (directory / soname.split(".so")[0]).with_suffix(".so").symlink_to(soname)

    def fake_build(self, name="ffmpeg-9.0.2", *, version="9.0.2", nonfree=False, restricted=False):
        build = self.root / name
        lib = build / "lib"
        self.library(lib, "libavutil.so.60")
        extra = []
        if restricted:
            codecs = self.root / "codecs"
            for codec in ("libx264.so.164", "libx265.so.215"):
                self.library(codecs, codec)
                extra.append(str(codecs / codec))
            extra.append(f"-Wl,-rpath,{codecs}")
        self.library(lib, "libavcodec.so.62", str(lib / "libavutil.so.60"), ZSTD, *extra, rpath=str(lib))
        self.library(lib, "libavformat.so.62", str(lib / "libavcodec.so.62"), str(lib / "libavutil.so.60"),
                     rpath=str(lib))
        (build / "bin").mkdir()
        flags = "--enable-gpl --enable-version3 --enable-shared" + (" --enable-nonfree" if nonfree else "")
        ffmpeg = build / "bin/ffmpeg"
        ffmpeg.write_text(
            "#!/bin/sh\n"
            f'case "$*" in *-buildconf*) echo "  configuration: {flags}" ;;\n'
            f'  *) echo "ffmpeg version {version} Copyright (c) 2000-2026 the FFmpeg developers" ;; esac\n'
        )
        ffmpeg.chmod(0o755)
        doc = build / "share/doc/ffmpeg"
        doc.mkdir(parents=True)
        for file in ("LICENSE.md", "COPYING.GPLv3", "BUILDINFO"):
            (doc / file).write_text("fixture\n")
        return build

    def stage(self, build: Path, *args: str):
        return subprocess.run(["bash", str(STAGE), "--build", str(build), "--dest", str(self.root / "dest"), *args],
                              capture_output=True, text=True, timeout=60, check=False)

    def test_bundle_has_sonames_origin_runpath_licences_and_depends(self):
        build = self.fake_build()
        depends = self.root / "depends.txt"
        result = self.stage(build, "--bundle", "--depends-out", str(depends))
        self.assertEqual(result.returncode, 0, result.stderr)
        lib = self.root / "dest/lib"
        names = sorted(p.name for p in lib.iterdir())
        self.assertEqual(names, ["libavcodec.so.62", "libavformat.so.62", "libavutil.so.60", "libzstd.so.1"])
        self.assertFalse([p for p in lib.iterdir() if p.is_symlink()], "no development symlinks")
        for path in lib.iterdir():
            runpath = subprocess.run(["patchelf", "--print-rpath", str(path)], capture_output=True, text=True, check=False).stdout
            self.assertEqual(runpath.strip(), "$ORIGIN", path.name)
        self.assertTrue((self.root / "dest/share/doc/ffmpeg/BUILDINFO").is_file())
        self.assertTrue((self.root / "dest/share/doc/third-party/libzstd1.copyright").stat().st_size)
        self.assertEqual(depends.read_text().split(), ["libzstd1"], "glibc is already in nfpm.yaml")

    def test_deb_layout_does_not_bundle_system_libraries(self):
        result = self.stage(self.fake_build())
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse((self.root / "dest/lib/libzstd.so.1").exists())
        self.assertFalse((self.root / "dest/share/doc/third-party").exists())

    def test_base_build_with_restricted_codecs_is_refused(self):
        result = self.stage(self.fake_build(restricted=True), "--bundle")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("cfa4-restricted-extras", result.stderr)
        self.assertFalse((self.root / "dest").exists(), "refused before anything is written")

    def test_restricted_build_needs_x264_and_x265(self):
        result = self.stage(self.fake_build("ffmpeg-9.0.2-restricted"), "--restricted")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("not linked with libx264", result.stderr)
        # With both linked the codec check passes; the fixture codecs then fail the next check,
        # since a real restricted build takes libx264/libx265 from Ubuntu's archive (Depends).
        result = self.stage(self.fake_build("other-restricted", restricted=True), "--restricted")
        self.assertIn("libx264.so.164) does not come from an Ubuntu package", result.stderr)

    def test_nonfree_or_other_version_is_refused(self):
        for build, message in ((self.fake_build("nonfree", nonfree=True), "non-free"),
                               (self.fake_build("old", version="8.0.1"), "CFA4 ships 9.0.2")):
            with self.subTest(message=message):
                result = self.stage(build)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(message, result.stderr)

    def test_missing_licence_files_are_refused(self):
        build = self.fake_build()
        os.remove(build / "share/doc/ffmpeg/BUILDINFO")
        result = self.stage(build)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("BUILDINFO is missing", result.stderr)


if __name__ == "__main__":
    unittest.main()
