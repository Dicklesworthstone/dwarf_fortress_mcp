#!/usr/bin/env python3

from __future__ import annotations

import base64
import importlib.util
import os
import sys
import tempfile
import unittest
from pathlib import Path

MODULE_PATH = Path(__file__).with_name("scan_live_read_secrets.py")
SPEC = importlib.util.spec_from_file_location("scan_live_read_secrets", MODULE_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load live secret scanner")
scanner = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = scanner
SPEC.loader.exec_module(scanner)

TOKEN = b"correct-horse-battery-staple-12345"


class LiveSecretScannerTests(unittest.TestCase):
    def test_clean_artifacts_produce_normalized_zero_match_event(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "artifacts"
            root.mkdir()
            (root / "bridge.log").write_text("bridge started without secrets\n")
            (root / "doctor.json").write_text('{"status":"healthy"}\n')
            output = root.parent / "secret-scan.json"
            event, matches = scanner.scan(root, output, TOKEN)
            self.assertEqual(matches, [])
            self.assertEqual(event["case"], "secret_scan")
            self.assertEqual(event["result"], "passed")
            self.assertEqual(event["match_count"], 0)
            self.assertEqual(event["scanned_file_count"], 2)
            self.assertNotIn(TOKEN.decode(), str(event))

    def test_raw_token_leak_is_detected_without_echoing_secret(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "artifacts"
            root.mkdir()
            (root / "bridge.log").write_bytes(b"prefix " + TOKEN + b" suffix")
            event, matches = scanner.scan(root, root.parent / "out.json", TOKEN)
            self.assertGreater(event["match_count"], 0)
            self.assertEqual(matches[0].path, "bridge.log")
            self.assertEqual(matches[0].representation, "raw")
            self.assertNotIn(TOKEN.decode(), repr(matches))

    def test_hex_and_base64_representations_are_detected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "artifacts"
            root.mkdir()
            (root / "hex.log").write_bytes(TOKEN.hex().encode())
            (root / "base64.log").write_bytes(base64.b64encode(TOKEN))
            event, matches = scanner.scan(root, root.parent / "out.json", TOKEN)
            representations = {match.representation for match in matches}
            self.assertIn("hex_lower", representations)
            self.assertIn("base64", representations)
            self.assertGreaterEqual(event["match_count"], 2)

    def test_environment_assignment_is_detected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "artifacts"
            root.mkdir()
            (root / "env.txt").write_bytes(b"DFMCP_BRIDGE_TOKEN=" + TOKEN)
            _, matches = scanner.scan(root, root.parent / "out.json", TOKEN)
            self.assertIn("environment_assignment", {match.representation for match in matches})

    def test_output_inside_artifact_root_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            output = root / "secret-scan.json"
            (root / "clean.log").write_text("clean\n")
            with self.assertRaises(scanner.ScanError):
                scanner.scan(root, output, TOKEN)

    def test_empty_tree_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "artifacts"
            root.mkdir()
            with self.assertRaises(scanner.ScanError):
                scanner.scan(root, root.parent / "out.json", TOKEN)

    def test_symbolic_link_artifact_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "artifacts"
            root.mkdir()
            target = root / "target.log"
            target.write_text("clean\n")
            link = root / "link.log"
            try:
                link.symlink_to(target)
            except OSError:
                self.skipTest("symbolic links are unavailable")
            with self.assertRaises(scanner.ScanError):
                scanner.scan(root, root.parent / "out.json", TOKEN)

    def test_symbolic_link_output_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "artifacts"
            root.mkdir()
            (root / "clean.log").write_text("clean\n")
            target = root.parent / "target.json"
            target.write_text("unchanged\n")
            output = root.parent / "scan-link.json"
            try:
                output.symlink_to(target)
            except OSError:
                self.skipTest("symbolic links are unavailable")
            with self.assertRaises(scanner.ScanError):
                scanner.scan(root, output, TOKEN)
            self.assertEqual(target.read_text(), "unchanged\n")

    def test_stable_reader_rejects_replaced_file_identity(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            path = root / "artifact.log"
            path.write_text("first\n")
            expected = path.lstat()
            replacement = root / "replacement.log"
            replacement.write_text("second\n")
            os.replace(replacement, path)
            with self.assertRaises(scanner.ScanError):
                scanner.read_stable_regular_file(path, expected, "artifact.log")

    def test_oversized_file_is_rejected_before_read(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "artifacts"
            root.mkdir()
            path = root / "large.log"
            with path.open("wb") as handle:
                handle.truncate(scanner.MAX_FILE_BYTES + 1)
            with self.assertRaises(scanner.ScanError):
                scanner.scan(root, root.parent / "out.json", TOKEN)

    def test_token_policy_is_enforced(self) -> None:
        with self.assertRaises(scanner.ScanError):
            scanner.representations(b"")


if __name__ == "__main__":
    unittest.main()
