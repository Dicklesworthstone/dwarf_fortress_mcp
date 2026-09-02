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
CORE_CHECKER = ROOT / "scripts/check_live_announcements_core.py"
PUBLICATION_CHECKER = ROOT / "scripts/check_live_announcement_publication.py"
SOURCE_CONTRACT = ROOT / "architecture/live_announcement_source_qualification_v1_1.json"
NATIVE_RECEIPT_CONTRACT = ROOT / "architecture/dfhack_plugin_native_receipt_v1_1.json"
JOURNAL_CONTRACT = ROOT / "architecture/live_announcement_evidence_journal_v1.json"
BASE_WIRE = ROOT / "crates/dfmcp-adapter/src/dfhack_wire.rs"
BASE_SESSION = ROOT / "crates/dfmcp-adapter/src/live_session.rs"


def source_bound_files() -> list[Path]:
    contract = json.loads(SOURCE_CONTRACT.read_text(encoding="utf-8"))
    mapping = contract.get("required_source_digests")
    if not isinstance(mapping, dict):
        raise RuntimeError("announcement source qualification mapping is malformed")
    files = {
        CHECKER,
        CORE_CHECKER,
        PUBLICATION_CHECKER,
        SOURCE_CONTRACT,
        NATIVE_RECEIPT_CONTRACT,
        JOURNAL_CONTRACT,
        BASE_WIRE,
        BASE_SESSION,
    }
    for relative in mapping.values():
        if not isinstance(relative, str):
            raise RuntimeError("announcement source qualification path is not a string")
        files.add(ROOT / relative)
    missing = sorted(
        path.relative_to(ROOT).as_posix() for path in files if not path.is_file()
    )
    if missing:
        raise RuntimeError(
            "announcement contract fixture source is missing: " + ", ".join(missing)
        )
    return sorted(files, key=lambda path: path.relative_to(ROOT).as_posix())


class LiveAnnouncementContractTests(unittest.TestCase):
    def run_script(
        self, root: Path, relative: str
    ) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, relative],
            cwd=root,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )

    def run_checker(self, root: Path) -> subprocess.CompletedProcess[str]:
        return self.run_script(root, "scripts/check_live_announcements.py")

    def run_publication_checker(
        self, root: Path
    ) -> subprocess.CompletedProcess[str]:
        return self.run_script(
            root, "scripts/check_live_announcement_publication.py"
        )

    def fixture(self) -> tuple[tempfile.TemporaryDirectory[str], Path]:
        temporary = tempfile.TemporaryDirectory()
        root = Path(temporary.name)
        for source in source_bound_files():
            destination = root / source.relative_to(ROOT)
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, destination)
        return temporary, root

    def test_repository_contract_and_publication_checks_pass(self) -> None:
        contract = self.run_checker(ROOT)
        publication = self.run_publication_checker(ROOT)
        self.assertEqual(contract.returncode, 0, contract.stderr)
        self.assertEqual(publication.returncode, 0, publication.stderr)

    def test_fixture_contains_every_source_bound_file(self) -> None:
        temporary, root = self.fixture()
        with temporary:
            contract = json.loads(
                (root / SOURCE_CONTRACT.relative_to(ROOT)).read_text(
                    encoding="utf-8"
                )
            )
            mapping = contract["required_source_digests"]
            self.assertTrue(
                all((root / relative).is_file() for relative in mapping.values())
            )
            self.assertEqual(self.run_checker(root).returncode, 0)
            self.assertEqual(self.run_publication_checker(root).returncode, 0)

    def test_standalone_method_or_inherited_admission_is_rejected(self) -> None:
        temporary, root = self.fixture()
        with temporary:
            path = root / "architecture/dfhack_read_bridge_v1_1.json"
            value = json.loads(path.read_text(encoding="utf-8"))
            value["method_manifest"].append("ReadAnnouncements")
            path.write_text(json.dumps(value) + "\n", encoding="utf-8")
            self.assertNotEqual(self.run_checker(root).returncode, 0)

        temporary, root = self.fixture()
        with temporary:
            path = root / "architecture/dfhack_read_bridge_v1_1.json"
            value = json.loads(path.read_text(encoding="utf-8"))
            value["compatibility"]["inherits_protocol_1_0_admission"] = True
            path.write_text(json.dumps(value) + "\n", encoding="utf-8")
            self.assertNotEqual(self.run_checker(root).returncode, 0)

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

    def test_protobuf_and_native_method_waist_widening_are_rejected(self) -> None:
        temporary, root = self.fixture()
        with temporary:
            path = root / "bridge/dfhack-plugin/proto/DfmcpBridgeV1_1.proto"
            source = path.read_text(encoding="utf-8").replace(
                "// RPC ReadObservation : ReadObservationRequest -> ReadObservationReply",
                "// RPC ReadObservation : ReadObservationRequest -> ReadObservationReply\n"
                "// RPC ReadAnnouncements : ReadObservationRequest -> ReadObservationReply",
            )
            path.write_text(source, encoding="utf-8")
            self.assertNotEqual(self.run_checker(root).returncode, 0)

        temporary, root = self.fixture()
        with temporary:
            path = root / "bridge/dfhack-plugin/src/dfmcp_bridge_v1_1.cpp"
            source = path.read_text(encoding="utf-8").replace(
                'out->add_supported_methods("ReadObservation");',
                'out->add_supported_methods("ReadObservation");\n'
                '    out->add_supported_methods("ReadAnnouncements");',
                1,
            )
            path.write_text(source, encoding="utf-8")
            self.assertNotEqual(self.run_checker(root).returncode, 0)

    def test_publication_transaction_guard_loss_is_rejected(self) -> None:
        temporary, root = self.fixture()
        with temporary:
            path = (
                root
                / "crates/dfmcp-adapter/src/live_observation_publication_v1_1.rs"
            )
            source = path.read_text(encoding="utf-8").replace(
                "expected_base != &base",
                "expected_base == &base",
                1,
            )
            path.write_text(source, encoding="utf-8")
            self.assertNotEqual(
                self.run_publication_checker(root).returncode, 0
            )

    def test_black_box_budget_regression_loss_is_rejected(self) -> None:
        temporary, root = self.fixture()
        with temporary:
            path = (
                root
                / "crates/dfmcp-adapter/tests/live_adapter_v1_1_transactional.rs"
            )
            path.unlink()
            self.assertNotEqual(
                self.run_publication_checker(root).returncode, 0
            )

    def test_source_contract_rejects_a_missing_bound_file(self) -> None:
        temporary, root = self.fixture()
        with temporary:
            path = root / "crates/dfmcp-adapter/src/live_adapter_v1_1.rs"
            path.unlink()
            self.assertNotEqual(self.run_checker(root).returncode, 0)


if __name__ == "__main__":
    unittest.main()
