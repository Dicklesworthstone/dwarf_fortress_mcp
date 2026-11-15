#!/usr/bin/env python3
"""Validate the closed schedule request schema; this is not Rust execution."""
from __future__ import annotations

import copy
import json
from pathlib import Path

from jsonschema import Draft202012Validator


def main() -> None:
    root = Path(__file__).resolve().parents[1]
    extension = json.loads((root / "schemas/mcp_dependency_schedule_v1.json").read_text())
    base = json.loads((root / "schemas/mcp_query_v1.json").read_text())
    base["$defs"]["query"]["oneOf"].append(extension)
    Draft202012Validator.check_schema(base)
    validator = Draft202012Validator(base)
    query = {"kind": "schedule", "edge_kinds": ["requires"], "duration_field": "duration"}
    accepted = [query, dict(query, targets=[])]
    accepted.extend(dict(query, targets=[{"entity_id": "4", "generation": generation}])
                    for generation in (0, 1, 2**32 - 1))
    accepted.extend(dict(query, edge_kinds=kinds) for kinds in
                    (["uses", "requires"], ["custom:prerequisite"], ["requires"] * 32))
    accepted.append(dict(query, targets=[{"entity_id": str(i + 1), "generation": 1}
                                        for i in range(64)]))
    rejected = []
    for key in query:
        item = copy.deepcopy(query)
        del item[key]
        rejected.append(item)
    for key, values in {
        "kind": ["schedules", None],
        "edge_kinds": [[], ["unknown"], ["custom:"], ["requires"] * 33, None, "requires"],
        "duration_field": ["", " ", "x" * 129, "x\0y", "x\ny", "x\u0085y", None, 1],
        "targets": [None, {}, [None], [{"entity_id": "1"}],
                    [{"generation": 1}], [{"entity_id": "01", "generation": 1}],
                    [{"entity_id": "0", "generation": 1}],
                    [{"entity_id": "1", "generation": -1}],
                    [{"entity_id": "1", "generation": 2**32}],
                    [{"entity_id": "1", "generation": True}],
                    [{"entity_id": "1", "generation": 1, "extra": True}],
                    [{"entity_id": "1", "generation": 1}] * 2,
                    [{"entity_id": str(i + 1), "generation": 1} for i in range(65)]],
        "capacity": [1],
    }.items():
        rejected.extend(dict(query, **{key: value}) for value in values)
    for item in accepted:
        envelope = {"schema": "dfmcp.query/1", "query": item}
        assert validator.is_valid(envelope), ("valid request refused", envelope)
    for item in rejected:
        envelope = {"schema": "dfmcp.query/1", "query": item}
        assert not validator.is_valid(envelope), ("invalid request accepted", envelope)
    for item in ({"kind": "dependencies", "edge_kinds": ["requires"]},
                 {"kind": "entities"}):
        assert validator.is_valid({"schema": "dfmcp.query/1", "query": item})
    print(json.dumps({"status": "passed", "accepted": len(accepted),
                      "rejected": len(rejected), "legacy_cases": 2,
                      "scope": "JSON Schema only"}, sort_keys=True))


if __name__ == "__main__":
    main()
