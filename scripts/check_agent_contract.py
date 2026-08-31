#!/usr/bin/env python3
"""Fail-closed static checks for the agent operating-model contract.

This checker intentionally runs before Cargo. It detects semantic drift between
machine registries, typed core vocabulary, the JSON presentation builder, the
registered eleven-tool facade, and the normative documentation. It does not
claim Rust compilation or behavioral correctness; it makes a narrower class of
architectural regressions cheap and deterministic to catch.
"""

from __future__ import annotations

import json
import pathlib
import re
import sys
from dataclasses import dataclass

ROOT = pathlib.Path(__file__).resolve().parents[1]

EXPECTED_PHASES = [
    "bootstrap",
    "orient",
    "inspect",
    "formulate",
    "propose",
    "compare",
    "commit",
    "verify",
    "learn",
    "handoff",
    "reconcile",
]
EXPECTED_CONTINUITY = [
    "bootstrap",
    "continuous",
    "heartbeat",
    "partial",
    "gap",
    "reset",
    "stale",
    "indeterminate",
]
EXPECTED_EPISTEMIC = [
    "observed",
    "certified_derived",
    "inferred",
    "predicted",
    "assumed",
    "stale",
    "unknown",
    "contradicted",
    "indeterminate",
]
EXPECTED_RECOVERY = [
    "never_unchanged",
    "safe_read_retry",
    "refresh_and_retry",
    "rebase_required",
    "backoff",
    "reconciliation_required",
    "confirmation_required",
    "operator_action_required",
]
EXPECTED_PROFILES = ["pulse", "briefing", "tactical", "forensic", "custom"]
EXPECTED_TOOLS = [
    "fortress.open_session",
    "fortress.observe",
    "fortress.query",
    "fortress.plan",
    "fortress.commit",
    "fortress.wait",
    "fortress.cancel",
    "fortress.checkpoint",
    "fortress.restore",
    "fortress.explain",
    "fortress.doctor",
]
EXPECTED_PACKET_FIELDS = [
    "schema",
    "operation",
    "phase",
    "session_id",
    "turn_id",
    "request_id",
    "anchor",
    "continuity",
    "profile",
    "briefing",
    "changes",
    "attention",
    "active_work",
    "affordances",
    "recommendations",
    "uncertainty",
    "coverage",
    "budget",
    "references",
]
EXPECTED_INVARIANTS = 13


@dataclass(frozen=True)
class Failure:
    path: str
    message: str


def read_text(relative: str) -> str:
    return (ROOT / relative).read_text(encoding="utf-8")


def require(condition: bool, path: str, message: str, failures: list[Failure]) -> None:
    if not condition:
        failures.append(Failure(path, message))


def check_registry(failures: list[Failure]) -> None:
    path = "architecture/agent_turn_contract.json"
    try:
        registry = json.loads(read_text(path))
    except (OSError, json.JSONDecodeError) as exc:
        failures.append(Failure(path, f"cannot load registry: {exc}"))
        return

    require(
        registry.get("schema_version") == "dfmcp.agent_turn_contract/1",
        path,
        "schema_version must be dfmcp.agent_turn_contract/1",
        failures,
    )
    require(
        registry.get("phases") == EXPECTED_PHASES,
        path,
        "phase registry drifted from the frozen order",
        failures,
    )
    require(
        registry.get("continuity_statuses") == EXPECTED_CONTINUITY,
        path,
        "continuity registry drifted",
        failures,
    )
    require(
        registry.get("epistemic_states") == EXPECTED_EPISTEMIC,
        path,
        "epistemic registry drifted",
        failures,
    )
    require(
        registry.get("recovery_classes") == EXPECTED_RECOVERY,
        path,
        "recovery registry drifted",
        failures,
    )
    require(
        registry.get("field_order") == EXPECTED_PACKET_FIELDS,
        path,
        "machine packet field order drifted from the executable builder",
        failures,
    )
    profiles = registry.get("observation_profiles")
    require(
        isinstance(profiles, dict) and list(profiles) == EXPECTED_PROFILES,
        path,
        "observation profile registry drifted",
        failures,
    )
    identity = registry.get("identity_rules")
    require(isinstance(identity, dict), path, "identity_rules must be an object", failures)
    if isinstance(identity, dict):
        require(
            identity.get("aliasing_forbidden") is True,
            path,
            "turn_id/request_id aliasing must remain forbidden",
            failures,
        )
        require(
            isinstance(identity.get("turn_id"), str) and isinstance(identity.get("request_id"), str),
            path,
            "both identity meanings must remain explicit",
            failures,
        )
    required = registry.get("required_invariants")
    require(
        isinstance(required, list) and len(required) == EXPECTED_INVARIANTS,
        path,
        f"exactly {EXPECTED_INVARIANTS} agent invariants must remain registered",
        failures,
    )
    if isinstance(required, list):
        ids = [entry.get("id") for entry in required if isinstance(entry, dict)]
        require(
            ids == [f"AGENT-INV-{index:03d}" for index in range(1, EXPECTED_INVARIANTS + 1)],
            path,
            "agent invariant IDs must be complete, contiguous, and canonically ordered",
            failures,
        )
        statements = [entry.get("statement", "") for entry in required if isinstance(entry, dict)]
        require(
            any("silently aliased" in statement for statement in statements),
            path,
            "identity-separation invariant is missing",
            failures,
        )
    authority = registry.get("authority_rules")
    require(isinstance(authority, dict), path, "authority_rules must be an object", failures)
    if isinstance(authority, dict):
        for key in [
            "attention_can_authorize",
            "recommendation_can_authorize",
            "memory_can_authorize",
            "counterfactual_can_authorize",
            "typed_affordance_can_authorize",
        ]:
            require(
                authority.get(key) is False,
                path,
                f"{key} must remain false",
                failures,
            )
        require(
            authority.get("prepared_plan_plus_valid_witnesses_capabilities_and_fences_required")
            is True,
            path,
            "the positive mutation-authority rule is missing",
            failures,
        )


def check_core(failures: list[Failure]) -> None:
    path = "crates/dfmcp-core/src/agent.rs"
    source = read_text(path)
    vocabulary = set(re.findall(r'"([a-z0-9_.]+)"', source))
    for value in (
        EXPECTED_PHASES
        + EXPECTED_CONTINUITY
        + EXPECTED_EPISTEMIC
        + EXPECTED_RECOVERY
        + EXPECTED_PROFILES
        + EXPECTED_TOOLS
    ):
        require(
            value in vocabulary,
            path,
            f"typed core vocabulary is missing {value}",
            failures,
        )
    require(
        "pub const ALL: [Self; 11]" in source,
        path,
        "FortressTool must freeze the eleven-tool waist",
        failures,
    )
    require(
        "may_satisfy_mutation_precondition" in source,
        path,
        "epistemic precondition gate is missing",
        failures,
    )
    require(
        "can_prove_absence" in source and "proves_absence_in" in source,
        path,
        "coverage/absence law is missing",
        failures,
    )
    require(
        source.count("can_dispatch_effect") >= 2,
        path,
        "affordance and recommendation non-dispatch laws are missing",
        failures,
    )
    require(
        "can_grant_authority" in source and "can_satisfy_live_precondition" in source,
        path,
        "memory non-authority laws are missing",
        failures,
    )
    require(
        "SurpriseRecord" in source and "HandoffPacket" in source,
        path,
        "surprise or handoff typed model is missing",
        failures,
    )

    ids_path = "crates/dfmcp-core/src/ids.rs"
    ids_source = read_text(ids_path)
    for identity_name in [
        "ObjectiveId",
        "AttentionId",
        "AffordanceId",
        "RecommendationId",
        "SurpriseId",
        "MemoryId",
        "HandoffId",
    ]:
        require(
            f"id_u128!({identity_name});" in ids_source,
            ids_path,
            f"missing stable {identity_name}",
            failures,
        )

    lib_path = "crates/dfmcp-core/src/lib.rs"
    lib_source = read_text(lib_path)
    require(
        "AffordanceId" in lib_source,
        lib_path,
        "the dedicated affordance identity is not exported",
        failures,
    )


def check_packet_builder(failures: list[Failure]) -> None:
    path = "crates/dfmcp-mcp/src/agent_turn.rs"
    source = read_text(path)
    require(
        'AGENT_TURN_SCHEMA: &str = "dfmcp.agent_turn/1"' in source,
        path,
        "packet schema constant drifted",
        failures,
    )
    require(
        "pub use dfmcp_core::{AgentPhase, ContinuityStatus, ObservationProfile, RecoveryClass};"
        in source,
        path,
        "MCP must reuse core semantic enums",
        failures,
    )
    for field in EXPECTED_PACKET_FIELDS:
        require(
            f'"{field}"' in source,
            path,
            f"packet builder is missing {field}",
            failures,
        )
    require(
        "pub fn turn_id" in source and "pub fn request_id" in source,
        path,
        "turn_id and semantic request_id must remain separate",
        failures,
    )
    require(
        "presentation_turn_and_semantic_request_are_not_aliased" in source,
        path,
        "identity-separation regression test is missing",
        failures,
    )


def check_facade(failures: list[Failure]) -> None:
    path = "crates/dfmcp-mcp/src/agent_facade.rs"
    source = read_text(path)
    require(
        '.turn_id(format!("presentation-turn-{turn_sequence}"))' in source,
        path,
        "facade must set turn_id, not semantic request_id",
        failures,
    )
    require(
        '.request_id(format!("presentation-turn-' not in source,
        path,
        "facade aliases presentation turn to semantic request_id",
        failures,
    )
    require(
        "semantic-request-id-not-projected" in source,
        path,
        "facade must disclose the missing semantic request ID",
        failures,
    )
    require(
        "last_checkpoint_id" in source and "checkpoint_is_visible_in_later_briefings" in source,
        path,
        "checkpoint handoff context is not visible or tested",
        failures,
    )
    for tool in EXPECTED_TOOLS:
        rust_name = tool.replace(".", "_")
        require(
            f"pub fn {rust_name}(" in source,
            path,
            f"facade is missing wrapper {rust_name}",
            failures,
        )
    registrations = re.findall(r"\.tool\((Fortress[A-Za-z]+)\)", source)
    require(
        len(registrations) == 11 and len(set(registrations)) == 11,
        path,
        "run_stdio must register exactly eleven unique facade tools",
        failures,
    )
    require(
        "crate::server::fortress_" in source,
        path,
        "facade must delegate to authority-bearing handlers",
        failures,
    )
    require(
        "can_dispatch" not in source,
        path,
        "presentation facade must not grow an effect-dispatch path",
        failures,
    )

    lib_path = "crates/dfmcp-mcp/src/lib.rs"
    lib_source = read_text(lib_path)
    require(
        "pub mod agent_facade;" in lib_source,
        lib_path,
        "agent facade module is not compiled",
        failures,
    )
    require(
        "pub use agent_facade::run_stdio;" in lib_source,
        lib_path,
        "agent facade is not the default server",
        failures,
    )
    require(
        "agent_server" not in lib_source,
        lib_path,
        "superseded facade remains wired",
        failures,
    )
    require(
        not (ROOT / "crates/dfmcp-mcp/src/agent_server.rs").exists(),
        lib_path,
        "superseded agent_server.rs must be deleted",
        failures,
    )


def check_docs(failures: list[Failure]) -> None:
    required_docs = {
        "AGENTS.md": ["docs/AGENT_OPERATING_MODEL.md", "Agent Turn Packet"],
        "ARCHITECTURE.md": ["Agent control loop", "Epistemic separation"],
        "MCP_SURFACE.md": ["Common Agent Turn Packet", "Observation profiles"],
        "IMPLEMENTATION_STATUS.md": [
            "Agent-operating-model status",
            "unqualified source changes",
        ],
        "docs/AGENT_OPERATING_MODEL.md": [
            "The canonical Agent Turn Packet",
            "Value-of-information planning",
        ],
    }
    for path, needles in required_docs.items():
        source = read_text(path)
        for needle in needles:
            require(
                needle in source,
                path,
                f"missing normative agent text: {needle}",
                failures,
            )


def main() -> int:
    failures: list[Failure] = []
    check_registry(failures)
    check_core(failures)
    check_packet_builder(failures)
    check_facade(failures)
    check_docs(failures)

    if failures:
        print(f"agent contract: FAIL ({len(failures)} violation(s))", file=sys.stderr)
        for failure in failures:
            print(f"  {failure.path}: {failure.message}", file=sys.stderr)
        return 1

    print(
        "agent contract: PASS "
        f"({len(EXPECTED_TOOLS)} tools, {len(EXPECTED_PACKET_FIELDS)} packet fields, "
        f"{len(EXPECTED_EPISTEMIC)} epistemic states, {EXPECTED_INVARIANTS} invariants)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
