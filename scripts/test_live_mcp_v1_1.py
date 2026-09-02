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
CHECKER = ROOT / "scripts/check_live_mcp_v1_1.py"
FILES = [
    CHECKER,
    ROOT / "architecture/live_mcp_server_v1_1.json",
    ROOT / "architecture/live_admission_ticket_v2.json",
    ROOT / "crates/dfmcp-mcp/src/live_server_v1_1.rs",
    ROOT / "crates/dfmcp-mcp/src/lib.rs",
    ROOT / "crates/dwarf-fortress-mcp/src/bin/dfmcp-live-v1-1-dev-server.rs",
    ROOT / "crates/dwarf-fortress-mcp/Cargo.toml",
    ROOT / "crates/dwarf-fortress-mcp/tests/live_v1_1_development_admission.rs",
    ROOT / "architecture/live_announcement_source_qualification_v1_1.json",
    ROOT / "docs/LIVE_ANNOUNCEMENT_IMPLEMENTATION_STATUS.md",
    ROOT / "docs/LIVE_ANNOUNCEMENT_STREAM.md",
]


class LiveMcpV1_1ContractTests(unittest.TestCase):
    def run_checker(self, root: Path) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, "scripts/check_live_mcp_v1_1.py"],
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

    def test_runtime_admission_overclaim_is_rejected(self) -> None:
        temporary, root = self.fixture()
        with temporary:
            path = root / "architecture/live_mcp_server_v1_1.json"
            value = json.loads(path.read_text(encoding="utf-8"))
            value["authority"]["runtime_admitted"] = True
            path.write_text(json.dumps(value) + "\n", encoding="utf-8")
            self.assertNotEqual(self.run_checker(root).returncode, 0)

    def test_exact_development_opt_in_is_mandatory(self) -> None:
        temporary, root = self.fixture()
        with temporary:
            path = root / "architecture/live_mcp_server_v1_1.json"
            value = json.loads(path.read_text(encoding="utf-8"))
            value["binary"]["required_opt_in_environment"] = {
                "DFMCP_ALLOW_UNADMITTED_LIVE_V1_1": "true"
            }
            path.write_text(json.dumps(value) + "\n", encoding="utf-8")
            self.assertNotEqual(self.run_checker(root).returncode, 0)

    def test_production_protocol_map_widening_is_rejected(self) -> None:
        temporary, root = self.fixture()
        with temporary:
            path = root / "architecture/live_admission_ticket_v2.json"
            value = json.loads(path.read_text(encoding="utf-8"))
            value["runtime_dispatch"]["admitted_protocols"]["1.1"] = {
                "binary_command": "serve-live-v1-1",
                "rust_runner": "crate::live_server_v1_1::run_live_v1_1_development_stdio",
            }
            path.write_text(json.dumps(value) + "\n", encoding="utf-8")
            self.assertNotEqual(self.run_checker(root).returncode, 0)

    def test_admission_provenance_dependency_is_rejected(self) -> None:
        temporary, root = self.fixture()
        with temporary:
            path = root / "crates/dfmcp-mcp/src/live_server_v1_1.rs"
            source = path.read_text(encoding="utf-8")
            source += "\n// current_admission_provenance must remain unreachable here\n"
            path.write_text(source, encoding="utf-8")
            self.assertNotEqual(self.run_checker(root).returncode, 0)

    def test_public_development_guard_loss_is_rejected(self) -> None:
        temporary, root = self.fixture()
        with temporary:
            path = root / "crates/dfmcp-mcp/src/lib.rs"
            source = path.read_text(encoding="utf-8").replace(
                "std::env::var_os(ADMITTED_PROTOCOL_ENVIRONMENT).is_some()",
                "false",
                1,
            )
            path.write_text(source, encoding="utf-8")
            self.assertNotEqual(self.run_checker(root).returncode, 0)

    def test_announcement_query_mode_loss_is_rejected(self) -> None:
        temporary, root = self.fixture()
        with temporary:
            path = root / "architecture/live_mcp_server_v1_1.json"
            value = json.loads(path.read_text(encoding="utf-8"))
            value["mcp"]["query_modes"] = ["summary", "citizens", "all"]
            path.write_text(json.dumps(value) + "\n", encoding="utf-8")
            self.assertNotEqual(self.run_checker(root).returncode, 0)

    def test_process_test_loss_is_rejected(self) -> None:
        temporary, root = self.fixture()
        with temporary:
            path = (
                root
                / "crates/dwarf-fortress-mcp/tests/live_v1_1_development_admission.rs"
            )
            source = path.read_text(encoding="utf-8").replace(
                "protocol_1_1_development_server_rejects_protocol_bound_admission_state",
                "removed_protocol_bound_admission_test",
            )
            path.write_text(source, encoding="utf-8")
            self.assertNotEqual(self.run_checker(root).returncode, 0)

    def test_source_qualification_runtime_binding_is_required(self) -> None:
        temporary, root = self.fixture()
        with temporary:
            path = root / "architecture/live_announcement_source_qualification_v1_1.json"
            value = json.loads(path.read_text(encoding="utf-8"))
            value["required_gates"].remove("announcement-mcp-contract")
            del value["required_source_digests"]["production_admission_ticket_contract"]
            path.write_text(json.dumps(value) + "\n", encoding="utf-8")
            self.assertNotEqual(self.run_checker(root).returncode, 0)

    def test_documented_unadmitted_posture_is_required(self) -> None:
        temporary, root = self.fixture()
        with temporary:
            path = root / "docs/LIVE_ANNOUNCEMENT_IMPLEMENTATION_STATUS.md"
            source = path.read_text(encoding="utf-8").replace(
                "unadmitted development",
                "development",
            )
            path.write_text(source, encoding="utf-8")
            self.assertNotEqual(self.run_checker(root).returncode, 0)


if __name__ == "__main__":
    unittest.main()
