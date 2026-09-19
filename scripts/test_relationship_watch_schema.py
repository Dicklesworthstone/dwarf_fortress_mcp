#!/usr/bin/env python3
"""Validate the additive relationship predicate component, not Rust execution."""
import copy
import hashlib
import json
from pathlib import Path

from jsonschema import Draft202012Validator


def main() -> None:
    root = Path(__file__).resolve().parents[1]
    path = root / "schemas/mcp_watch_count_v1.json"
    schema = json.loads(path.read_text())
    Draft202012Validator.check_schema(schema)
    variant = next(v for v in schema["$defs"]["watch_count_predicate"]["oneOf"]
                   if v.get("properties", {}).get("op", {}).get("const") == "related")
    validator = Draft202012Validator(variant)
    valid = {"op": "related", "entity_id": "10", "generation": 1,
             "relation": "contained_in", "direction": "incoming"}
    cases = []
    for relation in ("contained_in", "uses", "performs"):
        for direction in ("incoming", "outgoing"):
            cases.append((dict(valid, relation=relation, direction=direction), True))
    cases.append((dict(valid, entity_id="18446744073709551615", generation=4294967295), True))
    for key in valid:
        missing = copy.deepcopy(valid)
        del missing[key]
        cases.append((missing, False))
    bad_values = {
        "op": ("relation", None),
        "entity_id": ("", "0", "01", "-1", "+1", " 1", "1 ", "1\n", "1\r", "1\x00",
                      "1.0", "١", "1" * 21, 10, None),
        "generation": (0, -1, 4294967296, 1.5, "1", True, None),
        "relation": ("arbitrary", "ContainedIn", "uses\n", None),
        "direction": ("both", "Incoming", None),
        "unexpected": (True,),
    }
    for key, values in bad_values.items():
        for value in values:
            cases.append((dict(valid, **{key: value}), False))
    for request, expected in cases:
        actual = validator.is_valid(request)
        if actual != expected:
            raise AssertionError(f"expected valid={expected}: {request!r}")
    print(json.dumps({"cases": len(cases), "accepted": sum(wanted for _, wanted in cases),
        "rejected": sum(not wanted for _, wanted in cases),
        "schema_sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
        "evidence": "JSON Schema component only; not composed schema, Rust, MCP, or DFHack execution"}, indent=2))


if __name__ == "__main__":
    main()
