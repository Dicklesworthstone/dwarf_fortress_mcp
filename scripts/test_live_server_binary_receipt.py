#!/usr/bin/env python3

from __future__ import annotations

import hashlib
import importlib.util
import json
import os
import platform
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from typing import Any
from unittest import mock

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
        self.source_root = root / "source"
        self.source_root.mkdir()
        repository_root = MODULE_PATH.parents[1]
        self.contract_path = self.source_root / "architecture/live_server_binary_receipt_v1.json"
        self.verifier_path = self.source_root / "scripts/verify_live_server_binary_receipt.py"
        self.contract_path.parent.mkdir(parents=True)
        self.verifier_path.parent.mkdir(parents=True)
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
        run_git(self.source_root, "init", "-q")
        run_git(self.source_root, "config", "user.email", "fixture@example.invalid")
        run_git(self.source_root, "config", "user.name", "Fixture")
        run_git(self.source_root, "add", ".")
        run_git(self.source_root, "commit", "-q", "-m", "fixture")
        self.commit = run_git(self.source_root, "rev-parse", "HEAD")
        self.tree, self.inventory = verifier.collect_head_equivalent_source_inventory(
            self.source_root,
            self.commit,
        )
        self.local_receipt_path = root / "local-qualification.json"
        self.write_local_receipt()
        self.binary_path = root / "dwarf-fortress-mcp"
        self.binary_path.write_bytes(b"qualified-fixture-server")
        self.binary_path.chmod(0o700)
        self.receipt_path = root / "server-receipt.json"
        self.write_receipt(self.receipt())

    def local_receipt(self) -> dict[str, Any]:
        return {
            "schema": verifier.LOCAL_RECEIPT_SCHEMA,
            "status": "passed",
            "started_at": "2026-09-01T00:00:00Z",
            "finished_at": "2026-09-01T00:01:00Z",
            "source": {
                "commit": self.commit,
                "dirty": False,
                "head_equivalent": True,
                "tree": self.tree,
                "snapshot_digest": digest("fixture-source-snapshot"),
            },
            "host": {
                "system": platform.system(),
                "release": platform.release(),
                "machine": platform.machine(),
                "python": platform.python_version(),
            },
            "toolchain": {"rustc_vv": "rustc fixture", "cargo": "cargo fixture"},
            "digests": dict(self.inventory),
            "gates": [
                {"name": name, "state": "passed", "detail": None}
                for name in self.contract["source_binding"][
                    "required_local_qualification_gates"
                ]
            ],
        }

    def write_local_receipt(self, value: dict[str, Any] | None = None) -> None:
        self.local_receipt_path.write_text(
            json.dumps(self.local_receipt() if value is None else value, sort_keys=True) + "\n",
            encoding="utf-8",
        )

    def source_digests(self) -> dict[str, str]:
        return {
            name: verifier.sha256_file(self.source_root / relative)
            for name, relative in self.contract["source_binding"][
                "required_source_digests"
            ].items()
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

    def rebind_local_receipt(self, value: dict[str, Any]) -> None:
        self.write_local_receipt(value)
        self.write_receipt(self.receipt())

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
            local = fixture.local_receipt()
            local["source"]["commit"] = "2" * 40
            fixture.rebind_local_receipt(local)
            with self.assertRaisesRegex(
                verifier.VerificationError,
                "different source commit",
            ):
                fixture.verify()

    def test_non_head_equivalent_local_receipt_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            local = fixture.local_receipt()
            local["source"]["head_equivalent"] = False
            fixture.rebind_local_receipt(local)
            with self.assertRaisesRegex(verifier.VerificationError, "HEAD-equivalent"):
                fixture.verify()

    def test_local_receipt_tree_mismatch_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            local = fixture.local_receipt()
            local["source"]["tree"] = "2" * 40
            fixture.rebind_local_receipt(local)
            with self.assertRaisesRegex(verifier.VerificationError, "tree differs"):
                fixture.verify()

    def test_local_receipt_inventory_path_or_digest_drift_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            local = fixture.local_receipt()
            removed = next(iter(local["digests"]))
            del local["digests"][removed]
            fixture.rebind_local_receipt(local)
            with self.assertRaisesRegex(verifier.VerificationError, "digest inventory differs"):
                fixture.verify()

        temporary, fixture = self.fixture()
        with temporary:
            local = fixture.local_receipt()
            path = next(iter(local["digests"]))
            local["digests"][path] = digest("wrong")
            fixture.rebind_local_receipt(local)
            with self.assertRaisesRegex(verifier.VerificationError, "digest inventory differs"):
                fixture.verify()

    def test_missing_local_qualification_gate_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            local = fixture.local_receipt()
            local["gates"].pop()
            fixture.rebind_local_receipt(local)
            with self.assertRaises(verifier.VerificationError):
                fixture.verify()

    def test_reordered_local_qualification_gate_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            local = fixture.local_receipt()
            local["gates"][0], local["gates"][1] = local["gates"][1], local["gates"][0]
            fixture.rebind_local_receipt(local)
            with self.assertRaises(verifier.VerificationError):
                fixture.verify()

    def test_skipped_local_qualification_gate_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            local = fixture.local_receipt()
            local["gates"][0]["state"] = "skipped"
            fixture.rebind_local_receipt(local)
            with self.assertRaises(verifier.VerificationError):
                fixture.verify()

    def test_source_digest_drift_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            path = fixture.source_root / "crates/dfmcp-mcp/src/live_server.rs"
            path.write_text("drifted source\n", encoding="utf-8")
            with self.assertRaises(verifier.VerificationError):
                fixture.verify()

    def test_assume_unchanged_cannot_hide_source_drift_after_local_qualification(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            relative = "crates/dfmcp-mcp/src/live_server.rs"
            run_git(fixture.source_root, "update-index", "--assume-unchanged", relative)
            (fixture.source_root / relative).write_text("hidden drift\n", encoding="utf-8")
            self.assertEqual(run_git(fixture.source_root, "status", "--porcelain=v1"), "")
            with self.assertRaisesRegex(verifier.VerificationError, "bytes differ from HEAD"):
                fixture.verify()

    @unittest.skipUnless(os.name == "posix", "Unix executable semantics required")
    def test_core_filemode_false_cannot_hide_source_mode_drift(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            relative = "crates/dfmcp-mcp/src/live_server.rs"
            run_git(fixture.source_root, "config", "core.fileMode", "false")
            (fixture.source_root / relative).chmod(0o755)
            self.assertEqual(run_git(fixture.source_root, "status", "--porcelain=v1"), "")
            with self.assertRaisesRegex(verifier.VerificationError, "executable semantics differ"):
                fixture.verify()

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
                normalized, opened = fixture.verify()
            try:
                self.assertEqual(normalized["source"]["dfmcp_commit"], fixture.commit)
            finally:
                os.close(opened.descriptor)

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
