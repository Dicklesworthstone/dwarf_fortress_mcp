#!/usr/bin/env python3
"""Run every implemented, unadmitted protocol-1.1 source contract checker."""

from __future__ import annotations

import importlib.util
import sys
from pathlib import Path
from types import ModuleType

ROOT = Path(__file__).resolve().parents[1]
CHECKERS = [
    (
        "live_announcement_contract_core",
        ROOT / "scripts/check_live_announcements_core.py",
    ),
    (
        "live_announcement_publication",
        ROOT / "scripts/check_live_announcement_publication.py",
    ),
]


def load_module(name: str, path: Path) -> ModuleType:
    specification = importlib.util.spec_from_file_location(name, path)
    if specification is None or specification.loader is None:
        raise RuntimeError(f"cannot load announcement checker {path}")
    module = importlib.util.module_from_spec(specification)
    sys.modules[specification.name] = module
    specification.loader.exec_module(module)
    return module


def main() -> int:
    try:
        for name, path in CHECKERS:
            if not path.is_file():
                raise RuntimeError(f"announcement checker is missing: {path}")
            module = load_module(name, path)
            checker = getattr(module, "main", None)
            if not callable(checker):
                raise RuntimeError(f"announcement checker has no callable main: {path}")
            status = checker()
            if isinstance(status, bool) or not isinstance(status, int):
                raise RuntimeError(
                    f"announcement checker returned a non-integer status: {path}"
                )
            if status != 0:
                print(
                    f"live announcement aggregate: FAIL: {path.name} returned {status}",
                    file=sys.stderr,
                )
                return status
    except (OSError, RuntimeError, SyntaxError) as exc:
        print(f"live announcement aggregate: FAIL: {exc}", file=sys.stderr)
        return 1
    print(
        "live announcement aggregate: PASS "
        "(isolated protocol, transactional publication, and read-only adapter)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
