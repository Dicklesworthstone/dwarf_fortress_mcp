"""Run the actual batch/coordinator/private journals with injected native calls.

Map captures are synthetic; acquisition is injected, but its full frontier and
suffix checks use the production codecs and solvers. No journal/step function is
replaced. Separate TCP coverage lives in test_route_furniture_run_tcp.py.
"""
from __future__ import annotations

from contextlib import contextmanager
from dataclasses import replace
import hashlib
import io
import json
import os
from pathlib import Path
import struct
import tempfile
import types
import unittest
from unittest.mock import patch

import build_placement_client as placement
import build_placement_rpc as native
import build_placement_store as storage
from build_placement_wire import Capture, Insertion, Item, Plan, Record, Tile
from furniture_allocation import Candidate, Request, Slot
from furniture_plan import canonical
import route_furniture_allocation as routes
import route_furniture_batch as steps
from route_furniture_batch_store import Batch
from route_furniture_review import Source, create
import route_furniture_run as run


def terrain(width, occupied=(), blocked=()):
    size = (width, 5, 1)
    raw = bytearray(b'DFMM1500' + struct.pack('>IIBI', 0, 100, 1, 1))
    raw += struct.pack('>H', 4) + b'fort'
    raw += struct.pack('>9I', 128, 128, 8, 0, 0, 0, *size)
    raw += struct.pack('>I', width * 5)
    for y in range(5):
        for x in range(width):
            p = (x, y, 0)
            raw += b'\2' + struct.pack('>IBBBBBBBIHH', 10, 2 if p in blocked else 3,
                                       0, 0, 0, 0, int(p in occupied), 0, 1, 10015, 10015)
    return bytes(raw)


class Game:
    """Only the native/acquisition boundary is a double; no durable state is mocked."""
    def __init__(self, count=2):
        self.width = 3 * count + 6
        self.items = tuple(Candidate(100 + i, 'bed', (1, 2, 0), (419, -1), -1) for i in range(count))
        self.request = Request('fort', 1, tuple(Slot(f's{i:02}', 'bed', (3 + i * 3, 2, 0),
                                material=(419, -1), subtype=-1, max_distance=200) for i in range(count)))
        self.records, self.placed, self.calls, self.frontiers = {}, [], [], []
        self.manifest = native.Manifest(11, 'df', 'dfhack')
        self.address = ('127.0.0.1', 5000)
        self.on_commit = lambda plan: None
        self.on_frontier = lambda batch, before: None
        self.on_prepare = lambda plan: None
        self.fault = None
        self.blocked = set()
        self.budgets = []

    def source(self, raw):
        return Source(f'{self.address[0]}:{self.address[1]}', 'df', 'dfhack', 7, 9,
                      hashlib.sha256(raw).hexdigest(), 'e' * 64, 100, (128, 128, 8),
                      (0, 0, 0), (self.width, 5, 1),
                      (1000 + len(self.placed), 2000 + len(self.placed), 10000))

    def capture(self, selected):
        item = next(i for i in self.items if i.id == selected.item)
        floor = Tile(2, 10, 3)
        value = Item(presence=2, pos=item.position, kind=1, native_type=30, subtype=-1,
                     material=419, material_index=-1, on_ground=True, ground=floor)
        return Capture(11, len(self.placed), 100, 1, (128, 128, 8),
                       2000 + len(self.placed), 1000 + len(self.placed), len(self.placed),
                       'fort', True, True, True, selected, (floor,) * 9, value)

    def review(self):
        raw = terrain(self.width)
        value = routes.allocate_routes(self.request, self.items, raw, max_route_steps=200)
        return create(self.request, value, self.source(raw), {i.id: 30 for i in self.items})

    def frontier(self, batch, before, authority, budget):
        self.frontiers.append(before.selection.item)
        self.budgets.append(budget)
        self.on_frontier(batch, before)
        raw = terrain(self.width, (r.plan.before.selection.target for r in self.placed), self.blocked)
        used = {r.plan.before.selection.item for r in self.placed}
        available = tuple(i for i in self.items if i.id not in used)
        prefix = batch.placed_prefix()
        budget.work_steps -= steps.ANALYSIS_RESERVATION
        return steps.frontier.assess(batch.review, batch.id, prefix, before, available,
            {i.id: 30 for i in available}, raw, self.source(raw), budget.remaining,
            maximum_work=steps.ANALYSIS_RESERVATION)

    def client(self, authority, budget, selected):
        game = self
        class Client:
            def __init__(self):
                self.manifest = game.manifest
                self.permit = None
                self.closed = False
                game.budgets.append(budget)
                authority.guard('Handshake')
                budget.reserve_call()

            def reply(self, name, *, record=None, capture=None, replayed=None):
                authority.guard(name)
                budget.reserve_call()
                game.calls.append((name, None if record is None else record.plan.key))
                return native.Reply(game.manifest, any(r.phase == 'indeterminate' for r in game.records.values()),
                                    len(game.records), capture, record, replayed)

            def observe(self):
                return self.reply('ReadPlacement', capture=game.capture(selected))

            def query(self, plan):
                return self.reply('QueryPlacement', record=game.records.get(plan.key))

            def prepare(self, plan):
                assert self.permit is None and plan.before.raw == game.capture(selected).raw
                game.on_prepare(plan)
                if game.fault == 'prepare_lost':
                    game.records[plan.key] = Record(plan, 'prepared', 'none')
                    raise OSError('lost prepare')
                self.permit = plan
                game.records[plan.key] = Record(plan, 'prepared', 'none')
                return self.reply('PreparePlacement', record=game.records[plan.key], replayed=False)

            def commit(self, plan):
                assert self.permit == plan
                self.permit = None
                game.on_commit(plan)
                if game.fault == 'indeterminate':
                    record = Record(plan, 'indeterminate', 'native_failure')
                elif game.fault == 'refused':
                    record = Record(plan, 'refused', 'stale')
                else:
                    b = plan.before
                    ins = Insertion(b.next_building, b.next_job, selected.item, selected.kind,
                                    selected.target, b.item.material, b.item.material_index,
                                    0, 3, True, True, True, False)
                    record = Record(plan, 'placed', 'none', b.expected_after(), ins)
                    game.placed.append(record)
                game.records[plan.key] = record
                out = self.reply('CommitPlacement', record=record)
                if game.fault == 'commit_lost':
                    raise OSError('lost commit reply')
                return out

            def cancel(self, plan):
                raise AssertionError('run must not cancel')

            def __enter__(self): return self
            def __exit__(self, *_): self.closed = True
        return Client()


@contextmanager
def environment(count=2):
    game = Game(count)
    env = {key: value for key, value in os.environ.items() if not key.startswith('DFMCP_')}
    env.update({steps.OPT_IN: '1', steps.PLACE: '1', native.TOKEN: 't' * 32,
                steps.OPS_TOKEN: 'o' * 32, steps.MAP_TOKEN: 'm' * 32})
    with tempfile.TemporaryDirectory(prefix='dfmcp-run-test-') as path, patch.dict(os.environ, env, clear=True), \
         patch.object(native, 'Client', game.client), patch.object(placement, 'Client', game.client), \
         patch.object(steps, 'acquire_frontier', game.frontier):
        os.chmod(path, 0o700)
        saved = game.review()
        state = steps.initialize(path, saved, steps.Authority.load(), steps.Budget(60000))
        yield game, path, state['batch_id'], saved


def review(path, batch_id, limits):
    with Batch(path, batch_id, run.make_budget(limits)) as batch:
        return run.review_run(batch, steps.Authority.load(), limits)['run_review']['confirm_run']


def execute(path, batch_id, limits, token):
    with Batch(path, batch_id, run.make_budget(limits), writable=True) as batch:
        return run.run(batch, steps.Authority.load(True), limits, token)


class JournalRunTests(unittest.TestCase):
    def test_multiple_original_steps_real_journals_and_dispatch_order(self):
        with environment(3) as (game, path, identity, saved):
            limits = run.Limits(3, 60000)
            token = review(path, identity, limits)
            def check(plan):
                file = Path(path) / 'effects' / storage.filename(plan.key)
                state = storage.replay(file.read_bytes())
                self.assertTrue(state.dispatched)
                self.assertEqual(state.frames, 3)
                index = (Path(path) / 'index.jsonl').read_bytes()
                self.assertIn(plan.key.encode(), index)
                self.assertEqual(state.plan, plan)
            game.on_commit = check
            value = execute(path, identity, limits, token)
            self.assertEqual(value['status'], 'all_placed')
            self.assertEqual(value['run']['confirmed_placements_this_call'], 3)
            self.assertEqual([r.plan.before.selection.item for r in game.placed], [100, 101, 102])
            self.assertEqual(value['original_review'], saved.json())
            self.assertEqual(len(list((Path(path) / 'effects').iterdir())), 3)
            with Batch(path, identity, steps.Budget(60000)) as reopened:
                self.assertEqual(reopened.audit()['status'], 'all_placed')
                self.assertTrue(all(not j.fresh_intent and not j.prepared_this_owner for j in reopened.effects.journals.values()))

    def test_native_final_route_gate_still_blocks_after_dispatch_sync(self):
        with environment() as (game, path, identity, saved):
            limits = run.Limits(2, 60000)
            token = review(path, identity, limits)
            game.frontiers.clear()
            def block(batch, before):
                if len(game.frontiers) == 3:
                    journal = batch.effects.get(batch.key(batch.plan.ordered[0]))
                    self.assertTrue(journal.state.dispatched)
                    game.blocked.add((6, 2, 0))  # The LATER original target.
            game.on_frontier = block
            value = execute(path, identity, limits, token)
            self.assertTrue(value['run']['faulted'])
            self.assertEqual(value['pending_step'], 's00')
            self.assertEqual(value['run']['outcomes'][0]['phase'], 'unknown')
            self.assertEqual(game.placed, [])
            self.assertFalse(any(name == 'CommitPlacement' for name, _ in game.calls))
            self.assertEqual(len(list((Path(path) / 'effects').iterdir())), 1)

    def test_lost_commit_remains_pending_and_explicit_query_recovers_without_second_writer(self):
        with environment() as (game, path, identity, saved):
            limits = run.Limits(2, 60000)
            token = review(path, identity, limits)
            game.fault = 'commit_lost'
            value = execute(path, identity, limits, token)
            self.assertEqual(value['status'], 'pending_recovery')
            self.assertEqual(value['run']['outcomes'][0]['phase'], 'unknown')
            self.assertEqual(len(game.placed), 1)
            before = len(game.calls)
            with self.assertRaises(ValueError): execute(path, identity, limits, token)
            self.assertEqual(len(game.calls), before)
            game.fault = None
            with Batch(path, identity, steps.Budget(60000), writable=True) as batch:
                recovered = steps.recover(batch, 's00', steps.Authority.load())
                self.assertEqual(recovered['placed'], 1)
            self.assertEqual(sum(n == 'CommitPlacement' for n, _ in game.calls), 1)
            with self.assertRaises(ValueError): execute(path, identity, limits, token)
            token = review(path, identity, limits)
            self.assertEqual(execute(path, identity, limits, token)['status'], 'all_placed')
            self.assertEqual(sum(n == 'CommitPlacement' for n, _ in game.calls), 2)

    def test_indeterminate_refusal_and_prepare_loss_halt_original_suffix(self):
        for fault in ('indeterminate', 'refused', 'prepare_lost'):
            with self.subTest(fault=fault), environment() as (game, path, identity, saved):
                limits = run.Limits(2, 60000)
                token = review(path, identity, limits)
                game.fault = fault
                value = execute(path, identity, limits, token)
                self.assertEqual(len(list((Path(path) / 'effects').iterdir())), 1)
                self.assertNotEqual(value['status'], 'all_placed')
                self.assertEqual(game.placed, [])
                before = len(game.calls)
                with self.assertRaises(ValueError): execute(path, identity, limits, token)
                self.assertEqual(len(game.calls), before)

    def test_reopen_prefix_limit_and_stale_consent(self):
        with environment(3) as (game, path, identity, saved):
            limits = run.Limits(1, 60000)
            token = review(path, identity, limits)
            first = execute(path, identity, limits, token)
            self.assertEqual(first['run']['stop_reason'], 'step_limit')
            self.assertEqual(first['next_step'], 's01')
            with self.assertRaises(ValueError): execute(path, identity, limits, token)
            self.assertEqual(len(game.placed), 1)
            token = review(path, identity, limits)
            second = execute(path, identity, limits, token)
            self.assertEqual(second['placed'], 2)
            self.assertEqual(second['next_step'], 's02')

    def test_failed_and_torn_dispatch_writes_do_not_contact_writer_or_repair(self):
        for torn in (False, True):
            with self.subTest(torn=torn), environment() as (game, path, identity, saved):
                limits = run.Limits(2, 60000)
                token = review(path, identity, limits)
                original = storage.write_all
                def fail(fd, raw):
                    if b'"kind":"dispatch"' in raw:
                        if torn:
                            os.write(fd, raw[:10])
                        else:
                            original(fd, raw)
                        raise OSError('ambiguous dispatch append')
                    return original(fd, raw)
                with patch.object(storage, 'write_all', fail), self.assertRaises(ValueError):
                    execute(path, identity, limits, token)
                self.assertEqual(game.placed, [])
                files = list((Path(path) / 'effects').iterdir())
                self.assertEqual(len(files), 1)
                prior = files[0].read_bytes()
                if torn:
                    with self.assertRaises(ValueError):
                        Batch(path, identity, steps.Budget(60000))
                else:
                    with Batch(path, identity, steps.Budget(60000)) as reopened:
                        self.assertEqual(reopened.audit()['status'], 'pending_recovery')
                self.assertEqual(files[0].read_bytes(), prior)

    def test_preparation_and_dispatch_sync_failures_preserve_original_pending_work(self):
        for boundary in ('prepared', 'dispatch'):
            with self.subTest(boundary=boundary), environment() as (game, path, identity, saved):
                limits = run.Limits(2, 60000)
                token = review(path, identity, limits)
                original = os.fsync
                failed = []
                def sync(fd):
                    try:
                        raw = os.pread(fd, storage.MAX_FILE, 0)
                    except OSError:
                        return original(fd)
                    if raw and raw.endswith(b'\n'):
                        last = json.loads(raw.splitlines()[-1])
                        if last.get('format') == storage.FORMAT and last.get('kind') == boundary:
                            failed.append(boundary)
                            raise OSError('injected sync failure')
                    return original(fd)
                with patch.object(os, 'fsync', sync), self.assertRaises(ValueError):
                    execute(path, identity, limits, token)
                self.assertEqual(failed, [boundary])
                self.assertEqual(game.placed, [])
                with Batch(path, identity, steps.Budget(60000)) as reopened:
                    self.assertEqual(reopened.audit()['status'], 'pending_recovery')
                    journal = next(iter(reopened.effects.journals.values()))
                    self.assertEqual(journal.state.dispatched, boundary == 'dispatch')
                    self.assertFalse(journal.prepared_this_owner)

    def test_complete_dispatch_bytes_are_synced_before_each_writer(self):
        with environment() as (game, path, identity, saved):
            limits = run.Limits(2, 60000)
            token = review(path, identity, limits)
            synced = set()
            original = os.fsync
            def sync(fd):
                original(fd)
                try:
                    raw = os.pread(fd, storage.MAX_FILE, 0)
                except OSError:
                    return
                if raw and b'"format":"dfmcp.build-placement-journal/1"' in raw:
                    synced.add(hashlib.sha256(raw).digest())
            def check(plan):
                raw = (Path(path) / 'effects' / storage.filename(plan.key)).read_bytes()
                self.assertTrue(storage.replay(raw).dispatched)
                self.assertIn(hashlib.sha256(raw).digest(), synced)
            game.on_commit = check
            with patch.object(os, 'fsync', sync):
                result = execute(path, identity, limits, token)
            self.assertEqual(result['placed'], 2)

    def test_cli_uses_its_own_module_when_loaded_under_an_entrypoint_alias(self):
        import importlib.util
        import sys
        with environment() as (game, path, identity, saved):
            spec = importlib.util.spec_from_file_location('_route_run_entrypoint', steps.__file__)
            entry = importlib.util.module_from_spec(spec)
            with patch.dict(sys.modules, {spec.name: entry}):
                spec.loader.exec_module(entry)
                seen = []
                def acquire(batch, before, authority, budget):
                    self.assertIs(type(authority), entry.Authority)
                    self.assertIs(type(budget), entry.Budget)
                    seen.append(before.selection.item)
                    return game.frontier(batch, before, authority, budget)
                common = ['--directory', path, '--batch-id', identity,
                          '--max-steps', '2', '--timeout-ms', '60000']
                with patch.object(entry, 'acquire_frontier', acquire):
                    output = io.BytesIO()
                    with patch.object(entry.sys, 'stdout', types.SimpleNamespace(buffer=output)):
                        status = entry.main(['review-run', *common])
                    value = json.loads(output.getvalue())
                    self.assertEqual(status, 0, value)
                    token = value['result']['run_review']['confirm_run']
                    output = io.BytesIO()
                    with patch.object(entry.sys, 'stdout', types.SimpleNamespace(buffer=output)):
                        status = entry.main(['run', *common, '--confirm-run', token])
                    self.assertEqual(status, 0, output.getvalue())
                    self.assertEqual(len(seen), 7)  # proposal + three fresh gates per step
                    self.assertEqual(len(game.placed), 2)

    def test_real_output_budget_for_32_step_review(self):
        with environment(32) as (game, path, identity, saved):
            limits = run.Limits(32, 60000)
            token = review(path, identity, limits)
            self.assertEqual(len(token), 64)
            self.assertEqual(game.placed, [])

    def test_cli_explicit_review_and_run_reopen_private_batch(self):
        with environment() as (game, path, identity, saved):
            common = ['--directory', path, '--batch-id', identity, '--max-steps', '2', '--timeout-ms', '60000']
            out = io.BytesIO()
            with patch.object(steps.sys, 'stdout', types.SimpleNamespace(buffer=out)):
                code = steps.main(['review-run', *common])
            value = json.loads(out.getvalue())
            self.assertEqual(code, 0, value)
            token = value['result']['run_review']['confirm_run']
            self.assertEqual(game.placed, [])
            out = io.BytesIO()
            with patch.object(steps.sys, 'stdout', types.SimpleNamespace(buffer=out)):
                code = steps.main(['run', *common, '--confirm-run', token])
            value = json.loads(out.getvalue())
            self.assertEqual(code, 0, value)
            self.assertEqual(value['result']['run']['confirmed_placements_this_call'], 2)
            self.assertEqual(value['result']['status'], 'all_placed')
            self.assertFalse(value['runtime_admitted'])

    def test_cli_bad_modes_refuse_before_open_or_native_contact(self):
        bad = [
            ['run', '--max-steps', '0', '--confirm-run', 'e' * 64],
            ['run', '--max-steps', '33', '--confirm-run', 'e' * 64],
            ['run', '--max-steps', '1'],
            ['review-run', '--max-steps', '1', '--confirm-run', 'e' * 64],
            ['inspect', '--max-steps', '1'],
            ['review', '--confirm-run', 'e' * 64],
            ['run', '--max-steps', '1', '--confirm-run', 'bad'],
        ]
        for args in bad:
            with self.subTest(args=args), patch.object(steps, 'Batch', side_effect=AssertionError('opened')), \
                 patch.object(native, 'Client', side_effect=AssertionError('contacted')):
                out = io.BytesIO()
                with patch.object(steps.sys, 'stdout', types.SimpleNamespace(buffer=out)):
                    self.assertEqual(steps.main([*args, '--directory', '/unused', '--batch-id', 'f' * 64]), 2)
                self.assertFalse(json.loads(out.getvalue())['ok'])

    def test_cli_failed_run_retains_pending_summary_and_never_duplicates_stdout(self):
        with environment() as (game, path, identity, saved):
            limits = run.Limits(2, 60000)
            token = review(path, identity, limits)
            game.fault = 'commit_lost'
            out = io.BytesIO()
            with patch.object(steps.sys, 'stdout', types.SimpleNamespace(buffer=out)):
                code = steps.main(['run', '--directory', path, '--batch-id', identity,
                                   '--max-steps', '2', '--timeout-ms', '60000', '--confirm-run', token])
            lines = out.getvalue().splitlines()
            self.assertEqual(code, 2)
            self.assertEqual(len(lines), 1)
            value = json.loads(lines[0])
            self.assertFalse(value['ok'])
            self.assertEqual(value['result']['pending_step'], 's00')
            self.assertTrue(value['result']['run']['faulted'])
            self.assertEqual(len(game.placed), 1)

    def test_stdout_failure_never_repeats_execution_or_output(self):
        class Short:
            writes = 0
            def write(self, raw): self.writes += 1; return 1
            def flush(self): pass
        with environment() as (game, path, identity, saved):
            limits = run.Limits(2, 60000)
            token = review(path, identity, limits)
            output = Short()
            with patch.object(steps.sys, 'stdout', types.SimpleNamespace(buffer=output)):
                code = steps.main(['run', '--directory', path, '--batch-id', identity,
                                   '--max-steps', '2', '--timeout-ms', '60000', '--confirm-run', token])
            self.assertEqual(code, 2)
            self.assertEqual(output.writes, 1)
            self.assertEqual(len(game.placed), 2)


if __name__ == '__main__':
    unittest.main()
