#!/usr/bin/env python3
"""Input-schema checks; not Rust/MCP execution or qualification."""
import copy
import json
from pathlib import Path
from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[1]
SCHEMA = json.loads((ROOT / "schemas/mcp_workforce_plan_v1.json").read_text())
Draft202012Validator.check_schema(SCHEMA)
VALIDATOR = Draft202012Validator(SCHEMA)
BASE = {"kind": "workforce_plan", "demands": [
    {"key": "wood", "workers": 1, "target": [0, 0, 5], "skill_key": "CARPENTRY"}
]}


def main():
    checked = 0
    absent = object()
    for objective in [absent, None, "max_filled_slots", "priority_skill_distance", "greedy", True, 1, {}]:
        for priority in [absent, None, 0, 1, 1000, -1, 1001, 65536, 1.5, True, "1"]:
            for prefix in [None, "wp1", "wq1", "wc2"]:
                q = copy.deepcopy(BASE)
                if objective is not absent:
                    q["objective"] = objective
                if priority is not absent:
                    q["demands"][0]["priority"] = priority
                if prefix:
                    q["continuation"] = f"{prefix}:1:" + "a" * 64
                quality = objective == "priority_skill_distance"
                objective_ok = objective is absent or objective is None or objective in ("max_filled_slots", "priority_skill_distance")
                priority_ok = priority is absent or priority is None or (quality and type(priority) is int and 0 <= priority <= 1000)
                cursor_ok = prefix is None or prefix == ("wq1" if quality else "wp1")
                expected = objective_ok and priority_ok and cursor_ok
                assert VALIDATOR.is_valid(q) == expected, (q, list(VALIDATOR.iter_errors(q)))
                checked += 1
    for prefix in ["wp1", "wq1"]:
        for bad in [f"{prefix}:0:" + "a" * 64, f"{prefix}:01:" + "a" * 64,
                    f"{prefix}:1:" + "a" * 63, f"{prefix}:1:" + "A" * 64, f"{prefix}:1:" + "a" * 64 + "\n"]:
            q = copy.deepcopy(BASE)
            if prefix == "wq1":
                q["objective"] = "priority_skill_distance"
            q["continuation"] = bad
            assert not VALIDATOR.is_valid(q), q
            checked += 1
    print(f"PASS: {checked} workforce quality schema cases")
    print("Scope: JSON Schema only; Rust parser and MCP handler not executed.")


if __name__ == "__main__":
    main()
