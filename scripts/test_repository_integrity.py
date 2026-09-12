#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import io
import os
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path
from unittest.mock import patch

MODULE_PATH = Path(__file__).with_name("check_repository_integrity.py")
SPEC = importlib.util.spec_from_file_location("check_repository_integrity", MODULE_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load repository-integrity checker")
checker = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = checker
SPEC.loader.exec_module(checker)


class ToolchainPolicyTests(unittest.TestCase):
    def test_both_repository_gates_enforce_the_approved_fleet_pin(self) -> None:
        root = Path(__file__).resolve().parents[1]
        toolchain_path = root / "rust-toolchain.toml"
        policy_path = root / "architecture/dependency_allowlist.toml"
        original_read_text = Path.read_text
        original_open = Path.open
        for script in ("check_dependency_policy", "validate_repo"):
            spec = importlib.util.spec_from_file_location(script, root / "scripts" / f"{script}.py")
            if spec is None or spec.loader is None:
                raise RuntimeError(f"cannot load {script}")
            gate = importlib.util.module_from_spec(spec)
            sys.modules[spec.name] = gate
            spec.loader.exec_module(gate)
            for changed_path in (toolchain_path, policy_path):
                for candidate in ("nightly-2026-08-31", "nightly", "stable", "nightly-2026-08-30", ""):
                    with self.subTest(script=script, changed_path=changed_path, candidate=candidate):
                        replacement = original_read_text(changed_path).replace("nightly-2026-08-31", candidate)

                        def read_text(path: Path, *args, **kwargs) -> str:
                            if path == changed_path:
                                return replacement
                            return original_read_text(path, *args, **kwargs)

                        def open_path(path: Path, *args, **kwargs):
                            if path == changed_path:
                                return io.BytesIO(replacement.encode("utf-8"))
                            return original_open(path, *args, **kwargs)

                        if script == "validate_repo":
                            gate.FAILURES.clear()
                            gate.CHECKS = 0
                        output = io.StringIO()
                        with patch.object(Path, "read_text", read_text), patch.object(Path, "open", open_path):
                            with redirect_stdout(output), redirect_stderr(output):
                                status = gate.main()
                        self.assertEqual(status, 0 if candidate == "nightly-2026-08-31" else 1, output.getvalue())
                        if status != 0:
                            self.assertIn("approved fleet pin nightly-2026-08-31", output.getvalue())


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
