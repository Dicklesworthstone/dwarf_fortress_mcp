#!/usr/bin/env python3
"""Execute the published query schema, not Rust parsing, MCP, or native I/O."""
from __future__ import annotations

import argparse
import copy
import hashlib
import json
from pathlib import Path

from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[1]
SCHEMA = ROOT / "schemas/dig_control_query.json"


def cases() -> tuple[list[dict], list[object]]:
    digest = "a" * 64
    examples = [
        {"mode": "records"},
        {"mode": "selection_tiles", "witness": digest, "offset": 0},
        {"mode": "plan_tiles", "idempotency_key": "dig-001", "plan_digest": digest, "offset": 0},
        {"mode": "schema"},
    ]
    valid = copy.deepcopy(examples)
    for limit in (1, 4, 8):
        valid.append({"mode": "records", "limit": limit, "continuation": digest})
    for example in examples[1:3]:
        for offset in (0, 1, 299):
            for limit in (1, 8, 16):
                valid.append({**example, "offset": offset, "limit": limit})
    for key in ("a", "x" * 128, "a._-Z09"):
        valid.append({**examples[2], "idempotency_key": key})

    invalid: list[object] = [None, True, [], "records", {}, {"mode": "commit"}]
    for example in examples:
        invalid.append({**example, "command": "not-accepted"})
        invalid.append({**example, "mode": True})
    for example, maximum in ((examples[0], 8), (examples[1], 16), (examples[2], 16)):
        for limit in (None, False, 0, -1, maximum + 1, 1.5, "1", [], {}):
            invalid.append({**example, "limit": limit})
    for example in examples[1:3]:
        for offset in (None, True, -1, 300, 1.5, "0", [], {}):
            invalid.append({**example, "offset": offset})
        for required in ("offset", "witness" if example["mode"] == "selection_tiles" else "plan_digest"):
            invalid.append({k: v for k, v in example.items() if k != required})
    for example, field in ((examples[0], "continuation"), (examples[1], "witness"), (examples[2], "plan_digest")):
        for value in (None, 1, [], "", "a" * 63, "a" * 65, "A" * 64, "g" * 64, digest + "\n", "\x00" * 64):
            invalid.append({**example, field: value})
    for key in (None, 1, "", "x" * 129, "with space", "a/b", "a\n", "a\r", "é", "\x00"):
        invalid.append({**examples[2], "idempotency_key": key})
    invalid.append({k: v for k, v in examples[2].items() if k != "idempotency_key"})
    return valid, invalid


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    schema = json.loads(SCHEMA.read_bytes())
    Draft202012Validator.check_schema(schema)
    validator = Draft202012Validator(schema)
    valid, invalid = cases()
    for index, value in enumerate(valid):
        errors = list(validator.iter_errors(value))
        if errors:
            raise AssertionError(f"valid case {index} rejected: {errors[0].message}")
    for index, value in enumerate(invalid):
        if validator.is_valid(value):
            raise AssertionError(f"invalid case {index} accepted: {value!r}")
    result = {
        "schema": "dfmcp.dig-control-query-schema-evidence/1",
        "status": "passed_json_schema_only",
        "accepted_cases": len(valid),
        "rejected_cases": len(invalid),
        "rust_compiled_or_executed": False,
        "mcp_runtime_or_native_io_executed": False,
        "limitations": "JSON Schema mathematical integer semantics do not certify Rust lexical numbers, duplicate fields, raw input bounds, routing or stateful identity.",
        "source_sha256": {
            str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in (SCHEMA, Path(__file__).resolve())
        },
    }
    raw = json.dumps(result, sort_keys=True, indent=2) + "\n"
    if args.report:
        args.report.write_text(raw)
    print(raw, end="")


if __name__ == "__main__":
    main()
