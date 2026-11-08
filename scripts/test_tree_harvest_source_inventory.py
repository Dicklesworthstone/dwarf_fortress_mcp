#!/usr/bin/env python3
"""Required Tree input and registered consumer mutation controls; static only."""
from pathlib import Path
import os
import shutil
import tempfile
import unittest
from check_tree_harvest import ROOT, SOURCES, check
from test_tree_harvest_native import INPUTS, inventory


class TreeSourceInventoryTests(unittest.TestCase):
    def copy_sources(self, root):
        for name in SOURCES:
            target = root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, target)

    def test_registered_consumer_and_fixed_source_inventory_agree(self):
        result = check(ROOT)
        self.assertEqual(result["status"], "passed_static_only")
        self.assertEqual(set(result["source_sha256"]), set(SOURCES))
        self.assertFalse(result["rust_executed"])
        self.assertFalse(result["sdk_or_game_executed"])
        self.assertEqual(set(inventory(ROOT)), set(INPUTS))

    def test_missing_unregistered_ack_and_wire_tool_mutations_refuse(self):
        mutations = [
            ("crates/dfmcp-mcp/src/tree_harvest_server.rs", "unwrap_or(60000)", "unwrap_or(5000)"),
            ("crates/dfmcp-mcp/src/tree_harvest_server/runtime.rs", "source.preflight_commit(", "source.skipped_preflight("),
            ("crates/dfmcp-adapter/src/tree_harvest/rpc_tests.rs", None, None),
            ("crates/dfmcp-adapter/src/lib.rs", "pub mod tree_harvest;", ""),
            ("crates/dfmcp-adapter/src/tree_harvest/journal.rs", "ack.acknowledge(\n            self.pin(),", "acknowledgement_removed(\n            self.pin(),"),
            ("crates/dfmcp-mcp/src/tree_harvest_server.rs", '"fortress_commit"', '"fortress.commit"'),
            ("crates/dfmcp-mcp/src/tree_harvest_server/presentation.rs", "ResultMeta::empty()", "unexpected_result_metadata()"),
            ("crates/dfmcp-mcp/src/tree_harvest_server/presentation.rs", "CoreResult::Final(FinalCoreResult::ToolsCall", "unadmitted_codec_bypass"),
            ("crates/dfmcp-mcp/src/tree_harvest_server/runtime.rs", "c.authorize(Capability::Query, RiskTier::ReadOnly, &[], Some(area))?;", ""),
            ("crates/dfmcp-mcp/src/tree_harvest_server/runtime.rs", "nonce(c, self.scope)?", "nonce(c, unscoped_area())?"),
            ("crates/dfmcp-mcp/src/tree_harvest_server/runtime.rs", "nonce(&work.reserve(&current, 32)?, config.scope)?", "nonce(&work.reserve(&current, 32)?, unscoped_area())?"),
            ("crates/dfmcp-mcp/src/tree_harvest_server/runtime.rs", "c.budget.max_bytes < 32", "c.budget.max_bytes < 0"),
            ("crates/dfmcp-mcp/src/tree_harvest_server/presentation.rs", "const CHUNK_BYTES: usize = 24_000;", "const CHUNK_BYTES: usize = 48_000;"),
            ("crates/dfmcp-mcp/src/tree_harvest_server.rs", "-> McpResult<CompleteResult<FinalCallToolResult>>", "-> String"),
            ("crates/dfmcp-adapter/src/tree_harvest/mod.rs", "pub mod clock;", ""),
            ("crates/dfmcp-adapter/src/tree_harvest/journal/clock_origin.rs", None, None),
            ("crates/dfmcp-adapter/src/tree_harvest/journal/clock_origin.rs", "self.clock_projection(&mut work)?", "self.public_view_projection(&mut work)?"),
            ("crates/dfmcp-adapter/src/tree_harvest/rpc.rs", "self.progress_tick_floor.max(tick)", "tick"),
            ("crates/dfmcp-mcp/src/tree_harvest_server/clock.rs", "origin.require_capacity(", "origin.skip_capacity("),
            ("crates/dfmcp-mcp/src/tree_harvest_server/clock.rs", "clock.commit_guarded(", "clock.commit_unguarded("),
            ("crates/dfmcp-mcp/src/tree_harvest_server.rs", "Action::ClockCommit(review_seal, confirmed)", "Action::Commit(review_seal, confirmed)"),
            ("tests/stdio/tree_clock_runtime_peer.py", None, None),
            ("architecture/tree_harvest_v1_21.json", '"clock_control": true', '"clock_control": false'),
            ("scripts/verify.sh", " tests/stdio/tree_clock_runtime_peer.py", ""),
        ]
        mutations.append(("crates/dfmcp-mcp/src/tree_harvest_server/presentation.rs", ".len() + 4096 +", ".len() + 256 +"))
        for name, before, after in mutations:
            with self.subTest(name=name), tempfile.TemporaryDirectory() as folder:
                root = Path(folder)
                self.copy_sources(root)
                path = root / name
                if before is None:
                    path.unlink()
                else:
                    text = path.read_text()
                    self.assertGreater(text.count(before), 0)
                    path.write_text(text.replace(before, after))
                with self.assertRaises(ValueError):
                    check(root)

    def test_required_inputs_reject_missing_links_special_size_utf8_and_nul(self):
        for kind in ("missing", "link", "fifo", "large", "utf8", "nul"):
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as folder:
                root = Path(folder)
                for name in INPUTS:
                    path = root / name
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_text("bounded source\n")
                self.assertEqual(set(inventory(root)), set(INPUTS))
                path = root / INPUTS[0]
                path.unlink()
                if kind == "link":
                    path.symlink_to(root / INPUTS[1])
                elif kind == "fifo":
                    os.mkfifo(path)
                elif kind == "large":
                    with path.open("wb") as stream:
                        stream.truncate(4 * 1024 * 1024 + 1)
                elif kind in ("utf8", "nul"):
                    path.write_bytes(b"\xff" if kind == "utf8" else b"source\0data")
                with self.assertRaises((ValueError, UnicodeError)):
                    inventory(root)


if __name__ == "__main__":
    unittest.main()
