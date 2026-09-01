#!/usr/bin/env python3
"""Maintain one owner-private monotonic floor for live compatibility registry generations."""

from __future__ import annotations

import argparse
import contextlib
import importlib.util
import json
import os
import stat
import sys
import tempfile
from pathlib import Path
from typing import Any, Iterator, NoReturn

ROOT = Path(__file__).resolve().parents[1]
PROMOTION_PATH = ROOT / "scripts/promote_live_compatibility.py"
DEFAULT_REGISTRY = ROOT / "architecture/live_compatibility_registry_v1.json"
FLOOR_SCHEMA = "dfmcp.live-compatibility-floor/1"
MAX_FLOOR_BYTES = 1024 * 1024

SPEC = importlib.util.spec_from_file_location("promote_live_compatibility", PROMOTION_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load compatibility registry contract")
promotion = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = promotion
SPEC.loader.exec_module(promotion)


class FloorError(ValueError):
    pass


def fail(message: str) -> NoReturn:
    raise FloorError(message)


def require_nonnegative_int(value: Any, path: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        fail(f"{path} must be a nonnegative integer")
    return value


def require_absolute_path(path: Path, label: str) -> Path:
    if not path.is_absolute():
        fail(f"{label} path must be absolute")
    raw = os.fspath(path)
    if not raw or len(os.fsencode(raw)) > 4096:
        fail(f"{label} path is empty or exceeds its byte bound")
    return path


def permitted_owners() -> set[int]:
    if not hasattr(os, "geteuid"):
        fail("compatibility floor custody currently requires Unix owner metadata")
    return {0, os.geteuid()}


def validate_private_directory(path: Path) -> os.stat_result:
    try:
        metadata = os.lstat(path)
    except OSError as exc:
        fail(f"cannot inspect compatibility floor directory {path}: {exc}")
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISDIR(metadata.st_mode):
        fail("compatibility floor parent must be a real directory, not a symbolic link")
    if stat.S_IMODE(metadata.st_mode) != 0o700:
        fail("compatibility floor parent must have exact owner-only mode 0700")
    if metadata.st_uid not in permitted_owners():
        fail("compatibility floor parent is not owned by root or the effective user")
    return metadata


def validate_floor_path(path: Path) -> Path:
    absolute = require_absolute_path(path, "compatibility floor")
    validate_private_directory(absolute.parent)
    return absolute


def same_identity(left: os.stat_result, right: os.stat_result) -> bool:
    return (
        left.st_dev == right.st_dev
        and left.st_ino == right.st_ino
        and left.st_size == right.st_size
        and left.st_mode == right.st_mode
        and left.st_uid == right.st_uid
        and left.st_mtime_ns == right.st_mtime_ns
        and left.st_ctime_ns == right.st_ctime_ns
    )


def read_private_bytes(path: Path) -> tuple[bytes, str]:
    absolute = validate_floor_path(path)
    try:
        before = os.lstat(absolute)
    except OSError as exc:
        fail(f"cannot inspect compatibility floor {absolute}: {exc}")
    if stat.S_ISLNK(before.st_mode) or not stat.S_ISREG(before.st_mode):
        fail("compatibility floor must be a regular non-symbolic-link file")
    if stat.S_IMODE(before.st_mode) != 0o600:
        fail("compatibility floor must have exact owner-read/write mode 0600")
    if before.st_uid not in permitted_owners():
        fail("compatibility floor is not owned by root or the effective user")
    if before.st_size <= 0 or before.st_size > MAX_FLOOR_BYTES:
        fail(f"compatibility floor must contain 1..={MAX_FLOOR_BYTES} bytes")
    if not hasattr(os, "O_NOFOLLOW"):
        fail("this platform cannot enforce no-follow compatibility floor opening")
    flags = os.O_RDONLY | os.O_NOFOLLOW
    if hasattr(os, "O_CLOEXEC"):
        flags |= os.O_CLOEXEC
    try:
        descriptor = os.open(absolute, flags)
    except OSError as exc:
        fail(f"cannot open compatibility floor {absolute}: {exc}")
    try:
        opened = os.fstat(descriptor)
        if not same_identity(before, opened):
            fail("compatibility floor changed between path inspection and open")
        digest = promotion.hashlib.sha256()
        chunks: list[bytes] = []
        total = 0
        while True:
            chunk = os.read(descriptor, min(1024 * 1024, MAX_FLOOR_BYTES + 1 - total))
            if not chunk:
                break
            total += len(chunk)
            if total > MAX_FLOOR_BYTES:
                fail("compatibility floor grew beyond its byte bound while being read")
            digest.update(chunk)
            chunks.append(chunk)
        after = os.fstat(descriptor)
        if not same_identity(opened, after) or total != after.st_size:
            fail("compatibility floor changed while being read")
        validate_private_directory(absolute.parent)
        return b"".join(chunks), digest.hexdigest()
    finally:
        os.close(descriptor)


def validate_floor(value: dict[str, Any]) -> dict[str, Any]:
    promotion.require_exact_keys(
        value,
        {
            "schema",
            "sequence",
            "registry_file_sha256",
            "registry_digest",
            "entry_ids",
            "previous_floor_digest",
            "floor_digest",
        },
        "compatibility_floor",
    )
    if value.get("schema") != FLOOR_SCHEMA:
        fail("compatibility floor schema is unsupported")
    sequence = require_nonnegative_int(value.get("sequence"), "compatibility_floor.sequence")
    registry_file_sha256 = promotion.require_hash(
        value.get("registry_file_sha256"), "compatibility_floor.registry_file_sha256"
    )
    registry_digest = promotion.require_hash(
        value.get("registry_digest"), "compatibility_floor.registry_digest"
    )
    raw_entry_ids = promotion.require_list(value.get("entry_ids"), "compatibility_floor.entry_ids")
    entry_ids: list[str] = []
    previous = ""
    for index, raw in enumerate(raw_entry_ids):
        entry_id = promotion.require_hash(raw, f"compatibility_floor.entry_ids[{index}]")
        if previous and entry_id <= previous:
            fail("compatibility floor entry IDs are not in strict canonical order")
        entry_ids.append(entry_id)
        previous = entry_id
    previous_floor_digest = value.get("previous_floor_digest")
    if sequence == 0:
        if previous_floor_digest is not None:
            fail("initial compatibility floor must not name a previous floor digest")
    else:
        previous_floor_digest = promotion.require_hash(
            previous_floor_digest, "compatibility_floor.previous_floor_digest"
        )
    declared = promotion.require_hash(value.get("floor_digest"), "compatibility_floor.floor_digest")
    unsigned = dict(value)
    del unsigned["floor_digest"]
    expected = promotion.sha256_bytes(promotion.canonical_json(unsigned))
    if declared != expected:
        fail("compatibility floor digest does not reproduce its canonical fields")
    return {
        "schema": FLOOR_SCHEMA,
        "sequence": sequence,
        "registry_file_sha256": registry_file_sha256,
        "registry_digest": registry_digest,
        "entry_ids": entry_ids,
        "previous_floor_digest": previous_floor_digest,
        "floor_digest": declared,
    }


def read_floor(path: Path) -> tuple[dict[str, Any], str]:
    raw, file_sha256 = read_private_bytes(path)
    try:
        value = json.loads(
            raw.decode("utf-8"), object_pairs_hook=promotion.duplicate_rejecting_object
        )
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        fail(f"cannot parse compatibility floor: {exc}")
    if not isinstance(value, dict):
        fail("compatibility floor must be a JSON object")
    promotion.bounded_tree(value)
    return validate_floor(value), file_sha256


def registry_generation(registry_path: Path) -> dict[str, Any]:
    registry, file_sha256 = promotion.read_object_with_digest(
        registry_path, promotion.MAX_JSON_BYTES, "compatibility registry"
    )
    entries = promotion.validate_registry(registry)
    return {
        "registry_file_sha256": file_sha256,
        "registry_digest": promotion.sha256_bytes(promotion.canonical_json(registry)),
        "entry_ids": [entry["entry_id"] for entry in entries],
    }


def build_floor(
    generation: dict[str, Any],
    sequence: int,
    previous_floor_digest: str | None,
) -> dict[str, Any]:
    unsigned: dict[str, Any] = {
        "schema": FLOOR_SCHEMA,
        "sequence": sequence,
        "registry_file_sha256": generation["registry_file_sha256"],
        "registry_digest": generation["registry_digest"],
        "entry_ids": list(generation["entry_ids"]),
        "previous_floor_digest": previous_floor_digest,
    }
    return {
        **unsigned,
        "floor_digest": promotion.sha256_bytes(promotion.canonical_json(unsigned)),
    }


def generation_matches(floor: dict[str, Any], generation: dict[str, Any]) -> bool:
    return (
        floor["registry_file_sha256"] == generation["registry_file_sha256"]
        and floor["registry_digest"] == generation["registry_digest"]
        and floor["entry_ids"] == generation["entry_ids"]
    )


def verify_floor(
    floor_path: Path,
    registry_path: Path | None = None,
) -> tuple[dict[str, Any], str]:
    floor, floor_file_sha256 = read_floor(floor_path)
    if registry_path is not None:
        generation = registry_generation(registry_path)
        if not generation_matches(floor, generation):
            fail("compatibility registry generation does not match the trusted monotonic floor")
    return floor, floor_file_sha256


def fsync_directory(path: Path) -> None:
    flags = os.O_RDONLY
    if hasattr(os, "O_DIRECTORY"):
        flags |= os.O_DIRECTORY
    descriptor = os.open(path, flags)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def payload_bytes(value: dict[str, Any]) -> bytes:
    payload = json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False).encode("utf-8") + b"\n"
    if not payload or len(payload) > MAX_FLOOR_BYTES:
        fail("compatibility floor payload exceeds its byte bound")
    return payload


def write_private_exclusive(path: Path, value: dict[str, Any]) -> None:
    absolute = validate_floor_path(path)
    payload = payload_bytes(value)
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
    if hasattr(os, "O_CLOEXEC"):
        flags |= os.O_CLOEXEC
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    try:
        descriptor = os.open(absolute, flags, 0o600)
    except OSError as exc:
        fail(f"cannot initialize compatibility floor {absolute}: {exc}")
    try:
        with os.fdopen(descriptor, "wb", closefd=True) as handle:
            handle.write(payload)
            handle.flush()
            os.fchmod(handle.fileno(), 0o600)
            os.fsync(handle.fileno())
        fsync_directory(absolute.parent)
        read_floor(absolute)
    except BaseException:
        try:
            os.unlink(absolute)
        except OSError:
            pass
        raise


def write_private_atomic(path: Path, value: dict[str, Any]) -> None:
    absolute = validate_floor_path(path)
    payload = payload_bytes(value)
    descriptor, temporary = tempfile.mkstemp(prefix=f".{absolute.name}.", dir=absolute.parent)
    try:
        with os.fdopen(descriptor, "wb", closefd=True) as handle:
            os.fchmod(handle.fileno(), 0o600)
            handle.write(payload)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, absolute)
        fsync_directory(absolute.parent)
        read_floor(absolute)
    except BaseException:
        try:
            os.unlink(temporary)
        except OSError:
            pass
        raise


@contextlib.contextmanager
def floor_lock(path: Path) -> Iterator[None]:
    absolute = validate_floor_path(path)
    lock_path = absolute.with_name(f".{absolute.name}.lock")
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
    if hasattr(os, "O_CLOEXEC"):
        flags |= os.O_CLOEXEC
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    try:
        descriptor = os.open(lock_path, flags, 0o600)
    except FileExistsError:
        fail(
            f"compatibility floor lock already exists: {lock_path}; inspect it only after proving no floor operation is active"
        )
    except OSError as exc:
        fail(f"cannot acquire compatibility floor lock: {exc}")
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8", newline="\n") as handle:
            handle.write(f"pid={os.getpid()}\n")
            handle.flush()
            os.fsync(handle.fileno())
        fsync_directory(lock_path.parent)
        yield
    finally:
        try:
            lock_path.unlink()
            fsync_directory(lock_path.parent)
        except OSError:
            pass


def initialize_floor(floor_path: Path, registry_path: Path) -> dict[str, Any]:
    generation = registry_generation(registry_path)
    floor = build_floor(generation, 0, None)
    with floor_lock(floor_path):
        write_private_exclusive(floor_path, floor)
    return floor


def advance_floor(
    floor_path: Path,
    registry_path: Path,
    expected_floor_file_sha256: str,
) -> tuple[dict[str, Any], bool]:
    expected = promotion.require_hash(
        expected_floor_file_sha256, "expected_floor_file_sha256"
    )
    with floor_lock(floor_path):
        current, actual_file_sha256 = read_floor(floor_path)
        if actual_file_sha256 != expected:
            fail("compatibility floor changed since the caller selected its expected generation")
        generation = registry_generation(registry_path)
        missing = sorted(set(current["entry_ids"]) - set(generation["entry_ids"]))
        if missing:
            fail(
                "candidate compatibility registry rolls back prior admitted entry IDs: "
                + ", ".join(missing)
            )
        if generation_matches(current, generation):
            return current, False
        sequence = current["sequence"] + 1
        candidate = build_floor(generation, sequence, current["floor_digest"])
        write_private_atomic(floor_path, candidate)
        return candidate, True


def emit(value: dict[str, Any]) -> None:
    print(json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False))


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)

    initialize = subparsers.add_parser("init")
    initialize.add_argument("--floor", type=Path, required=True)
    initialize.add_argument("--registry", type=Path, default=DEFAULT_REGISTRY)

    verify = subparsers.add_parser("verify")
    verify.add_argument("--floor", type=Path, required=True)
    verify.add_argument("--registry", type=Path)

    advance = subparsers.add_parser("advance")
    advance.add_argument("--floor", type=Path, required=True)
    advance.add_argument("--registry", type=Path, default=DEFAULT_REGISTRY)
    advance.add_argument("--expected-floor-sha256", required=True)

    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    try:
        if args.command == "init":
            floor = initialize_floor(args.floor, args.registry)
            emit({"status": "initialized", "floor": floor})
        elif args.command == "verify":
            floor, file_sha256 = verify_floor(args.floor, args.registry)
            emit(
                {
                    "status": "verified",
                    "floor_file_sha256": file_sha256,
                    "floor": floor,
                }
            )
        elif args.command == "advance":
            floor, changed = advance_floor(
                args.floor, args.registry, args.expected_floor_sha256
            )
            emit(
                {
                    "status": "advanced" if changed else "unchanged",
                    "floor": floor,
                }
            )
        else:
            fail("unsupported compatibility floor command")
    except (FloorError, promotion.PromotionError, OSError) as exc:
        print(f"live compatibility floor: FAIL: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
