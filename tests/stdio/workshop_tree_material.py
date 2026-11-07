#!/usr/bin/env python3
"""Two actual modern stdio hosts with an acknowledged Tree-to-workshop handoff.

The joined loopback peers are explicit protocol doubles, not a game or native
R1. Independent Tree fixtures align their new-item horizon and material with
the unchanged workshop golden vector; no native golden vector is rewritten.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import struct
import unittest

import tree_harvest as tree
import workshop

ROOT = Path(__file__).resolve().parents[2]
LOGS: Path
TEST_COUNT = 5


def fixtures():
    vectors = {
        name: bytes.fromhex(value)
        for name, value in json.loads(
            (ROOT / "tests/native/tree_harvest/vectors.json").read_text()
        ).items()
    }
    capture = bytearray(vectors["capture"])
    assert len(capture) == 1090 and capture[:8] == b"DFMTC021"
    capture[48:52] = struct.pack(">I", 42)
    # Keep the pre-existing axe below the independent new-item horizon.
    assert int.from_bytes(capture[-36:-32], "big") == 62
    capture[-36:-32] = struct.pack(">I", 41)

    original = vectors["designated"]
    key_length = int.from_bytes(original[8:10], "big")
    at = 10 + key_length + 32 + 16
    before_length = int.from_bytes(original[at:at + 4], "big")
    at += 4 + before_length
    assert original[at:at + 3] == bytes([2, 0, 1])
    at += 3
    after_length = int.from_bytes(original[at:at + 4], "big")
    after = bytearray(original[at + 4:at + 4 + after_length])
    assert len(after) == len(capture) and after[:8] == b"DFMTC021"
    after[48:52] = struct.pack(">I", 42)
    after[-36:-32] = struct.pack(">I", 41)
    at += 4 + after_length
    assert original[at] == 1
    proof = original[at + 1:-32]
    assert proof[:8] == b"DFMTI021"

    plan = hashlib.sha256(
        b"dfmcp-tree-harvest-plan/1\0"
        + struct.pack(">III", 15, 15, 2)
        + hashlib.sha256(capture).digest()
    ).digest()
    key = b"tree-001"
    token = hashlib.sha256(
        b"dfmcp-tree-harvest-token/1\0"
        + struct.pack(">H", len(key)) + key + plan
    ).digest()[:16]
    vectors.update(capture=bytes(capture), plan=plan, token=token)
    for name, phase, reason in (
        ("prepared", 0, 0),
        ("indeterminate", 1, 5),
        ("designated", 2, 0),
        ("refused", 3, 1),
        ("cancelled", 4, 4),
    ):
        designated = name == "designated"
        raw = (
            b"DFMTR021" + struct.pack(">H", len(key)) + key + plan + token
            + struct.pack(">I", len(capture)) + capture
            + bytes([phase, reason, int(designated)])
        )
        if designated:
            raw += struct.pack(">I", len(after)) + after
        raw += bytes([int(designated)])
        if designated:
            raw += proof
        vectors[name] = raw + hashlib.sha256(
            b"dfmcp-tree-harvest-receipt/1\0" + raw
        ).digest()

    progress = bytearray(vectors["progress_supply"])
    assert len(progress) == 139 and int.from_bytes(progress[81:83], "big") == 2
    progress[75:79] = struct.pack(">I", 44)
    progress[83:111] = struct.pack(">IIIIiiI", 42, 10, 11, 2, 420, 163, 1)
    progress[111:139] = struct.pack(">IIIIiiI", 43, 12, 12, 2, 420, 163, 1)
    vectors["progress_supply"] = bytes(progress)
    return vectors


def file_bytes(directory):
    return {path.name: path.read_bytes() for path in directory.iterdir()
            if path.is_file()}


class WorkshopTreeMaterialTests(unittest.TestCase):
    def setUp(self):
        self.directory = LOGS / self._testMethodName
        self.directory.mkdir(mode=0o700)
        self.tree_directory = self.directory / "tree"
        self.workshop_directory = self.directory / "workshop"
        for directory in (self.tree_directory, self.workshop_directory):
            directory.mkdir(mode=0o700)
            (directory / "custody").mkdir(mode=0o700)
        workshop.VECTORS = {
            name: bytes.fromhex(value)
            for name, value in json.loads(
                (ROOT / "tests/native/workshop/vectors.json").read_text()
            ).items()
        }
        self.clients = []
        self.tree_peer = tree.Peer(self.tree_directory, fixtures())
        self.workshop_peer = workshop.Peer(self.workshop_directory)

    def tearDown(self):
        first = None
        for owner in [*self.clients, self.tree_peer, self.workshop_peer]:
            try:
                owner.close()
            except BaseException as cause:
                if first is None:
                    first = cause
        if first is not None:
            raise first

    def donor(self):
        original_config = tree.config(self.tree_directory, self.tree_peer)
        client = tree.Client(self.tree_directory, original_config)
        self.clients.append(client)
        opened = client.open()
        self.assertTrue(opened["result"]["ok"], opened)
        planned = client.execute_next(opened)
        self.assertTrue(planned["result"]["ok"], planned)
        committed = client.execute_next(planned)
        self.assertTrue(committed["result"]["ok"], committed)
        sampled = client.execute_next(committed)
        self.assertTrue(sampled["result"]["ok"], sampled)
        pin = sampled["result"]["custody"]["acknowledged"]
        closed = client.tool("cancel", session_id=opened["agent_turn"]["session_id"],
                             scope="session")
        self.assertTrue(closed["result"]["ok"], closed)
        client.close()
        self.assertEqual(self.tree_peer.writer_count, 1)
        self.donor_files = file_bytes(self.tree_directory / "custody")
        self.donor_call_count = len(self.tree_peer.calls)

        value = workshop.config(self.workshop_directory, self.workshop_peer)
        value.pop("item_id")
        value["schema"] = "dfmcp.workshop-tree-material-mcp-development/1"
        value["tree_material_source"] = {
            name: original_config[name]
            for name in ("directory", "endpoint", "generation", "root",
                         "scope", "read_scope")
        }
        value["tree_material_source"]["acknowledged"] = pin
        return value

    def client(self, value, suffix="server", credential=True):
        env = workshop.environment(value)
        env["DFMCP_ALLOW_UNADMITTED_WORKSHOP_TREE_MATERIAL"] = "1"
        if credential:
            env["DFMCP_TREE_HARVEST_TOKEN"] = "t" * 32
        client = workshop.Client(self.workshop_directory, value, suffix, env=env)
        self.clients.append(client)
        return client

    def selection(self, opened):
        self.assertTrue(opened["result"]["ok"], opened)
        discovery = opened["result"]["tree_material"]
        self.assertFalse(discovery["harvest_goal_completion_proven"])
        self.assertFalse(discovery["causal_provenance_known"])
        self.assertEqual([offer["item_id"] for offer in discovery["offers"]], [42, 43])
        return {
            "kind": "carpenters_from_tree_supply",
            "expected_observation": discovery["witness"],
            "material": discovery["offers"][0]["material_handle"],
        }

    def plan(self, client, opened, request=None):
        return client.tool(
            "plan", session_id=opened["agent_turn"]["session_id"],
            request=json.dumps(self.selection(opened) if request is None else request),
        )

    def assert_read_only_donor(self):
        self.assertEqual(file_bytes(self.tree_directory / "custody"), self.donor_files)
        self.assertEqual(self.tree_peer.writer_count, 1)
        self.assertFalse({2, 3, 5} & set(self.tree_peer.calls[self.donor_call_count:]))

    def test_harvested_material_builds_and_offline_recovers_original_without_tree(self):
        value = self.donor()
        client = self.client(value)
        opened = client.open()
        planned = self.plan(client, opened)
        self.assertTrue(planned["result"]["ok"], planned)
        original = planned["result"]["original"]
        committed = client.execute_next(planned)
        self.assertTrue(committed["result"]["ok"], committed)
        sampled = client.execute_next(committed)
        self.assertTrue(sampled["result"]["original"]["fresh_completion_proven"], sampled)
        self.assertEqual(sampled["result"]["original"]["plan_digest"],
                         original["plan_digest"])
        self.assert_read_only_donor()
        client.tool("cancel", session_id=opened["agent_turn"]["session_id"], scope="session")
        client.close()
        before = file_bytes(self.workshop_directory / "custody")
        tree_calls, workshop_calls = len(self.tree_peer.calls), len(self.workshop_peer.calls)
        value.update(initialize=False, acknowledged=sampled["result"]["acknowledged"])
        offline = self.client(value, "offline", credential=False)
        restored = offline.open("offline")
        self.assertTrue(restored["result"]["ok"], restored)
        for field in ("key", "plan_digest"):
            self.assertEqual(restored["result"]["original"][field], original[field])
        self.assertEqual(len(self.tree_peer.calls), tree_calls)
        self.assertEqual(len(self.workshop_peer.calls), workshop_calls)
        self.assertEqual(file_bytes(self.workshop_directory / "custody"), before)
        self.assertEqual(self.workshop_peer.writer_count, 1)
        self.assert_read_only_donor()

    def test_forged_and_previous_discovery_handles_never_prepare(self):
        value = self.donor()
        client = self.client(value)
        opened = client.open()
        original = self.selection(opened)
        forged = dict(original)
        suffix = "0" if original["material"][-1] != "0" else "1"
        forged["material"] = original["material"][:-1] + suffix
        rejected = self.plan(client, opened, forged)
        self.assertFalse(rejected["result"]["ok"], rejected)

        progress = bytearray(self.tree_peer.vectors["progress_supply"])
        progress[16:24] = struct.pack(">Q", 2)
        self.tree_peer.vectors["progress_supply"] = bytes(progress)
        observed = client.tool("observe", session_id=opened["agent_turn"]["session_id"])
        self.assertTrue(observed["result"]["ok"], observed)
        self.assertNotEqual(observed["result"]["tree_material"]["witness"],
                            original["expected_observation"])
        rejected = self.plan(client, opened, original)
        self.assertFalse(rejected["result"]["ok"], rejected)
        self.assertNotIn(2, self.workshop_peer.calls)
        self.assertEqual(self.workshop_peer.writer_count, 0)
        self.assert_read_only_donor()

    def test_independent_workshop_material_and_tree_source_drift_refuse(self):
        value = self.donor()
        client = self.client(value)
        opened = client.open()
        request = self.selection(opened)
        before = workshop.VECTORS["capture"]
        capture = bytearray(before)
        self.assertEqual(int.from_bytes(capture[274:278], "big", signed=True), 163)
        capture[274:278] = struct.pack(">i", 164)
        workshop.VECTORS["capture"] = bytes(capture)
        rejected = self.plan(client, opened, request)
        self.assertFalse(rejected["result"]["ok"], rejected)
        self.assertNotIn(2, self.workshop_peer.calls)
        workshop.VECTORS["capture"] = before

        progress = bytearray(self.tree_peer.vectors["progress_supply"])
        progress[8:16] = struct.pack(">Q", 8)
        self.tree_peer.vectors["progress_supply"] = bytes(progress)
        rejected = client.tool("observe", session_id=opened["agent_turn"]["session_id"])
        self.assertFalse(rejected["result"]["ok"], rejected)
        self.assertEqual(self.workshop_peer.writer_count, 0)
        self.assert_read_only_donor()

    def test_consumed_tree_tick_keeps_authority_expired_across_failed_requests(self):
        value = self.donor()
        original_progress = self.tree_peer.vectors["progress_supply"]
        initial_tick = int.from_bytes(original_progress[24:32], "big")
        value["authority_expires_tick"] = initial_tick + 5
        client = self.client(value)
        opened = client.open()
        request = self.selection(opened)
        progress = bytearray(original_progress)
        progress[24:32] = struct.pack(">Q", value["authority_expires_tick"] + 1)
        self.tree_peer.vectors["progress_supply"] = bytes(progress)
        rejected = self.plan(client, opened, request)
        self.assertFalse(rejected["result"]["ok"], rejected)
        tree_calls = len(self.tree_peer.calls)
        workshop_calls = len(self.workshop_peer.calls)
        self.tree_peer.vectors["progress_supply"] = original_progress
        for operation in ("observe", "query"):
            with self.subTest(operation=operation):
                rejected = client.tool(operation, session_id=opened["agent_turn"]["session_id"])
                self.assertFalse(rejected["result"]["ok"], rejected)
                self.assertEqual(len(self.tree_peer.calls), tree_calls)
                self.assertEqual(len(self.workshop_peer.calls), workshop_calls)
        self.assertNotIn(2, self.workshop_peer.calls)
        self.assertEqual(self.workshop_peer.writer_count, 0)
        self.assert_read_only_donor()

    def test_lost_workshop_reply_recovers_same_selected_original_without_tree_reads(self):
        value = self.donor()
        client = self.client(value)
        opened = client.open()
        planned = self.plan(client, opened)
        self.assertTrue(planned["result"]["ok"], planned)
        original = planned["result"]["original"]
        self.workshop_peer.drop_commit = True
        client.send("tools/call", {
            "name": "fortress_commit",
            "arguments": {
                "session_id": opened["agent_turn"]["session_id"],
                "review_seal": original["review_seal"],
                "confirmed": True,
            },
        })
        self.assertTrue(self.workshop_peer.entered.wait(5), self.workshop_peer.errors)
        client.close(crash=True)
        self.workshop_peer.release_commit.set()
        value.update(initialize=False, acknowledged=planned["result"]["acknowledged"])
        tree_calls = len(self.tree_peer.calls)
        recovered = self.client(value, "recovered", credential=False)
        reopened = recovered.open("recover")
        self.assertTrue(reopened["result"]["ok"], reopened)
        resumed = recovered.tool("wait", session_id=reopened["agent_turn"]["session_id"])
        self.assertTrue(resumed["result"]["ok"], resumed)
        for field in ("key", "plan_digest"):
            self.assertEqual(resumed["result"]["original"][field], original[field])
        self.assertEqual(resumed["result"]["original"]["native_phase"], "placed")
        self.assertFalse(resumed["result"]["original"]["prepared_in_this_connection"])
        self.assertEqual(len(self.tree_peer.calls), tree_calls)
        self.assertEqual(self.workshop_peer.writer_count, 1)
        self.assert_read_only_donor()


def main():
    global LOGS
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--workshop-binary", required=True, type=Path)
    parser.add_argument("--tree-binary", required=True, type=Path)
    parser.add_argument("--logs", required=True, type=Path)
    args = parser.parse_args()
    workshop.BINARY = args.workshop_binary.resolve(strict=True)
    tree.BINARY = args.tree_binary.resolve(strict=True)
    LOGS = args.logs
    LOGS.mkdir(mode=0o700)
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(WorkshopTreeMaterialTests)
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    print(json.dumps({
        "schema": "dfmcp.workshop-tree-material-stdio-component/1",
        "tests": result.testsRun, "failed": len(result.failures),
        "errors": len(result.errors), "skipped": len(result.skipped),
        "actual_modern_stdio": True, "loopback_peers_are_doubles": True,
        "live_game": False,
    }))
    raise SystemExit(0 if result.wasSuccessful() and result.testsRun == TEST_COUNT
                     and not result.skipped else 1)


if __name__ == "__main__":
    main()
