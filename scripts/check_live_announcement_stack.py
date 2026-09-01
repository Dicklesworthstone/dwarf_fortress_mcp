#!/usr/bin/env python3
"""Validate the prospective protocol-1.1 announcement stack end to end."""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BRIDGE_CONTRACT = ROOT / "architecture/dfhack_read_bridge_v1_1.json"
WINDOW_CONTRACT = ROOT / "architecture/live_announcement_read_v1.json"
PROTO = ROOT / "bridge/dfhack-plugin/proto/DfmcpBridge.proto"
PLUGIN = ROOT / "bridge/dfhack-plugin/src/dfmcp_bridge.cpp"
WIRE = ROOT / "crates/dfmcp-adapter/src/dfhack_wire.rs"
WINDOW = ROOT / "crates/dfmcp-adapter/src/live_announcements.rs"
DRIVER = ROOT / "crates/dfmcp-adapter/src/live_session.rs"
LIB = ROOT / "crates/dfmcp-adapter/src/lib.rs"
DESIGN = ROOT / "docs/LIVE_ANNOUNCEMENT_READ_GENERATION.md"


class ContractError(ValueError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ContractError(message)


def rust_tests(source: str) -> set[str]:
    return set(re.findall(r"#\[test\]\s*fn\s+([A-Za-z0-9_]+)\s*\(", source))


def main() -> int:
    try:
        bridge = json.loads(BRIDGE_CONTRACT.read_text(encoding="utf-8"))
        window = json.loads(WINDOW_CONTRACT.read_text(encoding="utf-8"))
        require(
            bridge.get("schema_version") == "dfmcp.dfhack_read_bridge/1.1",
            "protocol-1.1 bridge contract schema drifted",
        )
        require(
            bridge.get("status") == "prospective_unadmitted_read_only_generation",
            "protocol-1.1 bridge contract overclaims admission",
        )
        require(
            bridge.get("method_manifest")
            == ["Handshake", "ReadObservation", "ReadAnnouncements"],
            "protocol-1.1 method manifest drifted",
        )
        require(
            bridge.get("compatibility", {}).get(
                "protocol_1_0_default_client_path_preserved"
            )
            is True,
            "protocol 1.0 default path is not preserved",
        )
        require(
            bridge.get("acceptance", {}).get("current_admission") == "none",
            "protocol 1.1 must remain unadmitted before live evidence",
        )
        require(
            window.get("bridge_protocol", {}).get("mutation_capabilities") == [],
            "announcement contract admits mutation",
        )

        proto = PROTO.read_text(encoding="utf-8")
        for marker in [
            "RPC ReadAnnouncements",
            "message ReadAnnouncementsRequest",
            "message AnnouncementRecord",
            "message ReadAnnouncementsReply",
            "optional sint32 through_report_id = 6 [default = -1]",
        ]:
            require(marker in proto, f"protobuf contract omits {marker}")

        plugin = PLUGIN.read_text(encoding="utf-8")
        for marker in [
            'constexpr std::uint32_t CITIZEN_PROTOCOL_MINOR = 0;',
            'constexpr std::uint32_t ANNOUNCEMENT_PROTOCOL_MINOR = 1;',
            'constexpr const char *BRIDGE_VERSION = "0.2.0";',
            "command_result ReadAnnouncements",
            "validate_announcement_protocol",
            "authenticate(in->bearer_token()",
            "df::global::world->status.reports",
            "window_latest_report_id",
            "history_truncated",
            'service->addFunction("ReadAnnouncements", ReadAnnouncements, 0);',
        ]:
            require(marker in plugin, f"native announcement bridge omits {marker}")
        auth_index = plugin.index("authenticate(in->bearer_token()", plugin.index("command_result ReadAnnouncements"))
        report_index = plugin.index("df::global::world->status.reports", plugin.index("command_result ReadAnnouncements"))
        require(
            auth_index < report_index,
            "native announcement bridge inspects reports before authentication",
        )
        for forbidden in [
            'addFunction("RunCommand"',
            'addFunction("RunLua"',
            'addFunction("SetPauseState"',
            'addFunction("SendDigCommand"',
            'addFunction("PassKeyboardEvent"',
            "SF_ALLOW_REMOTE",
        ]:
            require(forbidden not in plugin, f"native bridge contains forbidden authority marker {forbidden}")

        wire = WIRE.read_text(encoding="utf-8")
        for marker in [
            "pub const BRIDGE_PROTOCOL_MINOR: u32 = 0;",
            "pub const ANNOUNCEMENT_PROTOCOL_MINOR: u32 = 1;",
            "pub fn negotiate_with_announcements",
            "fn encode_announcement_request",
            "fn decode_announcement_record",
            "fn decode_announcement_reply",
            "pub fn read_announcements",
            "pub fn announcement_source_identity",
            "bridge method manifest does not match the requested protocol generation",
        ]:
            require(marker in wire, f"safe-Rust announcement wire omits {marker}")
        required_wire_tests = {
            "protocol_1_0_remains_citizen_only",
            "protocol_1_1_reads_a_bounded_announcement_page",
            "duplicate_announcement_scalar_is_rejected",
            "reordered_announcement_records_are_rejected",
            "announcement_generation_and_nonce_drift_fail_closed",
            "announcement_request_bounds_fail_before_io",
            "duplicate_method_manifest_entries_are_rejected",
            "nonminimal_varints_are_rejected",
            "invalid_utf8_announcement_text_is_rejected",
        }
        require(
            required_wire_tests <= rust_tests(wire),
            "safe-Rust announcement wire omits adversarial tests",
        )

        canonical = WINDOW.read_text(encoding="utf-8")
        for marker in [
            "AnnouncementWindowAssembler",
            "can_prove_absence_in_frozen_interval",
            "retained announcement bounds or frozen high-water mark changed",
            "announcement window fields do not reproduce the stored canonical bytes",
        ]:
            require(marker in canonical, f"canonical announcement window omits {marker}")

        driver = DRIVER.read_text(encoding="utf-8")
        for marker in [
            "pub trait LiveAnnouncementSource",
            "pub fn read_complete_announcement_window",
            "frozen_high_water_mark",
            "announcement window exceeds the caller record ceiling",
            "announcement_driver_freezes_first_page_high_water",
            "announcement_driver_preserves_retained_history_loss",
        ]:
            require(marker in driver, f"announcement driver omits {marker}")

        library = LIB.read_text(encoding="utf-8")
        require(
            "pub mod live_announcements;" in library,
            "adapter crate does not compile the announcement window module",
        )
        require(
            "AnnouncementWindowAssembler" in library,
            "adapter crate does not export announcement window types",
        )
        require(
            "read_complete_announcement_window" in library,
            "adapter crate does not export the complete-window driver",
        )

        design = DESIGN.read_text(encoding="utf-8")
        for marker in [
            "retained-window witness",
            "frozen high-water mark",
            "partial coverage",
            "fresh disposable-fort evidence campaign",
        ]:
            require(marker.lower() in design.lower(), f"announcement design omits {marker}")
    except (OSError, ValueError, json.JSONDecodeError, ContractError) as exc:
        print(f"live announcement stack: FAIL: {exc}", file=sys.stderr)
        return 1
    print("live announcement stack: PASS (prospective protocol 1.1, no mutation authority)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
