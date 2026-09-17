#!/usr/bin/env python3
"""Request structure only; does not execute Rust, routes, allocations or MCP."""
from __future__ import annotations
import argparse
import copy
import hashlib
import json
from pathlib import Path
from jsonschema import Draft202012Validator


def blob(data: bytes) -> str:
    return hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest()


def run(schema_path: Path) -> dict:
    data = schema_path.read_bytes()
    schema = json.loads(data)
    Draft202012Validator.check_schema(schema)
    direct = Draft202012Validator(schema)
    wrapped = Draft202012Validator({"type": "object", "additionalProperties": False,
        "required": ["schema", "query"], "properties": {
            "schema": {"const": "dfmcp.query/1"}, "query": schema}})
    base = {"kind": "production_portfolio", "origin": [0, 0, 5],
        "quantity_unit": "stack_units", "tasks": [{"key": "west", "workers": 1,
        "skill_key": "CARPENTRY", "materials": [
            {"key": "wood", "units": 1, "item_types": ["WOOD"]}]}]}
    cases = [("legacy-default", base, True)]
    for value in [None, [0, 0, 5], [4, 0, 5], [32767, 32767, 32767], [0, 0, 0]]:
        query = copy.deepcopy(base)
        query["tasks"][0]["origin"] = value
        cases.append(("accepted-task-origin", query, True))
    for value in [False, True, 1, "4,0,5", {}, [], [4], [4, 0], [4, 0, 5, 6],
                  [-1, 0, 5], [32768, 0, 5], [1.5, 0, 5], [None, 0, 5], [False, 0, 5]]:
        query = copy.deepcopy(base)
        query["tasks"][0]["origin"] = value
        cases.append(("rejected-task-origin", query, False))
    for axis in range(3):
        for value in [-1, 32768, 2**32, "0", True, None, 0.5]:
            query = copy.deepcopy(base)
            query["tasks"][0]["origin"] = [0, 0, 5]
            query["tasks"][0]["origin"][axis] = value
            cases.append(("coordinate-bound", query, False))
    for count in [1, 2, 8, 9]:
        query = copy.deepcopy(base)
        query["tasks"] = [dict(copy.deepcopy(base["tasks"][0]), key=f"site-{n}",
            origin=[n, 0, 5]) for n in range(count)]
        cases.append(("site-cardinality", query, count <= 8))
    for pools in [None, [], [{"key": "buffer", "units": 1, "item_types": ["WOOD"]}]]:
        query = copy.deepcopy(base)
        query["tasks"][0]["origin"] = [4, 0, 5]
        query["reserves"] = pools
        cases.append(("reserve-preserved", query, True))
    for target in ["reserve", "input"]:
        query = copy.deepcopy(base)
        if target == "reserve":
            query["reserves"] = [dict(copy.deepcopy(base["tasks"][0]["materials"][0]), origin=[4, 0, 5])]
        else:
            query["tasks"][0]["materials"][0]["origin"] = [4, 0, 5]
        cases.append(("no-implicit-reserve-or-input-site", query, False))
    for field in ["workshop_id", "teleport", "skip_unreachable", "trust_routes"]:
        query = copy.deepcopy(base)
        query["tasks"][0][field] = True
        cases.append(("no-extra-task-authority", query, False))
    for value in [None, [], [0, 0], [-1, 0, 5], [32768, 0, 5]]:
        query = copy.deepcopy(base)
        query["origin"] = value
        query["tasks"][0]["origin"] = [4, 0, 5]
        cases.append(("default-origin-still-required-and-bounded", query, False))
    query = copy.deepcopy(base)
    del query["origin"]
    query["tasks"][0]["origin"] = [4, 0, 5]
    cases.append(("no-missing-default-origin", query, False))
    for name, query, expected in cases:
        actual = direct.is_valid(query)
        envelope = wrapped.is_valid({"schema": "dfmcp.query/1", "query": query})
        if actual != expected or envelope != expected:
            raise AssertionError(f"{name}: direct={actual}, wrapped={envelope}, expected={expected}")
    material = schema["properties"]["tasks"]["items"]["properties"]["materials"]["items"]
    assert material == schema["properties"]["reserves"]["items"]
    script = Path(__file__).read_bytes()
    return {"status": "passed", "scope": "request_schema_only_not_Rust_or_runtime_execution",
        "cases": len(cases), "accepted": sum(bool(c[2]) for c in cases),
        "rejected": sum(not c[2] for c in cases), "direct_and_envelope_checks": len(cases)*2,
        "task_material_and_reserve_schema_equal": True,
        "schema_git_blob": blob(data), "script_git_blob": blob(script),
        "script_sha256": hashlib.sha256(script).hexdigest(),
        "not_tested": ["Rust compilation", "site existence or reachability", "resource allocation",
            "aggregate work budgets", "MCP publication", "archive replay", "DFHack"]}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--schema", type=Path, default=Path(__file__).resolve().parents[1] / "schemas/mcp_production_portfolio_v1.json")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    report = run(args.schema)
    encoded = json.dumps(report, indent=2) + "\n"
    if args.output:
        args.output.write_text(encoded, encoding="utf-8")
    print(encoded, end="")


if __name__ == "__main__":
    main()
