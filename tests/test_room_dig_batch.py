"""Executable room->dig custody, receipt, restart and fault tests.

Actual native clients/codecs and private journals run against an independent
synthetic TCP peer. No DFHack plugin, fortress or production admission is used.
"""
from contextlib import redirect_stdout
import copy
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import dig_blueprint as b
import dig_blueprint_client as c
import dig_designation_client as d
import room_excavation_handoff as h
import room_terrain as terrain
import room_terrain_fixtures as f
from room_dig_peer import DigPeer, DIG_TOKEN
from test_room_excavation_handoff import fixture, environment as map_environment

ROOT = Path(__file__).resolve().parents[1]


def environment(address, control=True):
    return {'DFMCP_ALLOW_UNADMITTED_DIG_V1_16': '1', 'DFMCP_DIG_TOKEN': DIG_TOKEN.decode(),
            'DFMCP_DIG_ENDPOINT': address, **({'DFMCP_DIG_ALLOW_DESIGNATE': '1'} if control else {})}


class RoomDigTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='room-dig-')
        self.root = Path(self.temp.name).resolve()
        self.root.chmod(0o700)
        self.directory = self.root / 'batch'
        self.directory.mkdir(mode=0o700)
        self.plan, self.selected, self.raw, self.dug = fixture()
        self.input = self.root / 'handoff.json'

    def tearDown(self):
        self.temp.cleanup()

    def handoff(self, peer):
        return h.RoomExcavationHandoff.create(self.plan, self.raw, f.MANIFEST, peer.address)

    def initialize(self, peer, *, directory=None):
        with patch.dict(os.environ, environment(peer.address, False), clear=True):
            return c.initialize_rooms(directory or self.directory, self.handoff(peer), c.POLICY, c.Budget(10000))

    def review(self, peer):
        with patch.dict(os.environ, environment(peer.address, False), clear=True):
            return c.observe(self.directory, c.Budget(10000))

    def advance(self, peer, review, *, budget=None):
        with patch.dict(os.environ, environment(peer.address), clear=True):
            return c.advance(self.directory, review['batch_id'], review['step'], review['observation']['witness'],
                review['plan_digest_for_confirmation'], review['review_seal'], budget or c.Budget(10000))

    def cli(self, command, *arguments, peer=None, control=True):
        env = {k: v for k, v in os.environ.items() if not k.startswith('DFMCP_')}
        if peer:
            env.update(environment(peer.address, control))
        result = subprocess.run([sys.executable, str(ROOT / 'scripts/dig_blueprint_client.py'), command,
            '--directory', str(self.directory), *map(str, arguments)], env=env, capture_output=True,
            timeout=20, check=False)
        self.assertEqual(result.stderr, b'')
        return result.returncode, json.loads(result.stdout), result.stdout

    def assert_original(self, result):
        value = result.get('batch', result)['room_excavation']
        self.assertEqual(value['room_plan'], self.plan.json())
        self.assertEqual(f.expand(value['remaining_blueprint']), self.selected.floors - self.dug)
        self.assertFalse(value['room_completion_proven'])
        self.assertFalse(value['replacement_effect_key_authorized'])
        self.assertNotIn('capture_hex', value)

    def test_actual_cli_map_export_to_all_native_designations_and_offline_original_plan(self):
        with DigPeer(self.raw, self.selected.region, already_dug=self.dug) as peer:
            plan_file = self.root / 'plan.json'
            plan_file.write_bytes(self.plan.encode())
            exported = subprocess.run([sys.executable, str(ROOT / 'scripts/survey_rooms.py'),
                '--plan-file', str(plan_file), '--emit', 'excavation-handoff'], capture_output=True,
                env={**{k:v for k,v in os.environ.items() if not k.startswith('DFMCP_')},
                     **map_environment(peer.address)}, timeout=10)
            self.assertEqual((exported.returncode, exported.stderr), (0, b''))
            self.input.write_bytes(exported.stdout)
            code, first, _ = self.cli('init', '--room-handoff', self.input, '--checkpoint-policy', c.POLICY,
                                       peer=peer, control=False)
            self.assertEqual(code, 0, first)
            self.assertEqual(first['batch_format'], c.ROOM_FORMAT)
            self.assert_original(first)
            self.input.unlink(); plan_file.unlink()
            manifest = (self.directory / c.MANIFEST).read_bytes()
            for step in range(first['total_steps']):
                code, review, _ = self.cli('observe', peer=peer, control=False)
                self.assertEqual((code, review['step']), (0, step))
                self.assert_original(review)
                code, result, _ = self.cli('advance', '--batch-id', first['batch_id'], '--step', step,
                    '--expected-witness', review['observation']['witness'],
                    '--confirm-plan', review['plan_digest_for_confirmation'], '--review-seal', review['review_seal'], peer=peer)
                self.assertEqual(code, 0, result)
                self.assertEqual(result['batch']['designated_steps'], step + 1)
                self.assert_original(result)
            self.assertEqual(result['batch']['phase'], 'designations_verified')
            self.assertFalse(result['excavation_completion_proven'])
            self.assertEqual(peer.designated, self.selected.floors - self.dug)
            self.assertEqual(len(peer.commits), first['total_steps'])
            self.assertEqual(peer.map_reads, 1)
            self.assertEqual(manifest, (self.directory / c.MANIFEST).read_bytes())
            before = len(peer.operations)
            code, recovered, _ = self.cli('query', '--batch-id', first['batch_id'], '--step', 0)
            self.assertEqual((code, recovered['native_calls']), (0, 0))
            self.assert_original(recovered)
            code, plan, raw = self.cli('inspect', '--emit', 'room-plan')
            self.assertEqual((code, raw), (0, self.plan.encode()))
            self.assertEqual(f.RoomPlan.decode(raw).json(), self.plan.json())
            self.assertEqual(len(peer.operations), before)

    def test_source_mismatch_refuses_before_creating_any_batch_files(self):
        for field, value in (('folder', 'other'), ('site', 4), ('dimensions', (200, 200, 20)),
                             ('tick', 99), ('paused', False), ('df_version', 'different'),
                             ('dfhack_version', 'different')):
            with self.subTest(field=field), DigPeer(self.raw, self.selected.region) as peer:
                setattr(peer, field, value)
                self.assertRaises(ValueError, self.initialize, peer)
                self.assertEqual(list(self.directory.iterdir()), [])
                self.assertEqual(peer.commits, [])

    def test_endpoint_and_explicit_overrides_refused_before_native_contact(self):
        with DigPeer(self.raw, self.selected.region) as peer:
            handoff = self.handoff(peer)
            layout = b.Layout.from_json(handoff.json()['remaining_blueprint'])
            with patch.dict(os.environ, environment('127.0.0.1:1'), clear=True):
                self.assertRaises(ValueError, c.initialize_rooms, self.directory, handoff, c.POLICY, c.Budget(10000))
            self.input.write_bytes(handoff.encode())
            for override in (['--allow-hidden-neighbors'], ['--site', '2'], ['--world-folder', 'region1']):
                code, result, _ = self.cli('init', '--room-handoff', self.input, '--checkpoint-policy', c.POLICY,
                                          *override, peer=peer)
                self.assertEqual(code, 2)
                self.assertFalse(result['ok'])
            with patch.dict(os.environ, environment(peer.address), clear=True):
                self.assertRaises(ValueError, c.initialize, self.directory, layout, 'region1', 2, True,
                                  c.POLICY, c.Budget(10000), room_handoff=handoff)
            self.assertEqual(peer.accepted, 0)
            self.assertEqual(list(self.directory.iterdir()), [])

    def test_rehashed_retained_handoff_or_residual_substitution_blocks_reopening(self):
        with DigPeer(self.raw, self.selected.region) as peer:
            self.initialize(peer)
            original = (self.directory / c.MANIFEST).read_bytes()
            value = json.loads(original)['value']
            variants = []
            for target in ('blueprint', 'room_handoff'):
                changed = copy.deepcopy(value)
                if target == 'blueprint':
                    layout = b.Layout.from_json(self.plan.json()['excavation_blueprint'])
                    changed['blueprint'], changed['layout_digest'] = layout.blueprint(), layout.digest
                else:
                    changed['room_handoff']['remaining_mask_digest'] = 'f' * 64
                variants.append(c.file_bytes(changed))
            variants.extend([original[:-1], original + b'\n'])
            before = len(peer.operations)
            for raw in variants:
                (self.directory / c.MANIFEST).write_bytes(raw)
                self.assertRaises(ValueError, self.review, peer)
                self.assertEqual((self.directory / c.MANIFEST).read_bytes(), raw)
            self.assertEqual(len(peer.operations), before)
            (self.directory / c.MANIFEST).write_bytes(original)
            self.assert_original(c.inspect(self.directory, c.Budget(10000)))

    def test_native_generation_clock_dimensions_and_unpaused_review_cannot_rebind(self):
        with DigPeer(self.raw, self.selected.region) as peer:
            self.initialize(peer)
            for field, value in (('generation', 901), ('sequence', 0), ('tick', 99),
                                 ('dimensions', (100, 100, 100)), ('paused', False)):
                if field == 'sequence':
                    continue
                old = getattr(peer, field)
                setattr(peer, field, value)
                with self.subTest(field=field):
                    self.assertRaises(ValueError, self.review, peer)
                    self.assertEqual(peer.commits, [])
                setattr(peer, field, old)
            review = self.review(peer)
            self.advance(peer, review)
            peer.sequence = 0
            self.assertRaises(ValueError, self.review, peer)
            self.assertEqual(len(peer.commits), 1)

    def test_stale_confirmation_does_not_mint_intent_or_replan(self):
        with DigPeer(self.raw, self.selected.region) as peer:
            first = self.initialize(peer)
            review = self.review(peer)
            peer.tick += 1
            self.assertRaises(ValueError, self.advance, peer, review)
            result = c.inspect(self.directory, c.Budget(10000))
            self.assertEqual((result['batch_id'], result['records']), (first['batch_id'], []))
            self.assertFalse(any(op == 'PrepareDesignation' for _, op in peer.operations))
            self.assert_original(result)

    def test_lost_commit_reply_reconciles_original_key_without_second_dispatch(self):
        with DigPeer(self.raw, self.selected.region) as peer:
            first = self.initialize(peer)
            review = self.review(peer)
            peer.fault = 'lost_commit'
            self.assertRaises(ValueError, self.advance, peer, review)
            unresolved = c.inspect(self.directory, c.Budget(10000))
            self.assertEqual((unresolved['phase'], unresolved['unresolved_step']), ('unresolved', 0))
            self.assert_original(unresolved)
            before = len(peer.operations)
            self.assertRaises(ValueError, self.advance, peer, review)
            self.assertRaises(ValueError, self.review, peer)
            self.assertEqual(len(peer.operations), before)
            peer.fault = None
            with patch.dict(os.environ, environment(peer.address, False), clear=True):
                recovered = c.recover(self.directory, first['batch_id'], 0, False, c.Budget(10000))
            self.assertEqual(recovered['batch']['designated_steps'], 1)
            self.assertEqual(peer.operations[before:], [('dig', 'Handshake'), ('dig', 'QueryDesignation')])
            self.assertEqual(peer.commits, [review['key']])
            self.assert_original(recovered)
            with patch.dict(os.environ, {}, clear=True):
                cached = c.recover(self.directory, first['batch_id'], 0, True, c.Budget(10000))
            self.assertEqual(cached['native_calls'], 0)
            self.assertEqual(len(peer.commits), 1)

    def test_lost_prepare_unknown_query_then_explicit_cancel_never_dispatches(self):
        with DigPeer(self.raw, self.selected.region) as peer:
            first = self.initialize(peer)
            review = self.review(peer)
            peer.fault = 'lost_prepare'
            self.assertRaises(ValueError, self.advance, peer, review)
            peer.fault = 'unknown_query'
            with patch.dict(os.environ, environment(peer.address, False), clear=True):
                result = c.recover(self.directory, first['batch_id'], 0, False, c.Budget(10000))
            self.assertEqual(result['batch']['phase'], 'unresolved')
            self.assertFalse(result['absence_proves_non_application'])
            self.assert_original(result)
            peer.fault = None
            with patch.dict(os.environ, environment(peer.address), clear=True):
                result = c.recover(self.directory, first['batch_id'], 0, True, c.Budget(10000))
            self.assertEqual((result['batch']['phase'], result['batch']['next_step']), ('refused', None))
            self.assertEqual(result['effect']['reason'], 'cancelled_before_dispatch')
            self.assertEqual(peer.commits, [])
            self.assertRaises(ValueError, self.review, peer)

    def test_authority_revoked_after_prepare_leaves_original_uncertain_work(self):
        with DigPeer(self.raw, self.selected.region) as peer:
            self.initialize(peer)
            review = self.review(peer)
            def revoke(operation):
                if operation == 'PrepareDesignation':
                    os.environ.pop('DFMCP_DIG_ALLOW_DESIGNATE', None)
            peer.callback = revoke
            self.assertRaises(ValueError, self.advance, peer, review)
            self.assertEqual(peer.commits, [])
            result = c.inspect(self.directory, c.Budget(10000))
            self.assertEqual(result['phase'], 'unresolved')
            self.assert_original(result)

    def test_bad_but_rehashed_native_readback_is_not_terminal_proof(self):
        with DigPeer(self.raw, self.selected.region) as peer:
            self.initialize(peer)
            review = self.review(peer)
            peer.fault = 'bad_readback'
            self.assertRaises(ValueError, self.advance, peer, review)
            result = c.inspect(self.directory, c.Budget(10000))
            self.assertEqual(result['phase'], 'unresolved')
            self.assertIsNone(result['records'][0]['terminal_receipt_sha256'])
            self.assertEqual(len(peer.commits), 1)
            self.assertRaises(ValueError, self.review, peer)

    def test_live_manifest_loss_fences_commit_and_missing_original_blocks_disclosure(self):
        with DigPeer(self.raw, self.selected.region) as peer:
            self.initialize(peer)
            review = self.review(peer)
            def remove(operation):
                if operation == 'PrepareDesignation':
                    (self.directory / c.MANIFEST).unlink()
            peer.callback = remove
            self.assertRaises((OSError, ValueError), self.advance, peer, review)
            self.assertEqual(peer.commits, [])
            self.assertRaises(OSError, c.inspect, self.directory, c.Budget(10000))
            self.assertEqual(len(list((self.directory / c.EFFECTS).glob('step-*.json'))), 1)

    def test_final_serialization_revocation_after_durable_effect_withholds_success(self):
        with DigPeer(self.raw, self.selected.region) as peer:
            self.initialize(peer)
            review = self.review(peer)
            original = c.serialize_result
            def serialized(value, budget):
                if value.get('effect_status') == 'designated' and 'batch' in value:
                    os.environ.pop('DFMCP_DIG_TOKEN', None)
                return original(value, budget)
            out = io.StringIO()
            with patch.dict(os.environ, environment(peer.address), clear=True), \
                    patch.object(c, 'serialize_result', serialized), redirect_stdout(out):
                code = c.main(['advance', '--directory', str(self.directory), '--batch-id', review['batch_id'],
                    '--step', str(review['step']), '--expected-witness', review['observation']['witness'],
                    '--confirm-plan', review['plan_digest_for_confirmation'], '--review-seal', review['review_seal']])
            self.assertEqual(code, 2)
            self.assertFalse(json.loads(out.getvalue())['ok'])
            self.assertNotIn(DIG_TOKEN.decode(), out.getvalue())
            self.assertEqual(len(peer.commits), 1)
            result = c.inspect(self.directory, c.Budget(10000))
            self.assertEqual(result['designated_steps'], 1)
            self.assert_original(result)

    def test_local_stop_retains_plan_does_not_cancel_unknown_native_effect(self):
        with DigPeer(self.raw, self.selected.region) as peer:
            first = self.initialize(peer)
            review = self.review(peer)
            peer.fault = 'lost_commit'
            self.assertRaises(ValueError, self.advance, peer, review)
            before = len(peer.operations)
            with patch.dict(os.environ, {}, clear=True):
                result = c.stop(self.directory, first['batch_id'], c.Budget(10000))
            self.assertFalse(result['native_effects_cancelled'])
            self.assert_original(result)
            state = c.inspect(self.directory, c.Budget(10000))
            self.assertEqual(state['phase'], 'unresolved')
            self.assertIsNone(state['next_step'])
            self.assertEqual(len(peer.operations), before)
            peer.fault = None
            with patch.dict(os.environ, environment(peer.address, False), clear=True):
                result = c.recover(self.directory, first['batch_id'], 0, False, c.Budget(10000))
            self.assertEqual(result['batch']['designated_steps'], 1)
            self.assertIsNone(result['batch']['next_step'])

    def test_native_call_and_work_budgets_are_shared_and_never_renewed(self):
        with DigPeer(self.raw, self.selected.region) as peer:
            handoff = self.handoff(peer)
            for field, value in (('work_left', 0), ('calls_left', 7), ('network_left', 1)):
                budget = c.Budget(10000)
                setattr(budget, field, value)
                with patch.dict(os.environ, environment(peer.address, False), clear=True):
                    self.assertRaises(ValueError, c.initialize_rooms, self.directory, handoff, c.POLICY, budget)
                self.assertEqual(list(self.directory.iterdir()), [])
            self.initialize(peer)
            review = self.review(peer)
            budget = c.Budget(10000)
            self.advance(peer, review, budget=budget)
            self.assertEqual(budget.calls_left, 0)  # 6 binds + handshake + read + prepare + commit.
            self.assertLess(budget.network_left, 4 * 1024 * 1024)
            self.assertLess(budget.work_left, c.ROOM_MAX_WORK)

    def test_private_input_and_manifest_bounds_preserve_legacy_rules(self):
        self.input.write_bytes(self.handoff(type('Endpoint', (), {'address': '127.0.0.1:5000'})()).encode())
        self.assertEqual(c.read_room_handoff(self.input, c.Budget(10000)).plan().encode(), self.plan.encode())
        link = self.root / 'link'
        link.symlink_to(self.input)
        self.assertRaises(OSError, c.read_room_handoff, link, c.Budget(10000))
        for raw in (b'', b'x' * (h.MAX_BYTES + 1), b'[' * 15 + b'0' + b']' * 15):
            self.input.write_bytes(raw)
            self.assertRaises(ValueError, c.read_room_handoff, self.input, c.Budget(10000))
        self.assertEqual((c.MAX_MANIFEST, c.MAX_OUTPUT), (65536, 131072))
        self.assertRaises(ValueError, c.serialize_result, {'padding': 'x' * c.MAX_OUTPUT}, c.Budget(10000))
        with DigPeer(self.raw, self.selected.region) as peer:
            self.initialize(peer)
            manifest = self.directory / c.MANIFEST
            original = manifest.read_bytes()
            manifest.chmod(0o644)
            self.assertRaises(ValueError, c.inspect, self.directory, c.Budget(10000))
            manifest.chmod(0o600)
            value = json.loads(original)['value']
            value['format'] = c.FORMAT
            # New room data may not be smuggled into the legacy profile.
            manifest.write_bytes(c.file_bytes(value))
            self.assertRaises(ValueError, c.inspect, self.directory, c.Budget(10000))

    def test_original_room_constraints_bind_batch_identity_and_every_recovery_key(self):
        with DigPeer(self.raw, self.selected.region) as peer:
            first = self.initialize(peer)
            value = json.loads((self.directory / c.MANIFEST).read_bytes())['value']
            self.assertEqual(first['batch_id'], d.digest(b'dfmcp-dig-blueprint-batch/2', d.canonical(value)).hex())
            before = c.Batch(None, None, value, c.Budget(10000))
            altered = copy.deepcopy(self.plan.json()['intent'])
            altered['areas'][0]['item_constraints']['bed']['max_distance'] = 1
            plan = f.RoomPlan.compile(altered)
            replacement = h.RoomExcavationHandoff.create(plan, self.raw, f.MANIFEST, peer.address)
            other = c.Batch(None, None, {**value, 'room_handoff': replacement.json()}, c.Budget(10000))
            self.assertEqual(before.layout.digest, other.layout.digest)
            self.assertNotEqual(before.id, other.id)
            self.assertNotEqual(before.step_key(0), other.step_key(0))
            summary = before.room_summary()
            summary['survey_source']['folder'] = 'forged'
            summary['room_plan']['intent']['areas'].clear()
            self.assertEqual(before.room_summary()['room_plan'], self.plan.json())
            self.assertEqual(before.room_summary()['survey_source']['folder'], 'region1')

    def test_legacy_batch_still_executes_and_has_identical_contract(self):
        with DigPeer(self.raw, self.selected.region, already_dug=self.dug) as peer:
            handoff = self.handoff(peer)
            layout = b.Layout.from_json(handoff.json()['remaining_blueprint'])
            with patch.dict(os.environ, environment(peer.address), clear=True):
                first = c.initialize(self.directory, layout, 'region1', 2, False, c.POLICY, c.Budget(10000))
            value = json.loads((self.directory / c.MANIFEST).read_bytes())['value']
            self.assertEqual(value['format'], c.FORMAT)
            self.assertNotIn('room_handoff', value)
            self.assertNotIn('room_excavation', first)
            self.assertEqual(first['batch_id'], d.digest(b'dfmcp-dig-blueprint-batch/1', d.canonical(value)).hex())
            reviewed = self.review(peer)
            self.assertNotIn('room_excavation', reviewed)
            result = self.advance(peer, reviewed)
            self.assertEqual(result['batch']['designated_steps'], 1)
            self.assertNotIn('room_excavation', result['batch'])
            with patch.dict(os.environ, {}, clear=True):
                self.assertEqual(c.recover(self.directory, first['batch_id'], 0, False, c.Budget(10000))['native_calls'], 0)
            code, error, _ = self.cli('inspect', '--emit', 'room-plan')
            self.assertEqual(code, 2)
            self.assertFalse(error['ok'])

    def test_32_slot_plan_and_all_native_steps_preserve_every_exclusion(self):
        request = f.request(f.bedroom(8), f.dining(4, 4, origin=(26, 10, 2)))
        request['excluded_items'] = list(range(2147400000, 2147400646))
        self.plan = f.RoomPlan.compile(request)
        self.selected = terrain.selection(self.plan)
        # Retain one already-dug floor; all other requested tiles remain exact targets.
        self.dug = {min(self.selected.floors, key=terrain.order)}
        self.raw = f.capture(self.selected.region, {p: f.visible(3) for p in self.dug})
        with DigPeer(self.raw, self.selected.region, already_dug=self.dug) as peer:
            first = self.initialize(peer)
            self.assertEqual(first['room_excavation']['room_plan']['furniture_count'], 32)
            for _ in range(first['total_steps']):
                result = self.advance(peer, self.review(peer))
                self.assert_original(result)
            self.assertEqual(result['batch']['phase'], 'designations_verified')
            self.assertEqual(len(peer.commits), first['total_steps'])
            self.assertEqual(peer.designated, self.selected.floors - self.dug)
            self.assertEqual(len(result['batch']['room_excavation']['room_plan']['intent']['excluded_items']), 646)
            self.assertLessEqual(len(c.serialize_result(result, c.Budget(10000))), c.ROOM_MAX_OUTPUT)
            self.assertLessEqual((self.directory / c.MANIFEST).stat().st_size, c.ROOM_MAX_MANIFEST)


if __name__ == '__main__':
    unittest.main()
