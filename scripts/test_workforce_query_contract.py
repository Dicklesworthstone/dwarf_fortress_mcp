#!/usr/bin/env python3
"""Workforce schema checks and independent allocation oracles; never Rust evidence."""
import argparse
import copy
import hashlib
import itertools
import json
from pathlib import Path
from jsonschema import Draft202012Validator


def schema_cases(kind):
    demand = {"key": "wood", "workers": 1, "target": [0, 0, 5], "skill_key": "CARPENTRY"}
    candidate = {"kind": kind, "target": [0, 0, 5], "skill_key": "CARPENTRY"}
    base = {"kind": kind, "demands": [demand]} if kind == "workforce_plan" else candidate
    yield base, True
    for field, value in [("limit", 1), ("limit", 128), ("limit", None), ("max_work", 1),
                         ("max_work", None), ("continuation", None)]:
        value_dict = copy.deepcopy(base); value_dict[field] = value
        yield value_dict, True
    prefix = "wp1" if kind == "workforce_plan" else "wc2"
    value_dict = copy.deepcopy(base); value_dict["continuation"] = f"{prefix}:1:" + "a" * 64
    yield value_dict, True
    for field, value in [("limit", 0), ("limit", 129), ("limit", True), ("max_work", 0),
                         ("max_work", 10_000_001), ("max_work", "100"), ("dispatch", True),
                         ("command", "assign labor"), ("continuation", f"{prefix}:01:" + "a" * 64),
                         ("continuation", f"{prefix}:1:" + "a" * 64 + "\n"),
                         ("continuation", "wc1:1:" + "a" * 64)]:
        value_dict = copy.deepcopy(base); value_dict[field] = value
        yield value_dict, False
    for field, value, valid in [("skill_key", "", False), ("skill_key", "x" * 97, False),
                                ("skill_key", "CARPENTRY\n", False), ("skill_key", "x\0", False),
                                ("skill_key", "x\u0085", False), ("min_effective_skill", -1, False),
                                ("min_effective_skill", 2**31, False), ("min_effective_skill", 0, True),
                                ("min_effective_skill", None, True), ("preserve_social", False, True),
                                ("adults_only", None, True), ("target", [0, 0], False),
                                ("target", [-1, 0, 5], False), ("target", [32768, 0, 5], False),
                                ("target", [0.5, 0, 5], False), ("extra", 1, False)]:
        value_dict = copy.deepcopy(base)
        target = value_dict["demands"][0] if kind == "workforce_plan" else value_dict
        target[field] = value
        yield value_dict, valid
    if kind == "workforce_plan":
        for field, value in [("key", "bad key"), ("key", "wood\n"), ("workers", 0),
                             ("workers", 129), ("workers", True)]:
            value_dict = copy.deepcopy(base); value_dict["demands"][0][field] = value
            yield value_dict, False
        for size, valid in [(0, False), (16, True), (17, False)]:
            value_dict = copy.deepcopy(base)
            value_dict["demands"] = [dict(demand, key=f"d{i}") for i in range(size)]
            yield value_dict, valid
        for field in demand:
            value_dict = copy.deepcopy(base); del value_dict["demands"][0][field]
            yield value_dict, False
    else:
        value_dict = copy.deepcopy(base); value_dict["max_work"] = 1_000_001
        yield value_dict, False
    for field in base:
        value_dict = copy.deepcopy(base); del value_dict[field]
        yield value_dict, False


def brute_assignment(masks, capacities):
    best = 0
    for assignment in itertools.product(range(-1, 3), repeat=len(masks)):
        used = [0, 0, 0]
        legal = True
        for worker, demand in enumerate(assignment):
            if demand < 0:
                continue
            used[demand] += 1
            if not masks[worker] & (1 << demand) or used[demand] > capacities[demand]:
                legal = False; break
        if legal:
            best = max(best, sum(used))
    return best


def hall_bound(masks, capacities):
    deficit = 0
    for subset in range(1, 8):
        required = sum(capacities[i] for i in range(3) if subset & (1 << i))
        neighbors = sum(bool(mask & subset) for mask in masks)
        deficit = max(deficit, required - neighbors)
    return sum(capacities) - deficit


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    schema_results = []
    for name, kind in [("mcp_workforce_candidates_v1.json", "workforce_candidates"),
                       ("mcp_workforce_plan_v1.json", "workforce_plan")]:
        raw = (root / "schemas" / name).read_bytes()
        schema = json.loads(raw); Draft202012Validator.check_schema(schema)
        validator = Draft202012Validator(schema)
        accepted = rejected = 0
        for index, (instance, expected) in enumerate(schema_cases(kind)):
            actual = validator.is_valid(instance)
            assert actual == expected, (name, index, instance, list(validator.iter_errors(instance)))
            accepted += actual; rejected += not actual
        schema_results.append({"file": name, "accepted": accepted, "rejected": rejected,
                               "sha256": hashlib.sha256(raw).hexdigest(),
                               "git_blob_sha": hashlib.sha1(b"blob " + str(len(raw)).encode() + b"\0" + raw).hexdigest()})
    oracle_cases = 0
    for masks in itertools.product(range(8), repeat=3):
        for capacities in itertools.product([1, 2], repeat=3):
            actual = brute_assignment(masks, capacities)
            expected = hall_bound(masks, capacities)
            assert actual == expected, (masks, capacities, actual, expected)
            oracle_cases += 1
    result = {"schema": "dfmcp.workforce-query-reference-check/1", "passed": True,
              "schemas": schema_results, "allocation_oracle_cases": oracle_cases,
              "oracle_scope": "independent exhaustive assignments versus deficient-subset bound; no production allocator executed",
              "rust_executed": False, "mcp_executed": False, "native_executed": False,
              "semantic_limits_not_schema_proven": ["unique demand keys", "sum of worker slots <= 128", "UTF-8 byte bounds", "current capture authority and reachability"],
              "script_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest()}
    rendered = json.dumps(result, indent=2) + "\n"
    if args.output:
        args.output.write_text(rendered)
    print(rendered, end="")


if __name__ == "__main__":
    main()
