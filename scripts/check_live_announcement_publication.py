#!/usr/bin/env python3
"""Validate transactional protocol-1.1 publication and read-only adapter semantics."""

from __future__ import annotations

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SOURCE_CONTRACT = ROOT / "architecture/live_announcement_source_qualification_v1_1.json"
NATIVE = ROOT / "bridge/dfhack-plugin/src/dfmcp_bridge_v1_1.cpp"
BATCH = ROOT / "crates/dfmcp-adapter/src/live_announcement_batch.rs"
PUBLICATION = ROOT / "crates/dfmcp-adapter/src/live_observation_publication_v1_1.rs"
ADAPTER = ROOT / "crates/dfmcp-adapter/src/live_adapter_v1_1.rs"
LIBRARY = ROOT / "crates/dfmcp-adapter/src/lib.rs"
STATUS = ROOT / "docs/LIVE_ANNOUNCEMENT_IMPLEMENTATION_STATUS.md"


class ContractError(ValueError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ContractError(message)


def require_markers(path: Path, markers: list[str]) -> str:
    source = path.read_text(encoding="utf-8")
    for marker in markers:
        require(marker in source, f"{path.relative_to(ROOT)} omits {marker}")
    return source


def check_native_and_batch_cursor_fences() -> None:
    native = require_markers(
        NATIVE,
        [
            'out->set_failure_code("ANNOUNCEMENT_CURSOR_AHEAD")',
            "announcement cursor is ahead of the empty retained report window",
            "announcement cursor is ahead of the retained report high-water mark",
            "if (requested_after > latest)",
        ],
    )
    require(
        native.count('out->set_failure_code("ANNOUNCEMENT_CURSOR_AHEAD")') == 2,
        "native bridge must reject cursor-ahead state for empty and nonempty retained windows",
    )

    batch = require_markers(
        BATCH,
        [
            "retained_empty && self.requested_after_id != -1",
            "self.requested_after_id > self.latest_available_id",
            "announcement cursor is ahead of an empty retained report window",
            "announcement cursor is ahead of the retained report high-water mark",
            "cursor_ahead_of_retained_high_water_is_rejected",
        ],
    )
    require(batch.count("#[test]") >= 9, "announcement batch needs at least nine focused tests")


def check_publication_transaction() -> None:
    source = require_markers(
        PUBLICATION,
        [
            "pub struct LiveObservationPublicationConfigV1_1",
            "pub fn read_publishable_observation_v1_1",
            "read_complete_observation_v1_1_bounded",
            "expected_base != &base",
            "retained announcement window changed during transactional publication",
            "batch.coverage.requested_after_id != cursor",
            "partial announcement page did not fill the requested page size",
            "completing a multi-page announcement suffix requires a paused fortress",
            "retained announcement suffix exceeds the publication ceiling",
            "next_announcement_after_id",
            "fn combine_capsule",
            "capsule.validate()?",
        ],
    )
    require(source.count("#[test]") >= 7, "publication transaction needs at least seven tests")
    for name in [
        "announcement_transport_pagination_does_not_change_capsule_identity",
        "partial_suffix_requires_a_paused_fortress_before_followup",
        "citizen_or_clock_drift_aborts_without_a_capsule",
        "retained_window_drift_aborts_without_a_capsule",
        "incomplete_page_must_fill_the_requested_announcement_size",
        "publication_ceiling_fails_with_a_resume_cursor",
        "initial_retained_window_gap_survives_complete_publication",
    ]:
        require(f"fn {name}" in source, f"publication tests omit {name}")
    private_segment = source.split("fn combine_capsule", 1)[1].split("#[cfg(test)]", 1)[0]
    require(
        "pub fn" not in private_segment,
        "capsule construction helper must remain private to the publication seam",
    )


def check_read_only_adapter() -> None:
    source = require_markers(
        ADAPTER,
        [
            "pub struct LiveReadAdapterConfigV1_1",
            "pub struct LiveReadAdapterV1_1",
            "read_publishable_observation_v1_1",
            "impl<T: LiveObservationSourceV1_1> GameAdapter for LiveReadAdapterV1_1<T>",
            'bridge_protocol_version: "dfmcp-bridge/1.1"',
            "CompatibilityLevel::DegradedReadOnly",
            "Capability::Observe",
            "Capability::Query",
            "Capability::Doctor",
            "protocol-1.1 adapter publishes a complete configured suffix",
            "protocol-1.1 source switched world or fortress identity",
            "protocol-1.1 bridge or game version manifest changed",
            "ensure_snapshot_budget(request, &projection)?;",
            "self.current = Some(projection);",
            "DFHack adapter protocol 1.1 is read-only",
            "announcement coverage is a retained suffix, never complete fortress history",
        ],
    )
    require(
        source.index("ensure_snapshot_budget(request, &projection)?;")
        < source.rindex("self.current = Some(projection);"),
        "protocol-1.1 adapter publishes candidate state before final budget admission",
    )
    for method in [
        'read_only_rejection("prepare")',
        'read_only_rejection("commit")',
        'read_only_rejection("action polling")',
        'read_only_rejection("cancellation")',
        'read_only_rejection("cancellation finalization")',
        'read_only_rejection("checkpoint")',
        'read_only_rejection("restore")',
    ]:
        require(method in source, f"protocol-1.1 adapter omits fail-closed mutation path {method}")
    require(source.count("#[test]") >= 11, "protocol-1.1 adapter needs at least eleven tests")
    for name in [
        "bootstrap_publishes_citizens_and_announcements_under_one_anchor",
        "unchanged_combined_capsule_becomes_a_heartbeat",
        "new_announcement_advances_sequence",
        "partial_suffix_is_completed_before_bootstrap_publication",
        "over_ceiling_suffix_leaves_adapter_unbootstrapped",
        "retained_window_gap_is_visible_but_history_is_not_upgraded",
        "bridge_restart_advances_epoch",
        "candidate_over_budget_does_not_advance_anchor",
        "pinned_query_returns_announcement_entities_with_evidence",
        "mutation_surface_remains_absent",
        "hard_configuration_bounds_are_rejected",
    ]:
        require(f"fn {name}" in source, f"protocol-1.1 adapter tests omit {name}")
    candidate_test = source.split("fn candidate_over_budget_does_not_advance_anchor", 1)[1].split(
        "#[test]", 1
    )[0]
    require(
        "complete_page(42, 12_345, &[0], &[])" in candidate_test
        and "complete_page(42, 12_346, &[0], &[10])" in candidate_test,
        "candidate-over-budget test does not isolate growth beyond an already-admitted snapshot",
    )


def check_crate_and_source_identity() -> None:
    library = require_markers(
        LIBRARY,
        [
            "pub mod api;",
            "pub use api::*;",
            "pub mod live_adapter_v1_1;",
            "pub mod live_observation_publication_v1_1;",
            "pub use live_adapter_v1_1::{LiveReadAdapterConfigV1_1, LiveReadAdapterV1_1};",
            "LiveObservationPublicationConfigV1_1, read_publishable_observation_v1_1",
        ],
    )
    require(
        "pub enum CompatibilityLevel" not in library,
        "adapter API definitions remain duplicated in the crate root",
    )

    contract = json.loads(SOURCE_CONTRACT.read_text(encoding="utf-8"))
    mapping = contract.get("required_source_digests", {})
    require(isinstance(mapping, dict), "announcement source mapping must be an object")
    expected = {
        "adapter_api": "crates/dfmcp-adapter/src/api.rs",
        "adapter_root": "crates/dfmcp-adapter/src/lib.rs",
        "announcement_batch": "crates/dfmcp-adapter/src/live_announcement_batch.rs",
        "announcement_publication": "crates/dfmcp-adapter/src/live_observation_publication_v1_1.rs",
        "announcement_adapter": "crates/dfmcp-adapter/src/live_adapter_v1_1.rs",
        "publication_checker": "scripts/check_live_announcement_publication.py",
    }
    for name, relative in expected.items():
        require(mapping.get(name) == relative, f"announcement source identity omits {name}")
    claims = contract.get("claims_established", [])
    require(
        any("transactional protocol-1.1 publication" in claim for claim in claims),
        "source qualification does not state the new adapter claim",
    )


def check_status() -> None:
    source = STATUS.read_text(encoding="utf-8").lower()
    for marker in [
        "implemented in source",
        "still unadmitted",
        "transactional multi-page announcement publication",
        "livereadadapterv1_1",
        "rejects a cursor ahead",
        "returns no capsule",
        "candidate projection and budget validation before state publication",
        "no fresh passing source qualification receipt",
        "current normal admitted mcp server is still protocol 1.0",
        "no mutation authority",
    ]:
        require(marker in source, f"announcement implementation status omits {marker}")


def main() -> int:
    try:
        check_native_and_batch_cursor_fences()
        check_publication_transaction()
        check_read_only_adapter()
        check_crate_and_source_identity()
        check_status()
    except (OSError, json.JSONDecodeError, ContractError) as exc:
        print(f"live announcement publication: FAIL: {exc}", file=sys.stderr)
        return 1
    print(
        "live announcement publication: PASS "
        "(cursor-safe, transactional, budget-before-publish, and read-only)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
