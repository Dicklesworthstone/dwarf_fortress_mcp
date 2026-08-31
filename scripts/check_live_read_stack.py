#!/usr/bin/env python3
"""Fail-closed static contract for the complete authenticated live-read stack.

This does not replace compilation, the isolated native DFHack build, or a
running disposable-fort test. It cheaply rejects cross-layer drift and known
bypass classes before those more expensive gates run.
"""

from __future__ import annotations

import json
import pathlib
import re
import sys
from dataclasses import dataclass

ROOT = pathlib.Path(__file__).resolve().parents[1]

PRODUCTION_RUST = [
    "crates/dfmcp-adapter/src/asupersync_connector.rs",
    "crates/dfmcp-adapter/src/dfhack_rpc.rs",
    "crates/dfmcp-adapter/src/live_briefing.rs",
    "crates/dfmcp-adapter/src/live_compatibility.rs",
    "crates/dfmcp-adapter/src/live_evidence.rs",
    "crates/dfmcp-adapter/src/live_observation.rs",
    "crates/dfmcp-adapter/src/live_projection.rs",
    "crates/dfmcp-adapter/src/live_read_adapter.rs",
    "crates/dfmcp-adapter/src/live_session.rs",
    "crates/dfmcp-adapter/src/live_version.rs",
    "crates/dfmcp-mcp/src/live_agent.rs",
    "crates/dfmcp-mcp/src/live_bootstrap.rs",
    "crates/dfmcp-mcp/src/read_session.rs",
]

EXPECTED_RPC_METHODS = ["Handshake", "ReadObservation"]
FORBIDDEN_NATIVE = [
    "SF_ALLOW_REMOTE",
    'addFunction("RunCommand"',
    'addFunction("RunLua"',
    'addFunction("Pause"',
    'addFunction("Resume"',
    'addFunction("Dig"',
    'addFunction("Teleport"',
    'addFunction("Mutate"',
]
FORBIDDEN_PRODUCTION_TOKENS = [
    "unsafe {",
    ".unwrap()",
    ".expect(",
    "todo!",
    "unimplemented!",
]


@dataclass(frozen=True)
class Failure:
    path: str
    message: str


def read(relative: str) -> str:
    return (ROOT / relative).read_text(encoding="utf-8")


def require(condition: bool, path: str, message: str, failures: list[Failure]) -> None:
    if not condition:
        failures.append(Failure(path, message))


def check_registry(failures: list[Failure]) -> None:
    path = "architecture/dfhack_read_bridge_v1.json"
    try:
        value = json.loads(read(path))
    except (OSError, json.JSONDecodeError) as exc:
        failures.append(Failure(path, f"cannot parse registry: {exc}"))
        return
    require(value.get("schema_version") == "dfmcp.dfhack_read_bridge/1", path,
            "schema version drifted", failures)
    methods = value.get("methods")
    names = [entry.get("name") for entry in methods] if isinstance(methods, list) else []
    require(names == EXPECTED_RPC_METHODS, path,
            "registry must expose exactly Handshake and ReadObservation", failures)
    if isinstance(methods, list):
        for method in methods:
            require(method.get("effect") == "read_only", path,
                    f"{method.get('name')} is not read_only", failures)
            require(method.get("requires_authentication") is True, path,
                    f"{method.get('name')} is not authenticated", failures)
    transport = value.get("transport")
    require(isinstance(transport, dict) and transport.get("remote_access") is False,
            path, "remote access must remain false", failures)
    citizen_fields = value.get("observation_fields", {}).get("citizen", [])
    require("military" not in citizen_fields, path,
            "V1 reintroduced unstable direct military state", failures)


def check_native(failures: list[Failure]) -> None:
    cpp_path = "bridge/dfhack-plugin/src/dfmcp_bridge.cpp"
    cpp = read(cpp_path)
    registrations = re.findall(r'addFunction\("([A-Za-z0-9_]+)"', cpp)
    require(registrations == EXPECTED_RPC_METHODS, cpp_path,
            "native plugin RPC registrations drifted", failures)
    require(cpp.count("authenticate(in->bearer_token()") == 2, cpp_path,
            "every native RPC must authenticate", failures)
    require("constant_time_equal" in cpp, cpp_path,
            "constant-time token comparison is missing", failures)
    require("std::sort(citizens.begin(), citizens.end()" in cpp, cpp_path,
            "citizen ordering is not canonical before pagination", failures)
    for token in FORBIDDEN_NATIVE:
        require(token not in cpp, cpp_path,
                f"forbidden native route or flag present: {token}", failures)

    proto_path = "bridge/dfhack-plugin/proto/DfmcpBridge.proto"
    proto = read(proto_path)
    rpc_comments = re.findall(r"// RPC\s+([A-Za-z0-9_]+)\s*:", proto)
    require(rpc_comments == EXPECTED_RPC_METHODS, proto_path,
            "protobuf RPC declarations drifted", failures)
    require(proto.count("required bytes bearer_token") == 2, proto_path,
            "both requests must carry authentication", failures)

    cmake_path = "bridge/dfhack-plugin/CMakeLists.txt"
    cmake = read(cmake_path)
    require("dfhack_plugin(" in cmake and "PROTOBUFS" in cmake, cmake_path,
            "plugin is not built through DFHack's native macro", failures)
    require(not (ROOT / "bridge/dfhack-plugin/include/dfmcp_ipc.h").exists(),
            cmake_path, "obsolete custom socket header reappeared", failures)


def check_workspace_and_exports(failures: list[Failure]) -> None:
    cargo_path = "Cargo.toml"
    cargo = read(cargo_path)
    match = re.search(
        r'asupersync\s*=\s*\{[^}]*git\s*=\s*"https://github.com/Dicklesworthstone/asupersync"[^}]*rev\s*=\s*"([0-9a-f]{40})"[^}]*\}',
        cargo,
    )
    require(match is not None, cargo_path,
            "asupersync must be exact-revision pinned from the owned repository", failures)
    adapter_cargo = read("crates/dfmcp-adapter/Cargo.toml")
    mcp_cargo = read("crates/dfmcp-mcp/Cargo.toml")
    require("asupersync.workspace = true" in adapter_cargo,
            "crates/dfmcp-adapter/Cargo.toml", "adapter does not use asupersync", failures)
    require("asupersync.workspace = true" in mcp_cargo,
            "crates/dfmcp-mcp/Cargo.toml", "MCP bootstrap does not use asupersync", failures)

    adapter_lib_path = "crates/dfmcp-adapter/src/lib.rs"
    adapter_lib = read(adapter_lib_path)
    for module in [
        "asupersync_connector",
        "dfhack_rpc",
        "live_briefing",
        "live_compatibility",
        "live_evidence",
        "live_observation",
        "live_projection",
        "live_read_adapter",
        "live_session",
        "live_version",
    ]:
        require(f"pub mod {module};" in adapter_lib, adapter_lib_path,
                f"live module {module} is not compiled", failures)
    for symbol in [
        "LiveBridgeEndpoint",
        "DfHackRpcClient",
        "LiveCompatibilityPolicy",
        "LiveObservationCapsule",
        "LiveObservationReceipt",
        "LiveReadAdapter",
        "LiveVersionTracker",
        "build_live_briefing",
        "project_live_observation",
    ]:
        require(symbol in adapter_lib, adapter_lib_path,
                f"live symbol {symbol} is not exported", failures)

    mcp_lib_path = "crates/dfmcp-mcp/src/lib.rs"
    mcp_lib = read(mcp_lib_path)
    for module in ["live_agent", "live_bootstrap", "read_session"]:
        require(f"pub mod {module};" in mcp_lib, mcp_lib_path,
                f"MCP live module {module} is not compiled", failures)
    require("open_context_bound_live_read_session" in mcp_lib, mcp_lib_path,
            "context-bound live bootstrap is not exported", failures)


def check_rust_contracts(failures: list[Failure]) -> None:
    for path in PRODUCTION_RUST:
        source = read(path)
        require("#![forbid(unsafe_code)]" in source, path,
                "safe-Rust prohibition is missing", failures)
        for token in FORBIDDEN_PRODUCTION_TOKENS:
            require(token not in source, path,
                    f"forbidden production token present: {token}", failures)

    rpc = read("crates/dfmcp-adapter/src/dfhack_rpc.rs")
    for needle in [
        'b"DFHack?\\n"',
        'b"DFHack!\\n"',
        "protobuf varint is not minimally encoded",
        "reserved core method ID",
        'field("token", &"<redacted>")',
        "strict unit-ID order",
        "loopback_transport_survives_fragmented_dfhack_replies",
    ]:
        require(needle in rpc, "crates/dfmcp-adapter/src/dfhack_rpc.rs",
                f"wire hardening contract missing: {needle}", failures)

    capsule = read("crates/dfmcp-adapter/src/live_observation.rs")
    for needle in [
        "fields do not reproduce its canonical bytes",
        "semantic_field_tampering_invalidates_the_capsule",
        "pagination_does_not_change_capsule_identity",
    ]:
        require(needle in capsule, "crates/dfmcp-adapter/src/live_observation.rs",
                f"capsule integrity contract missing: {needle}", failures)

    projection = read("crates/dfmcp-adapter/src/live_projection.rs")
    require("FactSource::DfhackField" in projection, "crates/dfmcp-adapter/src/live_projection.rs",
            "live facts do not preserve DFHack provenance", failures)
    require("duplicate native unit identity" in projection,
            "crates/dfmcp-adapter/src/live_projection.rs",
            "duplicate native identity rejection is missing", failures)

    evidence = read("crates/dfmcp-adapter/src/live_evidence.rs")
    require("fact.source_digest != expected_source" in evidence,
            "crates/dfmcp-adapter/src/live_evidence.rs",
            "receipt does not verify every fact source", failures)
    require("receipt.verify" in read("crates/dfmcp-adapter/src/live_read_adapter.rs"),
            "crates/dfmcp-adapter/src/live_read_adapter.rs",
            "live publication does not retain/verify its proof receipt", failures)

    version = read("crates/dfmcp-adapter/src/live_version.rs")
    for needle in ["BridgeRestart", "CompatibilityChanged", "RestoreRequired"]:
        require(needle in version, "crates/dfmcp-adapter/src/live_version.rs",
                f"live lineage distinction missing: {needle}", failures)

    compatibility = read("crates/dfmcp-adapter/src/live_compatibility.rs")
    require("require_canonical_observation" in compatibility,
            "crates/dfmcp-adapter/src/live_compatibility.rs",
            "canonical compatibility gate is missing", failures)
    require("canonical_observation_allowed: false" in compatibility,
            "crates/dfmcp-adapter/src/live_compatibility.rs",
            "compatibility policy lacks fail-closed outcomes", failures)

    connector = read("crates/dfmcp-adapter/src/asupersync_connector.rs")
    require("cx:" in connector and "check_context(cx)?" in connector,
            "crates/dfmcp-adapter/src/asupersync_connector.rs",
            "connector is not context-bound and cancellation-checked", failures)
    require("is_loopback" in connector and "connect_timeout" in connector,
            "crates/dfmcp-adapter/src/asupersync_connector.rs",
            "connector is not bounded numeric loopback", failures)

    adapter = read("crates/dfmcp-adapter/src/live_read_adapter.rs")
    for method in ["prepare", "commit", "request_cancel", "checkpoint", "restore"]:
        require(f'live DFHack protocol V1 is read-only; {method}' in adapter or
                f'read_only("{method}")' in adapter,
                "crates/dfmcp-adapter/src/live_read_adapter.rs",
                f"read-only rejection missing for {method}", failures)
    require("ObservationPayload::Delta" in adapter,
            "crates/dfmcp-adapter/src/live_read_adapter.rs",
            "exact-basis semantic delta path is missing", failures)

    live_agent = read("crates/dfmcp-mcp/src/live_agent.rs")
    require("domains marked omitted are unknown" in live_agent,
            "crates/dfmcp-mcp/src/live_agent.rs",
            "agent projection hides omitted-domain uncertainty", failures)
    for secret in ["bearer_token", "client_nonce"]:
        require(f'"{secret}"' not in live_agent,
                "crates/dfmcp-mcp/src/live_agent.rs",
                f"agent projection includes secret-bearing field {secret}", failures)


def check_docs_and_status(failures: list[Failure]) -> None:
    path = "docs/LIVE_DFHACK_READ_PATH.md"
    source = read(path)
    for needle in [
        "Handshake",
        "ReadObservation",
        "LiveObservationCapsule",
        "R1: native build",
        "R5: agent orientation",
    ]:
        require(needle in source, path, f"live path documentation missing {needle}", failures)


def main() -> int:
    failures: list[Failure] = []
    check_registry(failures)
    check_native(failures)
    check_workspace_and_exports(failures)
    check_rust_contracts(failures)
    check_docs_and_status(failures)
    if failures:
        print(f"live read stack: FAIL ({len(failures)} violation(s))", file=sys.stderr)
        for failure in failures:
            print(f"  {failure.path}: {failure.message}", file=sys.stderr)
        return 1
    print(
        "live read stack: PASS "
        "(2 authenticated native RPCs, canonical capsule/projection/receipt, "
        "exact compatibility, asupersync-bound loopback connector, MCP read session)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
