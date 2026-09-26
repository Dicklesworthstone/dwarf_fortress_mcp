#!/usr/bin/env python3
"""Drive the compiled furniture MCP process through its real stdio transport.

The native peer is an explicit joined TCP double using the independently built
furniture engine corpus. This executes Rust, private filesystem custody, MCP
routing, and native framing; it is not a DFHack SDK or live-game campaign.

Build the development executable, then pass its absolute path as --binary.
There is no success-by-skipping when that executable is unavailable.
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import selectors
import subprocess
import tempfile
import time
import unittest

from test_build_placement_client import NativeDouble as NativePeer, SECRET, native_record, plan


EXECUTABLE: Path | None = None
SELECTION = '["bed",42,15,15,2]'
TOOLS = {"fortress." + name for name in (
    "open_session", "observe", "query", "plan", "commit", "wait", "cancel",
    "checkpoint", "restore", "explain", "doctor",
)}
META = {
    "io.modelcontextprotocol/protocolVersion": "2026-07-28",
    "io.modelcontextprotocol/clientCapabilities": {"tools": {"listChanged": True}},
    "io.modelcontextprotocol/clientInfo": {"name": "furniture-integration", "version": "1"},
}


class NativeDouble(NativePeer):
    """Preserve the primary DUT failure when the unused peer times out."""

    def __exit__(self, kind, error, trace):
        try:
            super().__exit__(kind, error, trace)
        except AssertionError as cleanup:
            if error is None:
                raise
            error.add_note(f"Native peer cleanup also failed: {cleanup}; {self.error!r}")


def environment(directory, address, *, mode="control", checkpoint="disposable-fortress-no-checkpoint",
                credentials=True):
    values = {k: v for k, v in os.environ.items() if not k.startswith("DFMCP_")}
    values.update({
        "DFMCP_ALLOW_UNADMITTED_BUILD_MCP_V1_19": "1",
        "DFMCP_BUILD_WORLD_FOLDER": "region1",
        "DFMCP_BUILD_SITE_ID": "2",
        "DFMCP_BUILD_SCOPE": "[0,0,0,63,63,7]",
        "DFMCP_BUILD_JOURNAL": str(directory / "journal"),
        "DFMCP_BUILD_ENDPOINT": f"{address[0]}:{address[1]}",
        "DFMCP_BUILD_MODE": mode,
        "DFMCP_BUILD_CHECKPOINT_POLICY": checkpoint,
        "DFMCP_BUILD_PROTECTED": "[]",
    })
    if credentials:
        values["DFMCP_BUILD_TOKEN"] = SECRET.decode()
        values["DFMCP_BUILD_ALLOW_PLACE"] = "1"
    return values


class Stdio:
    """One owned child, bounded reads and complete cleanup even on failure."""

    def __init__(self, env):
        assert EXECUTABLE is not None
        self.errors = tempfile.TemporaryFile()
        self.child = subprocess.Popen([str(EXECUTABLE)], env=env, stdin=subprocess.PIPE,
                                      stdout=subprocess.PIPE, stderr=self.errors, bufsize=0)
        self.selector = selectors.DefaultSelector()
        self.selector.register(self.child.stdout, selectors.EVENT_READ)
        self.buffer = b""
        self.sequence = 0
        self.deadline = time.monotonic() + 40

    def __enter__(self):
        return self

    def __exit__(self, *_args):
        self.selector.close()
        if self.child.poll() is None:
            self.child.kill()
        self.child.wait(timeout=5)
        self.child.stdin.close()
        self.child.stdout.close()
        self.errors.close()

    def request(self, method, params=None):
        self.sequence += 1
        request = {"jsonrpc": "2.0", "id": self.sequence, "method": method,
                   "params": {"_meta": META, **(params or {})}}
        self.child.stdin.write(json.dumps(request, separators=(",", ":")).encode() + b"\n")
        local_deadline = min(self.deadline, time.monotonic() + 10)
        while b"\n" not in self.buffer:
            remaining = local_deadline - time.monotonic()
            if remaining <= 0 or not self.selector.select(remaining):
                raise AssertionError(f"MCP did not answer {method} within its independent deadline")
            chunk = os.read(self.child.stdout.fileno(), 65536)
            if not chunk:
                self.errors.seek(0)
                diagnostic = self.errors.read(4096).decode(errors="replace").replace(SECRET.decode(), "<secret>")
                raise AssertionError(f"MCP process closed stdout during {method}: {diagnostic}")
            self.buffer += chunk
            if len(self.buffer) > 1_048_576:
                raise AssertionError("MCP response exceeded the independent 1 MiB transport bound")
        line, self.buffer = self.buffer.split(b"\n", 1)
        response = json.loads(line)
        assert response.get("jsonrpc") == "2.0" and response.get("id") == self.sequence, response
        assert "error" not in response, response
        assert SECRET.decode() not in line.decode(), "credential leaked into MCP output"
        return response["result"]

    def tools(self):
        discover = self.request("server/discover")
        assert discover["supportedVersions"] == ["2026-07-28"], discover
        listing = self.request("tools/list")
        assert {x["name"] for x in listing["tools"]} == TOOLS, listing
        return listing

    def call(self, tool, **arguments):
        result = self.request("tools/call", {"name": "fortress." + tool, "arguments": arguments})
        texts = [v["text"] for v in result["content"] if v.get("type") == "text"]
        assert len(texts) == 1, result
        packet = json.loads(texts[0])
        assert packet["agent_turn"]["schema"] == "dfmcp.agent_turn/1", packet
        assert packet["agent_turn"]["anchor"] is None, "native identity must not masquerade as canonical state"
        assert "active_work" in packet["agent_turn"], packet
        return packet


class FurnitureMcpTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name)
        self.directory.chmod(0o700)

    def open(self, client, *, online=True):
        packet = client.call("open_session", **({"selection": SELECTION} if online else {}),
                             max_wall_millis=60000)
        self.assertTrue(packet["result"]["ok"], packet)
        self.assertFalse(packet["result"]["game_mutation_dispatched"])
        return packet["result"]["session_id"]

    def prepare(self, client, session, key="golden"):
        observed = client.call("observe", session_id=session, selection=SELECTION)
        self.assertTrue(observed["result"]["ok"], observed)
        witness = observed["result"]["observation"]["observation_witness"]
        self.assertEqual(witness, plan(key).before.witness.hex())
        prepared = client.call("plan", session_id=session, idempotency_key=key,
                               observation_witness=witness)
        self.assertTrue(prepared["result"]["ok"], prepared)
        self.assertEqual(prepared["result"]["plan_digest"], plan(key).digest.hex())
        self.assertIsInstance(prepared["result"]["review_seal"], str)
        return {"session_id": session, "idempotency_key": key,
                "plan_digest": prepared["result"]["plan_digest"],
                "review_seal": prepared["result"]["review_seal"]}

    def effect(self, packet, phase):
        self.assertTrue(packet["result"]["ok"], packet)
        record = packet["result"]["effect"]
        self.assertEqual(record["summary"]["native_phase"], phase)
        self.assertFalse(record["building_completion_proven"])
        self.assertFalse(record["current_building_usable_proven"])
        self.assertFalse(record["retry_commit_permitted"])
        self.assertEqual(record["native"]["canonical_record_hex"], native_record(phase=phase).hex())
        if phase == "placed":
            insertion = record["native"]["insertion"]
            self.assertEqual((insertion["building_id"], insertion["job_id"], insertion["item_id"]),
                             (70, 90, 42))
            self.assertEqual((insertion["stage"], insertion["max_stage"]), (0, 1))
            self.assertTrue(insertion["historical_evidence_only"])
        else:
            self.assertIsNone(record["native"]["insertion"])
        return record

    def test_real_stdio_review_commit_replay_and_offline_evidence(self):
        with NativeDouble(connections=2) as peer:
            with Stdio(environment(self.directory, peer.address)) as client:
                client.tools()
                session = self.open(client)
                review = self.prepare(client, session)
                self.effect(client.call("commit", **review), "placed")
                replay = client.call("commit", **review)
                self.effect(replay, "placed")
                self.assertEqual(replay["result"]["native_calls"], 0)
                inventory = client.call("query", session_id=session,
                                        query='{"mode":"records","limit":8}')
                self.assertEqual(inventory["result"]["journal"]["total_records"], 1)
                self.assertEqual(inventory["result"]["journal"]["unresolved_records"], 0)
                self.assertTrue(inventory["agent_turn"]["active_work"]["pending_absence_proven"])
            self.assertEqual(peer.effects, 1)
            self.assertEqual(peer.calls.count("CommitPlacement"), 1)
            self.assertEqual(peer.calls.count("PreparePlacement"), 1)
            self.assertEqual(peer.calls.count("Handshake"), 2)
        original = (self.directory / "journal").read_bytes()
        with Stdio(environment(self.directory, peer.address, mode="offline", credentials=False)) as client:
            session = self.open(client, online=False)
            record = client.call("explain", session_id=session, idempotency_key="golden",
                                 plan_digest=plan().digest.hex())
            self.assertEqual(record["result"]["record"]["native"]["phase"], "placed")
            self.assertEqual(record["result"]["native_calls"], 0)
            denied = client.call("observe", session_id=session, selection=SELECTION)
            self.assertFalse(denied["result"]["ok"])
        self.assertEqual((self.directory / "journal").read_bytes(), original)

    def test_lost_commit_restarts_with_query_and_never_repeats_writer(self):
        with NativeDouble(connections=3, lose_on="CommitPlacement") as peer:
            with Stdio(environment(self.directory, peer.address)) as client:
                session = self.open(client)
                review = self.prepare(client, session)
                lost = client.call("commit", **review)
                self.assertFalse(lost["result"]["ok"], lost)
                self.assertFalse(lost["result"]["retry_commit_permitted"])
                self.assertEqual(len(lost["agent_turn"]["active_work"]["pending_plans"]), 1)
            with Stdio(environment(self.directory, peer.address, mode="recover")) as client:
                session = self.open(client, online=False)
                retry = client.call("commit", **{**review, "session_id": session})
                self.assertFalse(retry["result"]["ok"], retry)
                recovered = client.call("wait", session_id=session, idempotency_key="golden",
                                        plan_digest=plan().digest.hex())
                self.effect(recovered, "placed")
            self.assertEqual(peer.effects, 1)
            self.assertEqual(peer.calls.count("CommitPlacement"), 1)
            self.assertEqual(peer.calls[-1], "QueryPlacement")

    def test_reopened_preparation_can_retire_with_query_only_authority(self):
        with NativeDouble(connections=3) as peer:
            with Stdio(environment(self.directory, peer.address)) as client:
                review = self.prepare(client, self.open(client))
            env = environment(self.directory, peer.address, mode="recover", checkpoint="required")
            del env["DFMCP_BUILD_ALLOW_PLACE"]
            with Stdio(env) as client:
                session = self.open(client, online=False)
                denied = client.call("commit", **{**review, "session_id": session})
                self.assertFalse(denied["result"]["ok"])
                retired = client.call("cancel", session_id=session, scope="effect",
                                      idempotency_key="golden", plan_digest=plan().digest.hex())
                self.effect(retired, "cancelled")
            self.assertEqual(peer.effects, 0)
            self.assertNotIn("CommitPlacement", peer.calls)
            self.assertEqual(peer.calls[-1], "CancelPlacement")

    def test_required_checkpoint_and_protected_item_refuse_before_native_prepare(self):
        for policy in ("required", "protected_item"):
            with self.subTest(policy=policy), tempfile.TemporaryDirectory(dir=self.directory) as raw:
                directory = Path(raw)
                directory.chmod(0o700)
                with NativeDouble(connections=2) as peer:
                    env = environment(directory, peer.address, checkpoint=("required" if policy == "required"
                                      else "disposable-fortress-no-checkpoint"))
                    if policy == "protected_item":
                        env["DFMCP_BUILD_PROTECTED"] = "[[10,11,2,10,11,2]]"
                    with Stdio(env) as client:
                        session = self.open(client)
                        observed = client.call("observe", session_id=session, selection=SELECTION)
                        self.assertTrue(observed["result"]["ok"], observed)
                        refused = client.call("plan", session_id=session, idempotency_key="golden",
                                              observation_witness=plan().before.witness.hex())
                        self.assertFalse(refused["result"]["ok"], refused)
                        self.assertEqual(refused["result"]["error"]["code"],
                                         "checkpoint_required" if policy == "required" else "capability_denied")
                        inventory = client.call("doctor", session_id=session)
                        self.assertEqual(inventory["result"]["journal"]["total_records"], 0)
                    self.assertNotIn("PreparePlacement", peer.calls)
                    self.assertEqual(peer.effects, 0)

    def test_indeterminate_history_survives_offline_restart_and_blocks_new_keys(self):
        with NativeDouble(connections=2, outcome="indeterminate") as peer:
            with Stdio(environment(self.directory, peer.address)) as client:
                session = self.open(client)
                review = self.prepare(client, session)
                uncertain = client.call("commit", **review)
                record = self.effect(uncertain, "indeterminate")
                self.assertEqual(record["summary"]["state"], "indeterminate")
                self.assertEqual(record["summary"]["coordinator_state"], "terminal")
                active = uncertain["agent_turn"]["active_work"]
                self.assertFalse(active["pending_absence_proven"])
                self.assertEqual(len(active["indeterminate_effects"]), 1)
            self.assertEqual(peer.effects, 1)
        original = (self.directory / "journal").read_bytes()
        with Stdio(environment(self.directory, peer.address, mode="offline", credentials=False)) as client:
            session = self.open(client, online=False)
            recovered = client.call("wait", session_id=session, idempotency_key="golden",
                                    plan_digest=plan().digest.hex())
            self.effect(recovered, "indeterminate")
            denied = client.call("plan", session_id=session, idempotency_key="changed-key",
                                 observation_witness=plan().before.witness.hex())
            self.assertFalse(denied["result"]["ok"])
            self.assertFalse(denied["agent_turn"]["active_work"]["pending_absence_proven"])
        self.assertEqual((self.directory / "journal").read_bytes(), original)

    def test_closed_query_shapes_keep_pending_identity_visible(self):
        with NativeDouble(connections=2) as peer:
            with Stdio(environment(self.directory, peer.address)) as client:
                session = self.open(client)
                self.prepare(client, session)
                for raw in ('{"mode":"records","limit":0}', '{"mode":"records","limit":1.0}',
                            '{"mode":"records","limit":null}', '{"mode":"records","limit":1,"limit":2}',
                            '{"mode":"records","path":"/private/elsewhere"}',
                            '{"mode":"schema","path":"/private/elsewhere"}',
                            '{"mode":"records","offset":1}'):
                    with self.subTest(query=raw):
                        refused = client.call("query", session_id=session, query=raw)
                        self.assertFalse(refused["result"]["ok"], refused)
                        self.assertEqual(len(refused["agent_turn"]["active_work"]["pending_plans"]), 1)
                released = client.call("cancel", session_id=session, scope="session", release_for_recovery=True)
                self.assertTrue(released["result"]["ok"], released)
                self.assertFalse(released["result"]["history_erased"])
            self.assertEqual(peer.effects, 0)

    def test_native_global_fence_never_strands_a_fresh_local_intent(self):
        for timing in ("already_fenced", "fenced_during_query"):
            with self.subTest(timing=timing), tempfile.TemporaryDirectory(dir=self.directory) as raw:
                directory = Path(raw)
                directory.chmod(0o700)
                def hook(name, _request, response):
                    if timing == "fenced_during_query" and name == "QueryPlacement":
                        response[12], response[13] = 1, 1
                    return response
                retained = ({"other-key": native_record("other-key", "indeterminate")}
                            if timing == "already_fenced" else {})
                with NativeDouble(connections=2, retained=retained, hook=hook) as peer:
                    with Stdio(environment(directory, peer.address)) as client:
                        session = self.open(client)
                        observed = client.call("observe", session_id=session, selection=SELECTION)
                        self.assertTrue(observed["result"]["ok"], observed)
                        self.assertEqual(observed["result"]["source_summary"]["unresolved"],
                                         timing == "already_fenced")
                        original = (directory / "journal").read_bytes()
                        refused = client.call("plan", session_id=session, idempotency_key="golden",
                                              observation_witness=plan().before.witness.hex())
                        self.assertFalse(refused["result"]["ok"], refused)
                        self.assertEqual(refused["result"]["error"]["code"], "effect_indeterminate")
                        self.assertTrue(refused["result"]["source_summary"]["unresolved"])
                        self.assertFalse(refused["result"]["source_summary"]["prepare_available"])
                        self.assertFalse(refused["result"]["new_local_obligation_created"])
                        self.assertEqual((directory / "journal").read_bytes(), original)
                        inventory = client.call("doctor", session_id=session)
                        self.assertEqual(inventory["result"]["journal"]["total_records"], 0)
                    self.assertNotIn("PreparePlacement", peer.calls)
                    self.assertNotIn("CommitPlacement", peer.calls)
                    self.assertEqual(peer.effects, 0)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    args, remaining = parser.parse_known_args()
    global EXECUTABLE
    EXECUTABLE = args.binary.resolve(strict=True)
    if not EXECUTABLE.is_file() or not os.access(EXECUTABLE, os.X_OK):
        parser.error("--binary must name an executable regular file")
    unittest.main(argv=[__file__, *remaining])


if __name__ == "__main__":
    main()
