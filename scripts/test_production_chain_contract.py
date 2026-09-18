#!/usr/bin/env python3
"""Executable request-schema tests; not Rust, native or live qualification."""
import copy
import json
from pathlib import Path
from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[1]


def request():
    return {"kind": "production_chain", "quantity_unit": "stack_units",
            "resources": [{"key": "raw", "item_types": ["item_type_3"]},
                          {"key": "product", "item_types": ["item_type_999"]}],
            "quotas": [{"resource": "product", "minimum_stock": 4}],
            "recipes": [{"output": "product", "output_batch_size": 2,
                         "inputs": [{"resource": "raw", "units": 1}], "job_token": "DeclaredProduct",
                         "workshop": {"kind": "workshop", "type_key": "DeclaredWorkshop"}}]}


def main():
    schema = json.loads((ROOT / "schemas/mcp_production_chain_v1.json").read_text())
    Draft202012Validator.check_schema(schema)
    validator = Draft202012Validator(schema)
    passed = rejected = 0
    base = request()
    positives = [base, dict(base, recipes=[])]
    for section in [None, "all", "resources", "recipes", "steps", "shortages"]:
        for limit in [None, 1, 128]:
            positives.append(dict(base, section=section, limit=limit))
    for value in [None, 1, 10_000_000]:
        positives.append(dict(base, max_work=value))
    positives += [dict(base, continuation=None), dict(base, continuation="pc1:1:" + "a" * 64)]
    furnace = copy.deepcopy(base)
    furnace["recipes"][0]["workshop"]["kind"] = "furnace"
    positives.append(furnace)
    for q in positives:
        assert validator.is_valid(q), list(validator.iter_errors(q))
        passed += 1
    negatives = []
    for field in ["kind", "quantity_unit", "resources", "quotas", "recipes"]:
        q = copy.deepcopy(base)
        del q[field]
        negatives.append(q)
    for field, value in [("commit", True), ("raw_lua", "return true"), ("quantity_unit", "mass"),
                         ("resources", []), ("resources", base["resources"] * 17),
                         ("quotas", []), ("quotas", base["quotas"] * 65),
                         ("recipes", base["recipes"] * 33), ("section", "dispatch"),
                         ("limit", 0), ("limit", 129), ("max_work", 0), ("max_work", 10_000_001),
                         ("continuation", "pc1:01:" + "a" * 64), ("continuation", "pc1:1:" + "A" * 64)]:
        negatives.append(dict(base, **{field: value}))
    for field, value in [("key", ""), ("key", "x" * 65), ("key", "a\0b"), ("item_types", []),
                         ("item_types", ["type"] * 9), ("subtype", 2**31), ("dispatch", True)]:
        q = copy.deepcopy(base); q["resources"][0][field] = value; negatives.append(q)
    for field, value in [("output", ""), ("output_batch_size", 0), ("output_batch_size", 2**32),
                         ("output_batch_size", True), ("inputs", [{}]), ("job_token", ""), ("raw_lua", "x")]:
        q = copy.deepcopy(base); q["recipes"][0][field] = value; negatives.append(q)
    for field, value in [("kind", "custom"), ("type_key", ""), ("type_key", "x" * 129), ("execute", True)]:
        q = copy.deepcopy(base); q["recipes"][0]["workshop"][field] = value; negatives.append(q)
    for field, value in [("resource", ""), ("units", 0), ("units", 2**32), ("units", -1), ("command", "x")]:
        q = copy.deepcopy(base); q["recipes"][0]["inputs"][0][field] = value; negatives.append(q)
    for value in [-1, 2**32, True, "4"]:
        q = copy.deepcopy(base); q["quotas"][0]["minimum_stock"] = value; negatives.append(q)
    for q in negatives:
        assert not validator.is_valid(q), q
        rejected += 1
    print(json.dumps({"evidence": "production_chain_json_schema_only", "accepted": passed,
                      "rejected": rejected, "rust_executed": False, "mcp_executed": False}, sort_keys=True))


if __name__ == "__main__":
    main()
