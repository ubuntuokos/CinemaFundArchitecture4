"""A throw-away PostgreSQL cluster for tests: temporary directory, Unix socket only, trust auth.

Without PostgreSQL binaries the tests skip (``CFA4_PG_BIN`` points at another ``bin`` directory);
``CFA4_PG_REQUIRE=1`` turns the skip into a failure. As root the server runs through ``runuser -u postgres``
because ``initdb`` refuses to run as root. Same conventions as the Rust ``cfa4-store`` tests.
"""

from __future__ import annotations

import glob
import os
import shutil
import subprocess
import tempfile
from collections.abc import Iterator
from contextlib import contextmanager
from pathlib import Path

import pytest


def _bin_dir() -> Path | None:
    env = os.environ.get("CFA4_PG_BIN")
    candidates = [env] if env else sorted(glob.glob("/usr/lib/postgresql/*/bin"), reverse=True)
    for c in candidates:
        if c and (Path(c) / "initdb").exists() and (Path(c) / "pg_ctl").exists():
            return Path(c)
    return None


def _run(argv: list[str], *, as_postgres: bool) -> None:
    if as_postgres and os.geteuid() == 0:
        argv = ["runuser", "-u", "postgres", "--", *argv]
    subprocess.run(argv, check=True, capture_output=True, text=True, timeout=120)


@contextmanager
def throwaway_cluster() -> Iterator[str]:
    """Yield a libpq connection string (``host=<socket dir> user=postgres``) to a fresh cluster."""
    bindir = _bin_dir()
    if bindir is None:
        msg = "PostgreSQL binaries not found (set CFA4_PG_BIN)"
        if os.environ.get("CFA4_PG_REQUIRE") == "1":
            pytest.fail(msg)
        pytest.skip(msg)
    root = Path(tempfile.mkdtemp(prefix="cfa4-pg-"))
    data, sock = root / "data", root / "sock"
    sock.mkdir()
    if os.geteuid() == 0:
        shutil.chown(root, "postgres", "postgres")
        shutil.chown(sock, "postgres", "postgres")
    try:
        _run([str(bindir / "initdb"), "-D", str(data), "-U", "postgres", "--auth=trust", "-E", "UTF8"], as_postgres=True)
        opts = f"-c listen_addresses='' -c unix_socket_directories={sock} -c fsync=off -c max_connections=30"
        _run([str(bindir / "pg_ctl"), "-D", str(data), "-o", opts, "-w", "-l", str(root / "log"), "start"], as_postgres=True)
        try:
            yield f"host={sock} user=postgres dbname=postgres"
        finally:
            _run([str(bindir / "pg_ctl"), "-D", str(data), "-m", "immediate", "stop"], as_postgres=True)
    finally:
        shutil.rmtree(root, ignore_errors=True)
