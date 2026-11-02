#!/usr/bin/env python3
"""Actual modern stdio consumer with joined loopback peers and real private files.

Peers use unchanged native component vectors and independent key commitments.
This exercises Rust handlers/transport/custody, never a real game or native R1.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import selectors
import socket
import struct
import subprocess
import threading
import time
import unittest

ROOT = Path(__file__).resolve().parents[2]
META = {"io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": {"tools": {"listChanged": True}},
        "io.modelcontextprotocol/clientInfo": {"name": "workshop-component", "version": "1"}}
METHODS = ["Handshake", "ReadWorkshop", "PrepareWorkshop", "CommitWorkshop", "QueryWorkshop", "CancelWorkshop", "ReadWorkshopProgress"]
BINARY: Path
SERVER_ARGS: list[str] = []
LOGS: Path
VECTORS = {}


def varint(n):
    out = bytearray()
    while n > 127:
        out.append((n & 127) | 128)
        n >>= 7
    out.append(n)
    return bytes(out)


def number(tag, n):
    return varint(tag << 3) + varint(n)


def blob(tag, value):
    return varint((tag << 3) | 2) + varint(len(value)) + value


def fields(raw):
    at = 0
    out = {}
    def take():
        nonlocal at
        n = shift = 0
        while True:
            b = raw[at]
            at += 1
            n |= (b & 127) << shift
            if b < 128:
                return n
            shift += 7
            assert shift < 70
    while at < len(raw):
        key = take()
        tag, kind = key >> 3, key & 7
        assert tag not in out
        if kind == 0:
            out[tag] = take()
        else:
            assert kind == 2
            length = take()
            out[tag] = raw[at:at + length]
            assert len(out[tag]) == length
            at += length
    return out


def keyed(name, key):
    original = VECTORS[name]
    old_len = int.from_bytes(original[8:10], "big")
    key = key.encode()
    raw = bytearray(original[:8] + len(key).to_bytes(2, "big") + key + original[10 + old_len:])
    capture_at = 10 + len(key)
    capture_size = int.from_bytes(raw[capture_at:capture_at + 2], "big")
    digest_at = capture_at + 2 + capture_size
    plan = bytes(raw[digest_at:digest_at + 32])
    token = hashlib.sha256(b"dfmcp-workshop-token/1\0" + len(key).to_bytes(2, "big") + key + plan).digest()[:16]
    raw[digest_at + 32:digest_at + 48] = token
    raw[-32:] = hashlib.sha256(b"dfmcp-workshop-receipt/1\0" + raw[:-32]).digest()
    return bytes(raw)


def receive(stream, n):
    out = bytearray()
    while len(out) < n:
        value = stream.recv(n - len(out))
        if not value:
            raise EOFError
        out.extend(value)
    return bytes(out)


class Peer:
    def __init__(self, directory):
        self.directory = directory
        self.listener = socket.socket()
        self.listener.bind(("127.0.0.1", 0))
        self.listener.listen(4)
        self.listener.settimeout(.1)
        self.request_timeout = 5
        self.endpoint = f"127.0.0.1:{self.listener.getsockname()[1]}"
        self.calls = []
        self.errors = []
        self.stop = threading.Event()
        self.entered = threading.Event()
        self.release_commit = threading.Event()
        self.record = None
        self.key = None
        self.stale = None
        self.drop_commit = False
        self.active = None
        self.writer_count = 0
        self.thread = threading.Thread(target=self.run, name="owned-workshop-peer")
        self.thread.start()

    def reply(self, stream, raw):
        stream.sendall(struct.pack("<hhI", -1, 0, len(raw)) + raw)

    def run(self):
        try:
            while not self.stop.is_set():
                try:
                    stream, _ = self.listener.accept()
                except socket.timeout:
                    continue
                self.active = stream
                with stream:
                    stream.settimeout(self.request_timeout)
                    assert receive(stream, 12) == b"DFHack?\n\x01\0\0\0"
                    stream.sendall(b"DFHack!\n\x01\0\0\0")
                    bound = 0
                    while not self.stop.is_set():
                        try:
                            method, pad, size = struct.unpack("<hhI", receive(stream, 8))
                            assert pad == 0 and size <= 2048
                            request = fields(receive(stream, size))
                        except EOFError:
                            break
                        if method == 0:
                            assert request == {1: METHODS[bound].encode(), 2: b"dfmcp.workshop.v1_20.Request", 3: b"dfmcp.workshop.v1_20.Reply", 4: b"dfmcp_workshop_v1_20"}
                            self.reply(stream, number(1, bound + 2))
                            bound += 1
                            continue
                        assert bound == 7
                        op = method - 2
                        assert 0 <= op < 7 and request[1] == b"t" * 32 and len(request[2]) == 32 and request[3] == 1 and request[4] == 20
                        self.calls.append(op)
                        with (self.directory / "native-calls.jsonl").open("a") as log:
                            log.write(json.dumps({"op": METHODS[op], "fields": sorted(request)}) + "\n")
                        generation = int.from_bytes(VECTORS["capture"][8:16], "big")
                        raw = number(1, 1) + number(2, 0) + blob(3, request[2]) + number(4, 1) + number(5, 20)
                        if op == 1 and self.record and self.stale == "source":
                            generation += 1
                        raw += number(6, generation) + blob(7, b"test-df") + blob(8, b"test-dfhack")
                        raw += number(12, 0) + number(13, int(self.record is not None))
                        if op == 1:
                            assert set(request) == {1, 2, 3, 4, 5, 6, 7, 8}
                            assert [request[t] for t in (5, 6, 7, 8)] == [42, 15, 15, 2]
                            capture = bytearray(VECTORS["capture"])
                            if self.record and self.stale:
                                if self.stale == "worker":
                                    capture[-2] = 0
                                elif self.stale == "footprint":
                                    capture[94 + 5] = 4
                                elif self.stale == "material":
                                    capture[248 + 12 + 2 + 4 + 4 + 4 + 3] ^= 1
                            raw += blob(9, capture)
                        elif op == 2:
                            assert set(request) == set(range(1, 12))
                            assert request[10] == hashlib.sha256(VECTORS["capture"]).digest() and request[11] == VECTORS["plan"]
                            self.key = request[9].decode()
                            self.record = "prepared"
                            raw = raw[:-(len(number(13, 0)))] + number(13, 1)
                            raw += blob(10, keyed("prepared", self.key)) + number(11, 0)
                        elif op == 3:
                            assert set(request) == {1, 2, 3, 4, 9, 11, 12}
                            assert request[9].decode() == self.key and request[11] == VECTORS["plan"]
                            # The independent acknowledgement must already cover
                            # DispatchStarted when a real commit frame arrives.
                            ack = (self.directory / "custody/workshop.ack").read_bytes()
                            assert len(ack) >= 140 and len(ack) % 140 == 0
                            ack_frames = int.from_bytes(ack[-140 + 72:-140 + 76], "big")
                            assert ack_frames >= 3
                            self.writer_count += 1
                            assert self.writer_count == 1
                            self.record = "placed"
                            self.entered.set()
                            if self.drop_commit:
                                while not self.release_commit.wait(.01):
                                    if self.stop.is_set():
                                        return
                                break
                            raw += blob(10, keyed("placed", self.key))
                        elif op == 4:
                            assert set(request) == {1, 2, 3, 4, 9, 11}
                            if self.record:
                                raw += blob(10, keyed(self.record, request[9].decode()))
                        elif op == 5:
                            self.record = "cancelled"
                            raw += blob(10, keyed("cancelled", request[9].decode()))
                        elif op == 6:
                            assert set(request) == {1, 2, 3, 4, 5, 13} and request[5] == 42 and request[13] == 70
                            raw += blob(9, VECTORS["progress_complete"])
                        self.reply(stream, raw)
                self.active = None
        except (OSError, EOFError) as cause:
            if not self.stop.is_set():
                self.errors.append(repr(cause))
        except BaseException as cause:
            self.errors.append(repr(cause))
        finally:
            self.listener.close()

    def close(self):
        self.stop.set()
        if self.active is not None:
            try:
                self.active.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
        self.thread.join(6)
        assert not self.thread.is_alive(), "owned peer failed to drain"
        assert not self.errors, self.errors


def config(directory, peer, **changes):
    capture = VECTORS["capture"]
    value = {"schema": "dfmcp.workshop-mcp-development/1", "directory": str(directory / "custody"),
             "endpoint": peer.endpoint, "initialize": True, "acknowledged": None,
             "world_folder": "region1", "site_id": int.from_bytes(capture[32:36], "big"),
             "df_version": "test-df", "dfhack_version": "test-dfhack", "generation": int.from_bytes(capture[8:16], "big"),
             "map_dimensions": [int.from_bytes(capture[i:i + 4], "big") for i in (36, 40, 44)],
             "item_id": 42, "center": [15, 15, 2], "scope": [0, 0, 0, 63, 63, 3], "protected": [],
             "authority_expires_tick": 1000000, "lease_ticks": 1200,
             "checkpoint": "disposable-fortress-no-checkpoint"}
    value.update(changes)
    return value


def environment(value):
    env = {k: v for k, v in os.environ.items() if not k.startswith("DFMCP_")}
    env.update(DFMCP_ALLOW_UNADMITTED_WORKSHOP_V1_20="1", DFMCP_WORKSHOP_ALLOW_BUILD="1",
               DFMCP_WORKSHOP_TOKEN="t" * 32, DFMCP_WORKSHOP_MCP_CONFIG=json.dumps(value))
    return env


class Client:
    def __init__(self, directory, value, suffix="server", env=None):
        self.directory = directory
        self.stderr = (directory / f"{suffix}.stderr").open("xb")
        self.stdout = (directory / f"{suffix}.stdout").open("xb")
        self.requests = (directory / f"{suffix}.requests").open("xb")
        self.process = subprocess.Popen([str(BINARY), *SERVER_ARGS], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        stderr=self.stderr, env=environment(value) if env is None else env)
        self.n = 0
        self.closed = False
        self.pending = bytearray()
        self.selector = selectors.DefaultSelector()
        self.selector.register(self.process.stdout, selectors.EVENT_READ)

    def send(self, method, params):
        self.n += 1
        request = {"jsonrpc": "2.0", "id": self.n, "method": method, "params": {**params, "_meta": META}}
        raw = json.dumps(request).encode() + b"\n"
        self.requests.write(raw)
        self.requests.flush()
        self.process.stdin.write(raw)
        self.process.stdin.flush()

    def call(self, method, params):
        self.send(method, params)
        end = time.monotonic() + 15
        while time.monotonic() < end:
            while b"\n" not in self.pending:
                assert self.selector.select(max(.01, end - time.monotonic())), "modern response timed out"
                chunk = os.read(self.process.stdout.fileno(), 65536)
                assert chunk, "modern response missing"
                self.pending.extend(chunk)
                assert len(self.pending) < 512 * 1024, "modern buffered response bound exceeded"
            raw, _, rest = self.pending.partition(b"\n")
            self.pending = bytearray(rest)
            raw = bytes(raw) + b"\n"
            assert len(raw) < 256 * 1024, "modern response oversized"
            self.stdout.write(raw)
            self.stdout.flush()
            response = json.loads(raw)
            if response.get("id") == self.n:
                return response
        raise AssertionError("no matching modern response")

    def tool(self, name, **arguments):
        response = self.call("tools/call", {"name": "fortress_" + name, "arguments": arguments})
        assert "result" in response, response
        return json.loads(response["result"]["content"][0]["text"])

    def execute_next(self, packet):
        step = packet["agent_turn"]["recommendations"][0]
        assert step["tool"].startswith("fortress_")
        return self.tool(step["tool"][len("fortress_"):], **step["arguments"])

    def open(self, mode="control", max_wall_millis=None):
        self.call("server/discover", {})
        tools = self.call("tools/list", {})["result"]["tools"]
        assert len(tools) == 11
        arguments = {"mode": mode}
        if max_wall_millis is not None:
            arguments["max_wall_millis"] = max_wall_millis
        return self.tool("open_session", **arguments)

    def close(self, crash=False):
        if self.closed:
            return
        self.closed = True
        if self.process.poll() is None:
            if crash:
                self.process.kill()
            else:
                self.process.stdin.close()
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=5)
        remainder = bytes(self.pending) + self.process.stdout.read()
        self.stdout.write(remainder)
        self.stdout.close()
        self.stderr.close()
        self.requests.close()
        self.selector.close()
        (self.directory / f"exit-{self.process.pid}").write_text(str(self.process.returncode) + "\n")


class WorkshopStdioTests(unittest.TestCase):
    def setUp(self):
        self.directory = LOGS / self._testMethodName
        self.directory.mkdir(mode=0o700)
        (self.directory / "custody").mkdir(mode=0o700)
        self.peer = Peer(self.directory)
        self.clients = []

    def tearDown(self):
        for client in self.clients:
            client.close()
        self.peer.close()

    def client(self, value=None, suffix="server"):
        client = Client(self.directory, config(self.directory, self.peer) if value is None else value, suffix)
        self.clients.append(client)
        return client

    def prepared(self, client, **open_arguments):
        opened = client.open(**open_arguments)
        self.assertTrue(opened["result"]["ok"], opened)
        sid = opened["agent_turn"]["session_id"]
        source = opened["result"]["observation"]
        self.assertEqual(opened["agent_turn"]["recommendations"][0]["tool"], "fortress_plan")
        expected_request = json.loads(opened["agent_turn"]["recommendations"][0]["arguments"]["request"])
        self.assertEqual(expected_request, {"kind": "carpenters", "expected_observation": source["witness"], "material": source["material_handle"]})
        planned = client.execute_next(opened)
        self.assertTrue(planned["result"]["ok"], planned)
        self.assertIsNone(planned["agent_turn"]["anchor"])
        self.assertFalse(planned["agent_turn"]["continuity"]["gap"]["canonical_world_anchor_available"])
        self.assertIsNotNone(planned["agent_turn"]["request_id"])
        original = planned["result"]["original"]
        self.assertEqual(planned["agent_turn"]["recommendations"][0]["arguments"], {"session_id": sid, "review_seal": original["review_seal"], "confirmed": True})
        self.assertEqual(planned["agent_turn"]["recommendations"][0]["tool"], "fortress_commit")
        self.assertTrue(original["source"]["worker_census_complete"])
        self.assertTrue(all(w["labor_valid"] and w["job_available"] for w in original["source"]["workers"]))
        return sid, original, planned["result"]["acknowledged"], planned

    def test_modern_stdio_original_workshop_positive_and_offline_recovery(self):
        client = self.client()
        sid, original, _, planned = self.prepared(client)
        committed = client.execute_next(planned)
        self.assertTrue(committed["result"]["ok"], committed)
        self.assertEqual(committed["result"]["original"]["native_phase"], "placed")
        self.assertFalse(committed["result"]["current_goal_available"])
        self.assertEqual(committed["agent_turn"]["recommendations"][0]["tool"], "fortress_wait")
        sampled = client.execute_next(committed)
        self.assertTrue(sampled["result"]["original"]["fresh_completion_proven"], sampled)
        self.assertEqual(sampled["agent_turn"]["recommendations"][0]["tool"], "fortress_explain")
        explained = client.execute_next(sampled)
        self.assertTrue(explained["result"]["ok"], explained)
        calls = len(self.peer.calls)
        historical = client.tool("wait", session_id=sid)
        self.assertFalse(historical["result"]["current_goal_available"])
        self.assertEqual(historical["result"]["original"]["historical_goal"], "complete")
        # Repeated wait may authenticate, but cannot relabel cached completion fresh.
        self.assertNotIn(6, self.peer.calls[calls:])
        pin = historical["result"]["acknowledged"]
        closed = client.tool("cancel", session_id=sid, scope="session")
        self.assertEqual(closed["agent_turn"]["recommendations"], [])
        client.close()
        before = {p.name: p.read_bytes() for p in (self.directory / "custody").iterdir()}
        value = config(self.directory, self.peer, initialize=False, acknowledged=pin)
        offline = self.client(value, "offline")
        opened = offline.open("offline")
        self.assertTrue(opened["result"]["ok"], opened)
        self.assertEqual(opened["result"]["original"]["plan_digest"], original["plan_digest"])
        self.assertFalse(opened["result"]["current_goal_available"])
        self.assertEqual(before, {p.name: p.read_bytes() for p in (self.directory / "custody").iterdir()})
        self.assertEqual(self.peer.writer_count, 1)

    def test_modern_stdio_stale_footprint_material_worker_and_source_refuse_writer(self):
        # Each independent original goal uses its own process and custody.
        for kind in ("footprint", "material", "worker", "source"):
            with self.subTest(kind=kind):
                directory = self.directory / kind
                directory.mkdir(mode=0o700)
                (directory / "custody").mkdir(mode=0o700)
                peer = Peer(directory)
                client = Client(directory, config(directory, peer))
                try:
                    sid, original, _, _ = self.prepared(client)
                    peer.stale = kind
                    result = client.tool("commit", session_id=sid, review_seal=original["review_seal"], confirmed=True)
                    self.assertFalse(result["result"]["ok"], result)
                    self.assertFalse(result["result"]["current_goal_available"])
                    self.assertEqual(peer.writer_count, 0)
                    self.assertNotIn(3, peer.calls)
                    cancelled = client.tool("cancel", session_id=sid, scope="goal")
                    self.assertTrue(cancelled["result"]["ok"], cancelled)
                    self.assertEqual(cancelled["result"]["original"]["native_phase"], "cancelled")
                    self.assertEqual(cancelled["agent_turn"]["recommendations"][0]["tool"], "fortress_explain")
                    self.assertIn("cancelled", cancelled["agent_turn"]["recommendations"][0]["reason"])
                    self.assertTrue(client.execute_next(cancelled)["result"]["ok"])
                finally:
                    client.close()
                    peer.close()

    def test_modern_stdio_crash_pins_dispatch_before_writer_and_rejects_prefix_rollback(self):
        client = self.client()
        sid, original, external, _ = self.prepared(client)
        journal = self.directory / "custody/workshop.journal"
        prefix = journal.read_bytes()
        (self.directory / "prepared-prefix.bin").write_bytes(prefix)
        self.peer.drop_commit = True
        client.send("tools/call", {"name": "fortress_commit", "arguments": {"session_id": sid, "review_seal": original["review_seal"], "confirmed": True}})
        self.assertTrue(self.peer.entered.wait(10), self.peer.errors)
        client.close(crash=True)
        self.peer.release_commit.set()
        self.assertGreater(len(journal.read_bytes()), len(prefix))
        (self.directory / "dispatch-journal.bin").write_bytes(journal.read_bytes())
        value = config(self.directory, self.peer, initialize=False, acknowledged=external)
        legitimate = self.client(value, "legitimate-restart")
        opened = legitimate.open("recover")
        self.assertTrue(opened["result"]["ok"], opened)
        resumed = legitimate.tool("wait", session_id=opened["agent_turn"]["session_id"])
        self.assertTrue(resumed["result"]["ok"], resumed)
        self.assertEqual(resumed["result"]["original"]["native_phase"], "placed")
        self.assertFalse(resumed["result"]["current_goal_available"])
        legitimate.close()
        self.assertEqual(self.peer.writer_count, 1)
        # A valid original Prepared prefix cannot erase independently retained
        # DispatchStarted, even when the native double loses its retained table.
        (self.directory / "settled-journal.bin").write_bytes(journal.read_bytes())
        journal.write_bytes(prefix)
        self.peer.record = None
        self.peer.key = None
        calls = len(self.peer.calls)
        recovered = self.client(value, "rollback")
        opened = recovered.open("recover")
        self.assertFalse(opened["result"]["ok"], opened)
        self.assertEqual(opened["result"]["error"]["code"], "corrupt_ledger")
        self.assertEqual(self.peer.calls[calls:], [])
        self.assertEqual(self.peer.writer_count, 1)

    def test_modern_stdio_closed_requests_authority_and_scope_refuse_writer(self):
        client = self.client()
        opened = client.open()
        self.assertTrue(opened["result"]["ok"], opened)
        sid = opened["agent_turn"]["session_id"]
        source = opened["result"]["observation"]
        result = client.tool("plan", session_id=sid, request=json.dumps({"kind": "carpenters", "expected_observation": source["witness"], "material": source["material_handle"], "native_address": 1}))
        self.assertFalse(result["result"]["ok"])
        self.assertNotIn(2, self.peer.calls)
        self.assertNotIn(3, self.peer.calls)
        self.assertFalse(result["result"]["current_goal_available"])
        inspected = client.tool("query", session_id=sid)
        self.assertEqual(inspected["agent_turn"]["recommendations"][0]["tool"], "fortress_observe")
        reacquired = client.execute_next(inspected)
        self.assertTrue(reacquired["result"]["ok"], reacquired)
        self.assertEqual(reacquired["agent_turn"]["recommendations"][0]["tool"], "fortress_plan")
        self.assertNotIn(2, self.peer.calls)
        for name, changes in (
            ("expired-authority", {"authority_expires_tick": 1}),
            ("material-outside-scope", {"scope": [14, 14, 2, 16, 16, 2]}),
            ("protected-footprint", {"protected": [[14, 14, 2, 16, 16, 2]]}),
            ("checkpoint-required", {"checkpoint": "required"}),
        ):
            with self.subTest(name=name):
                directory = self.directory / name
                directory.mkdir(mode=0o700)
                (directory / "custody").mkdir(mode=0o700)
                peer = Peer(directory)
                child = Client(directory, config(directory, peer, **changes))
                try:
                    opened = child.open()
                    if name == "expired-authority":
                        self.assertFalse(opened["result"]["ok"], opened)
                    else:
                        self.assertTrue(opened["result"]["ok"], opened)
                        source = opened["result"]["observation"]
                        result = child.tool("plan", session_id=opened["agent_turn"]["session_id"],
                            request=json.dumps({"kind": "carpenters", "expected_observation": source["witness"], "material": source["material_handle"]}))
                        self.assertFalse(result["result"]["ok"], result)
                    self.assertNotIn(2, peer.calls)
                    self.assertNotIn(3, peer.calls)
                    self.assertEqual(peer.writer_count, 0)
                finally:
                    child.close()
                    peer.close()

    def test_modern_stdio_production_environment_is_rejected(self):
        for marker in ("", "1.0", "1.20"):
            env = environment(config(self.directory, self.peer))
            env["DFMCP_ADMITTED_BRIDGE_PROTOCOL"] = marker
            result = subprocess.run([str(BINARY), *SERVER_ARGS], input=b"", capture_output=True, env=env, timeout=5)
            stem = self.directory / f"production-{marker or 'empty'}"
            Path(str(stem) + ".stdout").write_bytes(result.stdout)
            Path(str(stem) + ".stderr").write_bytes(result.stderr)
            Path(str(stem) + ".exit").write_text(str(result.returncode) + "\n")
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(result.stdout, b"")
            self.assertIn(b"production", result.stderr)
        self.assertEqual(self.peer.calls, [])


    def test_default_budget_allows_delayed_review_on_original_connection(self):
        # A real agent took >9s to review an S plan. Omit the wall argument:
        # explicit 60000 in older tests masked the unusable 5000ms default.
        self.peer.request_timeout = 15
        client = self.client()
        sid, original, _, planned = self.prepared(client)
        start = time.monotonic()
        time.sleep(5.2)
        self.assertGreater(time.monotonic() - start, 5)
        committed = client.execute_next(planned)
        self.assertTrue(committed["result"]["ok"], committed)
        self.assertEqual(committed["result"]["original"]["native_phase"], 'placed')
        self.assertEqual(self.peer.writer_count, 1)
        self.assertEqual(self.peer.calls.count(2), 1)
        self.assertEqual(self.peer.calls.count(3), 1)

    def test_expired_preparation_does_not_record_or_send_dispatch(self):
        self.peer.request_timeout = 15
        client = self.client()
        sid, original, _, planned = self.prepared(client, max_wall_millis=5000)
        custody = self.directory / "custody"
        before = {p.name: p.read_bytes() for p in custody.iterdir()}
        time.sleep(5.2)
        failed = client.execute_next(planned)
        self.assertFalse(failed["result"]["ok"], failed)
        self.assertEqual(failed["result"]["error"]["code"], "budget_exceeded")
        self.assertEqual(self.peer.writer_count, 0)
        self.assertNotIn(3, self.peer.calls)
        self.assertEqual(before, {p.name: p.read_bytes() for p in custody.iterdir()})
        # A new call budget must not restore the original single-use permit.
        again = client.execute_next(planned)
        self.assertFalse(again["result"]["ok"], again)
        self.assertNotIn(3, self.peer.calls)
        self.assertEqual(before, {p.name: p.read_bytes() for p in custody.iterdir()})
        retained = client.tool("query", session_id=sid)
        self.assertTrue(retained["result"]["ok"], retained)
        self.assertEqual(retained["result"]["original"]["review_seal"], original["review_seal"])
        self.assertFalse(retained["result"]["original"]["dispatch_started"])

def main():
    global BINARY, LOGS, VECTORS, SERVER_ARGS
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--budget-unit", type=Path)
    parser.add_argument("--logs", type=Path, required=True)
    parser.add_argument("--server-arg", action="append", default=[])
    args = parser.parse_args()
    SERVER_ARGS = args.server_arg
    LOGS = args.logs
    LOGS.mkdir(mode=0o700)
    VECTORS = {k: bytes.fromhex(v) for k, v in json.loads((ROOT / "tests/native/workshop/vectors.json").read_text()).items()}
    if args.budget_unit:
        directory = LOGS / "direct-runtime"
        directory.mkdir(mode=0o700)
        (directory / "custody").mkdir(mode=0o700)
        peer = Peer(directory)
        try:
            env = environment(config(directory, peer))
            env["WORKSHOP_BUDGET_UNIT_CHILD"] = "1"
            command = [str(args.budget_unit.resolve(strict=True)), "--exact",
                "workshop_construction_server::tests::failed_budget_withdraws_prepared_connection_and_old_seal", "--nocapture"]
            result = subprocess.run(command, capture_output=True, env=env, timeout=30)
            (directory / "unit.stdout").write_bytes(result.stdout)
            (directory / "unit.stderr").write_bytes(result.stderr)
            (directory / "unit.exit").write_text(str(result.returncode) + "\n")
            assert result.returncode == 0, (result.stdout, result.stderr)
            assert b"1 passed; 0 failed; 0 ignored" in result.stdout, result.stdout
            assert peer.calls.count(2) == 1 and peer.writer_count == 0 and 3 not in peer.calls, peer.calls
            print("Direct production runtime: actual TCP preparation, invalid budget, restored valid context, old seal refused; zero commit frames.")
        finally:
            peer.close()
        return
    if not args.binary:
        parser.error("--binary or --budget-unit is required")
    BINARY = args.binary.resolve(strict=True)
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(WorkshopStdioTests)
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    print(json.dumps({"schema": "dfmcp.workshop-stdio-component/1", "tests": result.testsRun,
        "failed": len(result.failures), "errors": len(result.errors), "skipped": len(result.skipped),
        "actual_modern_stdio": True, "loopback_peers_are_doubles": True, "live_game": False}))
    raise SystemExit(0 if result.wasSuccessful() and result.testsRun == 7 and not result.skipped else 1)


if __name__ == "__main__":
    main()
