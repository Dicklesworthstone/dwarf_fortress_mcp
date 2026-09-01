#!/usr/bin/env python3
"""Reject source corruption, symlinks, local placeholders, and recovery debris."""

from __future__ import annotations

import argparse
import os
import re
import stat
import sys
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
IGNORED_DIRECTORIES = {".git", ".venv", "node_modules", "target", "__pycache__"}
FORBIDDEN_NAMES = {"audit-upload-test.rs"}
FORBIDDEN_PREFIXES = (".tool_probe",)
FORBIDDEN_SUFFIXES = (".restore-pointer",)
MAX_TEXT_BYTES = 16 * 1024 * 1024
TEXT_SUFFIXES = {
    ".c",
    ".cc",
    ".cfg",
    ".cpp",
    ".css",
    ".csv",
    ".h",
    ".hpp",
    ".html",
    ".ini",
    ".js",
    ".json",
    ".jsonl",
    ".lock",
    ".md",
    ".proto",
    ".ps1",
    ".py",
    ".rs",
    ".sh",
    ".sql",
    ".toml",
    ".ts",
    ".tsv",
    ".tsx",
    ".txt",
    ".xml",
    ".yaml",
    ".yml",
}
TEXT_FILENAMES = {
    ".gitattributes",
    ".gitignore",
    "CODEOWNERS",
    "LICENSE",
    "Makefile",
}
ABSOLUTE_PATH = re.compile(
    r"^(?:/mnt/data/|/tmp/|/private/tmp/|/home/[^/]+/|/Users/[^/]+/|[A-Za-z]:\\).+"
)
RECOVERY_MARKER = re.compile(r"^Recovery plumbing file for\s+.+\.$")


@dataclass(frozen=True)
class Failure:
    path: str
    reason: str


def expected_text_file(path: Path) -> bool:
    return path.name in TEXT_FILENAMES or path.suffix.lower() in TEXT_SUFFIXES


def inspect(root: Path) -> list[Failure]:
    failures: list[Failure] = []
    for directory, names, files in os.walk(root, followlinks=False):
        directory_path = Path(directory)
        retained_directories: list[str] = []
        for name in sorted(names):
            if name in IGNORED_DIRECTORIES:
                continue
            path = directory_path / name
            relative = path.relative_to(root).as_posix()
            try:
                metadata = path.lstat()
            except OSError as exc:
                failures.append(Failure(relative, f"cannot inspect directory entry: {exc}"))
                continue
            if stat.S_ISLNK(metadata.st_mode):
                failures.append(Failure(relative, "repository directory is a symbolic link"))
                continue
            if not stat.S_ISDIR(metadata.st_mode):
                failures.append(Failure(relative, "repository directory entry is not a directory"))
                continue
            retained_directories.append(name)
        names[:] = retained_directories

        for name in sorted(files):
            path = directory_path / name
            relative = path.relative_to(root).as_posix()
            if (
                name in FORBIDDEN_NAMES
                or name.startswith(FORBIDDEN_PREFIXES)
                or name.endswith(FORBIDDEN_SUFFIXES)
            ):
                failures.append(
                    Failure(relative, "forbidden probe or recovery-plumbing filename")
                )
                continue
            try:
                metadata = path.lstat()
            except OSError as exc:
                failures.append(Failure(relative, f"cannot inspect file: {exc}"))
                continue
            if stat.S_ISLNK(metadata.st_mode):
                failures.append(Failure(relative, "repository file is a symbolic link"))
                continue
            if not stat.S_ISREG(metadata.st_mode):
                failures.append(Failure(relative, "repository file is not a regular file"))
                continue
            size = metadata.st_size
            text_expected = expected_text_file(path)
            if text_expected and size > MAX_TEXT_BYTES:
                failures.append(
                    Failure(
                        relative,
                        f"source or contract text exceeds the {MAX_TEXT_BYTES}-byte integrity bound",
                    )
                )
                continue
            if size == 0 or (not text_expected and size > MAX_TEXT_BYTES):
                continue
            try:
                raw = path.read_bytes()
            except OSError as exc:
                failures.append(Failure(relative, f"cannot read file: {exc}"))
                continue
            try:
                after = path.lstat()
            except OSError as exc:
                failures.append(Failure(relative, f"cannot reinspect file: {exc}"))
                continue
            if (
                after.st_dev != metadata.st_dev
                or after.st_ino != metadata.st_ino
                or after.st_size != metadata.st_size
                or after.st_mode != metadata.st_mode
                or after.st_mtime_ns != metadata.st_mtime_ns
                or after.st_ctime_ns != metadata.st_ctime_ns
            ):
                failures.append(Failure(relative, "repository file changed while being inspected"))
                continue
            if text_expected and b"\x00" in raw:
                failures.append(
                    Failure(relative, "source or contract text contains a NUL byte")
                )
                continue
            if not text_expected and b"\x00" in raw:
                continue
            try:
                text = raw.decode("utf-8")
            except UnicodeDecodeError as exc:
                if text_expected:
                    failures.append(
                        Failure(
                            relative,
                            f"source or contract text is not valid UTF-8: {exc}",
                        )
                    )
                continue
            stripped = text.strip()
            if not stripped or "\n" in stripped or "\r" in stripped:
                continue
            if ABSOLUTE_PATH.fullmatch(stripped):
                failures.append(
                    Failure(relative, "file contains only a machine-local absolute path")
                )
            elif RECOVERY_MARKER.fullmatch(stripped):
                failures.append(
                    Failure(relative, "file contains only a recovery-plumbing marker")
                )
            elif stripped in {"probe", "probe2", "test", "placeholder"}:
                failures.append(
                    Failure(relative, "file contains only a connector/test placeholder")
                )
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
    print("repository integrity: PASS (regular source text is valid and no probe debris exists)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
