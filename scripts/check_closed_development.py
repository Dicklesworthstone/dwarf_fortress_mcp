#!/usr/bin/env python3
"""Enforce the repository's closed-development custody model.

The source is public and may be forked under the license, but this upstream
repository does not accept pull requests, unsolicited patches, contributor
onboarding, or requests for commit access. This check prevents generic
community-project scaffolding from silently reappearing.
"""

from __future__ import annotations

import pathlib
import re
import sys
from dataclasses import dataclass

ROOT = pathlib.Path(__file__).resolve().parents[1]

FORBIDDEN_PATHS = [
    "CODE_OF_CONDUCT.md",
    "CONTRIBUTING.md",
    ".github/PULL_REQUEST_TEMPLATE.md",
    ".github/pull_request_template.md",
    ".github/PULL_REQUEST_TEMPLATE",
]

PUBLIC_ENTRYPOINTS = [
    "README.md",
    "AGENTS.md",
    "SECURITY.md",
    "docs/DEVELOPMENT_POLICY.md",
]

INVITATION_PATTERNS = [
    re.compile(r"\bpull requests? (?:are )?welcome\b", re.IGNORECASE),
    re.compile(r"\bcontributions? (?:are )?welcome\b", re.IGNORECASE),
    re.compile(r"\bsubmit (?:a |your )?(?:pull request|pr)\b", re.IGNORECASE),
    re.compile(r"\bopen (?:a |your )?(?:pull request|pr)\b", re.IGNORECASE),
    re.compile(r"\brequest (?:commit|write) access\b", re.IGNORECASE),
]

REQUIRED_POLICY_MARKERS = [
    "does not accept pull requests",
    "authorized agents",
    "independent forks",
]


@dataclass(frozen=True)
class Failure:
    path: str
    message: str


def read(path: pathlib.Path, failures: list[Failure]) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except OSError as exc:
        failures.append(Failure(str(path.relative_to(ROOT)), f"cannot read file: {exc}"))
        return ""


def main() -> int:
    failures: list[Failure] = []

    for relative in FORBIDDEN_PATHS:
        path = ROOT / relative
        if path.exists():
            failures.append(
                Failure(
                    relative,
                    "closed-development upstream must not advertise a contribution or PR workflow",
                )
            )

    policy_path = ROOT / "docs/DEVELOPMENT_POLICY.md"
    policy = read(policy_path, failures)
    lowered_policy = policy.lower()
    for marker in REQUIRED_POLICY_MARKERS:
        if marker not in lowered_policy:
            failures.append(
                Failure(
                    "docs/DEVELOPMENT_POLICY.md",
                    f"missing required custody marker {marker!r}",
                )
            )

    for relative in PUBLIC_ENTRYPOINTS:
        path = ROOT / relative
        if not path.is_file():
            failures.append(Failure(relative, "required public policy entrypoint is missing"))
            continue
        source = read(path, failures)
        for pattern in INVITATION_PATTERNS:
            match = pattern.search(source)
            if match is not None:
                failures.append(
                    Failure(
                        relative,
                        f"contains external-contribution invitation {match.group(0)!r}",
                    )
                )

    workflows = ROOT / ".github/workflows"
    if workflows.is_dir():
        for path in sorted(workflows.glob("*.y*ml")):
            source = read(path, failures)
            if re.search(r"(?m)^\s*pull_request(?:_target)?\s*:", source):
                failures.append(
                    Failure(
                        str(path.relative_to(ROOT)),
                        "workflow must not be triggered by external pull requests",
                    )
                )

    if failures:
        print(
            f"closed development contract: FAIL ({len(failures)} violation(s))",
            file=sys.stderr,
        )
        for failure in failures:
            print(f"  {failure.path}: {failure.message}", file=sys.stderr)
        return 1

    print("closed development contract: PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
