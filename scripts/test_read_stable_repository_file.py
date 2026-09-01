#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

MODULE_PATH = Path(__file__).with_name("read_stable_repository_file.py")
SPEC = importlib.util.spec_from_file_location("read_stable_repository_file", MODULE_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load stable repository file reader")
reader = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(reader)


class StableRepositoryFileTests(unittest.TestCase):
    def test_regular_file_returns_exact_content_and_identity(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "source.py"
            path.write_bytes(b"print('stable')\n")
            result = reader.read_stable_regular_file(path, 1024, "source")
            metadata = path.stat()
            self.assertEqual(result.content, b"print('stable')\n")
            self.assertEqual(result.size, metadata.st_size)
            self.assertEqual(result.device, metadata.st_dev)
            self.assertEqual(result.inode, metadata.st_ino)
            self.assertEqual(len(result.sha256), 64)

    def test_empty_file_requires_explicit_permission(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "empty.txt"
            path.write_bytes(b"")
            with self.assertRaises(reader.StableReadError):
                reader.read_stable_regular_file(path, 1, "empty")
            result = reader.read_stable_regular_file(
                path, 1, "empty", allow_empty=True
            )
            self.assertEqual(result.content, b"")
            self.assertEqual(result.size, 0)

    def test_symbolic_link_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            target = root / "target.py"
            target.write_text("print('target')\n")
            link = root / "link.py"
            try:
                link.symlink_to(target)
            except OSError:
                self.skipTest("symbolic links are unavailable")
            with self.assertRaises(reader.StableReadError):
                reader.read_stable_regular_file(link, 1024, "source")

    def test_fifo_is_rejected_without_blocking(self) -> None:
        if not hasattr(os, "mkfifo"):
            self.skipTest("FIFOs are unavailable")
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "source.py"
            os.mkfifo(path)
            with self.assertRaises(reader.StableReadError):
                reader.read_stable_regular_file(path, 1024, "source")

    def test_declared_size_bound_is_enforced(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "large.py"
            path.write_bytes(b"x" * 33)
            with self.assertRaises(reader.StableReadError):
                reader.read_stable_regular_file(path, 32, "source")

    def test_path_replacement_between_lstat_and_open_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            path = root / "source.py"
            replacement = root / "replacement.py"
            path.write_bytes(b"first\n")
            replacement.write_bytes(b"second\n")
            real_open = os.open
            replaced = False

            def replacing_open(candidate: object, flags: int, *args: object) -> int:
                nonlocal replaced
                if not replaced and Path(candidate) == path:
                    replaced = True
                    replacement.replace(path)
                return real_open(candidate, flags, *args)

            with mock.patch.object(reader.os, "open", side_effect=replacing_open):
                with self.assertRaises(reader.StableReadError):
                    reader.read_stable_regular_file(path, 1024, "source")

    def test_growth_while_reading_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "source.py"
            path.write_bytes(b"first\n")
            real_read = os.read
            mutated = False

            def growing_read(descriptor: int, count: int) -> bytes:
                nonlocal mutated
                chunk = real_read(descriptor, count)
                if chunk and not mutated:
                    mutated = True
                    with path.open("ab") as handle:
                        handle.write(b"second\n")
                        handle.flush()
                        os.fsync(handle.fileno())
                return chunk

            with mock.patch.object(reader.os, "read", side_effect=growing_read):
                with self.assertRaises(reader.StableReadError):
                    reader.read_stable_regular_file(path, 1024, "source")

    def test_invalid_limits_and_control_character_paths_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "source.py"
            path.write_bytes(b"ok\n")
            for value in [0, -1, True]:
                with self.subTest(value=value):
                    with self.assertRaises(reader.StableReadError):
                        reader.read_stable_regular_file(path, value, "source")
            bad = Path(os.fspath(path) + "\n")
            with self.assertRaises(reader.StableReadError):
                reader.read_stable_regular_file(bad, 1024, "source")


if __name__ == "__main__":
    unittest.main()
