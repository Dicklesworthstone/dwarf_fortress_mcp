#!/usr/bin/env python3

from __future__ import annotations

import copy
import hashlib
import importlib.util
import json
import os
import stat
import sys
import tempfile
import unittest
from pathlib import Path
from typing import Any

JOURNAL_PATH = Path(__file__).with_name("live_announcement_evidence_journal.py")
JOURNAL_SPEC = importlib.util.spec_from_file_location(
    "live_announcement_evidence_journal", JOURNAL_PATH
)
if JOURNAL_SPEC is None or JOURNAL_SPEC.loader is None:
    raise RuntimeError("cannot load live announcement evidence journal")
journal = importlib.util.module_from_spec(JOURNAL_SPEC)
sys.modules[JOURNAL_SPEC.name] = journal
JOURNAL_SPEC.loader.exec_module(journal)

ACCEPTANCE_TEST_PATH = Path(__file__).with_name("test_live_announcement_acceptance.py")
ACCEPTANCE_TEST_SPEC = importlib.util.spec_from_file_location(
    "test_live_announcement_acceptance_fixture", ACCEPTANCE_TEST_PATH
)
if ACCEPTANCE_TEST_SPEC is None or ACCEPTANCE_TEST_SPEC.loader is None:
    raise RuntimeError("cannot load live announcement acceptance fixture")
acceptance_fixture = importlib.util.module_from_spec(ACCEPTANCE_TEST_SPEC)
sys.modules[ACCEPTANCE_TEST_SPEC.name] = acceptance_fixture
ACCEPTANCE_TEST_SPEC.loader.exec_module(acceptance_fixture)


def file_sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


class Fixture:
    def __init__(self, root: Path) -> None:
        self.root = root
        self.base = acceptance_fixture.Fixture(root)
        self.journal_path = root / "private" / "campaign.json"
        self.stream_path = root / "private" / "events.jsonl"
        self.assertions_path = root / "assertions.json"
        self.artifact_dir = root / "artifacts"
        self.artifact_dir.mkdir()

    @property
    def native_path(self) -> Path:
        return self.base.native_path

    @property
    def contract_path(self) -> Path:
        return self.base.contract_path

    @property
    def contract(self) -> dict[str, Any]:
        return self.base.contract

    def initialize(self) -> dict[str, Any]:
        return journal.initialize(
            self.journal_path,
            self.native_path,
            self.contract_path,
            journal.DEFAULT_JOURNAL_CONTRACT,
            "0.51.11",
            "0.51.11-r1",
            "Linux",
            "x86_64",
        )

    def next_case(self, captured: int) -> tuple[str, dict[str, Any]]:
        return journal.expected_case_list(self.contract)[captured]

    def write_assertions(self, value: dict[str, Any]) -> None:
        self.assertions_path.write_text(
            json.dumps(value, sort_keys=True) + "\n", encoding="utf-8"
        )

    def artifact_arguments(self, captured: int) -> list[str]:
        _gate, case = self.next_case(captured)
        arguments = []
        for name in case["required_artifact_digests"]:
            path = self.artifact_dir / f"{captured:02d}-{name}.bin"
            path.write_bytes(f"artifact:{captured}:{name}".encode())
            arguments.append(f"{name}={path}")
        return arguments

    def append_next(
        self,
        expected_sha: str | None = None,
        assertions: dict[str, Any] | None = None,
        artifact_arguments: list[str] | None = None,
    ) -> dict[str, Any]:
        current = json.loads(self.journal_path.read_text(encoding="utf-8"))
        captured = len(current["events"])
        _gate, case = self.next_case(captured)
        self.write_assertions(
            copy.deepcopy(case["required_equals"])
            if assertions is None
            else assertions
        )
        return journal.append_event(
            self.journal_path,
            self.native_path,
            self.contract_path,
            file_sha256(self.journal_path) if expected_sha is None else expected_sha,
            self.assertions_path,
            self.artifact_arguments(captured)
            if artifact_arguments is None
            else artifact_arguments,
            [],
        )

    def complete(self) -> dict[str, Any]:
        value = json.loads(self.journal_path.read_text(encoding="utf-8"))
        total = len(journal.expected_case_list(self.contract))
        while len(value["events"]) < total:
            value = self.append_next()
        return value


class LiveAnnouncementEvidenceJournalTests(unittest.TestCase):
    def fixture(self) -> tuple[tempfile.TemporaryDirectory[str], Fixture]:
        temporary = tempfile.TemporaryDirectory()
        return temporary, Fixture(Path(temporary.name))

    def test_initialize_is_owner_private_and_reports_exact_next_case(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            value = fixture.initialize()
            self.assertEqual(value["status"], "capturing")
            self.assertEqual(value["next_sequence"], 1)
            self.assertEqual(stat.S_IMODE(fixture.journal_path.stat().st_mode), 0o600)
            status = journal.next_case_status(
                fixture.journal_path,
                fixture.native_path,
                fixture.contract_path,
            )
            self.assertEqual(status["next_case"]["gate"], "A1")
            self.assertEqual(status["next_case"]["case"], "native_receipt_bound")
            self.assertEqual(status["journal_file_sha256"], file_sha256(fixture.journal_path))

    def test_exact_append_advances_once_and_stale_cas_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.initialize()
            stale = file_sha256(fixture.journal_path)
            updated = fixture.append_next(expected_sha=stale)
            self.assertEqual(updated["next_sequence"], 2)
            with self.assertRaises(journal.JournalError):
                fixture.append_next(expected_sha=stale)
            persisted = json.loads(fixture.journal_path.read_text(encoding="utf-8"))
            self.assertEqual(len(persisted["events"]), 1)

    def test_wrong_assertions_do_not_mutate_journal(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.initialize()
            before = fixture.journal_path.read_bytes()
            wrong = copy.deepcopy(fixture.next_case(0)[1]["required_equals"])
            wrong["native_receipt_valid"] = False
            with self.assertRaises(journal.JournalError):
                fixture.append_next(assertions=wrong)
            self.assertEqual(fixture.journal_path.read_bytes(), before)

    def test_artifact_order_zero_digest_and_symlink_are_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.initialize()
            fixture.append_next()
            captured = 1
            arguments = fixture.artifact_arguments(captured)
            self.assertGreater(len(arguments), 1)
            with self.assertRaises(journal.JournalError):
                fixture.append_next(artifact_arguments=list(reversed(arguments)))

            _gate, case = fixture.next_case(captured)
            fixture.write_assertions(copy.deepcopy(case["required_equals"]))
            with self.assertRaises(journal.JournalError):
                journal.append_event(
                    fixture.journal_path,
                    fixture.native_path,
                    fixture.contract_path,
                    file_sha256(fixture.journal_path),
                    fixture.assertions_path,
                    [],
                    [
                        f"{name}={'0' * 64}"
                        for name in case["required_artifact_digests"]
                    ],
                )

            target = fixture.artifact_dir / "target.bin"
            target.write_bytes(b"target")
            link = fixture.artifact_dir / "link.bin"
            try:
                link.symlink_to(target)
            except OSError:
                self.skipTest("symbolic links are unavailable")
            names = case["required_artifact_digests"]
            symlink_arguments = [
                f"{names[0]}={link}",
                *[
                    f"{name}={fixture.artifact_dir / f'{name}.bin'}"
                    for name in names[1:]
                ],
            ]
            for raw in symlink_arguments[1:]:
                Path(raw.split("=", 1)[1]).write_bytes(b"artifact")
            with self.assertRaises(journal.JournalError):
                fixture.append_next(artifact_arguments=symlink_arguments)

    def test_incomplete_export_is_rejected_without_creating_output(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.initialize()
            with self.assertRaises(journal.JournalError):
                journal.export_stream(
                    fixture.journal_path,
                    fixture.native_path,
                    fixture.contract_path,
                    file_sha256(fixture.journal_path),
                    fixture.stream_path,
                )
            self.assertFalse(fixture.stream_path.exists())

    def test_complete_export_revalidates_to_the_same_receipt(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.initialize()
            complete = fixture.complete()
            self.assertEqual(complete["status"], "complete")
            result = journal.export_stream(
                fixture.journal_path,
                fixture.native_path,
                fixture.contract_path,
                file_sha256(fixture.journal_path),
                fixture.stream_path,
            )
            receipt = journal.verifier.verify(
                fixture.stream_path,
                fixture.native_path,
                fixture.contract_path,
            )
            self.assertEqual(
                result["acceptance_receipt_digest"], receipt["receipt_digest"]
            )
            self.assertEqual(result["event_count"], len(complete["events"]))
            self.assertEqual(stat.S_IMODE(fixture.stream_path.stat().st_mode), 0o600)

    def test_journal_digest_and_event_order_tampering_fail_closed(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.initialize()
            fixture.append_next()
            value = json.loads(fixture.journal_path.read_text(encoding="utf-8"))
            value["journal_digest"] = hashlib.sha256(b"wrong").hexdigest()
            fixture.journal_path.write_text(json.dumps(value) + "\n", encoding="utf-8")
            fixture.journal_path.chmod(0o600)
            with self.assertRaises(journal.JournalError):
                journal.next_case_status(
                    fixture.journal_path,
                    fixture.native_path,
                    fixture.contract_path,
                )

            fixture.journal_path.unlink()
            fixture.initialize()
            first = fixture.append_next()
            tampered = copy.deepcopy(first)
            tampered["events"][0]["sequence"] = 2
            tampered["journal_digest"] = journal.journal_digest(tampered)
            fixture.journal_path.write_text(json.dumps(tampered) + "\n", encoding="utf-8")
            fixture.journal_path.chmod(0o600)
            with self.assertRaises(journal.JournalError):
                journal.next_case_status(
                    fixture.journal_path,
                    fixture.native_path,
                    fixture.contract_path,
                )

    def test_existing_journal_and_concurrent_lock_are_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.initialize()
            with self.assertRaises(journal.JournalError):
                fixture.initialize()
            with journal.journal_lock(fixture.journal_path):
                with self.assertRaises(journal.JournalError):
                    with journal.journal_lock(fixture.journal_path):
                        self.fail("second journal lock unexpectedly acquired")

    def test_permissive_journal_mode_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.initialize()
            fixture.journal_path.chmod(0o640)
            with self.assertRaises(journal.JournalError):
                journal.next_case_status(
                    fixture.journal_path,
                    fixture.native_path,
                    fixture.contract_path,
                )

    def test_status_output_is_deterministic_for_identical_state(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.initialize()
            first = journal.next_case_status(
                fixture.journal_path,
                fixture.native_path,
                fixture.contract_path,
            )
            second = journal.next_case_status(
                fixture.journal_path,
                fixture.native_path,
                fixture.contract_path,
            )
            self.assertEqual(first, second)


if __name__ == "__main__":
    unittest.main()
