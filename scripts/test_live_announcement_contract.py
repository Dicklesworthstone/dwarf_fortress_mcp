#!/usr/bin/env python3

from __future__ import annotations

import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from typing import Any, Callable

ROOT = Path(__file__).resolve().parents[1]
CHECKER = ROOT / "scripts/check_live_announcements.py"
FILES = [
    CHECKER,
    ROOT / "architecture/dfhack_read_bridge_v1_1.json",
    ROOT / "architecture/live_announcement_projection_v1.json",
    ROOT / "architecture/live_announcement_source_qualification_v1_1.json",
    ROOT / "architecture/dfhack_plugin_native_receipt_v1_1.json",
    ROOT / "architecture/live_announcement_acceptance_v1_1.json",
    ROOT / "architecture/live_announcement_evidence_journal_v1.json",
    ROOT / "bridge/dfhack-plugin/proto/DfmcpBridge.proto",
    ROOT / "bridge/dfhack-plugin/src/dfmcp_bridge.cpp",
    ROOT / "crates/dfmcp-adapter/src/dfhack_wire.rs",
    ROOT / "crates/dfmcp-adapter/src/live_session.rs",
    ROOT / "bridge/dfhack-plugin/proto/DfmcpBridgeV1_1.proto",
    ROOT / "bridge/dfhack-plugin/src/dfmcp_bridge_v1_1.cpp",
    ROOT / "crates/dfmcp-adapter/src/live_announcement_batch.rs",
    ROOT / "crates/dfmcp-adapter/src/announcement_wire.rs",
    ROOT / "crates/dfmcp-adapter/src/dfhack_wire_v1_1.rs",
    ROOT / "crates/dfmcp-adapter/src/live_observation_v1_1.rs",
    ROOT / "crates/dfmcp-adapter/src/live_session_v1_1.rs",
    ROOT / "crates/dfmcp-adapter/src/fenced_live_source_v1_1.rs",
    ROOT / "crates/dfmcp-adapter/src/live_connect_v1_1.rs",
    ROOT / "crates/dfmcp-adapter/src/live_announcement_projection.rs",
    ROOT / "crates/dfmcp-adapter/src/live_projection_v1_1.rs",
    ROOT / "crates/dfmcp-adapter/src/live_announcement_briefing.rs",
    ROOT / "crates/dfmcp-adapter/src/lib.rs",
    ROOT / "crates/dwarf-fortress-mcp/src/bin/dfmcp-live-announcement-probe.rs",
    ROOT / "scripts/qualify_live_announcement_source.sh",
    ROOT / "docs/LIVE_ANNOUNCEMENT_STREAM.md",
    ROOT / "docs/LIVE_ANNOUNCEMENT_IMPLEMENTATION_STATUS.md",
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

    def assert_checker_rejects_json_mutation(
        self,
        relative: str,
        mutate: Callable[[dict[str, Any]], None],
    ) -> None:
        temporary, root = self.fixture()
        with temporary:
            path = root / relative
            value = json.loads(path.read_text(encoding="utf-8"))
            mutate(value)
            path.write_text(json.dumps(value) + "\n", encoding="utf-8")
            self.assertNotEqual(self.run_checker(root).returncode, 0)

    def test_repository_contract_passes(self) -> None:
        result = self.run_checker(ROOT)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_protocol_1_0_announcement_resurrection_is_rejected(self) -> None:
        temporary, root = self.fixture()
        with temporary:
            proto = root / "bridge/dfhack-plugin/proto/DfmcpBridge.proto"
            proto.write_text(
                proto.read_text(encoding="utf-8")
                + "\n// RPC ReadAnnouncements : ReadObservationRequest -> ReadObservationReply\n",
                encoding="utf-8",
            )
            self.assertNotEqual(self.run_checker(root).returncode, 0)

        temporary, root = self.fixture()
        with temporary:
            session = root / "crates/dfmcp-adapter/src/live_session.rs"
            session.write_text(
                session.read_text(encoding="utf-8")
                + "\npub trait LiveAnnouncementSource {}\n",
                encoding="utf-8",
            )
            self.assertNotEqual(self.run_checker(root).returncode, 0)

    def test_standalone_method_or_inherited_admission_is_rejected(self) -> None:
        self.assert_checker_rejects_json_mutation(
            "architecture/dfhack_read_bridge_v1_1.json",
            lambda value: value["method_manifest"].append("ReadAnnouncements"),
        )
        self.assert_checker_rejects_json_mutation(
            "architecture/dfhack_read_bridge_v1_1.json",
            lambda value: value["compatibility"].update(
                {"inherits_protocol_1_0_admission": True}
            ),
        )

    def test_projection_history_overclaim_is_rejected(self) -> None:
        self.assert_checker_rejects_json_mutation(
            "architecture/live_announcement_projection_v1.json",
            lambda value: value["coverage"].update(
                {"may_prove_complete_history": True}
            ),
        )

    def test_acceptance_case_loss_is_rejected(self) -> None:
        self.assert_checker_rejects_json_mutation(
            "architecture/live_announcement_acceptance_v1_1.json",
            lambda value: value["gates"][-1]["cases"].pop(),
        )

    def test_native_receipt_mutation_authority_is_rejected(self) -> None:
        self.assert_checker_rejects_json_mutation(
            "architecture/dfhack_plugin_native_receipt_v1_1.json",
            lambda value: value["bridge"].update(
                {"mutation_rpc_methods": ["Pause"]}
            ),
        )

    def test_source_contract_must_bind_batch_and_complete_gate_order(self) -> None:
        self.assert_checker_rejects_json_mutation(
            "architecture/live_announcement_source_qualification_v1_1.json",
            lambda value: value["required_source_digests"].pop(
                "announcement_batch"
            ),
        )
        self.assert_checker_rejects_json_mutation(
            "architecture/live_announcement_source_qualification_v1_1.json",
            lambda value: value["required_gates"].reverse(),
        )

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

    def test_batch_bound_and_crate_wiring_drift_are_rejected(self) -> None:
        temporary, root = self.fixture()
        with temporary:
            batch = root / "crates/dfmcp-adapter/src/live_announcement_batch.rs"
            source = batch.read_text(encoding="utf-8").replace(
                "MAX_ANNOUNCEMENTS_PER_BATCH: usize = 512",
                "MAX_ANNOUNCEMENTS_PER_BATCH: usize = 513",
            )
            batch.write_text(source, encoding="utf-8")
            self.assertNotEqual(self.run_checker(root).returncode, 0)

        temporary, root = self.fixture()
        with temporary:
            library = root / "crates/dfmcp-adapter/src/lib.rs"
            source = library.read_text(encoding="utf-8").replace(
                "pub mod live_announcement_batch;\n", ""
            )
            library.write_text(source, encoding="utf-8")
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

    def test_resurrected_retired_contract_or_model_is_rejected(self) -> None:
        temporary, root = self.fixture()
        with temporary:
            retired = root / "architecture/live_announcement_read_v1.json"
            retired.write_text("{}\n", encoding="utf-8")
            self.assertNotEqual(self.run_checker(root).returncode, 0)

        temporary, root = self.fixture()
        with temporary:
            retired = root / "crates/dfmcp-adapter/src/live_announcements.rs"
            retired.write_text("pub struct AnnouncementWindowAssembler;\n", encoding="utf-8")
            self.assertNotEqual(self.run_checker(root).returncode, 0)


if __name__ == "__main__":
    unittest.main()
