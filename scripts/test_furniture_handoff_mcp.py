#!/usr/bin/env python3
"""Exercise native inventory allocation into durable exact Rust MCP batches.

The real furniture MCP executable reads operations/1.4, retains the complete
request and chosen identities, and later prepares/commits furniture/1.19 against
an independent joined TCP peer. No Python implementation substitutes for the
Rust process. These are development regressions, not live-game admission.
Beads: df-dfhack-bridge-plane-c-pic.3 / df-dfhack-bridge-plane-c-pic.4.
"""
from __future__ import annotations

import argparse
from dataclasses import replace
import hashlib
import json
import os
from pathlib import Path
import struct
import tempfile
import unittest

import furniture_allocation as model
import furniture_plan as plan_model
import test_build_placement_mcp as stdio
import test_furniture_batch_mcp as batch_mcp
import test_furniture_completion_mcp as completion_mcp
from test_construction_monitor_rpc import read_message
from test_construction_receipt import operations
from test_furniture_allocation_mcp import item as inventory_item
from test_furniture_batch import proto


def request(slots=None, excluded=()):
    return model.Request("test-fort", 2, tuple(slots or (
        model.Slot("a", "bed", (10, 10, 2), max_distance=20),
        model.Slot("b", "bed", (11, 10, 2), after=("a",), material=(7, 8),
                   subtype=-1, max_distance=20),
    )), excluded)


def candidates():
    return (model.Candidate(42, "bed", (10, 10, 2), (7, 8), -1),
            model.Candidate(43, "bed", (20, 10, 2), (3, -1), -1))


class ReplayedHandshake:
    """Return the consumed routing prefix to an unchanged placement handler."""

    def __init__(self, sock, prefix):
        self.sock, self.prefix = sock, prefix
        self.replied = False

    def recv(self, size):
        if self.prefix:
            result, self.prefix = self.prefix[:size], self.prefix[size:]
            return result
        return self.sock.recv(size)

    def sendall(self, raw):
        if not self.replied:
            assert raw == b"DFHack!\n" + struct.pack("<i", 1)
            self.replied = True
        else:
            self.sock.sendall(raw)


class NativePeer(completion_mcp.NativePeer):
    """Serve both profiles on one listener with independent generations."""

    def __init__(self, requested, supplied):
        self.requested = requested
        self.supplied = tuple(supplied)
        self.expected = model.allocate(requested, self.supplied)
        if self.expected["plan"] is None:
            plan = plan_model.FurniturePlan(tuple(
                plan_model.Step(s.name, s.kind, i, s.target, s.after)
                for i, s in enumerate(requested.slots)))
        else:
            plan = plan_model.FurniturePlan.from_json(self.expected["plan"])
        self.operations_generation = 987
        self.operations_software = None
        self.operations_folder = None
        self.operations_hook = None
        self.allocation_captures = self.allocation_pages = self.allocation_releases = 0
        self.allocation_bindings = []
        self.allocation_payloads = []
        self.placement_change = lambda capture: capture
        super().__init__(plan)

    def capture(self, selection):
        current = super().capture(selection)
        candidate = next(v for v in self.supplied if v.id == selection.item)
        item = replace(current.item, pos=candidate.position, subtype=candidate.subtype,
                       material=candidate.material[0], material_index=candidate.material[1],
                       native_type=101 + plan_model.KINDS.index(candidate.kind))
        return self.placement_change(replace(current, item=item))

    def allocation_payload(self):
        rows = tuple(inventory_item(v.id, v.kind.upper(), x=v.position[0], y=v.position[1],
                                    z=v.position[2], material=v.material[0],
                                    material_index=v.material[1], subtype=v.subtype)
                     for v in sorted(self.supplied, key=lambda v: v.id))
        return operations(self.tick, jobs=(), buildings=(), items=rows,
                          folder=self.operations_folder or self.folder, site=self.site,
                          horizons=(self.next_job, self.next_building,
                                    max((v.id for v in self.supplied), default=0) + 1))

    def connection(self, sock):
        greeting = self.read(sock, 12)
        assert greeting == b"DFHack?\n" + struct.pack("<i", 1)
        self.send(sock, b"DFHack!\n" + struct.pack("<i", 1))
        header = self.read(sock, 8)
        method, size = struct.unpack("<h2xi", header)
        assert method == 0 and 0 <= size <= 2048
        raw = self.read(sock, size)
        first = read_message(raw)
        replay = ReplayedHandshake(sock, greeting + header + raw)
        if first[4] == b"dfmcp_build_v1_19":
            return super().connection(replay)
        assert first[4] == b"dfmcp_operations_v1_4"
        self.allocation_connection(replay)

    def allocation_connection(self, sock):
        assert self.read(sock, 12) == b"DFHack?\n" + struct.pack("<i", 1)
        self.send(sock, b"DFHack!\n" + struct.pack("<i", 1))
        methods, limits, nonce, raw, offset, released = {}, None, None, None, 0, False
        while not self.closed.is_set():
            method, size = struct.unpack("<h2xi", self.read(sock, 8))
            assert 0 <= size <= 2048
            fields = read_message(self.read(sock, size))
            if method == 0:
                assert set(fields) == {1, 2, 3, 4}
                name = fields[1].decode("ascii")
                assert name in ("Handshake", "ReadObservation") and name not in methods.values()
                assert fields[2] == b"dfmcp.operations.v1_4.Request"
                assert fields[3] == b"dfmcp.operations.v1_4.Reply"
                assert fields[4] == b"dfmcp_operations_v1_4"
                identity = len(methods) + 2
                methods[identity] = name
                self.allocation_bindings.append(name)
                response, tag = {1: identity}, "bind"
            else:
                name = methods[method]
                self.calls.append("Operations." + name)
                assert fields[1] == completion_mcp.OPS_SECRET.encode()
                assert len(fields[2]) == 32 and fields[3] == 1 and fields[4] == 4
                expected = set(range(1, 9)) | {10, 11, 12}
                assert set(fields) in (expected, expected | {9})
                actual = tuple(fields[n] for n in (5, 6, 7, 8, 11))
                if limits is None:
                    limits, nonce = actual, fields[2]
                assert actual == limits and fields[2] == nonce
                assert limits[:3] == (4096, 4096, 65536)
                assert limits[3] == 16 * 1024 * 1024 and limits[4] == 65536
                df, dfhack = self.operations_software or (self.df, self.dfhack)
                response = {1: 1, 2: 0, 3: nonce, 4: 1, 5: 4,
                            6: self.operations_generation, 7: df.encode(), 8: dfhack.encode()}
                token, requested_offset, release = fields.get(9, b""), fields[10], fields[12]
                if name == "Handshake":
                    assert not token and requested_offset == 0 and release == 0 and raw is None
                    tag = "handshake"
                elif release:
                    assert release == 1 and token == b"a" * 16 and requested_offset == 0
                    assert raw is not None and offset == len(raw) and not released
                    released = True
                    self.allocation_releases += 1
                    response[10], tag = token, "release"
                else:
                    assert not released
                    if not token:
                        assert raw is None and requested_offset == 0
                        raw = self.allocation_payload()
                        self.allocation_payloads.append(raw)
                        self.allocation_captures += 1
                    else:
                        assert raw is not None and token == b"a" * 16
                    assert requested_offset == offset
                    page = raw[offset:offset + limits[4]]
                    response.update({9: page, 10: b"a" * 16, 11: offset, 12: len(raw),
                                     13: hashlib.sha256(raw).digest(),
                                     14: int(offset + len(page) == len(raw))})
                    offset += len(page)
                    self.allocation_pages += 1
                    tag = "page"
            if self.operations_hook:
                response = self.operations_hook(tag, fields, response)
            encoded = proto(response)
            self.send(sock, struct.pack("<h2xi", -1, len(encoded)) + encoded)


class FurnitureHandoffMcpTests(unittest.TestCase):
    ok = batch_mcp.FurnitureBatchMcpTests.ok
    refused = batch_mcp.FurnitureBatchMcpTests.refused
    batch = batch_mcp.FurnitureBatchMcpTests.batch
    inventory = batch_mcp.FurnitureBatchMcpTests.inventory
    prepare = batch_mcp.FurnitureBatchMcpTests.prepare
    effect = batch_mcp.FurnitureBatchMcpTests.effect

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="dfmcp-rust-furniture-handoff-")
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name)
        self.directory.chmod(0o700)
        self.expected_plans = {}

    def env(self, peer, **kwargs):
        return completion_mcp.environment(self.directory, peer, **kwargs)

    def open_request(self, client, peer):
        packet = client.call("open_session", furniture_request=json.dumps(peer.requested.json()),
                             max_wall_millis=60000)
        result = self.ok(packet)
        self.assertTrue(result["session_opened"], packet)
        self.assertFalse(result["game_mutation_dispatched"])
        self.assertFalse(result["native_preparation_dispatched"])
        self.batch(packet, peer.plan)
        summary = result["batch"]["allocation"]
        for key in ("handoff_digest", "request_digest", "capture_sha256", "source_digest"):
            self.assertRegex(summary[key], r"^[0-9a-f]{64}$")
        self.assertEqual(summary["request_digest"], peer.requested.digest)
        self.assertEqual(summary["capture_sha256"], hashlib.sha256(peer.allocation_payloads[0]).hexdigest())
        self.assertEqual(summary["operations_generation"], peer.operations_generation)
        self.assertEqual(summary["captured_tick"], peer.tick)
        self.assertTrue(summary["constraints_retained"])
        self.assertFalse(summary["items_reserved"])
        self.assertEqual(peer.allocation_captures, 1)
        self.assertEqual(peer.allocation_releases, 1)
        self.assertEqual(peer.allocation_bindings, ["Handshake", "ReadObservation"])
        self.assertNotIn("PreparePlacement", peer.calls)
        self.assertNotIn("CommitPlacement", peer.calls)
        return result["session_id"], packet

    def allocation(self, client, session, peer, view=None):
        calls, connections = list(peer.calls), peer.connections
        before = {n: (self.directory / n).read_bytes() for n in ("batch", "journal")}
        query = {"mode": "allocation"}
        if view is not None:
            query["view"] = view
        packet = client.call("query", session_id=session, query=json.dumps(query))
        value = self.ok(packet)["allocation"]
        self.assertEqual(peer.calls, calls)
        self.assertEqual(peer.connections, connections)
        self.assertEqual({n: (self.directory / n).read_bytes() for n in before}, before)
        self.assertEqual(value["request_digest"], peer.requested.digest)
        if view in (None, "request"):
            self.assertEqual(value["request"], peer.requested.json())
        else:
            expected = {row["slot"]: row for row in peer.expected["assignments"]}
            self.assertEqual({row["slot"] for row in value["items"]}, set(expected))
            for row in value["items"]:
                wanted = expected[row["slot"]]
                self.assertEqual(row["item_id"], wanted["item"])
                for key in ("kind", "position", "material", "subtype", "distance"):
                    self.assertEqual(row[key], wanted[key])
                self.assertEqual(row["native_type"], 101 + plan_model.KINDS.index(wanted["kind"]))
                self.assertIn("entity_id", row)
                self.assertIn("generation", row)
                self.assertIn("revision", row)
        return value

    def assert_no_original_files(self):
        for name in ("journal", "batch", "completion"):
            self.assertFalse((self.directory / name).exists(), name)

    def test_global_scarce_material_allocation_retains_constraints_and_executes_exact_items(self):
        supplied = candidates() + (model.Candidate(44, "bed", (11, 10, 2), (7, 8), -1),)
        with NativePeer(request(excluded=(44,)), supplied) as peer, \
                completion_mcp.Client(self.env(peer)) as client:
            client.tools()
            session, _ = self.open_request(client, peer)
            self.assertEqual([s.item for s in peer.plan.ordered], [43, 42])
            self.allocation(client, session, peer)
            self.allocation(client, session, peer, "request")
            self.allocation(client, session, peer, "items")
            for index in range(2):
                review = self.prepare(client, session, peer.plan, peer, index)
                packet = client.call("commit", **review)
                self.effect(packet, peer, review["idempotency_key"], "placed")
            self.assertEqual(self.batch(packet, peer.plan)["status"], "all_placed")
            self.assertEqual(peer.calls.count("PreparePlacement"), 2)
            self.assertEqual(peer.calls.count("CommitPlacement"), 2)
            self.assertEqual(peer.allocation_captures, 1)

    def test_shortage_has_complete_witness_no_partial_batch_or_placement_rpc(self):
        with NativePeer(request(), candidates()[:1]) as peer, \
                completion_mcp.Client(self.env(peer)) as client:
            packet = client.call("open_session", furniture_request=json.dumps(peer.requested.json()),
                                 max_wall_millis=60000)
            result = self.ok(packet)
            self.assertEqual(result["status"], "furniture_shortage")
            self.assertFalse(result["session_opened"])
            self.assertIsNone(result["session_id"])
            value = result["allocation"]
            self.assertEqual(value["request"], peer.requested.json())
            self.assertEqual(value["maximum_assignable"], 1)
            for key in ("slots", "candidate_items", "missing"):
                self.assertEqual(value["shortage"][key], peer.expected["shortage"][key])
            self.assertTrue(all(call.startswith("Operations.") for call in peer.calls))
            self.assertEqual((peer.allocation_captures, peer.allocation_releases), (1, 1))
            self.assert_no_original_files()

    def test_malformed_conflicting_and_unauthorized_requests_fail_before_native_work(self):
        wanted = request().json()
        bad = [dict(wanted, slots=[]), dict(wanted, unknown=True), dict(wanted, site=True),
               dict(wanted, world_folder="different-fortress"), dict(wanted, site=3),
               dict(wanted, excluded_items=[42, 42]),
               dict(wanted, slots=[dict(wanted["slots"][0], max_distance=-1)]),
               dict(wanted, slots=[dict(wanted["slots"][0], after=["missing"])])]
        with NativePeer(request(), candidates()) as peer:
            with completion_mcp.Client(self.env(peer)) as client:
                for value in bad:
                    with self.subTest(value=value):
                        self.refused(client.call("open_session", furniture_request=json.dumps(value)))
                        self.assert_no_original_files()
                for extra in ({"selection": '["bed",42,10,10,2]'},
                              {"furniture_plan": json.dumps(peer.plan.json())}):
                    self.refused(client.call("open_session", furniture_request=json.dumps(wanted), **extra))
                    self.assert_no_original_files()
            for mode, construct in (("offline", False), ("recover", False), ("control", False)):
                with self.subTest(mode=mode), completion_mcp.Client(
                        self.env(peer, mode=mode, construct=construct)) as client:
                    self.refused(client.call("open_session", furniture_request=json.dumps(wanted)))
                    self.assert_no_original_files()
            self.assertEqual(peer.connections, 0)
            self.assertEqual(peer.calls, [])

    def test_existing_parent_and_restart_preserve_allocation_without_native_reallocation(self):
        with NativePeer(request(), candidates()) as peer:
            with completion_mcp.Client(self.env(peer)) as client:
                session, opened = self.open_request(client, peer)
                original_summary = opened["result"]["batch"]["allocation"]
                review = self.prepare(client, session, peer.plan, peer, 0)
                self.effect(client.call("commit", **review), peer, review["idempotency_key"], "placed")
            originals = {n: (self.directory / n).read_bytes() for n in ("batch", "journal")}
            calls, connections = list(peer.calls), peer.connections
            with completion_mcp.Client(self.env(peer)) as client:
                self.refused(client.call("open_session", furniture_request=json.dumps(peer.requested.json())))
                self.assertEqual(peer.calls, calls)
                self.assertEqual(peer.connections, connections)
                opened = client.call("open_session", max_wall_millis=60000)
                session = self.ok(opened)["session_id"]
                self.assertEqual(opened["result"]["batch"]["allocation"], original_summary)
                self.assertEqual(peer.calls, calls)
                self.assertEqual(peer.connections, connections)
                self.assertEqual({n: (self.directory / n).read_bytes() for n in originals}, originals)
                self.allocation(client, session, peer, "items")
                review = self.prepare(client, session, peer.plan, peer, 1)
                packet = client.call("commit", **review)
                self.effect(packet, peer, review["idempotency_key"], "placed")
                self.assertEqual(self.batch(packet, peer.plan)["status"], "all_placed")
            self.assertEqual(peer.allocation_captures, 1)
            self.assertEqual(len(peer.commits), 2)

    def test_changed_constraints_and_allocation_horizons_refuse_before_prepare(self):
        wanted = request((model.Slot("bed", "bed", (10, 10, 2), material=(3, -1),
                                     subtype=-1, max_distance=20),))
        supplied = (candidates()[1],)
        changes = {
            "material": lambda c: replace(c, item=replace(c.item, material=7, material_index=8)),
            "subtype": lambda c: replace(c, item=replace(c.item, subtype=4)),
            "native_type": lambda c: replace(c, item=replace(c.item, native_type=104)),
            "distance": lambda c: replace(c, item=replace(c.item, pos=(31, 10, 2))),
            "level": lambda c: replace(c, item=replace(c.item, pos=(20, 10, 3))),
            "building_horizon": lambda c: replace(c, next_building=99),
            "job_horizon": lambda c: replace(c, next_job=199),
            "tick": lambda c: replace(c, tick=899),
        }
        for label, change in changes.items():
            with self.subTest(change=label), tempfile.TemporaryDirectory(dir=self.directory) as raw:
                old, self.directory = self.directory, Path(raw)
                self.directory.chmod(0o700)
                try:
                    with NativePeer(wanted, supplied) as peer, completion_mcp.Client(self.env(peer)) as client:
                        session, opened = self.open_request(client, peer)
                        peer.placement_change = change
                        observed = client.call("observe", session_id=session, selection="next")
                        if observed["result"]["ok"]:
                            observed = client.call("plan", session_id=session,
                                idempotency_key=opened["result"]["batch"]["next"]["idempotency_key"],
                                observation_witness=observed["result"]["observation"]["observation_witness"])
                        self.refused(observed)
                        self.assertNotIn("PreparePlacement", peer.calls)
                        self.assertNotIn("CommitPlacement", peer.calls)
                        self.assertEqual(peer.records, {})
                finally:
                    self.directory = old

    def test_high_tick_empty_child_restart_preserves_lease_for_first_placement(self):
        with NativePeer(request(), candidates()) as peer:
            peer.tick = 806500
            with completion_mcp.Client(self.env(peer)) as client:
                _session, opened = self.open_request(client, peer)
                self.assertEqual(opened["result"]["batch"]["placed"], 0)
                self.assertEqual(peer.records, {})
            originals = {n: (self.directory / n).read_bytes() for n in ("batch", "journal")}
            calls, connections = list(peer.calls), peer.connections
            with completion_mcp.Client(self.env(peer)) as client:
                opened = client.call("open_session", max_wall_millis=60000)
                session = self.ok(opened)["session_id"]
                self.assertEqual(opened["result"]["batch"]["allocation"]["captured_tick"], 806500)
                self.assertEqual(peer.calls, calls, "empty-child restart performed native bootstrap")
                self.assertEqual(peer.connections, connections)
                self.assertEqual({n: (self.directory / n).read_bytes() for n in originals}, originals)
                self.allocation(client, session, peer)
                # Inspection alone does not expose an incorrectly renewed lease
                # at tick zero. The first fresh observation and effect must use
                # the retained inventory tick before the child has any receipt.
                review = self.prepare(client, session, peer.plan, peer, 0)
                packet = client.call("commit", **review)
                self.effect(packet, peer, review["idempotency_key"], "placed")
                self.assertEqual(self.batch(packet, peer.plan)["placed"], 1)
            self.assertEqual(peer.allocation_captures, 1)
            self.assertEqual(peer.calls.count("PreparePlacement"), 1)
            self.assertEqual(peer.calls.count("CommitPlacement"), 1)

    def test_item_may_move_within_original_radius_without_reallocation(self):
        wanted = request((model.Slot("bed", "bed", (10, 10, 2), material=(3, -1),
                                     subtype=-1, max_distance=20),))
        with NativePeer(wanted, (candidates()[1],)) as peer, completion_mcp.Client(self.env(peer)) as client:
            session, _ = self.open_request(client, peer)
            peer.placement_change = lambda c: replace(c, item=replace(c.item, pos=(25, 10, 2)))
            review = self.prepare(client, session, peer.plan, peer)
            packet = client.call("commit", **review)
            self.effect(packet, peer, review["idempotency_key"], "placed")
            self.assertEqual(peer.records[review["idempotency_key"]].plan.before.selection.item, 43)
            self.assertEqual(peer.allocation_captures, 1)

    def test_source_and_paged_capture_disagreement_cannot_create_original_files(self):
        mutations = ("software", "folder", "generation", "digest", "release")
        for label in mutations:
            with self.subTest(change=label), tempfile.TemporaryDirectory(dir=self.directory) as raw:
                old, self.directory = self.directory, Path(raw)
                self.directory.chmod(0o700)
                try:
                    with NativePeer(request(), candidates()) as peer, completion_mcp.Client(self.env(peer)) as client:
                        if label == "software":
                            peer.operations_software = ("other-df", peer.dfhack)
                        elif label == "folder":
                            peer.operations_folder = "different-fortress"
                        else:
                            def corrupt(tag, fields, reply, boundary=label):
                                if boundary == "generation" and tag == "page":
                                    reply[6] = peer.operations_generation + 1
                                if boundary == "digest" and tag == "page":
                                    reply[13] = b"x" * 32
                                if boundary == "release" and tag == "release":
                                    reply[10] = b"x" * 16
                                return reply
                            peer.operations_hook = corrupt
                        packet = client.call("open_session", furniture_request=json.dumps(peer.requested.json()),
                                             max_wall_millis=60000)
                        self.refused(packet)
                        self.assertNotIn("PreparePlacement", peer.calls)
                        self.assertNotIn("CommitPlacement", peer.calls)
                        self.assert_no_original_files()
                finally:
                    self.directory = old

    def test_32_wide_slots_allocate_paged_inventory_and_execute_complete_bounded_batch(self):
        expected_plan = batch_mcp.exact_plan(32, wide=True)
        wanted = request(tuple(model.Slot(s.name, s.kind, s.target, s.after, max_distance=0)
                               for s in expected_plan.steps))
        supplied = tuple(model.Candidate(s.item, s.kind, s.target, (3, -1), -1)
                         for s in expected_plan.steps)
        supplied += tuple(model.Candidate(n, "bed", (200, 200, 0), (3, -1), -1)
                          for n in range(1, 2001))
        with NativePeer(wanted, supplied) as peer, completion_mcp.Client(self.env(peer)) as client:
            self.assertEqual(peer.plan, expected_plan)
            self.assertGreater(len(peer.allocation_payload()), 65536)
            session, _ = self.open_request(client, peer)
            self.allocation(client, session, peer)
            self.allocation(client, session, peer, "items")
            for index in range(32):
                review = self.prepare(client, session, peer.plan, peer, index)
                packet = client.call("commit", **review)
                self.effect(packet, peer, review["idempotency_key"], "placed")
            self.assertEqual(self.batch(packet, peer.plan)["status"], "all_placed")
            self.assertEqual((peer.allocation_captures, peer.allocation_releases), (1, 1))
            self.assertGreater(peer.allocation_pages, 1)
            self.assertEqual(len(peer.commits), 32)
            self.assertLess(client.maximum_packet_bytes, batch_mcp.MAX_OUTPUT)
            print("MAXIMUM_32_FURNITURE_HANDOFF_MCP_RESPONSE_BYTES", client.maximum_packet_bytes)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    args, remaining = parser.parse_known_args()
    executable = args.binary.resolve(strict=True)
    if not executable.is_file() or not os.access(executable, os.X_OK):
        parser.error("--binary must name an executable regular file")
    stdio.EXECUTABLE = executable
    print("MCP_HANDOFF_BINARY_SHA256", hashlib.sha256(executable.read_bytes()).hexdigest())
    program = unittest.main(argv=[__file__, *remaining], exit=False)
    print("MAXIMUM_HANDOFF_MCP_RESPONSE_BYTES", completion_mcp.MAXIMUM_COMPLETION_PACKET_BYTES)
    raise SystemExit(0 if program.result.wasSuccessful() else 1)


if __name__ == "__main__":
    main()
