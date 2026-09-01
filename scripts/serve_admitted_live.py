#!/usr/bin/env python3
"""Resolve an exact tuple, verify one qualified inode, and exec read-only live MCP."""

from __future__ import annotations

import argparse
import importlib.util
import json
import os
import stat
import sys
import time
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
TICKET_SCHEMA = "dfmcp.live-admission-ticket/1"
TICKET_DIRECTORY_NAME = ".dfmcp-admission"
TICKET_TTL_SECONDS = 120
MAX_TICKET_BYTES = 64 * 1024


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


def ensure_private_ticket_directory(path: Path) -> Path:
    absolute = path if path.is_absolute() else Path.cwd() / path
    try:
        os.mkdir(absolute, 0o700)
    except FileExistsError:
        pass
    except OSError as exc:
        fail(f"cannot create admission ticket directory {absolute}: {exc}")
    try:
        metadata = os.lstat(absolute)
    except OSError as exc:
        fail(f"cannot inspect admission ticket directory {absolute}: {exc}")
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISDIR(metadata.st_mode):
        fail("admission ticket directory must be a real directory, not a symbolic link")
    if stat.S_IMODE(metadata.st_mode) != 0o700:
        fail("admission ticket directory must have exact owner-only mode 0700")
    if hasattr(os, "geteuid") and metadata.st_uid != os.geteuid():
        fail("admission ticket directory is not owned by the launching effective user")
    return absolute


def build_admission_ticket(
    record: dict[str, Any],
    opened_binary: Any,
    *,
    now_unix_seconds: int | None = None,
    process_id: int | None = None,
    ticket_id: str | None = None,
) -> dict[str, Any]:
    now = int(time.time()) if now_unix_seconds is None else now_unix_seconds
    pid = os.getpid() if process_id is None else process_id
    identifier = (
        promotion.sha256_bytes(os.urandom(32)) if ticket_id is None else ticket_id
    )
    promotion.require_hash(identifier, "admission_ticket.ticket_id")
    if now < 0:
        fail("admission ticket creation time must not be negative")
    if pid <= 0 or pid > 0xFFFF_FFFF:
        fail("admission ticket process ID is outside the u32 domain")
    metadata = os.fstat(opened_binary.descriptor)
    if (
        metadata.st_dev != opened_binary.device
        or metadata.st_ino != opened_binary.inode
        or metadata.st_size != opened_binary.size
    ):
        fail("opened server binary changed before admission ticket issuance")
    unsigned: dict[str, Any] = {
        "schema": TICKET_SCHEMA,
        "state": "authorized_to_exec",
        "ticket_id": identifier,
        "process_id": pid,
        "created_unix_seconds": now,
        "expires_unix_seconds": now + TICKET_TTL_SECONDS,
        "compatibility_entry_id": record["compatibility_entry_id"],
        "compatibility_registry_digest": record["compatibility_registry_digest"],
        "compatibility_decision_digest": record["compatibility_decision_digest"],
        "server_receipt_digest": record["server_receipt"]["content_digest"],
        "launch_digest": record["launch_digest"],
        "server_binary_sha256": record["server_binary"]["sha256"],
        "server_binary_device": metadata.st_dev,
        "server_binary_inode": metadata.st_ino,
        "server_binary_bytes": metadata.st_size,
        "server_binary_mode": metadata.st_mode,
        "server_binary_owner_uid": metadata.st_uid,
        "mode": "authenticated_live_read_only",
        "capabilities": list(record["capabilities"]),
        "mutation_capabilities": [],
    }
    return {
        **unsigned,
        "ticket_digest": promotion.sha256_bytes(promotion.canonical_json(unsigned)),
    }


def fsync_directory(path: Path) -> None:
    flags = os.O_RDONLY
    if hasattr(os, "O_DIRECTORY"):
        flags |= os.O_DIRECTORY
    descriptor = os.open(path, flags)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def write_admission_ticket(
    directory: Path,
    record: dict[str, Any],
    opened_binary: Any,
) -> Path:
    private_directory = ensure_private_ticket_directory(directory)
    ticket = build_admission_ticket(record, opened_binary)
    path = private_directory / f"{ticket['ticket_id']}.json"
    payload = promotion.canonical_json(ticket) + b"\n"
    if not payload or len(payload) > MAX_TICKET_BYTES:
        fail("admission ticket exceeds its 64 KiB byte bound")
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
    if hasattr(os, "O_CLOEXEC"):
        flags |= os.O_CLOEXEC
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    try:
        descriptor = os.open(path, flags, 0o600)
    except OSError as exc:
        fail(f"cannot create admission ticket {path}: {exc}")
    try:
        with os.fdopen(descriptor, "wb", closefd=True) as handle:
            handle.write(payload)
            handle.flush()
            os.fchmod(handle.fileno(), 0o600)
            os.fsync(handle.fileno())
        metadata = os.lstat(path)
        if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
            fail("written admission ticket is not a regular non-symbolic-link file")
        if stat.S_IMODE(metadata.st_mode) != 0o600:
            fail("written admission ticket does not have exact mode 0600")
        fsync_directory(private_directory)
    except BaseException:
        try:
            os.unlink(path)
        except OSError:
            pass
        raise
    return path


def remove_admission_ticket(path: Path | None) -> None:
    if path is None:
        return
    try:
        path.unlink()
    except FileNotFoundError:
        return
    except OSError:
        return


def admitted_environment(
    source: dict[str, str],
    record: dict[str, Any],
    ticket_path: Path,
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
    environment["DFMCP_ADMISSION_TICKET"] = os.fspath(ticket_path)
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
    ticket_path: Path | None = None
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
        ticket_path = write_admission_ticket(
            args.launch_record.parent / TICKET_DIRECTORY_NAME,
            record,
            opened,
        )
        environment = admitted_environment(inherited, record, ticket_path)
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
        remove_admission_ticket(ticket_path)
        if opened is not None:
            try:
                os.close(opened.descriptor)
            except OSError:
                pass
        print(f"admitted live launcher: FAIL: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
