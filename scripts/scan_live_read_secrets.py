#!/usr/bin/env python3
"""Produce bounded R2 secret non-disclosure evidence for live-read artifacts."""

from __future__ import annotations

import argparse
import base64
import errno
import hashlib
import json
import os
import stat
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Any

EVENT_SCHEMA = "dfmcp.live-read-acceptance-event/1"
SCANNER_ID = "dfmcp-secret-scan/1"
MIN_TOKEN_BYTES = 32
MAX_TOKEN_BYTES = 256
MAX_FILES = 512
MAX_FILE_BYTES = 32 * 1024 * 1024
MAX_TOTAL_BYTES = 256 * 1024 * 1024
MAX_PATH_BYTES = 1_024
READ_CHUNK_BYTES = 1024 * 1024


class ScanError(ValueError):
    pass


@dataclass(frozen=True)
class Match:
    path: str
    representation: str
    occurrences: int


def fail(message: str) -> None:
    raise ScanError(message)


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def ensure_real_output(path: Path) -> None:
    if path.is_symlink():
        fail("secret-scan output must not be a symbolic link")
    parent = path.parent
    if parent.is_symlink() or not parent.is_dir():
        fail("secret-scan output parent must be a real directory")
    if path.exists():
        try:
            mode = path.lstat().st_mode
        except OSError as exc:
            fail(f"cannot inspect existing secret-scan output: {exc}")
        if not stat.S_ISREG(mode):
            fail("existing secret-scan output must be a regular file")


def atomic_write_json(path: Path, value: dict[str, Any]) -> None:
    ensure_real_output(path)
    content = json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False).encode("utf-8") + b"\n"
    descriptor, temporary_name = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "wb") as handle:
            handle.write(content)
            handle.flush()
            os.fsync(handle.fileno())
        ensure_real_output(path)
        os.replace(temporary, path)
    except BaseException:
        try:
            temporary.unlink()
        except OSError:
            pass
        raise


def validate_token(token: bytes) -> bytes:
    if len(token) < MIN_TOKEN_BYTES or len(token) > MAX_TOKEN_BYTES:
        fail(f"token must contain {MIN_TOKEN_BYTES}..={MAX_TOKEN_BYTES} bytes")
    return token


def token_from_environment(name: str) -> bytes:
    try:
        value = os.environ[name]
    except KeyError:
        fail(f"{name} is required for secret scanning")
    return validate_token(value.encode("utf-8"))


def representations(token: bytes) -> dict[str, bytes]:
    token = validate_token(token)
    candidates = {
        "raw": token,
        "hex_lower": token.hex().encode("ascii"),
        "hex_upper": token.hex().upper().encode("ascii"),
        "base64": base64.b64encode(token),
        "base64_urlsafe": base64.urlsafe_b64encode(token),
        "environment_assignment": b"DFMCP_BRIDGE_TOKEN=" + token,
    }
    output: dict[str, bytes] = {}
    seen: set[bytes] = set()
    for name, candidate in candidates.items():
        if candidate and candidate not in seen:
            seen.add(candidate)
            output[name] = candidate
    return output


def relative_path(root: Path, path: Path) -> str:
    relative = path.relative_to(root).as_posix()
    if not relative or relative.startswith("/") or ".." in Path(relative).parts:
        fail("artifact path is not a safe relative path")
    if len(relative.encode("utf-8")) > MAX_PATH_BYTES:
        fail(f"artifact path exceeds {MAX_PATH_BYTES} UTF-8 bytes")
    return relative


def identity(value: os.stat_result) -> tuple[int, int, int, int]:
    return (value.st_dev, value.st_ino, value.st_size, value.st_mtime_ns)


def read_stable_regular_file(path: Path, expected: os.stat_result, label: str) -> bytes:
    flags = os.O_RDONLY
    flags |= getattr(os, "O_CLOEXEC", 0)
    flags |= getattr(os, "O_NOFOLLOW", 0)
    try:
        descriptor = os.open(path, flags)
    except OSError as exc:
        if exc.errno == errno.ELOOP:
            fail(f"artifact {label} became a symbolic link before it was opened")
        fail(f"cannot open artifact {label}: {exc}")
    try:
        opened_before = os.fstat(descriptor)
        if not stat.S_ISREG(opened_before.st_mode):
            fail(f"artifact {label} is not a regular file after opening")
        if identity(opened_before) != identity(expected):
            fail(f"artifact {label} changed identity before it was opened")
        if opened_before.st_size > MAX_FILE_BYTES:
            fail(f"artifact {label} exceeds the {MAX_FILE_BYTES}-byte file bound")
        chunks: list[bytes] = []
        remaining = opened_before.st_size
        while remaining:
            chunk = os.read(descriptor, min(remaining, READ_CHUNK_BYTES))
            if not chunk:
                fail(f"artifact {label} was truncated while being read")
            chunks.append(chunk)
            remaining -= len(chunk)
        if os.read(descriptor, 1):
            fail(f"artifact {label} grew while being read")
        opened_after = os.fstat(descriptor)
    finally:
        os.close(descriptor)
    try:
        path_after = path.lstat()
    except OSError as exc:
        fail(f"artifact {label} disappeared after it was read: {exc}")
    if not stat.S_ISREG(path_after.st_mode):
        fail(f"artifact {label} was redirected after it was read")
    if identity(opened_before) != identity(opened_after) or identity(opened_after) != identity(path_after):
        fail(f"artifact {label} changed while it was being scanned")
    data = b"".join(chunks)
    if len(data) != opened_after.st_size:
        fail(f"artifact {label} byte count disagrees with its stable file identity")
    return data


def regular_files(root: Path, output: Path) -> list[tuple[Path, os.stat_result]]:
    if root.is_symlink() or not root.is_dir():
        fail("artifact root must be a real directory, not a symbolic link")
    output_resolved = output.resolve()
    files: list[tuple[Path, os.stat_result]] = []
    for directory, names, filenames in os.walk(root, followlinks=False):
        names.sort()
        filenames.sort()
        directory_path = Path(directory)
        for name in names:
            child = directory_path / name
            if child.is_symlink():
                fail(f"artifact tree contains symbolic-link directory {relative_path(root, child)}")
        for name in filenames:
            path = directory_path / name
            try:
                metadata = path.lstat()
            except OSError as exc:
                fail(f"cannot inspect artifact {relative_path(root, path)}: {exc}")
            if stat.S_ISLNK(metadata.st_mode):
                fail(f"artifact tree contains symbolic-link file {relative_path(root, path)}")
            if not stat.S_ISREG(metadata.st_mode):
                fail(f"artifact tree contains non-regular file {relative_path(root, path)}")
            if path.resolve() == output_resolved:
                continue
            if metadata.st_size < 0 or metadata.st_size > MAX_FILE_BYTES:
                fail(
                    f"artifact {relative_path(root, path)} exceeds the {MAX_FILE_BYTES}-byte file bound"
                )
            files.append((path, metadata))
            if len(files) > MAX_FILES:
                fail(f"artifact tree exceeds the explicit {MAX_FILES}-file bound")
    if not files:
        fail("artifact tree contains no regular files to scan")
    return files


def scan(root: Path, output: Path, token: bytes) -> tuple[dict[str, Any], list[Match]]:
    if root.is_symlink():
        fail("artifact root must not be a symbolic link")
    root = root.resolve()
    ensure_real_output(output)
    candidates = representations(token)
    artifacts: list[dict[str, Any]] = []
    matches: list[Match] = []
    total_bytes = 0
    for path, metadata in regular_files(root, output):
        relative = relative_path(root, path)
        total_bytes += metadata.st_size
        if total_bytes > MAX_TOTAL_BYTES:
            fail(f"artifact set exceeds the {MAX_TOTAL_BYTES}-byte aggregate bound")
        data = read_stable_regular_file(path, metadata, relative)
        for representation, needle in candidates.items():
            occurrences = data.count(needle)
            if occurrences:
                matches.append(Match(relative, representation, occurrences))
        artifacts.append(
            {
                "path": relative,
                "bytes": len(data),
                "sha256": sha256_bytes(data),
            }
        )
    event = {
        "schema": EVENT_SCHEMA,
        "event_id": "r2.secret_scan",
        "gate": "R2",
        "case": "secret_scan",
        "result": "passed",
        "error_code": None,
        "scanner": SCANNER_ID,
        "token_fingerprint_sha256": sha256_bytes(validate_token(token)),
        "match_count": sum(match.occurrences for match in matches),
        "representation_count": len(candidates),
        "scanned_file_count": len(artifacts),
        "scanned_byte_count": total_bytes,
        "scanned_artifacts": artifacts,
    }
    return event, matches


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("artifact_root", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--token-environment", default="DFMCP_BRIDGE_TOKEN")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    try:
        token = token_from_environment(args.token_environment)
        event, matches = scan(args.artifact_root, args.output, token)
        if matches:
            print(
                f"live secret scan: FAIL ({event['match_count']} occurrence(s) across {len(matches)} representation/file pair(s))",
                file=sys.stderr,
            )
            for match in matches:
                print(
                    f"  {match.path}: {match.representation} ({match.occurrences} occurrence(s))",
                    file=sys.stderr,
                )
            return 1
        atomic_write_json(args.output, event)
    except (ScanError, OSError) as exc:
        print(f"live secret scan: FAIL: {exc}", file=sys.stderr)
        return 1
    print(
        f"live secret scan: PASS ({event['scanned_file_count']} files, {event['scanned_byte_count']} bytes, zero matches)"
    )
    print(args.output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
