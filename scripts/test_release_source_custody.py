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
CHECKER = ROOT / "scripts/check_release_source_custody.py"
FILES = [
    CHECKER,
    ROOT / "architecture/release_source_custody_v1.json",
    ROOT / "architecture/local_qualification_receipt_v1.json",
    ROOT / "scripts/write_local_qualification_receipt.py",
    ROOT / "scripts/check_local_qualification_receipt.py",
    ROOT / "scripts/test_local_qualification_receipt.py",
    ROOT / "scripts/qualify_local.sh",
    ROOT / "architecture/live_server_binary_receipt_v1.json",
    ROOT / "scripts/verify_live_server_binary_receipt.py",
    ROOT / "scripts/test_live_server_binary_receipt.py",
    ROOT / "scripts/qualify_live_server_binary.sh",
    ROOT / "scripts/test_qualify_live_server_binary.py",
    ROOT / "scripts/verify.sh",
    ROOT / "docs/LOCAL_QUALIFICATION_AND_RELEASE.md",
]


class ReleaseSourceCustodyTests(unittest.TestCase):
    def run_checker(self, root: Path) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, "scripts/check_release_source_custody.py"],
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

    def test_machine_contract_cannot_weaken_head_equivalence(self) -> None:
        temporary, root = self.fixture()
        with temporary:
            path = root / "architecture/release_source_custody_v1.json"
            value = json.loads(path.read_text(encoding="utf-8"))
            value["tracked_source"]["working_tree_bytes_must_match_head_blobs"] = False
            path.write_text(json.dumps(value) + "\n", encoding="utf-8")
            self.assertNotEqual(self.run_checker(root).returncode, 0)

    def test_local_no_replace_regression_is_rejected(self) -> None:
        temporary, root = self.fixture()
        with temporary:
            path = root / "scripts/write_local_qualification_receipt.py"
            source = path.read_text(encoding="utf-8").replace(
                "os.link(temporary, destination, follow_symlinks=False)",
                "os.replace(temporary, destination)",
            )
            path.write_text(source, encoding="utf-8")
            self.assertNotEqual(self.run_checker(root).returncode, 0)

    def test_hidden_index_drift_test_loss_is_rejected(self) -> None:
        temporary, root = self.fixture()
        with temporary:
            path = root / "scripts/test_local_qualification_receipt.py"
            source = path.read_text(encoding="utf-8").replace(
                "test_assume_unchanged_cannot_hide_head_divergent_bytes",
                "removed_hidden_index_test",
            )
            path.write_text(source, encoding="utf-8")
            self.assertNotEqual(self.run_checker(root).returncode, 0)

    def test_private_receipt_custody_loss_is_rejected(self) -> None:
        temporary, root = self.fixture()
        with temporary:
            path = root / "scripts/verify_live_server_binary_receipt.py"
            source = path.read_text(encoding="utf-8").replace(
                "must have exact owner-read/write mode 0600",
                "mode is acceptable",
            )
            path.write_text(source, encoding="utf-8")
            self.assertNotEqual(self.run_checker(root).returncode, 0)

    def test_complete_inventory_replay_loss_is_rejected(self) -> None:
        temporary, root = self.fixture()
        with temporary:
            path = root / "scripts/verify_live_server_binary_receipt.py"
            source = path.read_text(encoding="utf-8").replace(
                "server receipt source inventory changed after local receipt verification",
                "server receipt source inventory was inspected",
            )
            path.write_text(source, encoding="utf-8")
            self.assertNotEqual(self.run_checker(root).returncode, 0)

    def test_server_no_replace_regression_is_rejected(self) -> None:
        temporary, root = self.fixture()
        with temporary:
            path = root / "scripts/qualify_live_server_binary.sh"
            source = path.read_text(encoding="utf-8").replace(
                "os.link(temporary,destination,follow_symlinks=False)",
                "os.replace(temporary,destination)",
            )
            path.write_text(source, encoding="utf-8")
            self.assertNotEqual(self.run_checker(root).returncode, 0)

    def test_build_time_source_revalidation_loss_is_rejected(self) -> None:
        temporary, root = self.fixture()
        with temporary:
            path = root / "scripts/qualify_live_server_binary.sh"
            source = path.read_text(encoding="utf-8").replace(
                'info "Revalidating source after build and executable checks"\n'
                "validate_local_receipt\n"
                'ok "Source remained identical to the prerequisite receipt"\n',
                "",
            )
            path.write_text(source, encoding="utf-8")
            self.assertNotEqual(self.run_checker(root).returncode, 0)

    def test_top_level_wiring_loss_is_rejected(self) -> None:
        temporary, root = self.fixture()
        with temporary:
            path = root / "scripts/verify.sh"
            source = path.read_text(encoding="utf-8").replace(
                "python3 scripts/check_release_source_custody.py\n",
                "",
            )
            path.write_text(source, encoding="utf-8")
            self.assertNotEqual(self.run_checker(root).returncode, 0)

    def test_documented_head_equivalence_is_required(self) -> None:
        temporary, root = self.fixture()
        with temporary:
            path = root / "docs/LOCAL_QUALIFICATION_AND_RELEASE.md"
            source = (
                path.read_text(encoding="utf-8")
                .replace("HEAD-equivalent", "source-matched")
                .replace("head-equivalent", "source-matched")
            )
            path.write_text(source, encoding="utf-8")
            self.assertNotEqual(self.run_checker(root).returncode, 0)


if __name__ == "__main__":
    unittest.main()
