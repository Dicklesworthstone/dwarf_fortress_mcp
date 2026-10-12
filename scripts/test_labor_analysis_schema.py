#!/usr/bin/env python3
"""Structural contract checks only; these do not execute the Rust query or allocator."""
from __future__ import annotations

import copy
import itertools
import json
from pathlib import Path

from jsonschema import Draft202012Validator


def main() -> None:
    root = Path(__file__).resolve().parents[1]
    schema = json.loads((root / "schemas/labor.analysis.query.schema.json").read_text())
    execution = json.loads((root / "schemas/labor.execution.request.schema.json").read_text())
    example = json.loads((root / "schemas/examples/labor_analysis_query_v1.json").read_text())
    Draft202012Validator.check_schema(schema)
    Draft202012Validator.check_schema(execution)
    validate = Draft202012Validator(schema)
    validate.validate(example)
    if len(json.dumps(example, separators=(",", ":")).encode()) > 1024:
        raise AssertionError("example does not fit the existing query input bound")

    bad = []
    for key in ("mode", "roster", "demands"):
        value = copy.deepcopy(example)
        del value[key]
        bad.append(value)
    for key in ("authority", "source_policy", "workflow", "native_endpoint"):
        value = copy.deepcopy(example)
        value[key] = True
        bad.append(value)
    for key, values in {
        "id": ["0", "01", "+1", "-1", "x", 1001, "1" * 21],
        "skills": [[], None, {"MINING": -1}, {"MINING": 21}, {"MINING": True}, {" ": 1}],
        "stress_level": [-1, 1001, True, "100", 1.5],
        "assigned_labors": [[], ["MINING"]],
    }.items():
        for item in values:
            value = copy.deepcopy(example)
            value["roster"][0][key] = item
            bad.append(value)
    for key, values in {
        "workers": [-1, 65, True, "1", 1.5],
        "minimum_skill": [-1, 21, True],
        "priority": [-1, 256, True],
        "labor": ["", " ", "x" * 129, "a\nb"],
        "skill": ["", " ", "x" * 257, "a\nb"],
    }.items():
        for item in values:
            value = copy.deepcopy(example)
            value["demands"][0][key] = item
            bad.append(value)
    for maximum in [0, -1, 1_000_001, True, "100", 1.5]:
        value = copy.deepcopy(example)
        value["max_work"] = maximum
        bad.append(value)
    for key, count in [("roster", 0), ("roster", 65), ("demands", 0), ("demands", 17)]:
        value = copy.deepcopy(example)
        value[key] = value[key][:1] * count
        bad.append(value)
    for index, value in enumerate(bad):
        if validate.is_valid(value):
            raise AssertionError(f"malformed structural mutation {index} was accepted")

    # Check that all fixture initial matrices can be represented by the EXISTING
    # guarded workflow schema. This is not a second allocation implementation.
    workflow_validator = Draft202012Validator(execution)
    variants = 0
    for bits in itertools.product([False, True], repeat=4):
        workflow = {"workflow": "labor_allocation_v1", "roster": copy.deepcopy(example["roster"]),
                    "demands": copy.deepcopy(example["demands"])}
        for index, row in enumerate(workflow["roster"]):
            row["assigned_labors"] = [labor for col, labor in enumerate(["BREW", "MINING"])
                                      if bits[2 * index + col]]
        workflow_validator.validate(workflow)
        variants += 1
    print(json.dumps({"scope": "Python JSON-Schema structural checks, not Rust execution",
                      "schemas_checked": 2, "examples_accepted": 1,
                      "malformed_structural_mutations_rejected": len(bad),
                      "existing_workflow_fixture_variants_accepted": variants,
                      "rust_tests_executed": 0}, sort_keys=True))


if __name__ == "__main__":
    main()
