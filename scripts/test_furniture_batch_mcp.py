#!/usr/bin/env python3
"""Exercise compiled Rust exact furniture batches through real modern MCP stdio.

The joined furniture/1.19 TCP peer and canonical Python codec are independent of
the Rust implementation under test. These tests exercise the executable, public
eleven-tool surface, native framing, private parent custody, and original build
journal. They do not execute DFHack or establish live-game or production admission.

Build dfmcp-build-placement-dev-server and supply its absolute path as --binary.
There is no alternate implementation or successful skip when Rust is unavailable.
Beads: df-dfhack-bridge-plane-c-pic.4 / df-dfhack-bridge-plane-c-pic.5.
"""
from __future__ import annotations

import argparse
from copy import deepcopy
from dataclasses import replace
import json
import os
from pathlib import Path
import selectors
import tempfile
import time
import unittest

import build_placement_wire as wire
import furniture_plan as model
import test_build_placement_mcp as stdio
from test_furniture_batch import Peer as FurniturePeer


MAX_OUTPUT = 65_536
PEER_SECRET = "t" * 32
MAXIMUM_OBSERVED_PACKET_BYTES = 0


def exact_plan(count=3, *, wide=False):
    """Reverse input order; the independent compiler determines execution order."""
    names = [f"room{i:02}" + ("x" * 42 if wide else "") for i in range(count)]
    return model.FurniturePlan(tuple(
        model.Step(names[i], model.KINDS[i % 3], 2_147_483_600 + i if wide else 42 + i,
                   (10 + 3 * i, 10, i % 4), (names[i - 1],) if i else ())
        for i in reversed(range(count))
    ))


def selected(step):
    return [step.kind, step.item, *step.target]


def native_selection(step):
    return wire.Selection(model.KINDS.index(step.kind) + 1, step.item, *step.target)


def environment(directory, peer, *, mode="control", credentials=True):
    values = stdio.environment(directory, peer.address, mode=mode, credentials=credentials)
    values.update({
        "DFMCP_BUILD_BATCH": str(directory / "batch"),
        "DFMCP_BUILD_WORLD_FOLDER": peer.folder,
        "DFMCP_BUILD_SCOPE": "[0,0,0,255,255,15]",
    })
    if credentials:
        values["DFMCP_BUILD_TOKEN"] = PEER_SECRET
    return values


class NativePeer(FurniturePeer):
    """Reuse the independent dynamic peer, always join it and preserve failures."""

    def __enter__(self):
        return self

    def __exit__(self, kind, error, trace):
        try:
            self.close()
        except BaseException as cleanup:
            if error is None:
                raise
            error.add_note(f"Native peer cleanup also failed: {cleanup!r}")


class Client(stdio.Stdio):
    """Measure the exact complete Agent Turn bytes, with a bounded child lifetime."""

    def __init__(self, env):
        super().__init__(env)
        self.deadline = time.monotonic() + 120
        self.maximum_packet_bytes = 0
        os.set_blocking(self.child.stdin.fileno(), False)
        self.writer = selectors.DefaultSelector()
        self.writer.register(self.child.stdin, selectors.EVENT_WRITE)

    def __exit__(self, *arguments):
        self.writer.close()
        return super().__exit__(*arguments)

    def request(self, method, params=None):
        self.sequence += 1
        request = {"jsonrpc": "2.0", "id": self.sequence, "method": method,
                   "params": {"_meta": stdio.META, **(params or {})}}
        encoded = json.dumps(request, separators=(",", ":")).encode("utf-8") + b"\n"
        deadline = min(self.deadline, time.monotonic() + 10)
        offset = 0
        # The oversized-plan case is larger than some pipe buffers. Bound writes
        # as well as reads so a stalled executable cannot strand the test runner.
        while offset < len(encoded):
            remaining = deadline - time.monotonic()
            if remaining <= 0 or not self.writer.select(remaining):
                raise AssertionError(f"MCP did not consume {method} within its independent deadline")
            try:
                count = os.write(self.child.stdin.fileno(), encoded[offset:])
            except BlockingIOError:
                continue
            if count <= 0:
                raise AssertionError(f"MCP closed stdin while writing {method}")
            offset += count
        while b"\n" not in self.buffer:
            remaining = deadline - time.monotonic()
            if remaining <= 0 or not self.selector.select(remaining):
                raise AssertionError(f"MCP did not answer {method} within its independent deadline")
            chunk = os.read(self.child.stdout.fileno(), 65_536)
            if not chunk:
                self.errors.seek(0)
                diagnostic = self.errors.read(4096).decode(errors="replace")
                diagnostic = diagnostic.replace(stdio.SECRET.decode(), "<secret>").replace(PEER_SECRET, "<secret>")
                raise AssertionError(f"MCP process closed stdout during {method}: {diagnostic}")
            self.buffer += chunk
            if len(self.buffer) > 1_048_576:
                raise AssertionError("MCP response exceeded the independent 1 MiB transport bound")
        line, self.buffer = self.buffer.split(b"\n", 1)
        assert PEER_SECRET.encode() not in line, "native credential leaked into MCP output"
        assert stdio.SECRET not in line, "native credential leaked into MCP output"
        response = json.loads(line)
        assert response.get("jsonrpc") == "2.0" and response.get("id") == self.sequence, response
        assert "error" not in response, response
        return response["result"]

    def call(self, tool, **arguments):
        global MAXIMUM_OBSERVED_PACKET_BYTES
        result = self.request("tools/call", {"name": "fortress." + tool, "arguments": arguments})
        texts = [part["text"] for part in result["content"] if part.get("type") == "text"]
        assert len(texts) == 1, result
        size = len(texts[0].encode("utf-8"))
        assert size < MAX_OUTPUT, f"complete Agent Turn response exceeds 64 KiB: {size}"
        self.maximum_packet_bytes = max(self.maximum_packet_bytes, size)
        MAXIMUM_OBSERVED_PACKET_BYTES = max(MAXIMUM_OBSERVED_PACKET_BYTES, size)
        packet = json.loads(texts[0])
        assert packet["agent_turn"]["schema"] == "dfmcp.agent_turn/1", packet
        assert packet["agent_turn"]["anchor"] is None, "native identity is not a canonical world anchor"
        assert "active_work" in packet["agent_turn"], packet
        return packet


class FurnitureBatchMcpTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name)
        self.directory.chmod(0o700)
        self.expected_plans = {}

    def ok(self, packet):
        self.assertTrue(packet["result"]["ok"], packet)
        return packet["result"]

    def refused(self, packet):
        self.assertFalse(packet["result"]["ok"], packet)
        self.assertFalse(packet["result"]["retry_commit_permitted"], packet)
        return packet["result"]

    def open(self, client, plan=None):
        arguments = {"max_wall_millis": 60000}
        if plan is not None:
            value = plan.json()
            value["steps"].reverse()
            for step in value["steps"]:
                if not step["after"]:
                    del step["after"]
            arguments["furniture_plan"] = json.dumps(value)
        packet = client.call("open_session", **arguments)
        result = self.ok(packet)
        self.assertFalse(result["game_mutation_dispatched"], packet)
        self.assertFalse(result["native_preparation_dispatched"], packet)
        return result["session_id"], packet

    def batch(self, packet, plan, *, verified=True):
        batch = packet["result"]["batch"]
        active = packet["agent_turn"]["active_work"]
        self.assertEqual(batch["plan_digest"], plan.digest, packet)
        self.assertEqual(batch["plan"], plan.json(), packet)
        self.assertEqual(batch["inventory_verified"], verified, packet)
        self.assertEqual(active["inventory_verified"], verified, packet)
        self.assertTrue(active["original_build_journal_inventory_verified"], packet)
        self.assertEqual(batch["total"], len(plan.steps), packet)
        self.assertEqual([step["name"] for step in batch["steps"]],
                         [step.name for step in plan.ordered], packet)
        self.assertRegex(batch["batch_id"], r"^[0-9a-f]{64}$")
        for row, step in zip(batch["steps"], plan.ordered):
            self.assertEqual(row["idempotency_key"], f"fb-{batch['batch_id']}-{step.name}")
            self.assertIn(row["state"], ("not_started", "intent", "prepared", "dispatch_started",
                                        "tracking", "cancel_requested", "terminal"))
            self.assertIn(row["outcome"], (None, "prepared", "placed", "refused", "cancelled", "indeterminate"))
            if row["state"] == "not_started":
                self.assertIsNone(row["native_plan_digest"])
                self.assertIsNone(row["outcome"])
            else:
                self.assertRegex(row["native_plan_digest"], r"^[0-9a-f]{64}$")
                expected = self.expected_plans.get(row["idempotency_key"])
                self.assertIsNotNone(expected, "started row lacks an independently witnessed expected plan")
                self.assertEqual(row["native_plan_digest"], expected.digest.hex())
        self.assertFalse(batch["atomic"], packet)
        self.assertFalse(batch["construction_completion_proven"], packet)
        self.assertFalse(batch["retry_permitted"], packet)
        if not verified:
            self.assertIsNone(batch["next"], "unverified parent custody cannot offer another placement")
            self.assertFalse(active["pending_absence_proven"], packet)
        return batch

    def inventory(self, client, session, plan, peer):
        before = list(peer.calls)
        packet = client.call("query", session_id=session, query='{"mode":"batch"}')
        result = self.ok(packet)
        self.assertEqual(result["native_calls"], 0, packet)
        self.assertEqual(peer.calls, before, "local batch query opened native work")
        return self.batch(packet, plan)

    def prepare(self, client, session, plan, peer, index=0):
        current = self.inventory(client, session, plan, peer)
        step = plan.ordered[index]
        next_step = current["next"]
        self.assertEqual(next_step["name"], step.name)
        key = next_step["idempotency_key"]
        self.assertEqual(key, f"fb-{current['batch_id']}-{step.name}")
        self.assertEqual(next_step["selection"], selected(step))
        expected = peer.capture(native_selection(step))
        observed = client.call("observe", session_id=session, selection="next")
        observation = self.ok(observed)["observation"]
        self.batch(observed, plan)
        self.assertEqual(observation["selection"], selected(step))
        self.assertEqual(observation["observation_witness"], expected.witness.hex())
        self.assertEqual(observation["canonical_capture_hex"], expected.raw.hex())
        self.assertTrue(observation["eligible"], observed)
        expected_plan = wire.Plan(key, expected)
        self.expected_plans[key] = expected_plan
        prepared = client.call("plan", session_id=session, idempotency_key=key,
                               observation_witness=observation["observation_witness"])
        result = self.ok(prepared)
        self.batch(prepared, plan)
        self.assertEqual(result["plan_digest"], expected_plan.digest.hex())
        self.assertRegex(result["review_seal"], r"^[0-9a-f]{64}$")
        self.assertFalse(result["game_mutation_dispatched"], prepared)
        self.assertEqual(peer.records[key].plan, expected_plan)
        return {"session_id": session, "idempotency_key": key,
                "plan_digest": result["plan_digest"], "review_seal": result["review_seal"]}

    def effect(self, packet, peer, key, phase):
        record = self.ok(packet)["effect"]
        expected = peer.records[key]
        self.assertEqual(expected.phase, phase)
        self.assertEqual(record["summary"]["native_phase"], phase)
        self.assertEqual(record["native"]["canonical_record_hex"], expected.raw.hex())
        self.assertEqual(wire.Record.decode(bytes.fromhex(record["native"]["canonical_record_hex"])), expected)
        self.assertFalse(record["building_completion_proven"])
        self.assertFalse(record["current_building_usable_proven"])
        self.assertFalse(record["retry_commit_permitted"])
        if phase == "placed":
            insertion = record["native"]["insertion"]
            before = expected.plan.before
            self.assertEqual((insertion["building_id"], insertion["job_id"], insertion["item_id"]),
                             (before.next_building, before.next_job, before.selection.item))
            self.assertEqual(insertion["position"], list(before.selection.target))
            self.assertEqual(insertion["kind"], model.KINDS[before.selection.kind - 1])
            self.assertEqual((insertion["stage"], insertion["max_stage"]), (0, 3))
            self.assertTrue(insertion["historical_evidence_only"])
        else:
            self.assertIsNone(record["native"]["insertion"])
        return record

    def test_32_exact_placements_fresh_reviews_complete_inventory_and_output_bound(self):
        plan = exact_plan(32, wide=True)
        with NativePeer(plan) as peer:
            peer.folder = "é" * 256
            peer.df, peer.dfhack = "d" * 128, "h" * 128
            peer.generation, peer.sequence = 2**64 - 2, 2**64 - 34
            peer.next_building = peer.next_job = 2_147_483_600
            with Client(environment(self.directory, peer)) as client:
                client.tools()
                session, opened = self.open(client, plan)
                initial = self.batch(opened, plan)
                self.assertEqual(initial["placed"], 0)
                self.assertFalse(initial["stopped"])
                self.assertEqual(peer.calls, ["Handshake", "ReadPlacement"])
                batch_id, seals, keys = initial["batch_id"], set(), []
                for index, step in enumerate(plan.ordered):
                    with self.subTest(step=step.name):
                        review = self.prepare(client, session, plan, peer, index)
                        self.assertNotIn(review["review_seal"], seals)
                        seals.add(review["review_seal"])
                        keys.append(review["idempotency_key"])
                        self.assertEqual(len(peer.commits), index)
                        committed = client.call("commit", **review)
                        self.effect(committed, peer, review["idempotency_key"], "placed")
                        current = self.batch(committed, plan)
                        self.assertEqual(current["batch_id"], batch_id)
                        self.assertEqual(current["placed"], index + 1)
                        self.assertEqual([row["outcome"] for row in current["steps"]],
                                         ["placed"] * (index + 1) + [None] * (31 - index))
                        self.assertEqual([row["state"] for row in current["steps"]],
                                         ["terminal"] * (index + 1) + ["not_started"] * (31 - index))
                        self.assertEqual(peer.commits, keys)
                        if index == 0:
                            native_before = list(peer.calls)
                            replay = client.call("commit", **review)
                            self.effect(replay, peer, review["idempotency_key"], "placed")
                            self.assertEqual(self.batch(replay, plan)["placed"], 1)
                            self.assertEqual(peer.calls, native_before)
                            # The previous step's valid review cannot authorize a new key.
                            denied = client.call("commit", **{
                                **review, "idempotency_key": current["next"]["idempotency_key"],
                            })
                            self.refused(denied)
                            self.assertEqual(peer.calls, native_before)
                            self.assertEqual(self.batch(denied, plan)["placed"], 1)
                final = self.inventory(client, session, plan, peer)
                self.assertEqual(final["status"], "all_placed")
                self.assertEqual(final["placed"], 32)
                self.assertIsNone(final["next"])
                self.assertIsNone(final["pending_step"])
                self.assertEqual(set(peer.placed), {step.target for step in plan.steps})
                self.assertEqual(peer.calls.count("PreparePlacement"), 32)
                self.assertEqual(peer.calls.count("CommitPlacement"), 32)
                self.assertNotIn("CancelPlacement", peer.calls)
                self.assertEqual(peer.connections, 33)
                self.assertLess(client.maximum_packet_bytes, MAX_OUTPUT)
                print("MAXIMUM_32_STEP_MCP_RESPONSE_BYTES", client.maximum_packet_bytes)
            before_calls = list(peer.calls)
            original = {name: (self.directory / name).read_bytes() for name in ("batch", "journal")}
            with Client(environment(self.directory, peer, mode="offline", credentials=False)) as client:
                session, opened = self.open(client)
                self.assertEqual(self.batch(opened, plan)["batch_id"], batch_id)
                self.assertEqual(self.inventory(client, session, plan, peer)["status"], "all_placed")
            self.assertEqual(peer.calls, before_calls)
            self.assertEqual({name: (self.directory / name).read_bytes() for name in original}, original)

    def test_wrong_item_step_key_plan_digest_and_review_never_dispatch(self):
        plan = exact_plan()
        with NativePeer(plan) as peer:
            with Client(environment(self.directory, peer)) as client:
                session, _opened = self.open(client, plan)
                initial = self.inventory(client, session, plan, peer)
                for selection in (selected(plan.ordered[1]),
                                  [plan.ordered[0].kind, plan.ordered[0].item + 100,
                                   *plan.ordered[0].target]):
                    with self.subTest(selection=selection):
                        native_before = list(peer.calls)
                        denied = client.call("observe", session_id=session,
                                             selection=json.dumps(selection))
                        self.refused(denied)
                        self.assertEqual(peer.calls, native_before)
                        self.assertEqual(self.batch(denied, plan)["placed"], 0)
                observed = client.call("observe", session_id=session,
                                       selection=json.dumps(selected(plan.ordered[0])))
                witness = self.ok(observed)["observation"]["observation_witness"]
                self.assertEqual(observed["result"]["observation"]["selection"], selected(plan.ordered[0]))
                native_before = list(peer.calls)
                denied = client.call("plan", session_id=session, idempotency_key="wrong-item-key",
                                     observation_witness=witness)
                self.refused(denied)
                self.assertEqual(peer.calls, native_before)
                self.assertEqual(self.inventory(client, session, plan, peer)["next"], initial["next"])
                self.assertNotIn("PreparePlacement", peer.calls)
                self.assertFalse(peer.records)

        for field in ("plan_digest", "review_seal"):
            with self.subTest(substituted=field), tempfile.TemporaryDirectory(dir=self.directory) as raw:
                directory = Path(raw)
                directory.chmod(0o700)
                with NativePeer(plan) as peer:
                    with Client(environment(directory, peer)) as client:
                        session, _opened = self.open(client, plan)
                        review = self.prepare(client, session, plan, peer)
                        native_before = list(peer.calls)
                        denied = client.call("commit", **{**review, field: "0" * 64})
                        self.refused(denied)
                        self.assertEqual(peer.calls, native_before)
                        batch = self.batch(denied, plan)
                        self.assertEqual(batch["placed"], 0)
                        self.assertEqual(batch["pending_step"], plan.ordered[0].name)
                        self.assertIsNone(batch["next"])
                        self.assertNotIn("CommitPlacement", peer.calls)
                        # Invalid confirmation retires local permission; recovery keeps the original key.
                        retired = client.call("cancel", session_id=session, scope="effect",
                                              idempotency_key=review["idempotency_key"],
                                              plan_digest=review["plan_digest"])
                        self.effect(retired, peer, review["idempotency_key"], "cancelled")
                        self.assertEqual(self.batch(retired, plan)["status"], "halted_cancelled")
                        self.assertFalse(peer.commits)

    def test_refused_child_halts_dependencies_and_independent_remaining_steps(self):
        base = exact_plan()
        # The third step is independent; fixed-prefix refusal still halts it.
        plan = model.FurniturePlan((*base.steps[:2], replace(base.steps[2], after=())))
        with NativePeer(plan) as peer:
            peer.refused = True
            with Client(environment(self.directory, peer)) as client:
                session, _opened = self.open(client, plan)
                review = self.prepare(client, session, plan, peer)
                refused = client.call("commit", **review)
                self.effect(refused, peer, review["idempotency_key"], "refused")
                current = self.batch(refused, plan)
                self.assertEqual(current["status"], "halted_refused")
                self.assertEqual(current["placed"], 0)
                self.assertIsNone(current["next"])
                native_before = list(peer.calls)
                denied = client.call("observe", session_id=session, selection="next")
                self.refused(denied)
                self.assertEqual(self.batch(denied, plan)["status"], "halted_refused")
                self.assertEqual(peer.calls, native_before)
                self.assertEqual(len(peer.records), 1)
                self.assertEqual(peer.calls.count("CommitPlacement"), 1)
                self.assertFalse(peer.commits)

    def test_lost_reply_control_restart_queries_original_key_then_requires_new_review(self):
        plan = exact_plan()
        with NativePeer(plan) as peer:
            with Client(environment(self.directory, peer)) as client:
                session, opened = self.open(client, plan)
                batch_id = self.batch(opened, plan)["batch_id"]
                review = self.prepare(client, session, plan, peer)
                peer.lost = True
                lost = client.call("commit", **review)
                self.refused(lost)
                pending = self.batch(lost, plan)
                self.assertEqual(pending["pending_step"], plan.ordered[0].name)
                self.assertIsNone(pending["next"])
                self.assertEqual(len(lost["agent_turn"]["active_work"]["pending_plans"]), 1)
            calls_at_restart = list(peer.calls)
            # A configured batch in control mode reopens its retained original source without bootstrap.
            with Client(environment(self.directory, peer)) as client:
                session, opened = self.open(client)
                pending = self.batch(opened, plan)
                self.assertEqual(pending["batch_id"], batch_id)
                self.assertEqual(pending["status"], "pending_recovery")
                self.assertEqual(peer.calls, calls_at_restart)
                self.inventory(client, session, plan, peer)
                denied = client.call("commit", **{**review, "session_id": session})
                self.refused(denied)
                self.assertEqual(peer.calls, calls_at_restart)
                recovered = client.call("wait", session_id=session,
                                        idempotency_key=review["idempotency_key"],
                                        plan_digest=review["plan_digest"])
                self.effect(recovered, peer, review["idempotency_key"], "placed")
                current = self.batch(recovered, plan)
                self.assertEqual(current["placed"], 1)
                self.assertEqual(current["next"]["name"], plan.ordered[1].name)
                self.assertEqual(peer.calls[len(calls_at_restart):], ["Handshake", "QueryPlacement"])
                self.assertEqual(peer.calls.count("CommitPlacement"), 1)
                self.assertEqual(peer.calls.count("PreparePlacement"), 1)
                native_before = list(peer.calls)
                # Recovered bytes never supply the next step's fresh observation or local review.
                denied = client.call("plan", session_id=session,
                                     idempotency_key=current["next"]["idempotency_key"],
                                     observation_witness=peer.records[review["idempotency_key"]].plan.before.witness.hex())
                self.refused(denied)
                self.assertEqual(peer.calls, native_before)
                next_review = self.prepare(client, session, plan, peer, 1)
                self.assertNotEqual(next_review["review_seal"], review["review_seal"])
                committed = client.call("commit", **next_review)
                self.effect(committed, peer, next_review["idempotency_key"], "placed")
                self.assertEqual(self.batch(committed, plan)["placed"], 2)
                self.assertEqual(peer.commits, [review["idempotency_key"], next_review["idempotency_key"]])

    def test_offline_pending_inventory_then_query_authorized_stop_survives_recovery(self):
        plan = exact_plan()
        with NativePeer(plan) as peer:
            with Client(environment(self.directory, peer)) as client:
                session, opened = self.open(client, plan)
                batch_id = self.batch(opened, plan)["batch_id"]
                review = self.prepare(client, session, plan, peer)
                peer.lost = True
                self.refused(client.call("commit", **review))
            calls_at_restart = list(peer.calls)
            original = {name: (self.directory / name).read_bytes() for name in ("batch", "journal")}
            with Client(environment(self.directory, peer, mode="offline", credentials=False)) as client:
                session, opened = self.open(client)
                pending = self.batch(opened, plan)
                self.assertEqual(pending["batch_id"], batch_id)
                self.assertEqual(pending["pending_step"], plan.ordered[0].name)
                self.assertEqual(self.inventory(client, session, plan, peer)["status"], "pending_recovery")
                self.refused(client.call("cancel", session_id=session, scope="batch"))
                self.assertEqual(peer.calls, calls_at_restart)
            self.assertEqual({name: (self.directory / name).read_bytes() for name in original}, original)
            recovery_env = environment(self.directory, peer, mode="recover")
            del recovery_env["DFMCP_BUILD_ALLOW_PLACE"]
            with Client(recovery_env) as client:
                session, _opened = self.open(client)
                stopped = client.call("cancel", session_id=session, scope="batch")
                self.ok(stopped)
                pending = self.batch(stopped, plan)
                self.assertTrue(pending["stopped"])
                self.assertEqual(pending["pending_step"], plan.ordered[0].name)
                self.assertEqual(peer.calls, calls_at_restart)
                self.assertNotIn("CancelPlacement", peer.calls)
                stopped_bytes = (self.directory / "batch").read_bytes()
                self.ok(client.call("cancel", session_id=session, scope="batch"))
                self.assertEqual((self.directory / "batch").read_bytes(), stopped_bytes)
                recovered = client.call("wait", session_id=session,
                                        idempotency_key=review["idempotency_key"],
                                        plan_digest=review["plan_digest"])
                self.effect(recovered, peer, review["idempotency_key"], "placed")
                stopped = self.batch(recovered, plan)
                self.assertTrue(stopped["stopped"])
                self.assertEqual(stopped["placed"], 1)
                self.assertIsNone(stopped["next"])
            after_recovery = list(peer.calls)
            with Client(environment(self.directory, peer)) as client:
                session, opened = self.open(client)
                stopped = self.batch(opened, plan)
                self.assertTrue(stopped["stopped"])
                self.assertEqual(stopped["batch_id"], batch_id)
                self.refused(client.call("observe", session_id=session, selection="next"))
                self.assertEqual(peer.calls, after_recovery)
                self.assertEqual(peer.calls.count("CommitPlacement"), 1)
                self.assertEqual(len(peer.commits), 1)
                self.assertNotIn("CancelPlacement", peer.calls)

    def test_parent_loss_or_same_bytes_substitution_blocks_commit_but_keeps_original_recovery(self):
        plan = exact_plan()
        for mutation in ("remove", "replace"):
            with self.subTest(custody=mutation), tempfile.TemporaryDirectory(dir=self.directory) as raw:
                directory = Path(raw)
                directory.chmod(0o700)
                with NativePeer(plan) as peer:
                    with Client(environment(directory, peer)) as client:
                        session, _opened = self.open(client, plan)
                        review = self.prepare(client, session, plan, peer)
                        parent = directory / "batch"
                        journal_before = (directory / "journal").read_bytes()
                        if mutation == "remove":
                            parent.unlink()
                        else:
                            replacement = directory / "different-inode"
                            replacement.write_bytes(parent.read_bytes())
                            replacement.chmod(0o600)
                            os.replace(replacement, parent)
                        native_before = list(peer.calls)
                        denied = client.call("commit", **review)
                        self.refused(denied)
                        unverified = self.batch(denied, plan, verified=False)
                        self.assertEqual(unverified["pending_step"], plan.ordered[0].name)
                        self.assertEqual(peer.calls, native_before)
                        self.assertEqual((directory / "journal").read_bytes(), journal_before)
                        self.assertNotIn("CommitPlacement", peer.calls)
                        recovered = client.call("wait", session_id=session,
                                                idempotency_key=review["idempotency_key"],
                                                plan_digest=review["plan_digest"])
                        self.effect(recovered, peer, review["idempotency_key"], "prepared")
                        self.batch(recovered, plan, verified=False)
                        self.assertEqual(peer.calls[len(native_before):], ["Handshake", "QueryPlacement"])
                        retired = client.call("cancel", session_id=session, scope="effect",
                                              idempotency_key=review["idempotency_key"],
                                              plan_digest=review["plan_digest"])
                        self.effect(retired, peer, review["idempotency_key"], "cancelled")
                        self.batch(retired, plan, verified=False)
                        self.assertFalse(peer.commits)
                        self.assertNotIn("CommitPlacement", peer.calls)

    def test_restart_missing_parent_refuses_batch_open_and_recovers_original_journal(self):
        plan = exact_plan()
        with NativePeer(plan) as peer:
            with Client(environment(self.directory, peer)) as client:
                session, _opened = self.open(client, plan)
                review = self.prepare(client, session, plan, peer)
            (self.directory / "batch").unlink()
            calls_before = list(peer.calls)
            original = (self.directory / "journal").read_bytes()
            with Client(environment(self.directory, peer)) as client:
                self.refused(client.call("open_session", max_wall_millis=60000))
                self.assertEqual(peer.calls, calls_before)
                self.assertFalse((self.directory / "batch").exists())
                self.assertEqual((self.directory / "journal").read_bytes(), original)

            # The operator explicitly selects the original child journal for
            # Query-only recovery; a missing parent never becomes a new batch.
            recovery_env = environment(self.directory, peer, mode="recover")
            del recovery_env["DFMCP_BUILD_BATCH"]
            del recovery_env["DFMCP_BUILD_ALLOW_PLACE"]
            with Client(recovery_env) as client:
                session, opened = self.open(client)
                self.assertIsNone(opened["result"].get("batch"))
                self.assertEqual(opened["result"]["journal"]["total_records"], 1)
                pending = opened["agent_turn"]["active_work"]["pending_plans"]
                self.assertEqual([row["idempotency_key"] for row in pending], [review["idempotency_key"]])
                self.assertEqual(pending[0]["plan_digest"], review["plan_digest"])
                self.assertEqual(peer.calls, calls_before)
                denied = client.call("commit", **{**review, "session_id": session})
                self.refused(denied)
                self.assertEqual(peer.calls, calls_before)
                recovered = client.call("wait", session_id=session,
                                        idempotency_key=review["idempotency_key"],
                                        plan_digest=review["plan_digest"])
                self.effect(recovered, peer, review["idempotency_key"], "prepared")
                self.assertEqual(peer.calls[len(calls_before):], ["Handshake", "QueryPlacement"])
                retired = client.call("cancel", session_id=session, scope="effect",
                                      idempotency_key=review["idempotency_key"],
                                      plan_digest=review["plan_digest"])
                self.effect(retired, peer, review["idempotency_key"], "cancelled")
                self.assertEqual(peer.calls.count("PreparePlacement"), 1)
                self.assertEqual(peer.calls.count("CancelPlacement"), 1)
                self.assertNotIn("CommitPlacement", peer.calls)
                self.assertFalse(peer.commits)
                self.assertFalse((self.directory / "batch").exists())

    def test_parent_deleted_after_native_commit_retains_original_identity_for_restart_query(self):
        plan = exact_plan()
        with NativePeer(plan) as peer:
            with Client(environment(self.directory, peer)) as client:
                session, _opened = self.open(client, plan)
                review = self.prepare(client, session, plan, peer)

                def lose_parent_after_commit(operation, _response):
                    if operation == "CommitPlacement":
                        self.assertEqual(peer.records[review["idempotency_key"]].phase, "placed")
                        (self.directory / "batch").unlink()

                peer.before_reply = lose_parent_after_commit
                lost = client.call("commit", **review)
                self.refused(lost)
                uncertain = self.batch(lost, plan, verified=False)
                self.assertEqual(uncertain["pending_step"], plan.ordered[0].name)
                self.assertEqual(uncertain["steps"][0]["state"], "dispatch_started")
                self.assertEqual(uncertain["steps"][0]["native_plan_digest"], review["plan_digest"])
                pending = lost["agent_turn"]["active_work"]["pending_plans"]
                self.assertEqual([row["idempotency_key"] for row in pending], [review["idempotency_key"]])
                self.assertTrue(pending[0]["dispatch_started"])
                self.assertEqual(peer.commits, [review["idempotency_key"]])
            peer.before_reply = None
            calls_before = list(peer.calls)
            with Client(environment(self.directory, peer)) as client:
                self.refused(client.call("open_session", max_wall_millis=60000))
                self.assertEqual(peer.calls, calls_before)

            recovery_env = environment(self.directory, peer, mode="recover")
            del recovery_env["DFMCP_BUILD_BATCH"]
            del recovery_env["DFMCP_BUILD_ALLOW_PLACE"]
            with Client(recovery_env) as client:
                session, opened = self.open(client)
                pending = opened["agent_turn"]["active_work"]["pending_plans"]
                self.assertEqual([row["idempotency_key"] for row in pending], [review["idempotency_key"]])
                self.assertEqual(pending[0]["plan_digest"], review["plan_digest"])
                self.assertTrue(pending[0]["dispatch_started"])
                self.assertEqual(peer.calls, calls_before)
                recovered = client.call("wait", session_id=session,
                                        idempotency_key=review["idempotency_key"],
                                        plan_digest=review["plan_digest"])
                self.effect(recovered, peer, review["idempotency_key"], "placed")
                self.assertEqual(peer.calls[len(calls_before):], ["Handshake", "QueryPlacement"])
                self.assertTrue(recovered["agent_turn"]["active_work"]["pending_absence_proven"])
                receipt_bytes = (self.directory / "journal").read_bytes()
                reconciled_calls = list(peer.calls)
                replay = client.call("wait", session_id=session,
                                     idempotency_key=review["idempotency_key"],
                                     plan_digest=review["plan_digest"])
                self.effect(replay, peer, review["idempotency_key"], "placed")
                self.assertEqual(peer.calls, reconciled_calls)
                self.assertEqual((self.directory / "journal").read_bytes(), receipt_bytes)
                self.assertEqual(peer.calls.count("PreparePlacement"), 1)
                self.assertEqual(peer.calls.count("CommitPlacement"), 1)
                self.assertEqual(peer.commits, [review["idempotency_key"]])
                self.assertFalse((self.directory / "batch").exists())

    def test_retained_plan_mismatch_and_malformed_plans_refuse_before_native_calls(self):
        plan = exact_plan()
        base = plan.json()
        bad = []

        def changed(label, alter):
            value = deepcopy(base)
            alter(value)
            bad.append((label, json.dumps(value)))

        changed("empty steps", lambda value: value.update(steps=[]))
        changed("wrong schema", lambda value: value.update(schema="dfmcp.furniture-plan/2"))
        changed("unknown envelope field", lambda value: value.update(path="/private/alternate"))
        changed("unknown step field", lambda value: value["steps"][0].update(replace_item=True))
        changed("unsupported kind", lambda value: value["steps"][0].update(kind="door"))
        changed("boolean item", lambda value: value["steps"][0].update(item=True))
        changed("floating item", lambda value: value["steps"][0].update(item=42.0))
        changed("duplicate item", lambda value: value["steps"][1].update(item=value["steps"][0]["item"]))
        changed("duplicate target", lambda value: value["steps"][1].update(target=value["steps"][0]["target"]))
        changed("duplicate name", lambda value: value["steps"][1].update(name=value["steps"][0]["name"]))
        changed("missing dependency", lambda value: value["steps"][0].update(after=["missing"]))
        changed("cyclic dependency", lambda value: value["steps"][0].update(after=[value["steps"][2]["name"]]))
        changed("null dependencies", lambda value: value["steps"][0].update(after=None))
        changed("missing 3x3 halo", lambda value: value["steps"][0].update(target=[0, 10, 0]))
        changed("floating coordinate", lambda value: value["steps"][0].update(target=[10, 10.0, 0]))
        changed("negative coordinate", lambda value: value["steps"][0].update(target=[10, 10, -1]))
        raw = model.canonical(base).decode("ascii")
        bad.append(("duplicate JSON field", raw.replace('"schema":', '"schema":"ignored","schema":', 1)))
        bad.append(("byte bound", raw + " " * model.MAX_BYTES))
        bad.append(("too many steps", json.dumps({"schema": model.SCHEMA, "steps": [
            {"name": f"room{i:02}", "kind": "bed", "item": i, "target": [10 + i, 10, 0]}
            for i in range(33)
        ]})))
        with NativePeer(plan) as peer:
            no_batch = environment(self.directory, peer)
            del no_batch["DFMCP_BUILD_BATCH"]
            with Client(no_batch) as client:
                denied = client.call("open_session", furniture_plan=model.canonical(base).decode("ascii"),
                                     max_wall_millis=60000)
                self.refused(denied)
                self.assertEqual(peer.calls, [])
                self.assertFalse((self.directory / "journal").exists())
            with Client(environment(self.directory, peer)) as client:
                client.tools()
                for label, raw in bad:
                    with self.subTest(invalid=label):
                        denied = client.call("open_session", furniture_plan=raw, max_wall_millis=60000)
                        self.refused(denied)
                        self.assertEqual(peer.calls, [])
                        self.assertFalse((self.directory / "batch").exists())
                        self.assertFalse((self.directory / "journal").exists())
                session, opened = self.open(client, plan)
                batch_id = self.batch(opened, plan)["batch_id"]
                self.assertEqual(self.inventory(client, session, plan, peer)["placed"], 0)
            original = {name: (self.directory / name).read_bytes() for name in ("batch", "journal")}
            native_before = list(peer.calls)
            changed_plan = model.FurniturePlan((replace(plan.steps[0], item=999), *plan.steps[1:]))
            with Client(environment(self.directory, peer)) as client:
                denied = client.call("open_session", furniture_plan=model.canonical(changed_plan.json()).decode("ascii"),
                                     max_wall_millis=60000)
                self.refused(denied)
                self.assertEqual(peer.calls, native_before)
                self.assertEqual({name: (self.directory / name).read_bytes() for name in original}, original)
                session, opened = self.open(client)
                self.assertEqual(self.batch(opened, plan)["batch_id"], batch_id)
                self.assertEqual(peer.calls, native_before)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    args, remaining = parser.parse_known_args()
    executable = args.binary.resolve(strict=True)
    if not executable.is_file() or not os.access(executable, os.X_OK):
        parser.error("--binary must name an executable regular file")
    stdio.EXECUTABLE = executable
    program = unittest.main(argv=[__file__, *remaining], exit=False)
    print("MAXIMUM_MCP_RESPONSE_BYTES", MAXIMUM_OBSERVED_PACKET_BYTES)
    raise SystemExit(0 if program.result.wasSuccessful() else 1)


if __name__ == "__main__":
    main()
