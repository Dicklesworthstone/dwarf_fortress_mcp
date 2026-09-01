#!/usr/bin/env python3

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CHECKER = ROOT / "scripts/check_live_announcement_contract.py"
CONTRACT = ROOT / "architecture/live_announcement_read_v1.json"
MODULE = ROOT / "crates/dfmcp-adapter/src/live_announcements.rs"
PROTO = ROOT / "bridge/dfhack-plugin/proto/DfmcpBridge.proto"
DESIGN = ROOT / "docs/LIVE_ANNOUNCEMENT_READ_GENERATION.md"


class LiveAnnouncementContractTests(unittest.TestCase):
    def run_checker(self, root: Path) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, "scripts/check_live_announcement_contract.py"],
            cwd=root,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )

    def fixture(self) -> tuple[tempfile.TemporaryDirectory[str], Path]:
        temporary = tempfile.TemporaryDirectory()
        root = Path(temporary.name)
        for source in [CHECKER, CONTRACT, MODULE, PROTO, DESIGN]:
            destination = root / source.relative_to(ROOT)
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(source.read_bytes())
        return temporary, root

    def test_repository_contract_passes(self) -> None:
        result = self.run_checker(ROOT)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_mutation_capability_contamination_is_rejected(self) -> None:
        temporary, root = self.fixture()
        with temporary:
            path = root / CONTRACT.relative_to(ROOT)
            value = json.loads(path.read_text(encoding="utf-8"))
            value["bridge_protocol"]["mutation_capabilities"] = ["pause"]
            path.write_text(json.dumps(value) + "\n", encoding="utf-8")
            self.assertNotEqual(self.run_checker(root).returncode, 0)

    def test_complete_history_overclaim_is_rejected(self) -> None:
        temporary, root = self.fixture()
        with temporary:
            path = root / CONTRACT.relative_to(ROOT)
            value = json.loads(path.read_text(encoding="utf-8"))
            value["coverage"]["never_claims"] = "nothing"
            path.write_text(json.dumps(value) + "\n", encoding="utf-8")
            self.assertNotEqual(self.run_checker(root).returncode, 0)

    def test_missing_frozen_high_water_semantics_are_rejected(self) -> None:
        temporary, root = self.fixture()
        with temporary:
            path = root / MODULE.relative_to(ROOT)
            path.write_text(
                path.read_text(encoding="utf-8").replace(
                    "window_latest_report_id", "removed_window_latest_report_id"
                ),
                encoding="utf-8",
            )
            self.assertNotEqual(self.run_checker(root).returncode, 0)

    def test_protocol_method_drift_is_rejected(self) -> None:
        temporary, root = self.fixture()
        with temporary:
            path = root / PROTO.relative_to(ROOT)
            path.write_text(
                path.read_text(encoding="utf-8").replace(
                    "ReadAnnouncements", "ReadSomethingElse"
                ),
                encoding="utf-8",
            )
            self.assertNotEqual(self.run_checker(root).returncode, 0)


if __name__ == "__main__":
    unittest.main()
