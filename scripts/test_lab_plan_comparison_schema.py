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
    owned = {**copy.deepcopy(compare), "version": 3,
             "forecast_scope": "current_stage", "progression_scope": "owned_session"}
    cases.append((owned, True))
    for source in ({"actions": [{"action": {"kind": "pause", "paused": True}}]},
                   {"blueprint": {"template": "production", "quotas": [{"item": "DRINK", "minimum": 60}]}}):
        value = copy.deepcopy(owned)
        value["candidates"][0] = {"key": "other", **source}
        cases.append((value, True))
    for key in owned:
        value = copy.deepcopy(owned)
        del value[key]
        cases.append((value, False))
    for key, bad in (("version", 1), ("version", 2), ("version", 4),
                     ("forecast_scope", "complete_project"), ("forecast_scope", None),
                     ("progression_scope", "candidate_only"), ("progression_scope", None),
                     ("horizon_ticks", 0), ("restore_recovered_actions", True)):
        value = copy.deepcopy(owned)
        value[key] = bad
        cases.append((value, False))
    for baseline in (compare, choose, project):
        value = copy.deepcopy(baseline)
        value["progression_scope"] = "owned_session"
        cases.append((value, False))
    owned_project = {**copy.deepcopy(project), "version": 4,
                     "progression_scope": "owned_session"}
    cases.append((owned_project, True))
    for template in ("provisioned_room", "connected_room"):
        value = copy.deepcopy(owned_project)
        value["candidates"][0]["blueprint"] = {"template": template}
        cases.append((value, True))
    for count in (3, 4):
        value = copy.deepcopy(owned_project)
        value["candidates"] = [
            {**copy.deepcopy(project["candidates"][0]), "key": f"project{i}"}
            for i in range(count)
        ]
        cases.append((value, True))
    for key in owned_project:
        value = copy.deepcopy(owned_project)
        del value[key]
        cases.append((value, False))
    for key, bad in (("version", 1), ("version", 2), ("version", 3), ("version", 5),
                     ("forecast_scope", "current_stage"), ("forecast_scope", None),
                     ("progression_scope", "candidate_only"), ("progression_scope", None),
                     ("horizon_ticks", 0), ("automatic_continuation", True)):
        value = copy.deepcopy(owned_project)
        value[key] = bad
        cases.append((value, False))
    for bad in ({"key": "pause", "paused_target": True},
                {"key": "actions", "actions": []},
                {"key": "legacy", "blueprint": {"template": "dining_hall"}},
                {"key": "continuation", "blueprint": {"template": "continue_goal"}},
                {"key": "recursive", "blueprint": {"template": "compare_candidates"}}):
        value = copy.deepcopy(owned_project)
        value["candidates"][0] = bad
        cases.append((value, False))
    competing_projects = {
        **copy.deepcopy(owned_project), "version": 5,
        "competing_plan_digests": ["a" * 64],
    }
    cases.append((competing_projects, True))
    for count in (2, 7):
        value = copy.deepcopy(competing_projects)
        value["competing_plan_digests"] = [f"{i:064x}" for i in range(count)]
        cases.append((value, True))
    # Selection order is not a new authority requirement; runtime normalizes it.
    value = copy.deepcopy(competing_projects)
    value["competing_plan_digests"] = ["f" * 64, "0" * 64, "a" * 64]
    cases.append((value, True))
    for horizon in (1, 18446744073709551615):
        value = copy.deepcopy(competing_projects)
        value["horizon_ticks"] = horizon
        cases.append((value, True))
    for template in ("provisioned_room", "connected_room"):
        value = copy.deepcopy(competing_projects)
        value["candidates"][0]["blueprint"] = {"template": template}
        cases.append((value, True))
    for count in (3, 4):
        value = copy.deepcopy(competing_projects)
        value["candidates"] = [
            {**copy.deepcopy(project["candidates"][0]), "key": f"peer_project{i}"}
            for i in range(count)
        ]
        cases.append((value, True))
    for key in competing_projects:
        value = copy.deepcopy(competing_projects)
        del value[key]
        cases.append((value, False))
    for key, bad in (
        ("version", 1), ("version", 2), ("version", 3), ("version", 4),
        ("version", 6), ("version", None), ("version", True), ("version", "5"),
        ("template", "select_candidate"), ("template", "continue_goal"),
        ("forecast_scope", "current_stage"), ("forecast_scope", None),
        ("forecast_scope", True),
        ("progression_scope", "candidate_only"), ("progression_scope", None),
        ("progression_scope", True),
        ("automatic_continuation", True), ("restore_recovered_actions", True),
        ("unexpected", True),
    ):
        value = copy.deepcopy(competing_projects)
        value[key] = bad
        cases.append((value, False))
    for bad in (
        None, [], "a" * 64, {}, True, 1,
        [f"{i:064x}" for i in range(8)],
        ["a" * 64, "a" * 64],
        ["a" * 64, "b" * 64, "a" * 64],
        [""], ["a" * 63], ["a" * 65], ["A" * 64], ["g" * 64],
        ["0x" + "a" * 62], ["a" * 63 + " "],
        ["a" * 64 + "\\n"], ["\\n" + "a" * 64],
        [None], [True], [1], [{}], [["a" * 64]],
    ):
        value = copy.deepcopy(competing_projects)
        value["competing_plan_digests"] = bad
        cases.append((value, False))
    for bad in (0, -1, 1.5, None, True, "100", 18446744073709551616):
        value = copy.deepcopy(competing_projects)
        value["horizon_ticks"] = bad
        cases.append((value, False))
    for bad in (
        None, {}, [], competing_projects["candidates"][:1],
        [competing_projects["candidates"][0]] * 2,
        [{**copy.deepcopy(project["candidates"][0]), "key": f"over{i}"}
         for i in range(5)],
    ):
        value = copy.deepcopy(competing_projects)
        value["candidates"] = bad
        cases.append((value, False))
    for bad in (
        {"key": "pause", "paused_target": True},
        {"key": "actions", "actions": []},
        {"key": "legacy", "blueprint": {"template": "dining_hall"}},
        {"key": "continuation", "blueprint": {"template": "continue_goal"}},
        {"key": "recursive", "blueprint": {"template": "compare_candidates"}},
        {"key": "selection", "blueprint": {"template": "select_candidate"}},
        {"key": "missing_template", "blueprint": {}},
        {"key": "null_blueprint", "blueprint": None},
        {"key": "unknown_field", "blueprint": {"template": "production"},
         "competing_plan_digests": ["a" * 64]},
        {"key": "both_sources", "blueprint": {"template": "production"},
         "paused_target": True},
    ):
        value = copy.deepcopy(competing_projects)
        value["candidates"][0] = bad
        cases.append((value, False))
    # A peer list cannot opt an older generation or selection into new behavior.
    # Explicit null and empty lists remain rejected, rather than meaning omission.
    for baseline in (compare, choose, project, owned, owned_project):
        for peers in (None, [], ["a" * 64]):
            value = copy.deepcopy(baseline)
            value["competing_plan_digests"] = peers
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
