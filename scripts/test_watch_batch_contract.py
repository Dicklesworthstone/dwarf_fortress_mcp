#!/usr/bin/env python3
"""Validate watch-batch JSON requests. Does not execute Rust or a game runtime."""
from __future__ import annotations
import argparse
import hashlib
import itertools
import json
from pathlib import Path
from jsonschema import Draft202012Validator


def blob_sha(data: bytes) -> str:
    return hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest()


def run(schema_path: Path) -> dict:
    schema_bytes = schema_path.read_bytes()
    schema = json.loads(schema_bytes)
    Draft202012Validator.check_schema(schema)
    validator = Draft202012Validator(schema)
    handles = [f"watch:{n:064x}" for n in range(1, 10)]
    counts = {"accepted": 0, "rejected": 0}

    def check(value: object, expected: bool) -> None:
        errors = list(validator.iter_errors(value))
        if (not errors) != expected:
            raise AssertionError(f"unexpected validity: {value!r}; errors={errors!r}")
        counts["accepted" if expected else "rejected"] += 1

    for kind in ("poll_watches", "await_watches"):
        check({"kind": kind}, True)
        check({"kind": kind, "watches": None}, True)
        for size in range(1, 9):
            check({"kind": kind, "watches": handles[:size]}, True)
        for permutation in itertools.permutations(handles[:3]):
            check({"kind": kind, "watches": list(permutation)}, True)
        for invalid in (
            [], handles, [handles[0], handles[0]], "", {}, 0, True,
            [None], [1], [{}], [[handles[0]]], [""],
            ["watch:" + "a" * 63], ["watch:" + "a" * 65],
            ["watch:" + "A" * 64], ["watch:" + "g" * 64],
            ["Watch:" + "a" * 64], [" watch:" + "a" * 64],
            ["watch:" + "a" * 64 + "\n"], ["watch:" + "a" * 63 + "\n"],
            ["watch:" + "\0" * 64], ["watch:" + "\u00e9" * 64],
        ):
            check({"kind": kind, "watches": invalid}, False)
        for field, value in (("watch", handles[0]), ("limit", 1),
                             ("continuation", None), ("background", True),
                             ("max_captures", 2), ("path", "/tmp/other")):
            check({"kind": kind, field: value}, False)
    for invalid in (None, [], {}, {"watches": handles[:1]},
                    {"kind": "poll_watch"}, {"kind": "await_watch"},
                    {"kind": "watch"}, {"kind": 1}):
        check(invalid, False)
    source = Path(__file__).read_bytes()
    return {
        "status": "passed",
        "scope": "watch-batch request schema only",
        "cases": sum(counts.values()), **counts,
        "schema_sha256": hashlib.sha256(schema_bytes).hexdigest(),
        "schema_git_blob": blob_sha(schema_bytes),
        "script_sha256": hashlib.sha256(source).hexdigest(),
        "script_git_blob": blob_sha(source),
        "rust_compiled": False, "rust_tests_executed": False,
        "mcp_executed": False, "native_or_live_qualification": False,
        "limitations": "No watch state, authority, custody, capture, durability or response semantics are executed.",
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--schema", type=Path,
                        default=Path(__file__).resolve().parents[1] / "schemas/mcp_watch_batch_v1.json")
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    report = json.dumps(run(args.schema), indent=2, sort_keys=True) + "\n"
    if args.report is not None:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(report, encoding="utf-8")
    print(report, end="")


if __name__ == "__main__":
    main()
