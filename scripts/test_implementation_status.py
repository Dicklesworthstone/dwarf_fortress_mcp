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
CHECKER = ROOT / "scripts/check_implementation_status.py"
FILES = [
    ROOT / "architecture/implementation_status_v1.json",
    ROOT / "architecture/live_compatibility_registry_v1.json",
    ROOT / "architecture/live_admission_ticket_v2.json",
    ROOT / "architecture/live_mcp_server_v1_1.json",
    ROOT / "architecture/dfhack_read_bridge_v1.json",
    ROOT / "architecture/dfhack_read_bridge_v1_1.json",
    ROOT / "IMPLEMENTATION_STATUS.md",
    ROOT / "README.md",
    ROOT / "CHANGELOG.md",
    ROOT / "ROADMAP.md",
    ROOT / "SECURITY.md",
    CHECKER,
]


class Fixture:
    def __init__(self, root: Path) -> None:
        self.root = root
        for source in FILES:
            destination = root / source.relative_to(ROOT)
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, destination)

    def run(self) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, "scripts/check_implementation_status.py", "--root", "."],
            cwd=self.root,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )

    def mutate_json(self, relative: str, mutation: Callable[[dict[str, Any]], None]) -> None:
        path = self.root / relative
        value = json.loads(path.read_text(encoding="utf-8"))
        mutation(value)
        path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


class ImplementationStatusTests(unittest.TestCase):
    def fixture(self) -> tuple[tempfile.TemporaryDirectory[str], Fixture]:
        temporary = tempfile.TemporaryDirectory()
        return temporary, Fixture(Path(temporary.name))

    def test_repository_status_contract_passes(self) -> None:
        result = subprocess.run(
            [sys.executable, "scripts/check_implementation_status.py"],
            cwd=ROOT,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_registry_entry_or_admitted_status_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.mutate_json(
                "architecture/live_compatibility_registry_v1.json",
                lambda value: (
                    value.__setitem__("status", "admitted_live_tuples"),
                    value["entries"].append({"entry_id": "0" * 64}),
                ),
            )
            self.assertNotEqual(fixture.run().returncode, 0)

    def test_production_protocol_map_widening_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.mutate_json(
                "architecture/live_admission_ticket_v2.json",
                lambda value: value["runtime_dispatch"]["admitted_protocols"].__setitem__(
                    "1.1",
                    {
                        "binary_command": "serve-live-v1-1",
                        "rust_runner": "crate::live_server_v1_1::run_live_v1_1_development_stdio",
                    },
                ),
            )
            self.assertNotEqual(fixture.run().returncode, 0)

    def test_legacy_ticket_acceptance_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.mutate_json(
                "architecture/live_admission_ticket_v2.json",
                lambda value: value["canonical_binding"].__setitem__(
                    "legacy_ticket_schema_accepted", True
                ),
            )
            self.assertNotEqual(fixture.run().returncode, 0)

    def test_development_runtime_admission_overclaim_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.mutate_json(
                "architecture/live_mcp_server_v1_1.json",
                lambda value: value["authority"].__setitem__("runtime_admitted", True),
            )
            self.assertNotEqual(fixture.run().returncode, 0)

    def test_development_runtime_dispatch_permission_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.mutate_json(
                "architecture/live_mcp_server_v1_1.json",
                lambda value: value["binary"].__setitem__(
                    "production_protocol_dispatch_allowed", True
                ),
            )
            self.assertNotEqual(fixture.run().returncode, 0)

    def test_bridge_method_widening_or_mutation_effect_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.mutate_json(
                "architecture/dfhack_read_bridge_v1_1.json",
                lambda value: (
                    value["method_manifest"].append("Pause"),
                    value["methods"].append(
                        {
                            "name": "Pause",
                            "effect": "mutation",
                            "requires_world": True,
                            "requires_authentication": True,
                        }
                    ),
                ),
            )
            self.assertNotEqual(fixture.run().returncode, 0)

    def test_authoritative_document_marker_loss_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            path = fixture.root / "IMPLEMENTATION_STATUS.md"
            source = path.read_text(encoding="utf-8").replace(
                "No live tuple is currently admitted",
                "Current status",
                1,
            )
            path.write_text(source, encoding="utf-8")
            self.assertNotEqual(fixture.run().returncode, 0)

    def test_tracked_qualification_or_deployment_evidence_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            evidence = fixture.root / "evidence" / "qualification-receipt.json"
            evidence.parent.mkdir()
            evidence.write_text('{"status":"passed"}\n', encoding="utf-8")
            result = fixture.run()
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("qualification-receipt.json", result.stderr)

    def test_phase_change_requires_contract_update(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.mutate_json(
                "architecture/implementation_status_v1.json",
                lambda value: value.__setitem__("phase", "production"),
            )
            self.assertNotEqual(fixture.run().returncode, 0)

    def test_unknown_protocol_policy_drift_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.mutate_json(
                "architecture/live_admission_ticket_v2.json",
                lambda value: value["runtime_dispatch"].__setitem__(
                    "unknown_or_unadmitted_protocol_policy", "fallback_to_1.0"
                ),
            )
            self.assertNotEqual(fixture.run().returncode, 0)

    def test_duplicate_contract_key_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            path = fixture.root / "architecture/implementation_status_v1.json"
            source = path.read_text(encoding="utf-8")
            path.write_text(
                source.replace(
                    '"schema_version": "dfmcp.implementation-status-contract/1",',
                    '"schema_version": "dfmcp.implementation-status-contract/1",\n'
                    '  "schema_version": "dfmcp.implementation-status-contract/1",',
                    1,
                ),
                encoding="utf-8",
            )
            self.assertNotEqual(fixture.run().returncode, 0)


if __name__ == "__main__":
    unittest.main()
