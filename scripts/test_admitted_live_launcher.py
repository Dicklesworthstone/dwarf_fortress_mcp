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

    def test_exact_admitted_chain_binds_opened_inode_and_receipts(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            try:
                opened, record = fixture.prepare()
                self.assertEqual(record["state"], "authorized_to_exec")
                self.assertEqual(record["compatibility_entry_id"], fixture.entry["entry_id"])
                self.assertEqual(record["required_entry_id"], fixture.entry["entry_id"])
                self.assertEqual(
                    record["compatibility_registry_digest"],
                    promotion.sha256_bytes(promotion.canonical_json(fixture.registry)),
                )
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

    def test_required_entry_fence_is_mandatory_and_exact(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            with mock.patch.object(launcher.binary_verifier, "verify", side_effect=fixture.fake_verify):
                with self.assertRaises(launcher.LaunchError):
                    launcher.prepare_launch(
                        fixture.registry_path,
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

    def test_admitted_environment_contains_proof_bindings_but_preserves_secret_only_in_environment(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            try:
                _, record = fixture.prepare()
                environment = launcher.admitted_environment(fixture.environment(), record)
                self.assertEqual(environment["DFMCP_BRIDGE_TOKEN"], "x" * 32)
                self.assertEqual(
                    environment["DFMCP_COMPATIBILITY_ENTRY_ID"], fixture.entry["entry_id"]
                )
                self.assertEqual(
                    environment["DFMCP_SERVER_RECEIPT_DIGEST"],
                    fixture.normalized["receipt_digest"],
                )
                self.assertNotIn("x" * 32, json.dumps(record))
            finally:
                fixture.close_all()

    def test_no_path_fallback_when_descriptor_exec_is_unsupported(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            try:
                opened, _ = fixture.prepare()
                with mock.patch.object(launcher.os, "supports_fd", set()):
                    with self.assertRaises(launcher.LaunchError):
                        launcher.execute_verified_descriptor(opened, fixture.environment())
            finally:
                fixture.close_all()

    def test_cli_requires_receipts_commit_and_entry_fences(self) -> None:
        with mock.patch.object(sys, "stderr"):
            with self.assertRaises(SystemExit):
                launcher.parse_args(["manifest.json", "--launch-record", "launch.json"])


if __name__ == "__main__":
    unittest.main()
