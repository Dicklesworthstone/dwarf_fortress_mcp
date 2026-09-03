#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import json
import os
import stat
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from typing import Any
from unittest import mock

MODULE_PATH = Path(__file__).with_name("write_local_qualification_receipt.py")
if os.fspath(MODULE_PATH.parent) not in sys.path:
    sys.path.insert(0, os.fspath(MODULE_PATH.parent))
SPEC = importlib.util.spec_from_file_location("write_local_qualification_receipt_test", MODULE_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load local qualification receipt issuer")
issuer = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = issuer
SPEC.loader.exec_module(issuer)


def run_git(root: Path, *arguments: str) -> str:
    completed = subprocess.run(
        ["git", "-C", os.fspath(root), *arguments],
        check=True,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    return completed.stdout.strip()


class Fixture:
    def __init__(self, root: Path) -> None:
        self.root = root
        self.repository = root / "repository"
        self.output = root / "output"
        self.repository.mkdir()
        self.output.mkdir(mode=0o700)
        self.output.chmod(0o700)
        run_git(self.repository, "init", "-q")
        run_git(self.repository, "config", "user.email", "qualification@example.invalid")
        run_git(self.repository, "config", "user.name", "Qualification Tests")
        (self.repository / "src").mkdir()
        (self.repository / "src" / "lib.rs").write_text(
            "pub fn answer() -> u32 { 42 }\n", encoding="utf-8"
        )
        tool = self.repository / "qualify.sh"
        tool.write_text("#!/usr/bin/env bash\nexit 0\n", encoding="utf-8")
        tool.chmod(0o755)
        run_git(self.repository, "add", ".")
        run_git(self.repository, "commit", "-q", "-m", "fixture")
        self.commit = run_git(self.repository, "rev-parse", "HEAD")
        self.gate_contract = root / "gate-contract.json"
        self.gate_contract.write_text(
            json.dumps(
                {
                    "schema_version": "dfmcp.live-server-binary-receipt-contract/1",
                    "source_binding": {
                        "required_local_qualification_gates": [
                            "repository-integrity",
                            "rustfmt",
                            "tests",
                        ]
                    },
                },
                sort_keys=True,
            )
            + "\n",
            encoding="utf-8",
        )
        self.snapshot = self.output / "source-snapshot.json"
        self.gates = self.output / "gates.tsv"
        self.receipt = self.output / "qualification-receipt.json"

    def begin(self, *, allow_dirty: bool = False, snapshot: Path | None = None) -> dict[str, Any]:
        return issuer.begin(
            self.repository.resolve(),
            issuer.DEFAULT_CONTRACT,
            (self.snapshot if snapshot is None else snapshot).resolve(),
            self.commit,
            allow_dirty,
        )

    def write_gates(self, rows: list[tuple[str, str, str]]) -> None:
        self.gates.write_text(
            "".join("\t".join(row) + "\n" for row in rows), encoding="utf-8"
        )
        self.gates.chmod(0o600)

    def finish(self, requested_status: str = "passed") -> dict[str, Any]:
        return issuer.finish(
            self.repository.resolve(),
            issuer.DEFAULT_CONTRACT,
            self.gate_contract.resolve(),
            self.snapshot.resolve(),
            self.gates.resolve(),
            self.receipt.resolve(),
            self.commit,
            "2026-09-02T12:00:00Z",
            requested_status,
        )

    def passing_gates(self) -> None:
        self.write_gates(
            [
                ("repository-integrity", "passed", ""),
                ("rustfmt", "passed", ""),
                ("tests", "passed", ""),
            ]
        )


class LocalQualificationReceiptTests(unittest.TestCase):
    def fixture(self) -> tuple[tempfile.TemporaryDirectory[str], Fixture]:
        temporary = tempfile.TemporaryDirectory()
        return temporary, Fixture(Path(temporary.name))

    def test_clean_snapshot_and_passed_receipt_are_exact_and_owner_private(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            first = fixture.begin()
            second_path = fixture.output / "second-snapshot.json"
            second = fixture.begin(snapshot=second_path)
            self.assertEqual(fixture.snapshot.read_bytes(), second_path.read_bytes())
            self.assertEqual(first["snapshot_digest"], second["snapshot_digest"])
            self.assertTrue(first["head_equivalent"])
            fixture.passing_gates()
            receipt = fixture.finish()
            self.assertEqual(receipt["schema"], issuer.RECEIPT_SCHEMA)
            self.assertEqual(receipt["status"], "passed")
            self.assertEqual(receipt["source"]["commit"], fixture.commit)
            self.assertEqual(receipt["source"]["dirty"], False)
            self.assertEqual(receipt["source"]["head_equivalent"], True)
            self.assertEqual(list(receipt["digests"]), ["qualify.sh", "src/lib.rs"])
            self.assertTrue(all(len(value) == 64 for value in receipt["digests"].values()))
            self.assertEqual(stat.S_IMODE(fixture.output.stat().st_mode), 0o700)
            self.assertEqual(stat.S_IMODE(fixture.snapshot.stat().st_mode), 0o600)
            self.assertEqual(stat.S_IMODE(fixture.receipt.stat().st_mode), 0o600)

    def test_tracked_byte_drift_prevents_receipt_publication(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.begin()
            fixture.passing_gates()
            (fixture.repository / "src/lib.rs").write_text(
                "pub fn answer() -> u32 { 7 }\n", encoding="utf-8"
            )
            with self.assertRaises(issuer.QualificationReceiptError):
                fixture.finish()
            self.assertFalse(fixture.receipt.exists())

    def test_commit_drift_prevents_receipt_publication(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.begin()
            fixture.passing_gates()
            (fixture.repository / "new.txt").write_text("new commit\n", encoding="utf-8")
            run_git(fixture.repository, "add", ".")
            run_git(fixture.repository, "commit", "-q", "-m", "drift")
            with self.assertRaises(issuer.QualificationReceiptError):
                fixture.finish()
            self.assertFalse(fixture.receipt.exists())

    def test_untracked_status_drift_prevents_receipt_publication(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.begin()
            fixture.passing_gates()
            (fixture.repository / "untracked.txt").write_text("drift\n", encoding="utf-8")
            with self.assertRaises(issuer.QualificationReceiptError):
                fixture.finish()
            self.assertFalse(fixture.receipt.exists())

    def test_dirty_source_requires_opt_in_and_downgrades_passed_status(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            (fixture.repository / "src/lib.rs").write_text(
                "pub fn answer() -> u32 { 43 }\n", encoding="utf-8"
            )
            with self.assertRaises(issuer.QualificationReceiptError):
                fixture.begin()
            result = fixture.begin(allow_dirty=True)
            self.assertTrue(result["dirty"])
            self.assertFalse(result["head_equivalent"])
            fixture.passing_gates()
            receipt = fixture.finish()
            self.assertEqual(receipt["status"], "development_dirty")
            self.assertTrue(receipt["source"]["dirty"])
            self.assertFalse(receipt["source"]["head_equivalent"])

    def test_assume_unchanged_cannot_hide_head_divergent_bytes(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            run_git(fixture.repository, "update-index", "--assume-unchanged", "src/lib.rs")
            (fixture.repository / "src/lib.rs").write_text(
                "pub fn answer() -> u32 { 99 }\n", encoding="utf-8"
            )
            self.assertEqual(run_git(fixture.repository, "status", "--porcelain=v1"), "")
            with self.assertRaises(issuer.QualificationReceiptError):
                fixture.begin()
            result = fixture.begin(allow_dirty=True)
            self.assertTrue(result["dirty"])
            self.assertFalse(result["head_equivalent"])

    @unittest.skipUnless(os.name == "posix", "Unix executable semantics required")
    def test_executable_mode_drift_cannot_hide_behind_core_filemode_false(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            run_git(fixture.repository, "config", "core.fileMode", "false")
            path = fixture.repository / "src/lib.rs"
            path.chmod(0o755)
            self.assertEqual(run_git(fixture.repository, "status", "--porcelain=v1"), "")
            with self.assertRaises(issuer.QualificationReceiptError):
                fixture.begin()
            result = fixture.begin(allow_dirty=True)
            self.assertFalse(result["head_equivalent"])

    def test_inherited_git_directory_override_is_ignored(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            other = fixture.root / "other"
            other.mkdir()
            run_git(other, "init", "-q")
            run_git(other, "config", "user.email", "other@example.invalid")
            run_git(other, "config", "user.name", "Other")
            (other / "other.txt").write_text("other\n", encoding="utf-8")
            run_git(other, "add", ".")
            run_git(other, "commit", "-q", "-m", "other")
            with mock.patch.dict(os.environ, {"GIT_DIR": os.fspath(other / ".git")}, clear=False):
                result = fixture.begin()
            self.assertEqual(result["commit"], fixture.commit)

    def test_empty_tracked_file_is_bound_without_false_failure(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            empty = fixture.repository / "EMPTY"
            empty.write_bytes(b"")
            run_git(fixture.repository, "add", "EMPTY")
            run_git(fixture.repository, "commit", "-q", "-m", "empty")
            fixture.commit = run_git(fixture.repository, "rev-parse", "HEAD")
            result = fixture.begin()
            self.assertTrue(result["head_equivalent"])
            snapshot = json.loads(fixture.snapshot.read_text(encoding="utf-8"))
            entry = next(item for item in snapshot["entries"] if item["path"] == "EMPTY")
            self.assertEqual(entry["bytes"], 0)
            self.assertEqual(entry["sha256"], issuer.sha256_bytes(b""))

    def test_static_only_accepts_only_a_passing_canonical_prefix(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.begin()
            fixture.write_gates(
                [
                    ("repository-integrity", "passed", ""),
                    ("rustfmt", "passed", ""),
                ]
            )
            receipt = fixture.finish("static_only")
            self.assertEqual(receipt["status"], "static_only")

        temporary, fixture = self.fixture()
        with temporary:
            fixture.begin()
            fixture.write_gates([("rustfmt", "passed", "")])
            with self.assertRaises(issuer.QualificationReceiptError):
                fixture.finish("static_only")

    def test_incomplete_or_reordered_passed_gates_are_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.begin()
            fixture.write_gates([("repository-integrity", "passed", "")])
            with self.assertRaises(issuer.QualificationReceiptError):
                fixture.finish()

        temporary, fixture = self.fixture()
        with temporary:
            fixture.begin()
            fixture.write_gates(
                [
                    ("repository-integrity", "passed", ""),
                    ("tests", "passed", ""),
                    ("rustfmt", "passed", ""),
                ]
            )
            with self.assertRaises(issuer.QualificationReceiptError):
                fixture.finish()

    def test_failed_receipt_accepts_a_failed_canonical_prefix(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.begin()
            fixture.write_gates(
                [
                    ("repository-integrity", "passed", ""),
                    ("rustfmt", "failed", "exit=1"),
                ]
            )
            receipt = fixture.finish("failed")
            self.assertEqual(receipt["status"], "failed")
            self.assertEqual(receipt["gates"][-1]["state"], "failed")

    def test_snapshot_and_receipt_outputs_are_create_only(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.begin()
            before = fixture.snapshot.read_bytes()
            with self.assertRaises(issuer.QualificationReceiptError):
                fixture.begin()
            self.assertEqual(fixture.snapshot.read_bytes(), before)
            fixture.passing_gates()
            fixture.finish()
            receipt_before = fixture.receipt.read_bytes()
            with self.assertRaises(issuer.QualificationReceiptError):
                fixture.finish()
            self.assertEqual(fixture.receipt.read_bytes(), receipt_before)

    def test_destination_race_cannot_overwrite_an_existing_file(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            destination = fixture.output / "raced.json"
            real_link = os.link

            def race(source: str, target: Path, **kwargs: Any) -> None:
                Path(target).write_bytes(b"racer-owned\n")
                Path(target).chmod(0o600)
                real_link(source, target, **kwargs)

            with mock.patch.object(issuer.os, "link", side_effect=race):
                with self.assertRaises(issuer.QualificationReceiptError):
                    issuer.write_atomic_create_only(destination.resolve(), {"safe": True})
            self.assertEqual(destination.read_bytes(), b"racer-owned\n")

    @unittest.skipUnless(os.name == "posix", "Unix custody modes required")
    def test_private_run_directory_and_gate_journal_modes_are_required(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            permissive = fixture.root / "permissive"
            permissive.mkdir(mode=0o755)
            permissive.chmod(0o755)
            with self.assertRaises(issuer.QualificationReceiptError):
                fixture.begin(snapshot=permissive / "snapshot.json")

            fixture.begin()
            fixture.passing_gates()
            fixture.gates.chmod(0o644)
            with self.assertRaises(issuer.QualificationReceiptError):
                fixture.finish()

    def test_evidence_files_must_share_one_private_run_directory(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.begin()
            fixture.passing_gates()
            other = fixture.root / "other-output"
            other.mkdir(mode=0o700)
            other.chmod(0o700)
            other_gates = other / "gates.tsv"
            other_gates.write_bytes(fixture.gates.read_bytes())
            other_gates.chmod(0o600)
            with self.assertRaises(issuer.QualificationReceiptError):
                issuer.finish(
                    fixture.repository.resolve(),
                    issuer.DEFAULT_CONTRACT,
                    fixture.gate_contract.resolve(),
                    fixture.snapshot.resolve(),
                    other_gates.resolve(),
                    fixture.receipt.resolve(),
                    fixture.commit,
                    "2026-09-02T12:00:00Z",
                    "passed",
                )

    def test_tracked_symbolic_link_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            link = fixture.repository / "tracked-link"
            try:
                link.symlink_to("src/lib.rs")
            except OSError:
                self.skipTest("symbolic links are unavailable")
            run_git(fixture.repository, "add", "tracked-link")
            run_git(fixture.repository, "commit", "-q", "-m", "add symlink")
            fixture.commit = run_git(fixture.repository, "rev-parse", "HEAD")
            with self.assertRaises(issuer.QualificationReceiptError):
                fixture.begin()

    def test_post_publication_source_change_removes_receipt(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.begin()
            fixture.passing_gates()
            original = issuer.collect_source_snapshot
            calls = 0

            def drifting_snapshot(*args: Any, **kwargs: Any) -> dict[str, Any]:
                nonlocal calls
                calls += 1
                value = original(*args, **kwargs)
                if calls >= 3:
                    value = json.loads(json.dumps(value))
                    value["source"]["status_sha256"] = "0" * 64
                return value

            with mock.patch.object(
                issuer, "collect_source_snapshot", side_effect=drifting_snapshot
            ):
                with self.assertRaises(issuer.QualificationReceiptError):
                    fixture.finish()
            self.assertFalse(fixture.receipt.exists())


if __name__ == "__main__":
    unittest.main()
