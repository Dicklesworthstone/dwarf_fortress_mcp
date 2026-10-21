"""Real private batch/child journals and original placement coordinator.

Native placement I/O and the acquisition/constructed-prefix producer are
injected. The real route checker, suffix replay, room gate, codecs, authority
loader, output checks and complete existing coordinator/store modules execute.
No DFHack/TCP or construction-roster qualification is implied.
"""
from __future__ import annotations

from contextlib import ExitStack
from dataclasses import replace
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from types import ModuleType, SimpleNamespace
import unittest
from unittest.mock import patch

import build_placement_client as placement
import build_placement_rpc as native
import build_placement_store as storage
from build_placement_wire import Capture, Insertion, Item, KINDS, Plan, Record, Tile
from furniture_allocation import Candidate
from furniture_plan import canonical
from room_route_handoff import RoomRouteHandoff, _requirements
import room_route_frontier as room_gate
import route_furniture_batch as runtime
import route_furniture_batch_store as stores
import route_furniture_frontier as routes
from route_furniture_review import candidate
from test_room_route_frontier import fixture, map_bytes, reseal


ENV = {runtime.OPT_IN: '1', runtime.PLACE: '1', native.TOKEN: 'b' * 32,
       runtime.MAP_TOKEN: 'm' * 32, runtime.OPS_TOKEN: 'o' * 32}


def capture(review, room, step=None, *, number=0):
    original, request, source, plan = review.unpack()
    step = plan.ordered[number] if step is None else step
    row = next(row for row in original['selections'] if row['slot'] == step.name)
    selected = candidate(row['candidate'])
    required = _requirements(room.json(), lambda: None)
    x, y, z = step.target
    tiles = tuple(Tile(2, 1, required.get((xx, yy, z), 3))
                  for yy in range(y - 1, y + 2) for xx in range(x - 1, x + 2))
    item = Item(presence=2, pos=selected.position, kind=KINDS.index(selected.kind),
        native_type=row['native_type'], subtype=selected.subtype, material=selected.material[0],
        material_index=selected.material[1], on_ground=True, ground=Tile(2, 1, 3))
    return Capture(11, number, source.tick + number, request.site, source.dimensions,
                   source.horizons[1] + number, source.horizons[0] + number, number,
                   request.folder, True, True, True, runtime.selection(step), tiles, item)


def placed(plan):
    before = plan.before
    insertion = Insertion(before.next_building, before.next_job, before.selection.item,
        before.selection.kind, before.selection.target, before.item.material,
        before.item.material_index, 0, 1, True, True, True, False)
    return Record(plan, 'placed', 'none', before.expected_after(), insertion)


class Game:
    """Injected protocol endpoint; the production coordinator is NOT replaced."""
    def __init__(self, review, room):
        self.review, self.room = review, room
        self.manifest = native.Manifest(11, 'df', 'dfhack')
        self.records, self.calls, self.connections = {}, [], []
        self.number = 0
        self.lose_prepare = self.lose_commit = False
        self.indeterminate = False
        self.batch = None
        self.before_prepare = self.before_commit = lambda plan: None

    def client(self, authority, budget, selection):
        game = self
        class Native:
            def __init__(self):
                self.manifest, self.closed, self.permit = game.manifest, False, None
                game.connections.append(self)
            def observe(self):
                authority.guard('ReadPlacement')
                game.calls.append('observe')
                step = next(s for s in game.review.unpack()[3].steps if runtime.selection(s) == selection)
                before = capture(game.review, game.room, step, number=game.number)
                return native.Reply(self.manifest, False, len(game.records), capture=before)
            def query(self, plan):
                authority.guard('QueryPlacement')
                game.calls.append('query')
                record = game.records.get(plan.key)
                return native.Reply(self.manifest, bool(record and record.phase == 'indeterminate'),
                                    len(game.records), record=record)
            def prepare(self, plan):
                authority.guard('PreparePlacement')
                game.calls.append('prepare')
                if game.batch is not None:
                    assert any(row['key'] == plan.key for row in game.batch.entries)
                game.before_prepare(plan)
                record = Record(plan, 'prepared', 'none')
                game.records[plan.key] = record
                if game.lose_prepare:
                    raise ConnectionError('lost preparation reply')
                self.permit = plan
                return native.Reply(self.manifest, False, len(game.records), record=record, replayed=False)
            def commit(self, plan):
                authority.guard('CommitPlacement')
                assert self.permit == plan and not self.closed
                self.permit = None
                game.calls.append('commit')
                if game.batch is not None:
                    assert game.batch.effects.get(plan.key).state.dispatched
                game.before_commit(plan)
                record = Record(plan, 'indeterminate', 'native_failure') if game.indeterminate else placed(plan)
                game.records[plan.key] = record
                game.number += int(record.phase == 'placed')
                if game.lose_commit:
                    raise ConnectionError('lost commit reply')
                return native.Reply(self.manifest, record.phase == 'indeterminate',
                                    len(game.records), record=record)
            def cancel(self, plan):
                authority.guard('CancelPlacement')
                game.calls.append('cancel')
                record = game.records.get(plan.key, Record(plan, 'cancelled', 'cancelled'))
                return native.Reply(self.manifest, record.phase == 'indeterminate', len(game.records), record=record)
            def close(self):
                self.closed, self.permit = True, None
            def __enter__(self):
                return self
            def __exit__(self, *_):
                self.close()
        return Native()


class Acquisition:
    """Real route/room analysis around an injected predecessor-condition port."""
    def __init__(self, room):
        self.room, self.changes, self.calls = room, {}, 0
        self.construct_status = 'quiescent'
        self.before = lambda owner, before, call: None
        self.old_only = False

    def __call__(self, owner, before, authority, budget):
        self.calls += 1
        self.before(owner, before, self.calls)
        authority.guard()
        prefix = owner.placed_prefix()
        changed = dict(self.changes)
        done = {step.item for step in owner.plan.ordered[:len(prefix)]}
        for step in owner.plan.ordered[:len(prefix)]:
            changed.setdefault(step.target, {6: 1})
        raw = map_bytes(self.room, changed, tick=before.tick)
        source = replace(owner.source, tick=before.tick, map_sha256=hashlib.sha256(raw).hexdigest(),
            horizons=(before.next_job, before.next_building, owner.source.horizons[2]))
        rows = owner.review.unpack()[0]['selections']
        candidates = tuple(candidate(row['candidate']) for row in rows if row['candidate']['item'] not in done)
        types = {row['candidate']['item']: row['native_type'] for row in rows
                 if row['candidate']['item'] not in done}
        budget.remaining()
        allowance = runtime.ANALYSIS_RESERVATION
        assert budget.work_steps > allowance
        budget.work_steps -= allowance
        def guard():
            budget.remaining()
            authority.guard()
        report = routes.assess(owner.review, owner.id, prefix, before, candidates, types,
                               raw, source, guard, maximum_work=allowance)
        # Explicit injected construction producer: these tests do not assert
        # current installed-item/roster correctness. Real receipts and prefix
        # identity/order are still checked by the actual owner/route checker.
        report.update(policy=room_gate.INPUT_POLICY, prefix_matches_route_frontier=True,
            construction_prefix={'status': self.construct_status if prefix else 'no_predecessors',
                                 'predecessor_gate_met': not prefix or self.construct_status == 'quiescent'})
        if report['status'] == 'ready' and prefix and self.construct_status != 'quiescent':
            report['status'] = 'construction_' + self.construct_status
        report = reseal(report)
        return report if self.old_only else room_gate.gate_frontier(owner.room_handoff, report, raw, guard)


class ReadPorts:
    """Injected acquisition/operations/construction ports for the default shell.

    Unlike Acquisition above, tests call production acquire_frontier itself.
    Socket I/O, operations decoding and the construction producer are bounded
    test ports. Actual raw map decoding, route analysis and final room gate run.
    """
    def __init__(self, owner, room, raw, before, *, construction_status='ready'):
        self.clients, self.calls = [], []
        self.owner, self.raw, self.before = owner, raw, before
        self.operations = b'independently-selected-test-operations'
        self.construction_status = construction_status
        self.inventory = ModuleType('route_furniture_inventory')
        self.receipts = ModuleType('construction_receipt')
        self.construction = ModuleType('construction_frontier')
        ports = self
        class ReadAuthority:
            def __init__(self, address, operations_token, map_token):
                self.address = address
                self.operations_token, self.map_token = operations_token, map_token
        class ReadClient:
            def __init__(self, authority, region, budget):
                ports.calls.append('connect')
                self.closed = False
                self.manifests = {name: SimpleNamespace(generation=generation,
                    software=('df', 'dfhack'), df_version='df', dfhack_version='dfhack')
                    for name, generation in (('map', 3), ('operations', 4))}
                ports.clients.append(self)
                authority.guard()
                budget.remaining()
                assert (region.origin, region.size) == (owner.source.origin, owner.source.size)
            def capture_once(self):
                ports.calls.append('bracket')
                self.closed = True
                return self.manifests['map'], ports.raw, self.manifests['operations'], ports.operations
            def __enter__(self):
                return self
            def __exit__(self, *_):
                self.closed = True
        self.inventory.Authority = ReadAuthority
        self.inventory.Region = lambda origin, size: SimpleNamespace(origin=origin, size=size)
        self.inventory.Client = ReadClient
        selected = owner.review.unpack()[0]['selections']
        candidates = tuple(candidate(row['candidate']) for row in selected)
        types = {row['candidate']['item']: SimpleNamespace(native_type=row['native_type']) for row in selected}
        def decode(raw, work):
            work()
            assert raw == ports.operations and ports.clients[0].closed
            ports.calls.append('decode_operations')
            return SimpleNamespace(folder=owner.request.folder, site=owner.request.site,
                tick=before.tick, paused=True, digest=hashlib.sha256(raw).hexdigest(),
                horizons=(before.next_job, before.next_building, owner.source.horizons[2]), items=types)
        def project(observed, request, work):
            assert request == owner.request
            for _ in candidates:
                work()
            ports.calls.append('project')
            return candidates, {}
        def construction(report, prefix, operations, guard, *, maximum_work):
            assert not prefix and operations == ports.operations
            assert maximum_work == runtime.ANALYSIS_RESERVATION
            guard()
            ports.calls.append('construction')
            report.update(policy=room_gate.INPUT_POLICY, prefix_matches_route_frontier=True,
                construction_prefix={'status': 'no_predecessors', 'predecessor_gate_met': True},
                status=ports.construction_status if report['status'] == 'ready' else report['status'])
            return reseal(report)
        self.inventory.candidates_from = project
        self.receipts.decode_operations = decode
        self.construction.gate_frontier = construction

    def installed(self):
        return patch.dict(sys.modules, {'route_furniture_inventory': self.inventory,
            'construction_receipt': self.receipts, 'construction_frontier': self.construction})


class RoomBatchTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.path = self.root/'batch'
        self.path.mkdir(mode=0o700)
        self.handoff, self.room, self.review, self.source, self.raw = fixture()
        self.before = capture(self.review, self.room)
        self.manifest = native.Manifest(11, 'df', 'dfhack')
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.stack.enter_context(patch.dict(os.environ, ENV, clear=True))
        self.authority = runtime.Authority.load(True)

    def create(self, *, legacy=False, handoff=None):
        origin = self.review if legacy else self.handoff if handoff is None else handoff
        return stores.Batch(str(self.path), None, runtime.Budget(60000), writable=True,
                            initial=(origin, self.manifest, self.before))

    def game(self):
        game = Game(self.review, self.room)
        self.stack.enter_context(patch.object(native, 'Client', game.client))
        self.stack.enter_context(patch.object(placement, 'Client', game.client))
        return game

    def acquire(self):
        return Acquisition(self.room)

    def test_complete_parent_is_durable_and_reopen_never_needs_original_input_files(self):
        with self.create() as owner:
            identity = owner.id
            definition = owner.files['batch.json'].raw
            state = owner.audit()
            self.assertEqual(owner.value['schema'], stores.ROOM_SCHEMA)
            self.assertNotIn('review', owner.value)
            self.assertEqual(owner.value['room_handoff_digest'], self.handoff.digest)
            self.assertEqual(state['original_room_plan'], self.room.json())
            self.assertEqual(state['original_review'], self.review.json())
            self.assertFalse(state['room_constraints_checked_this_call'])
            self.assertLess(len(definition), stores.MAX_DEFINITION)
        with stores.Batch(str(self.path), identity, runtime.Budget(60000)) as owner:
            self.assertEqual(owner.room_handoff.raw, self.handoff.raw)
            self.assertEqual(owner.files['batch.json'].raw, definition)
            self.assertEqual(owner.audit(), state)
            self.assertEqual(owner.request.json(), self.room.request().json())

    def test_legacy_definition_and_report_remain_exact_original_shape(self):
        with patch.object(stores.secrets, 'token_hex', return_value='1' * 48):
            with self.create(legacy=True) as owner:
                self.assertIsNone(owner.room_handoff)
                expected = {'schema': stores.SCHEMA, 'nonce': '1' * 48,
                    'review': self.review.json(), 'review_digest': self.review.digest,
                    'root_identity': owner.root_identity, 'effects_identity': list(owner.effects.identity),
                    'native_manifest': self.manifest.view(), 'native_before_hex': self.before.raw.hex()}
                self.assertEqual(owner.files['batch.json'].raw, stores.seal(expected))
                self.assertFalse(any('room' in key for key in owner.audit()))
                identity = owner.id
        with stores.Batch(str(self.path), identity, runtime.Budget(60000)) as owner:
            self.assertIsNone(owner.room_handoff)

    def test_invalid_parent_refuses_before_creating_any_batch_content(self):
        for altered in (RoomRouteHandoff(self.handoff.raw[:-1], self.handoff.digest),
                        RoomRouteHandoff(self.handoff.raw, '0' * 64), object()):
            with self.assertRaises((ValueError, TypeError)):
                self.create(handoff=altered)
            self.assertEqual(list(self.path.iterdir()), [])

    def test_parent_cannot_be_dropped_downgraded_or_replaced_with_a_sidecar(self):
        with self.create() as owner:
            original = owner.files['batch.json'].raw
            value = stores.unseal(original, owner.budget)
            identity = owner.id
        mutations = []
        dropped = dict(value); dropped.pop('room_handoff'); mutations.append(dropped)
        downgraded = dict(value); downgraded['schema'] = stores.SCHEMA; mutations.append(downgraded)
        changed = dict(value); changed['room_handoff_digest'] = '0' * 64; mutations.append(changed)
        for changed in mutations:
            raw = stores.seal(changed)
            (self.path/'batch.json').write_bytes(raw)
            # Even a caller selecting the altered outer digest cannot bypass
            # the new closed schema and independent child identity checks.
            with self.assertRaises(ValueError):
                stores.Batch(str(self.path), stores.sha(raw), runtime.Budget(60000))
        (self.path/'batch.json').write_bytes(original)
        (self.path/'room.json').write_bytes(self.handoff.raw)
        with self.assertRaises(ValueError):
            stores.Batch(str(self.path), identity, runtime.Budget(60000))

    def test_private_lock_modes_and_live_parent_change_are_enforced(self):
        with self.create() as owner:
            with self.assertRaises(BlockingIOError):
                stores.Batch(str(self.path), owner.id, runtime.Budget())
            before = owner.files['batch.json'].raw
            (self.path/'batch.json').write_bytes(before.replace(b'"bedroom_cluster"', b'"changed_cluster"'))
            with self.assertRaises(ValueError):
                owner.audit()
            self.assertTrue(owner.fenced)
        (self.path/'batch.json').write_bytes(before)
        (self.path/'batch.json').chmod(0o644)
        with self.assertRaises(ValueError):
            stores.Batch(str(self.path), stores.sha(before), runtime.Budget())

    def test_room_gate_is_required_even_for_a_valid_furniture_only_ready_callback(self):
        self.game()
        with self.create() as owner:
            acquisition = self.acquire(); acquisition.old_only = True
            with self.assertRaises(ValueError):
                runtime.review_step(owner, self.authority, acquisition)
            self.assertEqual(owner.effects.journals, {})

    def test_missing_wall_prevents_review_confirmation_while_all_routes_pass(self):
        self.game()
        with self.create() as owner:
            acquisition = self.acquire()
            wall = next(p for p, shape in _requirements(self.room.json(), lambda: None).items() if shape == 2)
            acquisition.changes[wall] = {1: 3}
            state = runtime.review_step(owner, self.authority, acquisition)
            self.assertEqual(state['route_frontier']['execution_frontier_status'], 'ready')
            self.assertEqual(state['route_frontier']['status'], 'room_blocked')
            self.assertIsNone(state['expected_plan'])
            self.assertIsNone(state['confirm_review'])
            self.assertFalse(state['advance_allowed'])
            self.assertEqual(owner.effects.journals, {})

    def test_original_coordinator_places_once_after_two_room_gates_and_syncs_parent_link(self):
        game = self.game()
        with self.create() as owner:
            game.batch = owner
            acquisition = self.acquire()
            review = runtime.review_step(owner, self.authority, acquisition)
            state = runtime.advance(owner, review['expected_plan'], review['confirm_review'],
                                    self.authority, acquisition)
            self.assertEqual(game.calls.count('commit'), 1)
            self.assertEqual(game.calls.count('prepare'), 1)
            self.assertEqual(acquisition.calls, 3)  # review + before intent + before commit
            self.assertEqual(state['placed'], 1)
            self.assertTrue(state['room_constraints_checked_this_call'])
            self.assertEqual(state['room_handoff_digest'], self.handoff.digest)
            self.assertEqual(owner.entries[0]['route_evidence_digest'], state['route_frontier']['evidence_digest'])
            identity = owner.id
        self.assertTrue(all(client.closed for client in game.connections))
        with stores.Batch(str(self.path), identity, runtime.Budget(60000), writable=True) as owner:
            self.assertEqual(owner.audit()['placed'], 1)
            self.assertFalse(owner.audit()['room_constraints_checked_this_call'])
            self.assertEqual(owner.room_handoff.raw, self.handoff.raw)
            old = owner.effects.get(owner.key(owner.plan.ordered[0]))
            self.assertFalse(old.fresh_intent)
            self.assertFalse(old.prepared_this_owner)

    def test_room_loss_after_synced_dispatch_prevents_commit_without_erasing_uncertainty(self):
        game = self.game()
        with self.create() as owner:
            game.batch = owner
            acquisition = self.acquire()
            review = runtime.review_step(owner, self.authority, acquisition)
            wall = next(p for p, shape in _requirements(self.room.json(), lambda: None).items() if shape == 2)
            def obstruct(plan):
                acquisition.changes[wall] = {1: 3}
            game.before_prepare = obstruct
            with self.assertRaises(ValueError):
                runtime.advance(owner, review['expected_plan'], review['confirm_review'], self.authority, acquisition)
            self.assertEqual(game.calls.count('commit'), 0)
            self.assertEqual(game.calls.count('prepare'), 1)
            child = owner.effects.get(owner.key(owner.plan.ordered[0]))
            self.assertTrue(child.state.dispatched)
            self.assertTrue(child.state.pending)
            identity = owner.id
        with stores.Batch(str(self.path), identity, runtime.Budget(60000), writable=True) as owner:
            self.assertEqual(owner.audit()['status'], 'pending_recovery')
            self.assertFalse(owner.audit()['advance_allowed'])
            with self.assertRaises(ValueError):
                runtime.advance(owner, review['expected_plan'], review['confirm_review'], self.authority, acquisition)
            self.assertEqual(game.calls.count('commit'), 0)

    def test_reopened_second_delivery_preserves_full_parent_and_placed_floor_requirement(self):
        game = self.game()
        with self.create() as owner:
            game.batch = owner; acquisition = self.acquire()
            review = runtime.review_step(owner, self.authority, acquisition)
            runtime.advance(owner, review['expected_plan'], review['confirm_review'], self.authority, acquisition)
            identity = owner.id
        with stores.Batch(str(self.path), identity, runtime.Budget(60000), writable=True) as owner:
            game.batch = owner; acquisition = self.acquire()
            state = runtime.review_step(owner, self.authority, acquisition)
            self.assertEqual(state['route_frontier']['status'], 'ready')
            self.assertEqual(state['route_frontier']['room_terrain']['occupied_floor_tiles'], 1)
            state = runtime.advance(owner, state['expected_plan'], state['confirm_review'], self.authority, acquisition)
            self.assertEqual(state['placed'], 2)
            self.assertEqual(game.calls.count('commit'), 2)
            self.assertEqual(state['original_room_plan'], self.room.json())
        with stores.Batch(str(self.path), identity, runtime.Budget(60000)) as owner:
            acquisition = self.acquire()
            acquisition.changes[owner.plan.ordered[0].target] = {1: 2, 6: 1}
            state = runtime.review_step(owner, self.authority, acquisition)
            self.assertEqual(state['route_frontier']['status'], 'room_blocked')
            self.assertEqual(state['placed'], 2)

    def test_pending_predecessor_is_not_overridden_by_a_matched_room(self):
        game = self.game()
        with self.create() as owner:
            game.batch = owner; acquisition = self.acquire()
            state = runtime.review_step(owner, self.authority, acquisition)
            runtime.advance(owner, state['expected_plan'], state['confirm_review'], self.authority, acquisition)
            identity = owner.id
        with stores.Batch(str(self.path), identity, runtime.Budget(60000)) as owner:
            acquisition = self.acquire()
            acquisition.construct_status = 'pending'
            state = runtime.review_step(owner, self.authority, acquisition)
            self.assertEqual(state['route_frontier']['room_terrain']['status'], 'matched')
            self.assertEqual(state['route_frontier']['status'], 'construction_pending')
            self.assertIsNone(state['confirm_review'])
            self.assertEqual(game.calls.count('commit'), 1)

    def test_lost_commit_recovers_original_key_without_room_acquisition_or_new_dispatch(self):
        game = self.game()
        with self.create() as owner:
            game.batch = owner; acquisition = self.acquire()
            state = runtime.review_step(owner, self.authority, acquisition)
            game.lose_commit = True
            with self.assertRaises(ConnectionError):
                runtime.advance(owner, state['expected_plan'], state['confirm_review'], self.authority, acquisition)
            identity = owner.id
        with stores.Batch(str(self.path), identity, runtime.Budget(60000), writable=True) as owner:
            # Recovery has ONLY build-read authority; map and placement grants
            # are deliberately absent. No room/route acquisition is requested.
            with patch.dict(os.environ, {native.OPT_IN: '1', native.TOKEN: 'b' * 32}, clear=True):
                read = native.Authority.load()
                state = runtime.recover(owner, owner.plan.ordered[0].name, read)
            self.assertEqual(state['placed'], 1)
            self.assertEqual(state['room_handoff_digest'], self.handoff.digest)
            self.assertEqual(game.calls.count('commit'), 1)
            self.assertEqual(game.calls.count('prepare'), 1)
            self.assertEqual(acquisition.calls, 3)

    def test_stop_and_resolved_receipt_inspection_keep_parent_offline(self):
        game = self.game()
        with self.create() as owner:
            game.batch = owner; acquisition = self.acquire()
            state = runtime.review_step(owner, self.authority, acquisition)
            runtime.advance(owner, state['expected_plan'], state['confirm_review'], self.authority, acquisition)
            calls = len(game.calls)
            with patch.dict(os.environ, {}, clear=True):
                state = runtime.recover(owner, owner.plan.ordered[0].name)
                stopped = owner.stop()
            self.assertEqual(len(game.calls), calls)
            self.assertEqual(stopped['original_room_plan'], self.room.json())
            self.assertFalse(stopped['advance_allowed'])
            self.assertEqual(state['placed'], 1)

    def test_room_confirmation_cannot_be_reused_after_parent_replacement(self):
        game = self.game()
        with self.create() as owner:
            game.batch = owner; acquisition = self.acquire()
            state = runtime.review_step(owner, self.authority, acquisition)
            original = owner.files['batch.json'].raw
            (self.path/'batch.json').write_bytes(original + b' ')
            with self.assertRaises(ValueError):
                runtime.advance(owner, state['expected_plan'], state['confirm_review'], self.authority, acquisition)
            self.assertEqual(game.calls.count('prepare'), 0)
            self.assertEqual(game.calls.count('commit'), 0)
            self.assertTrue(owner.fenced)

    def test_full_32_target_parent_fits_unchanged_storage_and_output_reservation(self):
        self.handoff, self.room, self.review, self.source, self.raw = fixture(count=16, dining=True)
        self.before = capture(self.review, self.room)
        self.game()
        with self.create() as owner:
            state = runtime.review_step(owner, self.authority, self.acquire())
            self.assertEqual(len(state['plan']['steps']), 32)
            self.assertEqual(state['route_frontier']['status'], 'ready')
            runtime.reserve(owner.audit(), state['route_frontier'])
            self.assertLess(len(owner.files['batch.json'].raw), stores.MAX_DEFINITION)
            self.assertLess(len(runtime.encoded('review', state)), runtime.MAX_OUTPUT)

    def test_cli_init_accepts_complete_room_origin_and_inspect_subprocess_is_offline(self):
        self.game()
        path = self.root/'handoff.json'; path.write_bytes(self.handoff.raw)
        buffer = io.BytesIO()
        with patch.object(sys, 'stdout', SimpleNamespace(buffer=buffer)):
            status = runtime.main(['init', '--directory', str(self.path), '--room-handoff-file', str(path),
                                   '--expected-room-handoff', self.handoff.digest, '--timeout-ms', '60000'])
        self.assertEqual(status, 0)
        result = json.loads(buffer.getvalue())['result']
        path.unlink()  # Original input is no longer needed for private recovery.
        command = "import runpy,sys; sys.addaudithook(lambda e,a: (_ for _ in ()).throw(RuntimeError(e)) if e in ('socket.__new__','subprocess.Popen') else None); runpy.run_module('route_furniture_batch',run_name='__main__')"
        env = {'PYTHONPATH': str(Path(__file__).parent), 'PYTHONDONTWRITEBYTECODE': '1'}
        process = subprocess.run([sys.executable, '-W', 'error', '-c', command, 'inspect', '--directory',
                                  str(self.path), '--batch-id', result['batch_id']],
                                 env=env, capture_output=True, timeout=20)
        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertEqual(json.loads(process.stdout)['result']['original_room_plan'], self.room.json())

    def test_cli_origin_pairs_are_exclusive_initialization_only_before_contact(self):
        cases = [
            ['init', '--room-handoff-file', 'secret'],
            ['init', '--expected-room-handoff', '0' * 64],
            ['init', '--room-handoff-file', 'secret', '--expected-room-handoff', '0' * 64,
             '--review-file', 'other', '--expected-review', '1' * 64],
            ['inspect', '--batch-id', '0' * 64, '--room-handoff-file', 'secret',
             '--expected-room-handoff', '1' * 64],
            ['init'],
        ]
        with patch.object(runtime, 'read_review', side_effect=AssertionError('input contacted')), \
             patch.object(native, 'Client', side_effect=AssertionError('native contacted')):
            for argv in cases:
                buffer = io.BytesIO()
                with patch.object(sys, 'stdout', SimpleNamespace(buffer=buffer)):
                    code = runtime.main(argv + ['--directory', str(self.path)])
                self.assertEqual(code, 2)
                out = json.loads(buffer.getvalue())
                self.assertIsNone(out['result'])
                self.assertNotIn(b'secret', buffer.getvalue())
        self.assertEqual(list(self.path.iterdir()), [])

    def test_room_input_symlinks_wrong_digests_and_oversize_refuse_without_native_calls(self):
        path = self.root/'handoff'; path.write_bytes(self.handoff.raw)
        link = self.root/'link'; link.symlink_to(path)
        with self.assertRaises(OSError):
            runtime.read_review(str(link), self.handoff.digest, runtime.Budget(), room_backed=True)
        with self.assertRaises(ValueError):
            runtime.read_review(str(path), '0' * 64, runtime.Budget(), room_backed=True)
        path.write_bytes(b'x' * (100 * 1024 + 1))
        with self.assertRaises(ValueError):
            runtime.read_review(str(path), '0' * 64, runtime.Budget(), room_backed=True)

    def test_short_stdout_is_one_attempt_with_parent_custody_still_held(self):
        with self.create() as owner:
            identity = owner.id
        class Short:
            calls = 0
            def write(stream, raw):
                stream.calls += 1
                with self.assertRaises(BlockingIOError):
                    stores.Batch(str(self.path), identity, runtime.Budget())
                return len(raw) - 1
            def flush(stream):
                raise AssertionError('flush after short write')
        short = Short()
        with patch.dict(os.environ, {}, clear=True), patch.object(sys, 'stdout', SimpleNamespace(buffer=short)):
            code = runtime.main(['inspect', '--directory', str(self.path), '--batch-id', identity])
        self.assertEqual(code, 2)
        self.assertEqual(short.calls, 1)
        with stores.Batch(str(self.path), identity, runtime.Budget()) as owner:
            self.assertEqual(owner.room_handoff.digest, self.handoff.digest)


    def test_default_acquisition_shell_applies_room_gate_to_same_closed_capture(self):
        with self.create() as owner:
            ports = ReadPorts(owner, self.room, self.raw, self.before)
            start = owner.budget.work_steps
            with ports.installed():
                report = runtime.acquire_frontier(owner, self.before, self.authority, owner.budget)
            self.assertEqual(report['policy'], room_gate.POLICY)
            self.assertEqual(report['status'], 'ready')
            self.assertEqual(report['room_terrain']['map_sha256'], report['source']['map_sha256'])
            self.assertEqual(report['room_handoff_digest'], self.handoff.digest)
            self.assertEqual(ports.calls, ['connect', 'bracket', 'decode_operations', 'project', 'construction'])
            self.assertEqual(len(ports.clients), 1)
            self.assertTrue(ports.clients[0].closed)
            self.assertGreater(start - owner.budget.work_steps, runtime.ANALYSIS_RESERVATION)
            self.assertEqual(report['maximum_work'], runtime.ANALYSIS_RESERVATION)
            self.assertLess(report['work_units'], report['maximum_work'])

    def test_default_shell_room_failure_cannot_hide_behind_a_ready_route(self):
        self.game()
        wall = next(p for p, shape in _requirements(self.room.json(), lambda: None).items() if shape == 2)
        raw = map_bytes(self.room, {wall: {1: 3}})
        with self.create() as owner:
            ports = ReadPorts(owner, self.room, raw, self.before)
            with ports.installed():
                state = runtime.review_step(owner, self.authority)
            self.assertEqual(state['route_frontier']['execution_frontier_status'], 'ready')
            self.assertEqual(state['route_frontier']['status'], 'room_blocked')
            self.assertIsNone(state['confirm_review'])
            self.assertFalse(state['advance_allowed'])
            self.assertEqual(owner.effects.journals, {})

    def test_default_shell_preserves_construction_blocker_after_room_match(self):
        with self.create() as owner:
            ports = ReadPorts(owner, self.room, self.raw, self.before, construction_status='construction_pending')
            with ports.installed():
                report = runtime.acquire_frontier(owner, self.before, self.authority, owner.budget)
            self.assertEqual(report['room_terrain']['status'], 'matched')
            self.assertEqual(report['status'], 'construction_pending')

    def test_failed_initial_room_publication_is_neither_repaired_nor_reinitialized(self):
        def torn(fd, raw):
            os.write(fd, raw[:17])
            raise OSError('partial write')
        with patch.object(storage, 'write_all', side_effect=torn):
            with self.assertRaises(OSError):
                self.create()
        saved = (self.path/'batch.json').read_bytes()
        self.assertEqual(len(saved), 17)
        with self.assertRaises(ValueError):
            stores.Batch(str(self.path), 'a' * 64, runtime.Budget(60000), writable=True)
        with self.assertRaises(ValueError):
            self.create()
        self.assertEqual((self.path/'batch.json').read_bytes(), saved)
        self.assertEqual(list((self.path/'effects').iterdir()), [])

    def test_final_room_serialization_does_not_outlive_current_authority(self):
        game = self.game()
        real = runtime.encoded
        def revoke(operation, state, error=False):
            raw = real(operation, state, error)
            if operation == 'review':
                os.environ.pop(runtime.MAP_TOKEN, None)
            return raw
        with self.create() as owner, patch.object(runtime, 'encoded', side_effect=revoke):
            with self.assertRaises(ValueError):
                runtime.review_step(owner, self.authority, self.acquire())
            self.assertEqual(owner.effects.journals, {})
            self.assertEqual(game.calls.count('prepare'), 0)
            self.assertEqual(game.calls.count('commit'), 0)

    def test_exhausted_shared_analysis_does_not_publish_room_success(self):
        with self.create() as owner:
            ports = ReadPorts(owner, self.room, self.raw, self.before)
            owner.budget.work_steps = runtime.ANALYSIS_RESERVATION
            with ports.installed(), self.assertRaises(ValueError):
                runtime.acquire_frontier(owner, self.before, self.authority, owner.budget)
            self.assertEqual(ports.clients, [])
            self.assertEqual(owner.effects.journals, {})


    def test_machine_contract_matches_closed_origin_and_live_gate_limits(self):
        contract = json.loads((Path(__file__).parents[1]/'architecture/room_route_batch_v1.json').read_bytes())
        self.assertEqual(contract['batch_schema'], stores.ROOM_SCHEMA)
        self.assertEqual(contract['legacy_schema'], stores.SCHEMA)
        self.assertEqual(contract['frontier_policy'], room_gate.POLICY)
        self.assertEqual(contract['predecessor_policy'], room_gate.INPUT_POLICY)
        self.assertEqual(contract['limits']['definition_bytes'], stores.MAX_DEFINITION)
        self.assertEqual(contract['limits']['index_bytes'], stores.MAX_INDEX)
        self.assertEqual(contract['limits']['total_output_bytes'], runtime.MAX_OUTPUT)
        self.assertEqual(contract['limits']['analysis_work_per_acquisition'], runtime.ANALYSIS_RESERVATION)
        self.assertEqual(contract['limits']['room_frontier_input_bytes'], room_gate.MAX_INPUT)
        self.assertEqual(contract['limits']['room_frontier_output_bytes'], room_gate.MAX_OUTPUT)
        with self.create() as owner:
            self.assertEqual(set(owner.value), set(contract['common_fields'] + contract['origin_fields']))
            self.assertEqual(set(owner.names()), set(contract['required_files']))
        for field in ('automatic_migration', 'repair_or_retry', 'atomic_room_placement_proven',
                      'room_completion_proven', 'production_admitted'):
            self.assertFalse(contract[field])


if __name__ == '__main__':
    unittest.main()
