#!/usr/bin/env python3
"""Request-schema and independent deficit-frontier oracle, not Rust/MCP execution."""
from copy import deepcopy
from itertools import product
import json
from pathlib import Path
from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[1]


def request():
    def recipe(yield_units):
        return {"output": "product", "output_batch_size": yield_units,
                "inputs": [{"resource": "raw", "units": 1}], "job_token": "DeclaredProduct",
                "workshop": {"kind": "workshop", "type_key": "DeclaredWorkshop"}}
    return {"kind": "production_chain_compare", "quantity_unit": "stack_units",
            "resources": [{"key": "raw", "item_types": ["item_type_3"]},
                          {"key": "product", "item_types": ["item_type_999"]}],
            "quotas": [{"resource": "raw", "minimum_stock": 4},
                       {"resource": "product", "minimum_stock": 4}],
            "candidates": [{"key": "demanding", "recipes": [recipe(1)]},
                           {"key": "efficient", "recipes": [recipe(2)]}]}


def dominates(left, right):
    return (len(left) == len(right) and all(a <= b for a, b in zip(left, right))
            and any(a < b for a, b in zip(left, right)))


def frontier(candidates):
    return tuple(i for i, candidate in enumerate(candidates)
                 if not any(j != i and dominates(other, candidate)
                            for j, other in enumerate(candidates)))


def main():
    chain = json.loads((ROOT / "schemas/mcp_production_chain_v1.json").read_text())
    schema = json.loads((ROOT / "schemas/mcp_production_chain_compare_v1.json").read_text())
    Draft202012Validator.check_schema(schema)
    validator = Draft202012Validator(schema)
    # The comparison's common-model declarations must keep exactly the same contract.
    for key in ["quantity_unit", "resources", "quotas", "limit", "max_work", "continuation"]:
        assert schema["properties"][key] == chain["properties"][key]
    assert (schema["properties"]["candidates"]["items"]["properties"]["recipes"]
            == chain["properties"]["recipes"])
    base = request()
    positives = [base]
    for count in range(2, 9):
        q = deepcopy(base)
        q["candidates"] = [{"key": f"c{i}", "recipes": q["candidates"][0]["recipes"]}
                           for i in range(count)]
        positives.append(q)
    for field, value in [("limit", None), ("limit", 1), ("limit", 128),
                         ("max_work", None), ("max_work", 1), ("max_work", 10_000_000),
                         ("continuation", None), ("continuation", "pc1:1:" + "a" * 64)]:
        positives.append(dict(base, **{field: value}))
    q = deepcopy(base); q["candidates"][1]["recipes"] = []; positives.append(q)
    q = deepcopy(base); q["candidates"][0]["recipes"][0]["workshop"]["kind"] = "furnace"; positives.append(q)
    negatives = []
    for missing in ["kind", "quantity_unit", "resources", "quotas", "candidates"]:
        q = deepcopy(base); del q[missing]; negatives.append(q)
    for field, value in [("quantity_unit", "mass"), ("candidates", []),
                         ("candidates", base["candidates"][:1]), ("candidates", base["candidates"] * 5),
                         ("resources", []), ("quotas", []), ("section", "steps"),
                         ("limit", 0), ("limit", 129), ("limit", True), ("max_work", 0),
                         ("max_work", 10_000_001), ("commit", True), ("raw_lua", "return true"),
                         ("continuation", "pc1:01:" + "a" * 64), ("continuation", "pc1:1:" + "A" * 64)]:
        negatives.append(dict(base, **{field: value}))
    for field, value in [("key", ""), ("key", "x" * 65), ("key", "a\0b"), ("key", 1),
                         ("recipes", [{}]), ("recipes", base["candidates"][0]["recipes"] * 33),
                         ("resources", base["resources"]), ("quotas", []), ("max_work", 100),
                         ("stock", 100), ("execute", True)]:
        q = deepcopy(base); q["candidates"][1][field] = value; negatives.append(q)
    for missing in ["key", "recipes"]:
        q = deepcopy(base); del q["candidates"][1][missing]; negatives.append(q)
    for field, value in [("output", ""), ("output_batch_size", 0), ("output_batch_size", 2**32),
                         ("inputs", [{"resource": "raw", "units": 0}]), ("job_token", "a\0b"),
                         ("raw_lua", "return true")]:
        q = deepcopy(base); q["candidates"][1]["recipes"][0][field] = value; negatives.append(q)
    for q in positives:
        assert validator.is_valid(q), list(validator.iter_errors(q))
    for q in negatives:
        assert not validator.is_valid(q), q
    # Independent dominance oracle: membership in the strict integer lower box.
    vectors = tuple(product(range(3), repeat=3))
    lower_boxes = {b: set(product(*(range(n + 1) for n in b))) - {b} for b in vectors}
    pairs = 0
    for a, b in product(vectors, repeat=2):
        assert dominates(a, b) == (a in lower_boxes[b])
        pairs += 1
    triples = 0
    for candidates in product(vectors, repeat=3):
        expected = tuple(i for i, b in enumerate(candidates)
                         if not (set(candidates) & lower_boxes[b]))
        assert frontier(candidates) == expected
        triples += 1
    assert frontier(((0, 0), (0, 0))) == (0, 1)
    assert frontier(((1, 0), (0, 1))) == (0, 1)
    assert frontier(((1, 0), (0, 0))) == (1,)
    assert not dominates((), ())
    assert not dominates((0,), (1, 2))
    print(json.dumps({"evidence": "schema_and_independent_deficit_frontier_reference_only",
                      "schema_accepted": len(positives), "schema_rejected": len(negatives),
                      "shared_schema_fields_checked": 7, "dominance_pairs": pairs,
                      "frontier_triples": triples, "boundary_cases": 5,
                      "rust_executed": False, "mcp_executed": False}, sort_keys=True))


if __name__ == "__main__":
    main()
