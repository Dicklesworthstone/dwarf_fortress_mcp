#!/usr/bin/env python3
"""Validate the historical-change request contract, never the Rust implementation.

Default mode resolves the actual repository's entity-selector schema. Explicit
--envelope-only tests only this new fragment's record/page envelope; selector
validation is then delegated, not claimed. No native or archive data is opened.
"""
import argparse
import copy
import hashlib
import json
from pathlib import Path

from jsonschema import Draft202012Validator


def run(root: Path, envelope_only: bool) -> dict:
    fragment_path = root / "schemas/mcp_historical_changes_v1.json"
    fragment_bytes = fragment_path.read_bytes()
    fragment = json.loads(fragment_bytes)
    Draft202012Validator.check_schema(fragment)
    contract = copy.deepcopy(fragment)
    if envelope_only:
        contract["properties"]["select"] = {"type": "object"}
    else:
        base = json.loads((root / "schemas/mcp_query_v1.json").read_bytes())
        contract["$defs"] = base["$defs"]
    Draft202012Validator.check_schema(contract)
    validator = Draft202012Validator(contract)
    sample = {
        "kind": "historical_changes",
        "from": {"record": 1, "record_digest": "a" * 64},
        "to": {"record": 2, "record_digest": "b" * 64},
        "select": {"kind": "entities", "kinds": ["unit"], "fields": ["profession"]},
    }
    cases = []

    def case(name, valid, value):
        cases.append((name, valid, copy.deepcopy(value)))

    case("minimal", True, sample)
    for limit in [None, 1, 16, 128]:
        value = copy.deepcopy(sample)
        value["limit"] = limit
        case(f"limit-{limit}", True, value)
    for token in [None, "qh1:1:" + "c" * 64, "qh1:511:" + "d" * 64]:
        value = copy.deepcopy(sample)
        value["continuation"] = token
        case(f"valid-token-{len(cases)}", True, value)
    for endpoint in ["from", "to"]:
        for number in [1, 4096]:
            value = copy.deepcopy(sample)
            value[endpoint]["record"] = number
            case(f"{endpoint}-record-{number}", True, value)
        for number in [0, -1, 4097, 1.5, True, "1", None]:
            value = copy.deepcopy(sample)
            value[endpoint]["record"] = number
            case(f"invalid-{endpoint}-record-{number}", False, value)
        for digest in ["", "a" * 63, "a" * 65, "A" * 64, "z" * 64, "a" * 64 + "\n", None]:
            value = copy.deepcopy(sample)
            value[endpoint]["record_digest"] = digest
            case(f"invalid-{endpoint}-digest-{len(cases)}", False, value)
        for missing in ["record", "record_digest"]:
            value = copy.deepcopy(sample)
            del value[endpoint][missing]
            case(f"missing-{endpoint}-{missing}", False, value)
        value = copy.deepcopy(sample)
        value[endpoint]["path"] = "/another/archive"
        case(f"{endpoint}-reject-path", False, value)
    for missing in ["kind", "from", "to", "select"]:
        value = copy.deepcopy(sample)
        del value[missing]
        case(f"missing-{missing}", False, value)
    for limit in [0, 129, -1, True, 1.5, "8"]:
        value = copy.deepcopy(sample)
        value["limit"] = limit
        case(f"invalid-limit-{limit}", False, value)
    for token in ["", "qh1:0:" + "a" * 64, "qh1:01:" + "a" * 64,
                  "ar1:1:" + "a" * 64, "qh1:1:" + "A" * 64,
                  "qh1:1:" + "a" * 64 + "\n", "x" * 129, 1]:
        value = copy.deepcopy(sample)
        value["continuation"] = token
        case(f"invalid-token-{len(cases)}", False, value)
    value = copy.deepcopy(sample)
    value["repair"] = True
    case("reject-repair", False, value)
    value = copy.deepcopy(sample)
    value["kind"] = "changes"
    case("reject-baseline-operation", False, value)
    for name, expected, value in cases:
        actual = validator.is_valid(value)
        if actual != expected:
            errors = [error.message for error in validator.iter_errors(value)]
            raise AssertionError(f"{name}: expected {expected}, got {actual}: {errors}")
    return {
        "status": "passed",
        "mode": "record_and_page_envelope_only" if envelope_only else "fragment_with_repository_selector_definitions",
        "cases": len(cases),
        "accepted": sum(expected for _, expected, _ in cases),
        "rejected": sum(not expected for _, expected, _ in cases),
        "fragment_sha256": hashlib.sha256(fragment_bytes).hexdigest(),
        "script_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "selector_validation_executed": not envelope_only,
        "rust_executed": False,
        "not_established": ["record ordering or existence", "digest agreement", "selection completeness",
                            "generation comparisons", "output pagination", "filesystem custody",
                            "MCP execution", "native or live qualification"],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--envelope-only", action="store_true")
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    result = run(args.repo, args.envelope_only)
    text = json.dumps(result, indent=2, sort_keys=True) + "\n"
    if args.report:
        # Do not overwrite an existing evidence file.
        with args.report.open("x", encoding="utf-8") as output:
            output.write(text)
    print(text, end="")


if __name__ == "__main__":
    main()
