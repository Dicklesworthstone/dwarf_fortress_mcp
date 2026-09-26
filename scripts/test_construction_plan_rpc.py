"""Real joined TCP tests for one shared, receipt-bracketed whole-plan capture.

The peer checks actual fixed-profile request bytes and produces independently
framed native-layout test observations. This is not real DFHack qualification.
"""
from __future__ import annotations

import hashlib
import os
import socket
import struct
import time
import unittest
from unittest.mock import patch

import construction_monitor_rpc as single
import construction_plan_rpc as rpc
from build_placement_wire import Rejected, Record
from construction_plan import Goal, LinkedSample
from test_construction_plan import TICK, make_goal, operations_for
from test_construction_receipt import item, operations
from test_construction_monitor_rpc import (
    BUILD_SECRET, OPS_SECRET, NativePeer as SinglePeer, read_message, wire_message,
)


class NativePeer(SinglePeer):
    """One connection per capture; every receipt request must follow set order."""

    def __init__(self, receipts, captures=None, *, hook=None, lose=None, notifications=0, delay=0):
        self.receipts = tuple(receipts)
        self.records = tuple(Record.decode(raw) for raw in self.receipts)
        self.receipt_count = len(self.receipts)
        self.query_keys = []
        if captures is None:
            captures = [operations_for(Goal(self.receipts, TICK + 100))]
        super().__init__(captures, hook=hook, lose=lose, notifications=notifications, delay=delay)
        # Maximum-sized CLI goals validate and synchronize their complete
        # receipt set before connecting; the peer's accept timer is unrelated
        # to the client's shrinking operation deadline.
        self.listener.settimeout(10)

    def __exit__(self, error_type, *_args):
        self.thread.join(12)
        if self.thread.is_alive():
            self.listener.close()
            self.thread.join(3)
            raise AssertionError('whole-plan native peer did not quiesce')
        if self.error is not None and error_type is None:
            raise AssertionError('whole-plan native peer failed') from self.error

    def serve(self):
        try:
            for capture in self.captures:
                with self.listener.accept()[0] as sock:
                    self.connections += 1
                    sock.settimeout(2)
                    sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
                    methods, queried, captured, released = {}, 0, False, False
                    try:
                        assert self.exact(sock, 12) == b'DFHack?\n\x01\0\0\0'
                        if self.delay:
                            time.sleep(self.delay)
                        sock.sendall(b'DFHack!\n\x01\0\0\0')
                        while True:
                            method, size = struct.unpack('<h2xi', self.exact(sock, 8))
                            assert 0 <= size <= 2048
                            fields = read_message(self.exact(sock, size))
                            member = None
                            if method == 0:
                                assert set(fields) == {1, 2, 3, 4}
                                plugin = fields[4].decode()
                                family = 'build' if plugin == 'dfmcp_build_v1_19' else 'operations'
                                assert plugin == single.PROFILES[family][0]
                                name = fields[1].decode()
                                assert (family, name) in single.BINDINGS, 'effectful or unknown method bound'
                                assert fields[2] == (single.PROFILES[family][1] + '.Request').encode()
                                assert fields[3] == (single.PROFILES[family][1] + '.Reply').encode()
                                identity = len(methods) + 2
                                methods[identity] = family, name
                                self.bindings.append((family, name))
                                reply, tag = {1: identity}, 'bind'
                            else:
                                family, name = methods[method]
                                self.calls.append((family, name))
                                assert fields[1] == (BUILD_SECRET if family == 'build' else OPS_SECRET)
                                assert len(fields[2]) == 32 and fields[3] == 1 and fields[4] == single.PROFILES[family][2]
                                reply = {1: 1, 2: 0, 3: fields[2], 4: 1, 5: fields[4],
                                         6: 41 if family == 'build' else 987,
                                         7: b'test-df', 8: b'test-dfhack'}
                                tag = family + '_handshake'
                                if family == 'build':
                                    reply.update({12: 0, 13: self.receipt_count})
                                    if name == 'Handshake':
                                        assert set(fields) == {1, 2, 3, 4}
                                    else:
                                        assert name == 'QueryPlacement' and set(fields) == {1, 2, 3, 4, 10, 12}
                                        assert queried < 2 * self.receipt_count
                                        member = queried % self.receipt_count
                                        record = self.records[member]
                                        assert fields[10] == record.plan.key.encode('ascii')
                                        assert fields[12] == record.plan.digest
                                        tag = 'before_receipt' if queried < self.receipt_count else 'after_receipt'
                                        assert not captured if tag == 'before_receipt' else released
                                        queried += 1
                                        self.queries += 1
                                        self.query_keys.append((tag, record.plan.key))
                                        reply[10] = self.receipts[member]
                                else:
                                    assert tuple(fields[n] for n in (5, 6, 7, 8, 11)) == (4096, 4096, 65536, single.MAX_CAPTURE, single.PAGE)
                                    if name == 'Handshake':
                                        assert set(fields) == set(range(1, 9)) | {11}
                                    else:
                                        assert set(fields) == set(range(1, 13))
                                        assert name == 'ReadObservation' and queried == self.receipt_count
                                        token, offset, release = fields[9], fields[10], fields[12]
                                        if release:
                                            assert release == 1 and token == b't' * 16 and offset == 0 and captured and not released
                                            released = True
                                            self.releases += 1
                                            reply[10] = token
                                            tag = 'release'
                                        else:
                                            assert not released
                                            tag = 'page'
                                            self.reads += 1
                                            if not token:
                                                assert offset == 0 and not captured
                                                captured = True
                                            else:
                                                assert token == b't' * 16 and captured and offset > 0
                                            part = capture[offset:offset + single.PAGE]
                                            reply.update({9: part, 10: b't' * 16, 11: offset,
                                                          12: len(capture), 13: hashlib.sha256(capture).digest(),
                                                          14: int(offset + len(part) == len(capture))})
                            if self.hook:
                                reply = self.hook(tag, fields, reply, self)
                            if tag == self.lose or (tag, member) == self.lose:
                                sock.sendall(struct.pack('<h2xi', -1, 1000) + b'\x08')
                                break
                            for _ in range(self.notifications):
                                sock.sendall(struct.pack('<h2xi', -3, 0))
                            body = wire_message(reply) if isinstance(reply, dict) else reply
                            framed = struct.pack('<h2xi', -1, len(body)) + body
                            for offset in range(0, len(framed), 503):
                                sock.sendall(framed[offset:offset + 503])
                    except (EOFError, BrokenPipeError, ConnectionResetError):
                        pass
        except BaseException as error:
            self.error = error
        finally:
            self.listener.close()


def fetch(goal, timeout=5000, budget=None):
    return rpc.acquire(rpc.Authority.load(), goal, rpc.Budget(timeout) if budget is None else budget)


class PlanTransportTests(unittest.TestCase):
    def test_all_originals_bracket_one_capture_on_one_connection(self):
        goal = make_goal(3)
        with NativePeer(goal.receipts) as peer, peer.environment():
            result = fetch(goal)
            self.assertEqual(result.before_records, goal.receipts)
            self.assertEqual(result.after_records, goal.receipts)
            self.assertEqual(result.capture, operations_for(goal))
            self.assertEqual((peer.connections, peer.reads, peer.releases, peer.queries), (1, 1, 1, 6))
            self.assertEqual(peer.bindings, list(single.BINDINGS))
            self.assertEqual(peer.query_keys, [(tag, child.record.plan.key)
                for tag in ('before_receipt', 'after_receipt') for child in goal.goals])
            self.assertEqual(peer.calls, [('build', 'Handshake'), ('operations', 'Handshake')]
                + [('build', 'QueryPlacement')] * 3
                + [('operations', 'ReadObservation')] * 2
                + [('build', 'QueryPlacement')] * 3)
            self.assertEqual(LinkedSample.decode(result.encode()), result)

    def test_single_target_uses_same_native_reads_with_separate_set_profile(self):
        goal = make_goal(1)
        with NativePeer(goal.receipts) as peer, peer.environment():
            result = fetch(goal)
            self.assertEqual(len(result.before_records), 1)
            self.assertEqual((peer.connections, peer.reads, peer.releases, peer.queries), (1, 1, 1, 2))

    def test_32_receipts_with_fragmented_multi_page_capture(self):
        goal = make_goal(32)
        observed = operations_for(goal)
        # Add a large complete unselected item roster, keeping every target from
        # the native-layout helper unchanged and retained exactly once.
        decoded = single.LinkedSample(single.Manifest(41, 'test-df', 'test-dfhack'), goal.receipts[0],
            single.Manifest(987, 'test-df', 'test-dfhack'), observed,
            single.Manifest(41, 'test-df', 'test-dfhack'), goal.receipts[0]).validate(goal.goals[0], lambda: None)
        from test_construction_receipt import building
        buildings = tuple(building(b.id, b.kind, b.stage, b.max_stage, b.bounds, b.native_type)
                          for b in decoded.buildings.values())
        items = tuple(item(i.id, i.kind, i.native_type, i.subtype, i.material, i.material_index,
                           i.stack, i.flags, i.holder, i.container) for i in decoded.items.values())
        last = max(decoded.items)
        extra = tuple(item(n, holder=None) for n in range(last + 1, last + 2001))
        raw = operations(buildings=buildings, items=items + extra,
                         horizons=(decoded.horizons[0], decoded.horizons[1], last + 2001))
        self.assertGreater(len(raw), single.PAGE)
        with NativePeer(goal.receipts, [raw]) as peer, peer.environment():
            budget = rpc.Budget(10000)
            result = fetch(goal, budget=budget)
            self.assertEqual(result.capture, raw)
            self.assertEqual((peer.connections, peer.queries, peer.releases), (1, 64, 1))
            self.assertGreater(peer.reads, 1)
            expected_calls = 4 + 2 + 64 + peer.reads + 1
            self.assertEqual(budget.calls, rpc.MAX_RPC_CALLS - expected_calls)
            self.assertEqual(len(result.validate(goal, lambda: None).items), 2032)

    def test_each_missing_or_substituted_original_fails_complete_bracket(self):
        goal = make_goal(3)
        for boundary in ('before_receipt', 'after_receipt'):
            for index in range(3):
                for change in ('missing', 'another_receipt', 'modified'):
                    def hook(tag, fields, reply, peer):
                        if tag == boundary and fields[10] == goal.goals[index].record.plan.key.encode():
                            if change == 'missing':
                                del reply[10]
                            elif change == 'another_receipt':
                                reply[10] = goal.receipts[(index + 1) % 3]
                            else:
                                reply[10] = reply[10][:-1] + bytes([reply[10][-1] ^ 1])
                        return reply
                    with self.subTest(boundary=boundary, index=index, change=change), NativePeer(goal.receipts, hook=hook) as peer, peer.environment():
                        with self.assertRaises(Rejected):
                            fetch(goal)
                        self.assertEqual(peer.connections, 1)
                        self.assertEqual(peer.reads, int(boundary == 'after_receipt'))
                        self.assertEqual(peer.queries, index + 1 + (3 if boundary == 'after_receipt' else 0))

    def test_last_member_loss_does_not_publish_subset_or_reconnect(self):
        goal = make_goal(32)
        for boundary in ('before_receipt', 'after_receipt'):
            with self.subTest(boundary=boundary), NativePeer(goal.receipts, lose=(boundary, 31)) as peer, peer.environment():
                with self.assertRaises(Rejected):
                    fetch(goal)
                self.assertEqual(peer.connections, 1)
                self.assertEqual(peer.queries, 32 if boundary == 'before_receipt' else 64)
                self.assertEqual(peer.reads, int(boundary == 'after_receipt'))

    def test_last_of_32_substitution_source_drift_or_retained_count_contradiction(self):
        goal = make_goal(32)
        for boundary in ('before_receipt', 'after_receipt'):
            for changed in ({10: goal.receipts[0]}, {6: 42}, {8: b'other-dfhack'}, {13: 31}):
                def hook(tag, fields, reply, peer):
                    if tag == boundary and fields[10] == goal.goals[-1].record.plan.key.encode():
                        reply.update(changed)
                    return reply
                with self.subTest(boundary=boundary, changed=tuple(changed)), NativePeer(goal.receipts, hook=hook) as peer, peer.environment():
                    with self.assertRaises(Rejected):
                        fetch(goal, timeout=10000)
                    self.assertEqual(peer.connections, 1)
                    self.assertEqual(peer.queries, 32 if boundary == 'before_receipt' else 64)
                    self.assertEqual(peer.reads, int(boundary == 'after_receipt'))

    def test_source_drift_at_any_member_and_software_mismatch_refuse(self):
        goal = make_goal(3)
        for boundary in ('before_receipt', 'after_receipt'):
            for index in range(3):
                def hook(tag, fields, reply, peer):
                    if tag == boundary and fields[10] == goal.goals[index].record.plan.key.encode():
                        reply[6] = 42
                    return reply
                with self.subTest(boundary=boundary, index=index), NativePeer(goal.receipts, hook=hook) as peer, peer.environment():
                    with self.assertRaises(Rejected):
                        fetch(goal)
        with NativePeer(goal.receipts, hook=lambda t, f, r, p: {**r, 8: b'other'}
                if t == 'operations_handshake' else r) as peer, peer.environment():
            with self.assertRaises(Rejected):
                fetch(goal)
            self.assertEqual(peer.queries, 0)

    def test_release_and_capture_digest_must_succeed_before_trailing_queries(self):
        goal = make_goal(3)
        for boundary, changed in (('release', {10: b'z' * 16}), ('page', {13: b'z' * 32}),
                                  ('page', {11: 1}), ('page', {14: 0}), ('page', {10: b'z' * 15})):
            with self.subTest(boundary=boundary, changed=changed), NativePeer(goal.receipts,
                    hook=lambda t, f, r, p: {**r, **changed} if t == boundary else r) as peer, peer.environment():
                with self.assertRaises(Rejected):
                    fetch(goal)
                self.assertEqual(peer.queries, 3)

    def test_page_or_release_loss_preserves_whole_plan_uncertainty(self):
        goal = make_goal(3)
        for boundary in ('page', 'release'):
            with self.subTest(boundary=boundary), NativePeer(goal.receipts, lose=boundary) as peer, peer.environment():
                with self.assertRaises(Rejected):
                    fetch(goal)
                self.assertEqual(peer.connections, 1)
                self.assertEqual(peer.queries, 3)

    def test_shared_call_budget_and_work_budget_never_reset_per_target(self):
        goal = make_goal(3)
        self.assertEqual(rpc.MAX_RPC_CALLS, 327)
        with NativePeer(goal.receipts) as peer, peer.environment():
            budget = rpc.Budget(5000)
            budget.calls = 6 + 2
            with self.assertRaises(Rejected):
                fetch(goal, budget=budget)
            self.assertEqual((peer.queries, peer.reads, budget.calls), (2, 0, 0))
        with NativePeer(goal.receipts) as peer, peer.environment():
            budget = rpc.Budget(5000)
            budget.work_steps = 3  # Whole-plan validation plus two member reads.
            with self.assertRaises(Rejected):
                fetch(goal, budget=budget)
            self.assertEqual((peer.queries, peer.reads, budget.work_steps), (2, 0, 0))

    def test_shared_deadline_and_network_budget_never_reset(self):
        goal = make_goal(3)
        budget = rpc.Budget(5000)
        def expire_during_receipt(tag, fields, reply, peer):
            if tag == 'before_receipt':
                budget.deadline = time.monotonic() - 1
            return reply
        with NativePeer(goal.receipts, hook=expire_during_receipt) as peer, peer.environment():
            with rpc.Client(rpc.Authority.load(), goal, budget) as client:
                # Inject expiry at a real receipt-response boundary after both
                # handshakes; no assertion depends on host speed or sleep timing.
                with self.assertRaises(Rejected):
                    client.capture_once()
                self.assertTrue(client.closed)
            self.assertEqual((peer.connections, peer.queries, peer.reads), (1, 1, 0))
        with NativePeer(goal.receipts) as peer, peer.environment():
            with rpc.Client(rpc.Authority.load(), goal, rpc.Budget(5000)) as client:
                client.budget.network_bytes = 1
                with self.assertRaises(Rejected):
                    client.capture_once()
                self.assertTrue(client.closed)
            self.assertEqual(peer.queries, 0)

    def test_revocation_at_late_receipt_boundary_closes_connection(self):
        goal = make_goal(3)
        def hook(tag, fields, reply, peer):
            if tag == 'after_receipt' and fields[10] == goal.goals[-1].record.plan.key.encode():
                os.environ[single.OPT_IN] = '0'
            return reply
        with NativePeer(goal.receipts, hook=hook) as peer, peer.environment():
            with self.assertRaises(Rejected):
                fetch(goal)
            self.assertEqual((peer.queries, peer.reads, peer.releases), (6, 1, 1))

    def test_complete_decoder_runs_and_connection_has_one_sample_permit(self):
        goal = make_goal(3)
        with NativePeer(goal.receipts, [operations_for(goal) + b'\0']) as peer, peer.environment():
            with self.assertRaises(Rejected):
                fetch(goal)
            self.assertEqual(peer.queries, 6)
        with NativePeer(goal.receipts) as peer, peer.environment():
            with rpc.Client(rpc.Authority.load(), goal, rpc.Budget(5000)) as client:
                client.capture_once()
                with self.assertRaises(Rejected):
                    client.capture_once()
            self.assertEqual((peer.queries, peer.reads), (6, 1))

    def test_invalid_goal_cannot_open_socket_or_enable_effect_environment(self):
        goal = make_goal(3)
        environment = {single.OPT_IN: '1', single.BUILD_TOKEN: BUILD_SECRET.decode(),
                       single.OPERATIONS_TOKEN: OPS_SECRET.decode()}
        with patch.dict(os.environ, environment, clear=True), patch.object(socket, 'socket') as opened:
            with self.assertRaises(Rejected):
                rpc.Client(rpc.Authority.load(), goal.goals[0], rpc.Budget(5000))
            opened.assert_not_called()
            for name in ('DFMCP_BUILD_ALLOW_PLACE', 'DFMCP_ADMITTED_BRIDGE_PROTOCOL', 'DFMCP_ADMISSION_TICKET'):
                with patch.dict(os.environ, {name: '1'}), self.assertRaises(Rejected):
                    rpc.Authority.load()
            opened.assert_not_called()


if __name__ == '__main__':
    unittest.main(verbosity=2)
