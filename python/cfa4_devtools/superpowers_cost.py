# SPDX-License-Identifier: GPL-3.0-or-later
"""Offline Superpowers cost preflight. No model/API execution or automatic hooks.

ALLOW_ESTIMATE_ONLY is not authorization. A real provider-side atomic budget
reservation and bounded model output are required before production enablement.
"""
from __future__ import annotations

import argparse
import json
import sys
import tomllib
from decimal import Decimal, InvalidOperation
from pathlib import Path
from typing import Any

DEFAULT_POLICY = Path(__file__).resolve().parents[2] / "config/dev/superpowers-allowlist.toml"
TOKEN_LIMIT = 1_000_000_000
MONEY_LIMIT = Decimal(1000000000)


class CostGateError(ValueError):
    """Unverifiable cost, unsupported mode, or missing approval."""


def _map(value: Any) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise CostGateError("expected JSON/TOML mapping")
    return value


def _token(value: Any) -> int:
    if type(value) is not int or not 0 <= value <= TOKEN_LIMIT:
        raise CostGateError("invalid token count")
    return value


def _usd(value: Any) -> Decimal:
    if isinstance(value, bool) or not isinstance(value, (str, int, Decimal)):
        raise CostGateError("invalid USD value")
    try:
        amount = Decimal(str(value))
    except (InvalidOperation, ValueError) as exc:
        raise CostGateError("invalid USD value") from exc
    if not amount.is_finite() or not 0 <= amount <= MONEY_LIMIT:
        raise CostGateError("nonfinite or unbounded USD")
    return amount


def evaluate(policy: dict[str, Any], request: dict[str, Any], usage: dict[str, Any]) -> dict[str, str]:
    """Read-only preflight against comparable task baselines.

    The usage snapshot must originate from an independent trusted collector,
    not the requesting agent. No budget is reserved by this function.
    """
    try:
        policy, request, usage = _map(policy), _map(request), _map(usage)
        if type(policy.get("schema")) is not int or policy["schema"] != 1:
            raise CostGateError("unknown policy schema")
        cfg, caps = _map(policy["superpowers"]), _map(policy["limits"])
        if cfg.get("enabled") is not True:
            raise CostGateError("disabled")
        if any(cfg.get(key) is not False for key in
               ("automatic_bootstrap", "parallel_agents", "telemetry_enabled")):
            raise CostGateError("unsafe activation")
        if cfg.get("execution_mode") != "inline" or request.get("mode") != "inline":
            raise CostGateError("only inline")
        skills = cfg.get("allowed_skills")
        if not isinstance(skills, list) or any(type(skill) is not str for skill in skills):
            raise CostGateError("invalid allowlist")
        if request.get("skill") not in skills:
            raise CostGateError("disallowed skill")
        if request.get("approved") is not True:
            raise CostGateError("missing explicit approval")
        if type(request.get("paid")) is not bool:
            raise CostGateError("paid must be boolean")
        if _token(caps["max_parallel_paid_calls"]) != 0:
            raise CostGateError("parallel calls not supported")
        if request["paid"] and caps.get("allow_paid_calls") is not True:
            raise CostGateError("paid execution disabled")
        prompt = _token(request["input_tokens"])
        completion = _token(request["max_output_tokens"])
        input_rate = _usd(request["input_usd_per_million"])
        output_rate = _usd(request["output_usd_per_million"])
        if request["paid"] and input_rate == 0 and output_rate == 0:
            raise CostGateError("unknown paid rate")
        if not request["paid"] and (input_rate != 0 or output_rate != 0):
            raise CostGateError("unpaid rate contradiction")
        spend_tokens = _token(usage["spent_tokens"])
        baseline_tokens = _token(usage["baseline_tokens"])
        policy_tokens = _token(caps["max_total_tokens"])
        spend_usd = _usd(usage["spent_paid_usd"])
        baseline_usd = _usd(usage["baseline_paid_usd"])
        policy_usd = _usd(caps["max_paid_usd"])
        forecast = (Decimal(prompt) * input_rate + Decimal(completion) * output_rate) / Decimal(1_000_000)
        if spend_tokens + prompt + completion > min(policy_tokens, baseline_tokens):
            raise CostGateError("token limit")
        if spend_usd + forecast > min(policy_usd, baseline_usd):
            raise CostGateError("USD limit")
        return {
            "decision": "ALLOW_ESTIMATE_ONLY",
            "estimated_max_paid_usd": format(forecast, "f"),
            "estimated_tokens": str(prompt + completion),
            "notice": "No authorization or runtime cap; provider-side enforcement required",
        }
    except (CostGateError, KeyError, TypeError, ArithmeticError):
        return {"decision": "DENY", "reason": "disabled, unauthorized, incomplete or over budget"}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Offline CFA4 cost estimate (no model calls)")
    parser.add_argument("--policy", type=Path, default=DEFAULT_POLICY)
    parser.add_argument("--request", type=Path)
    parser.add_argument("--usage", type=Path)
    args = parser.parse_args(argv)
    try:
        with args.policy.open("rb") as stream:
            policy = tomllib.load(stream)
        if args.request is None or args.usage is None:
            result = {"decision": "DENY", "reason": "request and usage snapshot required"}
        else:
            with args.request.open("r", encoding="utf-8") as request_stream:
                request = json.load(request_stream, parse_float=Decimal)
            with args.usage.open("r", encoding="utf-8") as usage_stream:
                usage = json.load(usage_stream, parse_float=Decimal)
            result = evaluate(policy, request, usage)
    except (OSError, ValueError, TypeError, tomllib.TOMLDecodeError):
        result = {"decision": "DENY", "reason": "unavailable or invalid evidence"}
    print(json.dumps(result, sort_keys=True))
    return 0 if result["decision"] == "ALLOW_ESTIMATE_ONLY" else 2


if __name__ == "__main__":
    sys.exit(main())
