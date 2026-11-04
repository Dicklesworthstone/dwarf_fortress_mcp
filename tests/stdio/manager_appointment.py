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
        "io.modelcontextprotocol/clientInfo": {"name": "manager-component", "version": "1"}}
METHODS = ["Handshake", "ReadManager", "PrepareManager", "CommitManager", "QueryManager", "CancelManager"]
BINARY: Path
SERVER_ARGS: list[str] = []
LOGS: Path
VECTORS = {}
PROFILE = "1.22"
MINOR = 22

def custody_name(suffix):
    return "manager-appointment" + ("-v1-23" if MINOR == 23 else "") + "." + suffix



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
    raw[plan_at + 32:plan_at + 48] = hashlib.sha256(b"dfmcp-manager-token/1\0" + len(key).to_bytes(2, "big") + key + plan).digest()[:16]
    raw[-32:] = hashlib.sha256(b"dfmcp-manager-receipt/1\0" + raw[:-32]).digest()
    return bytes(raw)


def cache_bucket_offset(capture, entity_index, target=4):
    """Locate a 1.23 bucket in a native vector without guessing fixed offsets."""
    assert capture[:8] == b"DFMMC023" and entity_index in (0, 1)
    pos = 56
    folder = int.from_bytes(capture[pos:pos+2], "big"); pos += 2 + folder + 8
    for entity in range(2):
        pos += 48
        count = int.from_bytes(capture[pos:pos+2], "big"); pos += 2
        for _ in range(count):
            pos += 44
            size = int.from_bytes(capture[pos:pos+2], "big"); pos += 2 + size
            claims = int.from_bytes(capture[pos:pos+2], "big"); pos += 2 + claims * 4
        buckets = int.from_bytes(capture[pos:pos+2], "big"); pos += 2
        assert target < buckets <= 128
        for bucket in range(buckets):
            at = pos
            size = int.from_bytes(capture[pos:pos+2], "big"); pos += 2 + size * 4
            assert size <= 256 and pos <= len(capture)
            if entity == entity_index and bucket == target: return at
    raise AssertionError("bucket missing")


def assigned_without_cache(record):
    """Produce a checksum-valid protocol-double contradiction, never native evidence."""
    assert record[:8] == b"DFMMR023"
    key_len = int.from_bytes(record[8:10], "big")
    at = 10 + key_len + 48
    before_len = int.from_bytes(record[at:at+4], "big"); at += 4 + before_len
    assert record[at:at+3] == bytes([2, 0, 1]); at += 3
    length_at = at; size = int.from_bytes(record[at:at+4], "big"); at += 4
    after = bytearray(record[at:at+size]); bucket = cache_bucket_offset(after, 0)
    assert after[bucket:bucket+6] == b"\x00\x01" + (600).to_bytes(4, "big")
    after[bucket:bucket+6] = b"\x00\x00"
    body = bytearray(record[:-32]); body[at:at+size] = after
    body[length_at:length_at+4] = len(after).to_bytes(4, "big")
    return bytes(body) + hashlib.sha256(b"dfmcp-manager-receipt/1\0" + body).digest()


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
                assert isinstance(child, dict) and child["schema"] == "dfmcp.manager-native-capture/1"
                assert child["field_path"] == field and child["encoding"] == "hex"
                assert 0 < child["byte_length"] <= 64 * 1024 and child["blocks"]
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
        self.request_timeout = 5
        self.endpoint = f"127.0.0.1:{self.listener.getsockname()[1]}"
        self.calls, self.errors = [], []
        self.stop, self.entered, self.release_commit = threading.Event(), threading.Event(), threading.Event()
        self.record = self.key = self.stale = self.active = None
        self.bad_after = False
        self.drop_commit = False
        self.writer_count = 0
        self.reads = 0
        self.thread = threading.Thread(target=self.run, name="owned-manager-peer")
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
                            assert request == {1: METHODS[bound].encode(), 2: f"dfmcp.manager.v1_{MINOR}.Request".encode(), 3: f"dfmcp.manager.v1_{MINOR}.Reply".encode(), 4: f"dfmcp_manager_v1_{MINOR}".encode()}
                            self.reply(stream, number(1, bound + 2)); bound += 1; continue
                        assert bound == 6
                        op = method - 2
                        assert 0 <= op < 6 and request[1] == b"t" * 32 and len(request[2]) == 32 and request[3] == 1 and request[4] == MINOR
                        assert 1 <= request[5] <= 60000
                        self.calls.append(op)
                        with (self.directory / "native-calls.jsonl").open("a") as log:
                            log.write(json.dumps({"op": METHODS[op], "fields": sorted(request)}) + "\n")
                        generation = int.from_bytes(self.vectors["capture"][8:16], "big")
                        if op == 1 and self.stale == "source": generation += 1
                        raw = number(1, 1) + number(2, 0) + blob(3, request[2]) + number(4, 1) + number(5, MINOR)
                        raw += number(6, generation) + blob(7, b"test-df") + blob(8, b"test-dfhack")
                        if op == 1:
                            assert set(request) == {1, 2, 3, 4, 5, 6, 7}
                            assert [request[t] for t in (6, 7)] == [256, 361]
                            self.reads += 1
                            capture = bytearray(self.vectors["capture"])
                            if self.stale == "cache":
                                at = cache_bucket_offset(capture, 0); assert capture[at:at+2] == b"\x00\x00"
                                capture[at:at+2] = b"\x00\x01" + (600).to_bytes(4, "big")
                            if self.stale == "identity": capture[-6] = 0
                            if self.stale == "epoch": capture[8:16] = struct.pack(">Q", generation + 1)
                            if self.stale == "vacancy":
                                at = 56 + 2 + int.from_bytes(capture[56:58], "big") + 8
                                count = int.from_bytes(capture[at+48:at+50], "big"); at += 50
                                for _ in range(count):
                                    position = int.from_bytes(capture[at+4:at+8], "big")
                                    if position == 1000: capture[at+8:at+12] = struct.pack(">i", 100)
                                    at += 44
                                    size = int.from_bytes(capture[at:at+2], "big"); at += 2 + size
                                    count_claims = int.from_bytes(capture[at:at+2], "big"); at += 2 + 4 * count_claims
                            raw += blob(9, capture)
                        elif op == 2:
                            assert set(request) == {1, 2, 3, 4, 5, 6, 7, 9, 10, 11}
                            assert request[10] == hashlib.sha256(self.vectors["capture"]).digest() and request[11] == self.vectors["plan"]
                            self.key = request[9].decode(); self.record = "prepared"
                            raw += blob(10, keyed("prepared", self.key, self.vectors)) + number(11, 0)
                        elif op == 3:
                            assert set(request) == {1, 2, 3, 4, 5, 9, 11, 12}
                            assert request[9].decode() == self.key and request[11] == self.vectors["plan"]
                            prepared = keyed("prepared", self.key, self.vectors); token_at = 10 + len(self.key.encode()) + 32
                            assert request[12] == prepared[token_at:token_at + 16]
                            ack = (self.directory / "custody" / custody_name("ack")).read_bytes()
                            assert len(ack) >= 140 and len(ack) % 140 == 0 and int.from_bytes(ack[-68:-64], "big") >= 3
                            self.writer_count += 1; assert self.writer_count == 1
                            self.record = "assigned"; self.entered.set()
                            if self.drop_commit:
                                while not self.release_commit.wait(.01):
                                    if self.stop.is_set(): return
                                break
                            assigned = keyed("assigned", self.key, self.vectors)
                            raw += blob(10, assigned_without_cache(assigned) if self.bad_after else assigned)
                        elif op == 4:
                            assert set(request) == {1, 2, 3, 4, 5, 9, 11}
                            if self.record: raw += blob(10, keyed(self.record, request[9].decode(), self.vectors))
                        elif op == 5:
                            assert set(request) == {1, 2, 3, 4, 5, 9, 11, 12}
                            assert request[9].decode() == self.key and request[11] == self.vectors["plan"]
                            prepared = keyed("prepared", self.key, self.vectors); token_at = 10 + len(self.key.encode()) + 32
                            assert request[12] == prepared[token_at:token_at + 16]
                            self.record = "cancelled"; raw += blob(10, keyed("cancelled", request[9].decode(), self.vectors))
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
        assert not self.thread.is_alive(), "owned manager peer failed to drain"
        assert not self.errors, self.errors


def config(directory, peer, **changes):
    dimensions=list(struct.unpack(">III",peer.vectors["capture"][44:56]))
    scope=[0,0,0]+[n-1 for n in dimensions]
    value={"schema":"dfmcp.manager-appointment-mcp-development/" + ("2" if MINOR == 23 else "1"),"directory":str(directory/"custody"),"endpoint":peer.endpoint,
        "initialize":True,"acknowledged":None,"world_folder":"region1","site_id":38,"df_version":"test-df","dfhack_version":"test-dfhack","generation":7,
        "map_dimensions":dimensions,"unit_id":256,"histfig_id":361,"read_scope":scope,"scope":scope,
        "authority_expires_tick":30*403200+16800+12000,"lease_ticks":1200,"checkpoint":"disposable-fortress-no-checkpoint"}
    value.update(changes);return value


def environment(value):
    env = {k: v for k, v in os.environ.items() if not k.startswith("DFMCP_")}
    if MINOR == 23:
        env.update(DFMCP_ALLOW_UNADMITTED_MANAGER_V1_23="1", DFMCP_MANAGER_V1_23_ALLOW_APPOINT="1",
                   DFMCP_MANAGER_V1_23_TOKEN="t" * 32, DFMCP_MANAGER_V1_23_MCP_CONFIG=json.dumps(value))
    else:
        env.update(DFMCP_ALLOW_UNADMITTED_MANAGER_V1_22="1", DFMCP_MANAGER_ALLOW_APPOINT="1",
                   DFMCP_MANAGER_TOKEN="t" * 32, DFMCP_MANAGER_MCP_CONFIG=json.dumps(value))
    return env


class Client:
    def __init__(self, directory, value, suffix="server", env=None, server_args=None):
        self.directory = directory
        self.stderr = (directory / f"{suffix}.stderr").open("xb")
        self.stdout = (directory / f"{suffix}.stdout").open("xb")
        self.requests = (directory / f"{suffix}.requests").open("xb")
        self.process = subprocess.Popen([str(BINARY), *(SERVER_ARGS if server_args is None else server_args)], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
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

    def open(self, mode="control", max_wall_millis=None):
        self.call("server/discover", {})
        tools = self.call("tools/list", {})["result"]["tools"]
        assert len(tools) == 11
        assert {tool["name"] for tool in tools} == {"fortress_" + name for name in ("open_session", "observe", "query", "plan", "commit", "wait", "cancel", "explain", "doctor", "checkpoint", "restore")}
        arguments = {"mode": mode}
        if max_wall_millis is not None:
            arguments["max_wall_millis"] = max_wall_millis
        return self.tool("open_session", **arguments)

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


class ManagerStdioTests(unittest.TestCase):
    def setUp(self):
        self.directory=LOGS/self._testMethodName;self.directory.mkdir(mode=0o700);(self.directory/"custody").mkdir(mode=0o700)
        self.peer=Peer(self.directory);self.clients=[]
    def tearDown(self): drain_owned(self.clients,self.peer)
    def client(self,value=None,suffix="server"):
        c=Client(self.directory,config(self.directory,self.peer) if value is None else value,suffix);self.clients.append(c);return c
    def prepared(self,c, **open_arguments):
        opened=c.open(**open_arguments);self.assertTrue(opened["result"]["ok"],opened);sid=opened["agent_turn"]["session_id"]
        self.assertEqual(bytes.fromhex(opened["result"]["observation"]["native_capture_hex"]),VECTORS["capture"])
        self.assertTrue(opened["result"]["observation"]["eligible_at_anchor"]);self.assertEqual(opened["agent_turn"]["recommendations"][0]["tool"],"fortress_plan")
        planned=c.execute_next(opened);self.assertTrue(planned["result"]["ok"],planned);o=planned["result"]["original"]
        self.assertTrue(o["prepared_in_this_connection"]);self.assertFalse(o["appointment_proved"]);self.assertEqual(self.peer.writer_count,0)
        return sid,o,planned
    def test_positive_full_appointment_once_and_offline_original_custody(self):
        c=self.client();sid,o,planned=self.prepared(c)
        result=c.execute_next(planned);self.assertTrue(result["result"]["ok"],result);done=result["result"]["original"]
        if MINOR == 23:
            self.assertEqual(result["result"]["source_profile"], "manager/1.23")
            self.assertEqual(done["original_source"]["group_role_buckets"][4], [])
            self.assertEqual(done["after"]["group_role_buckets"][4], [600])
            self.assertEqual(done["original_source"]["civ_role_buckets"], done["after"]["civ_role_buckets"])
            before_buckets = done["original_source"]["group_role_buckets"]
            after_buckets = done["after"]["group_role_buckets"]
            self.assertEqual(before_buckets[:4] + before_buckets[5:], after_buckets[:4] + after_buckets[5:])
        self.assertTrue(done["appointment_proved"]);self.assertEqual(done["native_phase"],2)
        if MINOR == 23: self.assertTrue(done["production_manager_cache_proved"])
        after=done["after"];self.assertEqual(after["tick"],done["original_source"]["tick"]);self.assertTrue(after["paused"])
        self.assertTrue(after["dual_getters_agree"]);self.assertEqual(after["lookup_unit"],256);self.assertEqual(after["manager_links"],1)
        self.assertEqual(after["assignment_id"],600);self.assertEqual(after["assignment_index"],3);self.assertEqual(after["manager_position_id"],1000);self.assertEqual(after["manager_position_index"],2)
        self.assertEqual(after["history_count"],3);self.assertEqual(after["next_event_id"],1993)
        repeated=c.tool("commit",session_id=sid,review_seal=o["review_seal"],confirmed=True);self.assertTrue(repeated["result"]["ok"],repeated)
        self.assertEqual(repeated["result"]["original"]["native_receipt"],done["native_receipt"]);self.assertEqual(self.peer.writer_count,1)
        pin=repeated["result"]["custody"]["acknowledged"];c.tool("cancel",session_id=sid,scope="session");c.close()
        before={p.name:p.read_bytes() for p in (self.directory/"custody").iterdir()};calls=list(self.peer.calls)
        offline=self.client(config(self.directory,self.peer,initialize=False,acknowledged=pin),"offline");opened=offline.open("offline")
        self.assertTrue(opened["result"]["ok"],opened);self.assertTrue(opened["result"]["original"]["appointment_proved"])
        self.assertEqual(opened["result"]["original"]["key"],o["key"]);self.assertFalse(opened["result"]["original"]["prepared_in_this_connection"])
        self.assertEqual(self.peer.calls,calls);self.assertEqual(before,{p.name:p.read_bytes() for p in (self.directory/"custody").iterdir()})
    def test_crash_lost_reply_reconciles_original_without_redispatch(self):
        c=self.client();sid,o,planned=self.prepared(c);external=planned["result"]["custody"]["acknowledged"]
        journal=self.directory/"custody"/custody_name("journal");prefix=journal.read_bytes();self.peer.drop_commit=True
        c.send("tools/call",{"name":"fortress_commit","arguments":{"session_id":sid,"review_seal":o["review_seal"],"confirmed":True}})
        self.assertTrue(self.peer.entered.wait(10),self.peer.errors);c.close(crash=True);self.peer.release_commit.set();self.assertGreater(len(journal.read_bytes()),len(prefix))
        recovery=self.client(config(self.directory,self.peer,initialize=False,acknowledged=external),"recover");opened=recovery.open("recover")
        self.assertTrue(opened["result"]["ok"],opened);self.assertEqual(len(opened["agent_turn"]["active_work"]["indeterminate_effects"]),1)
        self.assertFalse(opened["result"]["original"]["prepared_in_this_connection"])
        settled=recovery.tool("wait",session_id=opened["agent_turn"]["session_id"]);self.assertTrue(settled["result"]["ok"],settled)
        self.assertTrue(settled["result"]["original"]["appointment_proved"]);self.assertEqual(settled["result"]["original"]["key"],o["key"]);self.assertEqual(self.peer.writer_count,1)
        recovery.close();journal.write_bytes(prefix);calls=len(self.peer.calls)
        bad=self.client(config(self.directory,self.peer,initialize=False,acknowledged=external),"rollback").open("recover")
        self.assertFalse(bad["result"]["ok"],bad);self.assertEqual(bad["result"]["error"]["code"],"corrupt_ledger");self.assertEqual(len(self.peer.calls),calls)
    def test_changed_vacancy_HF_source_epoch_refuse_before_prepare(self):
        for kind in (("vacancy","identity","source","epoch","cache") if MINOR == 23 else ("vacancy","identity","source","epoch")):
            directory=self.directory/kind;directory.mkdir(mode=0o700);(directory/"custody").mkdir(mode=0o700);peer=Peer(directory);child=Client(directory,config(directory,peer))
            try:
                opened=child.open();self.assertTrue(opened["result"]["ok"],opened);peer.stale=kind
                refused=child.execute_next(opened);self.assertFalse(refused["result"]["ok"],refused);self.assertNotIn(2,peer.calls);self.assertEqual(peer.writer_count,0)
            finally: drain_owned([child],peer)
    def test_closed_intent_confirmation_and_budget_refuse_without_effect(self):
        c=self.client();opened=c.open();sid=opened["agent_turn"]["session_id"];obs=opened["result"]["observation"]
        for extra in ({"unit_id":250},{"role":"MAYOR"},{"native_address":1}):
            result=c.tool("plan",session_id=sid,request=json.dumps({"kind":"appoint_vacant_manager","expected_observation":obs["witness"],"deadline_tick":obs["tick"]+100,**extra}))
            self.assertFalse(result["result"]["ok"],result)
        self.assertNotIn(2,self.peer.calls);self.assertEqual(self.peer.writer_count,0)
        observed=c.tool("observe",session_id=sid);planned=c.execute_next(observed);self.assertTrue(planned["result"]["ok"],planned)
        unconfirmed=c.tool("commit",session_id=sid,review_seal=planned["result"]["original"]["review_seal"],confirmed=False)
        self.assertFalse(unconfirmed["result"]["ok"],unconfirmed);self.assertEqual(self.peer.writer_count,0)
        c.tool("cancel",session_id=sid,scope="session");c.close()
        no_budget=self.client(config(self.directory,self.peer,initialize=False,acknowledged=planned["result"]["custody"]["acknowledged"]),"budget")
        no_budget.call("server/discover",{});result=no_budget.tool("open_session",mode="recover",max_wall_millis=0)
        self.assertFalse(result["result"]["ok"],result);self.assertEqual(self.peer.writer_count,0)
    def test_production_environment_refuses_before_any_native_call(self):
        for marker in ("DFMCP_ADMITTED_BRIDGE_PROTOCOL","DFMCP_ADMISSION_TICKET","DFMCP_COMPATIBILITY_FLOOR","DFMCP_SERVER_RECEIPT_DIGEST"):
            env=environment(config(self.directory,self.peer));env[marker]="1"
            result=subprocess.run([str(BINARY),*SERVER_ARGS],input=b"",capture_output=True,env=env,timeout=5)
            (self.directory/(marker+".stdout")).write_bytes(result.stdout);(self.directory/(marker+".stderr")).write_bytes(result.stderr)
            self.assertNotEqual(result.returncode,0);self.assertEqual(result.stdout,b"");self.assertIn(b"production",result.stderr)
        self.assertEqual(self.peer.calls,[])


    def test_default_budget_allows_delayed_review_on_original_connection(self):
        # A real agent took >9s to review an S plan. Omit the wall argument:
        # explicit 60000 in older tests masked the unusable 5000ms default.
        self.peer.request_timeout = 15
        client = self.client()
        sid, original, planned = self.prepared(client)
        start = time.monotonic()
        time.sleep(5.2)
        self.assertGreater(time.monotonic() - start, 5)
        committed = client.execute_next(planned)
        self.assertTrue(committed["result"]["ok"], committed)
        self.assertEqual(committed["result"]["original"]["native_phase"], 2)
        self.assertEqual(self.peer.writer_count, 1)
        self.assertEqual(self.peer.calls.count(2), 1)
        self.assertEqual(self.peer.calls.count(3), 1)

    def test_expired_preparation_does_not_record_or_send_dispatch(self):
        self.peer.request_timeout = 15
        client = self.client()
        sid, original, planned = self.prepared(client, max_wall_millis=5000)
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
        self.assertFalse(retained["result"]["custody"]["dispatch_started"])
        self.assertFalse(retained["result"]["custody"]["uncertain_effect"])
        self.assertFalse(retained["result"]["original"]["prepared_in_this_connection"])

class ManagerV123CacheTests(ManagerStdioTests):
    # Only this new method is loaded for 1.23; inherited cases run separately.
    def test_bad_checksum_valid_after_cache_cannot_complete_or_redispatch(self):
        c=self.client(); sid,original,planned=self.prepared(c); self.peer.bad_after=True
        failed=c.execute_next(planned)
        self.assertFalse(failed["result"]["ok"],failed)
        self.assertEqual(self.peer.writer_count,1)
        retained=c.tool("query",session_id=sid)
        self.assertTrue(retained["result"]["ok"],retained)
        self.assertTrue(retained["result"]["custody"]["uncertain_effect"])
        self.assertFalse(retained["result"]["original"]["appointment_proved"])
        self.assertFalse(retained["result"]["original"]["production_manager_cache_proved"])
        self.assertEqual(retained["result"]["original"]["key"],original["key"])
        self.assertEqual(self.peer.writer_count,1)
        again=c.execute_next(planned)
        self.assertFalse(again["result"]["ok"],again)
        self.assertEqual(self.peer.writer_count,1)


def main():
    global BINARY, LOGS, VECTORS, SERVER_ARGS, PROFILE, MINOR
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--logs", type=Path, required=True)
    parser.add_argument("--server-arg", action="append", default=[])
    parser.add_argument("--profile", choices=("1.22", "1.23"), default="1.22")
    args = parser.parse_args()
    PROFILE = args.profile; MINOR = int(PROFILE.split(".")[1])
    SERVER_ARGS = args.server_arg
    LOGS = args.logs; LOGS.mkdir(mode=0o700)
    VECTORS = {k: bytes.fromhex(v) for k,v in json.loads((ROOT / f"tests/fixtures/manager_v1_{MINOR}_component.json").read_text()).items()}
    # Only the three original vectors are native component output. Other phases
    # are explicit protocol-double responses, never represented as native/live captures.
    base=VECTORS["prepared"]; key_len=int.from_bytes(base[8:10],"big"); at=10+key_len+48+4+len(VECTORS["capture"])
    for name,phase,reason in (("cancelled",4,4),("refused",3,1),("indeterminate",1,5)):
        body=bytearray(base[:-32]);body[at:at+2]=bytes([phase,reason]);VECTORS[name]=bytes(body)+hashlib.sha256(b"dfmcp-manager-receipt/1\0"+body).digest()
    for name in ("prepared","assigned"): assert keyed(name,"manager-001")==VECTORS[name],name
    BINARY = args.binary.resolve(strict=True)
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(ManagerStdioTests)
    if MINOR == 23: suite.addTest(ManagerV123CacheTests("test_bad_checksum_valid_after_cache_cannot_complete_or_redispatch"))
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    print(json.dumps({"schema":"dfmcp.manager-stdio-component/1", "tests":result.testsRun, "failed":len(result.failures),
        "errors":len(result.errors), "skipped":len(result.skipped), "actual_modern_stdio":True,
        "loopback_peers_are_doubles":True, "live_game":False}))
    raise SystemExit(0 if result.wasSuccessful() and result.testsRun == (8 if MINOR == 23 else 7) and not result.skipped else 1)


if __name__ == "__main__": main()
