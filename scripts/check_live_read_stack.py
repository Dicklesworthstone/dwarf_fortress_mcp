#!/usr/bin/env python3
"""Fail-closed static contract for the compiled authenticated live-read stack.

This gate checks the source graph that is actually reachable from the workspace.
It deliberately does not treat orphan source files, aspirational module names, or
unwired prototypes as implemented functionality. Compilation, the isolated
native DFHack build, and a disposable-fort run remain separate evidence gates.
"""

from __future__ import annotations

import json
import pathlib
import re
import sys
from dataclasses import dataclass

ROOT = pathlib.Path(__file__).resolve().parents[1]

EXPECTED_RPC_METHODS = ["Handshake", "ReadObservation"]
COMPILED_ADAPTER_MODULES = [
    "dfhack_wire",
    "fenced_live_source",
    "live_adapter",
    "live_bootstrap",
    "live_connect",
    "live_identity",
    "live_observation",
    "live_projection",
    "live_session",
]
COMPILED_LIVE_TOOLS = [
    "fortress_open_session",
    "fortress_observe",
    "fortress_query",
    "fortress_plan",
    "fortress_commit",
    "fortress_wait",
    "fortress_cancel",
    "fortress_checkpoint",
    "fortress_restore",
    "fortress_explain",
    "fortress_doctor",
]
REFUSED_MUTATION_TOOLS = [
    "fortress_plan",
    "fortress_commit",
    "fortress_cancel",
    "fortress_checkpoint",
    "fortress_restore",
]
PRODUCTION_RUST = [
    "crates/dfmcp-adapter/src/dfhack_wire.rs",
    "crates/dfmcp-adapter/src/fenced_live_source.rs",
    "crates/dfmcp-adapter/src/live_adapter.rs",
    "crates/dfmcp-adapter/src/live_bootstrap.rs",
    "crates/dfmcp-adapter/src/live_connect.rs",
    "crates/dfmcp-adapter/src/live_identity.rs",
    "crates/dfmcp-adapter/src/live_observation.rs",
    "crates/dfmcp-adapter/src/live_projection.rs",
    "crates/dfmcp-adapter/src/live_session.rs",
    "crates/dfmcp-mcp/src/live_server.rs",
]
FORBIDDEN_PRODUCTION_TOKENS = [
    "unsafe {",
    ".unwrap()",
    ".expect(",
    "todo!",
    "unimplemented!",
]
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


@dataclass(frozen=True)
class Failure:
    path: str
    message: str


def require(
    condition: bool,
    path: str,
    message: str,
    failures: list[Failure],
) -> None:
    if not condition:
        failures.append(Failure(path, message))


def read_required(relative: str, failures: list[Failure]) -> str:
    path = ROOT / relative
    try:
        return path.read_text(encoding="utf-8")
    except OSError as exc:
        failures.append(Failure(relative, f"cannot read required file: {exc}"))
        return ""


def function_body(source: str, name: str) -> str:
    signature = re.search(
        rf"pub fn {re.escape(name)}\s*\([^{{]*\)\s*->\s*String\s*\{{",
        source,
        re.S,
    )
    if signature is None:
        return ""
    opening = source.find("{", signature.start())
    depth = 0
    for index in range(opening, len(source)):
        char = source[index]
        if char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
            if depth == 0:
                return source[opening + 1 : index]
    return ""


def check_registry(failures: list[Failure]) -> None:
    path = "architecture/dfhack_read_bridge_v1.json"
    source = read_required(path, failures)
    if not source:
        return
    try:
        value = json.loads(source)
    except json.JSONDecodeError as exc:
        failures.append(Failure(path, f"cannot parse registry: {exc}"))
        return

    require(
        value.get("schema_version") == "dfmcp.dfhack_read_bridge/1",
        path,
        "schema version drifted",
        failures,
    )
    methods = value.get("methods")
    names = [entry.get("name") for entry in methods] if isinstance(methods, list) else []
    require(
        names == EXPECTED_RPC_METHODS,
        path,
        "registry must expose exactly Handshake and ReadObservation",
        failures,
    )
    if isinstance(methods, list):
        for method in methods:
            require(
                method.get("effect") == "read_only",
                path,
                f"{method.get('name')} is not read_only",
                failures,
            )
            require(
                method.get("requires_authentication") is True,
                path,
                f"{method.get('name')} is not authentication-gated",
                failures,
            )
    transport = value.get("transport")
    require(
        isinstance(transport, dict) and transport.get("remote_access") is False,
        path,
        "remote access must remain false",
        failures,
    )


def check_native(failures: list[Failure]) -> None:
    cpp_path = "bridge/dfhack-plugin/src/dfmcp_bridge.cpp"
    cpp = read_required(cpp_path, failures)
    if cpp:
        registrations = re.findall(r'addFunction\("([A-Za-z0-9_]+)"', cpp)
        require(
            registrations == EXPECTED_RPC_METHODS,
            cpp_path,
            "native plugin RPC registrations drifted",
            failures,
        )
        require(
            cpp.count("authenticate(in->bearer_token()") == 2,
            cpp_path,
            "every native RPC must authenticate before reading state",
            failures,
        )
        require(
            "constant_time_equal" in cpp,
            cpp_path,
            "constant-time token comparison is missing",
            failures,
        )
        require(
            "std::sort(citizens.begin(), citizens.end()" in cpp,
            cpp_path,
            "citizens are not canonically ordered before publication",
            failures,
        )
        for token in FORBIDDEN_NATIVE:
            require(
                token not in cpp,
                cpp_path,
                f"forbidden native route or flag present: {token}",
                failures,
            )

    proto_path = "bridge/dfhack-plugin/proto/DfmcpBridge.proto"
    proto = read_required(proto_path, failures)
    if proto:
        declarations = re.findall(r"// RPC\s+([A-Za-z0-9_]+)\s*:", proto)
        require(
            declarations == EXPECTED_RPC_METHODS,
            proto_path,
            "protobuf RPC declarations drifted",
            failures,
        )
        require(
            proto.count("required bytes bearer_token") == 2,
            proto_path,
            "both requests must carry bearer authentication",
            failures,
        )

    cmake_path = "bridge/dfhack-plugin/CMakeLists.txt"
    cmake = read_required(cmake_path, failures)
    if cmake:
        require(
            "dfhack_plugin(" in cmake and "PROTOBUFS" in cmake,
            cmake_path,
            "plugin is not built through DFHack's native plugin/protobuf macro",
            failures,
        )
    require(
        not (ROOT / "bridge/dfhack-plugin/include/dfmcp_ipc.h").exists(),
        cmake_path,
        "obsolete custom socket header reappeared",
        failures,
    )


def check_adapter_graph(failures: list[Failure]) -> None:
    lib_path = "crates/dfmcp-adapter/src/lib.rs"
    source = read_required(lib_path, failures)
    if not source:
        return

    for module in COMPILED_ADAPTER_MODULES:
        require(
            f"pub mod {module};" in source,
            lib_path,
            f"live module {module} is not compiled",
            failures,
        )
    for symbol in [
        "BridgeCredentials",
        "DfHackRpcClient",
        "FencedLiveSource",
        "LiveConnectionConfig",
        "LiveObservationCapsule",
        "LiveReadAdapter",
        "LiveReadBootstrapConfig",
        "LiveWorldProjection",
        "bootstrap_live_read_adapter",
        "connect_authenticated_live_source",
        "derive_live_fortress_id",
        "project_live_capsule",
        "read_complete_observation_bounded",
    ]:
        require(
            symbol in source,
            lib_path,
            f"compiled live symbol {symbol} is not exported",
            failures,
        )

    for path in PRODUCTION_RUST:
        rust = read_required(path, failures)
        if not rust:
            continue
        require(
            "#![forbid(unsafe_code)]" in rust,
            path,
            "safe-Rust prohibition is missing",
            failures,
        )
        for token in FORBIDDEN_PRODUCTION_TOKENS:
            require(
                token not in rust,
                path,
                f"forbidden production token present: {token}",
                failures,
            )

    wire_path = "crates/dfmcp-adapter/src/dfhack_wire.rs"
    wire = read_required(wire_path, failures)
    for needle in [
        'b"DFHack?\\n"',
        'b"DFHack!\\n"',
        "protobuf varint is not minimally encoded",
        "reserved core method ID",
        'field("token", &"<redacted>")',
        "strict unit-ID order",
    ]:
        require(needle in wire, wire_path, f"wire hardening contract missing: {needle}", failures)

    capsule_path = "crates/dfmcp-adapter/src/live_observation.rs"
    capsule = read_required(capsule_path, failures)
    for needle in [
        "fields do not reproduce",
        "semantic_field_tampering_invalidates_the_capsule",
        "pagination_does_not_change_capsule_identity",
    ]:
        require(
            needle in capsule,
            capsule_path,
            f"capsule integrity contract missing: {needle}",
            failures,
        )

    projection_path = "crates/dfmcp-adapter/src/live_projection.rs"
    projection = read_required(projection_path, failures)
    require(
        "FactSource::DfhackField" in projection,
        projection_path,
        "live facts do not preserve DFHack field provenance",
        failures,
    )
    require(
        "duplicate native unit identity" in projection,
        projection_path,
        "duplicate native identity rejection is missing",
        failures,
    )

    adapter_path = "crates/dfmcp-adapter/src/live_adapter.rs"
    adapter = read_required(adapter_path, failures)
    require(
        "ensure_same_session_identity" in adapter,
        adapter_path,
        "world/save/version identity fencing is missing",
        failures,
    )
    require(
        "clock_regression" in adapter and "bridge_reset" in adapter,
        adapter_path,
        "restart and clock-regression epoch handling is missing",
        failures,
    )
    require(
        "execute_bounded_query" in adapter,
        adapter_path,
        "live canonical query path is not bounded through the world engine",
        failures,
    )
    for operation in [
        "prepare",
        "commit",
        "cancellation",
        "checkpoint",
        "restore",
    ]:
        require(
            operation in adapter and "read_only_rejection" in adapter,
            adapter_path,
            f"read-only rejection is not represented for {operation}",
            failures,
        )


def check_mcp_and_cli(failures: list[Failure]) -> None:
    lib_path = "crates/dfmcp-mcp/src/lib.rs"
    lib = read_required(lib_path, failures)
    require("pub mod live_server;" in lib, lib_path, "live server is not compiled", failures)
    require(
        "pub use live_server::run_live_stdio;" in lib,
        lib_path,
        "live server entrypoint is not exported",
        failures,
    )

    server_path = "crates/dfmcp-mcp/src/live_server.rs"
    server = read_required(server_path, failures)
    if server:
        functions = re.findall(r"pub fn (fortress_[a-z_]+)\s*\(", server)
        require(
            [name for name in functions if name in COMPILED_LIVE_TOOLS] == COMPILED_LIVE_TOOLS,
            server_path,
            "live server does not define the frozen eleven tools in canonical order",
            failures,
        )
        registrations = re.findall(r"\.tool\((Fortress[A-Za-z]+)\)", server)
        require(
            len(registrations) == 11 and len(set(registrations)) == 11,
            server_path,
            "live server must register exactly eleven unique tools",
            failures,
        )
        for tool in REFUSED_MUTATION_TOOLS:
            body = function_body(server, tool)
            require(bool(body), server_path, f"cannot isolate body for {tool}", failures)
            require(
                "read_only_tool_error" in body,
                server_path,
                f"{tool} bypasses the common read-only refusal",
                failures,
            )
        for tool in COMPILED_LIVE_TOOLS:
            signature = re.search(
                rf"pub fn {re.escape(tool)}\s*\((.*?)\)\s*->",
                server,
                re.S,
            )
            if signature is not None:
                arguments = signature.group(1).lower()
                require(
                    all(secret not in arguments for secret in ["token", "secret", "endpoint"]),
                    server_path,
                    f"{tool} exposes deployment secrets as MCP arguments",
                    failures,
                )
        require(
            "DFMCP_BRIDGE_TOKEN" in server,
            server_path,
            "bearer authentication is not sourced from process configuration",
            failures,
        )
        require(
            '"mutation_admissible": false' in server,
            server_path,
            "live Agent Turn does not make the read-only posture explicit",
            failures,
        )
        for forbidden in [
            ".prepare(",
            ".commit(",
            ".request_cancel(",
            ".finalize_cancel(",
            ".checkpoint(",
            ".restore(",
            "RunCommand",
            "RunLua",
        ]:
            require(
                forbidden not in server,
                server_path,
                f"live server contains forbidden effect path {forbidden}",
                failures,
            )

    cli_path = "crates/dwarf-fortress-mcp/src/main.rs"
    cli = read_required(cli_path, failures)
    require(
        '"serve-live" => dfmcp_mcp::run_live_stdio()' in cli,
        cli_path,
        "CLI does not expose explicit live read-only mode",
        failures,
    )
    require(
        '"serve" => dfmcp_mcp::run_stdio()' in cli,
        cli_path,
        "CLI no longer preserves deterministic laboratory mode",
        failures,
    )
    require(
        "connect_authenticated_live_source" in cli,
        cli_path,
        "CLI bypasses shared live connection admission",
        failures,
    )
    require(
        "TcpStream::connect" not in cli,
        cli_path,
        "CLI reimplements socket admission instead of using the adapter boundary",
        failures,
    )


def main() -> int:
    failures: list[Failure] = []
    check_registry(failures)
    check_native(failures)
    check_adapter_graph(failures)
    check_mcp_and_cli(failures)

    if failures:
        print(f"live read stack: FAIL ({len(failures)} violation(s))", file=sys.stderr)
        for failure in failures:
            print(f"  {failure.path}: {failure.message}", file=sys.stderr)
        return 1

    print(
        "live read stack: PASS "
        "(compiled native/auth/wire/capsule/projection/adapter/MCP chain, "
        "11 tools, 0 live effect paths)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
