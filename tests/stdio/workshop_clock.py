#!/usr/bin/env python3
"""Actual eleven-tool workshop server with joined Workshop/1.20 and Run/1.13 peers.

The peers share one endpoint and expose independently pinned native generations.
They use unchanged workshop vectors and the bounded-run wire contract. Passing
these component scenarios establishes neither real-game behavior nor admission.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import socket
import struct
import sys
import threading
import unittest
from pathlib import Path

import workshop as w

sys.path.insert(0, str(w.ROOT / "scripts"))
import bounded_run_client as run_wire

WORKSHOP_PLUGIN = "dfmcp_workshop_v1_20"
RUN_PLUGIN = "dfmcp_run_v1_13"
RUN_GENERATION = 41
RUN_TOKEN = b"s" * 32
VECTORS = {
    key: bytes.fromhex(value)
    for key, value in json.loads((w.ROOT / "tests/native/workshop/vectors.json").read_text()).items()
}
START = int.from_bytes(VECTORS["capture"][24:32], "big")
WORKSHOP_GENERATION = int.from_bytes(VECTORS["capture"][8:16], "big")
w.VECTORS = VECTORS
LOGS: Path


def run_observation(sequence, tick, paused=True, generation=RUN_GENERATION):
    return b"DFMRO013" + struct.pack(">QQQBBB", generation, sequence, tick, 1, 1, int(paused))


def run_record(plan, phase=0, reason=0, observed=None):
    attempted = phase in (1, 2, 3, 5)
    paused = phase == 3
    known = observed is not None
    raw = (b"DFMRE013" + run_wire.key_bytes(plan["key"])
           + struct.pack(">II", plan["ticks"], plan["wall"])
           + plan["before"] + plan["digest"] + plan["token"]
           + bytes([phase, reason, attempted, paused, known])
           + struct.pack(">Q", observed if known else 0))
    raw += hashlib.sha256(b"dfmcp-bounded-run-receipt/1\0" + raw).digest()
    run_wire.decode_record(raw)
    return raw


def journal_states(path):
    """Read complete frame state tags without mutating the real Rust journal."""
    raw = path.read_bytes()
    assert raw[:8] == b"DFMRJ001"
    at = 10 + int.from_bytes(raw[8:10], "big") + 16 + 16 + 32
    states = []
    while at < len(raw):
        assert raw[at:at + 8] == b"DFMRF001"
        size = int.from_bytes(raw[at + 8:at + 12], "big")
        end = at + 52 + size + 32 + 8
        assert end <= len(raw) and raw[end - 8:end] == b"DFMREND1"
        states.append(raw[at + 52])
        at = end
    assert at == len(raw)
    return states


class JoinedPeer:
    def __init__(self, directory):
        self.directory = directory
        self.lock = threading.RLock()
        self.stop = threading.Event()
        self.listener = socket.socket()
        self.listener.bind(("127.0.0.1", 0))
        self.listener.listen(8)
        self.listener.settimeout(.1)
        self.endpoint = f"127.0.0.1:{self.listener.getsockname()[1]}"
        self.connections = set()
        self.threads = []
        self.errors = []
        self.calls = []
        self.workshop_key = None
        self.workshop_phase = None
        self.workshop_writes = 0
        self.progress_samples = 0
        self.progress_generation = WORKSHOP_GENERATION
        self.progress_tick = None
        self.completed = False
        self.tick = START
        self.run_generation = RUN_GENERATION
        self.run_sequence = 3
        self.plans = {}
        self.records = {}
        self.run_commits = []
        self.drop_run_commit = False
        self.absent_run_query = False
        self.thread = threading.Thread(target=self.serve, name="joined-workshop-clock-peer")
        self.thread.start()

    def count(self, operation):
        return sum(name == operation for _, name in self.calls)

    def serve(self):
        try:
            while not self.stop.is_set():
                try:
                    stream, _ = self.listener.accept()
                except socket.timeout:
                    continue
                with self.lock:
                    self.connections.add(stream)
                    thread = threading.Thread(target=self.connection, args=(stream,),
                                              name="joined-workshop-clock-connection")
                    self.threads.append(thread)
                thread.start()
        except OSError as cause:
            if not self.stop.is_set():
                self.errors.append(repr(cause))
        finally:
            self.listener.close()

    def connection(self, stream):
        try:
            with stream:
                stream.settimeout(15)
                assert w.receive(stream, 12) == b"DFHack?\n\x01\0\0\0"
                stream.sendall(b"DFHack!\n\x01\0\0\0")
                bound = {}
                while not self.stop.is_set():
                    try:
                        method, pad, size = struct.unpack("<hhI", w.receive(stream, 8))
                    except EOFError:
                        break
                    assert pad == 0 and size <= 2048
                    wire_request = w.receive(stream, size)
                    request = w.fields(wire_request)
                    if method == 0:
                        plugin, operation = request[4].decode(), request[1].decode()
                        if plugin == WORKSHOP_PLUGIN:
                            assert operation in w.METHODS
                            namespace = b"dfmcp.workshop.v1_20"
                        else:
                            assert plugin == RUN_PLUGIN and operation in run_wire.METHODS
                            namespace = b"dfmcp.run.v1_13"
                        assert request[2] == namespace + b".Request"
                        assert request[3] == namespace + b".Reply"
                        identifier = len(bound) + 2
                        bound[identifier] = (plugin, operation)
                        response = w.number(1, identifier)
                    else:
                        plugin, operation = bound[method]
                        with self.lock:
                            self.calls.append((plugin, operation))
                            with (self.directory / "native-requests.jsonl").open("a") as log:
                                log.write(json.dumps({"plugin": plugin, "method": operation,
                                                      "request_hex": wire_request.hex()}) + "\n")
                            response = self.reply(plugin, operation, request)
                            if response is None:
                                break
                            with (self.directory / "native-replies.jsonl").open("a") as log:
                                log.write(json.dumps({"plugin": plugin, "method": operation,
                                                      "reply_hex": response.hex()}) + "\n")
                    stream.sendall(struct.pack("<hhI", -1, 0, len(response)) + response)
        except (EOFError, BrokenPipeError, ConnectionResetError):
            pass
        except OSError as cause:
            if not self.stop.is_set():
                self.errors.append(repr(cause))
        except BaseException as cause:
            self.errors.append(repr(cause))
        finally:
            with self.lock:
                self.connections.discard(stream)

    def reply(self, plugin, operation, request):
        if plugin == WORKSHOP_PLUGIN:
            return self.workshop_reply(operation, request)
        return self.run_reply(operation, request)

    def workshop_reply(self, operation, request):
        assert request[1] == b"t" * 32 and len(request[2]) == 32
        assert request[3] == 1 and request[4] == 20
        response = (w.number(1, 1) + w.number(2, 0) + w.blob(3, request[2])
                    + w.number(4, 1) + w.number(5, 20) + w.number(6, WORKSHOP_GENERATION)
                    + w.blob(7, b"test-df") + w.blob(8, b"test-dfhack"))
        if operation == "ReadWorkshop":
            assert set(request) == {1, 2, 3, 4, 5, 6, 7, 8}
            assert [request[tag] for tag in (5, 6, 7, 8)] == [42, 15, 15, 2]
            response += w.blob(9, VECTORS["capture"])
        elif operation == "PrepareWorkshop":
            assert set(request) == set(range(1, 12))
            assert request[10] == hashlib.sha256(VECTORS["capture"]).digest()
            assert request[11] == VECTORS["plan"] and self.workshop_key is None
            self.workshop_key = request[9].decode()
            self.workshop_phase = "prepared"
            response += w.blob(10, w.keyed("prepared", self.workshop_key)) + w.number(11, 0)
        elif operation == "CommitWorkshop":
            assert set(request) == {1, 2, 3, 4, 9, 11, 12}
            assert request[9].decode() == self.workshop_key and request[11] == VECTORS["plan"]
            ack = (self.directory / "custody/workshop.ack").read_bytes()
            assert len(ack) >= 140 and len(ack) % 140 == 0
            assert int.from_bytes(ack[-68:-64], "big") >= 3
            self.workshop_writes += 1
            assert self.workshop_writes == 1
            self.workshop_phase = "placed"
            response += w.blob(10, w.keyed("placed", self.workshop_key))
        elif operation == "QueryWorkshop":
            assert set(request) == {1, 2, 3, 4, 9, 11}
            if self.workshop_phase:
                response += w.blob(10, w.keyed(self.workshop_phase, request[9].decode()))
        elif operation == "CancelWorkshop":
            assert self.workshop_phase == "prepared"
            self.workshop_phase = "cancelled"
            response += w.blob(10, w.keyed("cancelled", request[9].decode()))
        elif operation == "ReadWorkshopProgress":
            assert set(request) == {1, 2, 3, 4, 5, 13}
            assert request[5] == 42 and request[13] == 70 and self.workshop_phase == "placed"
            self.progress_samples += 1
            progress = bytearray(VECTORS["progress_complete" if self.completed else "progress_stage0"])
            progress[8:16] = self.progress_generation.to_bytes(8, "big")
            tick = self.tick if self.progress_tick is None else self.progress_tick
            progress[16:24] = tick.to_bytes(8, "big")
            response += w.blob(9, progress)
        else:
            assert operation == "Handshake" and set(request) == {1, 2, 3, 4}
        return response + w.number(12, 0) + w.number(13, int(self.workshop_phase is not None))

    def active_run(self):
        return any(run_wire.decode_record(raw)["phase"] in ("running", "stopping")
                   for raw in self.records.values())

    def run_reply(self, operation, request):
        assert request[1] == RUN_TOKEN and len(request[2]) == 32
        assert request[3] == 1 and request[4] == 13
        response = (w.number(1, 1) + w.number(2, 0) + w.blob(3, request[2])
                    + w.number(4, 1) + w.number(5, 13) + w.number(6, self.run_generation)
                    + w.blob(7, b"test-df") + w.blob(8, b"test-dfhack"))
        if operation == "ObserveRun":
            assert set(request) == {1, 2, 3, 4}
            response += w.blob(9, run_observation(self.run_sequence, self.tick,
                                                 not self.active_run(), self.run_generation))
        elif operation == "PrepareRun":
            assert set(request) == set(range(1, 10))
            assert self.workshop_phase == "placed" and self.progress_samples >= 2
            assert not self.active_run()
            assert request[8] == run_observation(self.run_sequence, self.tick,
                                                  generation=self.run_generation)
            key = request[5].decode()
            assert key.startswith("workshop-clock-") and key not in self.plans
            digest = run_wire.plan_for(request[6], request[7], request[8])
            assert request[9] == digest
            assert journal_states(self.directory / "custody/workshop.clock.journal")[-1] == 0
            plan = {"key": key, "ticks": request[6], "wall": request[7], "before": request[8],
                    "digest": digest, "token": run_wire.token_for(key, digest)}
            self.plans[key] = plan
            self.records[key] = run_record(plan)
            response += w.blob(10, self.records[key])
        elif operation in ("CommitRun", "QueryRun", "CancelRun"):
            expected = {1, 2, 3, 4, 5, 9} if operation == "QueryRun" else {1, 2, 3, 4, 5, 9, 10}
            assert set(request) == expected
            key = request[5].decode()
            if key in self.plans:
                plan = self.plans[key]
                assert request[9] == plan["digest"]
                if operation != "QueryRun":
                    assert request[10] == plan["token"]
                if operation == "CommitRun":
                    assert key not in self.run_commits and self.progress_samples >= 3
                    assert journal_states(self.directory / "custody/workshop.clock.journal")[-1] == 2
                    self.run_commits.append(key)
                    self.run_sequence += 1
                    self.records[key] = run_record(plan, phase=1, observed=self.tick)
                    if self.drop_run_commit:
                        self.drop_run_commit = False
                        return None
                elif operation == "CancelRun":
                    if key in self.run_commits:
                        self.records[key] = run_record(plan, phase=3, reason=3, observed=self.tick)
                    else:
                        self.records[key] = run_record(plan, phase=4, reason=3)
                if operation != "QueryRun" or not self.absent_run_query:
                    response += w.blob(10, self.records[key])
        else:
            assert operation == "Handshake" and set(request) == {1, 2, 3, 4}
        return response + w.number(11, int(self.active_run())) + w.number(12, len(self.records))

    def finish_run(self, complete=True):
        with self.lock:
            key = self.run_commits[-1]
            plan = self.plans[key]
            self.tick = run_wire.snapshot(plan["before"])["tick"] + plan["ticks"]
            self.records[key] = run_record(plan, phase=3, reason=1, observed=self.tick)
            self.completed = complete

    def close(self):
        self.stop.set()
        with self.lock:
            connections = list(self.connections)
        for stream in connections:
            try:
                stream.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
        self.thread.join(5)
        for thread in self.threads:
            thread.join(5)
        assert not self.thread.is_alive() and all(not thread.is_alive() for thread in self.threads)
        assert not self.errors, self.errors


def configuration(directory, peer, deadline=START + 30, **changes):
    value = w.config(directory, peer)
    value["clock"] = {"initialize": True, "generation": RUN_GENERATION,
                      "deadline_tick": deadline, "recovery": None}
    value.update(changes)
    return value


def environment(value, mode="control"):
    env = w.environment(value)
    env.update(DFMCP_ALLOW_UNADMITTED_WORKSHOP_CLOCK="1", DFMCP_RUN_TOKEN=RUN_TOKEN.decode())
    if mode == "control":
        env["DFMCP_RUN_ALLOW_CLOCK"] = "1"
    else:
        env.pop("DFMCP_WORKSHOP_ALLOW_BUILD", None)
    if mode == "offline":
        env.pop("DFMCP_WORKSHOP_TOKEN", None)
        env.pop("DFMCP_RUN_TOKEN", None)
    return env


class WorkshopClockStdioTests(unittest.TestCase):
    def setUp(self):
        self.directory = LOGS / self._testMethodName
        self.directory.mkdir(mode=0o700)
        (self.directory / "custody").mkdir(mode=0o700)
        self.peer = JoinedPeer(self.directory)
        self.clients = []

    def tearDown(self):
        for client in self.clients:
            client.close()
        self.peer.close()

    def client(self, value=None, suffix="server", mode="control", env=None, directory=None):
        directory = self.directory if directory is None else directory
        value = configuration(directory, self.peer) if value is None else value
        client = w.Client(directory, value, suffix, environment(value, mode) if env is None else env)
        self.clients.append(client)
        return client

    def placed(self, client, ticks=10):
        opened = client.open(max_game_ticks=ticks)
        self.assertTrue(opened["result"]["ok"], opened)
        sid = opened["agent_turn"]["session_id"]
        planned = client.execute_next(opened)
        self.assertTrue(planned["result"]["ok"], planned)
        committed = client.execute_next(planned)
        self.assertTrue(committed["result"]["ok"], committed)
        self.assertEqual(committed["result"]["original"]["native_phase"], "placed")
        self.assertFalse(committed["result"]["current_goal_available"])
        sampled = client.tool("wait", session_id=sid, scope="goal")
        self.assertTrue(sampled["result"]["ok"], sampled)
        self.assertEqual(sampled["result"]["original"]["fresh_goal"], "in_progress")
        return sid, sampled

    def prepared_run(self, client, sampled):
        self.assertEqual(sampled["agent_turn"]["recommendations"][0]["tool"], "fortress_plan")
        recommended = json.loads(sampled["agent_turn"]["recommendations"][0]["arguments"]["request"])
        self.assertEqual(recommended["kind"], "run_workshop")
        self.assertEqual(recommended["target_id"], sampled["result"]["clock"]["target_id"])
        planned = client.execute_next(sampled)
        self.assertTrue(planned["result"]["ok"], planned)
        self.assertTrue(planned["result"]["clock"]["commit_available"])
        self.assertFalse(planned["result"]["current_goal_available"])
        self.assertEqual(planned["result"]["original"]["plan_digest"],
                         sampled["result"]["original"]["plan_digest"])
        self.assertEqual(planned["agent_turn"]["recommendations"][0]["tool"], "fortress_commit")
        self.assertEqual(planned["agent_turn"]["recommendations"][0]["arguments"]["review_seal"],
                         planned["result"]["clock"]["active_run"]["review_seal"])
        self.assertNotEqual(planned["result"]["clock"]["active_run"]["review_seal"],
                            planned["result"]["original"]["review_seal"])
        return planned

    def recovery_config(self, packet):
        value = configuration(self.directory, self.peer, initialize=False,
                              acknowledged=packet["result"]["acknowledged"])
        value["clock"]["initialize"] = False
        value["clock"]["deadline_tick"] = packet["result"]["clock"]["deadline_tick"]
        value["clock"]["recovery"] = packet["result"]["clock"]["recovery"]
        return value

    def test_reviewed_interval_running_stop_and_fresh_original_completion(self):
        client = self.client()
        sid, sampled = self.placed(client)
        original = sampled["result"]["original"]["plan_digest"]
        target = sampled["result"]["clock"]["target_id"]
        planned = self.prepared_run(client, sampled)
        self.assertEqual(self.peer.count("CommitRun"), 0)
        running = client.execute_next(planned)
        self.assertTrue(running["result"]["ok"], running)
        self.assertTrue(running["result"]["clock"]["pending"])
        self.assertFalse(running["agent_turn"]["briefing"]["fresh_original_completion_proven"])
        self.assertEqual(running["agent_turn"]["recommendations"][0]["arguments"]["scope"], "clock")
        samples = self.peer.progress_samples
        pending = client.execute_next(running)
        self.assertTrue(pending["result"]["clock"]["pending"], pending)
        self.assertEqual(self.peer.progress_samples, samples)
        self.peer.finish_run()
        completed = client.execute_next(pending)
        self.assertTrue(completed["result"]["ok"], completed)
        self.assertTrue(completed["result"]["original"]["fresh_completion_proven"])
        self.assertEqual(completed["result"]["original"]["sample_tick"], START + 10)
        self.assertEqual(completed["result"]["original"]["plan_digest"], original)
        self.assertEqual(completed["result"]["clock"]["target_id"], target)
        self.assertFalse(completed["result"]["clock"]["pending"])
        self.assertFalse(completed["agent_turn"]["briefing"]["current_pause_proven"])
        self.assertFalse(completed["agent_turn"]["briefing"]["clock_receipt_proves_workshop_completion"])
        self.assertEqual(completed["agent_turn"]["recommendations"][0]["tool"], "fortress_explain")
        self.assertEqual(self.peer.progress_samples, samples + 1)
        historical = client.tool("wait", session_id=sid, scope="goal")
        self.assertFalse(historical["result"]["current_goal_available"])
        self.assertEqual(self.peer.progress_samples, samples + 1)
        self.assertEqual(self.peer.workshop_writes, 1)
        self.assertEqual(self.peer.count("CommitRun"), 1)
        self.assertEqual(self.peer.count("PrepareRun"), 1)

    def test_second_interval_preserves_original_and_clamps_remaining_deadline(self):
        client = self.client(configuration(self.directory, self.peer, deadline=START + 16))
        sid, sampled = self.placed(client)
        first = self.prepared_run(client, sampled)
        running = client.execute_next(first)
        self.assertTrue(running["result"]["ok"], running)
        self.peer.finish_run(complete=False)
        sampled = client.tool("wait", session_id=sid, scope="clock")
        self.assertTrue(sampled["result"]["ok"], sampled)
        self.assertEqual(sampled["result"]["original"]["fresh_goal"], "in_progress")
        request = json.loads(sampled["agent_turn"]["recommendations"][0]["arguments"]["request"])
        self.assertEqual(request["game_ticks"], 6)
        self.assertEqual(self.peer.count("CommitRun"), 1)
        second = self.prepared_run(client, sampled)
        self.assertNotEqual(second["result"]["clock"]["active_run"]["key"],
                            first["result"]["clock"]["active_run"]["key"])
        self.assertEqual(second["result"]["clock"]["target_id"], first["result"]["clock"]["target_id"])
        self.assertTrue(client.execute_next(second)["result"]["ok"])
        self.peer.finish_run()
        complete = client.tool("wait", session_id=sid, scope="clock")
        self.assertTrue(complete["result"]["original"]["fresh_completion_proven"], complete)
        self.assertEqual(complete["result"]["original"]["sample_tick"], START + 16)
        self.assertEqual(complete["result"]["clock"]["deadline_tick"], START + 16)
        self.assertEqual(complete["result"]["clock"]["run_count"], 2)
        self.assertEqual(self.peer.workshop_writes, 1)
        self.assertEqual(self.peer.count("CommitRun"), 2)

    def test_lost_run_reply_reopens_original_for_query_without_replaying_unpause(self):
        client = self.client()
        _, sampled = self.placed(client)
        planned = self.prepared_run(client, sampled)
        value = self.recovery_config(planned)
        self.peer.drop_run_commit = True
        lost = client.execute_next(planned)
        self.assertFalse(lost["result"]["ok"], lost)
        self.assertFalse(lost["result"]["current_goal_available"])
        latest = lost["retained_reference"]["latest_unverified_reference"]["clock"]
        self.assertTrue(latest["pending"] and latest["unresolved"], lost)
        self.assertEqual(latest["active_run"]["key"], planned["result"]["clock"]["active_run"]["key"])
        client.close(crash=True)
        recovered = self.client(value, "recover", mode="recover")
        opened = recovered.open("recover", max_game_ticks=10)
        self.assertTrue(opened["result"]["ok"], opened)
        sid = opened["agent_turn"]["session_id"]
        self.assertFalse(opened["result"]["clock"]["commit_available"])
        uncertain = recovered.tool("wait", session_id=sid, scope="clock")
        self.assertTrue(uncertain["result"]["ok"], uncertain)
        self.assertTrue(uncertain["result"]["clock"]["pending"])
        self.peer.finish_run()
        complete = recovered.tool("wait", session_id=sid, scope="clock")
        self.assertTrue(complete["result"]["original"]["fresh_completion_proven"], complete)
        self.assertEqual(complete["result"]["clock"]["target_id"], planned["result"]["clock"]["target_id"])
        replay = recovered.tool("commit", session_id=sid,
            review_seal=planned["result"]["clock"]["active_run"]["review_seal"], confirmed=True)
        self.assertFalse(replay["result"]["ok"], replay)
        self.assertEqual(self.peer.count("CommitRun"), 1)
        self.assertEqual(self.peer.count("PrepareRun"), 1)
        self.assertEqual(self.peer.workshop_writes, 1)

    def test_offline_recovery_needs_no_credentials_socket_or_custody_writes(self):
        client = self.client()
        sid, sampled = self.placed(client)
        planned = self.prepared_run(client, sampled)
        self.assertTrue(client.execute_next(planned)["result"]["ok"])
        self.peer.finish_run()
        complete = client.tool("wait", session_id=sid, scope="clock")
        self.assertTrue(complete["result"]["ok"], complete)
        value = self.recovery_config(complete)
        self.assertTrue(client.tool("cancel", session_id=sid, scope="session")["result"]["ok"])
        client.close()
        before_calls = list(self.peer.calls)
        custody = self.directory / "custody"
        before = {path.name: path.read_bytes() for path in custody.iterdir()}
        offline = self.client(value, "offline", mode="offline")
        opened = offline.open("offline")
        self.assertTrue(opened["result"]["ok"], opened)
        self.assertEqual(opened["result"]["original"]["historical_goal"], "complete")
        self.assertEqual(opened["result"]["clock"]["run_count"], 1)
        self.assertFalse(opened["result"]["current_goal_available"])
        self.assertFalse(opened["result"]["clock"]["commit_available"])
        self.assertEqual(opened["agent_turn"]["recommendations"][0]["tool"], "fortress_query")
        inspected = offline.tool("query", session_id=opened["agent_turn"]["session_id"])
        self.assertTrue(inspected["result"]["ok"], inspected)
        self.assertEqual(inspected["result"]["clock"]["recovery"], complete["result"]["clock"]["recovery"])
        self.assertEqual(self.peer.calls, before_calls)
        self.assertEqual({path.name: path.read_bytes() for path in custody.iterdir()}, before)

    def test_default_zero_clock_allowance_refuses_native_prepare_and_unpause(self):
        client = self.client()
        sid, sampled = self.placed(client, ticks=None)
        self.assertEqual(sampled["agent_turn"]["budget"]["max_game_ticks"], 0)
        self.assertFalse(sampled["result"]["clock"]["new_run_available"])
        result = client.tool("plan", session_id=sid, request=json.dumps({
            "kind":"run_workshop", "target_id":sampled["result"]["clock"]["target_id"],
            "game_ticks":10, "wall_millis":1000}))
        self.assertFalse(result["result"]["ok"], result)
        self.assertEqual(self.peer.count("PrepareRun"), 0)
        self.assertEqual(self.peer.count("CommitRun"), 0)
        self.assertEqual(self.peer.workshop_writes, 1)

    def test_unknown_seal_consumes_run_review_and_clock_cancel_never_unpauses(self):
        client = self.client()
        sid, sampled = self.placed(client)
        planned = self.prepared_run(client, sampled)
        seal = planned["result"]["clock"]["active_run"]["review_seal"]
        refused = client.tool("commit", session_id=sid, review_seal="8" * 64, confirmed=True)
        self.assertFalse(refused["result"]["ok"], refused)
        replay = client.tool("commit", session_id=sid, review_seal=seal, confirmed=True)
        self.assertFalse(replay["result"]["ok"], replay)
        cancelled = client.tool("cancel", session_id=sid, scope="clock")
        self.assertTrue(cancelled["result"]["ok"], cancelled)
        self.assertFalse(cancelled["result"]["clock"]["pending"])
        self.assertFalse(cancelled["result"]["clock"]["commit_available"])
        self.assertEqual(cancelled["result"]["original"]["plan_digest"], sampled["result"]["original"]["plan_digest"])
        self.assertEqual(self.peer.count("CommitRun"), 0)
        self.assertEqual(self.peer.count("CancelRun"), 0)
        self.assertEqual(self.peer.workshop_writes, 1)

    def test_closed_request_target_deadline_source_and_completed_workshop_refuse_run(self):
        for name in ("closed", "target", "deadline", "source", "complete"):
            with self.subTest(name=name):
                directory = self.directory / name
                directory.mkdir(mode=0o700)
                (directory / "custody").mkdir(mode=0o700)
                peer = JoinedPeer(directory)
                value = configuration(directory, peer)
                client = self.client(value, directory=directory)
                try:
                    sid, sampled = self.placed(client, ticks=1200)
                    request = {"kind":"run_workshop", "target_id":sampled["result"]["clock"]["target_id"],
                               "game_ticks":10, "wall_millis":1000}
                    if name == "closed":
                        request["native_address"] = 1
                    elif name == "target":
                        request["target_id"] = "8" * 64
                    elif name == "deadline":
                        request["game_ticks"] = 31
                    elif name == "source":
                        peer.run_generation += 1
                    else:
                        peer.completed = True
                    refused = client.tool("plan", session_id=sid, request=json.dumps(request))
                    self.assertFalse(refused["result"]["ok"], refused)
                    self.assertFalse(refused["result"]["current_goal_available"])
                    self.assertEqual(peer.count("PrepareRun"), 0)
                    self.assertEqual(peer.count("CommitRun"), 0)
                    self.assertEqual(peer.workshop_writes, 1)
                finally:
                    client.close()
                    peer.close()

    def test_new_clock_attaches_to_recovered_workshop_without_restoring_build_permission(self):
        legacy_value = w.config(self.directory, self.peer)
        legacy = self.client(legacy_value, "legacy", env=w.environment(legacy_value))
        sid, sampled = self.placed(legacy, ticks=None)
        self.assertFalse((self.directory / "custody/workshop.clock.journal").exists())
        original = sampled["result"]["original"]["plan_digest"]
        self.assertTrue(legacy.tool("cancel", session_id=sid, scope="session")["result"]["ok"])
        legacy.close()
        value = configuration(self.directory, self.peer, initialize=False,
                              acknowledged=sampled["result"]["acknowledged"])
        env = environment(value)
        env.pop("DFMCP_WORKSHOP_ALLOW_BUILD")
        attached = self.client(value, "attached", env=env)
        opened = attached.open("control", max_game_ticks=10)
        self.assertTrue(opened["result"]["ok"], opened)
        sid = opened["agent_turn"]["session_id"]
        self.assertFalse(opened["result"]["original"]["prepared_in_this_connection"])
        sampled = attached.tool("wait", session_id=sid, scope="goal")
        self.assertTrue(sampled["result"]["ok"], sampled)
        planned = self.prepared_run(attached, sampled)
        self.assertTrue(attached.execute_next(planned)["result"]["ok"])
        self.peer.finish_run()
        complete = attached.tool("wait", session_id=sid, scope="clock")
        self.assertTrue(complete["result"]["original"]["fresh_completion_proven"], complete)
        self.assertEqual(complete["result"]["original"]["plan_digest"], original)
        self.assertEqual(self.peer.workshop_writes, 1)
        self.assertEqual(self.peer.count("PrepareWorkshop"), 1)
        self.assertEqual(self.peer.count("CommitRun"), 1)

    def test_failed_guard_generation_or_future_tick_permanently_fences_new_clock_control(self):
        for cause in ("generation", "future_tick"):
            with self.subTest(cause=cause):
                directory = self.directory / cause
                directory.mkdir(mode=0o700)
                (directory / "custody").mkdir(mode=0o700)
                peer = JoinedPeer(directory)
                value = configuration(directory, peer)
                client = self.client(value, directory=directory)
                try:
                    sid, sampled = self.placed(client)
                    planned = self.prepared_run(client, sampled)
                    self.assertEqual(peer.count("PrepareRun"), 1)
                    if cause == "generation":
                        peer.progress_generation += 1
                    else:
                        peer.progress_tick = START + 100
                    rejected = client.execute_next(planned)
                    self.assertFalse(rejected["result"]["ok"], rejected)
                    latest = rejected["retained_reference"]["latest_unverified_reference"]["clock"]
                    self.assertTrue(latest["advancement_fenced"], rejected)
                    peer.progress_generation = WORKSHOP_GENERATION
                    peer.progress_tick = None
                    cancelled = client.tool("cancel", session_id=sid, scope="clock")
                    self.assertTrue(cancelled["result"]["ok"], cancelled)
                    self.assertTrue(cancelled["result"]["clock"]["advancement_fenced"])
                    self.assertFalse(cancelled["result"]["clock"]["new_run_available"])
                    self.assertFalse(cancelled["result"]["clock"]["commit_available"])
                    attempted = client.tool("plan", session_id=sid, request=json.dumps({
                        "kind":"run_workshop", "target_id":sampled["result"]["clock"]["target_id"],
                        "game_ticks":10, "wall_millis":1000}))
                    self.assertFalse(attempted["result"]["ok"], attempted)
                    self.assertEqual(peer.count("PrepareRun"), 1)
                    self.assertEqual(peer.count("CommitRun"), 0)
                    self.assertEqual(peer.workshop_writes, 1)
                finally:
                    client.close()
                    peer.close()

    def test_stopped_run_rejects_older_completion_without_persisting_a_false_original_goal(self):
        client = self.client()
        sid, sampled = self.placed(client)
        original = sampled["result"]["original"]["plan_digest"]
        target = sampled["result"]["clock"]["target_id"]
        planned = self.prepared_run(client, sampled)
        running = client.execute_next(planned)
        self.assertTrue(running["result"]["ok"], running)
        self.peer.finish_run(complete=True)
        self.assertEqual(self.peer.tick, START + 10)

        # The original construction shape is otherwise a valid Complete vector,
        # but it predates the independently retained stopped-run clock evidence.
        self.peer.progress_tick = START
        rejected = client.tool("wait", session_id=sid, scope="clock")
        self.assertFalse(rejected["result"]["ok"], rejected)
        self.assertFalse(rejected["result"]["current_goal_available"])
        self.assertFalse(rejected["agent_turn"]["briefing"]["fresh_original_completion_proven"])
        latest = rejected["retained_reference"]["latest_unverified_reference"]["clock"]
        self.assertTrue(latest["advancement_fenced"], rejected)
        self.assertEqual(latest["target_id"], target)

        inspected = client.tool("query", session_id=sid)
        self.assertTrue(inspected["result"]["ok"], inspected)
        self.assertEqual(inspected["result"]["original"]["historical_goal"], "in_progress")
        self.assertFalse(inspected["result"]["original"]["fresh_completion_proven"])
        self.assertEqual(inspected["result"]["original"]["sample_tick"], START)
        self.assertEqual(inspected["result"]["original"]["plan_digest"], original)

        # The fence forbids new advancement, while correctly timed evidence can
        # still establish the same original construction goal through goal wait.
        self.peer.progress_tick = None
        completed = client.tool("wait", session_id=sid, scope="goal")
        self.assertTrue(completed["result"]["ok"], completed)
        self.assertTrue(completed["result"]["original"]["fresh_completion_proven"])
        self.assertEqual(completed["result"]["original"]["historical_goal"], "complete")
        self.assertEqual(completed["result"]["original"]["sample_tick"], START + 10)
        self.assertEqual(completed["result"]["original"]["plan_digest"], original)
        self.assertEqual(completed["result"]["clock"]["target_id"], target)
        self.assertTrue(completed["result"]["clock"]["advancement_fenced"])
        self.assertFalse(completed["result"]["clock"]["new_run_available"])
        self.assertFalse(completed["result"]["clock"]["commit_available"])
        self.assertEqual(self.peer.count("PrepareRun"), 1)
        self.assertEqual(self.peer.count("CommitRun"), 1)
        self.assertEqual(self.peer.workshop_writes, 1)


def main():
    global LOGS
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--logs", type=Path, required=True)
    parser.add_argument("--server-arg", action="append", default=[])
    args = parser.parse_args()
    w.BINARY = args.binary.resolve(strict=True)
    w.SERVER_ARGS = args.server_arg
    LOGS = args.logs
    LOGS.mkdir(mode=0o700)
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(WorkshopClockStdioTests)
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    print(json.dumps({"schema":"dfmcp.workshop-clock-stdio-component/1", "tests":result.testsRun,
        "failed":len(result.failures), "errors":len(result.errors), "skipped":len(result.skipped),
        "actual_modern_stdio":True, "loopback_peers_are_doubles":True, "live_game":False}))
    raise SystemExit(0 if result.wasSuccessful() and result.testsRun == 10 and not result.skipped else 1)


if __name__ == "__main__":
    main()
