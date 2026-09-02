#!/usr/bin/env python3
"""Validate that machine contracts and public status prose describe one evidence posture."""

from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path
from typing import Any, NoReturn

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_CONTRACT = Path("architecture/implementation_status_v1.json")
IGNORED_DIRECTORIES = {".git", ".venv", "__pycache__", "node_modules", "target"}


class StatusContractError(ValueError):
    pass


def fail(message: str) -> NoReturn:
    raise StatusContractError(message)


def duplicate_rejecting_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    output: dict[str, Any] = {}
    for key, value in pairs:
        if key in output:
            fail(f"JSON object repeats key {key!r}")
        output[key] = value
    return output


def read_json(path: Path, label: str) -> dict[str, Any]:
    try:
        raw = path.read_bytes()
    except OSError as exc:
        fail(f"cannot read {label}: {exc}")
    if not raw or len(raw) > 8 * 1024 * 1024:
        fail(f"{label} must contain 1..=8388608 bytes")
    try:
        value = json.loads(
            raw.decode("utf-8"),
            object_pairs_hook=duplicate_rejecting_object,
        )
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        fail(f"cannot parse {label}: {exc}")
    if not isinstance(value, dict):
        fail(f"{label} must be a JSON object")
    return value


def require_exact_keys(value: dict[str, Any], expected: set[str], path: str) -> None:
    actual = set(value)
    if actual != expected:
        fail(f"{path} fields differ: expected {sorted(expected)}, got {sorted(actual)}")


def require_string(value: Any, path: str, maximum: int = 16 * 1024) -> str:
    if not isinstance(value, str) or not value or len(value.encode("utf-8")) > maximum:
        fail(f"{path} must contain 1..={maximum} UTF-8 bytes")
    if any(ord(character) < 0x20 for character in value):
        fail(f"{path} contains a control character")
    return value


def require_string_list(value: Any, path: str) -> list[str]:
    if not isinstance(value, list):
        fail(f"{path} must be an array")
    output: list[str] = []
    for index, item in enumerate(value):
        text = require_string(item, f"{path}[{index}]", 4096)
        if text in output:
            fail(f"{path} repeats {text!r}")
        output.append(text)
    return output


def resolve_repository_path(root: Path, value: Any, path: str) -> Path:
    relative = require_string(value, path, 4096)
    candidate = Path(relative)
    if candidate.is_absolute() or ".." in candidate.parts or candidate.as_posix() != relative:
        fail(f"{path} must be a canonical repository-relative path")
    resolved = root / candidate
    if not resolved.is_file() or resolved.is_symlink():
        fail(f"{path} does not name a regular repository file: {relative}")
    return resolved


def validate_contract(root: Path, contract_path: Path) -> dict[str, Any]:
    contract = read_json(contract_path, "implementation status contract")
    require_exact_keys(
        contract,
        {
            "schema_version",
            "status",
            "phase",
            "compatibility_registry",
            "production_admission",
            "development_runtime",
            "bridge_generations",
            "authoritative_documents",
            "forbidden_tracked_evidence_basenames",
            "authority",
            "limitations",
        },
        "contract",
    )
    if contract.get("schema_version") != "dfmcp.implementation-status-contract/1":
        fail("implementation status contract schema is unsupported")
    if contract.get("status") != "normative_current_evidence_posture":
        fail("implementation status contract status drifted")
    if contract.get("phase") != "0D-R0+1A-source":
        fail("implementation phase drifted without a reviewed status-contract version")
    authority = contract.get("authority")
    if not isinstance(authority, dict):
        fail("contract.authority must be an object")
    require_exact_keys(authority, {"grants_capabilities", "mutation_capabilities"}, "contract.authority")
    if authority.get("grants_capabilities") != [] or authority.get("mutation_capabilities") != []:
        fail("implementation status contract grants authority")
    require_string_list(contract.get("limitations"), "contract.limitations")
    return contract


def check_registry(root: Path, contract: dict[str, Any]) -> None:
    expected = contract.get("compatibility_registry")
    if not isinstance(expected, dict):
        fail("contract.compatibility_registry must be an object")
    require_exact_keys(
        expected,
        {"path", "schema_version", "required_status", "required_entry_count"},
        "contract.compatibility_registry",
    )
    path = resolve_repository_path(root, expected.get("path"), "contract.compatibility_registry.path")
    registry = read_json(path, "compatibility registry")
    require_exact_keys(registry, {"schema_version", "status", "entries"}, "compatibility_registry")
    if registry.get("schema_version") != expected.get("schema_version"):
        fail("compatibility registry schema differs from the current status contract")
    if registry.get("status") != expected.get("required_status"):
        fail("compatibility registry status differs from the current status contract")
    entries = registry.get("entries")
    if not isinstance(entries, list):
        fail("compatibility_registry.entries must be an array")
    required_count = expected.get("required_entry_count")
    if isinstance(required_count, bool) or not isinstance(required_count, int) or required_count < 0:
        fail("contract compatibility entry count must be a nonnegative integer")
    if len(entries) != required_count:
        fail(
            f"checked-in compatibility registry has {len(entries)} entries; "
            f"the current status contract requires {required_count}"
        )


def check_production_admission(root: Path, contract: dict[str, Any]) -> None:
    expected = contract.get("production_admission")
    if not isinstance(expected, dict):
        fail("contract.production_admission must be an object")
    require_exact_keys(
        expected,
        {
            "path",
            "schema_version",
            "launch_schema",
            "ticket_schema",
            "admitted_protocols",
            "protocol_1_1_status",
            "unknown_protocol_policy",
        },
        "contract.production_admission",
    )
    path = resolve_repository_path(root, expected.get("path"), "contract.production_admission.path")
    admission = read_json(path, "production admission contract")
    if admission.get("schema_version") != expected.get("schema_version"):
        fail("production admission contract schema differs from status contract")
    if admission.get("launch_schema") != expected.get("launch_schema"):
        fail("production launch schema differs from status contract")
    if admission.get("ticket_schema") != expected.get("ticket_schema"):
        fail("production ticket schema differs from status contract")
    dispatch = admission.get("runtime_dispatch")
    if not isinstance(dispatch, dict):
        fail("production admission runtime_dispatch must be an object")
    if dispatch.get("admitted_protocols") != expected.get("admitted_protocols"):
        fail("production protocol runner map differs from the current status contract")
    if dispatch.get("protocol_1_1_status") != expected.get("protocol_1_1_status"):
        fail("protocol-1.1 production status differs from the current status contract")
    if dispatch.get("unknown_or_unadmitted_protocol_policy") != expected.get("unknown_protocol_policy"):
        fail("unknown protocol policy differs from the current status contract")
    canonical = admission.get("canonical_binding")
    if not isinstance(canonical, dict):
        fail("production admission canonical_binding must be an object")
    if canonical.get("legacy_ticket_schema_accepted") is not False:
        fail("production admission accepts a legacy ticket schema")
    authority = admission.get("authority")
    if not isinstance(authority, dict) or authority.get("mutation_capabilities") != []:
        fail("production admission contract grants mutation capability")


def check_development_runtime(root: Path, contract: dict[str, Any]) -> None:
    expected = contract.get("development_runtime")
    if not isinstance(expected, dict):
        fail("contract.development_runtime must be an object")
    require_exact_keys(
        expected,
        {
            "path",
            "schema_version",
            "required_status",
            "binary",
            "exact_opt_in",
            "production_protocol_dispatch_allowed",
            "compatibility_admitted",
            "server_artifact_qualified",
            "runtime_admitted",
        },
        "contract.development_runtime",
    )
    path = resolve_repository_path(root, expected.get("path"), "contract.development_runtime.path")
    runtime = read_json(path, "protocol-1.1 development runtime contract")
    if runtime.get("schema_version") != expected.get("schema_version"):
        fail("development runtime schema differs from status contract")
    if runtime.get("status") != expected.get("required_status"):
        fail("development runtime status differs from status contract")
    binary = runtime.get("binary")
    if not isinstance(binary, dict):
        fail("development runtime binary must be an object")
    if binary.get("name") != expected.get("binary"):
        fail("development runtime binary differs from status contract")
    if binary.get("required_opt_in_environment") != expected.get("exact_opt_in"):
        fail("development runtime opt-in differs from status contract")
    if binary.get("production_protocol_dispatch_allowed") != expected.get(
        "production_protocol_dispatch_allowed"
    ):
        fail("development runtime production-dispatch posture drifted")
    authority = runtime.get("authority")
    if not isinstance(authority, dict):
        fail("development runtime authority must be an object")
    for field in ["compatibility_admitted", "server_artifact_qualified", "runtime_admitted"]:
        if authority.get(field) != expected.get(field):
            fail(f"development runtime {field} differs from status contract")
    if authority.get("grants_capabilities") != [] or authority.get("mutation_capabilities") != []:
        fail("development runtime contract grants authority")


def check_bridge_generations(root: Path, contract: dict[str, Any]) -> None:
    generations = contract.get("bridge_generations")
    if not isinstance(generations, list) or len(generations) != 2:
        fail("contract.bridge_generations must contain exactly protocol 1.0 and 1.1")
    observed_protocols: list[str] = []
    for index, expected in enumerate(generations):
        if not isinstance(expected, dict):
            fail(f"contract.bridge_generations[{index}] must be an object")
        allowed = {"protocol", "path", "method_manifest", "mutation_capabilities", "required_status"}
        if not set(expected).issubset(allowed):
            fail(f"contract.bridge_generations[{index}] contains unknown fields")
        protocol = require_string(expected.get("protocol"), f"contract.bridge_generations[{index}].protocol", 16)
        observed_protocols.append(protocol)
        path = resolve_repository_path(root, expected.get("path"), f"contract.bridge_generations[{index}].path")
        bridge = read_json(path, f"protocol-{protocol} bridge contract")
        if bridge.get("method_manifest") != expected.get("method_manifest"):
            fail(f"protocol {protocol} method manifest differs from status contract")
        methods = bridge.get("methods")
        if not isinstance(methods, list):
            fail(f"protocol {protocol} methods must be an array")
        names: list[str] = []
        for method_index, method in enumerate(methods):
            if not isinstance(method, dict):
                fail(f"protocol {protocol} method {method_index} must be an object")
            name = require_string(method.get("name"), f"protocol {protocol} method name", 128)
            names.append(name)
            if method.get("effect") != "read_only":
                fail(f"protocol {protocol} method {name!r} is not read-only")
        if names != expected.get("method_manifest"):
            fail(f"protocol {protocol} method definitions differ from its method manifest")
        if expected.get("mutation_capabilities") != []:
            fail(f"status contract unexpectedly grants protocol {protocol} mutation")
        if "required_status" in expected and bridge.get("status") != expected.get("required_status"):
            fail(f"protocol {protocol} status differs from the current status contract")
    if observed_protocols != ["1.0", "1.1"]:
        fail("bridge generation order must remain protocol 1.0 then 1.1")


def check_documents(root: Path, contract: dict[str, Any]) -> None:
    documents = contract.get("authoritative_documents")
    if not isinstance(documents, dict) or not documents:
        fail("contract.authoritative_documents must be a nonempty object")
    for relative, raw_markers in documents.items():
        path = resolve_repository_path(root, relative, f"contract.authoritative_documents.{relative}")
        markers = require_string_list(raw_markers, f"contract.authoritative_documents.{relative}")
        try:
            text = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError) as exc:
            fail(f"cannot read authoritative document {relative}: {exc}")
        normalized = text.casefold()
        for marker in markers:
            if marker.casefold() not in normalized:
                fail(f"authoritative document {relative} omits status marker {marker!r}")


def check_forbidden_evidence(root: Path, contract: dict[str, Any]) -> None:
    forbidden = set(
        require_string_list(
            contract.get("forbidden_tracked_evidence_basenames"),
            "contract.forbidden_tracked_evidence_basenames",
        )
    )
    for directory, names, files in os.walk(root, followlinks=False):
        names[:] = sorted(name for name in names if name not in IGNORED_DIRECTORIES)
        for name in sorted(files):
            if name in forbidden:
                relative = (Path(directory) / name).relative_to(root).as_posix()
                fail(f"repository contains deployment or qualification evidence file {relative}")


def check(root: Path, contract_path: Path) -> None:
    repository_root = root.resolve(strict=True)
    if not repository_root.is_dir():
        fail("repository root is not a directory")
    effective_contract = contract_path
    if not effective_contract.is_absolute():
        effective_contract = repository_root / effective_contract
    if not effective_contract.is_file() or effective_contract.is_symlink():
        fail("implementation status contract is missing or symbolic")
    contract = validate_contract(repository_root, effective_contract)
    check_registry(repository_root, contract)
    check_production_admission(repository_root, contract)
    check_development_runtime(repository_root, contract)
    check_bridge_generations(repository_root, contract)
    check_documents(repository_root, contract)
    check_forbidden_evidence(repository_root, contract)


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--contract", type=Path, default=DEFAULT_CONTRACT)
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    try:
        check(args.root, args.contract)
    except (OSError, StatusContractError) as exc:
        print(f"implementation status: FAIL: {exc}", file=sys.stderr)
        return 1
    print(
        "implementation status: PASS "
        "(empty registry, protocol-1.0 production map, unadmitted 1.1 development, no mutation)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
