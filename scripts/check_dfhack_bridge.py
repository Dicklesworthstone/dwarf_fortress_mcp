#!/usr/bin/env python3
"""Static non-bypassability checks for the first live DFHack bridge slice."""

from __future__ import annotations

import json
import pathlib
import re
import sys
from dataclasses import dataclass

ROOT = pathlib.Path(__file__).resolve().parents[1]
EXPECTED_METHODS = ["Handshake", "ReadObservation"]
FORBIDDEN_NATIVE_TOKENS = [
    "SF_ALLOW_REMOTE",
    "RunCommand",
    "RunLua",
    "set_pause_state",
    "setPauseState",
    "teleport",
    "digCommand",
    "plugin_enable",
]
FORBIDDEN_RPC_NAMES = [
    "Pause",
    "Resume",
    "Dig",
    "Teleport",
    "RunCommand",
    "RunLua",
    "Mutate",
    "ApplyEffect",
]


@dataclass(frozen=True)
class Failure:
    path: str
    message: str


def read(path: str) -> str:
    return (ROOT / path).read_text(encoding="utf-8")


def require(value: bool, path: str, message: str, failures: list[Failure]) -> None:
    if not value:
        failures.append(Failure(path, message))


def check_registry(failures: list[Failure]) -> None:
    path = "architecture/dfhack_read_bridge_v1.json"
    try:
        registry = json.loads(read(path))
    except (OSError, json.JSONDecodeError) as exc:
        failures.append(Failure(path, f"cannot load registry: {exc}"))
        return
    require(registry.get("schema_version") == "dfmcp.dfhack_read_bridge/1", path,
            "schema version drifted", failures)
    methods = registry.get("methods")
    names = [entry.get("name") for entry in methods] if isinstance(methods, list) else []
    require(names == EXPECTED_METHODS, path, "registry must contain exactly the two V1 methods", failures)
    if isinstance(methods, list):
        for method in methods:
            require(method.get("effect") == "read_only", path,
                    f"method {method.get('name')} is not classified read_only", failures)
            require(method.get("requires_authentication") is True, path,
                    f"method {method.get('name')} is not authentication-gated", failures)
    transport = registry.get("transport")
    require(isinstance(transport, dict) and transport.get("remote_access") is False, path,
            "remote_access must remain false", failures)
    authority = registry.get("security_invariants")
    require(isinstance(authority, list) and len(authority) >= 8, path,
            "bridge security invariant registry is incomplete", failures)


def check_proto(failures: list[Failure]) -> None:
    path = "bridge/dfhack-plugin/proto/DfmcpBridge.proto"
    source = read(path)
    require('syntax = "proto2";' in source, path, "bridge must remain proto2", failures)
    require("option optimize_for = LITE_RUNTIME;" in source, path,
            "bridge must use protobuf Lite runtime", failures)
    rpc_comments = re.findall(r"// RPC\s+([A-Za-z0-9_]+)\s*:", source)
    require(rpc_comments == EXPECTED_METHODS, path,
            "proto RPC declaration comments must enumerate the exact method set", failures)
    require("required bytes bearer_token" in source, path,
            "both request families must carry bearer authentication", failures)
    require(source.count("required bytes bearer_token") == 2, path,
            "exactly two authenticated request messages are expected", failures)
    require("required bytes client_nonce" in source and source.count("client_nonce") >= 4, path,
            "nonce echo/binding fields are incomplete", failures)
    for name in FORBIDDEN_RPC_NAMES:
        require(f"RPC {name} " not in source, path, f"forbidden RPC {name} is present", failures)
    require("military" not in source.lower(), path,
            "V1 must not reach through unstable generated squad fields", failures)


def check_native_plugin(failures: list[Failure]) -> None:
    path = "bridge/dfhack-plugin/src/dfmcp_bridge.cpp"
    source = read(path)
    require('DFHACK_PLUGIN("dfmcp_bridge")' in source, path,
            "native target is not a genuine DFHack plugin", failures)
    registrations = re.findall(r'addFunction\("([A-Za-z0-9_]+)"', source)
    require(registrations == EXPECTED_METHODS, path,
            "plugin_rpcconnect must register exactly Handshake and ReadObservation", failures)
    require("DFhackCExport RPCService *plugin_rpcconnect" in source, path,
            "supported DFHack RPC extension export is missing", failures)
    require(source.count("authenticate(in->bearer_token()") == 2, path,
            "every RPC must authenticate", failures)
    require("std::sort(citizens.begin(), citizens.end()" in source, path,
            "citizens must be sorted before pagination", failures)
    require("HARD_MAX_CITIZENS" in source and "MAX_UNIT_NAME_BYTES" in source, path,
            "native repeated/string output bounds are missing", failures)
    require("constant_time_equal" in source and '"<redacted>"' not in source, path,
            "native token comparison is missing or token material is rendered", failures)
    for token in FORBIDDEN_NATIVE_TOKENS:
        require(token not in source, path, f"forbidden native token {token} is present", failures)
    require("unit->military" not in source, path,
            "V1 must not access generated military state directly", failures)

    cmake_path = "bridge/dfhack-plugin/CMakeLists.txt"
    cmake = read(cmake_path)
    require("dfhack_plugin(" in cmake and "PROTOBUFS ${PROJECT_PROTO}" in cmake, cmake_path,
            "plugin must build through DFHack's plugin/protobuf macro", failures)
    require("add_executable" not in cmake and "add_library" not in cmake, cmake_path,
            "standalone custom native target reappeared", failures)
    require(not (ROOT / "bridge/dfhack-plugin/include/dfmcp_ipc.h").exists(), cmake_path,
            "superseded bespoke socket header must stay deleted", failures)


def check_rust_client(failures: list[Failure]) -> None:
    path = "crates/dfmcp-adapter/src/dfhack_rpc.rs"
    source = read(path)
    for needle in [
        'b"DFHack?\\n"',
        'b"DFHack!\\n"',
        "const HANDSHAKE_HEADER_BYTES: usize = 12",
        "const MESSAGE_HEADER_BYTES: usize = 8",
        "MAX_RPC_PAYLOAD_BYTES",
        "BridgeCredentials",
        'field("token", &"<redacted>")',
        "decode_bind_reply",
        "decode_handshake_reply",
        "decode_observation_reply",
        "strict unit-ID order",
    ]:
        require(needle in source, path, f"missing live-client contract: {needle}", failures)
    require("prost" not in source and "tonic" not in source, path,
            "live client must remain dependency-free and non-gRPC", failures)
    require("TcpStream" not in source, path,
            "wire codec must not smuggle in unmanaged socket/runtime policy", failures)
    require("u64::from(value)" not in source, path,
            "protobuf bool encoding must use explicit canonical 0/1", failures)
    require("derive(Clone, Debug, PartialEq, Eq)\npub struct BridgeCredentials" not in source, path,
            "credentials must not derive token-revealing Debug", failures)

    capsule_path = "crates/dfmcp-adapter/src/live_observation.rs"
    capsule = read(capsule_path)
    for needle in [
        "LiveObservationCapsule",
        "ObservationAssembler",
        "pagination_does_not_change_capsule_identity",
        "summary_drift_between_pages_is_rejected",
        "incomplete_assembly_cannot_publish",
        "sha256(&canonical_bytes)",
    ]:
        require(needle in capsule, capsule_path,
                f"missing capsule contract: {needle}", failures)
    require("sort" not in capsule.split("fn canonical_bytes", 1)[-1], capsule_path,
            "canonicalization must consume already-certified order, not silently re-sort", failures)


def main() -> int:
    failures: list[Failure] = []
    check_registry(failures)
    check_proto(failures)
    check_native_plugin(failures)
    check_rust_client(failures)
    if failures:
        print(f"DFHack bridge contract: FAIL ({len(failures)} violation(s))", file=sys.stderr)
        for failure in failures:
            print(f"  {failure.path}: {failure.message}", file=sys.stderr)
        return 1
    print("DFHack bridge contract: PASS (2 authenticated read-only RPCs, 0 mutation paths)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
