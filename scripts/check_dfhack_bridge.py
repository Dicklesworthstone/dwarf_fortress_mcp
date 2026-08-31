#!/usr/bin/env python3
"""Fail-closed static checks for the first live DFHack read path.

This gate proves source alignment and absence of known bypasses. It does not
prove Rust/C++ compilation, plugin loading, authentication against a running
process, or live fortress correctness.
"""

from __future__ import annotations

import json
import pathlib
import re
import sys
from dataclasses import dataclass

ROOT = pathlib.Path(__file__).resolve().parents[1]
EXPECTED_METHODS = ["Handshake", "ReadObservation"]
EXPECTED_SUMMARY_FIELDS = [
    "world_loaded",
    "fortress_mode",
    "paused",
    "current_year",
    "current_year_tick",
    "world_name",
    "world_folder",
    "site_id",
    "citizen_count_total",
]
EXPECTED_CITIZEN_FIELDS = [
    "unit_id",
    "name",
    "race",
    "profession",
    "x",
    "y",
    "z",
    "alive",
    "sane",
    "active",
    "visible",
    "citizen",
    "resident",
    "baby",
    "child",
    "adult",
]
FORBIDDEN_NATIVE_TOKENS = [
    "SF_ALLOW_REMOTE",
    "RunCommand",
    "RunLua",
    "SetPauseState",
    "set_pause_state",
    "setPauseState",
    "teleport",
    "DigCommand",
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


def require(value: bool, path: str, message: str, failures: list[Failure]) -> None:
    if not value:
        failures.append(Failure(path, message))


def read_required(path: str, failures: list[Failure]) -> str:
    try:
        return (ROOT / path).read_text(encoding="utf-8")
    except OSError as exc:
        failures.append(Failure(path, f"cannot read required file: {exc}"))
        return ""


def check_registry(failures: list[Failure]) -> None:
    path = "architecture/dfhack_read_bridge_v1.json"
    source = read_required(path, failures)
    if not source:
        return
    try:
        registry = json.loads(source)
    except json.JSONDecodeError as exc:
        failures.append(Failure(path, f"cannot parse registry: {exc}"))
        return

    require(
        registry.get("schema_version") == "dfmcp.dfhack_read_bridge/1",
        path,
        "schema version drifted",
        failures,
    )
    methods = registry.get("methods")
    names = [entry.get("name") for entry in methods] if isinstance(methods, list) else []
    require(
        names == EXPECTED_METHODS,
        path,
        "registry must contain exactly the two ordered V1 methods",
        failures,
    )
    if isinstance(methods, list):
        for method in methods:
            require(
                method.get("effect") == "read_only",
                path,
                f"method {method.get('name')} is not classified read_only",
                failures,
            )
            require(
                method.get("requires_authentication") is True,
                path,
                f"method {method.get('name')} is not authentication-gated",
                failures,
            )
    transport = registry.get("transport")
    require(
        isinstance(transport, dict) and transport.get("remote_access") is False,
        path,
        "remote_access must remain false",
        failures,
    )
    observation_fields = registry.get("observation_fields")
    require(
        isinstance(observation_fields, dict)
        and observation_fields.get("summary") == EXPECTED_SUMMARY_FIELDS,
        path,
        "summary field registry drifted",
        failures,
    )
    require(
        isinstance(observation_fields, dict)
        and observation_fields.get("citizen") == EXPECTED_CITIZEN_FIELDS,
        path,
        "citizen field registry drifted",
        failures,
    )
    authentication = registry.get("authentication")
    require(
        isinstance(authentication, dict)
        and authentication.get("rust_debug_rendering") == "redacted",
        path,
        "credential redaction rule is missing",
        failures,
    )
    invariants = registry.get("security_invariants")
    require(
        isinstance(invariants, list) and len(invariants) >= 10,
        path,
        "bridge security invariant registry is incomplete",
        failures,
    )


def check_proto(failures: list[Failure]) -> None:
    path = "bridge/dfhack-plugin/proto/DfmcpBridge.proto"
    source = read_required(path, failures)
    if not source:
        return
    require('syntax = "proto2";' in source, path, "bridge must remain proto2", failures)
    require(
        "option optimize_for = LITE_RUNTIME;" in source,
        path,
        "bridge must use protobuf Lite runtime",
        failures,
    )
    rpc_comments = re.findall(r"// RPC\s+([A-Za-z0-9_]+)\s*:", source)
    require(
        rpc_comments == EXPECTED_METHODS,
        path,
        "proto RPC declaration comments must enumerate the exact method set",
        failures,
    )
    require(
        source.count("required bytes bearer_token") == 2,
        path,
        "exactly two authenticated request messages are expected",
        failures,
    )
    require(
        source.count("client_nonce") >= 4,
        path,
        "nonce request/echo fields are incomplete",
        failures,
    )
    for field in EXPECTED_CITIZEN_FIELDS:
        require(
            re.search(rf"required\s+\w+\s+{re.escape(field)}\s*=", source) is not None,
            path,
            f"CitizenRecord is missing required field {field}",
            failures,
        )
    for name in FORBIDDEN_RPC_NAMES:
        require(
            f"RPC {name} " not in source,
            path,
            f"forbidden RPC {name} is present",
            failures,
        )
    require(
        "military" not in source.lower(),
        path,
        "V1 must not reach through unstable generated squad fields",
        failures,
    )


def check_native_plugin(failures: list[Failure]) -> None:
    path = "bridge/dfhack-plugin/src/dfmcp_bridge.cpp"
    source = read_required(path, failures)
    if not source:
        return
    require(
        'DFHACK_PLUGIN("dfmcp_bridge")' in source,
        path,
        "native target is not a genuine DFHack plugin",
        failures,
    )
    registrations = re.findall(r'addFunction\("([A-Za-z0-9_]+)"', source)
    require(
        registrations == EXPECTED_METHODS,
        path,
        "plugin_rpcconnect must register exactly Handshake and ReadObservation",
        failures,
    )
    require(
        "DFhackCExport RPCService *plugin_rpcconnect" in source,
        path,
        "supported DFHack RPC extension export is missing",
        failures,
    )
    require(
        source.count("authenticate(in->bearer_token()") == 2,
        path,
        "every RPC must authenticate before reading world state",
        failures,
    )
    require(
        "std::sort(citizens.begin(), citizens.end()" in source,
        path,
        "citizens must be sorted before pagination",
        failures,
    )
    require(
        "HARD_MAX_CITIZENS" in source and "MAX_UNIT_NAME_BYTES" in source,
        path,
        "native repeated/string output bounds are missing",
        failures,
    )
    require(
        "constant_time_equal" in source,
        path,
        "constant-time token comparison is missing",
        failures,
    )
    for token in FORBIDDEN_NATIVE_TOKENS:
        require(token not in source, path, f"forbidden native token {token} is present", failures)
    require(
        "unit->military" not in source,
        path,
        "V1 must not access generated military state directly",
        failures,
    )

    cmake_path = "bridge/dfhack-plugin/CMakeLists.txt"
    cmake = read_required(cmake_path, failures)
    require(
        "dfhack_plugin(" in cmake and "PROTOBUFS ${PROJECT_PROTO}" in cmake,
        cmake_path,
        "plugin must build through DFHack's plugin/protobuf macro",
        failures,
    )
    require(
        "add_executable" not in cmake and "add_library" not in cmake,
        cmake_path,
        "standalone custom native target reappeared",
        failures,
    )
    require(
        not (ROOT / "bridge/dfhack-plugin/include/dfmcp_ipc.h").exists(),
        cmake_path,
        "superseded bespoke socket header must stay deleted",
        failures,
    )


def check_rust_wire(failures: list[Failure]) -> None:
    path = "crates/dfmcp-adapter/src/dfhack_wire.rs"
    source = read_required(path, failures)
    if not source:
        return
    for needle in [
        'b"DFHack?\\n"',
        'b"DFHack!\\n"',
        "const HANDSHAKE_HEADER_BYTES: usize = 12",
        "const MESSAGE_HEADER_BYTES: usize = 8",
        "BridgeCredentials",
        'field("token", &"<redacted>")',
        "decode_bind_reply",
        "decode_handshake_reply",
        "decode_observation_reply",
        "protobuf varint is not minimally encoded",
        "MAX_TEXT_NOTIFICATIONS_PER_CALL",
        "MAX_TEXT_NOTIFICATION_TOTAL_BYTES",
        "strict unit-ID order",
    ]:
        require(needle in source, path, f"missing live-wire contract: {needle}", failures)
    require(
        "prost" not in source and "tonic" not in source,
        path,
        "live client must remain dependency-free and non-gRPC",
        failures,
    )
    require(
        "TcpStream" not in source,
        path,
        "wire codec must not smuggle in unmanaged socket/runtime policy",
        failures,
    )
    require(
        "u64::from(value)" not in source,
        path,
        "protobuf Boolean encoding must use explicit canonical 0/1",
        failures,
    )
    require(
        "derive(Clone, Debug, PartialEq, Eq)\npub struct BridgeCredentials" not in source,
        path,
        "credentials must not derive token-revealing Debug",
        failures,
    )
    require(
        not (ROOT / "crates/dfmcp-adapter/src/dfhack_rpc.rs").exists(),
        path,
        "superseded unaudited wire source must stay deleted",
        failures,
    )

    lib_path = "crates/dfmcp-adapter/src/lib.rs"
    lib = read_required(lib_path, failures)
    for needle in [
        "pub mod dfhack_wire;",
        "pub mod live_observation;",
        "pub mod live_session;",
        "pub use dfhack_wire::{",
        "pub use live_observation::{",
        "pub use live_session::{",
    ]:
        require(needle in lib, lib_path, f"adapter crate is missing wiring: {needle}", failures)


def check_capsule_and_driver(failures: list[Failure]) -> None:
    capsule_path = "crates/dfmcp-adapter/src/live_observation.rs"
    capsule = read_required(capsule_path, failures)
    for needle in [
        "LiveObservationCapsule",
        "ObservationAssembler",
        "fields do not reproduce the stored canonical bytes",
        "pagination_does_not_change_capsule_identity",
        "structured_field_tampering_invalidates_the_capsule",
        "canonical_byte_tampering_invalidates_the_capsule",
        "summary_drift_between_pages_is_rejected",
        "incomplete_assembly_cannot_publish",
        "sha256(&canonical_bytes)",
    ]:
        require(
            needle in capsule,
            capsule_path,
            f"missing capsule contract: {needle}",
            failures,
        )
    canonical_tail = capsule.split("fn canonical_bytes", 1)[-1]
    require(
        "sort" not in canonical_tail,
        capsule_path,
        "canonicalization must consume certified order, not silently re-sort",
        failures,
    )

    driver_path = "crates/dfmcp-adapter/src/live_session.rs"
    driver = read_required(driver_path, failures)
    for needle in [
        "LiveObservationSource",
        "read_complete_observation",
        "empty nonterminal citizen page",
        "zero_citizen_fortress_finishes_in_one_empty_page",
    ]:
        require(needle in driver, driver_path, f"missing page-driver contract: {needle}", failures)
    require(
        "saturating_div" not in driver,
        driver_path,
        "page-count arithmetic must not rely on a nonportable saturating division",
        failures,
    )


def check_native_build_harness(failures: list[Failure]) -> None:
    path = "scripts/qualify_dfhack_plugin.sh"
    source = read_required(path, failures)
    for needle in [
        'EXTERNAL_DIR="$WORKTREE/plugins/external"',
        'PLUGIN_DST="$EXTERNAL_DIR/dfmcp_bridge"',
        'EXTERNAL_CMAKE="$EXTERNAL_DIR/CMakeLists.txt"',
        "add_subdirectory(dfmcp_bridge)",
        "--target dfmcp_bridge",
        "crates/dfmcp-adapter/src/dfhack_wire.rs",
        "crates/dfmcp-adapter/src/live_observation.rs",
        "crates/dfmcp-adapter/src/live_session.rs",
        "external_registration",
    ]:
        require(needle in source, path, f"native build harness is missing: {needle}", failures)
    require(
        'PLUGIN_DST="$WORKTREE/plugins/dfmcp_bridge"' not in source,
        path,
        "harness reverted to an unregistered top-level plugin directory",
        failures,
    )
    require(
        "crates/dfmcp-adapter/src/dfhack_rpc.rs" not in source,
        path,
        "native receipt references the deleted wire client",
        failures,
    )
    require(
        "trap - EXIT" in source,
        path,
        "EXIT cleanup must disable its trap before exiting",
        failures,
    )


def check_qualification_wiring(failures: list[Failure]) -> None:
    for path in ["scripts/verify.sh", "scripts/qualify_local.sh"]:
        source = read_required(path, failures)
        require(
            "scripts/check_dfhack_bridge.py" in source,
            path,
            "DFHack bridge contract is not a mandatory gate",
            failures,
        )
        require(
            "scripts/qualify_dfhack_plugin.sh" in source,
            path,
            "native DFHack build harness is not shell-syntax checked",
            failures,
        )
    qualify = read_required("scripts/qualify_local.sh", failures)
    for needle in [
        "dfhack_read_bridge_contract",
        "dfhack_bridge_proto",
        "dfhack_bridge_plugin",
        "dfhack_wire_client",
        "live_observation_capsule",
        "live_observation_driver",
        "dfhack_native_build_harness",
    ]:
        require(
            needle in qualify,
            "scripts/qualify_local.sh",
            f"qualification receipt omits {needle}",
            failures,
        )


def main() -> int:
    failures: list[Failure] = []
    check_registry(failures)
    check_proto(failures)
    check_native_plugin(failures)
    check_rust_wire(failures)
    check_capsule_and_driver(failures)
    check_native_build_harness(failures)
    check_qualification_wiring(failures)
    if failures:
        print(f"DFHack bridge contract: FAIL ({len(failures)} violation(s))", file=sys.stderr)
        for failure in failures:
            print(f"  {failure.path}: {failure.message}", file=sys.stderr)
        return 1
    print(
        "DFHack bridge contract: PASS "
        "(2 authenticated read-only RPCs, registered native target, bounded wire, complete capsule)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
