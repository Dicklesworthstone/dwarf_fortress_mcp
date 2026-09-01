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

MODULE_PATH = Path(__file__).with_name("doctor_live_admission.py")
SPEC = importlib.util.spec_from_file_location("doctor_live_admission", MODULE_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load live admission doctor")
doctor = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = doctor
SPEC.loader.exec_module(doctor)
promotion = doctor.promotion
resolver = doctor.resolver


def digest(label: str) -> str:
    return hashlib.sha256(label.encode()).hexdigest()


def manifest() -> dict[str, Any]:
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


def entry() -> dict[str, Any]:
    deployment = manifest()
    unsigned: dict[str, Any] = {
        "support_level": "experimental",
        "version_tuple": deployment["version_tuple"],
        "platform": deployment["platform"],
        "source": {
            **deployment["source"],
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
        self.floor_directory = root / "floor"
        self.floor_directory.mkdir(mode=0o700)
        self.floor_directory.chmod(0o700)
        self.floor_path = self.floor_directory / "compatibility-floor.json"
        self.binary_path = root / "dwarf-fortress-mcp"
        self.server_receipt_path = root / "server-receipt.json"
        self.local_receipt_path = root / "local-receipt.json"
        self.contract_path = root / "binary-contract.json"
        self.source_root = root / "source"
        self.source_root.mkdir()
        self.registry = {
            "schema_version": promotion.REGISTRY_SCHEMA,
            "status": "admitted_live_tuples",
            "entries": [entry()],
        }
        self.registry_path.write_text(json.dumps(self.registry, sort_keys=True) + "\n")
        self.manifest_path.write_text(json.dumps(manifest(), sort_keys=True) + "\n")
        doctor.compatibility_floor.initialize_floor(self.floor_path, self.registry_path)
        self.binary_path.write_bytes(b"doctor fixture executable")
        self.binary_path.chmod(0o700)
        self.server_receipt_path.write_text("{}\n")
        self.local_receipt_path.write_text("{}\n")
        self.contract_path.write_text("{}\n")
        self.opened_descriptors: list[int] = []
        self.normalized = {
            "receipt_sha256": digest("server-receipt-file"),
            "receipt_digest": digest("server-receipt-content"),
            "source": {
                "dfmcp_commit": manifest()["source"]["dfmcp_commit"],
                "dfmcp_dirty": False,
                "local_qualification_receipt_sha256": digest("local-receipt"),
            },
            "platform": manifest()["platform"],
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

    def diagnose(self, *, artifact: bool = False, entry_id: str | None = None) -> dict[str, Any]:
        kwargs: dict[str, Any] = {}
        if artifact:
            kwargs = {
                "binary_path": self.binary_path,
                "server_receipt_path": self.server_receipt_path,
                "local_qualification_receipt": self.local_receipt_path,
                "binary_contract_path": self.contract_path,
                "source_root": self.source_root,
                "expected_dfmcp_commit": manifest()["source"]["dfmcp_commit"],
            }
        with mock.patch.object(doctor.binary_verifier, "verify", side_effect=self.fake_verify):
            return doctor.diagnose(
                self.manifest_path,
                self.registry_path,
                self.floor_path,
                entry()["entry_id"] if entry_id is None else entry_id,
                **kwargs,
            )


class LiveAdmissionDoctorTests(unittest.TestCase):
    def fixture(self) -> tuple[tempfile.TemporaryDirectory[str], Fixture]:
        temporary = tempfile.TemporaryDirectory()
        return temporary, Fixture(Path(temporary.name))

    def test_compatibility_ready_report_is_deterministic_and_digest_bound(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            first = fixture.diagnose()
            second = fixture.diagnose()
            self.assertEqual(first, second)
            self.assertEqual(first["status"], "compatibility_ready")
            self.assertEqual(
                [item["stage"] for item in first["stages"]], doctor.STAGE_ORDER
            )
            self.assertEqual(
                [item["status"] for item in first["stages"]],
                ["passed", "passed", "passed", "not_checked"],
            )
            unsigned = dict(first)
            declared = unsigned.pop("report_digest")
            self.assertEqual(
                declared,
                promotion.sha256_bytes(promotion.canonical_json(unsigned)),
            )

    def test_authority_section_is_explicitly_empty(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            report = fixture.diagnose()
            self.assertFalse(report["authority"]["executes_server"])
            self.assertFalse(report["authority"]["connects_to_dfhack"])
            self.assertFalse(report["authority"]["reads_bridge_token"])
            self.assertEqual(report["authority"]["grants_capabilities"], [])
            self.assertEqual(report["authority"]["mutation_capabilities"], [])

    def test_secret_environment_does_not_affect_report(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            with mock.patch.dict(os.environ, {"DFMCP_BRIDGE_TOKEN": "a" * 64}, clear=False):
                first = fixture.diagnose()
            with mock.patch.dict(os.environ, {"DFMCP_BRIDGE_TOKEN": "b" * 64}, clear=False):
                second = fixture.diagnose()
            self.assertEqual(first, second)
            self.assertNotIn("a" * 64, json.dumps(first))
            self.assertNotIn("b" * 64, json.dumps(first))

    def test_registry_floor_mismatch_fails_before_tuple_resolution(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.registry_path.write_text(
                json.dumps(fixture.registry, sort_keys=True, indent=2) + "\n"
            )
            report = fixture.diagnose()
            self.assertEqual(report["status"], "not_ready")
            self.assertEqual(report["stages"][1]["status"], "failed")
            self.assertEqual(report["stages"][2]["status"], "not_checked")

    def test_permissive_floor_custody_fails_closed(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.floor_path.chmod(0o640)
            report = fixture.diagnose()
            self.assertEqual(report["status"], "not_ready")
            self.assertEqual(report["stages"][1]["status"], "failed")

    def test_wrong_entry_fence_is_not_ready(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            report = fixture.diagnose(entry_id=digest("wrong-entry"))
            self.assertEqual(report["status"], "not_ready")
            self.assertEqual(report["stages"][2]["status"], "failed")
            self.assertIsNone(report["server_artifact"])

    def test_empty_registry_reports_exact_tuple_failure(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            empty = {
                "schema_version": promotion.REGISTRY_SCHEMA,
                "status": "no_admitted_live_tuples",
                "entries": [],
            }
            fixture.registry_path.write_text(json.dumps(empty, sort_keys=True) + "\n")
            fixture.floor_path.unlink()
            doctor.compatibility_floor.initialize_floor(
                fixture.floor_path, fixture.registry_path
            )
            report = fixture.diagnose()
            self.assertEqual(report["status"], "not_ready")
            self.assertEqual(report["stages"][0]["status"], "passed")
            self.assertEqual(report["stages"][1]["status"], "passed")
            self.assertEqual(report["stages"][2]["status"], "failed")

    def test_partial_artifact_inputs_are_rejected_as_usage_error(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            with self.assertRaises(ValueError):
                doctor.diagnose(
                    fixture.manifest_path,
                    fixture.registry_path,
                    fixture.floor_path,
                    entry()["entry_id"],
                    binary_path=fixture.binary_path,
                )

    def test_artifact_preflight_ready_closes_opened_descriptor(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            report = fixture.diagnose(artifact=True)
            self.assertEqual(report["status"], "artifact_preflight_ready")
            self.assertEqual(report["stages"][3]["status"], "passed")
            self.assertEqual(
                report["server_artifact"]["binary"]["sha256"],
                fixture.binary_sha256(),
            )
            with self.assertRaises(OSError):
                os.fstat(fixture.opened_descriptors[-1])

    def test_artifact_source_mismatch_is_not_ready(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.normalized["source"]["dfmcp_commit"] = "3" * 40
            report = fixture.diagnose(artifact=True)
            self.assertEqual(report["status"], "not_ready")
            self.assertEqual(report["stages"][3]["status"], "failed")

    def test_artifact_platform_mismatch_is_not_ready(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.normalized["platform"] = {"system": "Other", "machine": "other"}
            report = fixture.diagnose(artifact=True)
            self.assertEqual(report["status"], "not_ready")
            self.assertEqual(report["stages"][3]["status"], "failed")

    def test_invalid_registry_preserves_fixed_stage_order(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.registry_path.write_text('{"bad":true}\n')
            report = fixture.diagnose()
            self.assertEqual(report["status"], "not_ready")
            self.assertEqual(
                [item["stage"] for item in report["stages"]], doctor.STAGE_ORDER
            )
            self.assertEqual(
                [item["status"] for item in report["stages"]],
                ["failed", "not_checked", "not_checked", "not_checked"],
            )

    def test_diagnostic_text_is_bounded_and_control_safe(self) -> None:
        value = "x" * 4096 + "\x00secret"
        bounded = doctor.bounded_text(value)
        self.assertLessEqual(len(bounded.encode("utf-8")), doctor.MAX_DIAGNOSTIC_BYTES)
        self.assertNotIn("\x00", bounded)

    def test_report_contains_no_timestamps_or_runtime_authority(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            encoded = json.dumps(fixture.diagnose(), sort_keys=True)
            for marker in ["created_at", "finished_at", "unix_seconds", "bearer_token"]:
                self.assertNotIn(marker, encoded)


if __name__ == "__main__":
    unittest.main()
