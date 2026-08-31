#!/usr/bin/env python3
"""Verify one bounded R2-R5 live-read acceptance evidence stream."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_CONTRACT = ROOT / "architecture/live_read_acceptance_v1.json"
HEX40 = re.compile(r"^[0-9a-f]{40}$")
HEX64 = re.compile(r"^[0-9a-f]{64}$")
EVENT_ID = re.compile(r"^[a-z0-9][a-z0-9._:-]{0,127}$")
DECIMAL_ID = re.compile(r"^[1-9][0-9]{0,19}$")


class VerificationError(ValueError):
    pass


@dataclass(frozen=True)
class VerificationOptions:
    source_root: Path
    expected_dfmcp_commit: str | None = None
    native_build_receipt: Path | None = None
    allow_synthetic: bool = False
    allow_dirty_development: bool = False


def fail(message: str) -> None:
    raise VerificationError(message)


def canonical_json(value: Any) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode(
        "utf-8"
    )


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def read_bounded_json(path: Path, maximum_bytes: int, label: str) -> dict[str, Any]:
    try:
        size = path.stat().st_size
    except OSError as exc:
        fail(f"cannot stat {label}: {exc}")
    if size <= 0 or size > maximum_bytes:
        fail(f"{label} must contain 1..={maximum_bytes} bytes, got {size}")
    try:
        raw = path.read_bytes()
    except OSError as exc:
        fail(f"cannot read {label}: {exc}")
    try:
        value = json.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        fail(f"cannot parse {label}: {exc}")
    if not isinstance(value, dict):
        fail(f"{label} must be a JSON object")
    return value


def bounded_tree(value: Any, limits: dict[str, int], path: str = "$", depth: int = 1) -> None:
    if depth > limits["maximum_depth"]:
        fail(f"{path} exceeds maximum JSON depth")
    if isinstance(value, str):
        encoded = value.encode("utf-8")
        if len(encoded) > limits["maximum_string_bytes"]:
            fail(f"{path} exceeds the string byte bound")
        if any(ord(character) < 0x20 and character not in "\t\n\r" for character in value):
            fail(f"{path} contains a forbidden control character")
        return
    if isinstance(value, list):
        if len(value) > limits["maximum_collection_items"]:
            fail(f"{path} exceeds the collection bound")
        for index, item in enumerate(value):
            bounded_tree(item, limits, f"{path}[{index}]", depth + 1)
        return
    if isinstance(value, dict):
        if len(value) > limits["maximum_collection_items"]:
            fail(f"{path} exceeds the collection bound")
        for key, item in value.items():
            if not isinstance(key, str):
                fail(f"{path} has a non-string object key")
            bounded_tree(key, limits, f"{path}.<key>", depth + 1)
            bounded_tree(item, limits, f"{path}.{key}", depth + 1)
        return
    if value is None or isinstance(value, (bool, int)):
        return
    if isinstance(value, float):
        fail(f"{path} contains a noncanonical floating-point value")
    fail(f"{path} contains unsupported JSON value {type(value).__name__}")


def require_object(value: Any, path: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        fail(f"{path} must be an object")
    return value


def require_list(value: Any, path: str) -> list[Any]:
    if not isinstance(value, list):
        fail(f"{path} must be an array")
    return value


def require_string(value: Any, path: str, maximum: int = 4096) -> str:
    if not isinstance(value, str) or not value or len(value.encode("utf-8")) > maximum:
        fail(f"{path} must contain 1..={maximum} UTF-8 bytes")
    return value


def require_bool(value: Any, path: str) -> bool:
    if not isinstance(value, bool):
        fail(f"{path} must be Boolean")
    return value


def require_int(value: Any, path: str, minimum: int = 0, maximum: int | None = None) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < minimum:
        fail(f"{path} must be an integer >= {minimum}")
    if maximum is not None and value > maximum:
        fail(f"{path} must be <= {maximum}")
    return value


def require_hash(value: Any, path: str) -> str:
    text = require_string(value, path, 64)
    if HEX64.fullmatch(text) is None:
        fail(f"{path} must be a lowercase SHA-256 digest")
    return text


def require_commit(value: Any, path: str) -> str:
    text = require_string(value, path, 40)
    if HEX40.fullmatch(text) is None:
        fail(f"{path} must be a lowercase 40-character Git commit")
    return text


def require_anchor(value: Any, path: str) -> dict[str, Any]:
    anchor = require_object(value, path)
    required = {"fortress_id", "epoch", "sequence", "game_tick", "state_hash"}
    if set(anchor) != required:
        fail(f"{path} must contain exactly {sorted(required)}")
    fortress_id = require_string(anchor["fortress_id"], f"{path}.fortress_id", 20)
    if DECIMAL_ID.fullmatch(fortress_id) is None:
        fail(f"{path}.fortress_id must be a nonzero decimal u64")
    require_int(anchor["epoch"], f"{path}.epoch")
    require_int(anchor["sequence"], f"{path}.sequence")
    require_int(anchor["game_tick"], f"{path}.game_tick")
    require_hash(anchor["state_hash"], f"{path}.state_hash")
    return anchor


def load_contract(path: Path) -> dict[str, Any]:
    contract = read_bounded_json(path, 1024 * 1024, "acceptance contract")
    if contract.get("schema_version") != "dfmcp.live-read-acceptance/1":
        fail("acceptance contract schema version is unsupported")
    if contract.get("event_schema") != "dfmcp.live-read-acceptance-event/1":
        fail("acceptance event schema drifted")
    if contract.get("receipt_schema") != "dfmcp.live-read-acceptance-receipt/1":
        fail("acceptance receipt schema drifted")
    limits = require_object(contract.get("limits"), "contract.limits")
    for name in [
        "maximum_stream_bytes",
        "maximum_event_bytes",
        "maximum_events",
        "maximum_string_bytes",
        "maximum_collection_items",
        "maximum_depth",
    ]:
        require_int(limits.get(name), f"contract.limits.{name}", 1)
    gates = require_object(contract.get("gates"), "contract.gates")
    order = require_list(contract.get("gate_order"), "contract.gate_order")
    if order != ["R2", "R3", "R4", "R5"] or set(gates) != set(order):
        fail("acceptance gate order or set drifted")
    for gate in order:
        cases = require_list(gates[gate].get("required_cases"), f"contract.gates.{gate}.required_cases")
        names: set[str] = set()
        for index, case in enumerate(cases):
            item = require_object(case, f"contract.gates.{gate}.required_cases[{index}]")
            name = require_string(item.get("case"), f"contract.gates.{gate}.case")
            if name in names:
                fail(f"acceptance contract repeats case {gate}/{name}")
            names.add(name)
            if item.get("result") not in {"accepted", "rejected", "passed"}:
                fail(f"acceptance contract has invalid result for {gate}/{name}")
    return contract


def read_events(path: Path, contract: dict[str, Any]) -> tuple[bytes, list[dict[str, Any]]]:
    limits = contract["limits"]
    try:
        size = path.stat().st_size
    except OSError as exc:
        fail(f"cannot stat evidence stream: {exc}")
    if size <= 0 or size > limits["maximum_stream_bytes"]:
        fail(
            f"evidence stream must contain 1..={limits['maximum_stream_bytes']} bytes, got {size}"
        )
    try:
        raw = path.read_bytes()
    except OSError as exc:
        fail(f"cannot read evidence stream: {exc}")
    events: list[dict[str, Any]] = []
    seen_ids: set[str] = set()
    for line_number, line in enumerate(raw.splitlines(), 1):
        if not line:
            fail(f"evidence stream contains a blank line at {line_number}")
        if len(line) > limits["maximum_event_bytes"]:
            fail(f"evidence event line {line_number} exceeds its byte bound")
        try:
            event = json.loads(line.decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError) as exc:
            fail(f"cannot parse evidence event line {line_number}: {exc}")
        event = require_object(event, f"event[{line_number}]")
        bounded_tree(event, limits, f"event[{line_number}]")
        event_id = require_string(event.get("event_id"), f"event[{line_number}].event_id", 128)
        if EVENT_ID.fullmatch(event_id) is None:
            fail(f"event[{line_number}].event_id has an invalid format")
        if event_id in seen_ids:
            fail(f"evidence stream repeats event_id {event_id}")
        seen_ids.add(event_id)
        events.append(event)
        if len(events) > limits["maximum_events"]:
            fail("evidence stream exceeds the event-count bound")
    if not events:
        fail("evidence stream contains no events")
    return raw, events


def reject_secret_material(raw: bytes, events: list[dict[str, Any]], contract: dict[str, Any]) -> None:
    forbidden = require_object(contract.get("forbidden_event_material"), "contract.forbidden_event_material")
    forbidden_keys = set(require_list(forbidden.get("keys"), "contract.forbidden_event_material.keys"))
    forbidden_substrings = require_list(
        forbidden.get("substrings"), "contract.forbidden_event_material.substrings"
    )
    for substring in forbidden_substrings:
        needle = require_string(substring, "contract.forbidden_event_material.substring").encode()
        if needle in raw:
            fail(f"evidence stream contains forbidden secret material marker {substring!r}")

    def walk(value: Any, path: str) -> None:
        if isinstance(value, dict):
            for key, child in value.items():
                if key.lower() in forbidden_keys:
                    fail(f"{path}.{key} uses a forbidden secret-bearing key")
                walk(child, f"{path}.{key}")
        elif isinstance(value, list):
            for index, child in enumerate(value):
                walk(child, f"{path}[{index}]")

    for index, event in enumerate(events):
        walk(event, f"event[{index}]")


def expected_sequence(contract: dict[str, Any]) -> list[tuple[str, str]]:
    sequence = [("meta", "manifest")]
    for gate in contract["gate_order"]:
        for item in contract["gates"][gate]["required_cases"]:
            sequence.append((gate, item["case"]))
    return sequence


def validate_event_sequence(events: list[dict[str, Any]], contract: dict[str, Any]) -> None:
    expected = expected_sequence(contract)
    actual = [(event.get("gate"), event.get("case")) for event in events]
    if actual != expected:
        fail(f"evidence event order/cases differ from the normative sequence: expected {expected}, got {actual}")
    for index, event in enumerate(events):
        if event.get("schema") != contract["event_schema"]:
            fail(f"event[{index}].schema drifted")
        result = event.get("result")
        if result not in {"accepted", "rejected", "passed"}:
            fail(f"event[{index}].result is invalid")
        if index == 0:
            if event.get("event_id") != "manifest" or result != "passed":
                fail("the first event must be the passed manifest event")
            continue
        gate = event["gate"]
        case = event["case"]
        expected_item = next(
            item for item in contract["gates"][gate]["required_cases"] if item["case"] == case
        )
        if result != expected_item["result"]:
            fail(f"{gate}/{case} result must be {expected_item['result']}")
        expected_error = expected_item["error_code"]
        actual_error = event.get("error_code")
        if actual_error != expected_error:
            fail(f"{gate}/{case} error_code must be {expected_error!r}, got {actual_error!r}")


def validate_manifest(
    event: dict[str, Any], contract: dict[str, Any], options: VerificationOptions
) -> dict[str, Any]:
    run_id = require_string(event.get("run_id"), "manifest.run_id", 128)
    synthetic = require_bool(event.get("synthetic"), "manifest.synthetic")
    if synthetic and not options.allow_synthetic:
        fail("synthetic evidence is rejected unless --allow-synthetic is explicit")
    if not synthetic and options.expected_dfmcp_commit is None:
        fail("non-synthetic qualification requires --expected-dfmcp-commit")
    source = require_object(event.get("source"), "manifest.source")
    dfmcp_commit = require_commit(source.get("dfmcp_commit"), "manifest.source.dfmcp_commit")
    dirty = require_bool(source.get("dfmcp_dirty"), "manifest.source.dfmcp_dirty")
    if options.expected_dfmcp_commit is not None:
        expected = require_commit(options.expected_dfmcp_commit, "expected_dfmcp_commit")
        if dfmcp_commit != expected:
            fail(f"manifest commit {dfmcp_commit} does not match expected commit {expected}")
    if dirty and not options.allow_dirty_development:
        fail("dirty source cannot produce qualified live-read evidence")
    dfhack_commit = require_commit(source.get("dfhack_commit"), "manifest.source.dfhack_commit")
    plugin_sha256 = require_hash(source.get("plugin_sha256"), "manifest.source.plugin_sha256")
    native_receipt_sha256 = require_hash(
        source.get("native_build_receipt_sha256"), "manifest.source.native_build_receipt_sha256"
    )
    versions = {
        "dwarf_fortress_version": require_string(
            source.get("dwarf_fortress_version"), "manifest.source.dwarf_fortress_version", 128
        ),
        "dfhack_version": require_string(source.get("dfhack_version"), "manifest.source.dfhack_version", 128),
        "bridge_version": require_string(source.get("bridge_version"), "manifest.source.bridge_version", 128),
        "bridge_protocol": require_string(source.get("bridge_protocol"), "manifest.source.bridge_protocol", 16),
    }
    if versions["bridge_protocol"] != "1.0":
        fail("manifest bridge protocol must be exactly 1.0")
    host = require_object(event.get("host"), "manifest.host")
    require_string(host.get("system"), "manifest.host.system", 128)
    require_string(host.get("machine"), "manifest.host.machine", 128)
    source_digests = require_object(event.get("source_digests"), "manifest.source_digests")
    required_digests = contract["source_binding"]["required_source_digests"]
    if set(source_digests) != set(required_digests):
        fail("manifest source_digests keys differ from the normative source binding")
    for name, relative in required_digests.items():
        declared = require_hash(source_digests.get(name), f"manifest.source_digests.{name}")
        path = options.source_root / relative
        if not path.is_file():
            fail(f"required source file is missing: {relative}")
        actual = sha256_file(path)
        if declared != actual:
            fail(f"source digest mismatch for {relative}: declared {declared}, actual {actual}")
    if not synthetic:
        if options.native_build_receipt is None:
            fail("non-synthetic qualification requires --native-build-receipt")
        validate_native_build_receipt(
            options.native_build_receipt,
            native_receipt_sha256,
            dfmcp_commit,
            dfhack_commit,
            plugin_sha256,
        )
    return {
        "run_id": run_id,
        "synthetic": synthetic,
        "dfmcp_commit": dfmcp_commit,
        "dfmcp_dirty": dirty,
        "dfhack_commit": dfhack_commit,
        "plugin_sha256": plugin_sha256,
        "native_build_receipt_sha256": native_receipt_sha256,
        **versions,
        "host": host,
        "source_digests": source_digests,
    }


def validate_native_build_receipt(
    path: Path,
    expected_digest: str,
    dfmcp_commit: str,
    dfhack_commit: str,
    plugin_sha256: str,
) -> None:
    actual_digest = sha256_file(path)
    if actual_digest != expected_digest:
        fail("native build receipt digest does not match the manifest")
    receipt = read_bounded_json(path, 4 * 1024 * 1024, "native build receipt")
    if receipt.get("schema") != "dfmcp.dfhack-plugin-qualification/1":
        fail("native build receipt schema is unsupported")
    if receipt.get("status") != "native-build-passed":
        fail("native build receipt is not a passing R1 receipt")
    source = require_object(receipt.get("source"), "native_build_receipt.source")
    if source.get("dfmcp_commit") != dfmcp_commit or source.get("dfhack_commit") != dfhack_commit:
        fail("native build receipt source commits do not match the live evidence manifest")
    if source.get("dfmcp_dirty") is not False:
        fail("native build receipt is not bound to a clean dfmcp source")
    plugin = require_object(receipt.get("plugin"), "native_build_receipt.plugin")
    if plugin.get("sha256") != plugin_sha256:
        fail("native build receipt plugin digest does not match the live evidence manifest")
    if plugin.get("rpc_methods") != ["Handshake", "ReadObservation"]:
        fail("native build receipt RPC method set drifted")
    if plugin.get("mutation_rpc_methods") != []:
        fail("native build receipt contains mutation RPC methods")


def event_map(events: list[dict[str, Any]], gate: str) -> dict[str, dict[str, Any]]:
    return {event["case"]: event for event in events if event["gate"] == gate}


def validate_rejection_non_disclosure(event: dict[str, Any], label: str) -> None:
    if event.get("sensitive_manifest_disclosed") is not False:
        fail(f"{label} must prove that the sensitive manifest was not disclosed")
    if event.get("bridge_generation") != 0:
        fail(f"{label} must report neutral bridge_generation 0")
    if event.get("supported_methods") != []:
        fail(f"{label} must report no supported methods")
    if event.get("world_loaded") is not False or event.get("fortress_mode") is not False:
        fail(f"{label} must report neutral world posture")


def validate_r2(events: list[dict[str, Any]], manifest: dict[str, Any]) -> None:
    cases = event_map(events, "R2")
    for name in [
        "missing_token",
        "configured_token_short",
        "configured_token_long",
        "presented_token_short",
        "presented_token_long",
        "wrong_token",
        "nonce_short",
        "nonce_long",
        "protocol_mismatch",
    ]:
        validate_rejection_non_disclosure(cases[name], f"R2/{name}")
    correct = cases["correct_token"]
    if correct.get("protocol_major") != 1 or correct.get("protocol_minor") != 0:
        fail("R2/correct_token returned the wrong protocol version")
    require_int(correct.get("bridge_generation"), "R2.correct_token.bridge_generation", 1)
    if correct.get("supported_methods") != ["Handshake", "ReadObservation"]:
        fail("R2/correct_token returned the wrong method set")
    if correct.get("nonce_correlated") is not True:
        fail("R2/correct_token did not prove nonce correlation")
    if correct.get("world_loaded") is not True or correct.get("fortress_mode") is not True:
        fail("R2/correct_token did not observe a loaded fortress-mode world")
    for name in ["dwarf_fortress_version", "dfhack_version", "bridge_version"]:
        if correct.get(name) != manifest[name]:
            fail(f"R2/correct_token {name} differs from the manifest")
    mismatch = cases["nonce_mismatch"]
    if mismatch.get("nonce_correlated") is not False or mismatch.get("published") is not False:
        fail("R2/nonce_mismatch must fail correlation without publication")
    scan = cases["secret_scan"]
    require_string(scan.get("scanner"), "R2.secret_scan.scanner", 256)
    require_hash(scan.get("token_fingerprint_sha256"), "R2.secret_scan.token_fingerprint_sha256")
    if require_int(scan.get("match_count"), "R2.secret_scan.match_count") != 0:
        fail("R2/secret_scan found secret material")
    artifacts = require_list(scan.get("scanned_artifacts"), "R2.secret_scan.scanned_artifacts")
    if not artifacts:
        fail("R2/secret_scan must bind at least one scanned artifact")
    paths: set[str] = set()
    for index, item in enumerate(artifacts):
        artifact = require_object(item, f"R2.secret_scan.scanned_artifacts[{index}]")
        path = require_string(artifact.get("path"), f"R2.secret_scan.artifact[{index}].path", 1024)
        if os.path.isabs(path) or ".." in Path(path).parts:
            fail("R2/secret_scan artifact paths must be relative and traversal-free")
        if path in paths:
            fail(f"R2/secret_scan repeats artifact path {path}")
        paths.add(path)
        require_hash(artifact.get("sha256"), f"R2.secret_scan.artifact[{index}].sha256")


def validate_observation(event: dict[str, Any], label: str, names_included: bool) -> dict[str, Any]:
    if event.get("paused") is not True:
        fail(f"{label} must be a paused-world observation")
    if event.get("names_included") is not names_included:
        fail(f"{label} has the wrong name projection")
    page_size = require_int(event.get("page_size"), f"{label}.page_size", 1, 4096)
    citizen_count = require_int(event.get("citizen_count"), f"{label}.citizen_count")
    page_count = require_int(event.get("page_count"), f"{label}.page_count", 1)
    expected_pages = max(1, (citizen_count + page_size - 1) // page_size)
    if page_count != expected_pages:
        fail(f"{label} page_count {page_count} does not equal canonical {expected_pages}")
    if event.get("complete") is not True or event.get("publication_count") != 1:
        fail(f"{label} must publish exactly one complete capsule")
    bridge_generation = require_int(event.get("bridge_generation"), f"{label}.bridge_generation", 1)
    return {
        "page_size": page_size,
        "citizen_count": citizen_count,
        "page_count": page_count,
        "bridge_generation": bridge_generation,
        "capsule_sha256": require_hash(event.get("capsule_sha256"), f"{label}.capsule_sha256"),
        "snapshot_sha256": require_hash(event.get("snapshot_sha256"), f"{label}.snapshot_sha256"),
        "citizen_identity_sha256": require_hash(
            event.get("citizen_identity_sha256"), f"{label}.citizen_identity_sha256"
        ),
        "anchor": require_anchor(event.get("anchor"), f"{label}.anchor"),
    }


def compare_observation(reference: dict[str, Any], candidate: dict[str, Any], label: str) -> None:
    for name in [
        "citizen_count",
        "bridge_generation",
        "capsule_sha256",
        "snapshot_sha256",
        "citizen_identity_sha256",
        "anchor",
    ]:
        if candidate[name] != reference[name]:
            fail(f"{label} differs from the deterministic baseline in {name}")


def validate_r3(events: list[dict[str, Any]], contract: dict[str, Any]) -> dict[str, Any]:
    cases = event_map(events, "R3")
    included = validate_observation(cases["baseline_names_included"], "R3/baseline_names_included", True)
    repeat = validate_observation(cases["repeat_names_included"], "R3/repeat_names_included", True)
    compare_observation(included, repeat, "R3/repeat_names_included")
    for size in contract["gates"]["R3"]["page_sizes"]:
        name = f"page_size_{size}"
        candidate = validate_observation(cases[name], f"R3/{name}", True)
        if candidate["page_size"] != size:
            fail(f"R3/{name} did not use page size {size}")
        compare_observation(included, candidate, f"R3/{name}")
    omitted = validate_observation(cases["baseline_names_omitted"], "R3/baseline_names_omitted", False)
    omitted_repeat = validate_observation(cases["repeat_names_omitted"], "R3/repeat_names_omitted", False)
    compare_observation(omitted, omitted_repeat, "R3/repeat_names_omitted")
    if omitted["citizen_identity_sha256"] != included["citizen_identity_sha256"]:
        fail("R3 name projections do not preserve citizen identity")
    if omitted["capsule_sha256"] == included["capsule_sha256"]:
        fail("R3 included and omitted name projections must have distinct capsule identities")
    if omitted["snapshot_sha256"] == included["snapshot_sha256"]:
        fail("R3 included and omitted name projections must have distinct snapshot identities")
    for name, beyond in [("offset_at_total", False), ("offset_beyond_total", True)]:
        event = cases[name]
        total = require_int(event.get("citizen_count"), f"R3/{name}.citizen_count")
        requested = require_int(event.get("requested_offset"), f"R3/{name}.requested_offset")
        canonical = require_int(event.get("canonical_offset"), f"R3/{name}.canonical_offset")
        if (requested > total) is not beyond:
            fail(f"R3/{name} has the wrong requested-offset relation")
        if canonical != total or event.get("returned_citizens") != 0 or event.get("complete") is not True:
            fail(f"R3/{name} must yield the canonical empty terminal page")
    running = cases["running_multipage_rejected"]
    if running.get("paused") is not False or running.get("published") is not False:
        fail("R3/running_multipage_rejected must fail without publication")
    if require_int(running.get("pages_attempted"), "R3.running_multipage_rejected.pages_attempted", 1) != 1:
        fail("R3/running_multipage_rejected must abort after the first nonterminal page")
    return included


def validate_r4(events: list[dict[str, Any]]) -> dict[str, Any]:
    cases = event_map(events, "R4")
    restart = cases["restart_generation_changed"]
    before_generation = require_int(restart.get("before_generation"), "R4.restart.before_generation", 1)
    after_generation = require_int(restart.get("after_generation"), "R4.restart.after_generation", 1)
    if before_generation == after_generation:
        fail("R4 restart did not change bridge generation")
    before_anchor = require_anchor(restart.get("before_anchor"), "R4.restart.before_anchor")
    after_anchor = require_anchor(restart.get("after_anchor"), "R4.restart.after_anchor")
    if before_anchor["fortress_id"] != after_anchor["fortress_id"]:
        fail("R4 restart changed fortress lineage")
    if after_anchor["epoch"] != before_anchor["epoch"] + 1 or after_anchor["sequence"] != 0:
        fail("R4 restart did not begin exactly one fresh epoch at sequence zero")
    old = cases["old_client_rejected"]
    if old.get("expected_generation") != before_generation or old.get("observed_generation") != after_generation:
        fail("R4 old-client rejection is not bound to the restart generations")
    if old.get("published") is not False:
        fail("R4 old client published state after a restart")
    for name in ["world_unloaded", "non_fortress_mode", "summary_drift"]:
        if cases[name].get("published") is not False:
            fail(f"R4/{name} published rejected state")
    partial = cases["partial_not_published"]
    require_int(partial.get("pages_received"), "R4.partial.pages_received", 1)
    if partial.get("complete") is not False or partial.get("published") is not False:
        fail("R4 partial capsule was published")
    if partial.get("canonical_anchor_issued") is not False:
        fail("R4 partial capsule issued a canonical anchor")
    fresh = cases["fresh_handshake"]
    if fresh.get("bridge_generation") != after_generation:
        fail("R4 fresh handshake did not bind the restarted generation")
    if fresh.get("supported_methods") != ["Handshake", "ReadObservation"]:
        fail("R4 fresh handshake method set drifted")
    return {
        "before_generation": before_generation,
        "after_generation": after_generation,
        "before_anchor": before_anchor,
        "after_anchor": after_anchor,
    }


def validate_r5(
    events: list[dict[str, Any]], contract: dict[str, Any], manifest: dict[str, Any], baseline: dict[str, Any]
) -> None:
    event = event_map(events, "R5")["cold_agent_turn"]
    anchor = require_anchor(event.get("anchor"), "R5.cold_agent_turn.anchor")
    if anchor != baseline["anchor"]:
        fail("R5 cold Agent Turn anchor differs from the accepted R3 baseline")
    if event.get("capsule_sha256") != baseline["capsule_sha256"]:
        fail("R5 cold Agent Turn cites a different capsule than R3")
    require_hash(event.get("receipt_sha256"), "R5.cold_agent_turn.receipt_sha256")
    source = require_object(event.get("source"), "R5.cold_agent_turn.source")
    for name in ["dwarf_fortress_version", "dfhack_version", "bridge_version"]:
        if source.get(name) != manifest[name]:
            fail(f"R5 cold Agent Turn source {name} differs from the manifest")
    if event.get("authority") != "read_only":
        fail("R5 cold Agent Turn must declare read_only authority")
    if event.get("continuity") not in {"bootstrap", "continuous", "heartbeat", "reset"}:
        fail("R5 cold Agent Turn continuity is not complete and explicit")
    summary = require_object(event.get("summary"), "R5.cold_agent_turn.summary")
    if summary.get("paused") is not True:
        fail("R5 cold Agent Turn does not report the paused baseline")
    require_int(summary.get("current_year"), "R5.summary.current_year")
    require_int(summary.get("current_year_tick"), "R5.summary.current_year_tick", 0, 403199)
    require_int(summary.get("site_id"), "R5.summary.site_id")
    if require_int(summary.get("citizen_count"), "R5.summary.citizen_count") != baseline["citizen_count"]:
        fail("R5 citizen summary differs from R3 complete coverage")
    if event.get("citizen_drilldown_bounded") is not True:
        fail("R5 cold Agent Turn lacks bounded citizen drill-down")
    for name in ["mutation_capabilities", "mutation_affordances", "mutation_recommendations"]:
        if event.get(name) != []:
            fail(f"R5 cold Agent Turn exposes forbidden {name}")
    coverage = require_list(event.get("coverage"), "R5.cold_agent_turn.coverage")
    domains: dict[str, dict[str, Any]] = {}
    for index, item in enumerate(coverage):
        entry = require_object(item, f"R5.coverage[{index}]")
        domain = require_string(entry.get("domain"), f"R5.coverage[{index}].domain", 256)
        if domain in domains:
            fail(f"R5 coverage repeats domain {domain}")
        domains[domain] = entry
    roster = domains.get("fortress.citizens.roster")
    if roster is None:
        fail("R5 coverage omits fortress.citizens.roster")
    if roster.get("status") != "complete" or roster.get("can_prove_absence") is not True:
        fail("R5 citizen roster coverage is not complete")
    if roster.get("anchor_state_hash") != anchor["state_hash"]:
        fail("R5 citizen roster coverage is not bound to the turn anchor")
    for domain in contract["gates"]["R5"]["required_omitted_domains"]:
        entry = domains.get(domain)
        if entry is None:
            fail(f"R5 coverage omits required unknown domain {domain}")
        if (
            entry.get("status") != "omitted"
            or entry.get("epistemic_state") != "unknown"
            or entry.get("can_prove_absence") is not False
        ):
            fail(f"R5 omitted domain {domain} overstates knowledge")


def build_receipt(
    raw: bytes,
    events: list[dict[str, Any]],
    contract: dict[str, Any],
    manifest: dict[str, Any],
    options: VerificationOptions,
) -> dict[str, Any]:
    gates = []
    for gate in contract["gate_order"]:
        gate_events = [event for event in events if event["gate"] == gate]
        gates.append(
            {
                "gate": gate,
                "status": "passed",
                "case_count": len(gate_events),
                "evidence_digest": sha256_bytes(canonical_json(gate_events)),
            }
        )
    if manifest["synthetic"]:
        status = "synthetic-contract-fixture"
    elif manifest["dfmcp_dirty"] or options.allow_dirty_development:
        status = "development-evidence"
    else:
        status = "qualified"
    receipt: dict[str, Any] = {
        "schema": contract["receipt_schema"],
        "status": status,
        "run_id": manifest["run_id"],
        "source": {
            "dfmcp_commit": manifest["dfmcp_commit"],
            "dfmcp_dirty": manifest["dfmcp_dirty"],
            "dfhack_commit": manifest["dfhack_commit"],
            "plugin_sha256": manifest["plugin_sha256"],
            "native_build_receipt_sha256": manifest["native_build_receipt_sha256"],
            "source_digests": manifest["source_digests"],
        },
        "version_tuple": {
            "dwarf_fortress": manifest["dwarf_fortress_version"],
            "dfhack": manifest["dfhack_version"],
            "bridge": manifest["bridge_version"],
            "protocol": manifest["bridge_protocol"],
        },
        "host": manifest["host"],
        "evidence": {
            "stream_sha256": sha256_bytes(raw),
            "event_count": len(events),
            "canonical_events_sha256": sha256_bytes(canonical_json(events)),
        },
        "gates": gates,
        "claims_established": [
            "R2 authenticated handshake matrix and bounded secret-scan evidence",
            "R3 paused-world deterministic capsule identity across the required pagination matrix",
            "R4 restart, world-mode, drift, and partial-publication fencing",
            "R5 cold-agent read-only orientation with explicit omitted-domain uncertainty",
        ],
        "claims_not_established": contract["claims_not_established_by_this_contract"],
    }
    receipt["receipt_digest"] = sha256_bytes(canonical_json(receipt))
    return receipt


def verify_acceptance(
    evidence_path: Path,
    contract_path: Path,
    options: VerificationOptions,
) -> dict[str, Any]:
    contract = load_contract(contract_path)
    raw, events = read_events(evidence_path, contract)
    reject_secret_material(raw, events, contract)
    validate_event_sequence(events, contract)
    manifest = validate_manifest(events[0], contract, options)
    validate_r2(events, manifest)
    baseline = validate_r3(events, contract)
    validate_r4(events)
    validate_r5(events, contract, manifest, baseline)
    return build_receipt(raw, events, contract, manifest, options)


def write_atomic(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    content = json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False) + "\n"
    descriptor, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8", newline="\n") as handle:
            handle.write(content)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
    except BaseException:
        try:
            os.unlink(temporary)
        except OSError:
            pass
        raise


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("evidence", type=Path, help="ordered R2-R5 JSONL evidence stream")
    parser.add_argument("--contract", type=Path, default=DEFAULT_CONTRACT)
    parser.add_argument("--source-root", type=Path, default=ROOT)
    parser.add_argument("--native-build-receipt", type=Path)
    parser.add_argument("--expected-dfmcp-commit")
    parser.add_argument("--receipt", type=Path)
    parser.add_argument("--allow-synthetic", action="store_true")
    parser.add_argument("--allow-dirty-development", action="store_true")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    receipt_path = args.receipt or args.evidence.with_suffix(".receipt.json")
    try:
        receipt = verify_acceptance(
            args.evidence,
            args.contract,
            VerificationOptions(
                source_root=args.source_root,
                expected_dfmcp_commit=args.expected_dfmcp_commit,
                native_build_receipt=args.native_build_receipt,
                allow_synthetic=args.allow_synthetic,
                allow_dirty_development=args.allow_dirty_development,
            ),
        )
        write_atomic(receipt_path, receipt)
    except VerificationError as exc:
        print(f"live read acceptance: FAIL: {exc}", file=sys.stderr)
        return 1
    except OSError as exc:
        print(f"live read acceptance: FAIL: {exc}", file=sys.stderr)
        return 1
    print(
        f"live read acceptance: PASS ({receipt['status']}, {receipt['evidence']['event_count']} events)"
    )
    print(receipt_path)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
