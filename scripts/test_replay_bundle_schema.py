#!/usr/bin/env python3
"""Validate the replay bundle schema against the golden bundle and mutations.

The golden bundle's replay semantics are checked by the Rust test
`replay::tests::the_golden_bundle_still_replays_exactly`; this script checks
only shape and the calls digest.
"""
import copy
import hashlib
import json
import sys
from pathlib import Path

from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[1]


def calls_digest(calls):
    compact = json.dumps(calls, separators=(",", ":"), ensure_ascii=False)
    return hashlib.sha256(b"dfmcp-replay-bundle-calls/1\0" + compact.encode()).hexdigest()


def main():
    schema = json.loads((ROOT / "schemas/replay.bundle.schema.json").read_text())
    Draft202012Validator.check_schema(schema)
    validator = Draft202012Validator(schema)
    golden = json.loads((ROOT / "schemas/examples/replay_bundle_v1.json").read_text())
    errors = list(validator.iter_errors(golden))
    assert not errors, errors
    assert calls_digest(golden["calls"]) == golden["calls_digest"], "golden calls_digest"
    assert [c["seq"] for c in golden["calls"]] == list(range(len(golden["calls"])))

    def mutated(edit):
        bundle = copy.deepcopy(golden)
        edit(bundle)
        return bundle

    rejected = [
        mutated(lambda b: b.update(schema="dfmcp.replay.bundle/2")),
        mutated(lambda b: b.update(calls_digest="0" * 63)),
        mutated(lambda b: b.update(replayable=False)),
        mutated(lambda b: b.update(extra=True)),
        mutated(lambda b: b["calls"][0].update(tool="fortress.shell")),
        mutated(lambda b: b["calls"][0]["arguments"].update(session_id="s")),
        mutated(lambda b: b["calls"][0]["anchor_after"].update(state_hash="XYZ")),
        mutated(lambda b: b["calls"][0]["anchor_after"].update(fortress_id="01")),
    ]
    for index, bundle in enumerate(rejected):
        assert not validator.is_valid(bundle), f"mutation {index} was accepted"
    accepted = mutated(lambda b: b.update(replayable=False, not_replayable_reason="shared fortress"))
    assert validator.is_valid(accepted)
    print(f"replay bundle schema: golden valid, {len(rejected)} mutations rejected")


if __name__ == "__main__":
    sys.exit(main())
