#!/usr/bin/env python3
"""Actual modern stdio consumer with joined loopback peers and real private files.

Peers use unchanged native component vectors and independent key commitments.
This exercises Rust handlers/transport/custody, never a real game or native R1.
"""
from __future__ import annotations
import argparse
import copy
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
        "io.modelcontextprotocol/clientInfo": {"name": "tree-component", "version": "1"}}
METHODS = ["Handshake", "ReadTreeHarvest", "PrepareTreeHarvest", "CommitTreeHarvest", "QueryTreeHarvest", "CancelTreeHarvest", "ReadTreeHarvestProgress"]
BINARY: Path
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


def keyed(name, key, vectors=None):
    original = (VECTORS if vectors is None else vectors)[name]
    old_len = int.from_bytes(original[8:10], "big")
    key = key.encode()
    raw = bytearray(original[:8] + len(key).to_bytes(2, "big") + key + original[10 + old_len:])
    plan_at = 10 + len(key)
    plan = bytes(raw[plan_at:plan_at + 32])
    raw[plan_at + 32:plan_at + 48] = hashlib.sha256(b"dfmcp-tree-harvest-token/1\0" + len(key).to_bytes(2, "big") + key + plan).digest()[:16]
    raw[-32:] = hashlib.sha256(b"dfmcp-tree-harvest-receipt/1\0" + raw[:-32]).digest()
    return bytes(raw)


def large_vectors(width):
    """Independent canonical fixtures, never rewrites native golden vectors.

    Width 13 is representative; width 14 exercises all 2048 supported cells.
    Both capsules' hex exceeds the pinned single-string ceiling. This is a
    protocol double, not native/game capture or causal wood evidence.
    """
    base = VECTORS["capture"]
    assert len(base) == 1090 and base[:8] == b"DFMTC021" and width in (13, 14)
    old_body = int.from_bytes(base[107:109], "big")
    at = 109 + old_body * 2
    old_cells = int.from_bytes(base[at:at + 2], "big"); at += 2
    tail = base[at + old_cells * 36:]
    assert len(tail) == 61
    header = bytearray(base[:107])
    header[36:48] = struct.pack(">III", 64, 64, 16)
    header[78:90] = struct.pack(">III", width, width, 8)
    header[90:102] = struct.pack(">III", 16, 15, 2)
    body = [0] * (width * width * 8)
    for z in range(8):
        body[(z * width + width // 2) * width + width // 2] = 1
        body[(z * width + width // 2) * width + width // 2 + 1] = 1
    cells = bytearray(); x1 = y1 = 15 - width // 2
    for z in range(2, 10):
        for y in range(y1 - 1, y1 + width + 1):
            for x in range(x1 - 1, x1 + width + 1):
                cells += struct.pack(">III", x, y, z) + base[at + 12:at + 36]
    capture = bytes(header) + struct.pack(">H", len(body)) + struct.pack(">" + "H" * len(body), *body)
    capture += struct.pack(">H", len(cells) // 36) + cells + tail
    assert len(capture) <= 128 * 1024 and len(capture.hex()) > 65536
    assert len(cells) // 36 == (width + 2) ** 2 * 8 <= 2048
    after = bytearray(capture); after[16:24] = (1).to_bytes(8, "big")
    cell_at = 111 + len(body) * 2
    for i in range(len(cells) // 36):
        pos = cell_at + i * 36
        x, y, z = struct.unpack(">III", after[pos:pos + 12])
        if [x, y, z] == [16, 15, 2]: after[pos + 33] = 1
        if z == 2 and x // 16 == 1 and y // 16 == 0: after[pos + 35] = 1
    plan = hashlib.sha256(b"dfmcp-tree-harvest-plan/1\0" + struct.pack(">III", 15, 15, 2) + hashlib.sha256(capture).digest()).digest()
    key = b"tree-001"
    token = hashlib.sha256(b"dfmcp-tree-harvest-token/1\0" + struct.pack(">H", len(key)) + key + plan).digest()[:16]
    proof = b"DFMTI021" + struct.pack(">IIIIIIiB", 15, 15, 2, 16, 15, 2, 3, 1)
    vectors = {**VECTORS, "capture": capture, "plan": plan}
    for name, phase, reason in (("prepared", 0, 0), ("indeterminate", 1, 5), ("designated", 2, 0), ("refused", 3, 1), ("cancelled", 4, 4)):
        raw = b"DFMTR021" + struct.pack(">H", len(key)) + key + plan + token + struct.pack(">I", len(capture)) + capture
        raw += bytes([phase, reason, int(name == "designated")])
        if name == "designated": raw += struct.pack(">I", len(after)) + after
        raw += bytes([int(name == "designated")])
        if name == "designated": raw += proof
        vectors[name] = raw + hashlib.sha256(b"dfmcp-tree-harvest-receipt/1\0" + raw).digest()
    progress = bytearray(vectors["progress_supply"]); progress[36:48] = struct.pack(">III", 64, 64, 16)
    vectors["progress_supply"] = bytes(progress)
    return vectors


def resolve_content(content):
    """Verify every explicit block binding before exposing complete evidence."""
    assert isinstance(content, list) and content
    for block in content:
        assert block["type"] == "text" and len(block["text"].encode()) <= 65536
    value = json.loads(content[0]["text"])
    next_index = 1
    def walk(node, path):
        nonlocal next_index
        if isinstance(node, dict):
            for key, child in list(node.items()):
                field = path + "/" + key.replace("~", "~0").replace("/", "~1")
                if key != "native_capture_hex":
                    walk(child, field); continue
                assert isinstance(child, dict) and child["schema"] == "dfmcp.tree-native-capture/1"
                assert child["field_path"] == field and child["encoding"] == "hex"
                assert 0 < child["byte_length"] <= 128 * 1024 and child["blocks"]
                raw = bytearray()
                for part in child["blocks"]:
                    index = part["content_index"]
                    assert index == next_index and index < len(content)
                    next_index += 1
                    text = content[index]["text"]
                    assert 0 < len(text) <= 48000 and len(text) % 2 == 0
                    assert all(c in "0123456789abcdef" for c in text)
                    chunk = bytes.fromhex(text)
                    assert part["offset_bytes"] == len(raw) and part["byte_length"] == len(chunk)
                    assert part["sha256"] == hashlib.sha256(chunk).hexdigest()
                    raw.extend(chunk)
                assert len(raw) == child["byte_length"] and child["sha256"] == hashlib.sha256(raw).hexdigest()
                node[key] = raw.hex()
        elif isinstance(node, list):
            for i, child in enumerate(node): walk(child, path + "/" + str(i))
    walk(value, "")
    assert next_index == len(content), "unreferenced/duplicated evidence block"
    return value


def receive(stream, n):
    out = bytearray()
    while len(out) < n:
        value = stream.recv(n - len(out))
        if not value:
            raise EOFError
        out.extend(value)
    return bytes(out)


class Peer:
    """Joined protocol double using the unmodified native component vectors."""
    def __init__(self, directory, vectors=None):
        self.directory = directory
        self.vectors = VECTORS if vectors is None else vectors
        self.listener = socket.socket()
        self.listener.bind(("127.0.0.1", 0))
        self.listener.listen(4)
        self.listener.settimeout(.1)
        self.endpoint = f"127.0.0.1:{self.listener.getsockname()[1]}"
        self.calls, self.errors = [], []
        self.stop, self.entered, self.release_commit = threading.Event(), threading.Event(), threading.Event()
        self.record = self.key = self.stale = self.active = None
        self.drop_commit = False
        self.writer_count = 0
        self.reads = 0
        self.thread = threading.Thread(target=self.run, name="owned-tree-peer")
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
                    stream.settimeout(5)
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
                            assert request == {1: METHODS[bound].encode(), 2: b"dfmcp.tree_harvest.v1_21.Request", 3: b"dfmcp.tree_harvest.v1_21.Reply", 4: b"dfmcp_tree_harvest_v1_21"}
                            self.reply(stream, number(1, bound + 2)); bound += 1; continue
                        assert bound == 7
                        op = method - 2
                        assert 0 <= op < 7 and request[1] == b"t" * 32 and len(request[2]) == 32 and request[3] == 1 and request[4] == 21
                        self.calls.append(op)
                        with (self.directory / "native-calls.jsonl").open("a") as log:
                            log.write(json.dumps({"op": METHODS[op], "fields": sorted(request)}) + "\n")
                        generation = int.from_bytes(self.vectors["capture"][8:16], "big")
                        if op == 1 and self.stale == "source": generation += 1
                        raw = number(1, 1) + number(2, 0) + blob(3, request[2]) + number(4, 1) + number(5, 21)
                        raw += number(6, generation) + blob(7, b"test-df") + blob(8, b"test-dfhack")
                        if op == 1:
                            assert set(request) == {1, 2, 3, 4, 6, 7, 8}
                            assert [request[t] for t in (6, 7, 8)] == [15, 15, 2]
                            self.reads += 1
                            capture = bytearray(self.vectors["capture"])
                            if self.stale in {"tree", "worker", "tool"}:
                                # Mutate an actual witness byte, leaving the canonical source vector untouched.
                                pos = {"tree": 77, "worker": -38, "tool": -1}[self.stale]
                                assert capture[pos] == {"tree": 3, "worker": 1, "tool": 1}[self.stale], "native witness layout changed"
                                capture[pos] ^= 1
                            raw += blob(9, capture)
                        elif op == 2:
                            assert set(request) == {1, 2, 3, 4, 6, 7, 8, 9, 10, 11}
                            assert request[10] == hashlib.sha256(self.vectors["capture"]).digest() and request[11] == self.vectors["plan"]
                            self.key = request[9].decode(); self.record = "prepared"
                            raw += blob(10, keyed("prepared", self.key, self.vectors)) + number(11, 0)
                        elif op == 3:
                            assert set(request) == {1, 2, 3, 4, 9, 11, 12}
                            assert request[9].decode() == self.key and request[11] == self.vectors["plan"]
                            prepared = keyed("prepared", self.key, self.vectors); token_at = 10 + len(self.key.encode()) + 32
                            assert request[12] == prepared[token_at:token_at + 16]
                            ack = (self.directory / "custody/tree-harvest.ack").read_bytes()
                            assert len(ack) >= 140 and len(ack) % 140 == 0 and int.from_bytes(ack[-68:-64], "big") >= 3
                            self.writer_count += 1; assert self.writer_count == 1
                            self.record = "designated"; self.entered.set()
                            if self.drop_commit:
                                while not self.release_commit.wait(.01):
                                    if self.stop.is_set(): return
                                break
                            raw += blob(10, keyed("designated", self.key, self.vectors))
                        elif op == 4:
                            assert set(request) == {1, 2, 3, 4, 9, 11}
                            if self.record: raw += blob(10, keyed(self.record, request[9].decode(), self.vectors))
                        elif op == 5:
                            assert set(request) == {1, 2, 3, 4, 9, 11, 12}
                            assert request[9].decode() == self.key and request[11] == self.vectors["plan"]
                            prepared = keyed("prepared", self.key, self.vectors); token_at = 10 + len(self.key.encode()) + 32
                            assert request[12] == prepared[token_at:token_at + 16]
                            self.record = "cancelled"; raw += blob(10, keyed("cancelled", request[9].decode(), self.vectors))
                        elif op == 6:
                            assert set(request) == {1, 2, 3, 4, 6, 7, 8, 13, 14}
                            assert [request[t] for t in (6, 7, 8, 13, 14)] == [15, 15, 2, 1000, 3]
                            raw += blob(9, self.vectors["progress_supply"])
                        raw += number(12, 0) + number(13, int(self.record is not None))
                        self.reply(stream, raw)
                self.active = None
        except (OSError, EOFError) as cause:
            if not self.stop.is_set(): self.errors.append(repr(cause))
        except BaseException as cause:
            self.errors.append(repr(cause))
        finally:
            self.listener.close()

    def close(self):
        self.stop.set()
        if self.active is not None:
            try: self.active.shutdown(socket.SHUT_RDWR)
            except OSError: pass
        self.thread.join(6)
        assert not self.thread.is_alive(), "owned tree peer failed to drain"
        assert not self.errors, self.errors


def config(directory, peer, **changes):
    dimensions = list(struct.unpack(">III", peer.vectors["capture"][36:48]))
    scope = [0, 0, 0] + [n - 1 for n in dimensions]
    value = {"schema": "dfmcp.tree-harvest-mcp-development/1", "directory": str(directory / "custody"),
             "endpoint": peer.endpoint, "initialize": True, "acknowledged": None,
             "world_folder": "region1", "site_id": 1, "df_version": "test-df", "dfhack_version": "test-dfhack", "generation": 7,
             "recovery_generation": None, "map_dimensions": dimensions, "root": [15, 15, 2],
             "read_scope": scope, "scope": scope, "protected": [],
             "authority_expires_tick": 1000000, "lease_ticks": 1200, "checkpoint": "disposable-fortress-no-checkpoint"}
    value.update(changes)
    return value


def environment(value):
    env = {k: v for k, v in os.environ.items() if not k.startswith("DFMCP_")}
    env.update(DFMCP_ALLOW_UNADMITTED_TREE_HARVEST_V1_21="1", DFMCP_TREE_HARVEST_ALLOW_DESIGNATE="1",
               DFMCP_TREE_HARVEST_TOKEN="t" * 32, DFMCP_TREE_HARVEST_MCP_CONFIG=json.dumps(value))
    return env


class Client:
    def __init__(self, directory, value, suffix="server", env=None):
        self.directory = directory
        self.stderr = (directory / f"{suffix}.stderr").open("xb")
        self.stdout = (directory / f"{suffix}.stdout").open("xb")
        self.requests = (directory / f"{suffix}.requests").open("xb")
        self.process = subprocess.Popen([str(BINARY)], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        stderr=self.stderr, env=environment(value) if env is None else env)
        self.n = 0
        self.closed = False
        self.pending = bytearray()
        self.selector = selectors.DefaultSelector()
        self.selector.register(self.process.stdout, selectors.EVENT_READ)

    def send(self, method, params, request_id=None):
        self.n += 1
        self.request_id = self.n if request_id is None else request_id
        request = {"jsonrpc": "2.0", "id": self.request_id, "method": method, "params": {**params, "_meta": META}}
        raw = json.dumps(request).encode() + b"\n"
        self.requests.write(raw)
        self.requests.flush()
        self.process.stdin.write(raw)
        self.process.stdin.flush()

    def call(self, method, params, request_id=None):
        self.send(method, params, request_id)
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
            assert len(raw) < 512 * 1024, "modern response oversized"
            self.stdout.write(raw)
            self.stdout.flush()
            response = json.loads(raw)
            if response.get("id") == self.request_id:
                self.last_response_bytes = len(raw)
                return response
        raise AssertionError("no matching modern response")

    def tool(self, name, **arguments):
        response = self.call("tools/call", {"name": "fortress_" + name, "arguments": arguments})
        assert "result" in response, response
        self.last_content = response["result"]["content"]
        return resolve_content(self.last_content)

    def execute_next(self, packet):
        step = packet["agent_turn"]["recommendations"][0]
        assert step["tool"].startswith("fortress_")
        return self.tool(step["tool"][len("fortress_"):], **step["arguments"])

    def open(self, mode="control"):
        self.call("server/discover", {})
        tools = self.call("tools/list", {})["result"]["tools"]
        assert len(tools) == 11
        assert {tool["name"] for tool in tools} == {"fortress_" + name for name in ("open_session", "observe", "query", "plan", "commit", "wait", "cancel", "explain", "doctor", "checkpoint", "restore")}
        return self.tool("open_session", mode=mode, max_wall_millis=60000)

    def close(self, crash=False):
        if self.closed: return
        self.closed = True
        first = None
        try:
            if self.process.poll() is None:
                try:
                    if crash: self.process.kill()
                    else: self.process.stdin.close()
                except BaseException as cause:
                    first = cause
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                try: self.process.kill()
                except ProcessLookupError: pass
                try: self.process.wait(timeout=5)
                except BaseException as cause:
                    if first is None: first = cause
            except BaseException as cause:
                if first is None: first = cause
                try: self.process.kill()
                except ProcessLookupError: pass
                try: self.process.wait(timeout=5)
                except BaseException as reap_error:
                    if first is None: first = reap_error
            if self.process.poll() is not None:
                self.stdout.write(bytes(self.pending) + self.process.stdout.read())
        finally:
            for stream in (self.process.stdin, self.process.stdout, self.stdout, self.stderr, self.requests, self.selector):
                try: stream.close()
                except (OSError, ValueError) as cause:
                    if first is None: first = cause
            (self.directory / f"exit-{self.process.pid}").write_text(str(self.process.returncode) + "\n")
        if first is not None: raise first


def drain_owned(clients, peer):
    first = None
    for client in clients:
        try: client.close()
        except BaseException as cause:
            if first is None: first = cause
    try: peer.close()
    except BaseException as cause:
        if first is None: first = cause
    if first is not None: raise first


class TreeStdioTests(unittest.TestCase):
    def setUp(self):
        self.directory = LOGS / self._testMethodName
        self.directory.mkdir(mode=0o700)
        (self.directory / "custody").mkdir(mode=0o700)
        self.peer = Peer(self.directory)
        self.clients = []

    def tearDown(self):
        drain_owned(self.clients, self.peer)

    def client(self, value=None, suffix="server"):
        client = Client(self.directory, config(self.directory, self.peer) if value is None else value, suffix)
        self.clients.append(client)
        return client

    def prepared(self, client):
        opened = client.open()
        self.assertTrue(opened["result"]["ok"], opened)
        self.assertTrue(opened["result"]["custody"]["acknowledged"])
        client.initial_acknowledged = copy.deepcopy(opened["result"]["custody"]["acknowledged"])
        self.assertEqual(bytes.fromhex(opened["result"]["observation"]["native_capture_hex"]), self.peer.vectors["capture"])
        self.assertGreater(len(client.last_content), 1)
        # A consumer must reject incomplete, reordered, duplicated or tampered
        # evidence before presenting the complete capsule as verified.
        mutations = []
        missing = copy.deepcopy(client.last_content); missing.pop(); mutations.append(missing)
        duplicate = copy.deepcopy(client.last_content); duplicate.append(duplicate[-1]); mutations.append(duplicate)
        tampered = copy.deepcopy(client.last_content); text = tampered[1]["text"]
        tampered[1]["text"] = ("0" if text[0] != "0" else "1") + text[1:]; mutations.append(tampered)
        if len(client.last_content) > 2:
            reordered = copy.deepcopy(client.last_content); reordered[1], reordered[2] = reordered[2], reordered[1]; mutations.append(reordered)
        for content in mutations:
            with self.assertRaises(AssertionError): resolve_content(content)
        sid = opened["agent_turn"]["session_id"]
        observation = opened["result"]["observation"]
        self.assertTrue(observation["eligible_at_anchor"], opened)
        self.assertIsNone(observation["stable_plant_id"])
        self.assertEqual(opened["agent_turn"]["recommendations"][0]["tool"], "fortress_plan")
        planned = client.execute_next(opened)
        self.assertTrue(planned["result"]["ok"], planned)
        original = planned["result"]["original"]
        self.assertEqual(bytes.fromhex(original["original_source"]["native_capture_hex"]), self.peer.vectors["capture"])
        self.assertIsNone(planned["agent_turn"]["anchor"])
        self.assertEqual(original["causal_item_provenance"], "unknown")
        self.assertTrue(original["prepared_in_this_connection"])
        self.assertEqual(planned["agent_turn"]["recommendations"][0]["arguments"], {"session_id": sid, "review_seal": original["review_seal"], "confirmed": True})
        return sid, original, planned["result"]["custody"]["acknowledged"], planned

    def test_modern_stdio_positive_designation_supply_and_offline(self):
        saved_directory, saved_peer = self.directory, self.peer
        for name, width in (("small", None), ("large", 13), ("maximum-cells", 14)):
            with self.subTest(capsule=name):
                if width is not None:
                    self.directory = saved_directory / name; self.directory.mkdir(mode=0o700)
                    (self.directory / "custody").mkdir(mode=0o700)
                    vectors = large_vectors(width)
                    self.peer = Peer(self.directory, vectors)
                    (self.directory / "capture.bin").write_bytes(vectors["capture"])
                    (self.directory / "fixture.json").write_text(json.dumps({"scope":"independent canonical protocol double", "capture_bytes":len(vectors["capture"]), "capture_sha256":hashlib.sha256(vectors["capture"]).hexdigest(), "cells":(width+2)**2*8})+"\n")
                first = len(self.clients)
                try:
                    self.positive_flow()
                finally:
                    if width is not None:
                        try: drain_owned(self.clients[first:], self.peer)
                        finally: self.directory, self.peer = saved_directory, saved_peer

    def positive_flow(self):
        client = self.client(); sid, original, _, plan = self.prepared(client)
        # Exercise the actual router/envelope, including its metadata and the
        # longest admitted encoded string ID, with complete large evidence.
        longest = "x" * 254
        self.assertEqual(len(json.dumps(longest).encode()), 256)
        response = client.call("tools/call", {"name":"fortress_query", "arguments":{"session_id":sid}}, request_id=longest)
        self.assertEqual(response["id"], longest)
        self.assertLessEqual(client.last_response_bytes, 512 * 1024)
        identity = response["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]
        self.assertEqual(identity["name"], "dfmcp-tree-harvest-v1-21-dev")
        viewed = resolve_content(response["result"]["content"])
        self.assertEqual(viewed["result"]["original"]["review_seal"], original["review_seal"])
        self.assertEqual(bytes.fromhex(viewed["result"]["original"]["original_source"]["native_capture_hex"]), self.peer.vectors["capture"])
        committed = client.execute_next(plan)
        self.assertTrue(committed["result"]["ok"], committed)
        self.assertEqual(committed["result"]["original"]["native_phase"], 2)
        self.assertIsNone(committed["result"]["original"]["historical_supply_observed_tick"])
        sample = client.tool("wait", session_id=sid)
        self.assertTrue(sample["result"]["ok"], sample)
        goal = sample["result"]["original"]
        self.assertTrue(goal["fresh_supply_observed"])
        self.assertEqual(goal["causal_item_provenance"], "unknown")
        self.assertIsNone(goal["stable_plant_id"])
        calls_before_repeat = len(self.peer.calls)
        repeat = client.tool("wait", session_id=sid)
        self.assertTrue(repeat["result"]["ok"], repeat)
        self.assertFalse(repeat["result"]["original"]["fresh_supply_observed"])
        self.assertNotIn(6, self.peer.calls[calls_before_repeat:])
        pin = repeat["result"]["custody"]["acknowledged"]
        self.assertEqual(self.peer.writer_count, 1)
        client.tool("cancel", session_id=sid, scope="session"); client.close()
        before = {p.name: p.read_bytes() for p in (self.directory / "custody").iterdir()}
        calls_before_offline = list(self.peer.calls)
        offline = self.client(config(self.directory, self.peer, initialize=False, acknowledged=pin), "offline")
        opened = offline.open("offline")
        self.assertEqual(self.peer.calls, calls_before_offline)
        self.assertTrue(opened["result"]["ok"], opened)
        self.assertEqual(opened["result"]["original"]["plan_digest"], original["plan_digest"])
        self.assertEqual(bytes.fromhex(opened["result"]["original"]["original_source"]["native_capture_hex"]), self.peer.vectors["capture"])
        self.assertFalse(opened["result"]["original"]["fresh_supply_observed"])
        self.assertEqual(before, {p.name: p.read_bytes() for p in (self.directory / "custody").iterdir()})
        self.assertEqual(self.peer.writer_count, 1)
        offline.tool("cancel", session_id=opened["agent_turn"]["session_id"], scope="session"); offline.close()
        # The pin actually handed out by the original open remains usable as
        # checked-prefix evidence of the later durable history, never authority.
        initial_pin = self.client(config(self.directory, self.peer, initialize=False, acknowledged=client.initial_acknowledged), "offline-initial-pin")
        resumed = initial_pin.open("offline")
        self.assertTrue(resumed["result"]["ok"], resumed)
        self.assertEqual(resumed["result"]["original"]["plan_digest"], original["plan_digest"])
        self.assertEqual(self.peer.calls, calls_before_offline)
        self.assertEqual(before, {p.name: p.read_bytes() for p in (self.directory / "custody").iterdir()})
        self.assertEqual(self.peer.writer_count, 1)

    def test_crash_lost_reply_reopen_and_ack_prefix_rollback(self):
        client = self.client(); sid, original, external, _ = self.prepared(client)
        journal = self.directory / "custody/tree-harvest.journal"; prefix = journal.read_bytes()
        (self.directory / "prepared-prefix.bin").write_bytes(prefix)
        self.peer.drop_commit = True
        client.send("tools/call", {"name": "fortress_commit", "arguments": {"session_id": sid, "review_seal": original["review_seal"], "confirmed": True}})
        self.assertTrue(self.peer.entered.wait(10), self.peer.errors)
        client.close(crash=True); self.peer.release_commit.set()
        self.assertGreater(len(journal.read_bytes()), len(prefix))
        value = config(self.directory, self.peer, initialize=False, acknowledged=external)
        recovery = self.client(value, "recovery"); opened = recovery.open("recover")
        self.assertTrue(opened["result"]["ok"], opened)
        uncertainty = opened["agent_turn"]["active_work"]["indeterminate_effects"]
        self.assertEqual(len(uncertainty), 1)
        self.assertEqual(uncertainty[0]["original_key"], original["key"])
        self.assertEqual(uncertainty[0]["original_review_seal"], original["review_seal"])
        self.assertFalse(uncertainty[0]["automatic_redispatch"])
        self.assertNotEqual(opened["agent_turn"]["recommendations"][0]["tool"], "fortress_commit")
        reconciled = recovery.tool("wait", session_id=opened["agent_turn"]["session_id"])
        self.assertTrue(reconciled["result"]["ok"], reconciled)
        self.assertEqual(reconciled["result"]["original"]["native_phase"], 2)
        recovery.close(); self.assertEqual(self.peer.writer_count, 1)
        (self.directory / "settled-journal.bin").write_bytes(journal.read_bytes())
        journal.write_bytes(prefix); self.peer.record = self.peer.key = None
        calls = len(self.peer.calls); rollback = self.client(value, "rollback").open("recover")
        self.assertFalse(rollback["result"]["ok"], rollback)
        self.assertEqual(rollback["result"]["error"]["code"], "corrupt_ledger")
        self.assertEqual(self.peer.calls[calls:], []); self.assertEqual(self.peer.writer_count, 1)

    def assert_error_spine(self, packet):
        self.assertFalse(packet["result"]["ok"], packet)
        turn = packet["agent_turn"]
        self.assertTrue({"schema", "profile", "turn_id", "operation", "phase", "session_id", "request_id", "anchor", "continuity", "briefing", "attention", "changes", "affordances", "active_work", "recommendations", "uncertainty", "coverage", "budget", "references"}.issubset(turn))
        self.assertEqual(turn["schema"], "dfmcp.agent_turn/1")
        self.assertIsNone(turn["anchor"])
        self.assertTrue(all(isinstance(ref, dict) and ref for ref in turn["references"]))
        self.assertEqual(turn["active_work"]["indeterminate_effects"], [])
        self.assertEqual(turn["active_work"]["pending_plans"], [])
        self.assertEqual(turn["active_work"]["obligations"], [])

    def test_closed_request_session_scope_authority_and_bounds(self):
        client = self.client(); opened = client.open(); self.assertTrue(opened["result"]["ok"])
        sid = opened["agent_turn"]["session_id"]; obs = opened["result"]["observation"]
        bad = client.tool("plan", session_id=sid, request=json.dumps({"kind":"fell_one_visible_tree", "expected_observation":obs["witness"], "deadline_tick":13000, "native_address":1}))
        self.assert_error_spine(bad)
        self.assertNotIn(2, self.peer.calls); self.assertNotIn(3, self.peer.calls)
        bad_session = client.tool("query", session_id="not-a-session")
        self.assert_error_spine(bad_session)
        bad_mode = client.tool("open_session", mode="invalid")
        self.assert_error_spine(bad_mode)
        # This is a request-bound error; it does not exercise packet output fallback.
        oversized_request = client.tool("plan", session_id=sid, request="x" * 4097)
        self.assert_error_spine(oversized_request)
        for name, changes in (("scope", {"scope":[15,15,2,15,15,2]}),
                              ("authority", {"authority_expires_tick": 1}),
                              ("protected", {"protected":[[15,15,2,15,15,2]]})):
            with self.subTest(name=name):
                directory = self.directory / name; directory.mkdir(mode=0o700); (directory / "custody").mkdir(mode=0o700)
                peer = Peer(directory); child = Client(directory, config(directory, peer, **changes))
                try:
                    result = child.open()
                    if result["result"]["ok"]:
                        obs = result["result"]["observation"]
                        result = child.tool("plan", session_id=result["agent_turn"]["session_id"], request=json.dumps({"kind":"fell_one_visible_tree", "expected_observation":obs["witness"], "deadline_tick":13000}))
                    self.assertFalse(result["result"]["ok"], result)
                    self.assertNotIn(2, peer.calls); self.assertNotIn(3, peer.calls); self.assertEqual(peer.writer_count, 0)
                finally: drain_owned([child], peer)

    def test_production_environment_refused_with_nine_raw_logs(self):
        for marker in ("", "1.0", "1.21"):
            env = environment(config(self.directory, self.peer)); env["DFMCP_ADMITTED_BRIDGE_PROTOCOL"] = marker
            result = subprocess.run([str(BINARY)], input=b"", capture_output=True, env=env, timeout=5)
            stem = self.directory / f"production-{marker or 'empty'}"
            Path(str(stem)+".stdout").write_bytes(result.stdout); Path(str(stem)+".stderr").write_bytes(result.stderr)
            Path(str(stem)+".exit").write_text(str(result.returncode)+"\n")
            self.assertNotEqual(result.returncode, 0); self.assertEqual(result.stdout, b""); self.assertIn(b"production", result.stderr)
        self.assertEqual(self.peer.calls, [])

    def test_stale_tree_worker_tool_and_source_no_writer(self):
        for kind in ("tree", "worker", "tool", "source"):
            with self.subTest(kind=kind):
                directory = self.directory / kind; directory.mkdir(mode=0o700); (directory / "custody").mkdir(mode=0o700)
                peer = Peer(directory); child = Client(directory, config(directory, peer))
                try:
                    opened = child.open(); self.assertTrue(opened["result"]["ok"], opened)
                    peer.stale = kind
                    result = child.execute_next(opened)
                    self.assertFalse(result["result"]["ok"], result)
                    self.assertFalse(result["result"]["current_goal_available"])
                    self.assertIsNone(result["agent_turn"]["anchor"])
                    self.assertNotIn(2, peer.calls); self.assertNotIn(3, peer.calls); self.assertEqual(peer.writer_count, 0)
                finally: drain_owned([child], peer)


def main():
    global BINARY, LOGS, VECTORS
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--logs", type=Path, required=True)
    args = parser.parse_args()
    LOGS = args.logs; LOGS.mkdir(mode=0o700)
    VECTORS = {k: bytes.fromhex(v) for k,v in json.loads((ROOT / "tests/native/tree_harvest/vectors.json").read_text()).items()}
    # Independent key/receipt checks against exact checked-in native vectors.
    for name in ("prepared", "designated", "cancelled", "refused", "indeterminate"):
        assert keyed(name, "tree-001") == VECTORS[name], name
    BINARY = args.binary.resolve(strict=True)
    result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(TreeStdioTests))
    print(json.dumps({"schema":"dfmcp.tree-stdio-component/1", "tests":result.testsRun, "failed":len(result.failures),
        "errors":len(result.errors), "skipped":len(result.skipped), "actual_modern_stdio":True,
        "loopback_peers_are_doubles":True, "live_game":False}))
    raise SystemExit(0 if result.wasSuccessful() and result.testsRun == 5 and not result.skipped else 1)


if __name__ == "__main__": main()
