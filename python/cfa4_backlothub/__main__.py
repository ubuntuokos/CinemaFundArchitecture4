"""``cfa4-backlothub``: migrate the schema, list the commands, run one command from the command line.

Connection: ``--conninfo``, else ``CFA4_DB_CONNINFO``, else the libpq defaults. Output is one JSON object; exit status
0 = ok, 1 = the command was refused (``code`` says why), 2 = the database could not be used. Never a traceback.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from collections.abc import Sequence

from sqlalchemy.exc import SQLAlchemyError

from . import migrate
from .commands import list_commands, run_command
from .db.engine import make_engine


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="cfa4-backlothub", description=(__doc__ or "").splitlines()[0])
    parser.add_argument("--conninfo", default=os.environ.get("CFA4_DB_CONNINFO", ""), help="libpq connection string")
    sub = parser.add_subparsers(dest="verb", required=True)
    mig = sub.add_parser("migrate", help="apply the schema migrations (or revert them with --down)")
    mig.add_argument("--down", action="store_true", help="revert everything (an empty schema; the data is lost)")
    sub.add_parser("commands", help="list the command descriptors (what commands.list publishes)")
    run = sub.add_parser("run", help="run one command")
    run.add_argument("command_id")
    run.add_argument("--params", default="{}", help="the parameters as a JSON object")
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    if args.verb == "commands":
        print(json.dumps({"ok": True, "commands": list_commands()}, ensure_ascii=False))
        return 0
    engine = make_engine(args.conninfo)
    try:
        if args.verb == "migrate":
            (migrate.downgrade if args.down else migrate.upgrade)(engine)
            print(json.dumps({"ok": True, "revision": migrate.current(engine)}))
            return 0
        try:
            params = json.loads(args.params)
            if not isinstance(params, dict):
                raise ValueError("not an object")
        except ValueError:
            print(json.dumps({"ok": False, "code": "INPUT_INVALID", "detail": "--params must be a JSON object"}))
            return 1
        result = run_command(engine, args.command_id, params)
        print(json.dumps(result, ensure_ascii=False, default=str))
        if result.get("ok"):
            return 0
        return 2 if result.get("code") in ("DB_UNAVAILABLE", "INTERNAL_ERROR") else 1
    except SQLAlchemyError as exc:
        print(json.dumps({"ok": False, "code": "DB_UNAVAILABLE", "detail": str(exc).splitlines()[0] if str(exc) else ""}))
        return 2
    finally:
        engine.dispose()


if __name__ == "__main__":
    sys.exit(main())
