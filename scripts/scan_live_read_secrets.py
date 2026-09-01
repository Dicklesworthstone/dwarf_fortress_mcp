#!/usr/bin/env python3
"""Produce bounded R2 secret non-disclosure evidence for live-read artifacts."""

from __future__ import annotations

import argparse
import base64
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


def atomic_write_json(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    content = json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False).encode("utf-8") + b"\n"
    descriptor, temporary_name = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "wb") as handle:
            handle.write(content)
            handle.flush()
            os.fsync(handle.fileno())
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


def regular_files(root: Path, output: Path) -> list[Path]:
    if root.is_symlink() or not root.is_dir():
        fail("artifact root must be a real directory, not a symbolic link")
    output_resolved = output.resolve()
    files: list[Path] = []
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
                mode = path.lstat().st_mode
            except OSError as exc:
                fail(f"cannot inspect artifact {relative_path(root, path)}: {exc}")
            if stat.S_ISLNK(mode):
                fail(f"artifact tree contains symbolic-link file {relative_path(root, path)}")
            if not stat.S_ISREG(mode):
                fail(f"artifact tree contains non-regular file {relative_path(root, path)}")
            if path.resolve() == output_resolved:
                continue
            files.append(path)
            if len(files) > MAX_FILES:
                fail(f"artifact tree exceeds the explicit {MAX_FILES}-file bound")
    if not files:
        fail("artifact tree contains no regular files to scan")
    return files


def scan(root: Path, output: Path, token: bytes) -> tuple[dict[str, Any], list[Match]]:
    if root.is_symlink():
        fail("artifact root must not be a symbolic link")
    root = root.resolve()
    candidates = representations(token)
    artifacts: list[dict[str, Any]] = []
    matches: list[Match] = []
    total_bytes = 0
    for path in regular_files(root, output):
        relative = relative_path(root, path)
        try:
            size = path.stat().st_size
        except OSError as exc:
            fail(f"cannot stat artifact {relative}: {exc}")
        if size < 0 or size > MAX_FILE_BYTES:
            fail(f"artifact {relative} exceeds the {MAX_FILE_BYTES}-byte file bound")
        total_bytes += size
        if total_bytes > MAX_TOTAL_BYTES:
            fail(f"artifact set exceeds the {MAX_TOTAL_BYTES}-byte aggregate bound")
        try:
            data = path.read_bytes()
        except OSError as exc:
            fail(f"cannot read artifact {relative}: {exc}")
        if len(data) != size:
            fail(f"artifact {relative} changed size while being scanned")
        for representation, needle in candidates.items():
            occurrences = data.count(needle)
            if occurrences:
                matches.append(Match(relative, representation, occurrences))
        artifacts.append(
            {
                "path": relative,
                "bytes": size,
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
