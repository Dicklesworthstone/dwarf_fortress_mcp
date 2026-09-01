#!/usr/bin/env python3

from __future__ import annotations

import hashlib
import json
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WRAPPER = ROOT / "scripts/qualify_live_server_binary.sh"
VERIFIER = ROOT / "scripts/verify_live_server_binary_receipt.py"
CONTRACT = ROOT / "architecture/live_server_binary_receipt_v1.json"


def run(args: list[str], cwd: Path, env: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
    return subprocess.run(args, cwd=cwd, env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)


class Fixture:
    def __init__(self, root: Path, fail_check: str | None = None) -> None:
        self.root = root
        self.repo = root / "repo"
        self.repo.mkdir()
        self.output = root / "output"
        self.local_receipt = root / "local-receipt.json"
        self.fake_bin = root / "bin"
        self.fake_bin.mkdir()
        self.contract = json.loads(CONTRACT.read_text())
        mapping = self.contract["source_binding"]["required_source_digests"]
        for relative in mapping.values():
            path = self.repo / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            source = {
                "architecture/live_server_binary_receipt_v1.json": CONTRACT,
                "scripts/qualify_live_server_binary.sh": WRAPPER,
                "scripts/verify_live_server_binary_receipt.py": VERIFIER,
            }.get(relative)
            if source is None:
                path.write_text(f"fixture source for {relative}\n")
            else:
                shutil.copy2(source, path)
        (self.repo / "scripts/qualify_live_server_binary.sh").chmod(0o755)
        (self.repo / "scripts/verify_live_server_binary_receipt.py").chmod(0o755)
        run(["git", "init", "-q"], self.repo)
        run(["git", "config", "user.email", "fixture@example.invalid"], self.repo)
        run(["git", "config", "user.name", "Fixture"], self.repo)
        run(["git", "add", "."], self.repo)
        committed = run(["git", "commit", "-qm", "fixture"], self.repo)
        if committed.returncode != 0:
            raise RuntimeError(committed.stderr)
        self.commit = run(["git", "rev-parse", "HEAD"], self.repo).stdout.strip()
        self.write_local_receipt()
        self.write_fake_tools(fail_check)

    def write_local_receipt(self, failed_gate: str | None = None) -> None:
        gates = []
        for name in self.contract["source_binding"]["required_local_qualification_gates"]:
            failed = name == failed_gate
            gates.append(
                {
                    "name": name,
                    "state": "failed" if failed else "passed",
                    "detail": "fixture failure" if failed else None,
                }
            )
        receipt = {
            "schema": "dfmcp.qualification-receipt.v1",
            "status": "failed" if failed_gate else "passed",
            "started_at": "2026-09-01T00:00:00Z",
            "finished_at": "2026-09-01T00:01:00Z",
            "source": {"commit": self.commit, "dirty": False},
            "host": {"system": "fixture", "machine": "fixture"},
            "toolchain": {"rustc_vv": "fixture", "cargo": "fixture"},
            "digests": {"fixture": hashlib.sha256(b"fixture").hexdigest()},
            "gates": gates,
        }
        self.local_receipt.write_text(json.dumps(receipt, sort_keys=True) + "\n")

    def write_fake_tools(self, fail_check: str | None) -> None:
        cargo = self.fake_bin / "cargo"
        cargo.write_text(
            "#!/bin/sh\n"
            "if [ \"$1\" = \"--version\" ]; then echo 'cargo 1.99.0 fixture'; exit 0; fi\n"
            "if [ \"$1\" = \"build\" ]; then\n"
            "  mkdir -p \"$PWD/target/release\"\n"
            "  cat > \"$PWD/target/release/dwarf-fortress-mcp\" <<'EOF'\n"
            "#!/bin/sh\n"
            f"if [ \"$1\" = \"{fail_check or '__never__'}\" ]; then echo forced failure >&2; exit 9; fi\n"
            "case \"$1\" in contract|doctor|demo) echo \"fixture-$1\";; *) exit 2;; esac\n"
            "EOF\n"
            "  chmod 700 \"$PWD/target/release/dwarf-fortress-mcp\"\n"
            "  exit 0\n"
            "fi\n"
            "exit 2\n"
        )
        cargo.chmod(0o755)
        rustc = self.fake_bin / "rustc"
        rustc.write_text(
            "#!/bin/sh\n"
            "if [ \"$1\" = \"-vV\" ]; then printf 'rustc 1.99.0 fixture\\nhost: fixture\\n'; exit 0; fi\n"
            "exit 2\n"
        )
        rustc.chmod(0o755)

    def environment(self) -> dict[str, str]:
        environment = dict(os.environ)
        environment["PATH"] = f"{self.fake_bin}:{environment.get('PATH', '')}"
        return environment

    def qualify(self) -> subprocess.CompletedProcess[str]:
        return run(
            [
                "bash",
                "scripts/qualify_live_server_binary.sh",
                os.fspath(self.local_receipt),
                os.fspath(self.output),
            ],
            self.repo,
            self.environment(),
        )


class QualifyLiveServerBinaryTests(unittest.TestCase):
    def fixture(self, fail_check: str | None = None) -> tuple[tempfile.TemporaryDirectory[str], Fixture]:
        temporary = tempfile.TemporaryDirectory()
        return temporary, Fixture(Path(temporary.name), fail_check)

    def test_wrapper_builds_checks_issues_and_reverifies_receipt(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            result = fixture.qualify()
            self.assertEqual(result.returncode, 0, result.stderr)
            receipt_path = fixture.output / "live-server-binary-receipt.json"
            receipt = json.loads(receipt_path.read_text())
            self.assertEqual(receipt["status"], "qualified")
            self.assertEqual(receipt["source"]["dfmcp_commit"], fixture.commit)
            self.assertEqual(
                [item["name"] for item in receipt["executable_checks"]],
                ["contract", "doctor", "demo"],
            )
            self.assertEqual(receipt["mutation_capabilities"], [])
            self.assertTrue((fixture.output / "SHA256SUMS").is_file())

    def test_dirty_source_is_rejected_before_build(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            (fixture.repo / "dirty.txt").write_text("dirty\n")
            result = fixture.qualify()
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse((fixture.repo / "target/release/dwarf-fortress-mcp").exists())

    def test_failed_local_gate_is_rejected_before_build(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.write_local_receipt("clippy")
            result = fixture.qualify()
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse((fixture.repo / "target/release/dwarf-fortress-mcp").exists())

    def test_failed_executable_check_prevents_receipt(self) -> None:
        temporary, fixture = self.fixture("doctor")
        with temporary:
            result = fixture.qualify()
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse((fixture.output / "live-server-binary-receipt.json").exists())

    def test_receipt_binds_qualification_wrapper_source(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            result = fixture.qualify()
            self.assertEqual(result.returncode, 0, result.stderr)
            receipt = json.loads(
                (fixture.output / "live-server-binary-receipt.json").read_text()
            )
            expected = hashlib.sha256(
                (fixture.repo / "scripts/qualify_live_server_binary.sh").read_bytes()
            ).hexdigest()
            self.assertEqual(receipt["source_digests"]["artifact_qualification"], expected)


if __name__ == "__main__":
    unittest.main()
