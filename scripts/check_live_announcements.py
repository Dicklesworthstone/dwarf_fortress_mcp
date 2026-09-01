#!/usr/bin/env python3
"""Validate the unadmitted protocol-1.1 announcement implementation contract."""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CONTRACT = ROOT / "architecture/dfhack_read_bridge_v1_1.json"
PROTO = ROOT / "bridge/dfhack-plugin/proto/DfmcpBridge.proto"
MODEL = ROOT / "crates/dfmcp-adapter/src/live_announcements.rs"
WIRE = ROOT / "crates/dfmcp-adapter/src/announcement_wire.rs"
ADAPTER_LIB = ROOT / "crates/dfmcp-adapter/src/lib.rs"
DOC = ROOT / "docs/LIVE_ANNOUNCEMENT_STREAM.md"


class ContractError(ValueError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ContractError(message)


def check_contract() -> None:
    value = json.loads(CONTRACT.read_text(encoding="utf-8"))
    require(value.get("schema_version") == "dfmcp.dfhack_read_bridge/1.1", "bridge schema drifted")
    require(
        value.get("status") == "implemented_unadmitted_live_read_generation",
        "protocol 1.1 must remain explicitly unadmitted before live evidence",
    )
    transport = value.get("transport", {})
    require(transport.get("plugin_protocol_major") == 1, "protocol major drifted")
    require(transport.get("plugin_protocol_minor") == 1, "protocol minor drifted")
    require(transport.get("bridge_version") == "0.2.0", "bridge implementation version drifted")
    methods = value.get("methods", [])
    require([method.get("name") for method in methods] == ["Handshake", "ReadObservation"], "method waist widened")
    require(
        value.get("compatibility", {}).get("inherits_protocol_1_0_admission") is False,
        "protocol 1.1 must not inherit protocol 1.0 admission",
    )


def check_proto() -> None:
    source = PROTO.read_text(encoding="utf-8")
    for marker in [
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
        require(marker in source, f"protobuf contract omits {marker}")
    rpc_names = re.findall(r"// RPC\s+(\w+)\s*:", source)
    require(rpc_names == ["Handshake", "ReadObservation"], "protobuf RPC surface widened")


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
        "protocol `1.1`",
        "does not admit this source generation",
        "no mutation method",
    ]:
        require(marker.lower() in source, f"announcement documentation omits {marker}")


def main() -> int:
    try:
        check_contract()
        check_proto()
        check_rust()
        check_docs()
    except (OSError, json.JSONDecodeError, ContractError) as exc:
        print(f"live announcement contract: FAIL: {exc}", file=sys.stderr)
        return 1
    print("live announcement contract: PASS (protocol 1.1 modeled, bounded, and unadmitted)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
