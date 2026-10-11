#!/usr/bin/env python3
"""Structural schema checks only; not Rust decoder or game execution evidence."""
from copy import deepcopy
import json
from pathlib import Path

from jsonschema import Draft202012Validator


def check(root: Path) -> dict:
    cases = [
        ("civilian.protection.request.schema.json", "civilian_protection_v1.json", "workflow", "civilian_protection_v1"),
        ("civilian.protection.query.schema.json", "civilian_protection_query_v1.json", "mode", "civilian_protection"),
    ]
    rejected = 0
    for schema_name, example_name, selector, tag in cases:
        schema = json.loads((root / "schemas" / schema_name).read_text())
        Draft202012Validator.check_schema(schema)
        validate = Draft202012Validator(schema)
        example = json.loads((root / "schemas/examples" / example_name).read_text())
        validate.validate(example)
        mutations = [None, [], True, "not a request"]
        for key in (selector, "burrow", "civilians"):
            bad = deepcopy(example)
            del bad[key]
            mutations.append(bad)
        for wrong in (None, True, 1, "unknown_v9"):
            bad = deepcopy(example)
            bad[selector] = wrong
            mutations.append(bad)
        for field in ("authority", "native_endpoint", "threat_clear", "release", "source_policy"):
            bad = deepcopy(example)
            bad[field] = True
            mutations.append(bad)
        for bad_id in ("", "0", "01", "+1", "-1", "1.0", "١", "1" * 21, 1001, None):
            for key in ("burrow", "civilians"):
                bad = deepcopy(example)
                bad[key] = bad_id if key == "burrow" else [bad_id]
                mutations.append(bad)
        for roster in ([], ["1001", "1001"], [str(i) for i in range(1, 66)], {}, "1001", None):
            bad = deepcopy(example)
            bad["civilians"] = roster
            mutations.append(bad)
        for index, candidate in enumerate(mutations):
            if validate.is_valid(candidate):
                raise AssertionError(f"{tag} accepted malformed shape {index}: {candidate}")
            rejected += 1
    return {
        "schemas_checked": len(cases),
        "examples_accepted": len(cases),
        "malformed_shapes_rejected": rejected,
        "rust_tests_executed": False,
        "native_or_live_qualification": False,
        "scope": "Draft 2020-12 structural schemas only; Rust additionally checks u64 range, aliases, duplicate JSON keys and byte bounds",
    }


if __name__ == "__main__":
    print(json.dumps(check(Path(__file__).resolve().parents[1]), indent=2))
