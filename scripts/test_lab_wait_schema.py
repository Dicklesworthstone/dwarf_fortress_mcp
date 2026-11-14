#!/usr/bin/env python3
"""Check wait selector syntax; Rust owns original custody and effect admission."""

import itertools
import json
from pathlib import Path

from jsonschema import Draft202012Validator


def main() -> None:
    root = Path(__file__).resolve().parents[1]
    schema = json.loads((root / "schemas/lab_wait.schema.json").read_text())
    Draft202012Validator.check_schema(schema)
    validator = Draft202012Validator(schema)
    selectors = {
        "plan_digest": "a" * 64,
        "plan_digests": ["a" * 64, "b" * 64],
        "watch_id": "watch:" + "c" * 64,
        "watch_ids": ["watch:" + "c" * 64, "watch:" + "d" * 64],
    }
    cases = [({}, True), ({"max_game_ticks": 0}, True)]
    for name, value in selectors.items():
        cases.append(({name: value}, True))
        cases.append(({**dict.fromkeys(selectors), name: value}, True))
    for left, right in itertools.combinations(selectors, 2):
        cases.append(({left: selectors[left], right: selectors[right]}, False))
    for policy in (None, "all", "any"):
        cases.append(({"plan_digests": selectors["plan_digests"], "plan_policy": policy}, True))
    cases.append(({"plan_digests": [f"{number:064x}" for number in range(1, 9)]}, True))
    for bad in ([], ["a" * 64] * 2, [f"{number:064x}" for number in range(9)],
                ["A" * 64], ["a" * 63], [None], [7], "a" * 64):
        cases.append(({"plan_digests": bad}, False))
    for bad in ("all_satisfied", "ALL", "", True, 1, []):
        cases.append(({"plan_digests": selectors["plan_digests"], "plan_policy": bad}, False))
    for selector in ({}, {"plan_digests": None}, {"plan_digest": "a" * 64},
                     {"watch_ids": selectors["watch_ids"]}):
        cases.append(({**selector, "plan_policy": "all"}, False))
    cases.extend([
        ({"plan_digests": selectors["plan_digests"], "watch_policy": "all_satisfied"}, False),
        ({"watch_ids": selectors["watch_ids"], "watch_policy": "any_satisfied"}, True),
        ({"plan_policy": None, "watch_policy": None}, True),
        ({"plan_digests": selectors["plan_digests"], "max_game_ticks": -1}, False),
        ({"plan_digests": selectors["plan_digests"], "unknown": True}, False),
    ])
    owner = "1" * 32
    for selector in ("plan_digest", "plan_digests"):
        for ticks in (None, 0):
            cases.append(({"session_id": owner, selector: selectors[selector],
                           "as_task": True, "max_game_ticks": ticks}, True))
        for extra in ({"max_game_ticks": 1}, {"session_id": None},
                      {"as_task": "true"}, {"as_task": None}):
            cases.append(({"session_id": owner, selector: selectors[selector],
                           "as_task": True, **extra}, False))
        cases.append(({selector: selectors[selector], "as_task": True}, False))
    cases.extend([
        ({"as_task": False, "max_game_ticks": 10}, True),
        ({"session_id": owner, "as_task": True}, False),
        ({"session_id": owner, "watch_ids": selectors["watch_ids"], "as_task": True}, False),
        ({"session_id": owner, "plan_digests": selectors["plan_digests"],
          "plan_policy": "any", "as_task": True}, True),
    ])
    cases.extend((example, True) for example in schema["examples"])
    for request, accepted in cases:
        if validator.is_valid(request) != accepted:
            raise AssertionError(f"expected accepted={accepted}: {request!r}")
    print(json.dumps({
        "accepted": sum(accepted for _, accepted in cases),
        "rejected": sum(not accepted for _, accepted in cases),
        "scope": "JSON Schema syntax; runtime custody, shared bounds and publication are separate",
    }))


if __name__ == "__main__":
    main()
