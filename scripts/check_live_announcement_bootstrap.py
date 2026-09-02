#!/usr/bin/env python3
"""Validate single-publication protocol-1.1 bootstrap and primed replay."""

from __future__ import annotations

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SOURCE_CONTRACT = ROOT / "architecture/live_announcement_source_qualification_v1_1.json"
BOOTSTRAP = ROOT / "crates/dfmcp-adapter/src/live_bootstrap_v1_1.rs"
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


def check_bootstrap() -> None:
    source = require_markers(
        BOOTSTRAP,
        [
            "pub struct LiveReadBootstrapConfigV1_1",
            "pub struct PrimedLiveSourceV1_1",
            "pub fn bootstrap_live_read_adapter_v1_1",
            "read_publishable_observation_v1_1",
            "derive_live_fortress_id(&capsule.base)",
            "let source_digest = capsule.content_digest;",
            "expected_announcement_after_id",
            "active_announcement_after_id",
            "next_citizen_offset",
            "primed replay changed announcement cursor before completing citizen pagination",
            "primed replay must begin each announcement page at citizen offset zero",
            "primed replay projection does not match the verified protocol-1.1 capsule",
            "source manifest changed between the first protocol-1.1 capsule and adapter bootstrap",
            "adapter.source().has_primed_capsule()",
            "protocol-1.1 adapter bootstrap did not preserve the verified first capsule",
        ],
    )
    ordered = [
        "let capsule = read_publishable_observation_v1_1(",
        "let source_digest = capsule.content_digest;",
        "let fortress_id = derive_live_fortress_id(&capsule.base)?;",
        "let primed = PrimedLiveSourceV1_1::new(source, capsule)?;",
        "let mut adapter = LiveReadAdapterV1_1::new(",
        "let projection_fortress_id = adapter.bootstrap()?.snapshot.fortress_id;",
        "let digest_preserved = adapter",
    ]
    offsets = [source.index(marker) for marker in ordered]
    require(
        offsets == sorted(offsets),
        "protocol-1.1 bootstrap no longer follows acquire, identify, prime, replay, verify order",
    )
    require(source.count("#[test]") >= 4, "protocol-1.1 bootstrap needs at least four tests")
    for name in [
        "bootstrap_does_not_repeat_the_underlying_publication",
        "primed_source_replays_citizen_and_announcement_pages",
        "primed_source_rejects_cursor_or_projection_drift",
        "primed_source_rejects_manifest_drift",
    ]:
        require(f"fn {name}" in source, f"protocol-1.1 bootstrap tests omit {name}")
    replay_test = source.split(
        "fn primed_source_replays_citizen_and_announcement_pages", 1
    )[1].split("#[test]", 1)[0]
    for marker in [
        "read_observation_page_v1_1(0, 2, true, -1, 2)",
        "read_observation_page_v1_1(2, 2, true, -1, 2)",
        "read_observation_page_v1_1(0, 2, true, 11, 2)",
        "read_observation_page_v1_1(2, 2, true, 11, 2)",
        "assert_eq!(primed.source().calls, 0)",
    ]:
        require(marker in replay_test, f"two-dimensional primed replay test omits {marker}")


def check_exports_and_source_identity() -> None:
    library = require_markers(
        LIBRARY,
        [
            "pub mod live_bootstrap_v1_1;",
            "DEFAULT_LIVE_ANNOUNCEMENT_PAGE_SIZE",
            "DEFAULT_MAX_LIVE_ANNOUNCEMENTS",
            "LiveReadBootstrapConfigV1_1",
            "PrimedLiveSourceV1_1",
            "bootstrap_live_read_adapter_v1_1",
        ],
    )
    require(
        library.index("pub mod live_bootstrap;")
        < library.index("pub mod live_bootstrap_v1_1;"),
        "protocol-1.1 bootstrap is not adjacent to the baseline bootstrap module",
    )

    contract = json.loads(SOURCE_CONTRACT.read_text(encoding="utf-8"))
    mapping = contract.get("required_source_digests")
    require(isinstance(mapping, dict), "announcement source mapping must be an object")
    require(
        mapping.get("announcement_bootstrap")
        == "crates/dfmcp-adapter/src/live_bootstrap_v1_1.rs",
        "announcement source identity omits the protocol-1.1 bootstrap",
    )
    require(
        mapping.get("bootstrap_checker")
        == "scripts/check_live_announcement_bootstrap.py",
        "announcement source identity omits the bootstrap checker",
    )
    claims = contract.get("claims_established")
    require(isinstance(claims, list), "announcement source claims must be an array")
    require(
        any(
            isinstance(claim, str)
            and "without a duplicate underlying bridge read" in claim
            for claim in claims
        ),
        "announcement source contract omits the one-publication bootstrap claim",
    )


def check_status() -> None:
    source = STATUS.read_text(encoding="utf-8").lower()
    for marker in [
        "single-publication bootstrap",
        "without another underlying bridge read",
        "two-dimensional",
        "citizen pagination",
        "announcement continuation",
        "still unadmitted",
    ]:
        require(marker in source, f"announcement implementation status omits {marker}")


def main() -> int:
    try:
        check_bootstrap()
        check_exports_and_source_identity()
        check_status()
    except (OSError, ValueError, json.JSONDecodeError, ContractError) as exc:
        print(f"live announcement bootstrap: FAIL: {exc}", file=sys.stderr)
        return 1
    print(
        "live announcement bootstrap: PASS "
        "(one publication, exact replay, identity preserved, and no extra bridge read)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
