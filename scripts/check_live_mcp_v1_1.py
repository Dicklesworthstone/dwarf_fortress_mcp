#!/usr/bin/env python3
"""Validate the explicitly unadmitted protocol-1.1 MCP development runtime."""

from __future__ import annotations

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CONTRACT = ROOT / "architecture/live_mcp_server_v1_1.json"
SERVER = ROOT / "crates/dfmcp-mcp/src/live_server_v1_1.rs"
MCP_ROOT = ROOT / "crates/dfmcp-mcp/src/lib.rs"
BINARY = ROOT / "crates/dwarf-fortress-mcp/src/bin/dfmcp-live-v1-1-dev-server.rs"
BINARY_MANIFEST = ROOT / "crates/dwarf-fortress-mcp/Cargo.toml"
PROCESS_TESTS = ROOT / "crates/dwarf-fortress-mcp/tests/live_v1_1_development_admission.rs"
SOURCE_CONTRACT = ROOT / "architecture/live_announcement_source_qualification_v1_1.json"
STATUS = ROOT / "docs/LIVE_ANNOUNCEMENT_IMPLEMENTATION_STATUS.md"
STREAM_DOC = ROOT / "docs/LIVE_ANNOUNCEMENT_STREAM.md"

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
EXPECTED_FORBIDDEN_ENVIRONMENT = [
    "DFMCP_ADMISSION_TICKET",
    "DFMCP_COMPATIBILITY_ENTRY_ID",
    "DFMCP_COMPATIBILITY_DECISION_DIGEST",
    "DFMCP_COMPATIBILITY_REGISTRY_DIGEST",
    "DFMCP_COMPATIBILITY_FLOOR_",
    "DFMCP_SERVER_RECEIPT_DIGEST",
    "DFMCP_ADMITTED_LAUNCH_DIGEST",
]
EXPECTED_SOURCE_DIGESTS = {
    "announcement_mcp_contract": "architecture/live_mcp_server_v1_1.json",
    "announcement_mcp_server": "crates/dfmcp-mcp/src/live_server_v1_1.rs",
    "announcement_mcp_root": "crates/dfmcp-mcp/src/lib.rs",
    "announcement_mcp_binary_manifest": "crates/dwarf-fortress-mcp/Cargo.toml",
    "announcement_mcp_binary": "crates/dwarf-fortress-mcp/src/bin/dfmcp-live-v1-1-dev-server.rs",
    "announcement_mcp_process_tests": "crates/dwarf-fortress-mcp/tests/live_v1_1_development_admission.rs",
    "announcement_mcp_checker": "scripts/check_live_mcp_v1_1.py",
    "announcement_mcp_checker_tests": "scripts/test_live_mcp_v1_1.py",
}


class ContractError(ValueError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ContractError(message)


def read_json(path: Path) -> dict[str, object]:
    value = json.loads(path.read_text(encoding="utf-8"))
    require(isinstance(value, dict), f"{path.relative_to(ROOT)} must be a JSON object")
    return value


def check_contract() -> None:
    value = read_json(CONTRACT)
    require(
        value.get("schema_version") == "dfmcp.live-mcp-server/1.1",
        "runtime contract schema drifted",
    )
    require(
        value.get("status") == "implemented_source_contract_unadmitted_runtime",
        "protocol-1.1 runtime must remain explicitly unadmitted",
    )
    binary = value.get("binary", {})
    require(isinstance(binary, dict), "runtime binary contract must be an object")
    require(binary.get("name") == "dfmcp-live-v1-1-dev-server", "runtime binary name drifted")
    require(
        binary.get("required_opt_in_environment")
        == {"DFMCP_ALLOW_UNADMITTED_LIVE_V1_1": "1"},
        "runtime opt-in contract drifted",
    )
    require(
        binary.get("forbidden_admission_environment_prefixes")
        == EXPECTED_FORBIDDEN_ENVIRONMENT,
        "runtime admission-environment refusal set drifted",
    )
    mcp = value.get("mcp", {})
    require(isinstance(mcp, dict), "runtime MCP contract must be an object")
    require(mcp.get("protocol") == "2026-07-28", "MCP protocol drifted")
    require(mcp.get("transport") == "stdio", "runtime transport drifted")
    require(mcp.get("tools") == EXPECTED_TOOLS, "frozen eleven-tool waist drifted")
    require(
        mcp.get("read_only_capabilities") == ["doctor", "observe", "query", "wait"],
        "read-only capability contract drifted",
    )
    require(mcp.get("mutation_capabilities") == [], "runtime contract grants mutation")
    require(
        mcp.get("query_modes") == ["summary", "citizens", "announcements", "all"],
        "runtime query modes drifted",
    )
    bridge = value.get("bridge", {})
    require(isinstance(bridge, dict), "runtime bridge contract must be an object")
    require(bridge.get("protocol") == "1.1", "runtime protocol generation drifted")
    require(bridge.get("version") == "0.2.0", "runtime bridge version drifted")
    require(bridge.get("plugin") == "dfmcp_bridge_v1_1", "runtime plugin generation drifted")
    require(
        bridge.get("methods") == ["Handshake", "ReadObservation"],
        "runtime bridge method waist widened",
    )
    require(bridge.get("mutation_methods") == [], "runtime bridge contract grants mutation")
    session = value.get("session", {})
    require(isinstance(session, dict), "runtime session contract must be an object")
    require(session.get("session_id_prefix_hex_byte") == "11", "runtime session namespace drifted")
    require(
        session.get("wrong_generation_session_ids_rejected") is True,
        "wrong-generation sessions are accepted",
    )
    semantics = value.get("agent_semantics", {})
    require(isinstance(semantics, dict), "runtime Agent Turn contract must be an object")
    require(
        semantics.get("complete_history_may_be_claimed") is False,
        "runtime may overclaim complete history",
    )
    require(
        semantics.get("development_and_unadmitted_state_explicit_in_every_turn") is True,
        "runtime hides development status",
    )
    require(
        semantics.get("admission_provenance_may_be_projected") is False,
        "development runtime may project admission provenance",
    )
    authority = value.get("authority", {})
    require(isinstance(authority, dict), "runtime authority contract must be an object")
    for field in ["compatibility_admitted", "server_artifact_qualified", "runtime_admitted"]:
        require(authority.get(field) is False, f"runtime contract overclaims {field}")
    require(authority.get("grants_capabilities") == [], "runtime contract grants capabilities")
    require(
        authority.get("mutation_capabilities") == [],
        "runtime contract grants mutation capability",
    )


def check_server_source() -> None:
    source = SERVER.read_text(encoding="utf-8")
    for marker in [
        'const DEVELOPMENT_OPT_IN: &str = "DFMCP_ALLOW_UNADMITTED_LIVE_V1_1";',
        'const SESSION_NAMESPACE_PREFIX: u128 = 0x11u128 << 120;',
        "validate_development_runtime_environment",
        "forbidden_admission_environment_name",
        '"runtime": "unadmitted_development"',
        '"compatibility_admitted": false',
        '"server_artifact_qualified": false',
        '"runtime_admitted": false',
        "BridgeCredentialsV1_1::new",
        "connect_authenticated_live_source_v1_1",
        "bootstrap_live_read_adapter_v1_1",
        "build_live_announcement_briefing",
        "summarize_live_announcement_change",
        '"announcements" => Ok(vec![EntityKind::Announcement])',
        "MAX_ANNOUNCEMENTS_PER_BATCH",
        "response_byte_limit",
        "final protocol-1.1 Agent Turn exceeded the negotiated response budget",
        "protocol 1.1 is read-only and registers no mutation methods",
        "run_live_v1_1_development_stdio",
        "dwarf-fortress-mcp-live-v1-1-development",
        ".tool(FortressOpenSession)",
        ".tool(FortressDoctor)",
    ]:
        require(marker in source, f"protocol-1.1 MCP server omits marker {marker}")
    for name in EXPECTED_FORBIDDEN_ENVIRONMENT:
        require(name in source, f"protocol-1.1 MCP server does not reject {name}")
    require(
        source.count("#[tool(") == 11,
        "protocol-1.1 MCP server does not expose exactly eleven tools",
    )
    for forbidden in [
        "current_admission_provenance",
        ".adapter.prepare(",
        ".adapter.commit(",
        ".adapter.request_cancel(",
        ".adapter.checkpoint(",
        ".adapter.restore(",
        "RunCommand",
        "RunLua",
        "shell=True",
    ]:
        require(
            forbidden not in source,
            f"protocol-1.1 MCP server contains forbidden path {forbidden}",
        )


def check_binary_and_exports() -> None:
    library = MCP_ROOT.read_text(encoding="utf-8")
    require(
        "mod live_server_v1_1;" in library,
        "MCP crate does not compile the protocol-1.1 server",
    )
    require(
        "pub use live_server_v1_1::run_live_v1_1_development_stdio;" in library,
        "MCP crate does not export the development runner",
    )
    require(
        "pub mod live_server_v1_1;" not in library,
        "raw protocol-1.1 tool module is public",
    )

    binary = BINARY.read_text(encoding="utf-8")
    require(
        binary.strip()
        == '#![forbid(unsafe_code)]\n\nfn main() {\n    dfmcp_mcp::run_live_v1_1_development_stdio();\n}',
        "development binary contains logic outside the reviewed MCP runner",
    )
    manifest = BINARY_MANIFEST.read_text(encoding="utf-8")
    for marker in [
        'name = "dfmcp-live-v1-1-dev-server"',
        'path = "src/bin/dfmcp-live-v1-1-dev-server.rs"',
    ]:
        require(marker in manifest, f"binary manifest omits {marker}")

    tests = PROCESS_TESTS.read_text(encoding="utf-8")
    require(tests.count("#[test]") >= 3, "development runtime needs at least three process tests")
    for marker in [
        "CARGO_BIN_EXE_dfmcp-live-v1-1-dev-server",
        "protocol_1_1_development_server_requires_exact_opt_in",
        "protocol_1_1_development_server_rejects_production_admission_state",
        "near_miss_opt_in_values_fail_before_bridge_configuration",
        'env_remove("DFMCP_ALLOW_UNADMITTED_LIVE_V1_1")',
        'env("DFMCP_COMPATIBILITY_ENTRY_ID"',
        "DFMCP_BRIDGE_TOKEN is required",
    ]:
        require(marker in tests, f"development runtime process tests omit {marker}")


def check_source_qualification_binding() -> None:
    contract = read_json(SOURCE_CONTRACT)
    gates = contract.get("required_gates", [])
    require(isinstance(gates, list), "source qualification gates must be an array")
    for gate in [
        "announcement-mcp-contract",
        "announcement-mcp-contract-tests",
        "announcement-mcp-process-tests",
    ]:
        require(gate in gates, f"source qualification omits runtime gate {gate}")
    digests = contract.get("required_source_digests", {})
    require(isinstance(digests, dict), "source qualification digests must be an object")
    for name, relative in EXPECTED_SOURCE_DIGESTS.items():
        require(
            digests.get(name) == relative,
            f"source qualification omits runtime digest {name}",
        )
    claims = contract.get("claims_established", [])
    require(isinstance(claims, list), "source qualification claims must be an array")
    require(
        any("development MCP runtime" in str(claim) for claim in claims),
        "source qualification does not state the runtime source claim",
    )
    not_established = contract.get("claims_not_established", [])
    require(
        isinstance(not_established, list),
        "source qualification nonclaims must be an array",
    )
    require(
        any("admitted protocol-1.1 MCP process" in str(claim) for claim in not_established),
        "source qualification does not refuse runtime-admission overclaim",
    )


def check_documentation() -> None:
    status = STATUS.read_text(encoding="utf-8").lower()
    stream = STREAM_DOC.read_text(encoding="utf-8").lower()
    for marker in [
        "dfmcp-live-v1-1-dev-server",
        "dfmcp_allow_unadmitted_live_v1_1=1",
        "unadmitted development",
        "summary, citizens, announcements, or all",
        "cannot consume",
    ]:
        require(marker in status, f"announcement implementation status omits {marker}")
    for marker in [
        "development mcp runtime",
        "fortress.query",
        "complete fortress history",
        "no mutation",
    ]:
        require(marker in stream, f"announcement stream documentation omits {marker}")


def main() -> int:
    try:
        check_contract()
        check_server_source()
        check_binary_and_exports()
        check_source_qualification_binding()
        check_documentation()
    except (OSError, json.JSONDecodeError, ContractError) as exc:
        print(f"protocol-1.1 MCP runtime: FAIL: {exc}", file=sys.stderr)
        return 1
    print(
        "protocol-1.1 MCP runtime: PASS "
        "(eleven-tool, announcement-aware, read-only, and explicitly unadmitted)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
