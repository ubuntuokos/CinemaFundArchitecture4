"""Measured runtime dependencies of the CFA4 binaries (F2-01, issue #22).

Run on the Ubuntu 26.04 reference machine, after a release build:

    bash scripts/dev/audit-runtime-deps.sh

For each binary it records, without installing anything:
- the ELF `NEEDED` libraries (`objdump -p`) and where `ldd` resolves them;
- the highest symbol version required per namespace (GLIBC_2.x, GCC_x, ...) from `objdump -T`;
- library names embedded in the binary that are loaded at run time with dlopen
  (Wayland, X11, Vulkan, EGL, NVIDIA, VA-API), which `ldd` cannot see;
- the Debian package and installed version that owns each resolved library (`dpkg-query`).

Missing tools are recorded as such; the audit never crashes on a malformed binary or output.
The result is evidence for the F9 `runtime_abi_dependencies` gate only when `host.ubuntu_2604`
is true.
"""

from __future__ import annotations

import argparse
import json
import os
import platform
import re
import shutil
import subprocess
import sys
from datetime import UTC, datetime
from pathlib import Path

SCHEMA = "cfa4-runtime-deps/1"
MAX_BINARY_BYTES = 1 << 30
TOOL_TIMEOUT_S = 120

NEEDED_RE = re.compile(r"^\s*NEEDED\s+(\S+)\s*$", re.MULTILINE)
SYMBOL_VERSION_RE = re.compile(r"\*UND\*\s+\S+\s+\(?([A-Za-z_]+)_([0-9][0-9A-Za-z]*(?:\.[0-9][0-9A-Za-z]*)*)\)?\s")
LDD_RE = re.compile(r"^\s*(\S+)\s+=>\s+(not found|\S+)", re.MULTILINE)
# The dynamic loader is printed by absolute path without "=>".
LDD_LOADER_RE = re.compile(r"^\s*(/\S+/(ld-linux[^/\s]*))\s+\(", re.MULTILINE)
# Symbol tables are NUL-separated, but string pools glue names together ("XSetXlibControls...libXi.so"):
# a library name starts after a non-identifier byte and is short.
EMBEDDED_LIB_RE = re.compile(rb"(?<![A-Za-z0-9_])lib[A-Za-z0-9_+\-]{1,64}\.so(?:\.[0-9]+)*")


def run(argv: list[str]) -> str | None:
    """Run a tool and return its stdout, or None when it is missing or fails."""
    if shutil.which(argv[0]) is None:
        return None
    # Parsers expect untranslated output (dpkg prints "diversion by" in English only under C).
    env = {**os.environ, "LC_ALL": "C", "LANGUAGE": ""}
    try:
        result = subprocess.run(
            argv, capture_output=True, text=True, timeout=TOOL_TIMEOUT_S, check=False, env=env
        )
    except (OSError, subprocess.SubprocessError):
        return None
    return result.stdout if result.returncode == 0 else None


def parse_needed(objdump_p: str) -> list[str]:
    return sorted(set(NEEDED_RE.findall(objdump_p)))


def _version_key(version: str) -> tuple[int, ...]:
    parts: list[int] = []
    for piece in version.split("."):
        digits = re.match(r"[0-9]+", piece)
        parts.append(int(digits.group()) if digits else 0)
    return tuple(parts)


def parse_symbol_versions(objdump_t: str) -> dict[str, str]:
    """Highest required version per namespace, e.g. {"GLIBC": "2.38", "GCC": "3.0"}."""
    best: dict[str, str] = {}
    for namespace, version in SYMBOL_VERSION_RE.findall(objdump_t):
        current = best.get(namespace)
        if current is None or _version_key(version) > _version_key(current):
            best[namespace] = version
    return dict(sorted(best.items()))


def parse_ldd(ldd: str) -> dict[str, str]:
    resolved = dict(LDD_RE.findall(ldd))
    for path, name in LDD_LOADER_RE.findall(ldd):
        resolved.setdefault(name, path)
    return dict(sorted(resolved.items()))


def embedded_libraries(data: bytes, needed: list[str]) -> list[str]:
    """Library names in the binary that are not ELF dependencies (dlopen candidates)."""
    found = {m.decode("ascii", "replace") for m in EMBEDDED_LIB_RE.findall(data)}
    return sorted(found - set(needed))


def parse_dpkg_search(output: str) -> str | None:
    """`dpkg-query -S /path` prints `pkg:arch: /path`; return `pkg:arch`."""
    for line in output.splitlines():
        package, sep, _ = line.partition(": ")
        # Diversion lines ("diversion by libc6 from: /path") contain spaces; package names never do.
        first = package.split(",")[0].strip()
        if sep and first and " " not in first:
            return first
    return None


def package_for(path: str) -> dict[str, str] | None:
    if path == "not found" or not path.startswith("/"):
        return None
    candidates = [path]
    resolved = str(Path(path).resolve())
    if resolved != path:
        candidates.append(resolved)
    # merged /usr: /lib/x86_64-linux-gnu is registered under /usr/lib in dpkg
    candidates += [c.replace("/lib/", "/usr/lib/", 1) for c in list(candidates) if c.startswith("/lib/")]
    for candidate in candidates:
        output = run(["dpkg-query", "-S", candidate])
        package = parse_dpkg_search(output) if output else None
        if package:
            version = run(["dpkg-query", "-W", "-f=${Version}", package])
            return {"package": package, "version": (version or "").strip() or "unknown"}
    return None


def audit_binary(path: Path) -> dict:
    entry: dict = {"path": str(path)}
    if not path.is_file():
        entry["error"] = "missing"
        return entry
    try:
        size = path.stat().st_size
        data = path.read_bytes() if size <= MAX_BINARY_BYTES else b""
    except OSError as exc:
        entry["error"] = f"unreadable: {exc}"
        return entry
    if not data.startswith(b"\x7fELF"):
        entry["error"] = "not an ELF binary"
        return entry
    objdump_p = run(["objdump", "-p", str(path)])
    objdump_t = run(["objdump", "-T", str(path)])
    ldd = run(["ldd", str(path)])
    needed = parse_needed(objdump_p) if objdump_p is not None else []
    entry["size_bytes"] = size
    entry["needed"] = needed
    entry["symbol_versions"] = parse_symbol_versions(objdump_t) if objdump_t is not None else {}
    resolved = parse_ldd(ldd) if ldd is not None else {}
    entry["resolved"] = {name: resolved.get(name, "unresolved") for name in needed}
    entry["packages"] = {
        name: pkg for name in needed if (pkg := package_for(entry["resolved"][name])) is not None
    }
    entry["dlopen_candidates"] = embedded_libraries(data, needed)
    entry["tools_missing"] = [
        tool for tool, out in (("objdump -p", objdump_p), ("objdump -T", objdump_t), ("ldd", ldd)) if out is None
    ]
    return entry


def read_os_release(path: Path = Path("/etc/os-release")) -> dict[str, str]:
    values: dict[str, str] = {}
    try:
        text = path.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return values
    for line in text.splitlines():
        key, sep, value = line.partition("=")
        if sep and key.isidentifier():
            values[key] = value.strip().strip('"')
    return values


def host_info() -> dict:
    os_release = read_os_release()
    like = os_release.get("ID_LIKE", "").split()
    ubuntu = os_release.get("ID") == "ubuntu" or "ubuntu" in like
    return {
        "pretty_name": os_release.get("PRETTY_NAME", "unknown"),
        "version_id": os_release.get("VERSION_ID", "unknown"),
        "machine": platform.machine(),
        "kernel": platform.release(),
        "ubuntu_2604": ubuntu and os_release.get("VERSION_ID") == "26.04" and platform.machine() == "x86_64",
    }


def summarize(binaries: list[dict]) -> dict:
    symbols: dict[str, str] = {}
    packages: dict[str, str] = {}
    dlopen: set[str] = set()
    unresolved: set[str] = set()
    for entry in binaries:
        for namespace, version in entry.get("symbol_versions", {}).items():
            if namespace not in symbols or _version_key(version) > _version_key(symbols[namespace]):
                symbols[namespace] = version
        for pkg in entry.get("packages", {}).values():
            packages[pkg["package"]] = pkg["version"]
        dlopen.update(entry.get("dlopen_candidates", []))
        unresolved.update(n for n, p in entry.get("resolved", {}).items() if p in ("unresolved", "not found"))
    return {
        "min_symbol_versions": dict(sorted(symbols.items())),
        "packages": dict(sorted(packages.items())),
        "dlopen_candidates": sorted(dlopen),
        "unresolved": sorted(unresolved),
    }


def to_markdown(report: dict) -> str:
    host = report["host"]
    lines = [
        "# CFA4 runtime dependencies (measured)",
        "",
        f"- Host: {host['pretty_name']} ({host['machine']}, kernel {host['kernel']})",
        f"- Valid F9 evidence (Ubuntu 26.04 AMD64): {'YES' if host['ubuntu_2604'] else 'NO'}",
        f"- Measured: {report['measured_at']}",
        "",
        "## Minimum symbol versions",
        "",
        "| Namespace | Minimum |",
        "|---|---|",
    ]
    summary = report["summary"]
    lines += [f"| {ns} | {v} |" for ns, v in summary["min_symbol_versions"].items()]
    lines += ["", "## Packages owning the linked libraries", "", "| Package | Installed version |", "|---|---|"]
    lines += [f"| {p} | {v} |" for p, v in summary["packages"].items()]
    lines += ["", "## Loaded at run time (dlopen candidates, not seen by ldd)", ""]
    lines += [f"- `{name}`" for name in summary["dlopen_candidates"]] or ["- none"]
    if summary["unresolved"]:
        lines += ["", "## Unresolved", ""] + [f"- `{name}`" for name in summary["unresolved"]]
    for entry in report["binaries"]:
        lines += ["", f"## `{entry['path']}`", ""]
        if "error" in entry:
            lines.append(f"Error: {entry['error']}")
            continue
        lines.append(f"NEEDED: {', '.join(f'`{n}`' for n in entry['needed']) or 'none'}")
        if entry["tools_missing"]:
            lines.append(f"Tools missing: {', '.join(entry['tools_missing'])}")
    return "\n".join(lines) + "\n"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Measure the runtime dependencies of CFA4 binaries.")
    parser.add_argument("binaries", nargs="+", type=Path)
    parser.add_argument("-o", "--output-dir", type=Path, required=True)
    args = parser.parse_args(argv)
    binaries = [audit_binary(path) for path in args.binaries]
    report = {
        "schema": SCHEMA,
        "measured_at": datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "host": host_info(),
        "binaries": binaries,
        "summary": summarize(binaries),
    }
    try:
        args.output_dir.mkdir(parents=True, exist_ok=True)
        (args.output_dir / "runtime-deps.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        (args.output_dir / "runtime-deps.md").write_text(to_markdown(report), encoding="utf-8")
    except OSError as exc:
        print(f"runtime deps: cannot write {args.output_dir}: {exc}", file=sys.stderr)
        return 2
    print(f"Wrote {args.output_dir / 'runtime-deps.json'} and runtime-deps.md")
    if any("error" in b for b in binaries):
        print("runtime deps: some binaries could not be audited (see report)", file=sys.stderr)
        return 1
    if not report["host"]["ubuntu_2604"]:
        print("runtime deps: host is not Ubuntu 26.04 AMD64; the report is NOT F9 evidence", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
