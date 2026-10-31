#!/usr/bin/env python3
"""Exercise announcement source custody against actual temporary Git trees."""

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import announcement_source_custody as announcement
from test_local_qualification_receipt import Fixture, run_git


class AnnouncementSourceCustodyTests(unittest.TestCase):
    def capture(self, fixture: Fixture) -> None:
        announcement.capture(
            fixture.repository, announcement.custody.DEFAULT_CONTRACT,
            fixture.snapshot, fixture.commit,
        )

    def publish(self, fixture: Fixture) -> dict:
        return announcement.publish(
            fixture.repository, announcement.custody.DEFAULT_CONTRACT,
            fixture.snapshot, fixture.commit, fixture.receipt,
            {"schema": "dfmcp.live-announcement-source-qualification/1",
             "status": "passed", "capabilities_granted": [], "mutation_capabilities": []},
        )

    def test_exact_source_publishes_private_create_only_bound_receipt(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            fixture = Fixture(Path(temp))
            self.capture(fixture)
            receipt = self.publish(fixture)
            source = receipt["source"]
            self.assertEqual(source["commit"], fixture.commit)
            self.assertFalse(source["dirty"])
            self.assertTrue(source["head_equivalent"])
            self.assertEqual(source["entry_count"], 2)
            snapshot = json.loads(fixture.snapshot.read_text())
            self.assertEqual(source["snapshot_digest"], snapshot["snapshot_digest"])
            self.assertEqual(source["entries_digest"], snapshot["entries_digest"])
            unsigned = dict(receipt)
            declared = unsigned.pop("receipt_digest")
            self.assertEqual(declared, announcement.custody.sha256_bytes(
                announcement.custody.canonical_json(unsigned)))
            original = fixture.receipt.read_bytes()
            with self.assertRaises(announcement.custody.QualificationReceiptError):
                self.publish(fixture)
            self.assertEqual(fixture.receipt.read_bytes(), original)

    def test_tracked_untracked_hidden_and_head_changes_refuse_publication(self) -> None:
        for kind in ("tracked", "untracked", "hidden", "head"):
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as temp:
                fixture = Fixture(Path(temp))
                self.capture(fixture)
                source = fixture.repository / "src/lib.rs"
                if kind == "hidden":
                    run_git(fixture.repository, "update-index", "--assume-unchanged", "src/lib.rs")
                if kind == "untracked":
                    (fixture.repository / "unknown.txt").write_text("changed\n")
                else:
                    source.write_text("pub fn answer() -> u32 { 17 }\n")
                if kind == "head":
                    run_git(fixture.repository, "add", ".")
                    run_git(fixture.repository, "commit", "-qm", "changed")
                if kind == "hidden":
                    self.assertEqual(run_git(fixture.repository, "status", "--porcelain=v1"), "")
                with self.assertRaises(announcement.custody.QualificationReceiptError):
                    self.publish(fixture)
                self.assertFalse(fixture.receipt.exists())

    def test_dirty_initial_source_cannot_be_captured(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            fixture = Fixture(Path(temp))
            (fixture.repository / "src/lib.rs").write_text("changed\n")
            with self.assertRaises(announcement.custody.QualificationReceiptError):
                self.capture(fixture)
            self.assertFalse(fixture.snapshot.exists())

    def test_post_publication_missing_source_removes_receipt(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            fixture = Fixture(Path(temp))
            self.capture(fixture)
            original = announcement.custody.write_atomic_create_only

            def remove_source(path: Path, value: dict) -> Path:
                published = original(path, value)
                (fixture.repository / "src/lib.rs").unlink()
                return published

            with mock.patch.object(
                announcement.custody, "write_atomic_create_only", side_effect=remove_source
            ):
                with self.assertRaises(FileNotFoundError):
                    self.publish(fixture)
            self.assertFalse(fixture.receipt.exists())

    def test_post_publication_changed_source_removes_receipt(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            fixture = Fixture(Path(temp))
            self.capture(fixture)
            original = announcement.custody.write_atomic_create_only

            def change_source(path: Path, value: dict) -> Path:
                published = original(path, value)
                (fixture.repository / "src/lib.rs").write_text("changed\n")
                return published

            with mock.patch.object(
                announcement.custody, "write_atomic_create_only", side_effect=change_source
            ):
                with self.assertRaises(announcement.custody.QualificationReceiptError):
                    self.publish(fixture)
            self.assertFalse(fixture.receipt.exists())


if __name__ == "__main__":
    unittest.main()
