#!/usr/bin/env python3
"""Exercise the composed max-flow query grammar, independently of Rust execution."""
from __future__ import annotations

import copy
import json
from pathlib import Path

from jsonschema import Draft202012Validator


def main() -> None:
    root = Path(__file__).resolve().parents[1]
    schema = json.loads((root / "schemas/mcp_query_v1.json").read_text())
    extension = json.loads((root / "schemas/mcp_max_flow_v1.json").read_text())
    schema["$defs"]["query"]["oneOf"].append(extension)
    Draft202012Validator.check_schema(schema)
    validate = Draft202012Validator(schema)
    query = {
        "kind": "max_flow",
        "source": {"entity_id": "1", "generation": 1},
        "sink": {"entity_id": "4", "generation": 1},
        "edge_kinds": ["uses"],
        "capacity_field": "capacity",
        "unit_field": "capacity_unit",
    }
    accepted = [query]
    for field in ("source", "sink"):
        for generation in (0, 2**32 - 1):
            item = copy.deepcopy(query)
            item[field] = {"entity_id": str(2**64 - 1), "generation": generation}
            accepted.append(item)
    accepted.extend(dict(query, edge_kinds=kinds) for kinds in (
        ["uses", "supports"], ["custom:hauling_link"], ["uses"] * 32))
    rejected = []
    for field in query:
        item = copy.deepcopy(query)
        del item[field]
        rejected.append(item)
    for field, values in {
        "kind": ["flow", None],
        "edge_kinds": [[], ["unknown"], ["custom:"], ["uses"] * 33, None, "uses"],
        "capacity_field": ["", " ", "\n", "x\n", "x\0", "x\u0085", "x" * 129, None, 1],
        "unit_field": ["", "\t", "unit\r", "unit\0", "unit\u009f", "x" * 129, None, 1],
        "direction": ["incoming"],
        "capacity": [100],
        "unit": ["items_per_tick"],
        "limit": [1],
        "continuation": [None],
    }.items():
        rejected.extend(dict(query, **{field: value}) for value in values)
    for field in ("source", "sink"):
        for handle in (
            None, "1", {}, {"entity_id": "1"}, {"generation": 1},
            {"entity_id": "01", "generation": 1},
            {"entity_id": "0", "generation": 1},
            {"entity_id": "1\n", "generation": 1},
            {"entity_id": "1", "generation": -1},
            {"entity_id": "1", "generation": 2**32},
            {"entity_id": "1", "generation": True},
            {"entity_id": "1", "generation": 1, "revision": 2},
        ):
            rejected.append(dict(query, **{field: handle}))
    for item in accepted:
        envelope = {"schema": "dfmcp.query/1", "query": item}
        assert validate.is_valid(envelope), ("valid query refused", envelope)
    for item in rejected:
        envelope = {"schema": "dfmcp.query/1", "query": item}
        assert not validate.is_valid(envelope), ("invalid query accepted", envelope)
    for legacy in ({"kind": "entities"}, {"kind": "dependencies", "edge_kinds": ["requires"]}):
        assert validate.is_valid({"schema": "dfmcp.query/1", "query": legacy})
    print(json.dumps({"status": "passed", "accepted": len(accepted),
                      "rejected": len(rejected), "legacy_cases": 2,
                      "scope": "composed JSON Schema only"}, sort_keys=True))


if __name__ == "__main__":
    main()
