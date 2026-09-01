#!/usr/bin/env python3
"""Verify one exact, secret-free protocol-1.1 announcement evidence stream."""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import re
import sys
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
PROMOTION_PATH = ROOT / "scripts/promote_live_compatibility.py"
NATIVE_GENERATION_PATH = ROOT / "scripts/issue_dfhack_plugin_receipt_v1_1.py"
DEFAULT_CONTRACT = ROOT / "architecture/live_announcement_acceptance_v1_1.json"
DEFAULT_NATIVE_CONTRACT = ROOT / "architecture/dfhack_plugin_native_receipt_v1_1.json"

SPEC = importlib.util.spec_from_file_location("promote_live_compatibility", PROMOTION_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load compatibility receipt primitives")
promotion = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = promotion
SPEC.loader.exec_module(promotion)

NATIVE_SPEC = importlib.util.spec_from_file_location(
    "issue_dfhack_plugin_receipt_v1_1", NATIVE_GENERATION_PATH
)
if NATIVE_SPEC is None or NATIVE_SPEC.loader is None:
    raise RuntimeError("cannot load protocol-1.1 native receipt contract")
native_generation = importlib.util.module_from_spec(NATIVE_SPEC)
sys.modules[NATIVE_SPEC.name] = native_generation
NATIVE_SPEC.loader.exec_module(native_generation)

HEX40 = re.compile(r"^[0-9a-f]{40}$")
HEX64 = re.compile(r"^[0-9a-f]{64}$")
MAX_STRING_BYTES = 64 * 1024
MAX_COLLECTION_ITEMS = 10_000
MAX_DEPTH = 64


class VerificationError(ValueError):
    pass


def fail(message: str) -> None:
    raise VerificationError(message)


def require(condition: bool, message: str) -> None:
    if not condition:
        fail(message)


def canonical_json(value: Any) -> bytes:
    return json.dumps(
        value,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
    ).encode("utf-8")


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def duplicate_rejecting_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    output: dict[str, Any] = {}
    for key, value in pairs:
        if key in output:
            fail(f"JSON object repeats key {key!r}")
        output[key] = value
    return output


def bounded_tree(value: Any, path: str = "$", depth: int = 1) -> None:
    if depth > MAX_DEPTH:
        fail(f"{path} exceeds the maximum JSON depth")
    if value is None or isinstance(value, (bool, int)):
        return
    if isinstance(value, float):
        fail(f"{path} contains a noncanonical floating-point value")
    if isinstance(value, str):
        encoded = value.encode("utf-8")
        if len(encoded) > MAX_STRING_BYTES:
            fail(f"{path} exceeds the string byte bound")
        if any(ord(character) < 0x20 and character not in "\t\n\r" for character in value):
            fail(f"{path} contains a forbidden control character")
        return
    if isinstance(value, list):
        if len(value) > MAX_COLLECTION_ITEMS:
            fail(f"{path} exceeds the array bound")
        for index, item in enumerate(value):
            bounded_tree(item, f"{path}[{index}]", depth + 1)
        return
    if isinstance(value, dict):
        if len(value) > MAX_COLLECTION_ITEMS:
            fail(f"{path} exceeds the object-member bound")
        for key, item in value.items():
            if not isinstance(key, str):
                fail(f"{path} contains a non-string key")
            bounded_tree(key, f"{path}.<key>", depth + 1)
            bounded_tree(item, f"{path}.{key}", depth + 1)
        return
    fail(f"{path} contains unsupported JSON type {type(value).__name__}")


def parse_object(raw: bytes, label: str) -> dict[str, Any]:
    try:
        value = json.loads(
            raw.decode("utf-8"),
            object_pairs_hook=duplicate_rejecting_object,
        )
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        fail(f"cannot parse {label}: {exc}")
    if not isinstance(value, dict):
        fail(f"{label} must be a JSON object")
    bounded_tree(value)
    return value


def require_exact_keys(value: dict[str, Any], expected: set[str], path: str) -> None:
    actual = set(value)
    if actual != expected:
        fail(
            f"{path} fields differ: expected {sorted(expected)}, got {sorted(actual)}"
        )


def require_object(value: Any, path: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        fail(f"{path} must be an object")
    return value


def require_list(value: Any, path: str) -> list[Any]:
    if not isinstance(value, list):
        fail(f"{path} must be an array")
    return value


def require_string(value: Any, path: str, maximum: int = MAX_STRING_BYTES) -> str:
    if not isinstance(value, str) or not value or len(value.encode("utf-8")) > maximum:
        fail(f"{path} must contain 1..={maximum} UTF-8 bytes")
    if any(ord(character) < 0x20 for character in value):
        fail(f"{path} contains a control character")
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


def expected_cases(contract: dict[str, Any]) -> list[tuple[str, dict[str, Any]]]:
    output: list[tuple[str, dict[str, Any]]] = []
    for raw_gate in contract["gates"]:
        gate = require_object(raw_gate, "contract.gates[]")
        require_exact_keys(
            gate,
            {"gate", "name", "cases"},
            f"contract.{gate.get('gate')}",
        )
        gate_name = require_string(gate.get("gate"), "contract.gate", 8)
        for raw_case in require_list(
            gate.get("cases"), f"contract.{gate_name}.cases"
        ):
            case = require_object(raw_case, f"contract.{gate_name}.cases[]")
            require_exact_keys(
                case,
                {"case", "required_equals", "required_artifact_digests"},
                f"contract.{gate_name}.case",
            )
            require_string(case.get("case"), f"contract.{gate_name}.case.case", 128)
            require_object(
                case.get("required_equals"),
                f"contract.{gate_name}.case.required_equals",
            )
            artifact_names = require_list(
                case.get("required_artifact_digests"),
                f"contract.{gate_name}.case.required_artifact_digests",
            )
            if not artifact_names:
                fail(f"contract.{gate_name}.case has no required artifact digest")
            normalized_names = [
                require_string(
                    name,
                    f"contract.{gate_name}.case.required_artifact_digests[]",
                    128,
                )
                for name in artifact_names
            ]
            if len(normalized_names) != len(set(normalized_names)):
                fail(f"contract.{gate_name}.case repeats an artifact name")
            output.append((gate_name, case))
    return output


def load_contract(path: Path) -> dict[str, Any]:
    raw = path.read_bytes()
    if not raw or len(raw) > 4 * 1024 * 1024:
        fail("announcement acceptance contract is empty or oversized")
    contract = parse_object(raw, "announcement acceptance contract")
    require_exact_keys(
        contract,
        {
            "schema_version",
            "event_schema",
            "receipt_schema",
            "status",
            "bridge",
            "limits",
            "authority",
            "source_binding",
            "event_fields",
            "gates",
            "promotion_rule",
            "claims_established",
            "claims_not_established",
        },
        "contract",
    )
    if contract.get("schema_version") != "dfmcp.live_announcement_acceptance/1.1":
        fail("announcement acceptance contract schema is unsupported")
    if contract.get("event_schema") != "dfmcp.live-announcement-evidence/1.1":
        fail("announcement evidence event schema drifted")
    if (
        contract.get("receipt_schema")
        != "dfmcp.live-announcement-acceptance-receipt/1.1"
    ):
        fail("announcement acceptance receipt schema drifted")
    if contract.get("status") != "normative_unexecuted_acceptance_contract":
        fail("announcement acceptance contract status drifted")
    bridge = require_object(contract.get("bridge"), "contract.bridge")
    require_exact_keys(
        bridge,
        {"plugin", "bridge_version", "protocol", "rpc_methods"},
        "contract.bridge",
    )
    if bridge != {
        "plugin": "dfmcp_bridge_v1_1",
        "bridge_version": "0.2.0",
        "protocol": "1.1",
        "rpc_methods": ["Handshake", "ReadObservation"],
    }:
        fail("announcement bridge version or method contract drifted")
    authority = require_object(contract.get("authority"), "contract.authority")
    require_exact_keys(
        authority,
        {"mode", "capabilities", "mutation_capabilities"},
        "contract.authority",
    )
    if authority.get("mode") != "read_only":
        fail("announcement acceptance contract is not read-only")
    if authority.get("capabilities") != promotion.READ_ONLY_CAPABILITIES:
        fail("announcement acceptance capability set or order drifted")
    if authority.get("mutation_capabilities") != []:
        fail("announcement acceptance contract exceeds read-only authority")
    source_binding = require_object(
        contract.get("source_binding"), "contract.source_binding"
    )
    require_exact_keys(
        source_binding,
        {
            "requires_clean_dfmcp_source",
            "requires_clean_dfhack_source",
            "requires_exact_native_receipt_bytes",
            "requires_same_source_version_platform_for_all_events",
            "inherits_protocol_1_0_evidence",
            "baseline_evidence_required_for_promotion",
        },
        "contract.source_binding",
    )
    if (
        source_binding.get("requires_clean_dfmcp_source") is not True
        or source_binding.get("requires_clean_dfhack_source") is not True
        or source_binding.get("requires_exact_native_receipt_bytes") is not True
        or source_binding.get("requires_same_source_version_platform_for_all_events")
        is not True
        or source_binding.get("inherits_protocol_1_0_evidence") is not False
        or source_binding.get("baseline_evidence_required_for_promotion")
        != ["R1", "R2", "R3", "R4", "R5"]
    ):
        fail("announcement acceptance source-binding policy drifted")
    event_fields = require_list(contract.get("event_fields"), "contract.event_fields")
    if event_fields != [
        "schema",
        "sequence",
        "gate",
        "case",
        "status",
        "source",
        "version_tuple",
        "host",
        "assertions",
        "artifacts",
        "evidence_digest",
    ]:
        fail("announcement event field order drifted")
    gates = require_list(contract.get("gates"), "contract.gates")
    if [require_object(gate, "contract.gates[]").get("gate") for gate in gates] != [
        "A1",
        "A2",
        "A3",
        "A4",
        "A5",
        "A6",
    ]:
        fail("announcement acceptance gate order drifted")
    cases = expected_cases(contract)
    limits = require_object(contract.get("limits"), "contract.limits")
    require_exact_keys(
        limits,
        {"maximum_stream_bytes", "maximum_event_bytes", "maximum_events"},
        "contract.limits",
    )
    for name in ["maximum_stream_bytes", "maximum_event_bytes", "maximum_events"]:
        value = limits.get(name)
        if isinstance(value, bool) or not isinstance(value, int) or value <= 0:
            fail(f"contract.limits.{name} must be a positive integer")
    if len(cases) != limits["maximum_events"]:
        fail("announcement acceptance case count and event bound drifted")
    return contract


def read_event_stream(
    path: Path, contract: dict[str, Any]
) -> tuple[list[dict[str, Any]], str]:
    maximum_stream = int(contract["limits"]["maximum_stream_bytes"])
    maximum_event = int(contract["limits"]["maximum_event_bytes"])
    maximum_events = int(contract["limits"]["maximum_events"])
    raw = path.read_bytes()
    if not raw or len(raw) > maximum_stream:
        fail("announcement evidence stream is empty or exceeds its byte bound")
    lowered = raw.lower()
    for marker in [
        b"dfmcp_bridge_token",
        b'"bearer_token":',
        b'"token_value":',
        b'"environment":',
        b"ld_preload",
        b"dyld_insert_libraries",
    ]:
        if marker in lowered:
            fail(f"announcement evidence stream contains forbidden secret marker {marker!r}")
    events: list[dict[str, Any]] = []
    for line_number, raw_line in enumerate(raw.splitlines(), 1):
        if not raw_line.strip():
            fail(f"announcement evidence line {line_number} is empty")
        if len(raw_line) > maximum_event:
            fail(f"announcement evidence line {line_number} exceeds its byte bound")
        events.append(parse_object(raw_line, f"announcement evidence line {line_number}"))
        if len(events) > maximum_events:
            fail("announcement evidence stream exceeds its event-count bound")
    return events, hashlib.sha256(raw).hexdigest()


def validate_native_receipt(
    native_receipt_path: Path,
    native_contract_path: Path = DEFAULT_NATIVE_CONTRACT,
) -> tuple[dict[str, Any], str]:
    native, native_file_sha = promotion.read_object_with_digest(
        native_receipt_path,
        native_generation.MAX_JSON_BYTES,
        "protocol-1.1 native build receipt",
    )
    contract = native_generation.load_contract(native_contract_path)
    normalized = native_generation.validate_receipt(native, contract)
    return normalized, native_file_sha


def normalize_event_artifacts(
    event: dict[str, Any], expected_case: dict[str, Any], index: int
) -> dict[str, str]:
    artifacts = require_object(event.get("artifacts"), f"event[{index}].artifacts")
    required_names = list(expected_case["required_artifact_digests"])
    if len(artifacts) != len(required_names) or set(artifacts) != set(required_names):
        fail(
            f"event[{index}] artifact key set differs: "
            f"expected {required_names}, got {sorted(artifacts)}"
        )
    return {
        name: require_hash(
            artifacts.get(name), f"event[{index}].artifacts.{name}"
        )
        for name in required_names
    }


def validate_events(
    events: list[dict[str, Any]],
    contract: dict[str, Any],
    native: dict[str, Any],
    native_file_sha: str,
) -> tuple[
    dict[str, Any],
    dict[str, Any],
    dict[str, Any],
    list[dict[str, Any]],
    list[dict[str, Any]],
]:
    cases = expected_cases(contract)
    if len(events) != len(cases):
        fail(
            f"announcement evidence event count differs: expected {len(cases)}, got {len(events)}"
        )
    event_fields = set(contract["event_fields"])
    canonical_events: list[dict[str, Any]] = []
    common_source: dict[str, Any] | None = None
    common_version: dict[str, Any] | None = None
    common_host: dict[str, Any] | None = None
    gate_events: dict[str, list[dict[str, Any]]] = {
        gate: [] for gate in ["A1", "A2", "A3", "A4", "A5", "A6"]
    }

    for index, (event, (expected_gate, expected_case)) in enumerate(
        zip(events, cases), 1
    ):
        require_exact_keys(event, event_fields, f"event[{index}]")
        if event.get("schema") != contract["event_schema"]:
            fail(f"event[{index}] schema drifted")
        if event.get("sequence") != index:
            fail(f"event[{index}] sequence must be exactly {index}")
        if event.get("gate") != expected_gate:
            fail(f"event[{index}] gate must be {expected_gate}")
        if event.get("case") != expected_case["case"]:
            fail(
                f"event[{index}] case must be {expected_case['case']!r}, "
                f"got {event.get('case')!r}"
            )
        if event.get("status") != "passed":
            fail(f"event[{index}] did not pass")

        source = require_object(event.get("source"), f"event[{index}].source")
        require_exact_keys(
            source,
            {
                "dfmcp_commit",
                "dfmcp_dirty",
                "dfhack_commit",
                "dfhack_dirty",
                "plugin_sha256",
                "native_build_receipt_sha256",
            },
            f"event[{index}].source",
        )
        normalized_source = {
            "dfmcp_commit": require_commit(
                source.get("dfmcp_commit"), f"event[{index}].source.dfmcp_commit"
            ),
            "dfmcp_dirty": source.get("dfmcp_dirty"),
            "dfhack_commit": require_commit(
                source.get("dfhack_commit"), f"event[{index}].source.dfhack_commit"
            ),
            "dfhack_dirty": source.get("dfhack_dirty"),
            "plugin_sha256": require_hash(
                source.get("plugin_sha256"), f"event[{index}].source.plugin_sha256"
            ),
            "native_build_receipt_sha256": require_hash(
                source.get("native_build_receipt_sha256"),
                f"event[{index}].source.native_build_receipt_sha256",
            ),
        }
        if normalized_source["dfmcp_dirty"] is not False:
            fail(f"event[{index}] is not bound to clean dfmcp source")
        if normalized_source["dfhack_dirty"] is not False:
            fail(f"event[{index}] is not bound to clean DFHack source")
        if normalized_source["native_build_receipt_sha256"] != native_file_sha:
            fail(f"event[{index}] names different protocol-1.1 native receipt bytes")
        if normalized_source["dfmcp_commit"] != native["source"]["dfmcp_commit"]:
            fail(f"event[{index}] dfmcp commit differs from the native receipt")
        if normalized_source["dfhack_commit"] != native["source"]["dfhack_commit"]:
            fail(f"event[{index}] DFHack commit differs from the native receipt")
        if normalized_source["plugin_sha256"] != native["plugin"]["sha256"]:
            fail(f"event[{index}] plugin digest differs from the native receipt")

        version = require_object(event.get("version_tuple"), f"event[{index}].version_tuple")
        require_exact_keys(
            version,
            {"dwarf_fortress", "dfhack", "bridge", "protocol"},
            f"event[{index}].version_tuple",
        )
        normalized_version = {
            "dwarf_fortress": require_string(
                version.get("dwarf_fortress"),
                f"event[{index}].version_tuple.dwarf_fortress",
                128,
            ),
            "dfhack": require_string(
                version.get("dfhack"), f"event[{index}].version_tuple.dfhack", 128
            ),
            "bridge": require_string(
                version.get("bridge"), f"event[{index}].version_tuple.bridge", 64
            ),
            "protocol": require_string(
                version.get("protocol"), f"event[{index}].version_tuple.protocol", 16
            ),
        }
        if (
            normalized_version["bridge"] != "0.2.0"
            or normalized_version["protocol"] != "1.1"
        ):
            fail(f"event[{index}] is not exact bridge 0.2.0 protocol 1.1 evidence")
        if native["bridge"]["bridge_version"] != normalized_version["bridge"]:
            fail(f"event[{index}] bridge version differs from the native generation receipt")
        if native["bridge"]["protocol"] != normalized_version["protocol"]:
            fail(f"event[{index}] protocol differs from the native generation receipt")

        host = require_object(event.get("host"), f"event[{index}].host")
        require_exact_keys(host, {"system", "machine"}, f"event[{index}].host")
        normalized_host = {
            "system": require_string(
                host.get("system"), f"event[{index}].host.system", 128
            ),
            "machine": require_string(
                host.get("machine"), f"event[{index}].host.machine", 128
            ),
        }

        assertions = require_object(event.get("assertions"), f"event[{index}].assertions")
        if assertions != expected_case["required_equals"]:
            fail(f"event[{index}] assertions differ from the exact case contract")
        normalized_artifacts = normalize_event_artifacts(event, expected_case, index)
        expected_evidence_digest = sha256_bytes(
            canonical_json(
                {
                    "assertions": assertions,
                    "artifacts": normalized_artifacts,
                }
            )
        )
        if (
            require_hash(
                event.get("evidence_digest"), f"event[{index}].evidence_digest"
            )
            != expected_evidence_digest
        ):
            fail(f"event[{index}] evidence digest is not canonical")

        if common_source is None:
            common_source = normalized_source
            common_version = normalized_version
            common_host = normalized_host
        elif (
            normalized_source != common_source
            or normalized_version != common_version
            or normalized_host != common_host
        ):
            fail(f"event[{index}] crosses source, version, or platform identity")

        normalized_event = {
            "schema": contract["event_schema"],
            "sequence": index,
            "gate": expected_gate,
            "case": expected_case["case"],
            "status": "passed",
            "source": normalized_source,
            "version_tuple": normalized_version,
            "host": normalized_host,
            "assertions": assertions,
            "artifacts": normalized_artifacts,
            "evidence_digest": expected_evidence_digest,
        }
        canonical_events.append(normalized_event)
        gate_events[expected_gate].append(normalized_event)

    if common_source is None or common_version is None or common_host is None:
        fail("announcement evidence stream contains no events")
    gate_receipts: list[dict[str, Any]] = []
    for gate in ["A1", "A2", "A3", "A4", "A5", "A6"]:
        members = gate_events[gate]
        gate_receipts.append(
            {
                "gate": gate,
                "status": "passed",
                "case_count": len(members),
                "evidence_digest": sha256_bytes(
                    canonical_json(
                        [
                            {
                                "case": member["case"],
                                "evidence_digest": member["evidence_digest"],
                            }
                            for member in members
                        ]
                    )
                ),
            }
        )
    return (
        common_source,
        common_version,
        common_host,
        gate_receipts,
        canonical_events,
    )


def verify(
    event_stream_path: Path,
    native_receipt_path: Path,
    contract_path: Path,
    native_contract_path: Path = DEFAULT_NATIVE_CONTRACT,
) -> dict[str, Any]:
    contract = load_contract(contract_path)
    native, native_file_sha = validate_native_receipt(
        native_receipt_path, native_contract_path
    )
    events, stream_sha = read_event_stream(event_stream_path, contract)
    source, version, host, gates, canonical_events = validate_events(
        events, contract, native, native_file_sha
    )
    canonical_events_sha = sha256_bytes(canonical_json(canonical_events))
    unsigned: dict[str, Any] = {
        "schema": contract["receipt_schema"],
        "status": "qualified",
        "source": source,
        "version_tuple": version,
        "host": host,
        "evidence": {
            "stream_sha256": stream_sha,
            "event_count": len(canonical_events),
            "canonical_events_sha256": canonical_events_sha,
        },
        "gates": gates,
        "capabilities": list(contract["authority"]["capabilities"]),
        "mutation_capabilities": [],
        "claims_established": list(contract["claims_established"]),
        "claims_not_established": list(contract["claims_not_established"]),
    }
    return {
        **unsigned,
        "receipt_digest": sha256_bytes(canonical_json(unsigned)),
    }


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("events", type=Path)
    parser.add_argument("native_receipt", type=Path)
    parser.add_argument("--contract", type=Path, default=DEFAULT_CONTRACT)
    parser.add_argument(
        "--native-contract", type=Path, default=DEFAULT_NATIVE_CONTRACT
    )
    parser.add_argument("--output", type=Path)
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    try:
        receipt = verify(
            args.events,
            args.native_receipt,
            args.contract,
            args.native_contract,
        )
        if args.output is None:
            print(
                json.dumps(
                    receipt,
                    sort_keys=True,
                    separators=(",", ":"),
                    ensure_ascii=False,
                )
            )
        else:
            promotion.write_atomic(args.output, receipt)
    except (
        OSError,
        promotion.PromotionError,
        native_generation.ReceiptError,
        VerificationError,
    ) as exc:
        print(f"live announcement acceptance: FAIL: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
