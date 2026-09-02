#!/usr/bin/env python3

from __future__ import annotations

import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CHECKER = ROOT / "scripts/check_live_announcements.py"
FILES = [
    CHECKER,
    ROOT / "architecture/dfhack_read_bridge_v1_1.json",
    ROOT / "architecture/live_announcement_projection_v1.json",
    ROOT / "architecture/dfhack_plugin_native_receipt_v1_1.json",
    ROOT / "architecture/live_announcement_acceptance_v1_1.json",
    ROOT / "architecture/live_announcement_evidence_journal_v1.json",
    ROOT / "bridge/dfhack-plugin/proto/DfmcpBridgeV1_1.proto",
    ROOT / "bridge/dfhack-plugin/src/dfmcp_bridge_v1_1.cpp",
    ROOT / "crates/dfmcp-adapter/src/live_announcements.rs",
    ROOT / "crates/dfmcp-adapter/src/announcement_wire.rs",
    ROOT / "crates/dfmcp-adapter/src/lib.rs",
    ROOT / "docs/LIVE_ANNOUNCEMENT_STREAM.md",
]


class LiveAnnouncementContractTests(unittest.TestCase):
    def run_checker(self, root: Path) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, "scripts/check_live_announcements.py"],
            cwd=root,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )

    def fixture(self) -> tuple[tempfile.TemporaryDirectory[str], Path]:
        temporary = tempfile.TemporaryDirectory()
        root = Path(temporary.name)
        for source in FILES:
            destination = root / source.relative_to(ROOT)
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, destination)
        return temporary, root

    def test_repository_contract_passes(self) -> None:
        result = self.run_checker(ROOT)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_standalone_method_or_inherited_admission_is_rejected(self) -> None:
        temporary, root = self.fixture()
        with temporary:
            path = root / "architecture/dfhack_read_bridge_v1_1.json"
            value = json.loads(path.read_text(encoding="utf-8"))
            value["method_manifest"].append("ReadAnnouncements")
            path.write_text(json.dumps(value) + "\n", encoding="utf-8")
            self.assertNotEqual(self.run_checker(root).returncode, 0)

            temporary2, root2 = self.fixture()
            with temporary2:
                path2 = root2 / "architecture/dfhack_read_bridge_v1_1.json"
                value2 = json.loads(path2.read_text(encoding="utf-8"))
                value2["compatibility"]["inherits_protocol_1_0_admission"] = True
                path2.write_text(json.dumps(value2) + "\n", encoding="utf-8")
                self.assertNotEqual(self.run_checker(root2).returncode, 0)

    def test_projection_history_overclaim_is_rejected(self) -> None:
        temporary, root = self.fixture()
        with temporary:
            path = root / "architecture/live_announcement_projection_v1.json"
            value = json.loads(path.read_text(encoding="utf-8"))
            value["coverage"]["may_prove_complete_history"] = True
            path.write_text(json.dumps(value) + "\n", encoding="utf-8")
            self.assertNotEqual(self.run_checker(root).returncode, 0)

    def test_acceptance_case_loss_is_rejected(self) -> None:
        temporary, root = self.fixture()
        with temporary:
            path = root / "architecture/live_announcement_acceptance_v1_1.json"
            value = json.loads(path.read_text(encoding="utf-8"))
            value["gates"][-1]["cases"].pop()
            path.write_text(json.dumps(value) + "\n", encoding="utf-8")
            self.assertNotEqual(self.run_checker(root).returncode, 0)

    def test_native_receipt_mutation_authority_is_rejected(self) -> None:
        temporary, root = self.fixture()
        with temporary:
            path = root / "architecture/dfhack_plugin_native_receipt_v1_1.json"
            value = json.loads(path.read_text(encoding="utf-8"))
            value["bridge"]["mutation_rpc_methods"] = ["Pause"]
            path.write_text(json.dumps(value) + "\n", encoding="utf-8")
            self.assertNotEqual(self.run_checker(root).returncode, 0)

    def test_protobuf_method_waist_widening_is_rejected(self) -> None:
        temporary, root = self.fixture()
        with temporary:
            path = root / "bridge/dfhack-plugin/proto/DfmcpBridgeV1_1.proto"
            source = path.read_text(encoding="utf-8")
            source = source.replace(
                "// RPC ReadObservation : ReadObservationRequest -> ReadObservationReply",
                "// RPC ReadObservation : ReadObservationRequest -> ReadObservationReply\n"
                "// RPC ReadAnnouncements : ReadObservationRequest -> ReadObservationReply",
            )
            path.write_text(source, encoding="utf-8")
            self.assertNotEqual(self.run_checker(root).returncode, 0)

    def test_native_method_manifest_widening_is_rejected(self) -> None:
        temporary, root = self.fixture()
        with temporary:
            path = root / "bridge/dfhack-plugin/src/dfmcp_bridge_v1_1.cpp"
            source = path.read_text(encoding="utf-8")
            source = source.replace(
                'out->add_supported_methods("ReadObservation");',
                'out->add_supported_methods("ReadObservation");\n'
                '    out->add_supported_methods("ReadAnnouncements");',
                1,
            )
            path.write_text(source, encoding="utf-8")
            self.assertNotEqual(self.run_checker(root).returncode, 0)

    def test_announcement_gap_documentation_loss_is_rejected(self) -> None:
        temporary, root = self.fixture()
        with temporary:
            path = root / "docs/LIVE_ANNOUNCEMENT_STREAM.md"
            source = path.read_text(encoding="utf-8").replace(
                "gap_before_window", "removed_gap_marker"
            )
            path.write_text(source, encoding="utf-8")
            self.assertNotEqual(self.run_checker(root).returncode, 0)


if __name__ == "__main__":
    unittest.main()
