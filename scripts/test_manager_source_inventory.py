#!/usr/bin/env python3
"""Manager source custody mutation controls. Static evidence only."""
from pathlib import Path
import json
import shutil
import tempfile
import unittest
from check_manager_appointment import ROOT, SOURCES, check


class ManagerSourceInventoryTests(unittest.TestCase):
    def copy(self, root):
        for name in SOURCES:
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, path)

    def test_exact_source_inventory_is_closed_and_not_execution(self):
        result = check()
        self.assertEqual(set(result["source_sha256"]), set(SOURCES))
        self.assertEqual(result["adapter_modules"], 8)
        self.assertFalse(result["rust_executed"])
        self.assertFalse(result["sdk_or_game_executed"])

    def test_missing_symlink_nul_and_vector_drift_refuse(self):
        for kind in ("missing", "link", "nul", "vector"):
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                self.copy(root)
                name = "tests/fixtures/manager_v1_22_component.json" if kind == "vector" else "crates/dfmcp-adapter/src/manager_appointment/record.rs"
                path = root / name
                if kind in ("missing", "link"):
                    path.unlink()
                    if kind == "link": path.symlink_to(root / SOURCES[0])
                else:
                    path.write_bytes(path.read_bytes() + (b"\0" if kind == "nul" else b" "))
                with self.assertRaises((ValueError, UnicodeError)):
                    check(root)

    def test_wrong_generation_or_unregistered_standard_route_refuse(self):
        for name, before, after in (("scripts/verify.sh", "python3 scripts/test_manager_native.py --mutations", "python3 scripts/test_manager_source_inventory.py"), ("bridge/dfhack-manager-v1_22/DfmcpManagerV1_22.proto", "package dfmcp.manager.v1_22;", "package dfmcp.manager.v1_21;"), ("crates/dwarf-fortress-mcp/src/main.rs", '"manager" => dfmcp_mcp::manager_appointment_server::run_stdio()', '"wrong" => dfmcp_mcp::manager_appointment_server::run_stdio()'), ("crates/dfmcp-mcp/src/manager_appointment_server/presentation.rs", "const MAX_JSONRPC_ID_BYTES: usize = 4096;", "const MAX_JSONRPC_ID_BYTES: usize = 256;"), ("crates/dfmcp-adapter/src/manager_appointment/rpc_tests.rs", "fn manager_prepare_requires_authority_at_capture_tick_without_prior_observe()", "fn unregistered_prepare_authority_test()")):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                self.copy(root)
                path = root / name
                self.assertIn(before, path.read_text())
                path.write_text(path.read_text().replace(before, after))
                with self.assertRaises(ValueError): check(root)


if __name__ == "__main__": unittest.main()
