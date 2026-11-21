#!/usr/bin/env python3
"""Validate min-cost-flow requests against the composed public query schema."""
from __future__ import annotations

import copy
import json
from pathlib import Path

from jsonschema import Draft202012Validator


def main() -> None:
    root = Path(__file__).resolve().parents[1]
    schema = json.loads((root / "schemas/mcp_query_v1.json").read_text())
    originals = copy.deepcopy(schema["$defs"]["query"]["oneOf"])
    for name in ("mcp_dominators_v1.json", "mcp_dependency_schedule_v1.json",
                 "mcp_weighted_routes_v1.json", "mcp_max_flow_v1.json",
                 "mcp_min_cost_flow_v1.json"):
        schema["$defs"]["query"]["oneOf"].append(
            json.loads((root / "schemas" / name).read_text()))
    assert schema["$defs"]["query"]["oneOf"][:len(originals)] == originals
    Draft202012Validator.check_schema(schema)
    validate = Draft202012Validator(schema)
    query = {
        "kind": "min_cost_flow",
        "source": {"entity_id": "1", "generation": 1},
        "sink": {"entity_id": "4", "generation": 1},
        "edge_kinds": ["uses"],
        "capacity_field": "capacity",
        "unit_field": "capacity_unit",
        "cost_field": "cost",
        "cost_unit_field": "cost_unit",
        "demand": 3,
    }
    accepted = [query]
    for value in (1, 2**53 + 1, 2**64 - 1):
        accepted.append(dict(query, demand=value))
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
        "demand": [0, -1, 1.5, 2**64, True, None, "3"],
        "direction": ["incoming"],
        "capacity": [100],
        "unit": ["items"],
        "cost": [0],
        "cost_unit": ["tile_steps"],
        "commit": [True],
        "limit": [1],
        "continuation": [None],
    }.items():
        rejected.extend(dict(query, **{field: value}) for value in values)
    for field in ("capacity_field", "unit_field", "cost_field", "cost_unit_field"):
        for value in ("", " ", "\n", "x\n", "x\0", "x\u0085", "x" * 129, None, 1):
            rejected.append(dict(query, **{field: value}))
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
    legacy = [
        {"kind": "entities"},
        {"kind": "dependencies", "edge_kinds": ["requires"]},
        {key: value for key, value in dict(query, kind="max_flow").items()
         if key not in {"cost_field", "cost_unit_field", "demand"}},
        {"kind": "weighted_routes", "edge_kinds": ["uses"], "cost_field": "cost",
         "roots": [{"entity_id": "1", "generation": 1}]},
        {"kind": "schedule", "edge_kinds": ["requires"], "duration_field": "duration",
         "capacity": 2},
    ]
    for item in legacy:
        assert validate.is_valid({"schema": "dfmcp.query/1", "query": item}), item
    print(json.dumps({"status": "passed", "accepted": len(accepted),
                      "rejected": len(rejected), "legacy_cases": len(legacy),
                      "scope": "composed JSON Schema only"}, sort_keys=True))


if __name__ == "__main__":
    main()
