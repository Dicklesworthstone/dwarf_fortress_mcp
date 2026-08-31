#!/usr/bin/env python3
"""Static non-bypassability checks for the authenticated live MCP mode.

This gate proves source shape and cross-layer alignment only. It does not prove
Rust compilation, fastmcp_rust dispatch, socket connectivity, native plugin
loading, or live fortress correctness.
"""

from __future__ import annotations

import pathlib
import re
import sys
from dataclasses import dataclass

ROOT = pathlib.Path(__file__).resolve().parents[1]
EXPECTED_TOOLS = [
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
MUTATION_TOOLS = [
    "fortress_plan",
    "fortress_commit",
    "fortress_wait",
    "fortress_cancel",
    "fortress_checkpoint",
    "fortress_restore",
]
FORBIDDEN_LIVE_EFFECT_TOKENS = [
    ".prepare(",
    ".commit(",
    ".request_cancel(",
    ".finalize_cancel(",
    ".checkpoint(",
    ".restore(",
    "RunCommand",
    "RunLua",
    "ApplyEffect",
    "SetPauseState",
]


@dataclass(frozen=True)
class Failure:
    path: str
    message: str


def read(path: str, failures: list[Failure]) -> str:
    try:
        return (ROOT / path).read_text(encoding="utf-8")
    except OSError as exc:
        failures.append(Failure(path, f"cannot read required file: {exc}"))
        return ""


def require(condition: bool, path: str, message: str, failures: list[Failure]) -> None:
    if not condition:
        failures.append(Failure(path, message))


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


def check_live_server(failures: list[Failure]) -> None:
    path = "crates/dfmcp-mcp/src/live_server.rs"
    source = read(path, failures)
    if not source:
        return

    functions = re.findall(r"pub fn (fortress_[a-z_]+)\s*\(", source)
    require(
        [name for name in functions if name in EXPECTED_TOOLS] == EXPECTED_TOOLS,
        path,
        "live server must define the frozen eleven tools in canonical order",
        failures,
    )
    registrations = re.findall(r"\.tool\((Fortress[A-Za-z]+)\)", source)
    require(
        len(registrations) == 11 and len(set(registrations)) == 11,
        path,
        "run_live_stdio must register exactly eleven unique tools",
        failures,
    )
    require(
        'ServerBuilder::new("dwarf-fortress-mcp-live"' in source,
        path,
        "live server identity is missing or ambiguous",
        failures,
    )
    require(
        "DFMCP_BRIDGE_TOKEN" in source and 'env::var("DFMCP_BRIDGE_TOKEN")' in source,
        path,
        "live bearer secret must come from process configuration",
        failures,
    )
    for tool in EXPECTED_TOOLS:
        body = function_body(source, tool)
        require(bool(body), path, f"cannot isolate body for {tool}", failures)
        signature = re.search(
            rf"pub fn {re.escape(tool)}\s*\((.*?)\)\s*->",
            source,
            re.S,
        )
        if signature is not None:
            args = signature.group(1).lower()
            require(
                "token" not in args and "endpoint" not in args and "secret" not in args,
                path,
                f"{tool} exposes deployment secrets or endpoint as MCP arguments",
                failures,
            )
    for tool in MUTATION_TOOLS:
        body = function_body(source, tool)
        require(
            "read_only_tool_error" in body,
            path,
            f"{tool} does not route through the common read-only refusal",
            failures,
        )
    for token in FORBIDDEN_LIVE_EFFECT_TOKENS:
        require(token not in source, path, f"live server contains forbidden effect token {token}", failures)

    for needle in [
        "bootstrap_live_read_adapter",
        "connect_authenticated_live_source",
        "parse_loopback_endpoint",
        "AuthenticatedLiveSource",
        "ContinuityStatus::Heartbeat",
        "ContinuityStatus::Reset",
        ".request_id(request_id.to_string())",
        "source_poisoned",
        "coverage_json",
        '"mutation_admissible": false',
    ]:
        require(needle in source, path, f"live server is missing contract marker {needle}", failures)

    require(
        "Capability::ControlClock" not in source
        and "Capability::Designate" not in source
        and "Capability::Construct" not in source,
        path,
        "live capability negotiation admits mutation authority",
        failures,
    )
    require(
        '"observe" => Capability::Observe' in source
        and '"query" => Capability::Query' in source
        and '"doctor" => Capability::Doctor' in source,
        path,
        "live capability allowlist is incomplete",
        failures,
    )


def check_adapter_chain(failures: list[Failure]) -> None:
    paths = {
        "crates/dfmcp-adapter/src/live_connect.rs": [
            "parse_loopback_endpoint",
            "connect_authenticated_live_source",
            "set_read_timeout",
            "set_write_timeout",
            "FencedLiveSource::new",
        ],
        "crates/dfmcp-adapter/src/fenced_live_source.rs": [
            "one failed page read poisons the source permanently",
            "source is poisoned; negotiate a fresh bridge connection",
            "one_failure_permanently_fences_the_source",
        ],
        "crates/dfmcp-adapter/src/live_bootstrap.rs": [
            "bootstrap_live_read_adapter",
            "PrimedLiveSource",
            "bootstrap_reads_the_underlying_source_once",
            "source manifest changed between the first capsule and adapter bootstrap",
        ],
        "crates/dfmcp-adapter/src/live_identity.rs": [
            "derive_live_fortress_id",
            "dfmcp-live-fortress-id-v1",
            "identity_ignores_projection_and_transport_details",
        ],
        "crates/dfmcp-adapter/src/live_observation.rs": [
            "dfmcp.live-observation-capsule.v2",
            "names_included",
            "name_projection_is_part_of_capsule_identity",
            "fields do not reproduce the stored canonical bytes",
        ],
        "crates/dfmcp-adapter/src/live_projection.rs": [
            "dfmcp.live_world_projection/2",
            "FactPresence::Omitted",
            "fortress.citizens.names",
            "omitted_names_remain_omitted_in_facts_and_coverage",
        ],
        "crates/dfmcp-adapter/src/live_session.rs": [
            "cannot assemble a coherent multipage observation while Dwarf Fortress is running",
            "moving_multipage_observation_is_rejected_before_assembly",
            "read_complete_observation_bounded",
        ],
    }
    for path, needles in paths.items():
        source = read(path, failures)
        for needle in needles:
            require(needle in source, path, f"missing live adapter contract marker {needle}", failures)

    lib_path = "crates/dfmcp-adapter/src/lib.rs"
    lib = read(lib_path, failures)
    for module in [
        "fenced_live_source",
        "live_adapter",
        "live_bootstrap",
        "live_connect",
        "live_identity",
        "live_observation",
        "live_projection",
        "live_session",
    ]:
        require(
            f"pub mod {module};" in lib,
            lib_path,
            f"adapter module {module} is not compiled",
            failures,
        )


def check_cli_and_crate_wiring(failures: list[Failure]) -> None:
    lib_path = "crates/dfmcp-mcp/src/lib.rs"
    lib = read(lib_path, failures)
    require("pub mod live_server;" in lib, lib_path, "live server module is not compiled", failures)
    require(
        "pub use live_server::run_live_stdio;" in lib,
        lib_path,
        "live server entrypoint is not exported",
        failures,
    )

    cli_path = "crates/dwarf-fortress-mcp/src/main.rs"
    cli = read(cli_path, failures)
    require(
        '"serve-live" => dfmcp_mcp::run_live_stdio()' in cli,
        cli_path,
        "CLI does not expose the explicit live server mode",
        failures,
    )
    require(
        '"serve" => dfmcp_mcp::run_stdio()' in cli,
        cli_path,
        "CLI no longer preserves the deterministic laboratory mode",
        failures,
    )
    require(
        "connect_authenticated_live_source" in cli,
        cli_path,
        "CLI bypasses shared live connection admission",
        failures,
    )
    require(
        "derive_live_fortress_id" in cli,
        cli_path,
        "CLI bypasses canonical live fortress identity",
        failures,
    )
    require(
        "TcpStream::connect" not in cli,
        cli_path,
        "CLI reimplemented socket admission instead of using the shared boundary",
        failures,
    )


def main() -> int:
    failures: list[Failure] = []
    check_live_server(failures)
    check_adapter_chain(failures)
    check_cli_and_crate_wiring(failures)
    if failures:
        print(f"live MCP contract: FAIL ({len(failures)} violation(s))", file=sys.stderr)
        for failure in failures:
            print(f"  {failure.path}: {failure.message}", file=sys.stderr)
        return 1
    print("live MCP contract: PASS (11 tools, 5 read-only operations, 0 live effect paths)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
