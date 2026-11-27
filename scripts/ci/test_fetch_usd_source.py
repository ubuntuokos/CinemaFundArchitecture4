# SPDX-License-Identifier: GPL-3.0-or-later
"""scripts/dev/fetch-usd-source.sh and packaging/usd-version.env, offline (G-USD-1, SB-G1/SB-G8).

No test touches the network: the script is run without --yes (it must not download) and with
--from-tarball on tiny synthetic archives and a throw-away pin file.
"""

from __future__ import annotations

import hashlib
import io
import re
import subprocess
import tarfile
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "dev" / "fetch-usd-source.sh"
PINS = ROOT / "packaging" / "usd-version.env"
NAMES = ("USD", "OPENSUBDIV", "ONETBB")


def read_pins(path: Path) -> dict[str, str]:
    pins: dict[str, str] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.startswith("#") or not line.strip():
            continue
        key, sep, value = line.partition("=")
        assert sep, f"not KEY=VALUE: {line!r}"
        pins[key] = value
    return pins


def make_tar(files: dict[str, bytes], extra: list[tarfile.TarInfo] | None = None) -> bytes:
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w") as tar:
        for name, data in files.items():
            info = tarfile.TarInfo(name)
            info.size = len(data)
            tar.addfile(info, io.BytesIO(data))
        for info in extra or []:
            tar.addfile(info)
    return buf.getvalue()


class PinFileTests(unittest.TestCase):
    def test_every_source_is_fully_pinned(self) -> None:
        pins = read_pins(PINS)
        for name in NAMES:
            with self.subTest(name=name):
                self.assertRegex(pins[f"CFA4_{name}_GIT_COMMIT"], r"^[0-9a-f]{40}$")
                self.assertRegex(pins[f"CFA4_{name}_SOURCE_SHA256"], r"^[0-9a-f]{64}$")
                self.assertRegex(pins[f"CFA4_{name}_LICENSE_SHA256"], r"^[0-9a-f]{64}$")
                self.assertRegex(pins[f"CFA4_{name}_GIT_URL"], r"^https://[A-Za-z0-9._/-]+$")
                self.assertIn(pins[f"CFA4_{name}_SIGNATURE_VERIFIED"], ("yes", "no"))
                self.assertTrue(pins[f"CFA4_{name}_VERSION"])

    def test_lines_are_plain_key_value(self) -> None:
        for line in PINS.read_text(encoding="utf-8").splitlines():
            if line.startswith("#") or not line.strip():
                continue
            self.assertRegex(line, r"^[A-Z0-9_]+=[^ \"'$`]*$", line)


class FetchScriptTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.dir = Path(self.tmp.name)

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def pin_file(self, tar_bytes: bytes, license_bytes: bytes = b"licence\n") -> Path:
        env = self.dir / "pins.env"
        env.write_text(
            "\n".join(
                [
                    "CFA4_USD_VERSION=1.2.3",
                    "CFA4_USD_GIT_URL=https://example.invalid/usd",
                    f"CFA4_USD_GIT_COMMIT={'a' * 40}",
                    f"CFA4_USD_SOURCE_SHA256={hashlib.sha256(tar_bytes).hexdigest()}",
                    f"CFA4_USD_LICENSE_SHA256={hashlib.sha256(license_bytes).hexdigest()}",
                    "CFA4_USD_NOTICE_SHA256=",
                    "CFA4_USD_SIGNATURE_VERIFIED=no",
                    "",
                ]
            ),
            encoding="utf-8",
        )
        return env

    def run_script(self, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(["bash", str(SCRIPT), *args], capture_output=True, text=True, check=False, timeout=60)

    def test_without_yes_it_does_not_download(self) -> None:
        dest = self.dir / "out"
        result = self.run_script("usd", "--dest", str(dest))
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertIn("Nothing was downloaded", result.stderr)
        self.assertFalse(dest.exists())

    def test_tarball_with_the_pinned_hash_is_unpacked(self) -> None:
        data = make_tar({"LICENSE.txt": b"licence\n", "src/a.txt": b"a"})
        env = self.pin_file(data)
        archive = self.dir / "usd.tar"
        archive.write_bytes(data)
        dest = self.dir / "out"
        result = self.run_script("usd", "--env", str(env), "--from-tarball", str(archive), "--dest", str(dest))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((dest / "usd-1.2.3" / "src" / "a.txt").is_file())

    def test_wrong_hash_leaves_nothing(self) -> None:
        data = make_tar({"LICENSE.txt": b"licence\n"})
        env = self.pin_file(data)
        archive = self.dir / "usd.tar"
        archive.write_bytes(make_tar({"LICENSE.txt": b"licence\n", "evil": b"x"}))
        dest = self.dir / "out"
        result = self.run_script("usd", "--env", str(env), "--from-tarball", str(archive), "--dest", str(dest))
        self.assertEqual(result.returncode, 1)
        self.assertIn("mismatch", result.stderr)
        self.assertFalse(dest.exists())

    def test_changed_licence_is_refused(self) -> None:
        data = make_tar({"LICENSE.txt": b"other licence\n"})
        env = self.pin_file(data)
        archive = self.dir / "usd.tar"
        archive.write_bytes(data)
        dest = self.dir / "out"
        result = self.run_script("usd", "--env", str(env), "--from-tarball", str(archive), "--dest", str(dest))
        self.assertEqual(result.returncode, 1)
        self.assertIn("LICENSE.txt", result.stderr)
        self.assertFalse(dest.exists())

    def test_parent_paths_are_refused_even_with_a_matching_hash(self) -> None:
        data = make_tar({"LICENSE.txt": b"licence\n", "../escape.txt": b"x"})
        env = self.pin_file(data)
        archive = self.dir / "usd.tar"
        archive.write_bytes(data)
        dest = self.dir / "out"
        result = self.run_script("usd", "--env", str(env), "--from-tarball", str(archive), "--dest", str(dest))
        self.assertEqual(result.returncode, 1)
        self.assertRegex(result.stderr, re.compile("absolute or parent paths"))
        self.assertFalse((self.dir / "escape.txt").exists())
        self.assertFalse(dest.exists())

    def test_existing_target_is_not_overwritten(self) -> None:
        data = make_tar({"LICENSE.txt": b"licence\n"})
        env = self.pin_file(data)
        archive = self.dir / "usd.tar"
        archive.write_bytes(data)
        dest = self.dir / "out"
        (dest / "usd-1.2.3").mkdir(parents=True)
        result = self.run_script("usd", "--env", str(env), "--from-tarball", str(archive), "--dest", str(dest))
        self.assertEqual(result.returncode, 1)
        self.assertIn("exists", result.stderr)


if __name__ == "__main__":
    unittest.main()
