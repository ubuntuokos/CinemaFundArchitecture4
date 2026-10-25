from __future__ import annotations

import json
import shutil
import sys
from pathlib import Path

import pytest
from cfa4_devtools.runtime_deps import (
    audit_binary,
    embedded_libraries,
    main,
    parse_dpkg_search,
    parse_ldd,
    parse_needed,
    parse_symbol_versions,
    read_os_release,
    summarize,
)

OBJDUMP_P = """
Dynamic Section:
  NEEDED               libasound.so.2
  NEEDED               libc.so.6
  NEEDED               libgcc_s.so.1
  SONAME               ignored.so
"""

OBJDUMP_T = """
DYNAMIC SYMBOL TABLE:
0000000000000000      DF *UND*\t0000000000000000 (GLIBC_2.2.5) getenv
0000000000000000      DF *UND*\t0000000000000000 (GLIBC_2.39) pidfd_spawnp
0000000000000000      DF *UND*\t0000000000000000 (GLIBC_2.9) pipe2
0000000000000000      DF *UND*\t0000000000000000  GCC_3.0     _Unwind_Resume
0000000000000000      DF *UND*\t0000000000000000 (ALSA_0.9.0rc4) snd_pcm_open
0000000000000000  w   D  *UND*\t0000000000000000  Base        __gmon_start__
0000000000001000 g    DF .text\t0000000000000010  GLIBC_9.99  not_undefined
"""

LDD = """\tlinux-vdso.so.1 (0x00007ffd)
\tlibasound.so.2 => /lib/x86_64-linux-gnu/libasound.so.2 (0x00007f)
\tlibc.so.6 => /lib/x86_64-linux-gnu/libc.so.6 (0x00007f)
\tlibgcc_s.so.1 => not found
\t/lib64/ld-linux-x86-64.so.2 (0x00007f)
"""


def test_parsers_extract_needed_versions_and_paths() -> None:
    assert parse_needed(OBJDUMP_P) == ["libasound.so.2", "libc.so.6", "libgcc_s.so.1"]
    # 2.39 > 2.9 numerically; defined symbols (.text) are not requirements
    assert parse_symbol_versions(OBJDUMP_T) == {"ALSA": "0.9.0rc4", "GCC": "3.0", "GLIBC": "2.39"}
    assert parse_ldd(LDD) == {
        "ld-linux-x86-64.so.2": "/lib64/ld-linux-x86-64.so.2",
        "libasound.so.2": "/lib/x86_64-linux-gnu/libasound.so.2",
        "libc.so.6": "/lib/x86_64-linux-gnu/libc.so.6",
        "libgcc_s.so.1": "not found",
    }
    assert parse_dpkg_search("libc6:amd64: /usr/lib/x86_64-linux-gnu/libc.so.6\n") == "libc6:amd64"
    assert parse_dpkg_search("") is None
    assert parse_dpkg_search("libc6:amd64, libc6-dev:amd64: /usr/lib/x\n") == "libc6:amd64"


def test_dpkg_search_skips_diversions_in_any_language() -> None:
    # Measured on a Hungarian 26.04 host before run() forced LC_ALL=C.
    output = (
        "eltérítés libc6 által innen: /lib64/ld-linux-x86-64.so.2\n"
        "diversion by libc6 to: /lib64/ld-linux-x86-64.so.2.usr-is-merged\n"
        "libc6:amd64: /usr/lib64/ld-linux-x86-64.so.2\n"
    )
    assert parse_dpkg_search(output) == "libc6:amd64"
    assert parse_dpkg_search("eltérítés libc6 által innen: /x\n") is None


def test_embedded_libraries_ignore_names_glued_into_string_pools() -> None:
    # X11 symbol names stored without separators once matched as one huge "lib...libXi.so.6".
    data = b"\0XSetXlibControlsXkbOpenDisplayXVaCreateNestedListlibXi.so.6\0libXi.so\0" + b"a" * 10
    assert embedded_libraries(data, []) == ["libXi.so"]
    assert embedded_libraries(b"\0lib" + b"x" * 100 + b".so\0", []) == []


def test_embedded_libraries_exclude_elf_dependencies() -> None:
    data = b"\x00libvulkan.so.1\x00libc.so.6\x00libwayland-client.so.0\x00notalib\x00"
    assert embedded_libraries(data, ["libc.so.6"]) == ["libvulkan.so.1", "libwayland-client.so.0"]


def test_hostile_inputs_do_not_crash(tmp_path: Path) -> None:
    assert parse_symbol_versions("*UND* x (GLIBC_x.y) f\n") == {}
    assert audit_binary(tmp_path / "absent")["error"] == "missing"
    text = tmp_path / "script"
    text.write_text("#!/bin/sh\n")
    assert audit_binary(text)["error"] == "not an ELF binary"
    os_release = tmp_path / "os-release"
    os_release.write_text('ID=ubuntu\nVERSION_ID="26.04"\n=broken\nnot a pair\n')
    assert read_os_release(os_release) == {"ID": "ubuntu", "VERSION_ID": "26.04"}
    assert read_os_release(tmp_path / "missing") == {}


def test_summary_takes_the_highest_requirement_per_namespace() -> None:
    binaries = [
        {"symbol_versions": {"GLIBC": "2.34"}, "packages": {"libc.so.6": {"package": "libc6:amd64", "version": "2.42"}},
         "dlopen_candidates": ["libvulkan.so.1"], "resolved": {"libc.so.6": "/lib/libc.so.6"}},
        {"symbol_versions": {"GLIBC": "2.39", "GCC": "3.0"}, "packages": {},
         "dlopen_candidates": ["libEGL.so.1"], "resolved": {"libx.so": "not found"}},
        {"error": "missing"},
    ]
    summary = summarize(binaries)
    assert summary["min_symbol_versions"] == {"GCC": "3.0", "GLIBC": "2.39"}
    assert summary["packages"] == {"libc6:amd64": "2.42"}
    assert summary["dlopen_candidates"] == ["libEGL.so.1", "libvulkan.so.1"]
    assert summary["unresolved"] == ["libx.so"]


@pytest.mark.skipif(shutil.which("objdump") is None or not sys.platform.startswith("linux"), reason="needs objdump")
def test_real_elf_binary_end_to_end(tmp_path: Path) -> None:
    binary = Path(sys.executable).resolve()
    assert main([str(binary), "-o", str(tmp_path)]) == 0
    report = json.loads((tmp_path / "runtime-deps.json").read_text())
    entry = report["binaries"][0]
    assert "libc.so.6" in entry["needed"] or entry["needed"] == []  # static builds have none
    assert "GLIBC" in report["summary"]["min_symbol_versions"]
    assert "# CFA4 runtime dependencies" in (tmp_path / "runtime-deps.md").read_text()
