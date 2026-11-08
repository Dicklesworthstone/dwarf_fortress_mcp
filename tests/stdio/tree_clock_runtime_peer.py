#!/usr/bin/env python3
"""Joined Tree/1.21 and Run/1.13 peers for actual Rust Runtime child tests.

No MCP server is simulated. Native vectors, production TCP clients and private
journals are exercised by the Rust test process. All owned connections are joined.
"""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import shutil
import socket
import struct
import subprocess
import sys
import tempfile

import tree_harvest as tree
import workshop_clock as clock

TREE_PLUGIN = "dfmcp_tree_harvest_v1_21"
TREE_VECTORS = {
    key: bytes.fromhex(value)
    for key, value in json.loads(
        (tree.ROOT / "tests/native/tree_harvest/vectors.json").read_text()
    ).items()
}
START = int.from_bytes(TREE_VECTORS["capture"][24:32], "big")
GENERATION = int.from_bytes(TREE_VECTORS["capture"][8:16], "big")
PHASE = "TREE_CLOCK_RUNTIME_PHASE"
CASE = "TREE_CLOCK_RUNTIME_CASE"
INPUT = "TREE_CLOCK_RUNTIME_INPUT"
OUTPUT = "TREE_CLOCK_RUNTIME_OUTPUT"
STAGE = "TREE_CLOCK_RUNTIME_STAGE"


def journal_states(path):
    """Verify the actual Run checksum chain before using its durable state tags."""
    if not path.exists():
        return []
    raw = path.read_bytes()
    assert raw[:8] == b"DFMRJ001"
    header_end = 10 + int.from_bytes(raw[8:10], "big") + 32
    head = raw[header_end:header_end + 32]
    assert len(head) == 32
    assert head == hashlib.sha256(b"dfmcp-run-journal/1\0" + raw[:header_end]).digest()
    at, sequence, states = header_end + 32, 0, []
    while at < len(raw):
        assert raw[at:at + 8] == b"DFMRF001"
        size = int.from_bytes(raw[at + 8:at + 12], "big")
        sequence += 1
        assert int.from_bytes(raw[at + 12:at + 20], "big") == sequence
        assert raw[at + 20:at + 52] == head
        body_end = at + 52 + size
        assert body_end + 40 <= len(raw)
        digest = hashlib.sha256(b"dfmcp-run-frame/1\0" + raw[at:body_end]).digest()
        assert digest == raw[body_end:body_end + 32]
        assert raw[body_end + 32:body_end + 40] == b"DFMREND1"
        states.append(raw[at + 52])
        head, at = digest, body_end + 40
    assert at == len(raw)
    return states


def custody_snapshot(directory):
    return {
        str(path.relative_to(directory)): (
            path.stat().st_mode & 0o777, path.stat().st_size,
            hashlib.sha256(path.read_bytes()).hexdigest(),
        )
        for path in sorted(directory.rglob("*")) if path.is_file()
    }


class TreeClockPeer(clock.JoinedPeer):
    """Reuse the owned Run peer lifecycle and independent receipt commitments."""

    def __init__(self, directory, case, stage):
        self.case, self.stage_path = case, stage
        self.last_stage = ""
        self.injected = False
        self.tree_key = self.tree_phase = None
        self.tree_writes = 0
        self.vectors = TREE_VECTORS
        super().__init__(directory)
        self.tick = START
        self.drop_run_commit = case == "lost_reply"

    @property
    def journal_path(self):
        return self.directory / "custody/tree-harvest.clock.journal"

    def connection(self, stream):
        try:
            with stream:
                stream.settimeout(15)
                assert tree.receive(stream, 12) == b"DFHack?\n\x01\0\0\0"
                stream.sendall(b"DFHack!\n\x01\0\0\0")
                bound = {}
                while not self.stop.is_set():
                    try:
                        method, pad, size = struct.unpack("<hhI", tree.receive(stream, 8))
                    except EOFError:
                        break
                    assert pad == 0 and size <= 2048
                    raw = tree.receive(stream, size)
                    request = tree.fields(raw)
                    if method == 0:
                        plugin, operation = request[4].decode(), request[1].decode()
                        if plugin == TREE_PLUGIN:
                            assert operation in tree.METHODS
                            namespace = b"dfmcp.tree_harvest.v1_21"
                        else:
                            assert plugin == clock.RUN_PLUGIN and operation in clock.run_wire.METHODS
                            namespace = b"dfmcp.run.v1_13"
                        assert request[2] == namespace + b".Request"
                        assert request[3] == namespace + b".Reply"
                        identifier = len(bound) + 2
                        bound[identifier] = plugin, operation
                        response = tree.number(1, identifier)
                    else:
                        plugin, operation = bound[method]
                        with self.lock:
                            self.calls.append((plugin, operation))
                            with (self.directory / "native-requests.jsonl").open("a") as log:
                                log.write(json.dumps({"plugin": plugin, "method": operation,
                                                      "request_hex": raw.hex()}) + "\n")
                            self.apply_stage()
                            response = self.tree_reply(operation, request) if plugin == TREE_PLUGIN else self.run_reply(operation, request)
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

    def apply_stage(self):
        current = self.stage_path.read_text() if self.stage_path.exists() else ""
        if current == self.last_stage:
            return
        self.last_stage = current
        if current.startswith("sample-"):
            self.tick = START + 10 * int(current.split("-")[1])
        elif current.startswith("finish-"):
            self.finish_run(complete=current != "finish-incomplete")
            if current == "finish-stale":
                self.progress_tick = self.tick - 1

    def progress(self):
        states = journal_states(self.journal_path)
        inject = (
            self.case.startswith("before_prepare_") and self.last_stage == "plan-clock" and not states
            or self.case.startswith("after_intent_") and states and states[-1] == 0
            or self.case.startswith("after_dispatch_") and states and states[-1] == 2
        )
        progress = bytearray(TREE_VECTORS["progress_supply" if self.completed else "progress_wait"])
        progress[24:32] = (self.tick if self.progress_tick is None else self.progress_tick).to_bytes(8, "big")
        if inject:
            self.injected = True
            if self.case.endswith("source"):
                progress[8:16] = (GENERATION + 1).to_bytes(8, "big")
            else:
                assert self.case.endswith("root")
                root_at = 50 + int.from_bytes(progress[48:50], "big")
                assert int.from_bytes(progress[root_at:root_at + 4], "big") == 15
                progress[root_at:root_at + 4] = (16).to_bytes(4, "big")
        return progress

    def tree_reply(self, operation, request):
        assert request[1] == b"t" * 32 and len(request[2]) == 32
        assert request[3] == 1 and request[4] == 21
        response = (tree.number(1, 1) + tree.number(2, 0) + tree.blob(3, request[2])
                    + tree.number(4, 1) + tree.number(5, 21) + tree.number(6, GENERATION)
                    + tree.blob(7, b"test-df") + tree.blob(8, b"test-dfhack"))
        if operation == "ReadTreeHarvest":
            assert set(request) == {1, 2, 3, 4, 6, 7, 8}
            assert [request[tag] for tag in (6, 7, 8)] == [15, 15, 2]
            response += tree.blob(9, TREE_VECTORS["capture"])
        elif operation == "PrepareTreeHarvest":
            assert set(request) == {1, 2, 3, 4, 6, 7, 8, 9, 10, 11}
            assert request[10] == hashlib.sha256(TREE_VECTORS["capture"]).digest()
            assert request[11] == TREE_VECTORS["plan"] and self.tree_key is None
            self.tree_key, self.tree_phase = request[9].decode(), "prepared"
            response += tree.blob(10, tree.keyed("prepared", self.tree_key, TREE_VECTORS)) + tree.number(11, 0)
        elif operation == "CommitTreeHarvest":
            assert set(request) == {1, 2, 3, 4, 9, 11, 12}
            assert request[9].decode() == self.tree_key and request[11] == TREE_VECTORS["plan"]
            prepared = tree.keyed("prepared", self.tree_key, TREE_VECTORS)
            token_at = 10 + len(self.tree_key.encode()) + 32
            assert request[12] == prepared[token_at:token_at + 16]
            ack = (self.directory / "custody/tree-harvest.ack").read_bytes()
            assert len(ack) >= 140 and len(ack) % 140 == 0
            assert int.from_bytes(ack[-68:-64], "big") >= 3
            self.tree_writes += 1
            assert self.tree_writes == 1
            self.tree_phase = "designated"
            response += tree.blob(10, tree.keyed("designated", self.tree_key, TREE_VECTORS))
        elif operation == "QueryTreeHarvest":
            assert set(request) == {1, 2, 3, 4, 9, 11}
            assert request[9].decode() == self.tree_key and request[11] == TREE_VECTORS["plan"]
            if self.tree_phase:
                response += tree.blob(10, tree.keyed(self.tree_phase, self.tree_key, TREE_VECTORS))
        elif operation == "ReadTreeHarvestProgress":
            assert set(request) == {1, 2, 3, 4, 6, 7, 8, 13, 14}
            assert [request[tag] for tag in (6, 7, 8, 13, 14)] == [15, 15, 2, 1000, 3]
            assert self.tree_phase == "designated"
            self.progress_samples += 1
            response += tree.blob(9, self.progress())
        else:
            assert operation == "Handshake" and set(request) == {1, 2, 3, 4}
        return response + tree.number(12, 0) + tree.number(13, int(self.tree_phase is not None))

    def run_reply(self, operation, request):
        assert request[1] == clock.RUN_TOKEN and len(request[2]) == 32
        assert request[3] == 1 and request[4] == 13
        response = (tree.number(1, 1) + tree.number(2, 0) + tree.blob(3, request[2])
                    + tree.number(4, 1) + tree.number(5, 13) + tree.number(6, self.run_generation)
                    + tree.blob(7, b"test-df") + tree.blob(8, b"test-dfhack"))
        if operation == "ObserveRun":
            assert set(request) == {1, 2, 3, 4}
            response += tree.blob(9, clock.run_observation(self.run_sequence, self.tick,
                                                        not self.active_run(), self.run_generation))
        elif operation == "PrepareRun":
            assert set(request) == set(range(1, 10))
            assert self.tree_phase == "designated" and self.progress_samples >= 2
            assert not self.active_run()
            assert request[8] == clock.run_observation(self.run_sequence, self.tick,
                                                     generation=self.run_generation)
            key = request[5].decode()
            assert key.startswith("tree-clock-") and key not in self.plans
            digest = clock.run_wire.plan_for(request[6], request[7], request[8])
            assert request[9] == digest
            assert journal_states(self.journal_path)[-1] == 0
            plan = {"key": key, "ticks": request[6], "wall": request[7], "before": request[8],
                    "digest": digest, "token": clock.run_wire.token_for(key, digest)}
            self.plans[key], self.records[key] = plan, clock.run_record(plan)
            response += tree.blob(10, self.records[key])
        elif operation in ("CommitRun", "QueryRun", "CancelRun"):
            expected = {1, 2, 3, 4, 5, 9} if operation == "QueryRun" else {1, 2, 3, 4, 5, 9, 10}
            assert set(request) == expected
            key = request[5].decode()
            assert key in self.plans
            plan = self.plans[key]
            assert request[9] == plan["digest"]
            if operation != "QueryRun":
                assert request[10] == plan["token"]
            if operation == "CommitRun":
                assert key not in self.run_commits and self.progress_samples >= 3
                assert journal_states(self.journal_path)[-1] == 2
                self.run_commits.append(key)
                self.run_sequence += 1
                self.records[key] = clock.run_record(plan, phase=1, observed=self.tick)
                if self.drop_run_commit:
                    self.drop_run_commit = False
                    return None
            elif operation == "CancelRun":
                assert key not in self.run_commits
                self.records[key] = clock.run_record(plan, phase=4, reason=3)
            response += tree.blob(10, self.records[key])
        else:
            assert operation == "Handshake" and set(request) == {1, 2, 3, 4}
        return response + tree.number(11, int(self.active_run())) + tree.number(12, len(self.records))


def environment(value, mode="control"):
    env = tree.environment(value)
    env.update(DFMCP_ALLOW_UNADMITTED_TREE_CLOCK="1", DFMCP_RUN_TOKEN=clock.RUN_TOKEN.decode())
    if mode == "control":
        env["DFMCP_RUN_ALLOW_CLOCK"] = "1"
    else:
        env.pop("DFMCP_TREE_HARVEST_ALLOW_DESIGNATE", None)
    if mode == "offline":
        env.pop("DFMCP_TREE_HARVEST_TOKEN", None)
        env.pop("DFMCP_RUN_TOKEN", None)
    return env


def run_child(executable, test_name, phase, case, env, input_path, output_path, stage):
    selected = {**env, PHASE: phase, CASE: case, INPUT: str(input_path),
                OUTPUT: str(output_path), STAGE: str(stage)}
    child = subprocess.run([executable, "--exact", test_name, "--nocapture", "--test-threads=1"],
                           env=selected, capture_output=True, text=True, timeout=90)
    output_path.with_suffix(".stdout").write_text(child.stdout)
    output_path.with_suffix(".stderr").write_text(child.stderr)
    assert child.returncode == 0, f"{case}/{phase}:\n{child.stdout}\n{child.stderr}"
    assert f"tree-clock-runtime:{phase}:{case}:checked" in child.stdout
    assert output_path.is_file()
    return json.loads(output_path.read_text())


def run_case(executable, test_name, case):
    directory = Path(tempfile.mkdtemp(prefix=f"dfmcp-tree-clock-{case}-"))
    peer = None
    passed = False
    try:
        (directory / "custody").mkdir(mode=0o700)
        stage = directory / "stage"
        peer = TreeClockPeer(directory, case, stage)
        value = tree.config(directory, peer)
        value["clock"] = {"initialize": True, "generation": clock.RUN_GENERATION,
                          "recovery_generation": None, "recovery": None}
        saved_path = directory / "retained.json"
        saved = run_child(executable, test_name, "control", case, environment(value),
                          directory / "unused.json", saved_path, stage)
        assert peer.tree_writes == 1
        assert peer.count("PrepareTreeHarvest") == peer.count("CommitTreeHarvest") == 1
        assert peer.count("CancelTreeHarvest") == 0
        expected_runs = 2 if case == "repeated" else int(case in ("complete", "stale_progress", "lost_reply"))
        assert peer.count("CommitRun") == expected_runs, peer.calls
        if case.startswith(("before_prepare_", "after_intent_")) or case == "capacity":
            assert peer.count("PrepareRun") == 0
        else:
            assert peer.count("PrepareRun") == (2 if case == "repeated" else 1)
        if case.startswith(("before_prepare_", "after_intent_", "after_dispatch_")):
            assert peer.injected, "negative source scenario never reached its intended boundary"
        if case.startswith("after_intent_"):
            assert 0 in journal_states(peer.journal_path)
        if case.startswith("after_dispatch_"):
            assert 2 in journal_states(peer.journal_path)
        if case == "capacity":
            assert peer.progress_samples == 14
            assert journal_states(peer.journal_path) == []
        if case == "lost_reply":
            value["initialize"] = value["clock"]["initialize"] = False
            value["acknowledged"] = saved["reference"]["acknowledged"]
            value["clock"]["recovery"] = saved["reference"]["clock"]["recovery"]
            offline = environment(value, "offline")
            before_calls = list(peer.calls)
            before_files = custody_snapshot(directory / "custody")
            run_child(executable, test_name, "offline", case, offline, saved_path,
                      directory / "offline.json", stage)
            assert peer.calls == before_calls, "Offline contacted a native source"
            assert custody_snapshot(directory / "custody") == before_files, "Offline changed custody"
            for name in ("tree-harvest.ack", "tree-harvest.clock.journal"):
                path = directory / "custody" / name
                original = path.read_bytes()
                damaged = bytearray(original)
                damaged[-1] ^= 1
                path.write_bytes(damaged)
                try:
                    run_child(executable, test_name, "open_rejected", case, offline, saved_path,
                              directory / f"bad-{name}.json", stage)
                    assert peer.calls == before_calls
                finally:
                    path.write_bytes(original)
            assert custody_snapshot(directory / "custody") == before_files
            peer.finish_run(complete=True)
            recovered = run_child(executable, test_name, "recover", case,
                                  environment(value, "recover"), saved_path,
                                  directory / "recovered.json", stage)
            assert recovered["original"]["fresh_supply_observed"]
            assert peer.count("PrepareRun") == peer.count("CommitRun") == 1
            assert peer.count("QueryRun") >= 1
            assert peer.tree_writes == 1
            assert peer.run_commits == [saved["original_run_key"]]
        passed = True
        print(f"tree-clock-case:{case}:checked")
    finally:
        try:
            if peer is not None:
                peer.close()
        finally:
            if passed:
                shutil.rmtree(directory)
            else:
                print(f"Tree clock failure evidence retained at {directory}", file=sys.stderr)


def main():
    executable, test_name, campaign = sys.argv[1:]
    assert campaign in ("progress", "custody")
    cases = ("complete", "repeated", "stale_progress", "capacity") if campaign == "progress" else (
        "before_prepare_source", "before_prepare_root", "after_intent_source", "after_intent_root",
        "after_dispatch_source", "after_dispatch_root", "wrong_review", "ack_drift", "lost_reply",
    )
    for case in cases:
        run_case(executable, test_name, case)
    print(f"tree-clock-campaign:{campaign}:checked")


if __name__ == "__main__":
    main()
