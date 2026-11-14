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
    v2_rejected = check_task_bundle_schema(golden)
    print(f"replay bundle schemas: v1 golden valid, {len(rejected)} legacy and {v2_rejected} Task-event mutations rejected")


def check_task_bundle_schema(golden):
    """Synthetic shape fixtures are not execution or live-game evidence."""
    schema = json.loads((ROOT / "schemas/replay.bundle.v2.schema.json").read_text())
    Draft202012Validator.check_schema(schema)
    validator = Draft202012Validator(schema)
    bundle = copy.deepcopy(golden)
    bundle["schema"] = "dfmcp.replay.bundle/2"
    for index, call in enumerate(bundle["calls"]):
        call["next_request_after"] = str(index + 1)
    anchor = copy.deepcopy(bundle["calls"][-1]["anchor_after"])
    frontier = {
        "anchor": anchor,
        "next_request_id": "9",
        "authority_digest": "12" * 32,
        "original_actions_digest": "34" * 32,
        "budget": {
            "max_wall_millis": 2000, "max_game_ticks": 10000,
            "max_entities": 2000, "max_bytes": 4 * 1024 * 1024,
            "max_output_tokens": 1500, "max_actions": 64,
        },
    }
    for phase in ["request", "finalize"]:
        event = {
            "seq": len(bundle["calls"]),
            "event": "task_cancel_phase",
            "arguments": {"plan_digest": "56" * 32, "phase": phase},
            "before": copy.deepcopy(frontier),
            "after": copy.deepcopy(frontier),
            "ok": True, "error_code": None, "evidence_digest": "78" * 32,
        }
        bundle["calls"].append(event)

    def v2_digest(calls):
        compact = json.dumps(calls, separators=(",", ":"), ensure_ascii=False, sort_keys=True)
        return hashlib.sha256(b"dfmcp-replay-bundle-calls/2\0" + compact.encode()).hexdigest()

    bundle["calls_digest"] = v2_digest(bundle["calls"])
    assert validator.is_valid(bundle), list(validator.iter_errors(bundle))
    assert bundle["calls_digest"] != calls_digest(bundle["calls"]), "version domains must differ"

    def changed(edit):
        value = copy.deepcopy(bundle)
        edit(value)
        return value

    mutations = [
        changed(lambda b: b.update(schema="dfmcp.replay.bundle/3")),
        changed(lambda b: b["calls"][0].pop("next_request_after")),
        changed(lambda b: b["calls"][-1].update(event="native_command")),
        changed(lambda b: b["calls"][-1].update(tool="fortress.cancel")),
        changed(lambda b: b["calls"][-1]["arguments"].update(phase="dispatch")),
        changed(lambda b: b["calls"][-1]["arguments"].update(plan_digest="AB" * 32)),
        changed(lambda b: b["calls"][-1]["arguments"].update(plan_digest="0" * 64)),
        changed(lambda b: b["calls"][-1]["arguments"].update(session_id="other")),
        changed(lambda b: b["calls"][-1]["before"].pop("authority_digest")),
        changed(lambda b: b["calls"][-1]["before"].update(next_request_id="01")),
        changed(lambda b: b["calls"][-1]["after"].update(original_actions_digest="z" * 64)),
        changed(lambda b: b["calls"][-1]["before"]["budget"].update(max_bytes=0)),
        changed(lambda b: b["calls"][-1].pop("evidence_digest")),
        changed(lambda b: b.update(calls=[])),
        changed(lambda b: b.update(calls=[copy.deepcopy(b["calls"][0])] * 4097)),
    ]
    for index, value in enumerate(mutations):
        assert not validator.is_valid(value), f"Task event mutation {index} accepted"
    failed = changed(lambda b: b["calls"][-1].update(ok=False, error_code="capability_denied"))
    assert validator.is_valid(failed), "failed partial phases remain valid evidence"
    old_validator = Draft202012Validator(
        json.loads((ROOT / "schemas/replay.bundle.schema.json").read_text())
    )
    assert not old_validator.is_valid(bundle), "old schema must reject /2"
    assert not validator.is_valid(golden), "new schema must not reinterpret /1"
    for edit in [
        lambda b: b["calls"][-1]["before"]["budget"].update(max_bytes=1),
        lambda b: b["calls"][-1].update(evidence_digest="90" * 32),
        lambda b: b["calls"][-1]["arguments"].update(phase="request"),
    ]:
        value = changed(edit)
        assert v2_digest(value["calls"]) != bundle["calls_digest"]
    return len(mutations)



if __name__ == "__main__":
    sys.exit(main())
