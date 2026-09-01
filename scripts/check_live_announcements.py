#!/usr/bin/env python3
"""Validate the implemented, unadmitted protocol-1.1 announcement generation."""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BRIDGE_CONTRACT = ROOT / "architecture/dfhack_read_bridge_v1_1.json"
PROJECTION_CONTRACT = ROOT / "architecture/live_announcement_projection_v1.json"
NATIVE_RECEIPT_CONTRACT = ROOT / "architecture/dfhack_plugin_native_receipt_v1_1.json"
ACCEPTANCE_CONTRACT = ROOT / "architecture/live_announcement_acceptance_v1_1.json"
JOURNAL_CONTRACT = ROOT / "architecture/live_announcement_evidence_journal_v1.json"
PROTO = ROOT / "bridge/dfhack-plugin/proto/DfmcpBridgeV1_1.proto"
NATIVE = ROOT / "bridge/dfhack-plugin/src/dfmcp_bridge_v1_1.cpp"
MODEL = ROOT / "crates/dfmcp-adapter/src/live_announcements.rs"
WIRE = ROOT / "crates/dfmcp-adapter/src/announcement_wire.rs"
ADAPTER_LIB = ROOT / "crates/dfmcp-adapter/src/lib.rs"
DOC = ROOT / "docs/LIVE_ANNOUNCEMENT_STREAM.md"


class ContractError(ValueError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ContractError(message)


def read_json(path: Path) -> dict[str, object]:
    value = json.loads(path.read_text(encoding="utf-8"))
    require(isinstance(value, dict), f"{path.relative_to(ROOT)} must be a JSON object")
    return value


def check_bridge_contract() -> None:
    value = read_json(BRIDGE_CONTRACT)
    require(
        value.get("schema_version") == "dfmcp.dfhack_read_bridge/1.1",
        "bridge schema drifted",
    )
    require(
        value.get("status") == "implemented_unadmitted_live_read_generation",
        "protocol 1.1 must remain explicitly implemented but unadmitted before live evidence",
    )
    transport = value.get("transport", {})
    require(isinstance(transport, dict), "bridge transport must be an object")
    require(transport.get("plugin_protocol_major") == 1, "protocol major drifted")
    require(transport.get("plugin_protocol_minor") == 1, "protocol minor drifted")
    require(transport.get("bridge_version") == "0.2.0", "bridge version drifted")
    require(transport.get("plugin") == "dfmcp_bridge_v1_1", "plugin generation drifted")
    require(transport.get("protobuf_package") == "dfmcp.bridge.v1_1", "protobuf package drifted")
    require(value.get("method_manifest") == ["Handshake", "ReadObservation"], "method waist widened")
    methods = value.get("methods", [])
    require(isinstance(methods, list), "methods must be an array")
    require(
        [method.get("name") for method in methods if isinstance(method, dict)]
        == ["Handshake", "ReadObservation"],
        "method definition set widened",
    )
    observation = methods[1]
    require(isinstance(observation, dict), "ReadObservation contract is malformed")
    extension = observation.get("announcement_extension", {})
    require(isinstance(extension, dict), "announcement extension must be an object")
    require(
        extension.get("request_fields")
        == ["announcement_after_id", "max_announcements"],
        "announcement request extension drifted",
    )
    require(
        extension.get("reply_fields")
        == [
            "announcement_oldest_available_id",
            "announcement_latest_available_id",
            "announcement_requested_after_id",
            "announcement_gap_before_window",
            "announcement_complete_through_latest",
            "announcements",
        ],
        "announcement reply extension drifted",
    )
    compatibility = value.get("compatibility", {})
    require(isinstance(compatibility, dict), "compatibility must be an object")
    require(
        compatibility.get("inherits_protocol_1_0_admission") is False,
        "protocol 1.1 must not inherit protocol 1.0 admission",
    )
    require(
        compatibility.get("method_manifest_alone_does_not_identify_generation")
        is True,
        "same method manifest must not collapse protocol generations",
    )
    acceptance = value.get("acceptance", {})
    require(isinstance(acceptance, dict), "acceptance must be an object")
    require(acceptance.get("current_admission") == "none", "protocol 1.1 is overclaimed as admitted")
    require(
        acceptance.get("baseline_fortress_citizen_campaign_required") is True,
        "protocol 1.1 must re-execute the baseline fortress/citizen campaign",
    )


def check_projection_contract() -> None:
    value = read_json(PROJECTION_CONTRACT)
    require(
        value.get("schema_version") == "dfmcp.live_announcement_projection/1",
        "announcement projection schema drifted",
    )
    require(value.get("status") == "implemented_source_contract", "projection is not implemented source")
    require(value.get("source") == "dfmcp.live-announcement-batch.v1", "projection source drifted")
    coverage = value.get("coverage", {})
    require(isinstance(coverage, dict), "projection coverage must be an object")
    require(coverage.get("preserves_gap_before_retained_window") is True, "projection drops gap evidence")
    require(coverage.get("preserves_complete_through_latest") is True, "projection drops suffix completeness")
    require(coverage.get("may_prove_complete_history") is False, "projection overclaims complete history")
    authority = value.get("authority", {})
    require(isinstance(authority, dict), "projection authority must be an object")
    require(authority.get("capabilities_granted") == [], "projection grants capability")
    require(authority.get("mutation_capabilities") == [], "projection grants mutation")


def check_evidence_contracts() -> None:
    native = read_json(NATIVE_RECEIPT_CONTRACT)
    require(
        native.get("schema_version")
        == "dfmcp.dfhack-plugin-native-qualification-contract/1.1",
        "native wrapper contract schema drifted",
    )
    require(
        native.get("base_receipt_schema") == "dfmcp.dfhack-plugin-qualification/1",
        "native wrapper names the wrong base receipt schema",
    )
    bridge = native.get("bridge", {})
    require(isinstance(bridge, dict), "native wrapper bridge must be an object")
    require(bridge.get("plugin") == "dfmcp_bridge_v1_1", "native wrapper plugin drifted")
    require(bridge.get("protocol") == "1.1", "native wrapper protocol drifted")
    require(bridge.get("rpc_methods") == ["Handshake", "ReadObservation"], "native RPC waist widened")
    require(bridge.get("mutation_rpc_methods") == [], "native wrapper admits mutation")

    acceptance = read_json(ACCEPTANCE_CONTRACT)
    require(
        acceptance.get("schema_version") == "dfmcp.live_announcement_acceptance/1.1",
        "A1-A6 contract schema drifted",
    )
    require(
        acceptance.get("event_schema") == "dfmcp.live-announcement-evidence/1.1",
        "A1-A6 event schema drifted",
    )
    require(
        acceptance.get("receipt_schema")
        == "dfmcp.live-announcement-acceptance-receipt/1.1",
        "A1-A6 receipt schema drifted",
    )
    gates = acceptance.get("gates", [])
    require(isinstance(gates, list), "A1-A6 gates must be an array")
    require(
        [gate.get("gate") for gate in gates if isinstance(gate, dict)]
        == ["A1", "A2", "A3", "A4", "A5", "A6"],
        "A1-A6 gate order drifted",
    )
    case_count = sum(
        len(gate.get("cases", []))
        for gate in gates
        if isinstance(gate, dict) and isinstance(gate.get("cases"), list)
    )
    require(case_count == 43, "A1-A6 case count drifted")
    limits = acceptance.get("limits", {})
    require(isinstance(limits, dict), "A1-A6 limits must be an object")
    require(limits.get("maximum_events") == case_count, "A1-A6 event bound differs from case count")
    authority = acceptance.get("authority", {})
    require(isinstance(authority, dict), "A1-A6 authority must be an object")
    require(authority.get("capabilities") == ["doctor", "observe", "query", "wait"], "A1-A6 capabilities drifted")
    require(authority.get("mutation_capabilities") == [], "A1-A6 contract grants mutation")

    journal = read_json(JOURNAL_CONTRACT)
    require(
        journal.get("schema_version")
        == "dfmcp.live-announcement-evidence-journal-contract/1",
        "announcement journal contract schema drifted",
    )
    require(
        journal.get("acceptance_contract")
        == "architecture/live_announcement_acceptance_v1_1.json",
        "announcement journal names the wrong acceptance contract",
    )
    journal_authority = journal.get("authority", {})
    require(isinstance(journal_authority, dict), "journal authority must be an object")
    require(journal_authority.get("capabilities_granted") == [], "journal grants capability")
    require(journal_authority.get("mutation_capabilities") == [], "journal grants mutation")


def check_proto_and_native() -> None:
    proto = PROTO.read_text(encoding="utf-8")
    require("package dfmcp.bridge.v1_1;" in proto, "protocol-1.1 protobuf package drifted")
    for marker in [
        "// Plugin: dfmcp_bridge_v1_1",
        "// RPC Handshake",
        "// RPC ReadObservation",
        "optional sint32 announcement_after_id = 8 [default = -1];",
        "optional uint32 max_announcements = 9 [default = 128];",
        "message AnnouncementRecord",
        "required sint32 report_id = 1;",
        "required string text = 3;",
        "required sint32 announcement_oldest_available_id = 20;",
        "required sint32 announcement_latest_available_id = 21;",
        "required sint32 announcement_requested_after_id = 22;",
        "required bool announcement_gap_before_window = 23;",
        "required bool announcement_complete_through_latest = 24;",
        "repeated AnnouncementRecord announcements = 25;",
    ]:
        require(marker in proto, f"protobuf contract omits {marker}")
    rpc_names = re.findall(r"// RPC\s+(\w+)\s*:", proto)
    require(rpc_names == ["Handshake", "ReadObservation"], "protobuf RPC surface widened")
    require("ReadAnnouncements" not in proto, "abandoned standalone announcement RPC returned")

    native = NATIVE.read_text(encoding="utf-8")
    for marker in [
        'DFHACK_PLUGIN("dfmcp_bridge_v1_1")',
        "constexpr std::uint32_t PROTOCOL_MINOR = 1;",
        'constexpr const char *BRIDGE_VERSION = "0.2.0";',
        'out->add_supported_methods("Handshake")',
        'out->add_supported_methods("ReadObservation")',
        "publish_announcements",
        "HARD_MAX_ANNOUNCEMENTS = 512",
        "MAX_ANNOUNCEMENT_TEXT_BYTES = 2048",
        "DFHACK_PLUGIN_RPC_HANDLERS",
    ]:
        require(marker in native, f"native protocol-1.1 plugin omits {marker}")
    for forbidden in [
        'add_supported_methods("ReadAnnouncements")',
        "RunCommand",
        "RunLua",
        "keyboard",
        "pause_state_mutation",
    ]:
        require(forbidden not in native, f"native plugin contains forbidden surface {forbidden}")


def check_rust() -> None:
    model = MODEL.read_text(encoding="utf-8")
    wire = WIRE.read_text(encoding="utf-8")
    library = ADAPTER_LIB.read_text(encoding="utf-8")
    for marker in [
        "MAX_ANNOUNCEMENTS_PER_BATCH: usize = 512",
        "MAX_ANNOUNCEMENT_TEXT_BYTES: usize = 2_048",
        "GapBeforeRetainedWindow",
        "can_prove_no_newer_retained_reports",
        "canonical announcement batch exceeds its 2 MiB ceiling",
        "canonical_batch_is_deterministic",
        "retained_window_gap_is_explicit",
        "tampering_breaks_canonical_validation",
    ]:
        require(marker in model, f"announcement model omits {marker}")
    for marker in [
        "ANNOUNCEMENT_AFTER_ID_FIELD: u32 = 8",
        "ANNOUNCEMENT_RECORD_FIELD: u32 = 25",
        "encode_announcement_request_fields",
        "decode_announcement_reply_fields",
        "protobuf varint is not minimally encoded",
        "protobuf bool field {field} has noncanonical value",
        "duplicate_required_extension_field_is_rejected",
        "oversized_text_is_rejected_before_allocation_growth",
        "unknown_fields_are_skipped_without_changing_identity",
    ]:
        require(marker in wire, f"announcement wire codec omits {marker}")
    require("pub mod announcement_wire;" in library, "adapter does not compile announcement wire codec")
    require("pub mod live_announcements;" in library, "adapter does not compile announcement model")
    require(model.count("#[test]") >= 6, "announcement model needs at least six focused tests")
    require(wire.count("#[test]") >= 8, "announcement wire codec needs at least eight adversarial tests")


def check_docs() -> None:
    source = DOC.read_text(encoding="utf-8").lower()
    for marker in [
        "retained-window",
        "gap_before_window",
        "complete_through_latest",
        "existing `readobservation` rpc",
        "protocol `1.1`",
        "does not admit this source generation",
        "no mutation method",
        "handshake\nreadobservation",
    ]:
        require(marker.lower() in source, f"announcement documentation omits {marker}")


def main() -> int:
    try:
        check_bridge_contract()
        check_projection_contract()
        check_evidence_contracts()
        check_proto_and_native()
        check_rust()
        check_docs()
    except (OSError, json.JSONDecodeError, ContractError) as exc:
        print(f"live announcement contract: FAIL: {exc}", file=sys.stderr)
        return 1
    print(
        "live announcement contract: PASS "
        "(protocol 1.1 implemented, bounded, read-only, and explicitly unadmitted)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
