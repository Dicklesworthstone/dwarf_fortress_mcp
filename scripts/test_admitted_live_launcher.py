#!/usr/bin/env python3

from __future__ import annotations

import copy
import hashlib
import importlib.util
import json
import os
import platform
import stat
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from typing import Any
from unittest import mock

MODULE_PATH = Path(__file__).with_name("serve_admitted_live.py")
SPEC = importlib.util.spec_from_file_location("serve_admitted_live", MODULE_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load admitted live launcher")
launcher = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = launcher
SPEC.loader.exec_module(launcher)

promotion = launcher.promotion
resolver = launcher.resolver


def digest(label: str) -> str:
    return hashlib.sha256(label.encode()).hexdigest()


def deployment_manifest() -> dict[str, Any]:
    return {
        "schema": resolver.MANIFEST_SCHEMA,
        "version_tuple": {
            "dwarf_fortress": "0.51.11",
            "dfhack": "0.51.11-r1",
            "bridge": "0.1.0",
            "protocol": "1.0",
        },
        "platform": {"system": platform.system(), "machine": platform.machine()},
        "source": {
            "dfmcp_commit": "1" * 40,
            "dfhack_commit": "2" * 40,
            "plugin_sha256": digest("plugin"),
        },
    }


def compatibility_entry() -> dict[str, Any]:
    manifest = deployment_manifest()
    unsigned: dict[str, Any] = {
        "support_level": "experimental",
        "version_tuple": manifest["version_tuple"],
        "platform": manifest["platform"],
        "source": {
            **manifest["source"],
            "dfmcp_dirty": False,
            "native_build_receipt_sha256": digest("native-receipt"),
            "live_acceptance_receipt_sha256": digest("live-receipt-file"),
            "live_acceptance_receipt_digest": digest("live-receipt-content"),
        },
        "gates": [
            {
                "gate": "R1",
                "status": "passed",
                "receipt_sha256": digest("native-receipt"),
            },
            *[
                {
                    "gate": gate,
                    "status": "passed",
                    "case_count": count,
                    "evidence_digest": digest(f"gate-{gate}"),
                }
                for gate, count in promotion.EXPECTED_LIVE_CASE_COUNTS.items()
            ],
        ],
        "capabilities": promotion.READ_ONLY_CAPABILITIES,
        "mutation_capabilities": [],
        "observed_domains": promotion.OBSERVED_DOMAINS,
        "conditional_domains": promotion.CONDITIONAL_DOMAINS,
        "omitted_domains": promotion.OMITTED_DOMAINS,
        "evidence_locator": "qualification/fixture/receipt.json",
        "limitations": promotion.LIMITATIONS,
    }
    return {"entry_id": promotion.sha256_bytes(promotion.canonical_json(unsigned)), **unsigned}


class Fixture:
    def __init__(self, root: Path) -> None:
        self.root = root
        self.registry_path = root / "registry.json"
        self.floor_directory = root / "floor"
        self.floor_directory.mkdir(mode=0o700)
        self.floor_directory.chmod(0o700)
        self.floor_path = self.floor_directory / "compatibility-floor.json"
        self.manifest_path = root / "manifest.json"
        self.binary_path = root / "dwarf-fortress-mcp"
        self.server_receipt_path = root / "server-receipt.json"
        self.local_receipt_path = root / "local-receipt.json"
        self.contract_path = root / "binary-contract.json"
        self.source_root = root / "source"
        self.source_root.mkdir()
        self.entry = compatibility_entry()
        self.registry = {
            "schema_version": promotion.REGISTRY_SCHEMA,
            "status": "admitted_live_tuples",
            "entries": [self.entry],
        }
        self.registry_path.write_text(json.dumps(self.registry, sort_keys=True) + "\n")
        launcher.compatibility_floor.initialize_floor(
            self.floor_path, self.registry_path
        )
        self.manifest_path.write_text(json.dumps(deployment_manifest(), sort_keys=True) + "\n")
        self.binary_path.write_bytes(b"fixture-executable")
        self.binary_path.chmod(0o700)
        self.server_receipt_path.write_text("{}\n")
        self.local_receipt_path.write_text("{}\n")
        self.contract_path.write_text("{}\n")
        self.opened_descriptors: list[int] = []
        self.normalized = {
            "receipt_sha256": digest("server-receipt-file"),
            "receipt_digest": digest("server-receipt-content"),
            "source": {
                "dfmcp_commit": deployment_manifest()["source"]["dfmcp_commit"],
                "dfmcp_dirty": False,
                "local_qualification_receipt_sha256": digest("local-receipt"),
            },
            "platform": deployment_manifest()["platform"],
            "binary": {
                "name": "dwarf-fortress-mcp",
                "profile": "release",
                "relative_path": "target/release/dwarf-fortress-mcp",
                "bytes": self.binary_path.stat().st_size,
                "sha256": self.binary_sha256(),
            },
            "executable_checks": [],
            "source_digests": {},
            "mutation_capabilities": [],
        }

    def binary_sha256(self) -> str:
        return hashlib.sha256(self.binary_path.read_bytes()).hexdigest()

    def environment(self) -> dict[str, str]:
        return {"PATH": os.environ.get("PATH", ""), "DFMCP_BRIDGE_TOKEN": "x" * 32}

    def fake_verify(self, *args: Any, **kwargs: Any) -> tuple[dict[str, Any], Any]:
        descriptor = os.open(self.binary_path, os.O_RDONLY)
        self.opened_descriptors.append(descriptor)
        metadata = os.fstat(descriptor)
        opened = SimpleNamespace(
            descriptor=descriptor,
            path=self.binary_path,
            sha256=self.binary_sha256(),
            size=metadata.st_size,
            device=metadata.st_dev,
            inode=metadata.st_ino,
            mode=stat.S_IMODE(metadata.st_mode),
            owner_uid=metadata.st_uid,
        )
        return copy.deepcopy(self.normalized), opened

    def prepare(self, environment: dict[str, str] | None = None) -> tuple[Any, dict[str, Any]]:
        with mock.patch.object(launcher.binary_verifier, "verify", side_effect=self.fake_verify):
            return launcher.prepare_launch(
                self.registry_path,
                self.floor_path,
                self.manifest_path,
                self.binary_path,
                self.server_receipt_path,
                self.local_receipt_path,
                self.contract_path,
                self.source_root,
                deployment_manifest()["source"]["dfmcp_commit"],
                self.entry["entry_id"],
                self.environment() if environment is None else environment,
            )

    def close_all(self) -> None:
        for descriptor in self.opened_descriptors:
            try:
                os.close(descriptor)
            except OSError:
                pass


class AdmittedLiveLauncherTests(unittest.TestCase):
    def fixture(self) -> tuple[tempfile.TemporaryDirectory[str], Fixture]:
        temporary = tempfile.TemporaryDirectory()
        return temporary, Fixture(Path(temporary.name))

    def test_exact_admitted_chain_binds_protocol_floor_inode_and_receipts(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            try:
                opened, record = fixture.prepare()
                floor_value, floor_file_sha256 = launcher.compatibility_floor.read_floor(
                    fixture.floor_path
                )
                self.assertEqual(record["schema"], launcher.LAUNCH_SCHEMA)
                self.assertEqual(record["state"], "authorized_to_exec")
                self.assertEqual(record["bridge_protocol"], "1.0")
                self.assertEqual(
                    record["bridge_protocol"],
                    record["deployment_manifest"]["version_tuple"]["protocol"],
                )
                self.assertEqual(record["compatibility_entry_id"], fixture.entry["entry_id"])
                self.assertEqual(record["required_entry_id"], fixture.entry["entry_id"])
                self.assertEqual(
                    record["compatibility_registry_digest"],
                    promotion.sha256_bytes(promotion.canonical_json(fixture.registry)),
                )
                self.assertEqual(
                    record["compatibility_floor"]["floor_digest"],
                    floor_value["floor_digest"],
                )
                self.assertEqual(
                    record["compatibility_floor"]["file_sha256"],
                    floor_file_sha256,
                )
                self.assertEqual(record["compatibility_floor"]["sequence"], 0)
                self.assertEqual(record["server_binary"]["inode"], opened.inode)
                self.assertEqual(record["server_binary"]["device"], opened.device)
                self.assertEqual(
                    record["server_receipt"]["file_sha256"],
                    fixture.normalized["receipt_sha256"],
                )
                self.assertNotIn("DFMCP_BRIDGE_TOKEN", json.dumps(record))
                unsigned = dict(record)
                del unsigned["launch_digest"]
                self.assertEqual(
                    record["launch_digest"],
                    promotion.sha256_bytes(promotion.canonical_json(unsigned)),
                )
            finally:
                fixture.close_all()

    def test_missing_or_short_token_is_rejected_before_binary_verification(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            with self.assertRaises(launcher.LaunchError):
                fixture.prepare({"PATH": os.environ.get("PATH", "")})
            with self.assertRaises(launcher.LaunchError):
                fixture.prepare({"DFMCP_BRIDGE_TOKEN": "short"})
            self.assertEqual(fixture.opened_descriptors, [])

    def test_loader_injection_environment_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            environment = fixture.environment()
            environment["LD_PRELOAD"] = "/tmp/injected.so"
            with self.assertRaises(launcher.LaunchError):
                fixture.prepare(environment)
            self.assertEqual(fixture.opened_descriptors, [])

    def test_registry_floor_mismatch_is_rejected_before_binary_verification(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.registry_path.write_text(
                json.dumps(fixture.registry, sort_keys=True, indent=2) + "\n"
            )
            with self.assertRaises(launcher.compatibility_floor.FloorError):
                fixture.prepare()
            self.assertEqual(fixture.opened_descriptors, [])

    def test_permissive_floor_custody_is_rejected_before_binary_verification(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.floor_path.chmod(0o640)
            with self.assertRaises(launcher.compatibility_floor.FloorError):
                fixture.prepare()
            self.assertEqual(fixture.opened_descriptors, [])

    def test_required_entry_fence_is_mandatory_and_exact(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            with mock.patch.object(launcher.binary_verifier, "verify", side_effect=fixture.fake_verify):
                with self.assertRaises(launcher.LaunchError):
                    launcher.prepare_launch(
                        fixture.registry_path,
                        fixture.floor_path,
                        fixture.manifest_path,
                        fixture.binary_path,
                        fixture.server_receipt_path,
                        fixture.local_receipt_path,
                        fixture.contract_path,
                        fixture.source_root,
                        deployment_manifest()["source"]["dfmcp_commit"],
                        digest("wrong-entry"),
                        fixture.environment(),
                    )
            self.assertEqual(fixture.opened_descriptors, [])

    def test_unadmitted_protocol_is_rejected_before_binary_verification(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            manifest = deployment_manifest()
            manifest["version_tuple"]["protocol"] = "1.1"
            fixture.manifest_path.write_text(json.dumps(manifest, sort_keys=True) + "\n")
            with self.assertRaises(
                (launcher.LaunchError, launcher.resolver.ResolutionError)
            ):
                fixture.prepare()
            self.assertEqual(fixture.opened_descriptors, [])
            with self.assertRaises(launcher.LaunchError):
                launcher.validate_admitted_bridge_protocol(
                    "1.1", "test.bridge_protocol"
                )

    def test_launch_protocol_mismatch_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            try:
                _, record = fixture.prepare()
                record["deployment_manifest"]["version_tuple"]["protocol"] = "1.1"
                with self.assertRaises(launcher.LaunchError):
                    launcher.launch_bridge_protocol(record)
            finally:
                fixture.close_all()

    def test_server_receipt_source_mismatch_closes_opened_descriptor(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.normalized["source"]["dfmcp_commit"] = "3" * 40
            with self.assertRaises(launcher.LaunchError):
                fixture.prepare()
            descriptor = fixture.opened_descriptors[-1]
            with self.assertRaises(OSError):
                os.fstat(descriptor)

    def test_server_receipt_platform_mismatch_closes_opened_descriptor(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.normalized["platform"] = {"system": "Other", "machine": "other"}
            with self.assertRaises(launcher.LaunchError):
                fixture.prepare()
            with self.assertRaises(OSError):
                os.fstat(fixture.opened_descriptors[-1])

    def test_mutation_capability_contamination_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.normalized["mutation_capabilities"] = ["pause"]
            with self.assertRaises(launcher.LaunchError):
                fixture.prepare()

    def test_generation_change_after_prepare_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            try:
                _, record = fixture.prepare()
                fixture.registry_path.write_text(
                    json.dumps(fixture.registry, sort_keys=True, indent=2) + "\n"
                )
                with self.assertRaises(launcher.compatibility_floor.FloorError):
                    launcher.reverify_launch_generation(
                        fixture.registry_path, fixture.floor_path, record
                    )
            finally:
                fixture.close_all()

    def test_same_size_binary_mutation_is_detected_before_exec(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            try:
                opened, record = fixture.prepare()
                replacement = b"fixture-executablE"
                self.assertEqual(len(replacement), fixture.binary_path.stat().st_size)
                fixture.binary_path.write_bytes(replacement)
                fixture.binary_path.chmod(0o700)
                with self.assertRaises(launcher.LaunchError):
                    launcher.reverify_opened_binary(opened, record)
            finally:
                fixture.close_all()

    def test_admitted_environment_contains_protocol_floor_and_receipt_bindings(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            try:
                _, record = fixture.prepare()
                ticket_path = fixture.root / "ticket.json"
                environment = launcher.admitted_environment(
                    fixture.environment(), record, ticket_path
                )
                self.assertEqual(environment["DFMCP_BRIDGE_TOKEN"], "x" * 32)
                self.assertEqual(
                    environment[launcher.ADMITTED_BRIDGE_PROTOCOL_ENVIRONMENT], "1.0"
                )
                self.assertEqual(
                    environment["DFMCP_COMPATIBILITY_ENTRY_ID"], fixture.entry["entry_id"]
                )
                self.assertEqual(
                    environment["DFMCP_COMPATIBILITY_FLOOR_DIGEST"],
                    record["compatibility_floor"]["floor_digest"],
                )
                self.assertEqual(
                    environment["DFMCP_COMPATIBILITY_FLOOR_SEQUENCE"], "0"
                )
                self.assertEqual(
                    environment["DFMCP_SERVER_RECEIPT_DIGEST"],
                    fixture.normalized["receipt_digest"],
                )
                self.assertEqual(
                    environment["DFMCP_ADMISSION_TICKET"], os.fspath(ticket_path)
                )
                self.assertNotIn("x" * 32, json.dumps(record))
            finally:
                fixture.close_all()

    def test_no_path_fallback_when_descriptor_exec_is_unsupported(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            try:
                opened, record = fixture.prepare()
                environment = launcher.admitted_environment(
                    fixture.environment(), record, fixture.root / "ticket.json"
                )
                with mock.patch.object(launcher.os, "supports_fd", set()):
                    with self.assertRaises(launcher.LaunchError):
                        launcher.execute_verified_descriptor(
                            opened, environment, record
                        )
            finally:
                fixture.close_all()

    def test_cli_requires_floor_receipts_commit_and_entry_fences(self) -> None:
        with mock.patch.object(sys, "stderr"):
            with self.assertRaises(SystemExit):
                launcher.parse_args(["manifest.json", "--launch-record", "launch.json"])


if __name__ == "__main__":
    unittest.main()
