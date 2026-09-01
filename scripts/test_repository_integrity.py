#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import os
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

    def test_symbolic_link_file_is_rejected_without_following_target(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            target = root / "real.py"
            target.write_text("print('real')\n")
            link = root / "linked.py"
            try:
                link.symlink_to(target)
            except OSError:
                self.skipTest("symbolic links are unavailable")
            failures = checker.inspect(root)
            self.assertEqual(len(failures), 1)
            self.assertEqual(failures[0].path, "linked.py")
            self.assertIn("symbolic link", failures[0].reason)

    def test_symbolic_link_directory_is_rejected_and_not_traversed(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            external = Path(temporary).parent / f"dfmcp-external-{os.getpid()}"
            external.mkdir(exist_ok=False)
            try:
                (external / "poison.py").write_bytes(b"\xbf")
                link = root / "linked-source"
                try:
                    link.symlink_to(external, target_is_directory=True)
                except OSError:
                    self.skipTest("symbolic links are unavailable")
                failures = checker.inspect(root)
                self.assertEqual(len(failures), 1)
                self.assertEqual(failures[0].path, "linked-source")
                self.assertIn("symbolic link", failures[0].reason)
            finally:
                for child in external.iterdir():
                    child.unlink()
                external.rmdir()

    def test_fifo_is_rejected_without_opening_or_blocking(self) -> None:
        if not hasattr(os, "mkfifo"):
            self.skipTest("FIFOs are unavailable")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            fifo = root / "source.py"
            os.mkfifo(fifo)
            failures = checker.inspect(root)
            self.assertEqual(len(failures), 1)
            self.assertEqual(failures[0].path, "source.py")
            self.assertIn("not a regular file", failures[0].reason)

    def test_file_replacement_during_read_is_detected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            path = root / "source.py"
            path.write_text("print('first')\n")
            original_read_bytes = Path.read_bytes

            def replacing_read_bytes(candidate: Path) -> bytes:
                value = original_read_bytes(candidate)
                if candidate == path:
                    replacement = root / "replacement.py"
                    replacement.write_text("print('second')\n")
                    replacement.replace(path)
                return value

            Path.read_bytes = replacing_read_bytes
            try:
                failures = checker.inspect(root)
            finally:
                Path.read_bytes = original_read_bytes
            self.assertEqual(len(failures), 1)
            self.assertIn("changed while being inspected", failures[0].reason)


if __name__ == "__main__":
    unittest.main()
