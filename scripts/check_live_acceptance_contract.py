#!/usr/bin/env python3
"""Validate the complete R2-R5 capture, journal, and verification contract."""

from __future__ import annotations

import ast
import json
import pathlib
import sys
from dataclasses import dataclass
from typing import Any

ROOT = pathlib.Path(__file__).resolve().parents[1]
CONTRACT = ROOT / "architecture/live_read_acceptance_v1.json"
BRIDGE_REGISTRY = ROOT / "architecture/dfhack_read_bridge_v1.json"
VERIFIER = ROOT / "scripts/verify_live_read_acceptance.py"
VERIFIER_TESTS = ROOT / "scripts/test_live_read_acceptance.py"
JOURNAL = ROOT / "scripts/live_read_evidence_journal.py"
JOURNAL_TESTS = ROOT / "scripts/test_live_read_evidence_journal.py"
SCANNER = ROOT / "scripts/scan_live_read_secrets.py"
SCANNER_TESTS = ROOT / "scripts/test_scan_live_read_secrets.py"
WRAPPER = ROOT / "scripts/qualify_live_read.sh"
PROBE_ADAPTER = ROOT / "crates/dfmcp-adapter/src/dfhack_probe.rs"
ADAPTER_LIB = ROOT / "crates/dfmcp-adapter/src/lib.rs"
PROBE_BINARY = ROOT / "crates/dwarf-fortress-mcp/src/bin/dfmcp-live-probe.rs"
PROBE_MANIFEST = ROOT / "crates/dwarf-fortress-mcp/Cargo.toml"

EXPECTED_CASES: dict[str, list[tuple[str, str, str | None]]] = {
    "R2": [
        ("missing_token", "rejected", "AUTH_REQUIRED"),
        ("configured_token_short", "rejected", "AUTH_REQUIRED"),
        ("configured_token_long", "rejected", "AUTH_REQUIRED"),
        ("presented_token_short", "rejected", "AUTH_FAILED"),
        ("presented_token_long", "rejected", "AUTH_FAILED"),
        ("wrong_token", "rejected", "AUTH_FAILED"),
        ("correct_token", "accepted", None),
        ("nonce_short", "rejected", "INVALID_BOUND"),
        ("nonce_long", "rejected", "INVALID_BOUND"),
        ("nonce_mismatch", "rejected", "CLIENT_NONCE_MISMATCH"),
        ("protocol_mismatch", "rejected", "PROTOCOL_MISMATCH"),
        ("secret_scan", "passed", None),
    ],
    "R3": [
        ("baseline_names_included", "accepted", None),
        ("repeat_names_included", "accepted", None),
        ("page_size_1", "accepted", None),
        ("page_size_2", "accepted", None),
        ("page_size_7", "accepted", None),
        ("page_size_64", "accepted", None),
        ("page_size_256", "accepted", None),
        ("page_size_4096", "accepted", None),
        ("baseline_names_omitted", "accepted", None),
        ("repeat_names_omitted", "accepted", None),
        ("offset_at_total", "accepted", None),
        ("offset_beyond_total", "accepted", None),
        ("oversize_request", "rejected", "INVALID_BOUND"),
        ("running_multipage_rejected", "rejected", "PRECONDITIONS_FAILED"),
    ],
    "R4": [
        ("restart_generation_changed", "passed", None),
        ("old_client_rejected", "rejected", "STALE_ANCHOR"),
        ("world_unloaded", "rejected", "WORLD_NOT_LOADED"),
        ("non_fortress_mode", "rejected", "NOT_FORTRESS_MODE"),
        ("summary_drift", "rejected", "ADAPTER_REJECTED"),
        ("partial_not_published", "passed", None),
        ("fresh_handshake", "accepted", None),
    ],
    "R5": [("cold_agent_turn", "accepted", None)],
}
EXPECTED_PAGE_SIZES = [1, 2, 7, 64, 256, 4096]
EXPECTED_OMITTED = [
    "fortress.items",
    "fortress.jobs",
    "fortress.map",
    "fortress.economy",
    "fortress.welfare",
    "fortress.military",
    "fortress.history",
]
EXPECTED_SOURCE_BINDINGS = {
    "bridge_registry": "architecture/dfhack_read_bridge_v1.json",
    "bridge_proto": "bridge/dfhack-plugin/proto/DfmcpBridge.proto",
    "bridge_cpp": "bridge/dfhack-plugin/src/dfmcp_bridge.cpp",
    "rust_wire": "crates/dfmcp-adapter/src/dfhack_wire.rs",
    "rust_acceptance_probe": "crates/dfmcp-adapter/src/dfhack_probe.rs",
    "live_capsule": "crates/dfmcp-adapter/src/live_observation.rs",
    "live_projection": "crates/dfmcp-adapter/src/live_projection.rs",
    "live_adapter": "crates/dfmcp-adapter/src/live_adapter.rs",
    "live_mcp_server": "crates/dfmcp-mcp/src/live_server.rs",
    "acceptance_probe_binary": "crates/dwarf-fortress-mcp/src/bin/dfmcp-live-probe.rs",
    "acceptance_probe_manifest": "crates/dwarf-fortress-mcp/Cargo.toml",
    "acceptance_contract": "architecture/live_read_acceptance_v1.json",
    "acceptance_contract_checker": "scripts/check_live_acceptance_contract.py",
    "acceptance_verifier": "scripts/verify_live_read_acceptance.py",
    "acceptance_journal": "scripts/live_read_evidence_journal.py",
    "acceptance_secret_scanner": "scripts/scan_live_read_secrets.py",
    "acceptance_secret_scanner_tests": "scripts/test_scan_live_read_secrets.py",
}
VERIFIER_IMPORTS = {
    "__future__",
    "argparse",
    "dataclasses",
    "hashlib",
    "json",
    "os",
    "pathlib",
    "re",
    "sys",
    "tempfile",
    "typing",
}
JOURNAL_IMPORTS = VERIFIER_IMPORTS | {
    "platform",
    "shutil",
    "subprocess",
    "verify_live_read_acceptance",
}
SCANNER_IMPORTS = {
    "__future__",
    "argparse",
    "base64",
    "dataclasses",
    "errno",
    "hashlib",
    "json",
    "os",
    "pathlib",
    "stat",
    "sys",
    "tempfile",
    "typing",
}
FORBIDDEN_RUST = ["unsafe {", ".unwrap()", ".expect(", "todo!", "unimplemented!", "panic!("]
FORBIDDEN_EFFECTS = ["RunCommand", "RunLua", "ApplyEffect", "SetPauseState", "SF_ALLOW_REMOTE"]


@dataclass(frozen=True)
class Failure:
    path: str
    message: str


def relative(path: pathlib.Path) -> str:
    return path.relative_to(ROOT).as_posix()


def read(path: pathlib.Path, failures: list[Failure]) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except OSError as exc:
        failures.append(Failure(relative(path), f"cannot read required file: {exc}"))
        return ""


def require(condition: bool, path: pathlib.Path, message: str, failures: list[Failure]) -> None:
    if not condition:
        failures.append(Failure(relative(path), message))


def json_object(path: pathlib.Path, failures: list[Failure]) -> dict[str, Any] | None:
    source = read(path, failures)
    if not source:
        return None
    try:
        value = json.loads(source)
    except json.JSONDecodeError as exc:
        failures.append(Failure(relative(path), f"invalid JSON: {exc}"))
        return None
    if not isinstance(value, dict):
        failures.append(Failure(relative(path), "top-level JSON value must be an object"))
        return None
    return value


def python_tree(path: pathlib.Path, failures: list[Failure]) -> tuple[str, ast.AST | None]:
    source = read(path, failures)
    if not source:
        return source, None
    try:
        return source, ast.parse(source)
    except SyntaxError as exc:
        failures.append(Failure(relative(path), f"syntax error: {exc}"))
        return source, None


def imported_roots(tree: ast.AST) -> set[str]:
    roots: set[str] = set()
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            roots.update(alias.name.split(".", 1)[0] for alias in node.names)
        elif isinstance(node, ast.ImportFrom) and node.module:
            roots.add(node.module.split(".", 1)[0])
    return roots


def functions(tree: ast.AST) -> set[str]:
    return {
        node.name
        for node in ast.walk(tree)
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
    }


def tests(tree: ast.AST) -> set[str]:
    return {name for name in functions(tree) if name.startswith("test_")}


def require_markers(
    source: str,
    path: pathlib.Path,
    markers: list[str],
    failures: list[Failure],
    *,
    case_insensitive: bool = False,
) -> None:
    haystack = source.lower() if case_insensitive else source
    for marker in markers:
        needle = marker.lower() if case_insensitive else marker
        require(needle in haystack, path, f"missing contract marker {marker!r}", failures)


def check_contract(failures: list[Failure]) -> None:
    value = json_object(CONTRACT, failures)
    if value is None:
        return
    require(value.get("schema_version") == "dfmcp.live-read-acceptance/1", CONTRACT, "schema version drifted", failures)
    require(value.get("event_schema") == "dfmcp.live-read-acceptance-event/1", CONTRACT, "event schema drifted", failures)
    require(value.get("receipt_schema") == "dfmcp.live-read-acceptance-receipt/1", CONTRACT, "receipt schema drifted", failures)
    require(value.get("gate_order") == ["R2", "R3", "R4", "R5"], CONTRACT, "gate order drifted", failures)
    limits = value.get("limits")
    require(
        isinstance(limits, dict)
        and set(limits)
        == {
            "maximum_stream_bytes",
            "maximum_event_bytes",
            "maximum_events",
            "maximum_string_bytes",
            "maximum_collection_items",
            "maximum_depth",
        }
        and all(isinstance(item, int) and not isinstance(item, bool) and item > 0 for item in limits.values()),
        CONTRACT,
        "limits must be the exact positive bounded set",
        failures,
    )
    gates = value.get("gates")
    require(isinstance(gates, dict) and list(gates) == ["R2", "R3", "R4", "R5"], CONTRACT, "gate object order or set drifted", failures)
    if isinstance(gates, dict):
        for gate, expected in EXPECTED_CASES.items():
            raw = gates.get(gate, {}).get("required_cases", [])
            actual = [
                (item.get("case"), item.get("result"), item.get("error_code"))
                for item in raw
                if isinstance(item, dict)
            ]
            require(actual == expected, CONTRACT, f"{gate} case/result/error matrix drifted", failures)
        require(gates.get("R3", {}).get("page_sizes") == EXPECTED_PAGE_SIZES, CONTRACT, "R3 page-size matrix drifted", failures)
        require(gates.get("R5", {}).get("required_omitted_domains") == EXPECTED_OMITTED, CONTRACT, "R5 omitted-domain matrix drifted", failures)
    binding = value.get("source_binding", {}).get("required_source_digests")
    require(binding == EXPECTED_SOURCE_BINDINGS, CONTRACT, "source-binding tuple drifted", failures)
    for bound_path in EXPECTED_SOURCE_BINDINGS.values():
        candidate = ROOT / bound_path
        require(candidate.is_file() and not candidate.is_symlink(), CONTRACT, f"bound source is missing or redirected: {bound_path}", failures)
    forbidden = value.get("forbidden_event_material")
    require(
        isinstance(forbidden, dict)
        and {"bearer_token", "credentials", "raw_nonce", "secret", "token"}
        <= set(forbidden.get("keys", []))
        and "DFMCP_BRIDGE_TOKEN=" in forbidden.get("substrings", []),
        CONTRACT,
        "secret-material denylist is incomplete",
        failures,
    )
    capture = value.get("evidence_capture")
    require(isinstance(capture, dict), CONTRACT, "evidence_capture policy is missing", failures)
    if isinstance(capture, dict):
        require(capture.get("raw_probe") == "dfmcp-live-probe", CONTRACT, "probe identity drifted", failures)
        require(capture.get("journal") == "scripts/live_read_evidence_journal.py", CONTRACT, "journal identity drifted", failures)
        require(capture.get("secret_scanner") == "scripts/scan_live_read_secrets.py", CONTRACT, "scanner identity drifted", failures)
        require("root last" in capture.get("publication_order", ""), CONTRACT, "root-last rule is missing", failures)
        require("one raw probe result may not fabricate" in capture.get("composite_case_policy", ""), CONTRACT, "composite-case rule is missing", failures)
        require("file-identity drift" in capture.get("secret_scan_policy", ""), CONTRACT, "stable secret-scan rule is missing", failures)


def check_python_component(
    path: pathlib.Path,
    allowed_imports: set[str],
    required_functions: set[str],
    markers: list[str],
    failures: list[Failure],
) -> None:
    source, tree = python_tree(path, failures)
    if tree is None:
        return
    unexpected = imported_roots(tree) - allowed_imports
    require(not unexpected, path, f"unexpected import roots: {sorted(unexpected)}", failures)
    present = functions(tree)
    for name in sorted(required_functions):
        require(name in present, path, f"required function {name} is missing", failures)
    require_markers(source, path, markers, failures, case_insensitive=True)


def check_verifier(failures: list[Failure]) -> None:
    check_python_component(
        VERIFIER,
        VERIFIER_IMPORTS,
        {
            "bounded_tree",
            "reject_secret_material",
            "validate_native_build_receipt",
            "validate_r2",
            "validate_r3",
            "validate_r4",
            "validate_r5",
            "build_receipt",
            "verify_acceptance",
            "write_atomic",
        },
        [
            "maximum_stream_bytes",
            "maximum_event_bytes",
            "expected-dfmcp-commit",
            "native-build-receipt",
            "allow-synthetic",
            "receipt_digest",
            "qualified",
        ],
        failures,
    )
    _, tree = python_tree(VERIFIER_TESTS, failures)
    if tree is not None:
        present = tests(tree)
        expected = {
            "test_valid_evidence_is_qualified_and_deterministic",
            "test_missing_case_fails_closed",
            "test_pagination_digest_drift_is_rejected",
            "test_secret_bearing_key_is_rejected",
            "test_restart_generation_must_change",
            "test_partial_capsule_cannot_publish",
            "test_agent_turn_cannot_advertise_mutation",
            "test_duplicate_event_identity_is_rejected",
            "test_oversized_event_line_is_rejected",
        }
        require(expected <= present and len(present) >= len(expected), VERIFIER_TESTS, "verifier adversarial test matrix is incomplete", failures)


def check_probe(failures: list[Failure]) -> None:
    adapter = read(PROBE_ADAPTER, failures)
    require_markers(
        adapter,
        PROBE_ADAPTER,
        [
            "#![forbid(unsafe_code)]",
            "DfHackProbeClient",
            "ProbeHandshakeRequest",
            "ProbeObservationRequest",
            "MAX_PROBE_FIELD_BYTES",
            'field("bearer_token", &"<redacted>")',
            "negotiate_transport",
            "protobuf varint is not minimally encoded",
            "raw_probe_sends_locally_invalid_credentials_and_returns_typed_rejection",
            "raw_probe_can_send_an_oversized_protocol_page_bound",
        ],
        failures,
    )
    for token in [*FORBIDDEN_RUST, "TcpStream", *FORBIDDEN_EFFECTS]:
        require(token not in adapter, PROBE_ADAPTER, f"forbidden raw-probe token {token!r}", failures)

    binary = read(PROBE_BINARY, failures)
    require_markers(
        binary,
        PROBE_BINARY,
        [
            "#![forbid(unsafe_code)]",
            '"handshake-case"',
            '"observation-case"',
            '"capsule"',
            '"agent-turn"',
            "parse_loopback_endpoint",
            "DfHackProbeClient::negotiate_transport",
            "LiveObservationReceipt::issue",
            "receipt_sha256",
            "sensitive_manifest_disclosed",
            "running_multipage_rejected",
            "probe never prints the bearer token",
        ],
        failures,
        case_insensitive=True,
    )
    for token in [*FORBIDDEN_RUST, *FORBIDDEN_EFFECTS]:
        require(token not in binary, PROBE_BINARY, f"forbidden probe-binary token {token!r}", failures)
    require('println!("{}", token' not in binary, PROBE_BINARY, "probe binary prints a token variable", failures)

    manifest = read(PROBE_MANIFEST, failures)
    require('name = "dfmcp-live-probe"' in manifest, PROBE_MANIFEST, "probe binary is not registered", failures)
    require('path = "src/bin/dfmcp-live-probe.rs"' in manifest, PROBE_MANIFEST, "probe path drifted", failures)
    adapter_lib = read(ADAPTER_LIB, failures)
    require("pub mod dfhack_probe;" in adapter_lib, ADAPTER_LIB, "raw probe module is not compiled", failures)
    require("DfHackProbeClient" in adapter_lib, ADAPTER_LIB, "raw probe API is not exported", failures)


def check_journal(failures: list[Failure]) -> None:
    check_python_component(
        JOURNAL,
        JOURNAL_IMPORTS,
        {
            "initialize",
            "load_journal",
            "normalize_probe",
            "validate_event",
            "append_record",
            "append_event",
            "show_status",
            "finalize",
            "atomic_write_bytes",
        },
        [
            "raw artifact first",
            "journal root last",
            "orphan event file conflicts",
            "sealed evidence journal cannot be modified",
            "verify_acceptance",
            "native-build-passed",
            "SHA256SUMS",
            "os.replace(staging, target)",
        ],
        failures,
    )
    _, tree = python_tree(JOURNAL_TESTS, failures)
    if tree is not None:
        present = tests(tree)
        expected = {
            "test_wrong_token_probe_normalizes_without_secret_material",
            "test_probe_acceptance_must_match_the_normative_case",
            "test_page_size_case_is_bound_to_the_requested_size",
            "test_name_projection_cases_are_not_interchangeable",
            "test_secret_bearing_normalized_event_is_rejected",
            "test_append_writes_artifacts_before_advancing_root",
            "test_sealed_journal_rejects_new_events",
            "test_composite_cases_cannot_be_fabricated_from_one_probe",
            "test_event_identity_must_match_next_slot",
        }
        require(expected <= present, JOURNAL_TESTS, "journal adversarial test matrix is incomplete", failures)


def check_scanner(failures: list[Failure]) -> None:
    check_python_component(
        SCANNER,
        SCANNER_IMPORTS,
        {
            "validate_token",
            "representations",
            "ensure_real_output",
            "read_stable_regular_file",
            "regular_files",
            "scan",
            "atomic_write_json",
        },
        [
            "MAX_FILES = 512",
            "MAX_FILE_BYTES = 32 * 1024 * 1024",
            "MAX_TOTAL_BYTES = 256 * 1024 * 1024",
            "O_NOFOLLOW",
            "os.fstat",
            "path.lstat",
            "file identity",
            '"raw"',
            '"hex_lower"',
            '"base64"',
            '"base64_urlsafe"',
            '"environment_assignment"',
            "token_fingerprint_sha256",
            "match_count",
        ],
        failures,
    )
    _, tree = python_tree(SCANNER_TESTS, failures)
    if tree is not None:
        present = tests(tree)
        expected = {
            "test_clean_artifacts_produce_normalized_zero_match_event",
            "test_raw_token_leak_is_detected_without_echoing_secret",
            "test_hex_and_base64_representations_are_detected",
            "test_environment_assignment_is_detected",
            "test_symbolic_link_artifact_is_rejected",
            "test_symbolic_link_output_is_rejected",
            "test_stable_reader_rejects_replaced_file_identity",
            "test_oversized_file_is_rejected_before_read",
            "test_token_policy_is_enforced",
        }
        require(expected <= present, SCANNER_TESTS, "scanner adversarial test matrix is incomplete", failures)


def check_wrapper_and_registry(failures: list[Failure]) -> None:
    wrapper = read(WRAPPER, failures)
    require_markers(
        wrapper,
        WRAPPER,
        [
            "git rev-parse HEAD",
            "git status --porcelain=v1",
            "--native-build-receipt",
            "--expected-dfmcp-commit",
            "live-read-acceptance-receipt.json",
            "SHA256SUMS",
        ],
        failures,
    )
    require("--allow-synthetic" not in wrapper, WRAPPER, "live wrapper admits synthetic evidence", failures)

    registry = json_object(BRIDGE_REGISTRY, failures)
    if registry is not None:
        evidence = registry.get("acceptance_evidence")
        require(isinstance(evidence, dict), BRIDGE_REGISTRY, "acceptance_evidence registry is missing", failures)
        if isinstance(evidence, dict):
            require(evidence.get("contract") == "architecture/live_read_acceptance_v1.json", BRIDGE_REGISTRY, "acceptance contract reference drifted", failures)
            require(evidence.get("verifier") == "scripts/verify_live_read_acceptance.py", BRIDGE_REGISTRY, "acceptance verifier reference drifted", failures)
            require(evidence.get("qualification_wrapper") == "scripts/qualify_live_read.sh", BRIDGE_REGISTRY, "acceptance wrapper reference drifted", failures)


def check_qualification_wiring(failures: list[Failure]) -> None:
    common = [
        "scripts/check_live_acceptance_contract.py",
        "scripts/test_live_read_acceptance.py",
        "scripts/live_read_evidence_journal.py",
        "scripts/test_live_read_evidence_journal.py",
        "scripts/scan_live_read_secrets.py",
        "scripts/test_scan_live_read_secrets.py",
        "scripts/qualify_live_read.sh",
    ]
    for relative_path in ["scripts/verify.sh", "scripts/qualify_local.sh"]:
        path = ROOT / relative_path
        source = read(path, failures)
        for marker in common:
            require(marker in source, path, f"qualification wiring omits {marker}", failures)
        require("dfmcp-live-probe" in source, path, "qualification does not execute probe help", failures)
    qualify = read(ROOT / "scripts/qualify_local.sh", failures)
    for digest_name in [
        "dfhack_acceptance_probe",
        "live_acceptance_probe_binary",
        "live_acceptance_journal",
        "live_acceptance_secret_scanner",
        "live_acceptance_secret_scanner_tests",
    ]:
        require(digest_name in qualify, ROOT / "scripts/qualify_local.sh", f"qualification receipt omits {digest_name}", failures)
    native = read(ROOT / "scripts/qualify_dfhack_plugin.sh", failures)
    for marker in [
        "scripts/check_live_acceptance_contract.py",
        "architecture/live_read_acceptance_v1.json",
        "scripts/verify_live_read_acceptance.py",
        "scripts/scan_live_read_secrets.py",
    ]:
        require(marker in native, ROOT / "scripts/qualify_dfhack_plugin.sh", f"R1 receipt/wiring omits {marker}", failures)


def main() -> int:
    failures: list[Failure] = []
    check_contract(failures)
    check_verifier(failures)
    check_probe(failures)
    check_journal(failures)
    check_scanner(failures)
    check_wrapper_and_registry(failures)
    check_qualification_wiring(failures)
    if failures:
        print(f"live acceptance contract: FAIL ({len(failures)} violation(s))", file=sys.stderr)
        for failure in failures:
            print(f"  {failure.path}: {failure.message}", file=sys.stderr)
        return 1
    print(
        "live acceptance contract: PASS (bounded probe, stable scanner, root-last journal, verifier, and qualification wiring)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
