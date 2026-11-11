#!/usr/bin/env python3
"""Check decoded candidate request syntax; Rust owns semantic admission."""

import copy
import json
from pathlib import Path

from jsonschema import Draft202012Validator


def main() -> None:
    root = Path(__file__).resolve().parents[1]
    schema = json.loads((root / "schemas/lab_plan_comparison.schema.json").read_text())
    Draft202012Validator.check_schema(schema)
    validator = Draft202012Validator(schema)
    candidates = [
        {"key": "one", "paused_target": True},
        {"key": "two", "paused_target": True},
    ]
    compare = {"template": "compare_candidates", "version": 1,
               "horizon_ticks": 100, "candidates": candidates}
    choose = {
        "template": "select_candidate", "version": 1,
        "expected_anchor": {"fortress_id": "1917", "epoch": 1,
                            "sequence": 1, "game_tick": 1, "state_hash": "a" * 64},
        "candidate": candidates[0], "candidate_digest": "b" * 64,
    }
    cases = [(compare, True), (choose, True)]
    for count in (3, 4):
        value = copy.deepcopy(compare)
        value["candidates"] = [{"key": f"c{i}", "paused_target": True} for i in range(count)]
        cases.append((value, True))
    for source in (
        {"actions": [{"action": {"kind": "pause", "paused": True}}]},
        {"blueprint": {"template": "production", "quotas": [{"item": "DRINK", "minimum": 60}]}},
    ):
        value = copy.deepcopy(compare)
        value["candidates"][0] = {"key": "other", "summary": "a reviewed alternative", **source}
        cases.append((value, True))
    for baseline in (compare, choose):
        for key in baseline:
            value = copy.deepcopy(baseline)
            del value[key]
            cases.append((value, False))
        for key, bad in (("version", 2), ("version", None), ("unexpected", True)):
            value = copy.deepcopy(baseline)
            value[key] = bad
            cases.append((value, False))
    for bad in (0, -1, 1.5, None, 18446744073709551616):
        value = copy.deepcopy(compare)
        value["horizon_ticks"] = bad
        cases.append((value, False))
    for bad in ([], candidates[:1], candidates + candidates + candidates[:1], [candidates[0]] * 2):
        value = copy.deepcopy(compare)
        value["candidates"] = bad
        cases.append((value, False))
    for bad in (
        {"key": "none"}, {"key": "both", "paused_target": True, "actions": []},
        {"key": "has space", "paused_target": True}, {"key": "k" * 65, "paused_target": True},
        {"key": "null", "paused_target": None}, {"key": "bad", "actions": "[]"},
        {"key": "bad", "blueprint": []}, {"key": "extra", "paused_target": True, "trust": True},
        {"key": "long", "paused_target": True, "summary": "x" * 4097},
    ):
        value = copy.deepcopy(compare)
        value["candidates"][0] = bad
        cases.append((value, False))
    for bad in ("b" * 63, "B" * 64, None):
        value = copy.deepcopy(choose)
        value["candidate_digest"] = bad
        cases.append((value, False))
    project = {
        "template": "compare_candidates", "version": 2,
        "forecast_scope": "complete_project", "horizon_ticks": 2000,
        "candidates": [
            {"key": "ordinary", "blueprint": {
                "template": "production", "quotas": [{"item": "BED", "minimum": 2}],
                "acquire_resources": {"assign_labor": False},
            }},
            {"key": "staffed", "blueprint": {
                "template": "production", "quotas": [{"item": "BED", "minimum": 2}],
                "acquire_resources": {"assign_labor": True},
            }},
        ],
    }
    cases.append((project, True))
    for template in ("provisioned_room", "connected_room"):
        value = copy.deepcopy(project)
        value["candidates"][0]["blueprint"] = {"template": template}
        # This schema gates the wrapper; each typed compiler validates geometry.
        cases.append((value, True))
    for key in project:
        value = copy.deepcopy(project)
        del value[key]
        cases.append((value, False))
    for key, bad in (("version", 1), ("version", 3),
                     ("forecast_scope", None), ("forecast_scope", "current_stage"),
                     ("forecast_scope", True), ("unexpected", True)):
        value = copy.deepcopy(project)
        value[key] = bad
        cases.append((value, False))
    for bad in ({"key": "pause", "paused_target": True},
                {"key": "actions", "actions": []},
                {"key": "legacy_room", "blueprint": {"template": "dining_hall"}},
                {"key": "recursive", "blueprint": {"template": "compare_candidates"}}):
        value = copy.deepcopy(project)
        value["candidates"][0] = bad
        cases.append((value, False))
    for baseline in (compare, choose):
        value = copy.deepcopy(baseline)
        value["forecast_scope"] = "complete_project"
        cases.append((value, False))
    for request, accepted in cases:
        if validator.is_valid(request) != accepted:
            raise AssertionError(f"expected accepted={accepted}: {request!r}")
    parent = json.loads((root / "schemas/plan.input.schema.json").read_text())
    Draft202012Validator.check_schema(parent)
    assert parent["$defs"]["laboratory_plan_comparison"]["$ref"] == "./lab_plan_comparison.schema.json"
    print(json.dumps({"accepted": sum(wanted for _, wanted in cases),
                      "rejected": sum(not wanted for _, wanted in cases),
                      "scope": "decoded JSON Schema syntax; Rust source/goal/authority/budget checks remain separate"}))


if __name__ == "__main__":
    main()
