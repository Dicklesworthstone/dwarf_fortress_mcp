"""Retained requests through real TCP clients and POSIX batch custody.

The joined peer is an explicit protocol test double, not a DFHack plugin.
Beads: df-dfhack-bridge-plane-c-pic.4 and df-dfhack-bridge-plane-c-pic.5.
"""
from __future__ import annotations

from contextlib import contextmanager
from dataclasses import replace
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
import furniture_batch as batch
import build_placement_rpc as rpc
import build_placement_store as storage
from build_placement_wire import Capture, Insertion, Item, Plan, Record, Tile
from furniture_allocation import Candidate, Request, Slot
from furniture_handoff import Handoff, InventorySource, Selected
from furniture_plan import canonical
from test_furniture_handoff import handoff


def encoded_fields(fields):
    def varint(value):
        out = bytearray()
        while value > 127:
            out.append((value & 127) | 128)
            value >>= 7
        out.append(value)
        return bytes(out)
    out = bytearray()
    for key, value in sorted(fields.items()):
        if isinstance(value, bytes):
            out += varint(key * 8 + 2) + varint(len(value)) + value
        else:
            out += varint(key * 8) + varint(value)
    return bytes(out)


def receive(connection, count):
    out = bytearray()
    while len(out) < count:
        part = connection.recv(count - len(out))
        if not part:
            raise EOFError
        out += part
    return bytes(out)


class Peer:
    """Own and join every test connection; retain native records across reconnects."""
    def __init__(self, retained):
        self.listener = socket.socket()
        self.listener.bind(('127.0.0.1', 0))
        self.listener.listen()
        self.listener.settimeout(0.05)
        self.address = self.listener.getsockname()
        self.handoff = replace(retained, source=replace(retained.source,
            address=f'{self.address[0]}:{self.address[1]}'))
        self.manifest = rpc.Manifest(91, retained.source.df_version, retained.source.dfhack_version)
        self.sequence, self.commits, self.reads = 10, 0, 0
        self.next_job, self.next_building = retained.source.horizons[:2]
        self.tick = retained.source.tick
        self.items = {row.candidate.id: row for row in retained.selections}
        self.records, self.events, self.errors = {}, [], []
        self.drop_commit = False
        self.after_prepare = None
        self.terminal_phase = 'placed'
        self.capture_change = lambda value: value
        self.done = threading.Event()
        self.thread = threading.Thread(target=self.run)

    def capture(self, selected):
        row = self.items[selected.item]
        item = row.candidate
        floor = Tile(presence=2, shape=3)
        value = Capture(self.manifest.generation, self.sequence, self.tick,
            self.handoff.request.site, (32768, 32768, 32768), self.next_building,
            self.next_job, self.commits, self.handoff.request.folder, True, True, True,
            selected, (floor,) * 9, Item(presence=2, pos=item.position,
                kind=selected.kind, native_type=row.native_type, subtype=item.subtype,
                material=item.material[0], material_index=item.material[1],
                on_ground=True, ground=floor))
        return self.capture_change(value)

    def run(self):
        try:
            while not self.done.is_set():
                try:
                    conn, _ = self.listener.accept()
                except socket.timeout:
                    continue
                with conn:
                    conn.settimeout(3)
                    try:
                        self.serve(conn)
                    except (EOFError, ConnectionResetError, BrokenPipeError):
                        pass
        except BaseException as error:
            self.errors.append(error)

    def serve(self, conn):
        assert receive(conn, 12) == b'DFHack?\n' + struct.pack('<i', 1)
        conn.sendall(b'DFHack!\n' + struct.pack('<i', 1))
        methods = {}
        while not self.done.is_set():
            method, width = struct.unpack('<h2xi', receive(conn, 8))
            assert 0 <= width <= 2048
            request = rpc.decode(receive(conn, width))
            if method == 0:
                assert request[2] == b'dfmcp.build.v1_19.Request'
                assert request[3] == b'dfmcp.build.v1_19.Reply'
                assert request[4] == b'dfmcp_build_v1_19'
                name = request[1].decode()
                assert name in rpc.METHODS and name not in methods.values()
                number = len(methods) + 2
                methods[number] = name
                response = {1: number}
            else:
                assert request[1] == b't' * 32 and request[3] == 1 and request[4] == 19
                name = methods[method]
                self.events.append(name)
                response = {1: 1, 2: 0, 3: request[2], 4: 1, 5: 19,
                    6: self.manifest.generation, 7: self.manifest.df_version.encode(),
                    8: self.manifest.dfhack_version.encode(), 12: 0, 13: len(self.records)}
                if name in ('ReadPlacement', 'PreparePlacement'):
                    selected = batch.Selection(*(request[n] for n in range(5, 10)))
                if name == 'ReadPlacement':
                    self.reads += 1
                    response[9] = self.capture(selected).raw
                elif name == 'PreparePlacement':
                    plan = Plan(request[10].decode(), self.capture(selected))
                    assert request[11] == plan.before.witness and request[12] == plan.digest
                    old = self.records.get(plan.key)
                    assert old is None or old.plan == plan
                    self.records[plan.key] = old or Record(plan, 'prepared', 'none')
                    response[10], response[11] = self.records[plan.key].raw, int(old is not None)
                    if self.after_prepare is not None:
                        self.after_prepare()
                elif name in ('QueryPlacement', 'CommitPlacement', 'CancelPlacement'):
                    key = request[10].decode()
                    record = self.records.get(key)
                    if record is not None:
                        assert request[12] == record.plan.digest
                        if name in ('CommitPlacement', 'CancelPlacement'):
                            assert request[13] == record.plan.token
                        if name == 'CommitPlacement' and record.phase == 'prepared':
                            before = record.plan.before
                            assert self.capture(before.selection).raw == before.raw
                            self.commits += 1
                            if self.terminal_phase == 'placed':
                                insertion = Insertion(before.next_building, before.next_job,
                                    before.selection.item, before.selection.kind, before.selection.target,
                                    before.item.material, before.item.material_index, 0, 1,
                                    True, True, True, False)
                                record = Record(record.plan, 'placed', 'none', before.expected_after(), insertion)
                                self.sequence += 1
                                self.next_job += 1
                                self.next_building += 1
                            else:
                                record = Record(record.plan, 'indeterminate', 'native_failure')
                            self.records[key] = record
                            if self.drop_commit:
                                self.drop_commit = False
                                return
                        elif name == 'CancelPlacement' and record.phase == 'prepared':
                            record = Record(record.plan, 'cancelled', 'cancelled')
                            self.records[key] = record
                        response[10] = record.raw
                    else:
                        assert name == 'QueryPlacement'
                else:
                    assert name == 'Handshake'
                response[12] = int(any(r.phase == 'indeterminate' for r in self.records.values()))
                response[13] = len(self.records)
            raw = encoded_fields(response)
            conn.sendall(struct.pack('<h2xi', -1, len(raw)) + raw)

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *_):
        self.done.set()
        self.thread.join(timeout=5)
        self.listener.close()
        assert not self.thread.is_alive(), 'test peer failed to drain'
        if self.errors:
            raise self.errors[0]

    def environment(self):
        return {rpc.OPT_IN: '1', rpc.TOKEN: 't' * 32, rpc.ENDPOINT: self.handoff.source.address,
                rpc.PLACE: '1'}


@contextmanager
def setup(retained=None):
    with tempfile.TemporaryDirectory() as directory, Peer(retained or handoff()) as peer:
        os.chmod(directory, 0o700)
        with patch.dict(os.environ, peer.environment(), clear=True):
            yield directory, peer


def initialize(directory, peer):
    h = peer.handoff
    return batch.initialize(directory, h.plan(), h.request.folder, h.request.site, handoff=h)


def advance(directory, identity):
    review = batch.review(directory, identity)
    return batch.advance(directory, identity, review['expected_plan'], review['confirm_review'])


class HandoffBatchTests(unittest.TestCase):
    def test_original_request_is_durable_and_reopen_is_offline(self):
        with setup() as (directory, peer):
            result = initialize(directory, peer)
            self.assertEqual(peer.commits, 0)
            self.assertEqual(peer.reads, 1)
            self.assertEqual(result['allocation']['handoff_digest'], peer.handoff.digest)
            with patch.dict(os.environ, {}, clear=True):
                reopened = batch.inspect(directory, result['batch_id'], allocation=True)
            self.assertEqual(reopened['allocation'], peer.handoff.json())
            self.assertEqual(reopened['plan'], peer.handoff.plan().json())
            self.assertEqual(peer.reads, 1)
            self.assertEqual(set(os.listdir(directory)), {'batch.json', 'steps.jsonl', 'effects'})
            self.assertFalse(reopened['native_contacted'])
            self.assertFalse(reopened['retry_permitted'])

    def test_whole_scarcity_plan_executes_in_dependency_order(self):
        with setup() as (directory, peer):
            result = initialize(directory, peer)
            identity = result['batch_id']
            self.assertEqual(result['next_step'], 'z-special')
            first = advance(directory, identity)
            self.assertEqual(first['next_step'], 'a-any')
            final = advance(directory, identity)
            self.assertEqual(final['status'], 'all_placed')
            self.assertEqual(peer.commits, 2)
            self.assertEqual([r.plan.before.selection.item for r in peer.records.values()], [41, 42])
            self.assertEqual(final['allocation']['handoff_digest'], peer.handoff.digest)
            self.assertFalse(final['construction_completion_proven'])
            with self.assertRaises(ValueError):
                advance(directory, identity)
            self.assertEqual(peer.commits, 2)

    def test_legacy_definitions_keep_the_original_shape_and_execution(self):
        with setup() as (directory, peer):
            h = peer.handoff
            result = batch.initialize(directory, h.plan(), h.request.folder, h.request.site)
            stored = batch.unseal(Path(directory, 'batch.json').read_bytes())
            self.assertEqual(stored['schema'], batch.SCHEMA)
            self.assertNotIn('handoff', stored)
            self.assertNotIn('allocation', result)
            self.assertEqual(advance(directory, result['batch_id'])['placed'], 1)
            with self.assertRaises(ValueError):
                batch.inspect(directory, result['batch_id'], allocation=True)

    def test_software_fortress_and_clock_drift_prevent_new_custody(self):
        for field, value in (('tick', 49999), ('next_job', 99), ('next_building', 199),
                             ('site', 3), ('folder', 'other')):
            with self.subTest(field=field), setup() as (directory, peer):
                peer.capture_change = lambda c: replace(c, **{field: value})
                with self.assertRaises(ValueError):
                    initialize(directory, peer)
                self.assertEqual(os.listdir(directory), [])
                self.assertEqual(peer.commits, 0)
        with setup() as (directory, peer):
            peer.manifest = replace(peer.manifest, dfhack_version='other')
            with self.assertRaises(ValueError):
                initialize(directory, peer)
            self.assertEqual(os.listdir(directory), [])

    def test_conflicting_explicit_selection_refuses_before_contact(self):
        with setup() as (directory, peer):
            h = peer.handoff
            with self.assertRaises(ValueError):
                batch.initialize(directory, h.plan(), 'other', h.request.site, handoff=h)
            with self.assertRaises(ValueError):
                batch.initialize(directory, replace(h.plan(), steps=h.plan().steps[:1]),
                                 h.request.folder, h.request.site, handoff=h)
            self.assertEqual(peer.events, [])
            self.assertEqual(os.listdir(directory), [])

    def test_native_item_drift_blocks_review_without_an_intent(self):
        changes = ({'material': 420}, {'material_index': 1}, {'subtype': 0},
                   {'native_type': 42}, {'kind': 2}, {'pos': (1, 1, 2)}, {'pos': (15, 15, 3)})
        for change in changes:
            with self.subTest(change=change), setup() as (directory, peer):
                result = initialize(directory, peer)
                peer.capture_change = lambda c: replace(c, item=replace(c.item, **change))
                with self.assertRaises(ValueError):
                    batch.review(directory, result['batch_id'])
                self.assertEqual(os.listdir(Path(directory, 'effects')), [])
                self.assertEqual(peer.commits, 0)

    def test_unconstrained_material_is_still_bound_to_original_selected_item(self):
        with setup() as (directory, peer):
            result = initialize(directory, peer)
            advance(directory, result['batch_id'])
            peer.capture_change = lambda c: replace(c, item=replace(c.item, material=999))
            with self.assertRaises(ValueError):
                batch.review(directory, result['batch_id'])
            self.assertEqual(peer.commits, 1)

    def test_movement_inside_original_distance_is_eligible(self):
        with setup() as (directory, peer):
            result = initialize(directory, peer)
            peer.capture_change = lambda c: replace(c, item=replace(c.item, pos=c.selection.target))
            self.assertEqual(advance(directory, result['batch_id'])['placed'], 1)

    def test_changed_item_after_review_cannot_be_reconfirmed_around_constraints(self):
        with setup() as (directory, peer):
            result = initialize(directory, peer)
            review = batch.review(directory, result['batch_id'])
            peer.capture_change = lambda c: replace(c, item=replace(c.item, material=999))
            with self.assertRaises(ValueError):
                batch.advance(directory, result['batch_id'], review['expected_plan'], review['confirm_review'])
            # Even a freshly calculated valid native digest/review cannot bypass original material.
            with batch.Batch(directory, rpc.Budget(10000)) as owner:
                step, inventory = batch.select_next(owner, result['batch_id'])
                plan = Plan(owner.key(step), peer.capture(batch.selection(step)))
                confirmation = batch.review_seal(owner, step, plan, inventory['inventory_digest'])
            with self.assertRaises(ValueError):
                batch.advance(directory, result['batch_id'], plan.digest.hex(), confirmation)
            self.assertEqual(peer.commits, 0)
            self.assertEqual(os.listdir(Path(directory, 'effects')), [])

    def test_last_dispatch_boundary_rechecks_retained_constraints(self):
        with setup() as (directory, peer):
            result = initialize(directory, peer)
            review = batch.review(directory, result['batch_id'])
            original = batch.validate_handoff_capture
            def guard(*args):
                original(*args)
                if any(b'"kind":"dispatch"' in p.read_bytes() for p in Path(directory, 'effects').iterdir()):
                    raise ValueError('injected last-boundary request invalidation')
            with patch.object(batch, 'validate_handoff_capture', guard), self.assertRaises(ValueError):
                batch.advance(directory, result['batch_id'], review['expected_plan'], review['confirm_review'])
            self.assertEqual(peer.commits, 0)
            self.assertIn('PreparePlacement', peer.events)
            self.assertNotIn('CommitPlacement', peer.events)
            stopped = batch.recover(directory, result['batch_id'], 'z-special', cancel=True)
            self.assertEqual(stopped['status'], 'halted_cancelled')

    def test_lost_commit_reply_recovers_original_key_without_reallocation(self):
        with setup() as (directory, peer):
            result = initialize(directory, peer)
            peer.drop_commit = True
            with self.assertRaises(ValueError):
                advance(directory, result['batch_id'])
            pending = batch.inspect(directory, result['batch_id'])
            self.assertEqual(pending['pending_step'], 'z-special')
            self.assertEqual(peer.commits, 1)
            with self.assertRaises(ValueError):
                advance(directory, result['batch_id'])
            recovered = batch.recover(directory, result['batch_id'], 'z-special')
            self.assertEqual(recovered['next_step'], 'a-any')
            self.assertEqual(recovered['allocation']['handoff_digest'], peer.handoff.digest)
            self.assertEqual(peer.commits, 1)
            final = advance(directory, result['batch_id'])
            self.assertEqual(final['status'], 'all_placed')
            self.assertEqual(peer.commits, 2)

    def test_indeterminate_record_remains_pending_and_blocks_other_items(self):
        with setup() as (directory, peer):
            result = initialize(directory, peer)
            peer.terminal_phase = 'indeterminate'
            outcome = advance(directory, result['batch_id'])
            self.assertEqual(outcome['status'], 'pending_recovery')
            with self.assertRaises(ValueError):
                advance(directory, result['batch_id'])
            before = len(peer.events)
            with patch.dict(os.environ, {}, clear=True):
                recovered = batch.recover(directory, result['batch_id'], 'z-special')
            self.assertEqual(recovered['status'], 'pending_recovery')
            self.assertEqual(len(peer.events), before)
            self.assertEqual(peer.commits, 1)

    def test_revocation_after_prepare_leaves_query_only_cancellation_available(self):
        with setup() as (directory, peer):
            result = initialize(directory, peer)
            peer.after_prepare = lambda: os.environ.__setitem__(rpc.PLACE, '0')
            with self.assertRaises(ValueError):
                advance(directory, result['batch_id'])
            self.assertEqual(peer.commits, 0)
            outcome = batch.recover(directory, result['batch_id'], 'z-special', cancel=True)
            self.assertEqual(outcome['status'], 'halted_cancelled')
            self.assertFalse(outcome['retry_permitted'])

    def test_original_handoff_custody_loss_after_prepare_prevents_dispatch(self):
        with setup() as (directory, peer):
            result = initialize(directory, peer)
            peer.after_prepare = lambda: Path(directory, 'batch.json').unlink()
            with self.assertRaisesRegex(ValueError, 'batch membership changed'):
                advance(directory, result['batch_id'])
            self.assertEqual(peer.commits, 0)
            # No replacement batch can hide the original child intent.
            with self.assertRaises(ValueError):
                initialize(directory, peer)
            self.assertEqual(len(os.listdir(Path(directory, 'effects'))), 1)

    def test_replay_rejects_an_independently_valid_but_wrong_original_child(self):
        with setup() as (directory, peer):
            result = initialize(directory, peer)
            with batch.Batch(directory, rpc.Budget(10000), True) as owner:
                step = owner.plan.ordered[0]
                before = peer.capture(batch.selection(step))
                wrong = Plan(owner.key(step), replace(before, item=replace(before.item, material=777)))
                owner.effects.create(wrong, peer.manifest, peer.address)
            with self.assertRaises(ValueError):
                batch.inspect(directory, result['batch_id'])
            self.assertEqual(peer.commits, 0)

    def test_v2_cannot_drop_its_handoff_or_switch_its_plan(self):
        for change in ('missing', 'legacy', 'plan'):
            with self.subTest(change=change), setup() as (directory, peer):
                result = initialize(directory, peer)
                path = Path(directory, 'batch.json')
                value = batch.unseal(path.read_bytes())
                if change == 'missing':
                    del value['handoff']
                elif change == 'legacy':
                    value['schema'] = batch.SCHEMA
                else:
                    value['plan']['steps'][0]['item'] = 900
                path.write_bytes(batch.seal(value))
                with self.assertRaises(ValueError):
                    batch.inspect(directory, batch.sha(path.read_bytes()))

    def test_initialization_refusal_before_publication_leaves_empty_directory(self):
        with setup() as (directory, peer):
            with patch.object(batch, 'MAX_OUTPUT', 1024), self.assertRaises(ValueError):
                initialize(directory, peer)
            self.assertEqual(os.listdir(directory), [])
        with setup() as (directory, peer):
            original = batch.encoded
            def revoked(*args, **kwargs):
                result = original(*args, **kwargs)
                os.environ[rpc.OPT_IN] = '0'
                return result
            with patch.object(batch, 'encoded', revoked), self.assertRaises(ValueError):
                initialize(directory, peer)
            self.assertEqual(os.listdir(directory), [])

    def test_existing_custody_is_refused_before_a_new_native_read(self):
        with setup() as (directory, peer):
            result = initialize(directory, peer)
            before = list(peer.events)
            with self.assertRaises(ValueError):
                initialize(directory, peer)
            self.assertEqual(peer.events, before)
            self.assertEqual(batch.inspect(directory, result['batch_id'])['placed'], 0)

    def test_input_symlink_and_oversize_refusal_and_full_offline_cli_inspection(self):
        with setup() as (directory, peer), tempfile.TemporaryDirectory() as inputs:
            source = Path(inputs, 'handoff.json')
            source.write_bytes(canonical(peer.handoff.json()))
            self.assertEqual(batch.read_handoff(str(source)), peer.handoff)
            link = Path(inputs, 'link')
            link.symlink_to(source)
            with self.assertRaises(OSError):
                batch.read_handoff(str(link))
            source.write_bytes(b'x' * (32768 + 1))
            with self.assertRaises(ValueError):
                batch.read_handoff(str(source))
            result = initialize(directory, peer)
            output = subprocess.run([sys.executable, str(Path(batch.__file__)), 'inspect',
                '--directory', directory, '--batch-id', result['batch_id'], '--allocation'],
                capture_output=True, timeout=10, env={})
            self.assertEqual(output.returncode, 0, output.stderr.decode())
            self.assertEqual(json.loads(output.stdout)['result']['allocation'], peer.handoff.json())
            self.assertEqual(peer.reads, 1)

    def test_dense_32_item_plan_executes_and_every_outcome_retains_constraints(self):
        names = tuple(f'{i:02}-' + 'x' * 30 for i in range(32))
        request = Request('r' * 512, 2**31 - 1, tuple(
            Slot(name, 'bed', (i + 2, 15, 2), after=names[max(0, i - 7):i], max_distance=10)
            for i, name in enumerate(names)))
        selections = tuple(Selected(s.name, Candidate(i + 1, 'bed', s.target, (419, -1), -1), 0)
                           for i, s in enumerate(request.slots))
        h = Handoff(request, replace(handoff().source, df_version='v' * 128, dfhack_version='h' * 128), selections)
        with setup(h) as (directory, peer):
            result = initialize(directory, peer)
            largest = len(batch.encoded('init', result))
            for i in range(32):
                result = advance(directory, result['batch_id'])
                largest = max(largest, len(batch.encoded('advance', result)))
                self.assertEqual(result['placed'], i + 1)
                self.assertEqual(result['allocation']['handoff_digest'], peer.handoff.digest)
            self.assertEqual(result['status'], 'all_placed')
            self.assertEqual(peer.commits, 32)
            full = batch.inspect(directory, result['batch_id'], allocation=True)
            largest = max(largest, len(batch.encoded('inspect', full)))
            self.assertLessEqual(largest, 65536)
            print('32-item largest complete response:', largest)


if __name__ == '__main__':
    unittest.main()
