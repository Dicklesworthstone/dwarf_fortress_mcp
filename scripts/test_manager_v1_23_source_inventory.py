#!/usr/bin/env python3
"""Mutation checks for the closed Manager/1.23 static inventory."""
import json
from pathlib import Path
import shutil
import tempfile
import unittest
from check_manager_v1_23 import ROOT, SOURCES, check


class ManagerV123InventoryTests(unittest.TestCase):
    def copy(self, root):
        for name in SOURCES:
            dest = root / name
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, dest)

    def test_real_source_inventory_passes(self):
        self.assertEqual(check()['status'], 'passed_static_only')

    def test_missing_native_or_wrapper_source_refuses(self):
        for name in ('bridge/dfhack-manager-v1_23/dfmcp_manager_v1_23.cpp', 'crates/dfmcp-mcp/tests/manager_appointment_v1_23_stdio.rs'):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                root = Path(directory); self.copy(root); (root / name).unlink()
                with self.assertRaises((ValueError, OSError)):
                    check(root)

    def test_generation_guard_and_both_standard_gates_are_required(self):
        cases = [
            ('bridge/common/manager_appointment.h', 'MAX_TOTAL_ROLE_ENTRIES=4096', 'MAX_TOTAL_ROLE_ENTRIES=8192'),
            ('crates/dfmcp-adapter/src/manager_appointment/mod.rs', 'MAX_ROLE_ENTRIES: usize = 256;', 'MAX_ROLE_ENTRIES: usize = 512;'),
            ('bridge/dfhack-manager-v1_23/dfmcp_manager_v1_23.cpp', '"DFMCP_MANAGER_V1_23_TOKEN"', '"DFMCP_MANAGER_TOKEN"'),
            ('bridge/dfhack-manager-v1_23/DfmcpManagerV1_23.proto', 'package dfmcp.manager.v1_23;', 'package dfmcp.manager.v1_22;'),
            ('crates/dfmcp-mcp/src/manager_appointment_v1_23_server.rs', 'DFMCP_ADMITTED_BRIDGE_PROTOCOL', 'REMOVED_PRODUCTION_GUARD'),
            ('scripts/verify.sh', 'python3 scripts/test_manager_v1_23_native.py --mutations', 'python3 scripts/test_manager_native.py --mutations'),
            ('scripts/qualify_local.sh', 'python3 scripts/test_manager_v1_23_native.py --mutations', 'python3 scripts/test_manager_native.py --mutations'),
            ('crates/dfmcp-mcp/tests/manager_appointment_v1_23_stdio.rs', 'assert_eq!(report["tests"], 8);', 'assert_eq!(report["tests"], 7);'),
        ]
        for name, old, new in cases:
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                root = Path(directory); self.copy(root); p = root / name; s = p.read_text()
                self.assertEqual(s.count(old), 1); p.write_text(s.replace(old, new))
                with self.assertRaises((ValueError, KeyError)):
                    check(root)

    def test_changed_vector_or_contract_refuses(self):
        for name in ('tests/fixtures/manager_v1_23_component.json', 'architecture/manager_appointment_v1_23.json'):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                root = Path(directory); self.copy(root); p = root / name; d = json.loads(p.read_text())
                if name.startswith('tests/'):
                    d['plan'] = '00' + d['plan'][2:]
                else:
                    d['native_component_source_paths'].pop()
                p.write_text(json.dumps(d))
                with self.assertRaises(ValueError):
                    check(root)


if __name__ == '__main__':
    unittest.main()
