#!/usr/bin/env python3

from __future__ import annotations

import hashlib
import importlib.util
import json
import os
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
WRAPPER = ROOT / "scripts/qualify_live_server_binary.sh"
VERIFIER = ROOT / "scripts/verify_live_server_binary_receipt.py"
CONTRACT = ROOT / "architecture/live_server_binary_receipt_v1.json"

SPEC = importlib.util.spec_from_file_location("verify_live_server_binary_receipt_test", VERIFIER)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load server artifact verifier")
verifier = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = verifier
SPEC.loader.exec_module(verifier)


def run(
    args: list[str],
    cwd: Path,
    env: dict[str, str] | None = None,
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        args,
        cwd=cwd,
        env=env,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )


def run_git(root: Path, *arguments: str) -> str:
    completed = run(["git", *arguments], root)
    if completed.returncode != 0:
        raise RuntimeError(completed.stderr)
    return completed.stdout.strip()


class Fixture:
    def __init__(
        self,
        root: Path,
        fail_check: str | None = None,
        mutate_source_on_build: bool = False,
    ) -> None:
        self.root = root
        self.repo = root / "repo"
        self.repo.mkdir()
        self.output = root / "output"
        self.local_receipt_directory = root / "qualification"
        self.local_receipt_directory.mkdir(mode=0o700)
        self.local_receipt_directory.chmod(0o700)
        self.local_receipt = self.local_receipt_directory / "local-receipt.json"
        self.fake_bin = root / "bin"
        self.fake_bin.mkdir()
        self.contract = json.loads(CONTRACT.read_text(encoding="utf-8"))
        mapping = self.contract["source_binding"]["required_source_digests"]
        sources = {
            "architecture/live_server_binary_receipt_v1.json": CONTRACT,
            "scripts/qualify_live_server_binary.sh": WRAPPER,
            "scripts/verify_live_server_binary_receipt.py": VERIFIER,
            "scripts/test_qualify_live_server_binary.py": Path(__file__),
        }
        for relative in mapping.values():
            path = self.repo / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            source = sources.get(relative)
            if source is None:
                path.write_text(f"fixture source for {relative}\n", encoding="utf-8")
            else:
                shutil.copy2(source, path)
        (self.repo / "scripts/qualify_live_server_binary.sh").chmod(0o755)
        (self.repo / "scripts/verify_live_server_binary_receipt.py").chmod(0o755)
        (self.repo / ".gitignore").write_text("/target/\n", encoding="utf-8")
        run_git(self.repo, "init", "-q")
        run_git(self.repo, "config", "user.email", "fixture@example.invalid")
        run_git(self.repo, "config", "user.name", "Fixture")
        run_git(self.repo, "add", ".")
        run_git(self.repo, "commit", "-q", "-m", "fixture")
        self.commit = run_git(self.repo, "rev-parse", "HEAD")
        self.tree, self.inventory = verifier.collect_head_equivalent_source_inventory(
            self.repo,
            self.commit,
        )
        self.write_local_receipt()
        self.write_fake_tools(fail_check, mutate_source_on_build)

    def local_receipt_value(self, failed_gate: str | None = None) -> dict[str, Any]:
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
        return {
            "schema": "dfmcp.qualification-receipt.v1",
            "status": "failed" if failed_gate else "passed",
            "started_at": "2026-09-01T00:00:00Z",
            "finished_at": "2026-09-01T00:01:00Z",
            "source": {
                "commit": self.commit,
                "dirty": False,
                "head_equivalent": True,
                "tree": self.tree,
                "snapshot_digest": hashlib.sha256(b"fixture snapshot").hexdigest(),
            },
            "host": {"system": "fixture", "machine": "fixture"},
            "toolchain": {"rustc_vv": "fixture", "cargo": "fixture"},
            "digests": dict(self.inventory),
            "gates": gates,
        }

    def write_local_receipt(self, failed_gate: str | None = None) -> None:
        self.local_receipt.write_text(
            json.dumps(self.local_receipt_value(failed_gate), sort_keys=True) + "\n",
            encoding="utf-8",
        )
        self.local_receipt.chmod(0o600)

    def write_fake_tools(
        self,
        fail_check: str | None,
        mutate_source_on_build: bool,
    ) -> None:
        mutation = ""
        if mutate_source_on_build:
            mutation = (
                "  git update-index --assume-unchanged crates/dfmcp-mcp/src/live_server.rs\n"
                "  printf 'hidden build drift\\n' > crates/dfmcp-mcp/src/live_server.rs\n"
            )
        cargo = self.fake_bin / "cargo"
        cargo.write_text(
            "#!/bin/sh\n"
            "if [ \"$1\" = \"--version\" ]; then echo 'cargo 1.99.0 fixture'; exit 0; fi\n"
            "if [ \"$1\" = \"build\" ]; then\n"
            f"{mutation}"
            "  mkdir -p \"$PWD/target/release\"\n"
            "  cat > \"$PWD/target/release/dwarf-fortress-mcp\" <<'EOF'\n"
            "#!/bin/sh\n"
            f"if [ \"$1\" = \"{fail_check or '__never__'}\" ]; then echo forced failure >&2; exit 9; fi\n"
            "case \"$1\" in contract|doctor|demo) echo \"fixture-$1\";; *) exit 2;; esac\n"
            "EOF\n"
            "  chmod 700 \"$PWD/target/release/dwarf-fortress-mcp\"\n"
            "  exit 0\n"
            "fi\n"
            "exit 2\n",
            encoding="utf-8",
        )
        cargo.chmod(0o755)
        rustc = self.fake_bin / "rustc"
        rustc.write_text(
            "#!/bin/sh\n"
            "if [ \"$1\" = \"-vV\" ]; then printf 'rustc 1.99.0 fixture\\nhost: fixture\\n'; exit 0; fi\n"
            "exit 2\n",
            encoding="utf-8",
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
    def fixture(
        self,
        fail_check: str | None = None,
        mutate_source_on_build: bool = False,
    ) -> tuple[tempfile.TemporaryDirectory[str], Fixture]:
        temporary = tempfile.TemporaryDirectory()
        return temporary, Fixture(
            Path(temporary.name),
            fail_check,
            mutate_source_on_build,
        )

    def test_wrapper_builds_checks_issues_and_reverifies_receipt(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            result = fixture.qualify()
            self.assertEqual(result.returncode, 0, result.stderr)
            receipt_path = fixture.output / "live-server-binary-receipt.json"
            receipt = json.loads(receipt_path.read_text(encoding="utf-8"))
            self.assertEqual(receipt["status"], "qualified")
            self.assertEqual(receipt["source"]["dfmcp_commit"], fixture.commit)
            self.assertEqual(
                [item["name"] for item in receipt["executable_checks"]],
                ["contract", "doctor", "demo"],
            )
            self.assertEqual(receipt["mutation_capabilities"], [])
            self.assertTrue((fixture.output / "SHA256SUMS").is_file())
            if os.name == "posix":
                self.assertEqual(stat.S_IMODE(fixture.output.stat().st_mode), 0o700)
                self.assertEqual(
                    stat.S_IMODE((fixture.output / "logs").stat().st_mode),
                    0o700,
                )
                self.assertEqual(stat.S_IMODE(receipt_path.stat().st_mode), 0o600)
                self.assertEqual(
                    stat.S_IMODE((fixture.output / "SHA256SUMS").stat().st_mode),
                    0o600,
                )

    def test_existing_output_directory_is_rejected_before_build(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.output.mkdir()
            result = fixture.qualify()
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse((fixture.repo / "target/release/dwarf-fortress-mcp").exists())

    def test_dirty_source_is_rejected_before_build(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            (fixture.repo / "dirty.txt").write_text("dirty\n", encoding="utf-8")
            result = fixture.qualify()
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse((fixture.repo / "target/release/dwarf-fortress-mcp").exists())

    def test_assume_unchanged_source_drift_is_rejected_before_build(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            relative = "crates/dfmcp-mcp/src/live_server.rs"
            run_git(fixture.repo, "update-index", "--assume-unchanged", relative)
            (fixture.repo / relative).write_text("hidden drift\n", encoding="utf-8")
            self.assertEqual(run_git(fixture.repo, "status", "--porcelain=v1"), "")
            result = fixture.qualify()
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("bytes differ from HEAD", result.stderr)
            self.assertFalse((fixture.repo / "target/release/dwarf-fortress-mcp").exists())

    def test_non_private_local_receipt_is_rejected_before_build(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.local_receipt.chmod(0o644)
            result = fixture.qualify()
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("mode 0600", result.stderr)
            self.assertFalse((fixture.repo / "target/release/dwarf-fortress-mcp").exists())

    def test_failed_local_gate_is_rejected_before_build(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.write_local_receipt("clippy")
            result = fixture.qualify()
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse((fixture.repo / "target/release/dwarf-fortress-mcp").exists())

    def test_source_drift_during_build_prevents_receipt(self) -> None:
        temporary, fixture = self.fixture(mutate_source_on_build=True)
        with temporary:
            result = fixture.qualify()
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("bytes differ from HEAD", result.stderr)
            self.assertFalse((fixture.output / "live-server-binary-receipt.json").exists())

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
                (fixture.output / "live-server-binary-receipt.json").read_text(
                    encoding="utf-8"
                )
            )
            expected = hashlib.sha256(
                (fixture.repo / "scripts/qualify_live_server_binary.sh").read_bytes()
            ).hexdigest()
            self.assertEqual(receipt["source_digests"]["artifact_qualification"], expected)


if __name__ == "__main__":
    unittest.main()
