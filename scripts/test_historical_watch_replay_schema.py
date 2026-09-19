#!/usr/bin/env python3
"""Check the historical-monitor request envelope, not the Rust monitor.

Conditions are deliberately opaque in --envelope-only mode. This does not
validate the full composed watch dialect, archive identities or game semantics.
The range-length checks below are an independent reference, not Rust execution.
"""
import argparse
import copy
import hashlib
import json
from pathlib import Path

from jsonschema import Draft202012Validator


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--envelope-only", action="store_true", required=True)
    parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    path = root / "schemas/mcp_historical_watch_replay_v1.json"
    original = json.loads(path.read_text())
    Draft202012Validator.check_schema(original)
    assert original["properties"]["definition"]["properties"]["condition"] == {
        "$ref": "#/$defs/watch_condition"}
    schema = copy.deepcopy(original)
    schema["$defs"] = {"watch_condition": {"type": "object"}}
    validator = Draft202012Validator(schema)
    request = {"kind": "historical_watch_replay",
               "from": {"record": 1, "record_digest": "a" * 64},
               "to": {"record": 4, "record_digest": "b" * 64},
               "definition": {"condition": {"op": "paused", "value": True},
                              "deadline_tick": 100}}
    cases = [(copy.deepcopy(request), True)]

    def change(keys, value, expected):
        altered = copy.deepcopy(request)
        parent = altered
        for key in keys[:-1]:
            parent = parent[key]
        parent[keys[-1]] = value
        cases.append((altered, expected))

    for detail in (None, "summary", "evidence"):
        change(["detail"], detail, True)
    for key in ("kind", "from", "to", "definition"):
        altered = copy.deepcopy(request)
        del altered[key]
        cases.append((altered, False))
    for key in ("condition", "deadline_tick"):
        altered = copy.deepcopy(request)
        del altered["definition"][key]
        cases.append((altered, False))
    for value in (None, "monitor", 1, True):
        change(["kind"], value, False)
    for value in ("all", 0, [], True):
        change(["detail"], value, False)
    for key in ("watch", "continuation", "limit", "path", "query", "endpoint"):
        change([key], "unregistered", False)
    for key in ("key", "label", "watch", "unknown"):
        change(["definition", key], "not accepted", False)
    for side in ("from", "to"):
        change([side, "record"], 4096, True)
        for value in (0, -1, 4097, 1.5, "1", True, None):
            change([side, "record"], value, False)
        for value in ("a" * 63, "a" * 65, "A" * 64, "g" * 64, "a" * 63 + "\n",
                      "a" * 64 + "\n", "a" * 63 + "\x00", 0, None):
            change([side, "record_digest"], value, False)
        for missing in ("record", "record_digest"):
            altered = copy.deepcopy(request)
            del altered[side][missing]
            cases.append((altered, False))
        change([side, "unexpected"], True, False)
        change([side], None, False)
    for key, maximum in (("stable_observations", 64), ("poll_interval_ticks", 1_000_000)):
        for value in (None, 1, maximum):
            change(["definition", key], value, True)
        for value in (0, -1, maximum + 1, True, "1", 1.5):
            change(["definition", key], value, False)
    for value in (0, 2**64 - 1):
        change(["definition", "deadline_tick"], value, True)
    for value in (None, -1, 2**64, True, "10", 1.5):
        change(["definition", "deadline_tick"], value, False)
    for value in (None, {"op": "paused", "value": False}):
        change(["definition", "failure_condition"], value, True)
    for value in (True, [], 3):
        change(["definition", "condition"], value, False)
        change(["definition", "failure_condition"], value, False)
    for value, expected in cases:
        if validator.is_valid(value) != expected:
            raise AssertionError(f"expected valid={expected}: {value!r}")

    ranges = [(1, 1, True), (1, 32, True), (4065, 4096, True), (4096, 4096, True),
              (1, 33, False), (2, 1, False), (0, 1, False), (4096, 4097, False)]
    for first, last, expected in ranges:
        valid = 1 <= first <= last <= 4096 and last - first < 32
        assert valid == expected
    print(json.dumps({"schema_sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
        "envelope_cases": len(cases), "accepted": sum(expected for _, expected in cases),
        "rejected": sum(not expected for _, expected in cases), "range_reference_cases": len(ranges),
        "evidence": "Opaque-condition envelope and independent range reference only; not composed schema, Rust, MCP, journal replay or game execution"}, indent=2))


if __name__ == "__main__":
    main()
