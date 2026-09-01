#!/usr/bin/env python3

from __future__ import annotations

import hashlib
import importlib.util
import json
import os
import platform
import sys
import tempfile
import unittest
from pathlib import Path
from typing import Any

MODULE_PATH = Path(__file__).with_name("verify_live_server_binary_receipt.py")
SPEC = importlib.util.spec_from_file_location("verify_live_server_binary_receipt", MODULE_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load live server binary verifier")
verifier = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = verifier
SPEC.loader.exec_module(verifier)


def digest(value: bytes | str) -> str:
    raw = value.encode() if isinstance(value, str) else value
    return hashlib.sha256(raw).hexdigest()


class Fixture:
    def __init__(self, root: Path) -> None:
        self.root = root
        self.source_root = root / "source"
        self.source_root.mkdir()
        self.contract_path = self.source_root / "architecture/live_server_binary_receipt_v1.json"
        self.verifier_path = self.source_root / "scripts/verify_live_server_binary_receipt.py"
        self.contract_path.parent.mkdir(parents=True)
        self.verifier_path.parent.mkdir(parents=True)
        repository_root = MODULE_PATH.parents[1]
        self.contract_path.write_bytes(
            repository_root.joinpath("architecture/live_server_binary_receipt_v1.json").read_bytes()
        )
        self.verifier_path.write_bytes(MODULE_PATH.read_bytes())
        self.contract = json.loads(self.contract_path.read_text(encoding="utf-8"))
        for relative in self.contract["source_binding"]["required_source_digests"].values():
            path = self.source_root / relative
            if path in {self.contract_path, self.verifier_path}:
                continue
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(f"fixture source for {relative}\n", encoding="utf-8")

        self.commit = "1" * 40
        self.local_receipt_path = root / "local-qualification.json"
        self.local_receipt_path.write_text(
            json.dumps(
                {
                    "schema": verifier.LOCAL_RECEIPT_SCHEMA,
                    "status": "passed",
                    "source": {"commit": self.commit, "dirty": False},
                    "gates": [
                        {"name": "static-contracts", "state": "passed", "detail": None},
                        {"name": "tests", "state": "passed", "detail": None},
                    ],
                },
                sort_keys=True,
            )
            + "\n",
            encoding="utf-8",
        )
        self.binary_path = root / "dwarf-fortress-mcp"
        self.binary_path.write_bytes(b"qualified-fixture-server")
        self.binary_path.chmod(0o700)
        self.receipt_path = root / "server-receipt.json"
        self.write_receipt(self.receipt())

    def source_digests(self) -> dict[str, str]:
        return {
            name: verifier.sha256_file(self.source_root / relative)
            for name, relative in self.contract["source_binding"]["required_source_digests"].items()
        }

    def receipt(self) -> dict[str, Any]:
        unsigned: dict[str, Any] = {
            "schema": verifier.RECEIPT_SCHEMA,
            "status": "qualified",
            "source": {
                "dfmcp_commit": self.commit,
                "dfmcp_dirty": False,
                "local_qualification_receipt_sha256": verifier.sha256_file(
                    self.local_receipt_path
                ),
            },
            "platform": {"system": platform.system(), "machine": platform.machine()},
            "toolchain": {"rustc_vv": "rustc fixture", "cargo": "cargo fixture"},
            "binary": {
                "name": "dwarf-fortress-mcp",
                "profile": "release",
                "relative_path": "target/release/dwarf-fortress-mcp",
                "bytes": self.binary_path.stat().st_size,
                "sha256": verifier.sha256_file(self.binary_path),
            },
            "executable_checks": [
                {
                    "name": name,
                    "status": "passed",
                    "stdout_sha256": digest(f"stdout-{name}"),
                    "stderr_sha256": digest(f"stderr-{name}"),
                }
                for name in self.contract["required_executable_checks"]
            ],
            "source_digests": self.source_digests(),
            "mutation_capabilities": [],
            "claims_not_established": self.contract["claims_not_established"],
        }
        return {
            **unsigned,
            "receipt_digest": verifier.sha256_bytes(verifier.canonical_json(unsigned)),
        }

    def write_receipt(self, value: dict[str, Any]) -> None:
        self.receipt_path.write_text(
            json.dumps(value, sort_keys=True) + "\n", encoding="utf-8"
        )

    def verify(self) -> tuple[dict[str, Any], Any]:
        return verifier.verify(
            self.receipt_path,
            self.binary_path,
            self.contract_path,
            self.source_root,
            self.local_receipt_path,
            self.commit,
        )


class LiveServerBinaryReceiptTests(unittest.TestCase):
    def fixture(self) -> tuple[tempfile.TemporaryDirectory[str], Fixture]:
        temporary = tempfile.TemporaryDirectory()
        return temporary, Fixture(Path(temporary.name))

    def test_valid_receipt_opens_exact_qualified_inode(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            normalized, opened = fixture.verify()
            try:
                metadata = os.fstat(opened.descriptor)
                self.assertEqual(opened.inode, metadata.st_ino)
                self.assertEqual(opened.device, metadata.st_dev)
                self.assertEqual(opened.sha256, normalized["binary"]["sha256"])
                self.assertEqual(
                    normalized["receipt_sha256"],
                    verifier.sha256_file(fixture.receipt_path),
                )
            finally:
                os.close(opened.descriptor)

    def test_duplicate_json_keys_are_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.receipt_path.write_text(
                '{"schema":"dfmcp.live-server-binary-qualification/1",'
                '"status":"qualified","status":"not-qualified"}\n',
                encoding="utf-8",
            )
            with self.assertRaises(verifier.VerificationError):
                fixture.verify()

    def test_receipt_field_tampering_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            value = fixture.receipt()
            value["binary"]["sha256"] = digest("tampered")
            fixture.write_receipt(value)
            with self.assertRaises(verifier.VerificationError):
                fixture.verify()

    def test_binary_without_execute_bit_is_rejected_on_opened_inode(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.binary_path.chmod(0o600)
            with self.assertRaises(verifier.VerificationError):
                fixture.verify()

    def test_group_writable_binary_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.binary_path.chmod(0o720)
            with self.assertRaises(verifier.VerificationError):
                fixture.verify()

    def test_symbolic_link_binary_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            target = fixture.root / "real-server"
            fixture.binary_path.replace(target)
            try:
                fixture.binary_path.symlink_to(target)
            except OSError:
                self.skipTest("symbolic links are unavailable")
            with self.assertRaises(verifier.VerificationError):
                fixture.verify()

    def test_binary_content_substitution_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.binary_path.write_bytes(b"substituted-server-bytes")
            fixture.binary_path.chmod(0o700)
            with self.assertRaises(verifier.VerificationError):
                fixture.verify()

    def test_local_qualification_receipt_mismatch_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            local = json.loads(fixture.local_receipt_path.read_text())
            local["source"]["commit"] = "2" * 40
            fixture.local_receipt_path.write_text(json.dumps(local) + "\n")
            with self.assertRaises(verifier.VerificationError):
                fixture.verify()

    def test_skipped_local_qualification_gate_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            local = json.loads(fixture.local_receipt_path.read_text())
            local["gates"][0]["state"] = "skipped"
            fixture.local_receipt_path.write_text(json.dumps(local) + "\n")
            fixture.write_receipt(fixture.receipt())
            with self.assertRaises(verifier.VerificationError):
                fixture.verify()

    def test_source_digest_drift_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            path = fixture.source_root / "crates/dfmcp-mcp/src/live_server.rs"
            path.write_text("drifted source\n")
            with self.assertRaises(verifier.VerificationError):
                fixture.verify()

    def test_mutation_capability_contamination_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            value = fixture.receipt()
            value["mutation_capabilities"] = ["pause"]
            unsigned = dict(value)
            unsigned.pop("receipt_digest", None)
            value["receipt_digest"] = verifier.sha256_bytes(
                verifier.canonical_json(unsigned)
            )
            fixture.write_receipt(value)
            with self.assertRaises(verifier.VerificationError):
                fixture.verify()

    def test_normalized_receipt_digest_matches_same_parsed_bytes(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            value, file_digest = verifier.read_object_with_digest(
                fixture.receipt_path, "server receipt"
            )
            self.assertEqual(file_digest, verifier.sha256_file(fixture.receipt_path))
            self.assertEqual(value, json.loads(fixture.receipt_path.read_text()))


if __name__ == "__main__":
    unittest.main()
