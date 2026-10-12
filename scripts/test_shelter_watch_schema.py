#!/usr/bin/env python3
"""Structural shelter condition and whole-request bounds, not Rust execution.

Owning bead: dfmcp-wp-pln03-labor-civilian-alert-fsm-zdm.
Run from a checkout with python-jsonschema installed.
"""
from __future__ import annotations
import copy
import json
from pathlib import Path
from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parent.parent


def input_shape(value: object) -> tuple[int, int]:
    """Independent accounting for the existing watch request's fixed limits."""
    pending, count, size = [(value, 0)], 0, 0
    while pending:
        item, depth = pending.pop()
        count += 1
        size += 32
        if depth > 24 or count > 1024:
            raise ValueError("watch shape limit")
        if isinstance(item, str):
            size += len(item.encode())
        elif isinstance(item, dict):
            size += sum(len(key.encode()) for key in item)
            pending.extend((child, depth + 1) for child in item.values())
        elif isinstance(item, list):
            pending.extend((child, depth + 1) for child in item)
        if count + len(pending) > 1024 or size > 32768:
            raise ValueError("watch shape/byte limit")
    return count, size


def main() -> None:
    schema = json.loads((ROOT / "schemas/mcp_watch_shelter_set_v1.json").read_text())
    Draft202012Validator.check_schema(schema)
    validator = Draft202012Validator(schema)
    envelope = json.loads((ROOT / "schemas/examples/shelter_watch_v1.json").read_text())
    original = envelope["query"]["condition"]
    validator.validate(original)
    input_shape(envelope)
    invalid = []
    for field in original:
        case = copy.deepcopy(original)
        del case[field]
        invalid.append(case)
    for field in ("source_policy", "authority", "release_civilians", "positions"):
        case = copy.deepcopy(original)
        case[field] = True
        invalid.append(case)
    for value in ([], None, "all", [original["citizens"][0]] * 129):
        case = copy.deepcopy(original)
        case["citizens"] = value
        invalid.append(case)
    for field, values in {
        "entity_id": ["", "0", "01", "+1", "1\n", "a", 10],
        "generation": [0, -1, 4294967296, 1.5, "1", None, True],
    }.items():
        for value in values:
            case = copy.deepcopy(original)
            case["citizens"][0][field] = value
            invalid.append(case)
    for field in ("entity_id", "generation"):
        case = copy.deepcopy(original)
        del case["citizens"][0][field]
        invalid.append(case)
    for value in ([], None, [original["shelters"][0]] * 17):
        case = copy.deepcopy(original)
        case["shelters"] = value
        invalid.append(case)
    for end in ("min", "max"):
        for value in ([], [0, 0], [0, 0, 0, 0], [-1, 0, 5], [32768, 0, 5], [0, 1.5, 5], [True, 0, 5]):
            case = copy.deepcopy(original)
            case["shelters"][0][end] = value
            invalid.append(case)
    for case in invalid:
        if validator.is_valid(case):
            raise AssertionError(f"malformed condition accepted: {case!r}")
    maximum = copy.deepcopy(envelope)
    selection = maximum["query"]["condition"]
    selection["citizens"] = [{"entity_id": str((3 << 40) + n), "generation": 1} for n in range(128)]
    selection["shelters"] = [{"min": [x, 0, 5], "max": [x, 0, 5]} for x in range(16)]
    validator.validate(selection)
    nodes, accounted_bytes = input_shape(maximum)
    print(json.dumps({"schema": "shelter-watch-structural-check/1", "metaschema": "passed",
        "examples": 1, "malformed_conditions_rejected": len(invalid),
        "maximum_targets": 128, "maximum_regions": 16,
        "maximum_request_nodes": nodes, "maximum_accounted_request_bytes": accounted_bytes,
        "rust_executed": False, "native_game_executed": False}, sort_keys=True))


if __name__ == "__main__":
    main()
