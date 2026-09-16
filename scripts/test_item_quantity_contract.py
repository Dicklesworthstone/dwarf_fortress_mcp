#!/usr/bin/env python3
"""Scoped schema + independent bound checks; does not execute the Rust runtime."""
from __future__ import annotations

import copy
import hashlib
import itertools
import json
import operator
from pathlib import Path

from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[1]
MAX_U64 = (1 << 64) - 1
OPS = {"eq": operator.eq, "ne": operator.ne, "lt": operator.lt,
       "le": operator.le, "gt": operator.gt, "ge": operator.ge}


def bounds(rows: tuple) -> tuple[int, int | None]:
    low, high = 0, 0
    for member, quantity in rows:
        if member is False:
            continue
        if quantity is None:
            high = None
            continue
        if member is True:
            low += quantity
            if low > MAX_U64:
                raise OverflowError("lower bound exceeds u64")
        if high is not None:
            high += quantity
            if high > MAX_U64:
                raise OverflowError("upper bound exceeds u64")
    return low, high


def interval_truth(low: int, high: int | None, op: str, value: int) -> bool | None:
    if high is None:
        decisive = {"eq": (low > value, False), "ne": (low > value, True),
                    "lt": (low >= value, False), "le": (low > value, False),
                    "gt": (low > value, True), "ge": (low >= value, True)}
        established, answer = decisive[op]
        return answer if established else None
    if op in ("eq", "ne"):
        if low == high:
            return OPS[op](low, value)
        if value < low or value > high:
            return op == "ne"
        return None
    left, right = OPS[op](low, value), OPS[op](high, value)
    return left if left == right else None


def completions(rows: tuple) -> set[int]:
    # Quantity unknowns have no finite maximum in the contract. These bounded
    # substitutions are adversarial witnesses, not an infinity enumeration.
    totals = {0}
    for member, quantity in rows:
        choices = {0} if member is False else set()
        if member is not False:
            choices.update((0, 1, 3, 17) if quantity is None else (quantity,))
        if member is None:
            choices.add(0)
        totals = {before + units for before in totals for units in choices}
    return totals


def run() -> dict:
    raw = {name: (ROOT / "schemas" / name).read_bytes() for name in
           ("mcp_query_v1.json", "mcp_watch_count_v1.json", "mcp_item_quantity_v1.json")}
    base, count, quantity = (json.loads(raw[name]) for name in raw)
    # Deliberately validate only the new request/condition contracts and their
    # actual delegated definitions, not unrelated MCP or runtime schemas.
    definitions = {key: base["$defs"][key] for key in ("name", "watch_literal")}
    definitions.update(count["$defs"])
    validators = {}
    for key in ("query", "condition"):
        schema = {"$schema": "https://json-schema.org/draft/2020-12/schema",
                  "$defs": definitions, **quantity[key]}
        Draft202012Validator.check_schema(schema)
        validators[key] = Draft202012Validator(schema)
    cases = {"accepted": 0, "rejected": 0}

    def check(kind: str, value: dict, valid: bool) -> None:
        actual = validators[kind].is_valid(value)
        assert actual == valid, (kind, value, list(validators[kind].iter_errors(value)))
        cases["accepted" if valid else "rejected"] += 1

    literals = [{"type": "null"}, {"type": "bool", "value": False},
                {"type": "i64", "value": -1}, {"type": "u64", "value": MAX_U64},
                {"type": "text", "value": "BAR"},
                {"type": "fixed", "value": {"units": 2, "scale": 1}}]
    predicates = [{"op": "always"}]
    for literal in literals:
        leaf = {"op": "field", "field": "type_key", "comparison": "eq", "value": literal}
        predicates.extend([leaf, {"op": "not", "arg": leaf},
                           {"op": "all", "args": [leaf, {"op": "always"}]},
                           {"op": "any", "args": [leaf, {"op": "always"}]}])
    query = {"kind": "item_quantity", "scope": "observed_projection",
             "quantity_unit": "stack_units", "predicate": {"op": "always"}}
    condition = {"op": "item_quantity", "scope": "observed_projection",
                 "quantity_unit": "stack_units", "predicate": {"op": "always"},
                 "comparison": "ge", "value": 7}
    for predicate in predicates:
        check("query", {**query, "predicate": predicate}, True)
        for op, value in itertools.product(OPS, (0, 7, MAX_U64)):
            check("condition", {**condition, "predicate": predicate, "comparison": op, "value": value}, True)
    for kind, sample in (("query", query), ("condition", condition)):
        for key in sample:
            bad = copy.deepcopy(sample)
            del bad[key]
            check(kind, bad, False)
        for key, value in (("scope", "complete_world"), ("scope", None),
                           ("quantity_unit", "portions"), ("quantity_unit", "records"),
                           ("limit", 1), ("continuation", "x"), ("field", "stack_size"),
                           ("predicate", {"op": "all", "args": []}),
                           ("predicate", {"op": "any", "args": []}),
                           ("predicate", {"op": "not", "arg": condition}),
                           ("predicate", {"op": "always", "extra": True}),
                           ("predicate", {"op": "field", "field": "", "comparison": "eq", "value": literals[0]})):
            check(kind, {**sample, key: value}, False)
    for value in (-1, MAX_U64 + 1, 0.5, "7", True, None):
        check("condition", {**condition, "value": value}, False)
    check("condition", {**condition, "comparison": "gte"}, False)
    for literal in ({"type": "u64", "value": -1}, {"type": "u64", "value": MAX_U64 + 1},
                    {"type": "text", "value": "a\0b"}, {"type": "bool", "value": 1}):
        check("query", {**query, "predicate": {"op": "field", "field": "keep",
              "comparison": "eq", "value": literal}}, False)

    interval_cases = 0
    for low in range(12):
        for high in range(low, 12):
            for op, value in itertools.product(OPS, range(14)):
                answers = {OPS[op](n, value) for n in range(low, high + 1)}
                expected = answers.pop() if len(answers) == 1 else None
                assert interval_truth(low, high, op, value) is expected
                interval_cases += 1
    population_cases = 0
    comparisons = 0
    choices = list(itertools.product((False, True, None), (0, 1, 3, None)))
    for size in range(4):
        for rows in itertools.product(choices, repeat=size):
            low, high = bounds(rows)
            possible = completions(rows)
            assert min(possible) == low
            assert high is None or max(possible) == high
            for op, value in itertools.product(OPS, range(11)):
                result = interval_truth(low, high, op, value)
                # Weighted selections may have gaps. Unknown is conservative;
                # only a definite conclusion must agree with EVERY completion.
                if result is not None:
                    assert all(OPS[op](n, value) == result for n in possible), (rows, op, value)
                comparisons += 1
            population_cases += 1
    assert interval_truth(0, 3, "eq", 2) is None  # no subset-sum exactness claim
    overflow_cases = 0
    for rows in (((True, MAX_U64), (True, 1)), ((None, MAX_U64), (None, 1))):
        try:
            bounds(rows)
        except OverflowError:
            overflow_cases += 1
        else:
            raise AssertionError("overflow was silently accepted")
    assert bounds(((True, (1 << 53) + 1), (True, 2))) == ((1 << 53) + 3, (1 << 53) + 3)
    assert bounds(((None, 0), (False, None))) == (0, 0)
    return {"status": "passed", "scope": "new condition/query schemas and independent quantity-bound model only",
            "schema_cases": cases, "finite_interval_cases": interval_cases,
            "population_models": population_cases, "population_comparisons": comparisons,
            "overflow_rejections": overflow_cases, "rust_executed": False,
            "mcp_executed": False, "journal_recovery_executed": False,
            "quantity_extension_sha256": hashlib.sha256(raw["mcp_item_quantity_v1.json"]).hexdigest(),
            "delegated_definitions_sha256": hashlib.sha256(json.dumps(definitions, sort_keys=True, separators=(",", ":")).encode()).hexdigest(),
            "script_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest()}


if __name__ == "__main__":
    print(json.dumps(run(), indent=2, sort_keys=True))
