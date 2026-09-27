#!/usr/bin/env python3
"""Exercise complete original furnishing goals through the real Rust MCP process.

The joined native peer retains placements made by the compiled server and serves
independently encoded operations/1.4 observations. Its monitor phase accepts only
the four existing read bindings and checks every original receipt around a full,
released capture on one connection. An absent executable is an error.

These are process/TCP/private-filesystem development regressions, not DFHack SDK,
live-fortress, physical power-loss, or production-admission evidence.
Beads: df-dfhack-bridge-plane-c-pic.4 / df-dfhack-bridge-plane-c-pic.5.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import struct
from pathlib import Path
import tempfile
import unittest

import construction_monitor_rpc as native
import construction_receipt as receipt_model
import test_build_placement_mcp as stdio
import test_furniture_batch_mcp as batch_mcp
from test_construction_receipt import building, item, job, operations
from test_track_furniture_batch import Peer as CompletePeer
from test_construction_monitor_rpc import read_message
from test_furniture_batch import proto


OPS_SECRET = "o" * 32
MAXIMUM_COMPLETION_PACKET_BYTES = 0


class NativePeer(CompletePeer):
    """A fully joined original-placement peer with optional large captures."""

    def __init__(self, plan):
        self.extra_items = 0
        super().__init__(plan)

    def __enter__(self):
        return self

    def __exit__(self, kind, error, trace):
        try:
            self.close()
        except BaseException as cleanup:
            if error is None:
                raise
            error.add_note(f"Native peer cleanup also failed: {cleanup!r}")

    def connection(self, sock):
        if not self.monitoring:
            return super().connection(sock)
        self.monitor_connections += 1
        assert self.read(sock, 12) == b"DFHack?\n" + struct.pack("<i", 1)
        self.send(sock, b"DFHack!\n" + struct.pack("<i", 1))
        methods, queried, raw, released = {}, 0, None, False
        originals = sorted((record for record in self.records.values() if record.phase == "placed"),
                           key=lambda record: record.insertion.building)
        while not self.closed.is_set():
            method, size = struct.unpack("<h2xi", self.read(sock, 8))
            assert 0 <= size <= 2048
            fields = read_message(self.read(sock, size))
            if method == 0:
                assert set(fields) == {1, 2, 3, 4}
                plugin = fields[4].decode("ascii")
                family = "build" if plugin == native.PROFILES["build"][0] else "operations"
                name = fields[1].decode("ascii")
                assert (family, name) in native.BINDINGS, "monitor bound an effectful or unknown method"
                assert plugin == native.PROFILES[family][0]
                assert fields[2] == (native.PROFILES[family][1] + ".Request").encode()
                assert fields[3] == (native.PROFILES[family][1] + ".Reply").encode()
                identity = len(methods) + 2
                methods[identity] = family, name
                self.monitor_bindings.append((family, name))
                response, tag = {1: identity}, "bind"
            else:
                family, operation = methods[method]
                self.calls.append(operation)
                self.monitor_calls.append((family, operation))
                assert fields[1] == (b"t" * 32 if family == "build" else b"o" * 32)
                assert len(fields[2]) == 32 and fields[3] == 1
                assert fields[4] == native.PROFILES[family][2]
                response = {1: 1, 2: 0, 3: fields[2], 4: 1, 5: fields[4],
                            6: self.generation if family == "build" else 987,
                            7: self.df.encode(), 8: self.dfhack.encode()}
                tag = family + "_handshake"
                if family == "build":
                    response.update({12: 0, 13: len(self.records)})
                    if operation == "Handshake":
                        assert set(fields) == {1, 2, 3, 4}
                    else:
                        assert operation == "QueryPlacement" and set(fields) == {1, 2, 3, 4, 10, 12}
                        assert queried < 2 * len(originals)
                        expected = originals[queried % len(originals)]
                        assert fields[10] == expected.plan.key.encode()
                        assert fields[12] == expected.plan.digest
                        tag = "before_receipt" if queried < len(originals) else "after_receipt"
                        assert raw is None if tag == "before_receipt" else released
                        queried += 1
                        self.monitor_queries += 1
                        response[10] = expected.raw
                else:
                    assert tuple(fields[n] for n in (5, 6, 7, 8, 11)) == (
                        4096, 4096, 65536, native.MAX_CAPTURE, native.PAGE)
                    if operation == "Handshake":
                        # Existing Rust paged requests explicitly encode zero
                        # offset/release; Python omits those native defaults.
                        expected = set(range(1, 9)) | {11}
                        assert set(fields) in (expected, expected | {10, 12})
                        assert fields.get(10, 0) == 0 and fields.get(12, 0) == 0
                    else:
                        assert operation == "ReadObservation"
                        expected = set(range(1, 13))
                        assert set(fields) in (expected, expected - {9})
                        assert queried == len(originals)
                        # A missing empty snapshot token has the same native
                        # default as an explicitly encoded empty bytes field.
                        token = fields.get(9, b"")
                        offset, release = fields[10], fields[12]
                        if release:
                            assert release == 1 and token == b"t" * 16 and offset == 0
                            assert raw is not None and not released
                            released = True
                            self.monitor_releases += 1
                            response[10], tag = token, "release"
                        else:
                            assert not released
                            if not token:
                                assert offset == 0 and raw is None
                                raw = self.operations()
                            else:
                                assert token == b"t" * 16 and raw is not None and offset > 0
                            part = raw[offset:offset + native.PAGE]
                            self.monitor_reads += 1
                            response.update({9: part, 10: b"t" * 16, 11: offset, 12: len(raw),
                                             13: hashlib.sha256(raw).digest(),
                                             14: int(offset + len(part) == len(raw))})
                            tag = "page"
            if self.monitor_hook:
                response = self.monitor_hook(tag, fields, response, self)
            encoded = proto(response)
            self.send(sock, struct.pack("<h2xi", -1, len(encoded)) + encoded)

    def operations(self):
        raw = super().operations()
        if not self.extra_items:
            return raw
        observed = receipt_model.decode_operations(raw, lambda: None)
        # Add complete unrelated inventory to force multiple pages, preserving
        # selected identities and all native signed-ID bounds.
        buildings = tuple(building(b.id, b.kind, b.stage, b.max_stage, b.bounds,
                                   b.native_type) for b in observed.buildings.values())
        items = tuple(item(i.id, i.kind, i.native_type, i.subtype, i.material,
                           i.material_index, i.stack, i.flags, i.holder, i.container)
                      for i in observed.items.values())
        jobs = tuple(job(j.id, j.kind, j.holder, j.suspended, j.attachments, j.filters)
                     for j in observed.jobs.values())
        first = max(observed.items) + 1
        if first + self.extra_items > 2_147_483_647:
            first = 1
        extra_ids = range(first, first + self.extra_items)
        assert not set(extra_ids).intersection(observed.items), "fixture reused a selected item ID"
        extras = tuple(item(n, holder=None) for n in extra_ids)
        combined = tuple(sorted(items + extras, key=lambda raw: raw[:4]))
        horizon = max(observed.horizons[2], first + self.extra_items)
        return operations(self.sample_tick, jobs=jobs, buildings=buildings, items=combined,
                          folder=self.folder, site=self.site,
                          horizons=(*observed.horizons[:2], horizon))


def environment(directory, peer, *, mode="control", credentials=True, construct=True):
    values = batch_mcp.environment(directory, peer, mode=mode, credentials=credentials)
    values["DFMCP_BUILD_COMPLETION"] = str(directory / "completion")
    if credentials:
        values["DFMCP_OPERATIONS_PAGED_TOKEN"] = OPS_SECRET
    if not construct:
        values.pop("DFMCP_BUILD_ALLOW_PLACE", None)
    return values


class Client(batch_mcp.Client):
    def request(self, method, params=None):
        result = super().request(method, params)
        assert OPS_SECRET not in json.dumps(result), "operations credential leaked into MCP output"
        return result

    def call(self, tool, **arguments):
        global MAXIMUM_COMPLETION_PACKET_BYTES
        packet = super().call(tool, **arguments)
        MAXIMUM_COMPLETION_PACKET_BYTES = max(MAXIMUM_COMPLETION_PACKET_BYTES,
                                              self.maximum_packet_bytes)
        return packet


class FurnitureCompletionMcpTests(unittest.TestCase):
    # Reuse placement assertions without inheriting the separate batch tests.
    # Every scenario executes real MCP review/commit and checks canonical native
    # placement bytes before exercising later construction observations.
    ok = batch_mcp.FurnitureBatchMcpTests.ok
    refused = batch_mcp.FurnitureBatchMcpTests.refused
    open = batch_mcp.FurnitureBatchMcpTests.open
    batch = batch_mcp.FurnitureBatchMcpTests.batch
    inventory = batch_mcp.FurnitureBatchMcpTests.inventory
    prepare = batch_mcp.FurnitureBatchMcpTests.prepare
    effect = batch_mcp.FurnitureBatchMcpTests.effect

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="dfmcp-rust-furniture-completion-")
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name)
        self.directory.chmod(0o700)
        self.expected_plans = {}
        self.original_reviews = {}

    def originals(self):
        return {name: (self.directory / name).read_bytes() for name in ("batch", "journal")}

    def place(self, client, peer, plan):
        session, opened = self.open(client, plan)
        batch = self.batch(opened, plan)
        for index, _step in enumerate(plan.ordered):
            review = self.prepare(client, session, plan, peer, index)
            self.original_reviews[review["idempotency_key"]] = dict(review)
            packet = client.call("commit", **review)
            self.effect(packet, peer, review["idempotency_key"], "placed")
            batch = self.batch(packet, plan)
        self.assertEqual(batch["status"], "all_placed")
        self.assertFalse((self.directory / "completion").exists(),
                         "placement must not silently create or start a monitor")
        peer.monitoring = True
        return session

    def completion(self, packet, plan, *, verified=True):
        value = packet["result"]["completion"]
        self.assertIsInstance(value, dict, packet)
        self.assertEqual(value["origin_verified"], verified, packet)
        self.assertEqual(value["inventory_verified"], verified, packet)
        self.assertRegex(value["goal_digest"], r"^[0-9a-f]{64}$")
        self.assertRegex(value["monitor_id"], r"^[0-9a-f]{64}$")
        self.assertEqual(value["selection_count"], len(plan.steps), packet)
        rows = value["assessments"]
        self.assertEqual(len(rows), len(plan.steps), packet)
        self.assertEqual({row["name"] for row in rows}, {step.name for step in plan.steps})
        for row in rows:
            original = self.expected_plans[row["key"]]
            self.assertEqual(row["building_id"], original.before.next_building)
            self.assertEqual(row["job_id"], original.before.next_job)
            self.assertEqual(row["item_id"], original.before.selection.item)
            self.assertRegex(row["receipt_digest"], r"^[0-9a-f]{64}$")
        # Evidence claims may be carried in the common result or the monitor
        # view, but neither location may turn completion into effect authority.
        for source in (packet["result"], value):
            for claim in ("placement_effects_discharged", "retry_placement_permitted",
                          "current_usability_proven", "continuous_stability_proven"):
                self.assertFalse(source.get(claim, False), packet)
        if not verified:
            self.assertEqual(value["phase"], "unverified")
            self.assertFalse(value["sampled_condition_satisfied"], packet)
        return value

    def start(self, client, session, peer, plan, **policy):
        before, originals = list(peer.calls), self.originals()
        connections = peer.connections
        selected = {"mode": "completion_start", "deadline": peer.tick + 100,
                    "interval": 1, "stable_samples": 2, "stable_span": 1,
                    "max_gap": 20, "max_observations": 32, **policy}
        packet = client.call("query", session_id=session, query=json.dumps(selected))
        self.ok(packet)
        value = self.completion(packet, plan)
        self.assertEqual(peer.calls, before, "starting a monitor must not contact native code")
        self.assertEqual(peer.connections, connections, "starting a monitor opened a native connection")
        self.assertEqual(self.originals(), originals)
        self.assertTrue((self.directory / "completion").is_file())
        self.assertEqual((value["observations"], value["streak"]), (0, 0))
        self.assertFalse(value["sampled_condition_satisfied"])
        self.assertFalse(value["read_outcome_unknown"])
        for key, expected in selected.items():
            if key != "mode":
                self.assertEqual(value["timing"][key], expected)
        return value

    def sample(self, client, session, peer, plan):
        before = peer.monitor_connections, peer.monitor_queries, peer.monitor_releases
        originals = self.originals()
        packet = client.call("observe", session_id=session, selection="completion")
        self.ok(packet)
        value = self.completion(packet, plan)
        self.assertEqual(peer.monitor_connections - before[0], 1)
        self.assertEqual(peer.monitor_queries - before[1], 2 * len(plan.steps))
        self.assertEqual(peer.monitor_releases - before[2], 1)
        self.assertEqual(self.originals(), originals)
        self.assertFalse(value["read_outcome_unknown"])
        return value

    def inspect(self, client, session, peer, plan):
        before = list(peer.calls)
        connections = peer.connections
        retained = (self.directory / "completion").read_bytes()
        packet = client.call("query", session_id=session, query='{"mode":"completion"}')
        self.ok(packet)
        self.assertEqual(peer.calls, before)
        self.assertEqual(peer.connections, connections)
        self.assertEqual((self.directory / "completion").read_bytes(), retained)
        return self.completion(packet, plan)

    def assert_read_only_monitor(self, peer, commits):
        self.assertEqual(peer.commits, commits)
        self.assertTrue(all(binding in native.BINDINGS for binding in peer.monitor_bindings))
        self.assertTrue(all(call in native.BINDINGS for call in peer.monitor_calls))
        self.assertEqual(peer.monitor_bindings, list(native.BINDINGS) * peer.monitor_connections)
        self.assertNotIn(("build", "CommitPlacement"), peer.monitor_calls)
        self.assertNotIn(("build", "CancelPlacement"), peer.monitor_calls)

    def recovered_completion(self, packet, plan, phase, *, monitor_verified=True):
        value = self.completion(packet, plan, verified=False)
        self.assertEqual(value["monitor_inventory_verified"], monitor_verified, packet)
        self.assertEqual(value["historical_phase"], phase, packet)
        self.assertFalse(value["sampled_condition_satisfied"], packet)
        retained = packet["result"]["batch"]
        self.assertEqual(retained["plan"], plan.json(), packet)
        self.assertEqual({step["idempotency_key"] for step in retained["steps"]},
                         {row["key"] for row in value["assessments"]})
        self.assertFalse(retained["inventory_verified"], packet)
        self.assertFalse(packet["agent_turn"]["active_work"]["pending_absence_proven"], packet)
        self.assertFalse(packet["result"].get("effect_may_have_occurred", False), packet)
        self.assertFalse(packet["result"].get("game_mutation_dispatched", False), packet)
        if monitor_verified and phase in ("cancelled", "expired", "satisfied", "failed", "invalidated"):
            self.assertEqual(packet["agent_turn"]["active_work"]["obligations"], [],
                             "verified local terminal monitoring must not remain active work")
        return value

    def test_original_three_kind_plan_completes_and_reopens_without_native_credentials(self):
        plan = batch_mcp.exact_plan(3)
        with NativePeer(plan) as peer:
            with Client(environment(self.directory, peer)) as client:
                client.tools()
                session = self.place(client, peer, plan)
                commits, original = list(peer.commits), self.originals()
                started = self.start(client, session, peer, plan)
                peer.statuses = {plan.ordered[-1].name: "pending"}
                partial = self.sample(client, session, peer, plan)
                self.assertEqual((partial["streak"], partial["observations"]), (0, 1))
                self.assertEqual(partial["condition_met_count"], 2)
                peer.statuses = {}
                for streak in (1, 2):
                    peer.sample_tick += 1
                    value = self.sample(client, session, peer, plan)
                    self.assertEqual(value["streak"], streak)
                    self.assertEqual(value["goal_digest"], started["goal_digest"])
                    self.assertEqual(value["sampled_condition_satisfied"], streak == 2)
                self.assertEqual(value["phase"], "satisfied")
                self.assertEqual(value["condition_met_count"], 3)
                retained = (self.directory / "completion").read_bytes()
            calls = list(peer.calls)
            with Client(environment(self.directory, peer, mode="offline", credentials=False)) as client:
                session, opened = self.open(client)
                self.assertEqual(self.completion(opened, plan)["goal_digest"], started["goal_digest"])
                self.assertTrue(self.inspect(client, session, peer, plan)["sampled_condition_satisfied"])
                for operation, arguments in (("observe", {"selection": "completion"}),
                                             ("cancel", {"scope": "completion"})):
                    packet = client.call(operation, session_id=session, **arguments)
                    self.ok(packet)
                    result = packet["result"]["completion"]
                    self.assertEqual(result.get("historical_phase", result["phase"]), "satisfied")
                    self.assertEqual(peer.calls, calls)
                    self.assertEqual((self.directory / "completion").read_bytes(), retained)
            self.assertEqual(self.originals(), original)
            self.assert_read_only_monitor(peer, commits)

    def test_completion_requires_every_original_placed_step_and_rejects_policy_replacement(self):
        plan = batch_mcp.exact_plan(2)
        with NativePeer(plan) as peer, Client(environment(self.directory, peer)) as client:
            session, _ = self.open(client, plan)
            for index in range(2):
                before = list(peer.calls)
                packet = client.call("query", session_id=session,
                                     query='{"mode":"completion_start","deadline":1000}')
                self.refused(packet)
                self.assertFalse((self.directory / "completion").exists())
                self.assertEqual(peer.calls, before)
                review = self.prepare(client, session, plan, peer, index)
                self.effect(client.call("commit", **review), peer, review["idempotency_key"], "placed")
            peer.monitoring = True
            initial = self.start(client, session, peer, plan)
            retained, calls = (self.directory / "completion").read_bytes(), list(peer.calls)
            for changed in ({"deadline": 2000}, {"stable_samples": 3}, {"max_observations": 512}):
                packet = client.call("query", session_id=session, query=json.dumps(
                    {"mode": "completion_start", "deadline": 1000, **changed}))
                self.refused(packet)
                self.assertEqual((self.directory / "completion").read_bytes(), retained)
                self.assertEqual(peer.calls, calls)
            self.assertEqual(self.inspect(client, session, peer, plan)["goal_digest"], initial["goal_digest"])

    def test_shared_conditions_never_latch_and_paused_captures_do_not_advance_stability(self):
        plan = batch_mcp.exact_plan(2)
        with NativePeer(plan) as peer, Client(environment(self.directory, peer)) as client:
            session = self.place(client, peer, plan)
            self.start(client, session, peer, plan)
            for row in (0, 1, 0):
                peer.statuses = {plan.ordered[row].name: "item_unverified"}
                value = self.sample(client, session, peer, plan)
                self.assertEqual((value["condition_met_count"], value["streak"]), (1, 0))
                self.assertFalse(value["sampled_condition_satisfied"])
                peer.sample_tick += 1
            peer.statuses = {}
            self.assertEqual(self.sample(client, session, peer, plan)["streak"], 1)
            for _ in range(2):
                paused = self.sample(client, session, peer, plan)
                self.assertEqual(paused["streak"], 1)
                self.assertFalse(paused["sampled_condition_satisfied"])
            peer.statuses = {plan.ordered[-1].name: "item_unverified"}
            self.assertEqual(self.sample(client, session, peer, plan)["streak"], 0)
            peer.statuses = {}
            for streak in (1, 2):
                peer.sample_tick += 1
                value = self.sample(client, session, peer, plan)
                self.assertEqual(value["streak"], streak)
            self.assertTrue(value["sampled_condition_satisfied"])

    def test_lost_trailing_receipt_persists_unknown_read_and_restart_requires_fresh_streak(self):
        plan = batch_mcp.exact_plan(2)
        with NativePeer(plan) as peer:
            with Client(environment(self.directory, peer)) as client:
                session = self.place(client, peer, plan)
                commits = list(peer.commits)
                self.start(client, session, peer, plan)
                self.assertEqual(self.sample(client, session, peer, plan)["streak"], 1)
                before = (self.directory / "completion").read_bytes()
                last = max(peer.records.values(), key=lambda record: record.insertion.building).plan.key

                def lose(tag, fields, reply, _peer):
                    if tag == "after_receipt" and fields[10] == last.encode():
                        raise EOFError("deliberately lost final original receipt reply")
                    return reply

                peer.monitor_hook = lose
                peer.sample_tick += 1
                self.refused(client.call("observe", session_id=session, selection="completion"))
                unknown = self.inspect(client, session, peer, plan)
                self.assertTrue(unknown["read_outcome_unknown"])
                self.assertEqual(unknown["observations"], 1)
                self.assertFalse(unknown["sampled_condition_satisfied"])
                self.assertTrue((self.directory / "completion").read_bytes().startswith(before))
                self.assertGreater((self.directory / "completion").stat().st_size, len(before))
            peer.monitor_hook = None
            calls = list(peer.calls)
            with Client(environment(self.directory, peer, construct=False)) as client:
                session, opened = self.open(client)
                self.assertEqual(peer.calls, calls, "reopening must not resume a lost native read")
                self.assertTrue(self.completion(opened, plan)["read_outcome_unknown"])
                for streak in (1, 2):
                    peer.sample_tick += 1
                    value = self.sample(client, session, peer, plan)
                    self.assertEqual(value["streak"], streak)
                    self.assertEqual(value["sampled_condition_satisfied"], streak == 2)
                self.assertEqual(value["observations"], 3)
            self.assertEqual(peer.monitor_connections, 4, "failed acquisition must not reconnect")
            self.assert_read_only_monitor(peer, commits)

    def test_every_receipt_source_digest_and_release_boundary_is_required(self):
        plan = batch_mcp.exact_plan(2)
        mutations = (
            ("first_receipt_missing", "before_receipt", lambda reply: reply.pop(10)),
            ("trailing_receipt_substituted", "after_receipt", lambda reply: reply.update({10: b"wrong"})),
            ("retention_too_small", "after_receipt", lambda reply: reply.update({13: 1})),
            ("furniture_generation", "before_receipt", lambda reply: reply.update({6: 8})),
            ("software_disagreement", "operations_handshake", lambda reply: reply.update({8: b"different"})),
            ("capture_digest", "page", lambda reply: reply.update({13: b"x" * 32})),
            ("page_offset", "page", lambda reply: reply.update({11: 1})),
            ("release_token", "release", lambda reply: reply.update({10: b"x" * 16})),
        )
        for label, boundary, mutate in mutations:
            with self.subTest(boundary=label), tempfile.TemporaryDirectory(dir=self.directory) as raw:
                directory, self.directory = self.directory, Path(raw)
                self.directory.chmod(0o700)
                try:
                    with NativePeer(plan) as peer, Client(environment(self.directory, peer)) as client:
                        session = self.place(client, peer, plan)
                        commits = list(peer.commits)
                        self.start(client, session, peer, plan)

                        def hook(tag, fields, reply, _peer):
                            if tag == boundary:
                                mutate(reply)
                            return reply

                        peer.monitor_hook = hook
                        self.refused(client.call("observe", session_id=session, selection="completion"))
                        value = self.inspect(client, session, peer, plan)
                        self.assertEqual(value["observations"], 0)
                        self.assertTrue(value["read_outcome_unknown"])
                        self.assertFalse(value["sampled_condition_satisfied"])
                        self.assertEqual(peer.monitor_connections, 1)
                        if boundary in ("page", "release"):
                            self.assertEqual(peer.monitor_queries, 2, "trailing queries require valid release")
                        self.assert_read_only_monitor(peer, commits)
                finally:
                    self.directory = directory

    def test_process_killed_before_final_reply_reopens_unknown_and_cannot_reuse_old_streak(self):
        plan = batch_mcp.exact_plan(2)
        with NativePeer(plan) as peer:
            with Client(environment(self.directory, peer)) as client:
                session = self.place(client, peer, plan)
                commits, original = list(peer.commits), self.originals()
                started = self.start(client, session, peer, plan)
                self.assertEqual(self.sample(client, session, peer, plan)["streak"], 1)
                retained = (self.directory / "completion").read_bytes()
                last = max(peer.records.values(), key=lambda record: record.insertion.building).plan.key
                killed = []

                def interrupt(tag, fields, reply, _peer):
                    if tag == "after_receipt" and fields[10] == last.encode():
                        # Native read intent must already be durable, but this
                        # final reply has not reached the Rust process. SIGKILL
                        # prevents its error path from performing any cleanup.
                        killed.append(tag)
                        client.child.kill()
                        raise EOFError("actual MCP process killed before final receipt reply")
                    return reply

                peer.monitor_hook = interrupt
                peer.sample_tick += 1
                with self.assertRaisesRegex(AssertionError, "MCP process closed stdout"):
                    client.call("observe", session_id=session, selection="completion")
                self.assertEqual(client.child.wait(timeout=5), -9)
                self.assertEqual(killed, ["after_receipt"])
                interrupted = (self.directory / "completion").read_bytes()
                self.assertTrue(interrupted.startswith(retained))
                self.assertGreater(len(interrupted), len(retained))
            peer.monitor_hook = None
            calls, connections = list(peer.calls), peer.connections
            with Client(environment(self.directory, peer, mode="recover", construct=False)) as client:
                session, opened = self.open(client)
                reopened = self.completion(opened, plan)
                self.assertTrue(reopened["read_outcome_unknown"])
                self.assertEqual(reopened["observations"], 1)
                self.assertFalse(reopened["sampled_condition_satisfied"])
                self.assertEqual(reopened["goal_digest"], started["goal_digest"])
                self.assertEqual(reopened["timing"], started["timing"])
                self.assertEqual((self.directory / "completion").read_bytes(), interrupted)
                self.assertEqual(peer.calls, calls)
                self.assertEqual(peer.connections, connections)
                for streak in (1, 2):
                    peer.sample_tick += 1
                    value = self.sample(client, session, peer, plan)
                    self.assertEqual(value["streak"], streak)
                    self.assertEqual(value["sampled_condition_satisfied"], streak == 2)
                self.assertEqual(value["observations"], 3)
            self.assertEqual(self.originals(), original)
            self.assertEqual(peer.monitor_connections, 4)
            self.assert_read_only_monitor(peer, commits)

    def test_same_bytes_original_inode_substitution_blocks_completion_but_allows_local_cancel(self):
        plan = batch_mcp.exact_plan(2)
        for source in ("batch", "journal"):
            with self.subTest(source=source), tempfile.TemporaryDirectory(dir=self.directory) as raw:
                directory, self.directory = self.directory, Path(raw)
                self.directory.chmod(0o700)
                try:
                    with NativePeer(plan) as peer, Client(environment(self.directory, peer)) as client:
                        session = self.place(client, peer, plan)
                        self.start(client, session, peer, plan)
                        self.sample(client, session, peer, plan)
                        retained, original = (self.directory / "completion").read_bytes(), self.originals()
                        target = self.directory / source
                        identity = target.stat().st_ino
                        replacement = self.directory / "replacement"
                        replacement.write_bytes(target.read_bytes())
                        replacement.chmod(0o600)
                        os.replace(replacement, target)
                        self.assertNotEqual(target.stat().st_ino, identity)
                        calls = list(peer.calls)
                        for operation, arguments in (("query", {"query": '{"mode":"completion"}'}),
                                                     ("observe", {"selection": "completion"})):
                            packet = client.call(operation, session_id=session, **arguments)
                            self.refused(packet)
                            self.assertFalse(packet["result"]["completion"]["sampled_condition_satisfied"])
                            self.assertEqual(peer.calls, calls)
                            self.assertEqual((self.directory / "completion").read_bytes(), retained)
                        packet = client.call("cancel", session_id=session, scope="completion")
                        self.ok(packet)
                        value = self.completion(packet, plan, verified=False)
                        self.assertEqual(value["historical_phase"], "cancelled")
                        self.assertTrue(value["monitor_inventory_verified"])
                        self.assertTrue(value["monitor_terminal"])
                        self.assertEqual(packet["agent_turn"]["active_work"]["obligations"], [])
                        self.assertEqual(peer.calls, calls)
                        self.assertEqual(self.originals(), original)
                finally:
                    self.directory = directory

    def test_origin_replaced_during_last_receipt_query_cannot_publish_satisfied_sample(self):
        plan = batch_mcp.exact_plan(2)
        with NativePeer(plan) as peer, Client(environment(self.directory, peer)) as client:
            session = self.place(client, peer, plan)
            self.start(client, session, peer, plan)
            self.sample(client, session, peer, plan)
            original = self.originals()
            last = max(peer.records.values(), key=lambda record: record.insertion.building).plan.key

            def replace_origin(tag, fields, reply, _peer):
                if tag == "after_receipt" and fields[10] == last.encode():
                    replacement = self.directory / "replacement"
                    replacement.write_bytes(original["batch"])
                    replacement.chmod(0o600)
                    os.replace(replacement, self.directory / "batch")
                return reply

            peer.monitor_hook = replace_origin
            peer.sample_tick += 1
            packet = client.call("observe", session_id=session, selection="completion")
            self.refused(packet)
            value = packet["result"]["completion"]
            self.assertFalse(value["sampled_condition_satisfied"])
            self.assertFalse(value["origin_verified"])
            self.assertTrue(value["read_outcome_unknown"])
            self.assertEqual(value["observations"], 1)
            self.assertEqual(self.originals(), original)

    def test_fixed_observation_allowance_expires_and_does_not_renew_on_restart(self):
        plan = batch_mcp.exact_plan(2)
        with NativePeer(plan) as peer:
            with Client(environment(self.directory, peer)) as client:
                session = self.place(client, peer, plan)
                self.start(client, session, peer, plan, max_observations=2)
                self.sample(client, session, peer, plan)
                value = self.sample(client, session, peer, plan)
                self.assertEqual(value["phase"], "expired")
                self.assertEqual(value["observations"], 2)
                self.assertFalse(value["sampled_condition_satisfied"])
                retained = (self.directory / "completion").read_bytes()
            calls = list(peer.calls)
            with Client(environment(self.directory, peer, mode="offline", credentials=False)) as client:
                session, opened = self.open(client)
                self.assertEqual(self.completion(opened, plan)["phase"], "expired")
                self.assertEqual(self.inspect(client, session, peer, plan)["observations"], 2)
                self.ok(client.call("observe", session_id=session, selection="completion"))
                self.assertEqual(peer.calls, calls)
                self.assertEqual((self.directory / "completion").read_bytes(), retained)

    def test_normal_recover_mode_with_query_only_authority_acquires_completion_samples(self):
        plan = batch_mcp.exact_plan(3)
        with NativePeer(plan) as peer:
            with Client(environment(self.directory, peer)) as client:
                session = self.place(client, peer, plan)
                started = self.start(client, session, peer, plan)
            original = self.originals()
            calls, commits = list(peer.calls), list(peer.commits)
            connections = peer.connections
            with Client(environment(self.directory, peer, mode="recover", construct=False)) as client:
                session, opened = self.open(client)
                self.assertEqual(opened["result"]["capabilities"], ["query"])
                current = self.completion(opened, plan)
                self.assertEqual(current["goal_digest"], started["goal_digest"])
                self.assertEqual((current["observations"], current["streak"]), (0, 0))
                self.assertEqual(peer.calls, calls, "normal recovery opened a native bootstrap")
                self.assertEqual(peer.connections, connections)
                for streak in (1, 2):
                    value = self.sample(client, session, peer, plan)
                    self.assertEqual(value["streak"], streak)
                    self.assertEqual(value["sampled_condition_satisfied"], streak == 2)
                    self.assertEqual(value["timing"], started["timing"])
                    peer.sample_tick += 1
                self.assertEqual(value["phase"], "satisfied")
                self.assertEqual(value["observations"], 2)
                self.assertEqual(value["condition_met_count"], 3)
            self.assertEqual(self.originals(), original)
            self.assertEqual(peer.calls.count("PreparePlacement"), len(plan.steps))
            self.assertEqual(peer.calls.count("CommitPlacement"), len(plan.steps))
            self.assertEqual((peer.monitor_connections, peer.monitor_queries, peer.monitor_releases),
                             (2, 12, 2))
            self.assert_read_only_monitor(peer, commits)

    def test_restart_without_original_parent_or_child_can_inspect_and_cancel_monitor_only(self):
        plan = batch_mcp.exact_plan(2)
        for source in ("batch", "journal"):
            for substituted in (False, True):
                with self.subTest(source=source, substituted=substituted), \
                        tempfile.TemporaryDirectory(dir=self.directory) as raw:
                    directory, self.directory = self.directory, Path(raw)
                    self.directory.chmod(0o700)
                    try:
                        with NativePeer(plan) as peer:
                            with Client(environment(self.directory, peer)) as client:
                                session = self.place(client, peer, plan)
                                self.start(client, session, peer, plan)
                                candidate = self.sample(client, session, peer, plan)
                                self.assertEqual(candidate["phase"], "candidate")
                            original = self.originals()
                            target = self.directory / source
                            retained_source = self.directory / "retained-original"
                            target.rename(retained_source)
                            if substituted:
                                target.write_bytes(original[source])
                                target.chmod(0o600)
                                self.assertNotEqual(target.stat().st_ino, retained_source.stat().st_ino)
                            monitor_before = (self.directory / "completion").read_bytes()
                            calls, commits = list(peer.calls), list(peer.commits)
                            connections = peer.connections
                            env = environment(self.directory, peer, mode="completion-recover",
                                              credentials=False, construct=False)
                            with Client(env) as client:
                                client.tools()
                                session, opened = self.open(client)
                                self.assertEqual(opened["result"]["capabilities"], ["query"])
                                value = self.recovered_completion(opened, plan, "candidate")
                                self.assertEqual(value["goal_digest"], candidate["goal_digest"])
                                self.assertEqual((self.directory / "completion").read_bytes(), monitor_before)
                                for query in ('{"mode":"completion"}', '{"mode":"schema"}'):
                                    packet = client.call("query", session_id=session, query=query)
                                    self.ok(packet)
                                    self.recovered_completion(packet, plan, "candidate")
                                original_key = value["assessments"][0]["key"]
                                original_plan = self.expected_plans[original_key]
                                original_review = self.original_reviews[original_key]
                                denied_operations = (
                                    ("observe", {"selection": "completion"}),
                                    ("observe", {"selection": "next"}),
                                    ("query", {"query": '{"mode":"completion_start","deadline":2000}'}),
                                    ("cancel", {"scope": "batch"}),
                                    ("plan", {"idempotency_key": original_key,
                                              "observation_witness": original_plan.before.witness.hex()}),
                                    ("commit", {key: value for key, value in original_review.items()
                                                if key != "session_id"}),
                                    ("wait", {"idempotency_key": original_key,
                                              "plan_digest": original_plan.digest.hex()}),
                                )
                                for operation, arguments in denied_operations:
                                    packet = client.call(operation, session_id=session, **arguments)
                                    self.refused(packet)
                                    self.recovered_completion(packet, plan, "candidate", monitor_verified=False)
                                    self.assertEqual((self.directory / "completion").read_bytes(), monitor_before)
                                cancelled = client.call("cancel", session_id=session, scope="completion")
                                self.ok(cancelled)
                                value = self.recovered_completion(cancelled, plan, "cancelled")
                                self.assertEqual(value["goal_digest"], candidate["goal_digest"])
                                self.assertEqual(value["timing"], candidate["timing"])
                                monitor_after = (self.directory / "completion").read_bytes()
                                self.assertTrue(monitor_after.startswith(monitor_before))
                                self.assertGreater(len(monitor_after), len(monitor_before))
                                repeated = client.call("cancel", session_id=session, scope="completion")
                                self.ok(repeated)
                                self.recovered_completion(repeated, plan, "cancelled")
                                self.assertEqual((self.directory / "completion").read_bytes(), monitor_after)
                                closed = client.call("cancel", session_id=session, scope="session")
                                self.ok(closed)
                            with Client(environment(self.directory, peer, mode="completion-offline",
                                                    credentials=False, construct=False)) as client:
                                session, opened = self.open(client)
                                self.recovered_completion(opened, plan, "cancelled")
                                repeated = client.call("cancel", session_id=session, scope="completion")
                                self.ok(repeated)
                                self.recovered_completion(repeated, plan, "cancelled")
                            self.assertEqual((self.directory / "completion").read_bytes(), monitor_after)
                            self.assertEqual(peer.calls, calls)
                            self.assertEqual(peer.commits, commits)
                            self.assertEqual(peer.connections, connections)
                            self.assertEqual(retained_source.read_bytes(), original[source])
                            other = "journal" if source == "batch" else "batch"
                            self.assertEqual((self.directory / other).read_bytes(), original[other])
                            if substituted:
                                self.assertEqual(target.read_bytes(), original[source])
                            else:
                                self.assertFalse(target.exists(), "recovery recreated a missing placement file")
                    finally:
                        self.directory = directory

    def test_monitor_only_offline_active_history_and_configuration_cannot_be_retargeted(self):
        plan = batch_mcp.exact_plan(2)
        with NativePeer(plan) as peer:
            with Client(environment(self.directory, peer)) as client:
                session = self.place(client, peer, plan)
                self.start(client, session, peer, plan)
                self.sample(client, session, peer, plan)
            original = self.originals()
            monitor_before = (self.directory / "completion").read_bytes()
            calls = list(peer.calls)
            connections = peer.connections
            # A missing parent is deliberate: only the independently retained
            # monitor can be inspected, and offline mode cannot append a cancel.
            (self.directory / "batch").rename(self.directory / "retained-batch")
            env = environment(self.directory, peer, mode="completion-offline",
                              credentials=False, construct=False)
            with Client(env) as client:
                for arguments in ({"selection": json.dumps(batch_mcp.selected(plan.ordered[0]))},
                                  {"furniture_plan": json.dumps(plan.json())}):
                    packet = client.call("open_session", max_wall_millis=60000, **arguments)
                    self.refused(packet)
                session, opened = self.open(client)
                self.recovered_completion(opened, plan, "candidate")
                for operation, arguments in (("cancel", {"scope": "completion"}),
                                             ("observe", {"selection": "completion"})):
                    packet = client.call(operation, session_id=session, **arguments)
                    self.refused(packet)
                    self.recovered_completion(packet, plan, "candidate", monitor_verified=False)
                    self.assertEqual((self.directory / "completion").read_bytes(), monitor_before)
            changes = (
                {"DFMCP_BUILD_BATCH": str(self.directory / "different-parent")},
                {"DFMCP_BUILD_JOURNAL": str(self.directory / "different-child")},
                {"DFMCP_BUILD_WORLD_FOLDER": "different-fortress"},
                {"DFMCP_BUILD_SITE_ID": "3"},
                {"DFMCP_BUILD_ENDPOINT": f"127.0.0.1:{5001 if peer.address[1] != 5001 else 5002}"},
            )
            for change in changes:
                with self.subTest(configured=next(iter(change))), Client({**env, **change}) as client:
                    packet = client.call("open_session", max_wall_millis=60000)
                    self.refused(packet)
                    self.assertEqual((self.directory / "completion").read_bytes(), monitor_before)
            self.assertEqual(peer.calls, calls)
            self.assertEqual(peer.connections, connections)
            self.assertEqual((self.directory / "retained-batch").read_bytes(), original["batch"])
            self.assertEqual((self.directory / "journal").read_bytes(), original["journal"])
            self.assertFalse((self.directory / "batch").exists())
            self.assertFalse((self.directory / "different-parent").exists())
            self.assertFalse((self.directory / "different-child").exists())

    def test_32_original_placements_complete_multi_page_capture_and_bounded_agent_turn(self):
        plan = batch_mcp.exact_plan(32, wide=True)
        with NativePeer(plan) as peer, Client(environment(self.directory, peer)) as client:
            session = self.place(client, peer, plan)
            commits = list(peer.commits)
            self.start(client, session, peer, plan)
            peer.extra_items = 2000
            self.assertGreater(len(peer.operations()), native.PAGE)
            for streak in (1, 2):
                value = self.sample(client, session, peer, plan)
                self.assertEqual(value["condition_met_count"], 32)
                self.assertEqual(value["streak"], streak)
                self.assertEqual(len(value["assessments"]), 32)
                peer.sample_tick += 1
            self.assertTrue(value["sampled_condition_satisfied"])
            self.assertEqual(peer.monitor_queries, 128)
            self.assertGreater(peer.monitor_reads, peer.monitor_releases)
            self.assertEqual((peer.monitor_connections, peer.monitor_releases), (2, 2))
            self.assertLess(client.maximum_packet_bytes, batch_mcp.MAX_OUTPUT)
            self.assert_read_only_monitor(peer, commits)
            print("MAXIMUM_32_FURNITURE_COMPLETION_MCP_RESPONSE_BYTES", client.maximum_packet_bytes)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    args, remaining = parser.parse_known_args()
    executable = args.binary.resolve(strict=True)
    if not executable.is_file() or not os.access(executable, os.X_OK):
        parser.error("--binary must name an executable regular file")
    stdio.EXECUTABLE = executable
    print("MCP_COMPLETION_BINARY_SHA256", hashlib.sha256(executable.read_bytes()).hexdigest())
    program = unittest.main(argv=[__file__, *remaining], exit=False)
    print("MAXIMUM_COMPLETION_MCP_RESPONSE_BYTES", MAXIMUM_COMPLETION_PACKET_BYTES)
    raise SystemExit(0 if program.result.wasSuccessful() else 1)


if __name__ == "__main__":
    main()
