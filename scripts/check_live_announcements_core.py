#!/usr/bin/env python3
"""Validate the implemented, isolated, unadmitted protocol-1.1 read stack."""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
BASE_PROTO = ROOT / "bridge/dfhack-plugin/proto/DfmcpBridge.proto"
BASE_NATIVE = ROOT / "bridge/dfhack-plugin/src/dfmcp_bridge.cpp"
BASE_WIRE = ROOT / "crates/dfmcp-adapter/src/dfhack_wire.rs"
BASE_SESSION = ROOT / "crates/dfmcp-adapter/src/live_session.rs"
BRIDGE_CONTRACT = ROOT / "architecture/dfhack_read_bridge_v1_1.json"
PROJECTION_CONTRACT = ROOT / "architecture/live_announcement_projection_v1.json"
SOURCE_CONTRACT = ROOT / "architecture/live_announcement_source_qualification_v1_1.json"
NATIVE_RECEIPT_CONTRACT = ROOT / "architecture/dfhack_plugin_native_receipt_v1_1.json"
ACCEPTANCE_CONTRACT = ROOT / "architecture/live_announcement_acceptance_v1_1.json"
JOURNAL_CONTRACT = ROOT / "architecture/live_announcement_evidence_journal_v1.json"
PROTO = ROOT / "bridge/dfhack-plugin/proto/DfmcpBridgeV1_1.proto"
NATIVE = ROOT / "bridge/dfhack-plugin/src/dfmcp_bridge_v1_1.cpp"
BATCH = ROOT / "crates/dfmcp-adapter/src/live_announcement_batch.rs"
WIRE = ROOT / "crates/dfmcp-adapter/src/announcement_wire.rs"
CLIENT = ROOT / "crates/dfmcp-adapter/src/dfhack_wire_v1_1.rs"
CAPSULE = ROOT / "crates/dfmcp-adapter/src/live_observation_v1_1.rs"
DRIVER = ROOT / "crates/dfmcp-adapter/src/live_session_v1_1.rs"
FENCE = ROOT / "crates/dfmcp-adapter/src/fenced_live_source_v1_1.rs"
CONNECTOR = ROOT / "crates/dfmcp-adapter/src/live_connect_v1_1.rs"
ANNOUNCEMENT_PROJECTION = ROOT / "crates/dfmcp-adapter/src/live_announcement_projection.rs"
COMBINED_PROJECTION = ROOT / "crates/dfmcp-adapter/src/live_projection_v1_1.rs"
BRIEFING = ROOT / "crates/dfmcp-adapter/src/live_announcement_briefing.rs"
ADAPTER_LIB = ROOT / "crates/dfmcp-adapter/src/lib.rs"
PROBE = ROOT / "crates/dwarf-fortress-mcp/src/bin/dfmcp-live-announcement-probe.rs"
SOURCE_WRAPPER = ROOT / "scripts/qualify_live_announcement_source.sh"
DOC = ROOT / "docs/LIVE_ANNOUNCEMENT_STREAM.md"
STATUS = ROOT / "docs/LIVE_ANNOUNCEMENT_IMPLEMENTATION_STATUS.md"
RETIRED_PATHS = [
    ROOT / "architecture/live_announcement_read_v1.json",
    ROOT / "architecture/live_announcement_read_v1.README",
    ROOT / "docs/LIVE_ANNOUNCEMENT_READ_GENERATION.md",
    ROOT / "docs/ANNOUNCEMENT_WINDOW_AGENT_SEMANTICS.md",
    ROOT / "scripts/check_live_announcement_contract.py",
    ROOT / "scripts/check_live_announcement_stack.py",
    ROOT / "scripts/qualify_live_announcement_generation.sh",
    ROOT / "crates/dfmcp-adapter/src/live_announcements.rs",
    ROOT / "crates/dfmcp-adapter/tests/live_announcements.rs",
]
EXPECTED_SOURCE_GATES = [
    "repository-integrity",
    "announcement-contract",
    "announcement-contract-tests",
    "announcement-acceptance-tests",
    "python-syntax",
    "shell-syntax",
    "cargo-metadata",
    "rustfmt",
    "clippy",
    "adapter-tests",
    "workspace-tests",
    "rustdoc",
    "announcement-probe-help",
]


class ContractError(ValueError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ContractError(message)


def read_json(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    require(isinstance(value, dict), f"{path.relative_to(ROOT)} must be a JSON object")
    return value


def require_markers(path: Path, markers: list[str]) -> str:
    source = path.read_text(encoding="utf-8")
    for marker in markers:
        require(marker in source, f"{path.relative_to(ROOT)} omits {marker}")
    return source


def rpc_comments(source: str) -> list[str]:
    return re.findall(r"// RPC\s+(\w+)\s*:", source)


def check_protocol_isolation() -> None:
    base_proto = require_markers(
        BASE_PROTO,
        [
            "package dfmcp.bridge.v1;",
            "// Plugin: dfmcp_bridge",
            "// RPC Handshake",
            "// RPC ReadObservation",
        ],
    )
    require(
        rpc_comments(base_proto) == ["Handshake", "ReadObservation"],
        "protocol 1.0 protobuf surface is not citizen-only",
    )
    for forbidden in ["ReadAnnouncements", "announcement_after_id", "AnnouncementRecord"]:
        require(forbidden not in base_proto, f"protocol 1.0 protobuf contains {forbidden}")

    base_native = require_markers(
        BASE_NATIVE,
        [
            'DFHACK_PLUGIN("dfmcp_bridge")',
            "constexpr std::uint32_t PROTOCOL_MINOR = 0;",
            'constexpr const char *BRIDGE_VERSION = "0.1.0";',
            'out->add_supported_methods("Handshake")',
            'out->add_supported_methods("ReadObservation")',
        ],
    )
    for forbidden in [
        "ReadAnnouncements",
        "publish_announcements",
        "df::global::world->status.reports",
        'add_supported_methods("ReadAnnouncements")',
    ]:
        require(forbidden not in base_native, f"protocol 1.0 native plugin contains {forbidden}")

    base_wire = BASE_WIRE.read_text(encoding="utf-8")
    for forbidden in [
        "ANNOUNCEMENT_PROTOCOL_MINOR",
        "ANNOUNCEMENT_METHOD",
        "ReadAnnouncements",
        "AnnouncementPage",
        "read_announcements",
    ]:
        require(forbidden not in base_wire, f"protocol 1.0 Rust wire contains {forbidden}")

    base_session = BASE_SESSION.read_text(encoding="utf-8")
    for forbidden in [
        "LiveAnnouncementSource",
        "AnnouncementWindowAssembler",
        "read_complete_announcement_window",
    ]:
        require(forbidden not in base_session, f"protocol 1.0 session driver contains {forbidden}")

    library = ADAPTER_LIB.read_text(encoding="utf-8")
    for forbidden in [
        "pub mod live_announcements;",
        "AnnouncementPage, AnnouncementRecord",
        "AnnouncementWindowAssembler",
        "LiveAnnouncementSource",
        "read_complete_announcement_window",
    ]:
        require(forbidden not in library, f"adapter root retains legacy announcement API {forbidden}")
    for retired in RETIRED_PATHS:
        require(
            not retired.exists() and not retired.is_symlink(),
            f"retired standalone announcement path remains: {retired.relative_to(ROOT)}",
        )


def check_bridge_contract() -> None:
    value = read_json(BRIDGE_CONTRACT)
    require(value.get("schema_version") == "dfmcp.dfhack_read_bridge/1.1", "bridge schema drifted")
    require(
        value.get("status") == "implemented_unadmitted_live_read_generation",
        "protocol 1.1 must remain implemented but explicitly unadmitted",
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
    require(isinstance(methods, list), "bridge methods must be an array")
    require(
        [method.get("name") for method in methods if isinstance(method, dict)]
        == ["Handshake", "ReadObservation"],
        "bridge method definition set widened",
    )
    observation = methods[1]
    require(isinstance(observation, dict), "ReadObservation contract is malformed")
    extension = observation.get("announcement_extension", {})
    require(isinstance(extension, dict), "announcement extension must be an object")
    require(
        extension.get("request_fields") == ["announcement_after_id", "max_announcements"],
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
    require(compatibility.get("inherits_protocol_1_0_admission") is False, "V1.1 inherits V1.0 admission")
    require(
        compatibility.get("method_manifest_alone_does_not_identify_generation") is True,
        "same method manifest collapses protocol generations",
    )
    acceptance = value.get("acceptance", {})
    require(isinstance(acceptance, dict), "acceptance must be an object")
    require(acceptance.get("current_admission") == "none", "protocol 1.1 is overclaimed as admitted")
    require(
        acceptance.get("baseline_fortress_citizen_campaign_required") is True,
        "protocol 1.1 does not require a fresh baseline campaign",
    )


def check_source_contract() -> None:
    value = read_json(SOURCE_CONTRACT)
    require(
        value.get("schema_version") == "dfmcp.live-announcement-source-qualification-contract/1",
        "source qualification contract schema drifted",
    )
    require(
        value.get("receipt_schema") == "dfmcp.live-announcement-source-qualification/1",
        "source qualification receipt schema drifted",
    )
    require(value.get("status") == "normative_source_only_contract", "source contract status drifted")
    require(
        value.get("bridge")
        == {"plugin": "dfmcp_bridge_v1_1", "bridge_version": "0.2.0", "protocol": "1.1"},
        "source qualification bridge identity drifted",
    )
    require(value.get("required_gates") == EXPECTED_SOURCE_GATES, "source gate set or order drifted")
    expected_sources = {
        "adapter_root": "crates/dfmcp-adapter/src/lib.rs",
        "announcement_batch": "crates/dfmcp-adapter/src/live_announcement_batch.rs",
        "announcement_wire": "crates/dfmcp-adapter/src/announcement_wire.rs",
        "announcement_client": "crates/dfmcp-adapter/src/dfhack_wire_v1_1.rs",
        "announcement_capsule": "crates/dfmcp-adapter/src/live_observation_v1_1.rs",
        "announcement_driver": "crates/dfmcp-adapter/src/live_session_v1_1.rs",
        "announcement_projection": "crates/dfmcp-adapter/src/live_announcement_projection.rs",
        "combined_projection": "crates/dfmcp-adapter/src/live_projection_v1_1.rs",
        "announcement_briefing": "crates/dfmcp-adapter/src/live_announcement_briefing.rs",
        "announcement_source_fence": "crates/dfmcp-adapter/src/fenced_live_source_v1_1.rs",
        "announcement_connector": "crates/dfmcp-adapter/src/live_connect_v1_1.rs",
        "protocol_1_0_proto": "bridge/dfhack-plugin/proto/DfmcpBridge.proto",
        "protocol_1_0_native": "bridge/dfhack-plugin/src/dfmcp_bridge.cpp",
        "protocol_1_1_proto": "bridge/dfhack-plugin/proto/DfmcpBridgeV1_1.proto",
        "protocol_1_1_native": "bridge/dfhack-plugin/src/dfmcp_bridge_v1_1.cpp",
        "protocol_contract": "architecture/dfhack_read_bridge_v1_1.json",
        "projection_contract": "architecture/live_announcement_projection_v1.json",
        "acceptance_contract": "architecture/live_announcement_acceptance_v1_1.json",
        "contract_checker": "scripts/check_live_announcements.py",
        "contract_checker_tests": "scripts/test_live_announcement_contract.py",
        "source_qualification_wrapper": "scripts/qualify_live_announcement_source.sh",
        "announcement_probe": "crates/dwarf-fortress-mcp/src/bin/dfmcp-live-announcement-probe.rs",
        "stream_documentation": "docs/LIVE_ANNOUNCEMENT_STREAM.md",
        "implementation_status": "docs/LIVE_ANNOUNCEMENT_IMPLEMENTATION_STATUS.md",
    }
    source_digests = value.get("required_source_digests", {})
    require(isinstance(source_digests, dict), "source digest mapping must be an object")
    for name, relative in expected_sources.items():
        require(source_digests.get(name) == relative, f"source contract omits {name}")
    for name, relative in source_digests.items():
        require(isinstance(name, str) and isinstance(relative, str), "source mapping is malformed")
        require((ROOT / relative).is_file(), f"source-bound file is missing: {relative}")
    authority = value.get("authority", {})
    require(authority.get("capabilities_granted") == [], "source contract grants capability")
    require(authority.get("mutation_capabilities") == [], "source contract grants mutation")


def check_projection_contract() -> None:
    value = read_json(PROJECTION_CONTRACT)
    require(
        value.get("schema_version") == "dfmcp.live_announcement_projection/1",
        "announcement projection schema drifted",
    )
    require(value.get("status") == "implemented_source_contract", "projection status drifted")
    require(value.get("source") == "dfmcp.live-announcement-batch.v1", "projection source drifted")
    coverage = value.get("coverage", {})
    require(coverage.get("preserves_gap_before_retained_window") is True, "projection drops gap evidence")
    require(coverage.get("preserves_complete_through_latest") is True, "projection drops suffix completeness")
    require(coverage.get("may_prove_complete_history") is False, "projection overclaims history")
    authority = value.get("authority", {})
    require(authority.get("capabilities_granted") == [], "projection grants capability")
    require(authority.get("mutation_capabilities") == [], "projection grants mutation")


def check_evidence_contracts() -> None:
    native = read_json(NATIVE_RECEIPT_CONTRACT)
    require(
        native.get("schema_version") == "dfmcp.dfhack-plugin-native-qualification-contract/1.1",
        "native qualification contract schema drifted",
    )
    bridge = native.get("bridge", {})
    require(bridge.get("plugin") == "dfmcp_bridge_v1_1", "native plugin identity drifted")
    require(bridge.get("protocol") == "1.1", "native protocol identity drifted")
    require(bridge.get("rpc_methods") == ["Handshake", "ReadObservation"], "native RPC waist widened")
    require(bridge.get("mutation_rpc_methods") == [], "native contract admits mutation")

    acceptance = read_json(ACCEPTANCE_CONTRACT)
    require(
        acceptance.get("schema_version") == "dfmcp.live_announcement_acceptance/1.1",
        "A1-A6 contract schema drifted",
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
    require(acceptance.get("limits", {}).get("maximum_events") == case_count, "event bound drifted")
    authority = acceptance.get("authority", {})
    require(
        authority.get("capabilities") == ["doctor", "observe", "query", "wait"],
        "A1-A6 read-only capability set drifted",
    )
    require(authority.get("mutation_capabilities") == [], "A1-A6 grants mutation")

    journal = read_json(JOURNAL_CONTRACT)
    require(
        journal.get("acceptance_contract") == "architecture/live_announcement_acceptance_v1_1.json",
        "announcement journal names the wrong acceptance contract",
    )
    journal_authority = journal.get("authority", {})
    require(journal_authority.get("capabilities_granted") == [], "journal grants capability")
    require(journal_authority.get("mutation_capabilities") == [], "journal grants mutation")


def check_proto_and_native() -> None:
    proto = require_markers(
        PROTO,
        [
            "package dfmcp.bridge.v1_1;",
            "// Plugin: dfmcp_bridge_v1_1",
            "// RPC Handshake",
            "// RPC ReadObservation",
            "optional sint32 announcement_after_id = 8 [default = -1];",
            "optional uint32 max_announcements = 9 [default = 128];",
            "message AnnouncementRecord",
            "required sint32 announcement_oldest_available_id = 20;",
            "required sint32 announcement_latest_available_id = 21;",
            "required sint32 announcement_requested_after_id = 22;",
            "required bool announcement_gap_before_window = 23;",
            "required bool announcement_complete_through_latest = 24;",
            "repeated AnnouncementRecord announcements = 25;",
        ],
    )
    require(rpc_comments(proto) == ["Handshake", "ReadObservation"], "V1.1 protobuf RPC waist widened")
    require("ReadAnnouncements" not in proto, "standalone announcement RPC returned to V1.1")

    native = require_markers(
        NATIVE,
        [
            'DFHACK_PLUGIN("dfmcp_bridge_v1_1")',
            "constexpr std::uint32_t PROTOCOL_MINOR = 1;",
            'constexpr const char *BRIDGE_VERSION = "0.2.0";',
            'out->add_supported_methods("Handshake")',
            'out->add_supported_methods("ReadObservation")',
            "publish_announcements",
            "HARD_MAX_ANNOUNCEMENTS = 512",
            "MAX_ANNOUNCEMENT_TEXT_BYTES = 2048",
            "DFHACK_PLUGIN_RPC_HANDLERS",
        ],
    )
    for forbidden in [
        'add_supported_methods("ReadAnnouncements")',
        'addFunction("ReadAnnouncements"',
        "RunCommand",
        "RunLua",
        "SetPauseState",
        "PassKeyboardEvent",
        "SF_ALLOW_REMOTE",
    ]:
        require(forbidden not in native, f"native V1.1 contains forbidden surface {forbidden}")


def check_rust_stack() -> None:
    batch = require_markers(
        BATCH,
        [
            "pub const MAX_ANNOUNCEMENTS_PER_BATCH: usize = 512",
            "pub const MAX_ANNOUNCEMENT_TEXT_BYTES: usize = 2_048",
            "pub enum AnnouncementContinuity",
            "pub struct AnnouncementReplyContext",
            "pub struct AnnouncementBatchRecord",
            "pub struct AnnouncementCoverage",
            "pub struct LiveAnnouncementBatch",
            "canonical announcement batch exceeds its",
            "canonical_batch_is_deterministic",
            "retained_window_gap_is_explicit",
            "tampering_breaks_canonical_validation",
        ],
    )
    require(batch.count("#[test]") >= 7, "announcement batch needs at least seven tests")

    wire = require_markers(
        WIRE,
        [
            "ANNOUNCEMENT_AFTER_ID_FIELD: u32 = 8",
            "ANNOUNCEMENT_RECORD_FIELD: u32 = 25",
            "AnnouncementBatchRecord",
            "AnnouncementReplyContext",
            "encode_announcement_request_fields",
            "decode_announcement_reply_fields",
            "protobuf varint is not minimally encoded",
            "protobuf bool field {field} has noncanonical value",
            "duplicate_required_extension_field_is_rejected",
            "oversized_text_is_rejected_before_allocation_growth",
            "unknown_fields_are_skipped_without_changing_identity",
        ],
    )
    require(wire.count("#[test]") >= 11, "announcement extension wire needs eleven tests")
    require(
        "use crate::live_announcement_batch" in wire,
        "announcement wire is not bound to the canonical batch module",
    )

    client = require_markers(
        CLIENT,
        [
            'const PLUGIN_NAME: &str = "dfmcp_bridge_v1_1";',
            'const HANDSHAKE_METHOD: &str = "Handshake";',
            'const OBSERVATION_METHOD: &str = "ReadObservation";',
            "AnnouncementReplyContext",
            "decode_announcement_reply_fields(",
            "BridgeCredentialsV1_1",
            "ObservationPageV1_1",
            "DfHackRpcClientV1_1",
            "negotiate_and_read_citizens_and_announcements",
            "retained_window_gap_survives_transport_decode",
        ],
    )
    require("ReadAnnouncements" not in client, "isolated V1.1 client binds a standalone RPC")
    require(client.count("#[test]") >= 8, "protocol-1.1 client needs focused wire tests")

    capsule = require_markers(
        CAPSULE,
        [
            "pub struct LiveObservationCapsuleV1_1",
            "pub struct ObservationAssemblerV1_1",
            "announcement_drift_between_pages_is_transactionally_rejected",
            "protocol-1.1 citizen and announcement evidence describe different observation instants",
        ],
    )
    require(capsule.count("#[test]") >= 6, "protocol-1.1 capsule needs focused tests")

    driver = require_markers(
        DRIVER,
        [
            "pub trait LiveObservationSourceV1_1",
            "pub fn read_complete_observation_v1_1",
            "pub fn read_complete_observation_v1_1_bounded",
            "The same announcement cursor and limit are sent with every citizen page",
        ],
    )
    require(driver.count("#[test]") >= 5, "protocol-1.1 driver needs focused tests")

    fence = require_markers(
        FENCE,
        [
            "pub struct FencedLiveSourceV1_1",
            "protocol-1.1 live source is permanently fenced after failure",
            "first_failure_permanently_fences_the_transport",
        ],
    )
    require(fence.count("#[test]") >= 3, "protocol-1.1 fence needs focused tests")

    connector = require_markers(
        CONNECTOR,
        [
            "pub type AuthenticatedLiveSourceV1_1",
            "pub fn connect_authenticated_live_source_v1_1",
            "protocol-1.1 live bridge endpoint must be numeric loopback",
        ],
    )
    require(connector.count("#[test]") >= 2, "protocol-1.1 connector needs loopback tests")

    projection = require_markers(
        ANNOUNCEMENT_PROJECTION,
        [
            "AnnouncementBatchRecord",
            "pub struct LiveAnnouncementProjection",
            "pub fn project_live_announcement_batch",
            "announcement projection produced a duplicate entity ID",
            "retained_window_gap_survives_projection",
        ],
    )
    require(projection.count("#[test]") >= 5, "announcement projection needs focused tests")

    combined = require_markers(
        COMBINED_PROJECTION,
        [
            "pub struct LiveWorldProjectionV1_1",
            "pub fn project_live_capsule_v1_1",
            "AnnouncementBatchRecord",
            "fortress.announcements.retained_suffix",
            "fortress.announcements.history",
            "complete retained suffix is not complete historical coverage",
        ],
    )
    require(combined.count("#[test]") >= 5, "combined projection needs focused tests")

    briefing = require_markers(
        BRIEFING,
        [
            "AnnouncementBatchRecord",
            "pub struct LiveAnnouncementBriefing",
            "pub fn build_live_announcement_briefing",
            "pub fn summarize_live_announcement_change",
            "complete_history: false",
            "briefing_never_claims_complete_history",
        ],
    )
    require(briefing.count("#[test]") >= 6, "announcement briefing needs focused tests")

    library = ADAPTER_LIB.read_text(encoding="utf-8")
    for declaration in [
        "pub mod announcement_wire;",
        "pub mod dfhack_wire_v1_1;",
        "pub mod fenced_live_source_v1_1;",
        "pub mod live_announcement_batch;",
        "pub mod live_announcement_briefing;",
        "pub mod live_announcement_projection;",
        "pub mod live_connect_v1_1;",
        "pub mod live_observation_v1_1;",
        "pub mod live_projection_v1_1;",
        "pub mod live_session_v1_1;",
    ]:
        require(declaration in library, f"adapter root omits {declaration}")
    for exported in [
        "AnnouncementBatchRecord",
        "AnnouncementReplyContext",
        "LiveAnnouncementBatch",
        "MAX_ANNOUNCEMENT_TEXT_BYTES",
        "BridgeCredentialsV1_1",
        "DfHackRpcClientV1_1",
        "FencedLiveSourceV1_1",
        "connect_authenticated_live_source_v1_1",
        "LiveObservationCapsuleV1_1",
        "read_complete_observation_v1_1_bounded",
        "project_live_capsule_v1_1",
        "build_live_announcement_briefing",
    ]:
        require(exported in library, f"adapter root omits V1.1 export {exported}")

    probe = require_markers(
        PROBE,
        [
            "DFMCP_ALLOW_UNQUALIFIED_ANNOUNCEMENT_PROBE",
            "connect_authenticated_live_source_v1_1",
            "read_complete_observation_v1_1_bounded",
            "project_live_capsule_v1_1",
            "does not establish compatibility",
        ],
    )
    require("serve-live" not in probe, "announcement probe can enter admitted live-server mode")


def check_qualification_and_docs() -> None:
    wrapper = SOURCE_WRAPPER.read_text(encoding="utf-8")
    for marker in [
        "announcement source qualification requires a clean worktree",
        "run_gate announcement-contract python3 scripts/check_live_announcements.py",
        "run_gate announcement-contract-tests python3 scripts/test_live_announcement_contract.py",
        "run_gate announcement-acceptance-tests python3 scripts/test_live_announcement_acceptance.py",
        "run_gate clippy cargo clippy --locked --workspace --all-targets --all-features -- -D warnings",
        "run_gate workspace-tests cargo test --locked --workspace --all-targets --all-features",
        "run_gate announcement-probe-help cargo run --locked --quiet --bin dfmcp-live-announcement-probe -- help",
        "capabilities_granted':[]",
        "mutation_capabilities':[]",
    ]:
        require(marker in wrapper, f"source qualification wrapper omits {marker}")

    documentation = DOC.read_text(encoding="utf-8").lower()
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
        require(marker in documentation, f"announcement documentation omits {marker}")

    status = STATUS.read_text(encoding="utf-8").lower()
    for marker in [
        "implemented in source",
        "not admitted",
        "canonical batch",
        "safe-rust",
        "world projection",
        "diagnostic probe",
        "a1-a6",
        "mutation",
        "standalone `readannouncements`",
    ]:
        require(marker in status, f"announcement implementation status omits {marker}")


def main() -> int:
    try:
        check_protocol_isolation()
        check_bridge_contract()
        check_source_contract()
        check_projection_contract()
        check_evidence_contracts()
        check_proto_and_native()
        check_rust_stack()
        check_qualification_and_docs()
    except (OSError, SyntaxError, json.JSONDecodeError, ContractError) as exc:
        print(f"live announcement contract: FAIL: {exc}", file=sys.stderr)
        return 1
    print(
        "live announcement contract: PASS "
        "(protocol 1.0 isolated; integrated 1.1 bounded, read-only, and unadmitted)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
