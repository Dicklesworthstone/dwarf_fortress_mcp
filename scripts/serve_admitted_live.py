#!/usr/bin/env python3
"""Resolve an exact tuple, verify one qualified inode, and exec read-only live MCP."""

from __future__ import annotations

import argparse
import importlib.util
import json
import os
import sys
from pathlib import Path
from typing import Any, NoReturn

ROOT = Path(__file__).resolve().parents[1]
PROMOTION_PATH = ROOT / "scripts/promote_live_compatibility.py"
RESOLVER_PATH = ROOT / "scripts/resolve_live_compatibility.py"
BINARY_VERIFIER_PATH = ROOT / "scripts/verify_live_server_binary_receipt.py"
DEFAULT_REGISTRY = ROOT / "architecture/live_compatibility_registry_v1.json"
DEFAULT_BINARY = ROOT / "target/release/dwarf-fortress-mcp"
DEFAULT_BINARY_CONTRACT = ROOT / "architecture/live_server_binary_receipt_v1.json"
LAUNCH_SCHEMA = "dfmcp.admitted-live-launch/1"


def load_module(name: str, path: Path) -> Any:
    specification = importlib.util.spec_from_file_location(name, path)
    if specification is None or specification.loader is None:
        raise RuntimeError(f"cannot load {name}")
    module = importlib.util.module_from_spec(specification)
    sys.modules[specification.name] = module
    specification.loader.exec_module(module)
    return module


promotion = load_module("promote_live_compatibility", PROMOTION_PATH)
resolver = load_module("resolve_live_compatibility", RESOLVER_PATH)
binary_verifier = load_module("verify_live_server_binary_receipt", BINARY_VERIFIER_PATH)


class LaunchError(ValueError):
    pass


def fail(message: str) -> NoReturn:
    raise LaunchError(message)


def validate_token_environment(environment: dict[str, str]) -> None:
    token = environment.get("DFMCP_BRIDGE_TOKEN")
    if token is None:
        fail("DFMCP_BRIDGE_TOKEN is required in the inherited environment")
    length = len(token.encode("utf-8"))
    if length < 32 or length > 256:
        fail("DFMCP_BRIDGE_TOKEN must contain 32..=256 UTF-8 bytes")
    if "\x00" in token:
        fail("DFMCP_BRIDGE_TOKEN contains a NUL byte")


def validate_loader_environment(environment: dict[str, str]) -> None:
    forbidden = sorted(
        key
        for key in environment
        if key.startswith("LD_")
        or key.startswith("DYLD_")
        or key
        in {
            "GLIBC_TUNABLES",
            "LIBPATH",
            "LDR_CONFIG",
            "LDR_PRELOAD",
            "SHLIB_PATH",
        }
    )
    if forbidden:
        fail(
            "dynamic-loader override variables are forbidden for admitted live execution: "
            + ", ".join(forbidden)
        )


def build_launch_record(
    compatibility_decision: dict[str, Any],
    normalized_server_receipt: dict[str, Any],
    opened_binary: Any,
) -> dict[str, Any]:
    if compatibility_decision.get("admitted") is not True:
        fail("compatibility decision is not admitted")
    if compatibility_decision.get("required_entry_id") != compatibility_decision.get("entry_id"):
        fail("compatibility decision is not fenced to its admitted entry identifier")
    source_commit = compatibility_decision["manifest"]["source"]["dfmcp_commit"]
    if normalized_server_receipt["source"]["dfmcp_commit"] != source_commit:
        fail("server binary receipt and compatibility decision name different source commits")
    if normalized_server_receipt["platform"] != compatibility_decision["manifest"]["platform"]:
        fail("server binary receipt and compatibility decision name different platforms")
    if normalized_server_receipt.get("mutation_capabilities") != []:
        fail("qualified server binary unexpectedly carries mutation capabilities")
    if opened_binary.sha256 != normalized_server_receipt["binary"]["sha256"]:
        fail("opened server inode differs from the normalized binary receipt")
    if opened_binary.size != normalized_server_receipt["binary"]["bytes"]:
        fail("opened server inode size differs from the normalized binary receipt")
    unsigned: dict[str, Any] = {
        "schema": LAUNCH_SCHEMA,
        "state": "authorized_to_exec",
        "compatibility_entry_id": compatibility_decision["entry_id"],
        "required_entry_id": compatibility_decision["required_entry_id"],
        "compatibility_decision_digest": compatibility_decision["decision_digest"],
        "compatibility_registry_digest": compatibility_decision["registry_digest"],
        "support_level": compatibility_decision["support_level"],
        "deployment_manifest": compatibility_decision["manifest"],
        "server_receipt": {
            "file_sha256": normalized_server_receipt["receipt_sha256"],
            "content_digest": normalized_server_receipt["receipt_digest"],
            "local_qualification_receipt_sha256": normalized_server_receipt["source"][
                "local_qualification_receipt_sha256"
            ],
        },
        "server_binary": {
            "path": os.fspath(opened_binary.path),
            "sha256": opened_binary.sha256,
            "bytes": opened_binary.size,
            "device": opened_binary.device,
            "inode": opened_binary.inode,
            "mode": opened_binary.mode,
            "owner_uid": opened_binary.owner_uid,
        },
        "capabilities": list(compatibility_decision["capabilities"]),
        "mode": "authenticated_live_read_only",
        "mutation_capabilities": [],
    }
    return {
        **unsigned,
        "launch_digest": promotion.sha256_bytes(promotion.canonical_json(unsigned)),
    }


def prepare_launch(
    registry_path: Path,
    manifest_path: Path,
    binary_path: Path,
    server_receipt_path: Path,
    local_qualification_receipt: Path,
    binary_contract_path: Path,
    source_root: Path,
    expected_dfmcp_commit: str,
    required_entry_id: str,
    environment: dict[str, str],
) -> tuple[Any, dict[str, Any]]:
    validate_token_environment(environment)
    validate_loader_environment(environment)
    registry = promotion.read_object(
        registry_path, promotion.MAX_JSON_BYTES, "compatibility registry"
    )
    manifest = promotion.read_object(
        manifest_path, 1024 * 1024, "deployment manifest"
    )
    decision = resolver.resolve(registry, manifest, required_entry_id)
    if decision["admitted"] is not True:
        fail("deployment manifest has no exact admitted compatibility entry")
    expected_commit = promotion.require_commit(
        expected_dfmcp_commit, "expected_dfmcp_commit"
    )
    if decision["manifest"]["source"]["dfmcp_commit"] != expected_commit:
        fail("deployment manifest source commit differs from the explicit launch fence")
    normalized_receipt, opened = binary_verifier.verify(
        server_receipt_path,
        binary_path,
        binary_contract_path,
        source_root,
        local_qualification_receipt,
        expected_commit,
    )
    try:
        record = build_launch_record(decision, normalized_receipt, opened)
    except BaseException:
        os.close(opened.descriptor)
        raise
    return opened, record


def admitted_environment(
    source: dict[str, str], record: dict[str, Any]
) -> dict[str, str]:
    environment = dict(source)
    environment["DFMCP_COMPATIBILITY_ENTRY_ID"] = record["compatibility_entry_id"]
    environment["DFMCP_COMPATIBILITY_DECISION_DIGEST"] = record[
        "compatibility_decision_digest"
    ]
    environment["DFMCP_COMPATIBILITY_REGISTRY_DIGEST"] = record[
        "compatibility_registry_digest"
    ]
    environment["DFMCP_SERVER_RECEIPT_DIGEST"] = record["server_receipt"][
        "content_digest"
    ]
    environment["DFMCP_ADMITTED_LAUNCH_DIGEST"] = record["launch_digest"]
    validate_loader_environment(environment)
    return environment


def execute_verified_descriptor(
    opened_binary: Any, environment: dict[str, str]
) -> NoReturn:
    if os.execve not in getattr(os, "supports_fd", set()):
        fail(
            "this Python runtime cannot execute an opened descriptor; "
            "refusing a path-based live-execution fallback"
        )
    arguments = [opened_binary.path.name, "serve-live"]
    os.execve(opened_binary.descriptor, arguments, environment)
    fail("descriptor-based exec returned unexpectedly")


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("--registry", type=Path, default=DEFAULT_REGISTRY)
    parser.add_argument("--binary", type=Path, default=DEFAULT_BINARY)
    parser.add_argument("--server-receipt", type=Path, required=True)
    parser.add_argument("--local-qualification-receipt", type=Path, required=True)
    parser.add_argument("--binary-contract", type=Path, default=DEFAULT_BINARY_CONTRACT)
    parser.add_argument("--source-root", type=Path, default=ROOT)
    parser.add_argument("--expected-dfmcp-commit", required=True)
    parser.add_argument("--require-entry-id", required=True)
    parser.add_argument("--launch-record", type=Path, required=True)
    parser.add_argument("--dry-run", action="store_true")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    opened = None
    try:
        inherited = dict(os.environ)
        opened, record = prepare_launch(
            args.registry,
            args.manifest,
            args.binary,
            args.server_receipt,
            args.local_qualification_receipt,
            args.binary_contract,
            args.source_root,
            args.expected_dfmcp_commit,
            args.require_entry_id,
            inherited,
        )
        promotion.write_atomic(args.launch_record, record)
        if args.dry_run:
            os.close(opened.descriptor)
            opened = None
            print(
                json.dumps(
                    record,
                    sort_keys=True,
                    separators=(",", ":"),
                    ensure_ascii=False,
                )
            )
            return 0
        environment = admitted_environment(inherited, record)
        execute_verified_descriptor(opened, environment)
    except (
        OSError,
        KeyError,
        TypeError,
        promotion.PromotionError,
        resolver.ResolutionError,
        binary_verifier.VerificationError,
        LaunchError,
    ) as exc:
        if opened is not None:
            try:
                os.close(opened.descriptor)
            except OSError:
                pass
        print(f"admitted live launcher: FAIL: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
