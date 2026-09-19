#!/usr/bin/env python3
"""Check the source-recovery envelope and its conditional schema composition.

This executes JSON Schema only, not Rust, the live dispatcher, storage or DFHack.
Canonical anchor equality, session authority/health and budget narrowing are
runtime constraints; the envelope alone cannot establish them.
"""
import copy
import hashlib
import json
from pathlib import Path

from jsonschema import Draft202012Validator


def main() -> None:
    root = Path(__file__).resolve().parents[1]
    path = root / "schemas/mcp_source_recovery_v1.json"
    schema = json.loads(path.read_text())
    Draft202012Validator.check_schema(schema)
    validator = Draft202012Validator(schema)
    anchor = {"fortress_id": "1", "epoch": 0, "sequence": 7,
              "game_tick": 123, "state_hash": "a" * 64}
    valid = {"schema": "dfmcp.query/1", "expected_anchor": anchor,
             "query": {"kind": "recover_source"}}
    cases = [(valid, True)]
    for millis in [1, 2, 1000, 5000, 60000]:
        request = copy.deepcopy(valid)
        request["query"]["max_wall_millis"] = millis
        cases.append((request, True))
    for field in valid:
        request = copy.deepcopy(valid)
        del request[field]
        cases.append((request, False))
    for selector in ["endpoint", "token", "protocol", "plugin", "method", "journal", "path", "repair"]:
        for nested in [False, True]:
            request = copy.deepcopy(valid)
            (request["query"] if nested else request)[selector] = "untrusted"
            cases.append((request, False))
    for millis in [0, -1, 60001, 2**64, 1.5, "100", True, None, [], {}]:
        request = copy.deepcopy(valid)
        request["query"]["max_wall_millis"] = millis
        cases.append((request, False))
    for field, values in {"schema": [None, "dfmcp.query/2", 1],
                          "expected_anchor": [None, [], "anchor", True],
                          "query": [None, [], "recover_source", {}]}.items():
        for value in values:
            request = copy.deepcopy(valid)
            request[field] = value
            cases.append((request, False))
    for kind in ["recover", "RecoverSource", "recover_source\n", None, 1]:
        request = copy.deepcopy(valid)
        request["query"]["kind"] = kind
        cases.append((request, False))
    for request, expected in cases:
        actual = validator.is_valid(request)
        if actual != expected:
            raise AssertionError(f"expected valid={expected}: {request!r}")

    # Independent reference for the additive conditional in extend_schema.
    # It checks that the new requirement does not break unrelated query kinds.
    conditional = {"if": {"required": ["query"], "properties": {"query": {
        "required": ["kind"], "properties": {"kind": {"const": "recover_source"}}}}},
        "then": {"required": ["expected_anchor"],
                 "properties": {"expected_anchor": {"type": "object"}}}}
    composed = Draft202012Validator({"allOf": [conditional]})
    compositions = [({}, True), ({"query": {}}, True),
                    ({"query": {"kind": "watches"}}, True),
                    ({"query": {"kind": "recover_source"}}, False),
                    ({"query": {"kind": "recover_source"}, "expected_anchor": None}, False),
                    (valid, True)]
    for value, expected in compositions:
        if composed.is_valid(value) != expected:
            raise AssertionError(f"conditional mismatch: {value!r}")
    print(json.dumps({"envelope_cases": len(cases),
        "accepted": sum(expected for _, expected in cases),
        "rejected": sum(not expected for _, expected in cases),
        "conditional_reference_cases": len(compositions),
        "schema_sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
        "evidence": "JSON Schema envelope and independent composition reference only; no Rust/MCP/storage/DFHack execution"}, indent=2))


if __name__ == "__main__":
    main()
