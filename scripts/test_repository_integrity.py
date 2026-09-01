#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path

MODULE_PATH = Path(__file__).with_name("check_repository_integrity.py")
SPEC = importlib.util.spec_from_file_location("check_repository_integrity", MODULE_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load repository-integrity checker")
checker = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = checker
SPEC.loader.exec_module(checker)


class RepositoryIntegrityTests(unittest.TestCase):
    def test_clean_tree_passes(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "src").mkdir()
            (root / "src/lib.rs").write_text("pub fn answer() -> u32 { 42 }\n")
            self.assertEqual(checker.inspect(root), [])

    def test_absolute_path_placeholder_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "test.rs").write_text("/mnt/data/work/test.rs\n")
            failures = checker.inspect(root)
            self.assertEqual(len(failures), 1)
            self.assertIn("absolute path", failures[0].reason)

    def test_probe_and_recovery_names_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / ".tool_probe_ignore").write_text("normal content\n")
            (root / "agent.rs.restore-pointer").write_text("normal content\n")
            self.assertEqual(len(checker.inspect(root)), 2)

    def test_generated_directories_are_ignored(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "target").mkdir()
            (root / "target/result.txt").write_text("/mnt/data/generated/result.txt\n")
            self.assertEqual(checker.inspect(root), [])

    def test_non_utf8_python_source_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "broken.py").write_bytes(b"print('ok')\n\xbf\n")
            failures = checker.inspect(root)
            self.assertEqual(len(failures), 1)
            self.assertIn("not valid UTF-8", failures[0].reason)

    def test_nul_corrupted_rust_source_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "broken.rs").write_bytes(b"pub fn ok() {}\x00\n")
            failures = checker.inspect(root)
            self.assertEqual(len(failures), 1)
            self.assertIn("NUL byte", failures[0].reason)

    def test_binary_asset_is_not_misclassified_as_source(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "fixture.bin").write_bytes(b"\x00\xff\x80")
            self.assertEqual(checker.inspect(root), [])

    def test_oversized_source_text_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            path = root / "oversized.py"
            with path.open("wb") as handle:
                handle.truncate(checker.MAX_TEXT_BYTES + 1)
            failures = checker.inspect(root)
            self.assertEqual(len(failures), 1)
            self.assertIn("integrity bound", failures[0].reason)


if __name__ == "__main__":
    unittest.main()
