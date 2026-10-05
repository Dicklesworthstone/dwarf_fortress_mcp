"""Full original-batch waits through actual private stores and the real CLI.

Placement records are explicit fixtures, not executed DFHack effects. All batch,
origin, completion, RPC, and command code is real; only the native TCP peer is a
joined double. Beads: df-dfhack-bridge-plane-c-pic.4 / df-dfhack-bridge-plane-c-pic.5.
"""
from __future__ import annotations

from contextlib import redirect_stdout
from dataclasses import replace
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
import build_placement_store as placement
from build_placement_rpc import Manifest, Reply
from build_placement_wire import Plan, Record
from construction_plan import Goal as Condition
from construction_plan_rpc import Budget
import construction_wait as wait
import furniture_batch as batch_model
from furniture_allocation import Candidate, Request, Slot
from furniture_handoff import Handoff, InventorySource, Selected
from furniture_plan import FurniturePlan, Step
from furniture_completion import Goal, Origin
from furniture_completion_store import Journal, replay
import track_furniture_batch as cli
from construction_wait_peer import Peer, capture, records, SOFTWARE, BUILD_GENERATION, OPS_GENERATION

ROOT = Path(__file__).resolve().parents[1]
NONCE = 'c' * 48


def fixture(count=2, dense=False):
    originals = records(count)
    names = [f'{i:02}-' + ('x' * 12 if dense else 'furnishing') for i in range(count)]
    steps, retained = [], []
    for index, record in enumerate(originals):
        before = record.plan.before
        kind = ('', 'bed', 'chair', 'table')[before.selection.kind]
        dependencies = tuple(names[:index]) if dense else tuple(names[max(0, index - 1):index])
        step = Step(names[index], kind, before.selection.item, before.selection.target, dependencies)
        steps.append(step)
        retained.append(replace(record, plan=Plan('fb-' + NONCE + '-' + step.name, before)))
    return FurniturePlan(tuple(steps)), tuple(retained)


def populate(path, plan, retained, address, *, handoff=True):
    """Create exact fixture history through the unchanged private store APIs."""
    path.mkdir(mode=0o700)
    (path / 'effects').mkdir(mode=0o700)
    source = Manifest(BUILD_GENERATION, *(value.decode() for value in SOFTWARE))
    budget = Budget(60000)
    handoff_value = None
    if handoff:
        request = Request('region1', 2, tuple(Slot(s.name, s.kind, s.target, s.after,
                            (419, -1), -1, 200) for s in plan.steps))
        by_item = {r.plan.before.selection.item: r.plan.before.item for r in retained}
        inventory = InventorySource(f'{address[0]}:{address[1]}', OPS_GENERATION,
                    source.df_version, source.dfhack_version, 'a' * 64, 100, 99, (200, 100, 2000))
        handoff_value = Handoff(request, inventory, tuple(Selected(s.name,
            Candidate(s.item, s.kind, by_item[s.item].pos, (419, -1), -1), by_item[s.item].native_type)
            for s in plan.steps))
    with batch_model.root_lock(str(path), budget) as root:
        effects = placement.open_directory(str(path / 'effects'))
        try:
            value = {'schema': batch_model.HANDOFF_SCHEMA if handoff else batch_model.SCHEMA,
                     'nonce': NONCE, 'plan': plan.json(), 'source': source.view(),
                     'endpoint': f'{address[0]}:{address[1]}', 'folder': 'region1', 'site': 2,
                     'dimensions': [128, 128, 10], 'first_tick': 100,
                     'root_identity': batch_model.identity(root), 'effects_identity': batch_model.identity(effects)}
        finally:
            os.close(effects)
        if handoff_value is not None:
            value['handoff'] = handoff_value.json()
        for name, raw in (('batch.json', batch_model.seal(value)), ('steps.jsonl', batch_model.HEADER)):
            owner = batch_model.File(root, name, budget, True, raw,
                        maximum=batch_model.MAX_DEFINITION if name == 'batch.json' else batch_model.MAX_FILE)
            owner.close()
    with batch_model.Batch(str(path), budget, True) as batch:
        by_item = {r.plan.before.selection.item: r for r in retained}
        for step in plan.ordered:
            record = by_item[step.item]
            journal = batch.effects.create(record.plan, source, address)
            batch.register(step)
            prepared = Record(record.plan, 'prepared', 'none')
            journal.retain(Reply(source, False, len(retained), record=prepared), prepared=True)
            journal.append('dispatch', {'plan_digest': record.plan.digest.hex()})
            journal.retain(Reply(source, False, len(retained), record=record))
        assert batch.audit()['status'] == 'all_placed'
        return batch.id


def file_map(path):
    return {str(p.relative_to(path)): (p.stat().st_ino, hashlib.sha256(p.read_bytes()).hexdigest())
            for p in path.rglob('*') if p.is_file()}


class CompletionWaitTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.batch_path = self.root / 'batch'
        self.monitor = self.root / 'monitor'

    def command(self, peer=None, *extra, operation='wait'):
        env = peer.environment() if peer else {k: v for k, v in os.environ.items() if not k.startswith('DFMCP_')}
        process = subprocess.run([sys.executable, str(ROOT / 'scripts/track_furniture_batch.py'), operation,
            '--journal', str(self.monitor), '--timeout-ms', '60000', *extra],
            env=env, capture_output=True, timeout=40)
        self.assertEqual(process.stderr, b'')
        self.assertLessEqual(len(process.stdout), 65536)
        return process.returncode, json.loads(process.stdout), len(process.stdout)

    def prepare(self, peer, plan, retained, **kwargs):
        identity = populate(self.batch_path, plan, retained, peer.address, **kwargs)
        budget = Budget(60000)
        with batch_model.Batch(str(self.batch_path), budget) as batch:
            origin = Origin.from_batch(batch, identity)
            goal = Goal(origin, Condition(origin.receipts, 10000))
            with Journal(str(self.monitor), budget, writable=True, create=(goal, peer.address)) as owner:
                owner.bind_batch(batch)
        return goal

    def state(self):
        return replay(self.monitor.read_bytes(), Budget(60000))

    def test_32_slot_allocation_batch_start_wait_and_terminal_replay(self):
        plan, retained = fixture(32, dense=True)
        raws = [capture(retained, 101, stages=[0] * 32, filler=2000)]
        raws += [capture(retained, tick, filler=2000) for tick in (102, 103)]
        with Peer(retained, raws) as peer:
            identity = populate(self.batch_path, plan, retained, peer.address)
            old = file_map(self.batch_path)
            code, started, _ = self.command(peer, '--batch', str(self.batch_path), '--batch-id', identity,
                                             '--deadline-tick', '10000', operation='start')
            self.assertEqual(code, 0, started)
            goal_digest = started['result']['identity']['goal_digest']
            code, result, width = self.command(peer, '--poll-ms', '10')
            self.assertEqual(code, 0, result)
            self.assertEqual(result['result']['requested_plan'], plan.json())
            self.assertEqual(len(result['result']['targets']), 32)
            self.assertTrue(result['result']['complete_original_plan_sampled_condition'])
            self.assertTrue(result['result']['original_placement_history_verified'])
            self.assertEqual(result['result']['wait']['samples_published'], 2)
            self.assertEqual(result['result']['identity']['goal_digest'], goal_digest)
            self.assertEqual(peer.connections, 3)
            self.assertEqual(file_map(self.batch_path), old)
            self.assertGreater(len(raws[0]), 65536)
            self.assertLessEqual(width, 65536)
        before = self.monitor.read_bytes()
        code, result, _ = self.command()
        self.assertEqual(code, 0, result)
        self.assertTrue(result['result']['complete_original_plan_sampled_condition'])
        self.assertEqual(result['result']['wait']['samples_published'], 0)
        self.assertFalse(result['result']['native_contacted'])
        self.assertEqual(before, self.monitor.read_bytes())
        self.assertEqual(self.state().goal.encode()[:8], b'DFMFCG02')

    def test_legacy_batch_wait_preserves_original_goal_generation(self):
        plan, retained = fixture()
        with Peer(retained, [capture(retained, 101), capture(retained, 102)]) as peer:
            goal = self.prepare(peer, plan, retained, handoff=False)
            self.assertEqual(goal.encode()[:8], b'DFMFCG01')
            code, result, _ = self.command(peer, '--poll-ms', '10')
            self.assertEqual(code, 0, result)
            self.assertTrue(result['result']['complete_original_plan_sampled_condition'])
            self.assertEqual(self.state().goal.encode(), goal.encode())

    def test_original_loss_during_delay_stops_before_next_read_and_allows_local_cancel(self):
        plan, retained = fixture()
        with Peer(retained, [capture(retained, 101)]) as peer:
            self.prepare(peer, plan, retained)
            def lose(_):
                (self.batch_path / 'batch.json').rename(self.root / 'preserved-original')
            out = io.StringIO()
            with patch.dict(os.environ, peer.environment(), clear=True), patch.object(wait.time, 'sleep', lose), redirect_stdout(out):
                code = cli.main(['wait', '--journal', str(self.monitor), '--timeout-ms', '60000'])
            self.assertEqual(code, 2)
            failed = json.loads(out.getvalue())
            self.assertFalse(failed['result']['complete_original_plan_sampled_condition'])
            self.assertFalse(failed['result']['original_placement_history_verified'])
            self.assertEqual(peer.connections, 1)
            self.assertEqual(self.state().progress.observations, 1)
            self.assertFalse(self.state().progress.reading)
        code, cancelled, _ = self.command(operation='cancel')
        self.assertEqual(code, 0, cancelled)
        self.assertEqual(cancelled['result']['progress']['phase'], 'cancelled')
        self.assertFalse(cancelled['result']['original_placement_history_verified'])
        self.assertFalse(cancelled['result']['complete_original_plan_sampled_condition'])

    def test_original_loss_after_read_intent_prevents_native_contact(self):
        plan, retained = fixture()
        with Peer(retained, []) as peer:
            self.prepare(peer, plan, retained)
            original = Journal.start_read
            def lose(owner):
                original(owner)
                (self.batch_path / 'steps.jsonl').rename(self.root / 'preserved-index')
            out = io.StringIO()
            with patch.dict(os.environ, peer.environment(), clear=True), patch.object(Journal, 'start_read', lose), redirect_stdout(out):
                code = cli.main(['wait', '--journal', str(self.monitor), '--timeout-ms', '60000'])
            self.assertEqual(code, 2)
            self.assertEqual(peer.connections, 0)
            self.assertTrue(self.state().progress.reading)
            self.assertEqual(self.state().progress.observations, 0)

    def test_final_render_original_loss_withholds_durable_satisfaction(self):
        plan, retained = fixture()
        with Peer(retained, [capture(retained, 101), capture(retained, 102)]) as peer:
            self.prepare(peer, plan, retained)
            original = cli.output
            def render(value):
                raw = original(value)
                if value['result'].get('wait', {}).get('stop_reason') == 'terminal':
                    (self.batch_path / 'steps.jsonl').rename(self.root / 'preserved-index')
                return raw
            out = io.StringIO()
            with patch.dict(os.environ, peer.environment(), clear=True), patch.object(cli, 'output', render), \
                 patch.object(wait.time, 'sleep', lambda _: None), redirect_stdout(out):
                code = cli.main(['wait', '--journal', str(self.monitor), '--timeout-ms', '60000'])
            self.assertEqual(code, 2)
            result = json.loads(out.getvalue())['result']
            self.assertFalse(result['complete_original_plan_sampled_condition'])
            self.assertIsNone(result['progress'])
            self.assertEqual(self.state().progress.phase, 'satisfied')
        code, result, _ = self.command()
        self.assertEqual(code, 2)
        self.assertFalse(result['result']['complete_original_plan_sampled_condition'])

    def test_final_render_revocation_withholds_cached_batch_completion(self):
        plan, retained = fixture()
        with Peer(retained, [capture(retained, 101), capture(retained, 102)]) as peer:
            self.prepare(peer, plan, retained)
            old = file_map(self.batch_path)
            original = cli.output
            def revoke(value):
                raw = original(value)
                if value['result'].get('wait', {}).get('stop_reason') == 'terminal':
                    os.environ['DFMCP_BUILD_TOKEN'] = 'r' * 40
                return raw
            out = io.StringIO()
            with patch.dict(os.environ, peer.environment(), clear=True), patch.object(cli, 'output', revoke), \
                 patch.object(wait.time, 'sleep', lambda _: None), redirect_stdout(out):
                code = cli.main(['wait', '--journal', str(self.monitor), '--timeout-ms', '60000'])
            self.assertEqual(code, 2)
            result = json.loads(out.getvalue())['result']
            self.assertFalse(result['complete_original_plan_sampled_condition'])
            self.assertIsNone(result['progress'])
            self.assertEqual(self.state().progress.phase, 'satisfied')
            self.assertEqual(file_map(self.batch_path), old)

    def test_original_operations_generation_is_not_silently_adopted(self):
        plan, retained = fixture()
        with Peer(retained, [capture(retained, 101), capture(retained, 102)], generations={1: 12}) as peer:
            goal = self.prepare(peer, plan, retained)
            code, result, _ = self.command(peer, '--poll-ms', '10')
            self.assertEqual(code, 2, result)
            self.assertEqual(peer.connections, 2)
            state = self.state()
            self.assertTrue(state.progress.reading)
            self.assertEqual(state.progress.observations, 1)
            self.assertEqual(state.goal.digest, goal.digest)

    def test_missing_original_steps_cannot_enter_wait(self):
        plan, retained = fixture()
        with Peer(retained, []) as peer:
            goal = self.prepare(peer, plan, retained)
            original = self.monitor.read_bytes()
            (self.batch_path / 'effects' / placement.filename(retained[-1].plan.key)).rename(self.root / 'preserved-child')
            code, result, _ = self.command(peer)
            self.assertEqual(code, 2, result)
            self.assertFalse(result['result']['original_placement_history_verified'])
            self.assertEqual(peer.connections, 0)
            self.assertEqual(self.monitor.read_bytes(), original)
            self.assertEqual(self.state().goal.digest, goal.digest)

    def test_wait_cannot_override_original_batch_or_fixed_policy(self):
        plan, retained = fixture()
        with Peer(retained, []) as peer:
            self.prepare(peer, plan, retained)
            original = self.monitor.read_bytes()
            for name, value in (('--batch', str(self.root)), ('--batch-id', 'd' * 64),
                                ('--deadline-tick', '20000'), ('--stable-samples', '5'), ('--poll-ms', '0')):
                code, result, _ = self.command(peer, name, value)
                self.assertEqual(code, 2, result)
                self.assertEqual(self.monitor.read_bytes(), original)
            self.assertEqual(peer.connections, 0)

    def test_killed_process_recovers_only_by_new_explicit_wait(self):
        plan, retained = fixture()
        reached, release = threading.Event(), threading.Event()
        def hold(index, method, request):
            if index == 1 and method == 3 and not reached.is_set():
                reached.set()
                if not release.wait(10):
                    raise AssertionError('process-kill test was not released')
        class KillPeer(Peer):
            def handle(self, sock, index):
                try:
                    super().handle(sock, index)
                except (BrokenPipeError, ConnectionResetError):
                    if index != 1:
                        raise
        with KillPeer(retained, [capture(retained, t) for t in (101, 102, 103, 104)], hook=hold) as peer:
            goal = self.prepare(peer, plan, retained)
            old = file_map(self.batch_path)
            child = subprocess.Popen([sys.executable, str(ROOT / 'scripts/track_furniture_batch.py'),
                'wait', '--journal', str(self.monitor), '--poll-ms', '10', '--timeout-ms', '60000'],
                env=peer.environment(), stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            try:
                self.assertTrue(reached.wait(10), 'child did not reach its second read')
                child.kill()
                child.communicate(timeout=5)
                release.set()
                state = self.state()
                self.assertTrue(state.progress.reading)
                self.assertEqual(state.progress.observations, 1)
                self.assertEqual(peer.connections, 2)
                code, result, _ = self.command(peer, '--poll-ms', '10')
                self.assertEqual(code, 0, result)
                self.assertEqual(peer.connections, 4)
                self.assertEqual(result['result']['progress']['interrupted_reads'], 1)
                self.assertEqual(result['result']['progress']['first_tick'], 103)
                self.assertTrue(result['result']['complete_original_plan_sampled_condition'])
                self.assertEqual(self.state().goal.digest, goal.digest)
                self.assertEqual(file_map(self.batch_path), old)
            finally:
                release.set()
                if child.poll() is None:
                    child.kill()
                    child.communicate(timeout=5)


if __name__ == '__main__':
    unittest.main()
