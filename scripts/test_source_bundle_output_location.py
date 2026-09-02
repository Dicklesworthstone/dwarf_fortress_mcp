#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

CREATOR_PATH = Path(__file__).with_name("create_source_bundle.py")
SPEC = importlib.util.spec_from_file_location("source_bundle_creator_output_tests", CREATOR_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load source bundle creator")
creator = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = creator
SPEC.loader.exec_module(creator)


def git(root: Path, *arguments: str) -> str:
    completed = subprocess.run(
        ["git", "-C", os.fspath(root), *arguments],
        check=True,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    return completed.stdout.strip()


def repository(root: Path) -> Path:
    source = root / "repository"
    source.mkdir()
    git(source, "init", "-q")
    git(source, "config", "user.email", "source-bundle@example.invalid")
    git(source, "config", "user.name", "Source Bundle Tests")
    (source / ".gitignore").write_text("target/\n", encoding="utf-8")
    (source / "README.md").write_text("bounded source bundle\n", encoding="utf-8")
    git(source, "add", ".")
    git(source, "commit", "-q", "-m", "fixture")
    return source


class SourceBundleOutputLocationTests(unittest.TestCase):
    def test_default_ignored_parent_is_created_and_verified(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            source = repository(Path(temporary))
            commit = git(source, "rev-parse", "HEAD")
            destination = source / "target" / "source-bundle" / commit
            self.assertFalse(destination.parent.exists())
            result = creator.create_bundle(
                source.resolve(),
                destination,
                creator.DEFAULT_CONTRACT,
            )
            self.assertEqual(result["status"], "created_and_verified")
            self.assertEqual(Path(result["output_directory"]), destination)
            self.assertTrue(Path(result["archive"]).is_file())
            self.assertEqual(
                git(source, "status", "--porcelain=v1", "--untracked-files=all"),
                "",
            )

    def test_symbolic_link_parent_is_rejected_without_external_write(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = repository(root)
            external = root / "external"
            external.mkdir()
            link = root / "linked-parent"
            try:
                link.symlink_to(external, target_is_directory=True)
            except OSError:
                self.skipTest("symbolic links are unavailable")
            destination = link / "bundle"
            with self.assertRaises(creator.SourceBundleCreationError):
                creator.create_bundle(
                    source.resolve(),
                    destination.absolute(),
                    creator.DEFAULT_CONTRACT,
                )
            self.assertFalse((external / "bundle").exists())

    def test_unignored_missing_parent_inside_source_is_not_created(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            source = repository(Path(temporary))
            parent = source / "release-output" / "nested"
            destination = parent / "bundle"
            with self.assertRaises(creator.SourceBundleCreationError):
                creator.create_bundle(
                    source.resolve(),
                    destination,
                    creator.DEFAULT_CONTRACT,
                )
            self.assertFalse(parent.exists())


if __name__ == "__main__":
    unittest.main()
