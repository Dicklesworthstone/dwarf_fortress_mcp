#!/usr/bin/env python3

from __future__ import annotations

import hashlib
import importlib.util
import json
import os
import stat
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from typing import Any

MODULE_PATH = Path(__file__).with_name("serve_admitted_live.py")
SPEC = importlib.util.spec_from_file_location("serve_admitted_live", MODULE_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load admitted live launcher")
launcher = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = launcher
SPEC.loader.exec_module(launcher)


def digest(label: str) -> str:
    return launcher.promotion.sha256_bytes(label.encode("utf-8"))


def file_digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def launch_record(binary_path: Path, metadata: os.stat_result) -> dict[str, Any]:
    return {
        "schema": launcher.LAUNCH_SCHEMA,
        "state": "authorized_to_exec",
        "bridge_protocol": "1.0",
        "compatibility_entry_id": digest("entry"),
        "required_entry_id": digest("entry"),
        "compatibility_decision_digest": digest("decision"),
        "compatibility_registry_digest": digest("registry"),
        "compatibility_floor": {
            "file_sha256": digest("floor-file"),
            "floor_digest": digest("floor-content"),
            "sequence": 7,
            "registry_file_sha256": digest("registry-file"),
            "registry_digest": digest("registry"),
            "entry_count": 1,
        },
        "support_level": "experimental",
        "deployment_manifest": {"version_tuple": {"protocol": "1.0"}},
        "server_receipt": {
            "file_sha256": digest("receipt-file"),
            "content_digest": digest("receipt-content"),
            "local_qualification_receipt_sha256": digest("local-receipt"),
        },
        "server_binary": {
            "path": os.fspath(binary_path),
            "sha256": file_digest(binary_path),
            "bytes": metadata.st_size,
            "device": metadata.st_dev,
            "inode": metadata.st_ino,
            "mode": stat.S_IMODE(metadata.st_mode),
            "owner_uid": metadata.st_uid,
        },
        "capabilities": ["doctor", "observe", "query", "wait"],
        "mode": "authenticated_live_read_only",
        "mutation_capabilities": [],
        "launch_digest": digest("launch"),
    }


class Fixture:
    def __init__(self, root: Path) -> None:
        self.root = root
        self.binary_path = root / "dwarf-fortress-mcp"
        self.binary_path.write_bytes(b"fixture server executable")
        self.binary_path.chmod(0o700)
        self.descriptor = os.open(self.binary_path, os.O_RDONLY)
        metadata = os.fstat(self.descriptor)
        self.opened = SimpleNamespace(
            descriptor=self.descriptor,
            path=self.binary_path,
            sha256=file_digest(self.binary_path),
            size=metadata.st_size,
            device=metadata.st_dev,
            inode=metadata.st_ino,
            mode=stat.S_IMODE(metadata.st_mode),
            owner_uid=metadata.st_uid,
        )
        self.record = launch_record(self.binary_path, metadata)

    def close(self) -> None:
        try:
            os.close(self.descriptor)
        except OSError:
            pass


class LiveAdmissionTicketTests(unittest.TestCase):
    def fixture(self) -> tuple[tempfile.TemporaryDirectory[str], Fixture]:
        temporary = tempfile.TemporaryDirectory()
        return temporary, Fixture(Path(temporary.name))

    def test_ticket_fields_and_digest_are_deterministic(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            try:
                identifier = digest("ticket")
                first = launcher.build_admission_ticket(
                    fixture.record,
                    fixture.opened,
                    now_unix_seconds=1_800_000_000,
                    process_id=1234,
                    ticket_id=identifier,
                )
                second = launcher.build_admission_ticket(
                    fixture.record,
                    fixture.opened,
                    now_unix_seconds=1_800_000_000,
                    process_id=1234,
                    ticket_id=identifier,
                )
                self.assertEqual(first, second)
                self.assertEqual(first["schema"], launcher.TICKET_SCHEMA)
                self.assertEqual(first["process_id"], 1234)
                self.assertEqual(first["bridge_protocol"], "1.0")
                self.assertEqual(first["compatibility_floor_sequence"], 7)
                self.assertEqual(
                    first["compatibility_floor_digest"],
                    fixture.record["compatibility_floor"]["floor_digest"],
                )
                self.assertEqual(first["mutation_capabilities"], [])
                unsigned = dict(first)
                declared = unsigned.pop("ticket_digest")
                self.assertEqual(
                    declared,
                    launcher.promotion.sha256_bytes(
                        launcher.promotion.canonical_json(unsigned)
                    ),
                )
            finally:
                fixture.close()

    def test_ticket_file_and_directory_are_owner_private(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            try:
                directory = fixture.root / launcher.TICKET_DIRECTORY_NAME
                path = launcher.write_admission_ticket(
                    directory, fixture.record, fixture.opened
                )
                directory_metadata = os.lstat(directory)
                ticket_metadata = os.lstat(path)
                self.assertEqual(stat.S_IMODE(directory_metadata.st_mode), 0o700)
                self.assertEqual(stat.S_IMODE(ticket_metadata.st_mode), 0o600)
                self.assertFalse(stat.S_ISLNK(ticket_metadata.st_mode))
                value = json.loads(path.read_text(encoding="utf-8"))
                self.assertEqual(value["process_id"], os.getpid())
                self.assertEqual(value["bridge_protocol"], "1.0")
                self.assertEqual(value["server_binary_inode"], fixture.opened.inode)
                self.assertEqual(value["compatibility_floor_sequence"], 7)
                self.assertNotIn("DFMCP_BRIDGE_TOKEN", path.read_text(encoding="utf-8"))
            finally:
                fixture.close()

    def test_admitted_environment_binds_protocol_floor_ticket_and_secret_only_in_environment(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            try:
                path = launcher.write_admission_ticket(
                    fixture.root / launcher.TICKET_DIRECTORY_NAME,
                    fixture.record,
                    fixture.opened,
                )
                environment = launcher.admitted_environment(
                    {"DFMCP_BRIDGE_TOKEN": "x" * 32}, fixture.record, path
                )
                self.assertEqual(environment["DFMCP_BRIDGE_TOKEN"], "x" * 32)
                self.assertEqual(environment["DFMCP_ADMISSION_TICKET"], os.fspath(path))
                self.assertEqual(
                    environment[launcher.ADMITTED_BRIDGE_PROTOCOL_ENVIRONMENT], "1.0"
                )
                self.assertEqual(
                    environment["DFMCP_COMPATIBILITY_ENTRY_ID"],
                    fixture.record["compatibility_entry_id"],
                )
                self.assertEqual(
                    environment["DFMCP_COMPATIBILITY_FLOOR_DIGEST"],
                    fixture.record["compatibility_floor"]["floor_digest"],
                )
                self.assertEqual(
                    environment["DFMCP_COMPATIBILITY_FLOOR_SEQUENCE"], "7"
                )
                self.assertNotIn("x" * 32, path.read_text(encoding="utf-8"))
            finally:
                fixture.close()

    def test_legacy_or_mismatched_protocol_records_are_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            try:
                legacy = dict(fixture.record)
                legacy["schema"] = "dfmcp.admitted-live-launch/1"
                with self.assertRaises(launcher.LaunchError):
                    launcher.build_admission_ticket(legacy, fixture.opened)

                mismatched = dict(fixture.record)
                mismatched["deployment_manifest"] = {
                    "version_tuple": {"protocol": "1.1"}
                }
                with self.assertRaises(launcher.LaunchError):
                    launcher.build_admission_ticket(mismatched, fixture.opened)

                unsupported = dict(fixture.record)
                unsupported["bridge_protocol"] = "1.1"
                unsupported["deployment_manifest"] = {
                    "version_tuple": {"protocol": "1.1"}
                }
                with self.assertRaises(launcher.LaunchError):
                    launcher.build_admission_ticket(unsupported, fixture.opened)
            finally:
                fixture.close()

    def test_permissive_existing_ticket_directory_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            try:
                directory = fixture.root / launcher.TICKET_DIRECTORY_NAME
                directory.mkdir(mode=0o755)
                directory.chmod(0o755)
                with self.assertRaises(launcher.LaunchError):
                    launcher.write_admission_ticket(
                        directory, fixture.record, fixture.opened
                    )
            finally:
                fixture.close()

    def test_noncanonical_owner_only_ticket_directory_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            try:
                directory = fixture.root / launcher.TICKET_DIRECTORY_NAME
                directory.mkdir(mode=0o500)
                directory.chmod(0o500)
                with self.assertRaises(launcher.LaunchError):
                    launcher.write_admission_ticket(
                        directory, fixture.record, fixture.opened
                    )
            finally:
                directory.chmod(0o700)
                fixture.close()

    def test_executable_metadata_drift_is_rejected_before_ticket_issue(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            try:
                fixture.binary_path.write_bytes(b"changed executable bytes")
                with self.assertRaises(launcher.LaunchError):
                    launcher.build_admission_ticket(fixture.record, fixture.opened)
            finally:
                fixture.close()

    def test_same_size_executable_byte_drift_is_rejected_before_ticket_issue(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            try:
                replacement = b"fixture server executablE"
                self.assertEqual(len(replacement), fixture.binary_path.stat().st_size)
                fixture.binary_path.write_bytes(replacement)
                fixture.binary_path.chmod(0o700)
                with self.assertRaises(launcher.LaunchError):
                    launcher.build_admission_ticket(fixture.record, fixture.opened)
            finally:
                fixture.close()


if __name__ == "__main__":
    unittest.main()
