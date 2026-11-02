#!/usr/bin/env python3
"""Actual standard CLI routes; native peers remain component doubles, never DF."""
from __future__ import annotations
import argparse
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import unittest

ROOT = Path(__file__).resolve().parents[2]
PROFILES = {
    "tree": ("DFMCP_ALLOW_UNADMITTED_TREE_HARVEST_V1_21", "dfmcp-tree-harvest-v1-21-dev"),
    "workshop": ("DFMCP_ALLOW_UNADMITTED_WORKSHOP_V1_20", "dfmcp-workshop-v1-20-dev"),
    "bounded-clock": ("DFMCP_ALLOW_UNADMITTED_RUN_V1_13", "dfmcp-live-run-dev"),
    "finite-bed": ("DFMCP_ALLOW_UNADMITTED_SEMANTIC_WORK_ORDERS", "dfmcp-live-semantic-work-orders-dev"),
}
TOOLS = {"fortress_" + n for n in ("open_session", "observe", "query", "plan", "commit", "wait", "cancel", "checkpoint", "restore", "explain", "doctor")}
BINARY: Path
LOGS: Path


def environment(profile):
    env = {k: v for k, v in os.environ.items() if not k.startswith("DFMCP_")}
    if profile is not None:
        env[PROFILES[profile][0]] = "1"
    return env


def retain(stem, result):
    for suffix, raw in (("stdout", result.stdout), ("stderr", result.stderr),
                        ("exit", (str(result.returncode) + "\n").encode())):
        with Path(str(stem) + "." + suffix).open("xb") as stream:
            stream.write(raw)


class GameplayCliTests(unittest.TestCase):
    def setUp(self):
        self.directory = LOGS / self._testMethodName
        self.directory.mkdir(mode=0o700)

    def test_closed_startup_profiles_refuse_unknown_missing_and_extra_arguments(self):
        requests = [[], ["unknown"], ["1.21"], ["/tmp/arbitrary-server"],
                    ["tree", "workshop"], ["tree", "--endpoint=127.0.0.1:5000"]]
        for index, arguments in enumerate(requests):
            with self.subTest(arguments=arguments):
                result = subprocess.run([str(BINARY), "serve-gameplay-dev", *arguments],
                    input=b"", capture_output=True, env=environment(None), timeout=5)
                retain(self.directory / str(index), result)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(result.stdout, b"")
                self.assertIn(b"usage: dwarf-fortress-mcp serve-gameplay-dev", result.stderr)

    def test_four_routes_have_exact_modern_waist_and_refuse_ungranted_control(self):
        spec = importlib.util.spec_from_file_location("gameplay_tree_client", ROOT / "tests/stdio/tree_harvest.py")
        module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
        module.BINARY = BINARY
        contract = json.loads((ROOT / "architecture/agent_turn_contract.json").read_text())
        for profile, (_, name) in PROFILES.items():
            with self.subTest(profile=profile):
                directory = self.directory / profile; directory.mkdir(mode=0o700)
                refused = subprocess.run([str(BINARY), "serve-gameplay-dev", profile],
                    input=b"", capture_output=True, env=environment(None), timeout=5)
                retain(directory / "missing-opt-in", refused)
                self.assertNotEqual(refused.returncode, 0)
                self.assertEqual(refused.stdout, b"")
                client = module.Client(directory, {}, env=environment(profile),
                    server_args=["serve-gameplay-dev", profile])
                try:
                    discovery = client.call("server/discover", {})["result"]
                    self.assertEqual(discovery["supportedVersions"], ["2026-07-28"])
                    self.assertEqual(discovery["_meta"]["io.modelcontextprotocol/serverInfo"]["name"], name)
                    tools = client.call("tools/list", {})["result"]["tools"]
                    self.assertEqual(len(tools), 11)
                    self.assertEqual({t["name"] for t in tools}, TOOLS)
                    result = client.tool("open_session", mode="control", max_wall_millis=60000)
                    self.assertFalse(result["result"]["ok"], result)
                    for key in contract["field_order"]:
                        self.assertIn(key, result["agent_turn"])
                    self.assertEqual(result["agent_turn"]["schema"], "dfmcp.agent_turn/1")
                finally:
                    client.close()
                # All three production markers refuse before any stdio result.
                for marker in ("", "1.0", "1.21"):
                    env = environment(profile); env["DFMCP_ADMITTED_BRIDGE_PROTOCOL"] = marker
                    result = subprocess.run([str(BINARY), "serve-gameplay-dev", profile],
                        input=b"", capture_output=True, env=env, timeout=5)
                    retain(directory / ("production-" + (marker or "empty")), result)
                    self.assertNotEqual(result.returncode, 0)
                    self.assertEqual(result.stdout, b"")
                    self.assertIn(b"production", result.stderr.lower())

    def test_existing_tree_and_workshop_positive_and_recovery_suites_use_standard_cli(self):
        for profile, script in (("tree", "tree_harvest.py"), ("workshop", "workshop.py")):
            with self.subTest(profile=profile):
                argv = [sys.executable, "-B", str(ROOT / "tests/stdio" / script),
                    "--binary", str(BINARY), "--logs", str(self.directory / (profile + "-raw")),
                    "--server-arg=serve-gameplay-dev", "--server-arg=" + profile]
                (self.directory / (profile + ".argv.json")).write_text(json.dumps(argv) + "\n")
                result = subprocess.run(argv, capture_output=True, env=environment(None))
                retain(self.directory / profile, result)
                self.assertEqual(result.returncode, 0, result.stderr.decode(errors="replace"))
                report = json.loads(result.stdout.splitlines()[-1])
                self.assertEqual(report["tests"], 5)
                for key in ("failed", "errors", "skipped"):
                    self.assertEqual(report[key], 0)
                self.assertTrue(report["actual_modern_stdio"])
                self.assertTrue(report["loopback_peers_are_doubles"])
                self.assertFalse(report["live_game"])


def main():
    global BINARY, LOGS
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--logs", type=Path, required=True)
    args = parser.parse_args(); BINARY = args.binary.resolve(strict=True)
    LOGS = args.logs; LOGS.mkdir(mode=0o700)
    result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(GameplayCliTests))
    print(json.dumps({"schema":"dfmcp.gameplay-cli-stdio/1", "tests":result.testsRun,
        "failed":len(result.failures), "errors":len(result.errors), "skipped":len(result.skipped),
        "actual_modern_stdio":True,"live_game":False}))
    raise SystemExit(0 if result.wasSuccessful() and result.testsRun == 3 and not result.skipped else 1)


if __name__ == "__main__": main()
