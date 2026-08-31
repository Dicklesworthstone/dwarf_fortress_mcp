#!/usr/bin/env python3
"""Reject local-path placeholders and connector/recovery debris in the source tree."""

from __future__ import annotations

import argparse
import os
import re
import sys
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
IGNORED_DIRECTORIES = {".git", ".venv", "node_modules", "target", "__pycache__"}
FORBIDDEN_NAMES = {"audit-upload-test.rs"}
FORBIDDEN_PREFIXES = (".tool_probe",)
FORBIDDEN_SUFFIXES = (".restore-pointer",)
MAX_INSPECT_BYTES = 1024 * 1024
ABSOLUTE_PATH = re.compile(
    r"^(?:/mnt/data/|/tmp/|/private/tmp/|/home/[^/]+/|/Users/[^/]+/|[A-Za-z]:\\).+"
)
RECOVERY_MARKER = re.compile(r"^Recovery plumbing file for\s+.+\.$")


@dataclass(frozen=True)
class Failure:
    path: str
    reason: str


def inspect(root: Path) -> list[Failure]:
    failures: list[Failure] = []
    for directory, names, files in os.walk(root):
        names[:] = sorted(name for name in names if name not in IGNORED_DIRECTORIES)
        for name in sorted(files):
            path = Path(directory) / name
            relative = path.relative_to(root).as_posix()
            if name in FORBIDDEN_NAMES or name.startswith(FORBIDDEN_PREFIXES) or name.endswith(FORBIDDEN_SUFFIXES):
                failures.append(Failure(relative, "forbidden probe or recovery-plumbing filename"))
                continue
            try:
                size = path.stat().st_size
            except OSError as exc:
                failures.append(Failure(relative, f"cannot stat file: {exc}"))
                continue
            if size == 0 or size > MAX_INSPECT_BYTES:
                continue
            try:
                raw = path.read_bytes()
            except OSError as exc:
                failures.append(Failure(relative, f"cannot read file: {exc}"))
                continue
            if b"\x00" in raw:
                continue
            try:
                text = raw.decode("utf-8")
            except UnicodeDecodeError:
                continue
            stripped = text.strip()
            if not stripped or "\n" in stripped or "\r" in stripped:
                continue
            if ABSOLUTE_PATH.fullmatch(stripped):
                failures.append(Failure(relative, "file contains only a machine-local absolute path"))
            elif RECOVERY_MARKER.fullmatch(stripped):
                failures.append(Failure(relative, "file contains only a recovery-plumbing marker"))
            elif stripped in {"probe", "probe2", "test", "placeholder"}:
                failures.append(Failure(relative, "file contains only a connector/test placeholder"))
    return failures


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=ROOT)
    args = parser.parse_args(argv)
    root = args.root.resolve()
    if not root.is_dir():
        print(f"repository integrity: FAIL: root is not a directory: {root}", file=sys.stderr)
        return 1
    failures = inspect(root)
    if failures:
        print(f"repository integrity: FAIL ({len(failures)} violation(s))", file=sys.stderr)
        for failure in failures:
            print(f"  {failure.path}: {failure.reason}", file=sys.stderr)
        return 1
    print("repository integrity: PASS (no local-path placeholders or probe debris)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
