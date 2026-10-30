#!/usr/bin/env python3
"""Mutation checks for the live MCP source-shape contract."""
import unittest
from unittest.mock import patch

import check_live_mcp as gate
import check_live_read_stack as stack


class LiveMcpContractTests(unittest.TestCase):
    def failures(self, mutations=None):
        mutations = mutations or {}
        read = gate.read

        def altered(path, failures):
            source = read(path, failures)
            return mutations[path](source) if path in mutations else source

        failures = []
        with patch.object(gate, "read", side_effect=altered):
            gate.check_live_server(failures)
            gate.check_adapter_chain(failures)
            gate.check_cli_and_crate_wiring(failures)
        return failures

    def test_current_source_passes(self):
        self.assertEqual(self.failures(), [])

    def test_secret_and_extra_open_session_arguments_are_rejected(self):
        path = "crates/dfmcp-mcp/src/live_server.rs"
        for added in ("bridge_token: Option<String>", "extra: Option<String>"):
            with self.subTest(added=added):
                failures = self.failures({path: lambda s: s.replace(
                    "    paused: Option<bool>,\n    fortress_selector:",
                    f"    {added},\n    paused: Option<bool>,\n    fortress_selector:", 1)})
                self.assertTrue(any("argument" in f.message for f in failures))

    def test_public_runner_bypass_is_rejected(self):
        path = "crates/dfmcp-mcp/src/lib.rs"
        failures = self.failures({path: lambda s: s.replace("mod live_server;", "pub mod live_server;", 1)})
        self.assertTrue(any("compiled privately" in f.message for f in failures))

    def test_ticket_or_protocol_dispatch_removal_is_rejected(self):
        path = "crates/dfmcp-mcp/src/admission.rs"
        for needle in ("match consume_admission_ticket_from_environment()", "ADMITTED_BRIDGE_PROTOCOL_V1 => Ok(crate::live_server::run_live_stdio)"):
            with self.subTest(needle=needle):
                failures = self.failures({path: lambda s: s.replace(needle, "REMOVED_ADMISSION", 1)})
                self.assertTrue(any("protocol-bound admission" in f.message for f in failures))

    def test_single_read_regression_is_rejected(self):
        path = "crates/dfmcp-adapter/src/live_bootstrap.rs"
        failures = self.failures({path: lambda s: s.replace("bootstrap_reads_the_underlying_source_once", "removed_single_read_test")})
        self.assertTrue(any("bootstrap_reads_the_underlying_source_once" in f.message for f in failures))

    def test_read_stack_rejects_lost_crate_safety_and_duplicate_guard(self):
        for path, needle, expected in (
            ("crates/dfmcp-mcp/src/lib.rs", "#![forbid(unsafe_code)]", "crate-wide safe-Rust"),
            ("crates/dfmcp-adapter/src/live_projection.rs", "entities.insert(entity_id, entity).is_some()", "duplicate canonical entity"),
            ("crates/dfmcp-adapter/src/live_projection.rs", "edges.insert(edge.id, edge).is_some()", "duplicate membership edge"),
        ):
            with self.subTest(needle=needle):
                read = stack.read_required
                failures = []
                def altered(p, f):
                    value = read(p, f)
                    return value.replace(needle, "removed_guard", 1) if p == path else value
                with patch.object(stack, "read_required", side_effect=altered):
                    stack.check_adapter_graph(failures)
                self.assertTrue(any(expected in f.message for f in failures))

    def test_read_stack_rejects_public_runner(self):
        path = "crates/dfmcp-mcp/src/lib.rs"
        read = gate.read
        def altered(p, f):
            value = read(p, f)
            return value.replace("mod live_server;", "pub mod live_server;", 1) if p == path else value
        failures = []
        with patch.object(gate, "read", side_effect=altered):
            stack.check_mcp_and_cli(failures)
        self.assertTrue(any("compiled privately" in f.message for f in failures))


if __name__ == "__main__":
    unittest.main()
