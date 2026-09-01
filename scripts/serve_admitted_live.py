#!/usr/bin/env python3
"""Resolve an exact live tuple, verify one server inode, and exec read-only live MCP."""

from __future__ import annotations

import argparse
import importlib.util
import json
import os
import stat
import sys
import tempfile
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
PROMOTION_PATH = ROOT / "scripts/promote_live_compatibility.py"
RESOLVER_PATH = ROOT / "scripts/resolve_live_compatibility.py"
DEFAULT_REGISTRY = ROOT / "architecture/live_compatibility_registry_v1.json"
DEFAULT_BINARY = ROOT / "target/release/dwarf-fortress-mcp"
LAUNCH_SCHEMA = "dfmcp.admitted-live-launch/1"
MAX_PATH_BYTES = 4096

PROMOTION_SPEC = importlib.util.spec_from_file_location("promote_live_compatibility", PROMOTION_PATH)
if PROMOTION_SPEC is None or PROMOTION_SPEC.loader is None:
    raise RuntimeError("cannot load compatibility promotion contract")
promotion = importlib.util.module_from_spec(PROMOTION_SPEC)
sys.modules[PROMOTION_SPEC.name] = promotion
PROMOTION_SPEC.loader.exec_module(promotion)

RESOLVER_SPEC = importlib.util.spec_from_file_location("resolve_live_compatibility", RESOLVER_PATH)
if RESOLVER_SPEC is None or RESOLVER_SPEC.loader is None:
    raise RuntimeError("cannot load compatibility resolver")
resolver = importlib.util.module_from_spec(RESOLVER_SPEC)
sys.modules[RESOLVER_SPEC.name] = resolver
RESOLVER_SPEC.loader.exec_module(resolver)


class LaunchError(ValueError):
    pass


def fail(message: str) -> None:
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


def validate_binary_path(path: Path) -> Path:
    raw = os.fspath(path)
    if not raw or len(os.fsencode(raw)) > MAX_PATH_BYTES:
        fail("server binary path is empty or exceeds its byte bound")
    if any(ord(character) < 0x20 for character in raw):
        fail("server binary path contains a control character")
    absolute = path if path.is_absolute() else (Path.cwd() / path)
    parent = absolute.parent.resolve(strict=True)
    candidate = parent / absolute.name
    try:
        metadata = candidate.lstat()
    except OSError as exc:
        fail(f"cannot inspect server binary: {exc}")
    if stat.S_ISLNK(metadata.st_mode):
        fail("server binary must not be a symbolic link")
    if not stat.S_ISREG(metadata.st_mode):
        fail("server binary must be a regular file")
    if metadata.st_mode & (stat.S_IWGRP | stat.S_IWOTH):
        fail("server binary must not be group- or world-writable")
    if hasattr(os, "getuid") and metadata.st_uid != os.getuid():
        fail("server binary must be owned by the launching user")
    if not os.access(candidate, os.X_OK):
        fail("server binary is not executable")
    return candidate


def open_verified_binary(path: Path, expected_sha256: str) -> tuple[int, str]:
    promotion.require_hash(expected_sha256, "server_sha256")
    admitted_path = validate_binary_path(path)
    flags = os.O_RDONLY
    if hasattr(os, "O_CLOEXEC"):
        flags |= os.O_CLOEXEC
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    try:
        descriptor = os.open(admitted_path, flags)
    except OSError as exc:
        fail(f"cannot open server binary without following links: {exc}")
    try:
        metadata = os.fstat(descriptor)
        if not stat.S_ISREG(metadata.st_mode):
            fail("opened server binary inode is not a regular file")
        if metadata.st_mode & (stat.S_IWGRP | stat.S_IWOTH):
            fail("opened server binary inode is group- or world-writable")
        if hasattr(os, "getuid") and metadata.st_uid != os.getuid():
            fail("opened server binary inode is not owned by the launching user")
        with os.fdopen(os.dup(descriptor), "rb") as handle:
            digest = promotion.hashlib.sha256()
            for chunk in iter(lambda: handle.read(1024 * 1024), b""):
                digest.update(chunk)
            actual = digest.hexdigest()
        if actual != expected_sha256:
            fail("server binary SHA-256 does not match the admitted launch argument")
        return descriptor, actual
    except BaseException:
        os.close(descriptor)
        raise


def build_launch_record(
    compatibility_decision: dict[str, Any],
    binary_path: Path,
    binary_sha256: str,
) -> dict[str, Any]:
    if compatibility_decision.get("admitted") is not True:
        fail("compatibility decision is not admitted")
    unsigned: dict[str, Any] = {
        "schema": LAUNCH_SCHEMA,
        "compatibility_entry_id": compatibility_decision["entry_id"],
        "compatibility_decision_digest": compatibility_decision["decision_digest"],
        "compatibility_registry_digest": compatibility_decision["registry_digest"],
        "support_level": compatibility_decision["support_level"],
        "server_binary": os.fspath(binary_path),
        "server_binary_sha256": binary_sha256,
        "mode": "authenticated_live_read_only",
        "mutation_capabilities": [],
    }
    return {
        **unsigned,
        "launch_digest": promotion.sha256_bytes(promotion.canonical_json(unsigned)),
    }


def write_atomic(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    payload = json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False) + "\n"
    descriptor, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8", newline="\n") as handle:
            handle.write(payload)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
    except BaseException:
        try:
            os.unlink(temporary)
        except OSError:
            pass
        raise


def prepare_launch(
    registry_path: Path,
    manifest_path: Path,
    binary_path: Path,
    server_sha256: str,
    required_entry_id: str | None,
    environment: dict[str, str],
) -> tuple[int, Path, dict[str, Any]]:
    validate_token_environment(environment)
    registry = promotion.read_object(registry_path, 8 * 1024 * 1024, "compatibility registry")
    manifest = promotion.read_object(manifest_path, 1024 * 1024, "deployment manifest")
    decision = resolver.resolve(registry, manifest, required_entry_id)
    if decision["admitted"] is not True:
        fail("deployment manifest has no exact admitted compatibility entry")
    descriptor, actual_sha256 = open_verified_binary(binary_path, server_sha256)
    admitted_path = validate_binary_path(binary_path)
    record = build_launch_record(decision, admitted_path, actual_sha256)
    return descriptor, admitted_path, record


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("--registry", type=Path, default=DEFAULT_REGISTRY)
    parser.add_argument("--binary", type=Path, default=DEFAULT_BINARY)
    parser.add_argument("--server-sha256", required=True)
    parser.add_argument("--require-entry-id")
    parser.add_argument("--launch-record", type=Path, required=True)
    parser.add_argument("--dry-run", action="store_true")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    descriptor = -1
    try:
        descriptor, binary_path, record = prepare_launch(
            args.registry,
            args.manifest,
            args.binary,
            args.server_sha256,
            args.require_entry_id,
            dict(os.environ),
        )
        write_atomic(args.launch_record, record)
        if args.dry_run:
            os.close(descriptor)
            print(json.dumps(record, sort_keys=True, separators=(",", ":"), ensure_ascii=False))
            return 0
        if not hasattr(os, "fexecve"):
            fail("this platform lacks fexecve; refusing path-based live execution")
        environment = dict(os.environ)
        environment["DFMCP_COMPATIBILITY_ENTRY_ID"] = record["compatibility_entry_id"]
        environment["DFMCP_COMPATIBILITY_DECISION_DIGEST"] = record[
            "compatibility_decision_digest"
        ]
        environment["DFMCP_ADMITTED_LAUNCH_DIGEST"] = record["launch_digest"]
        arguments = [os.fspath(binary_path), "serve-live"]
        os.fexecve(descriptor, arguments, environment)
        fail("fexecve returned unexpectedly")
    except (OSError, promotion.PromotionError, resolver.ResolutionError, LaunchError) as exc:
        if descriptor >= 0:
            try:
                os.close(descriptor)
            except OSError:
                pass
        print(f"admitted live launcher: FAIL: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
