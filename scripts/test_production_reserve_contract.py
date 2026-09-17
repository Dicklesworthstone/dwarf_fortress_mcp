#!/usr/bin/env python3
"""Exact request-schema checks only: no Rust, solver, MCP or native execution."""
from __future__ import annotations
import argparse
import copy
import hashlib
import json
from pathlib import Path
from jsonschema import Draft202012Validator


def blob(data: bytes) -> str:
    return hashlib.sha1(f"blob {len(data)}\0".encode() + data).hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--schema", type=Path, default=Path(__file__).resolve().parents[1] / "schemas/mcp_production_portfolio_v1.json")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    raw = args.schema.read_bytes()
    schema = json.loads(raw)
    Draft202012Validator.check_schema(schema)
    reserve_schema = schema["properties"]["reserves"]["items"]
    material_schema = schema["properties"]["tasks"]["items"]["properties"]["materials"]["items"]
    if reserve_schema != material_schema:
        raise AssertionError("reserve selectors diverged from task material selectors")
    validator = Draft202012Validator(schema)
    composed = Draft202012Validator({"type": "object", "additionalProperties": False,
        "required": ["schema", "query"], "properties": {"schema": {"const": "dfmcp.query/1"}, "query": schema}})
    pool = {"key": "buffer", "units": 2, "item_types": ["WOOD"]}
    base = {"kind": "production_portfolio", "origin": [0, 0, 5], "quantity_unit": "stack_units",
        "tasks": [{"key": "chairs", "workers": 1, "skill_key": "CARPENTRY", "materials": [copy.deepcopy(pool)]}]}
    cases: list[tuple[str, dict, bool]] = [("legacy omitted", copy.deepcopy(base), True)]

    def add(name: str, reserves: object, expected: bool) -> None:
        query = copy.deepcopy(base)
        query["reserves"] = copy.deepcopy(reserves)
        cases.append((name, query, expected))

    add("null", None, True)
    add("empty", [], True)
    add("one pool", [pool], True)
    add("eight pools", [{**pool, "key": f"r{i}"} for i in range(8)], True)
    add("largest exact quantity", [{**pool, "units": 2**64-1}], True)
    add("beyond JS exact integer", [{**pool, "units": 2**53+1}], True)
    add("long key and type", [{**pool, "key": "k"*48, "item_types": ["W"*128]*8}], True)
    for material in [None, -1, 0, 2**31-1]:
        add(f"material pair {material}", [{**pool, "material_type": material, "material_index": material}], True)
    add("nullable selectors", [{**pool, "subtype": None, "material_type": None, "material_index": None}], True)
    add("material type alone", [{**pool, "material_type": 0}], True)
    for value in [{}, "buffer", False, 1, -1]:
        add(f"wrong pool collection {value!r}", value, False)
    add("nine raw pools", [pool]*9, False)
    add("nonobject pool", [False], False)
    for field, values in {
        "key": ["", "k"*49, "bad key", "bad\0key", None, 7],
        "units": [0, -1, 1.5, "1", True, None, 2**64],
        "item_types": [[], ["WOOD"]*9, [""], ["W"*129], ["bad\0type"], "WOOD", [True]],
        "subtype": [-2, 2**31, 1.5, "1", True],
        "material_type": [-2, 2**31, 1.5, "1", True],
        "material_index": [-2, 2**31, 1.5, "1", True, 0],
    }.items():
        for value in values:
            add(f"invalid {field} {value!r}", [{**pool, field: value}], False)
    for field in ["key", "units", "item_types"]:
        broken = copy.deepcopy(pool)
        del broken[field]
        add(f"missing {field}", [broken], False)
    for field in ["priority", "share_units", "ignore_shortfall", "path"]:
        add(f"unrecognized reserve {field}", [{**pool, field: 1}], False)
    add("index with null type", [{**pool, "material_type": None, "material_index": 0}], False)
    for field, value in [("quantity_unit", "item_records"), ("limit", 0), ("max_work", 0)]:
        query = copy.deepcopy(base)
        query["reserves"] = [copy.deepcopy(pool)]
        query[field] = value
        cases.append((f"invalid outer {field}", query, False))
    accepted = rejected = 0
    for name, value, expected in cases:
        observed = validator.is_valid(value)
        wrapped = composed.is_valid({"schema": "dfmcp.query/1", "query": value})
        if observed != expected or wrapped != expected:
            raise AssertionError(f"{name}: expected {expected}, direct={observed}, composed={wrapped}")
        accepted += int(expected)
        rejected += int(not expected)
    script = Path(__file__).read_bytes()
    report = {"status": "passed", "cases": len(cases), "accepted": accepted, "rejected": rejected,
        "direct_and_composed_checks": 2*len(cases), "selector_schemas_equal": True,
        "schema_git_blob": blob(raw), "schema_sha256": hashlib.sha256(raw).hexdigest(),
        "script_git_blob": blob(script), "script_sha256": hashlib.sha256(script).hexdigest(),
        "scope": "request_structure_only_not_rust_solver_or_mcp_execution",
        "runtime_checks_not_exercised": ["unique reserve keys", "32 combined material demands", "UTF-8 byte bounds",
            "authority/custody", "capacity feasibility", "task optimization", "pagination", "archive replay"]}
    encoded = json.dumps(report, indent=2) + "\n"
    if args.output:
        args.output.write_text(encoded)
    print(encoded, end="")


if __name__ == "__main__":
    main()
