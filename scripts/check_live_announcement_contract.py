#!/usr/bin/env python3
"""Validate the prospective bounded live-announcement generation."""

from __future__ import annotations

import ast
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CONTRACT_PATH = ROOT / "architecture/live_announcement_read_v1.json"
MODULE_PATH = ROOT / "crates/dfmcp-adapter/src/live_announcements.rs"
PROTO_PATH = ROOT / "bridge/dfhack-plugin/proto/DfmcpBridge.proto"
DESIGN_PATH = ROOT / "docs/LIVE_ANNOUNCEMENT_READ_GENERATION.md"


class ContractError(ValueError):
    pass


def fail(message: str) -> None:
    raise ContractError(message)


def require(condition: bool, message: str) -> None:
    if not condition:
        fail(message)


def rust_test_names(source: str) -> set[str]:
    tree = ast.parse("\n".join(
        line for line in source.splitlines()
        if not line.lstrip().startswith("#![")
    )) if False else None
    del tree
    names = set()
    lines = source.splitlines()
    for index, line in enumerate(lines[:-1]):
        if line.strip() == "#[test]":
            next_line = lines[index + 1].strip()
            if next_line.startswith("fn "):
                names.add(next_line[3:].split("(", 1)[0])
    return names


def main() -> int:
    try:
        contract = json.loads(CONTRACT_PATH.read_text(encoding="utf-8"))
        require(
            contract.get("schema_version") == "dfmcp.live_announcement_read/1",
            "announcement contract schema drifted",
        )
        require(
            contract.get("status") == "prospective_unadmitted",
            "announcement contract must remain explicitly unadmitted",
        )
        bridge = contract.get("bridge_protocol", {})
        require(bridge.get("required_major") == 1, "announcement major protocol drifted")
        require(bridge.get("required_minor") == 1, "announcement minor protocol drifted")
        require(bridge.get("method") == "ReadAnnouncements", "announcement method drifted")
        require(bridge.get("effect") == "read_only", "announcement method is not read-only")
        require(bridge.get("mutation_capabilities") == [], "announcement contract admits mutation")
        request = contract.get("request", {})
        require(
            request.get("after_report_id", {}).get("bootstrap_value") == -1,
            "announcement bootstrap cursor drifted",
        )
        require(
            request.get("through_report_id", {}).get("select_current_high_water_value") == -1,
            "announcement high-water selection sentinel drifted",
        )
        maximum = request.get("max_announcements", {})
        require(maximum.get("minimum") == 1, "announcement minimum page size drifted")
        require(maximum.get("maximum") == 4096, "announcement maximum page size drifted")
        coverage = contract.get("coverage", {})
        require(
            coverage.get("domain") == "fortress.announcements.retained_window",
            "announcement coverage domain drifted",
        )
        require(
            "complete fortress announcement history" in coverage.get("never_claims", ""),
            "announcement coverage no longer forbids complete-history overclaiming",
        )

        source = MODULE_PATH.read_text(encoding="utf-8")
        for marker in [
            "AnnouncementSourceIdentity",
            "AnnouncementRecord",
            "AnnouncementPage",
            "AnnouncementWindowAssembler",
            "LiveAnnouncementWindow",
            "history_truncated",
            "window_latest_report_id",
            "can_prove_absence_in_frozen_interval",
            "dfmcp.live-announcement-window.v1",
            "MAX_ANNOUNCEMENTS_PER_PAGE",
            "MAX_ANNOUNCEMENT_TEXT_BYTES",
            "MAX_ANNOUNCEMENT_WINDOW_RECORDS",
            "MAX_CANONICAL_ANNOUNCEMENT_BYTES",
        ]:
            require(marker in source, f"announcement module omits {marker}")
        for forbidden in [
            "unsafe {",
            "Capability::ControlClock",
            "Capability::Designate",
            "Capability::SetLabor",
            "Capability::Military",
            "RunCommand",
            "RunLua",
        ]:
            require(forbidden not in source, f"announcement module contains forbidden authority marker {forbidden}")
        tests = rust_test_names(source)
        required_tests = {
            "pagination_does_not_change_window_identity",
            "retained_history_loss_is_explicit_partial_coverage",
            "appended_or_pruned_window_drift_is_rejected_without_partial_mutation",
            "cross_page_reordering_and_cursor_gaps_fail_closed",
            "nonterminal_short_or_empty_page_is_rejected",
            "malformed_record_semantics_fail_closed",
            "empty_retained_window_is_canonical",
            "structured_or_canonical_byte_tampering_invalidates_window",
            "source_protocol_and_generation_are_part_of_identity",
        }
        require(required_tests <= tests, "announcement module omits one or more adversarial tests")

        proto = PROTO_PATH.read_text(encoding="utf-8")
        for marker in [
            "RPC ReadAnnouncements",
            "message ReadAnnouncementsRequest",
            "message AnnouncementRecord",
            "message ReadAnnouncementsReply",
            "optional sint32 after_report_id = 5 [default = -1]",
            "optional sint32 through_report_id = 6 [default = -1]",
            "repeated AnnouncementRecord announcements = 15",
        ]:
            require(marker in proto, f"announcement protobuf contract omits {marker}")
        design = DESIGN_PATH.read_text(encoding="utf-8")
        for marker in [
            "retained-window witness",
            "frozen high-water mark",
            "history_truncated",
            "partial coverage",
            "no mutation methods",
            "fresh disposable-fort evidence campaign",
        ]:
            require(marker.lower() in design.lower(), f"announcement design omits {marker}")
    except (OSError, json.JSONDecodeError, ContractError) as exc:
        print(f"live announcement contract: FAIL: {exc}", file=sys.stderr)
        return 1
    print("live announcement contract: PASS (prospective retained-window generation)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
