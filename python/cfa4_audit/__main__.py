"""``cfa4-audit``: list, verify, export and checkpoint the audit trail from the command line.

Connection: ``--conninfo`` (a libpq connection string), else ``CFA4_DB_CONNINFO``, else the libpq defaults
(``PGHOST`` and friends). Output is one JSON object; exit status 0 = ok, 1 = a verification finding, 2 = an error.
Never a traceback for a database or audit problem.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from collections.abc import Sequence

import psycopg

from .commands import COMMANDS, run_command

_SHORT = {"list": "audit.list", "verify": "audit.verify", "export": "audit.export", "checkpoint": "audit.checkpoint"}


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="cfa4-audit", description=__doc__.splitlines()[0] if __doc__ else None)
    parser.add_argument("--conninfo", default=os.environ.get("CFA4_DB_CONNINFO", ""), help="libpq connection string")
    sub = parser.add_subparsers(dest="verb", required=True)
    for verb, cid in _SHORT.items():
        p = sub.add_parser(verb, help=COMMANDS[cid].label)
        for param in COMMANDS[cid].params:
            flag = "--" + param.name.replace("_", "-")
            if param.kind == "bool":
                p.add_argument(flag, action="store_true", help=param.help)
            else:
                p.add_argument(flag, default=None, required=param.required, help=param.help)
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    cid = _SHORT[args.verb]
    params = {
        p.name: getattr(args, p.name)
        for p in COMMANDS[cid].params
        if getattr(args, p.name, None) not in (None, False)
    }
    try:
        with psycopg.connect(args.conninfo, autocommit=True) as conn:
            result = run_command(conn, cid, params)
    except psycopg.Error as exc:
        result = {"ok": False, "code": "DB_UNAVAILABLE", "detail": str(exc).splitlines()[0] if str(exc) else ""}
        print(json.dumps(result, ensure_ascii=False))
        return 2
    print(json.dumps(result, ensure_ascii=False, default=str))
    if result.get("ok"):
        return 0
    return 1 if result.get("code") in ("AUDIT_CHAIN_BROKEN", "AUDIT_CHECKPOINT_MISMATCH", "AUDIT_SCHEMA_VERSION_UNKNOWN") else 2


if __name__ == "__main__":
    sys.exit(main())
