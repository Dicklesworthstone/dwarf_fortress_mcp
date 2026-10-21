"""Real run orchestration with explicit batch/per-step producer ports.

These tests do not qualify file custody, terrain acquisition or native effects.
The unchanged one-step owner remains responsible for those boundaries.
Beads: df-dfhack-bridge-plane-c-pic.4/.5.
"""
from contextlib import contextmanager
from copy import deepcopy
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import route_furniture_run as runner
from test_route_furniture_run import Authority, Batch, Port, consent


class RoomBatch(Batch):
    def __init__(self, size, budget):
        super().__init__(size, budget)
        self.room_handoff = SimpleNamespace(digest='d' * 64)
        self.room_plan_digest = 'e' * 64

    def audit(self):
        value = super().audit()
        value.update(room_handoff_digest=self.room_handoff.digest,
            room_plan_digest=self.room_plan_digest,
            original_room_plan={'plan_digest': self.room_plan_digest, 'all_rooms': ['original']},
            complete_original_room_intent_retained=True,
            room_constraints_checked_this_call=False, room_completion_proven=False)
        return value


@contextmanager
def environment(size=3, cap=3):
    port = Port()
    room_status = {}
    def decorate(batch, state):
        status = room_status.get(state['next_step'], 'ready')
        geometry = {'room_blocked': 'contradicted', 'room_unestablished': 'unknown'}.get(status, 'matched')
        prior = 'ready' if status in runner.ROOM_FRONTIERS | {'ready'} else status
        state['route_frontier'] = {'status': status, 'execution_frontier_status': prior,
            'room_handoff_digest': batch.room_handoff.digest, 'room_plan_digest': batch.room_plan_digest,
            'room_constraints_checked_this_call': True,
            'room_terrain': {'status': geometry, 'all_required_shapes_at_sample': geometry == 'matched',
                             'deficits': {'count': 0 if geometry == 'matched' else 1}}}
        state['room_constraints_checked_this_call'] = True
        if status != 'ready':
            state.update(advance_allowed=False, expected_plan=None, confirm_review=None)
    port.after_review = decorate
    with patch.dict('sys.modules', route_furniture_batch=port):
        limits = runner.Limits(cap, 60000)
        batch = RoomBatch(size, runner.make_budget(limits))
        yield port, batch, Authority(), limits, room_status


class RoomRunTests(unittest.TestCase):
    def test_unready_first_room_is_a_complete_read_only_stop(self):
        for status in runner.ROOM_FRONTIERS:
            with self.subTest(status=status), environment() as (port, batch, auth, limits, statuses):
                statuses['s00'] = status
                review = runner.review_run(batch, auth, limits)
                self.assertIsNone(review['run_review']['confirm_run'])
                batch.budget = runner.make_budget(limits)
                value = runner.run(batch, auth, limits, 'f' * 64)
                self.assertEqual(value['run']['stop_reason'], status)
                self.assertFalse(value['run']['faulted'])
                self.assertFalse(value['run']['confirmation_validated'])
                self.assertEqual(value['run']['outcomes'], [])
                self.assertEqual(value['route_frontier']['status'], status)
                self.assertTrue(value['room_constraints_checked_this_call'])
                self.assertFalse(value['advance_allowed'])
                self.assertEqual(port.advances, [])

    def test_room_loss_after_progress_preserves_exact_diagnosis_and_prefix(self):
        for status in runner.ROOM_FRONTIERS:
            with self.subTest(status=status), environment() as (port, batch, auth, limits, statuses):
                token = consent(batch, auth, limits)
                statuses['s01'] = status
                budget = batch.budget
                value = runner.run(batch, auth, limits, token)
                self.assertIs(batch.budget, budget)
                self.assertEqual(port.advances, ['s00'])
                self.assertEqual(value['run']['stop_reason'], status)
                self.assertFalse(value['run']['faulted'])
                self.assertEqual(value['run']['confirmed_placements_this_call'], 1)
                self.assertEqual(value['next_step'], 's01')
                self.assertEqual(len(value['plan']['steps']), 3)
                self.assertEqual(value['route_frontier']['room_terrain']['deficits']['count'], 1)
                self.assertEqual(value['run']['scope']['room_handoff_digest'], batch.room_handoff.digest)
                self.assertFalse(value['run']['automatic_recovery_attempted'])
                self.assertFalse(value['run']['automatic_retry_permitted'])
                batch.budget = runner.make_budget(limits)
                statuses.clear()
                with self.assertRaises(ValueError):
                    runner.run(batch, auth, limits, token)
                self.assertEqual(port.advances, ['s00'])

    def test_all_32_steps_and_each_cap_keep_parent_without_claiming_completion(self):
        for cap in range(1, 33):
            with self.subTest(cap=cap), environment(32, cap) as (port, batch, auth, limits, _):
                value = runner.run(batch, auth, limits, consent(batch, auth, limits))
                self.assertEqual(port.advances, [s.name for s in batch.plan.ordered[:cap]])
                self.assertEqual(value['run']['scope']['room_plan_digest'], batch.room_plan_digest)
                self.assertEqual(value['original_room_plan']['all_rooms'], ['original'])
                self.assertFalse(value['room_constraints_checked_this_call'])
                self.assertFalse(value['room_completion_proven'])
                self.assertFalse(value['advance_allowed'])

    def test_run_confirmation_binds_both_parent_identities(self):
        for field in ('room_handoff', 'room_plan'):
            with self.subTest(field=field), environment() as (port, batch, auth, limits, _):
                token = consent(batch, auth, limits)
                if field == 'room_handoff':
                    batch.room_handoff.digest = '1' * 64
                else:
                    batch.room_plan_digest = '2' * 64
                with self.assertRaises(ValueError):
                    runner.run(batch, auth, limits, token)
                self.assertEqual(port.advances, [])

    def test_missing_or_changed_parent_in_review_is_never_a_new_scope(self):
        for field in runner.ROOM_CUSTODY_FIELDS:
            with self.subTest(field=field), environment() as (port, batch, auth, limits, _):
                original = port.after_review
                def corrupt(batch, state):
                    original(batch, state)
                    state.pop(field)
                port.after_review = corrupt
                with self.assertRaises((ValueError, KeyError)):
                    runner.review_run(batch, auth, limits)
                self.assertEqual(port.advances, [])

    def test_ready_room_cannot_omit_current_geometry_or_launder_prior_blocker(self):
        for case in range(7):
            with self.subTest(case=case), environment() as (port, batch, auth, limits, _):
                original = port.after_review
                def corrupt(batch, state):
                    original(batch, state)
                    report = state['route_frontier']
                    if case == 0: state['room_constraints_checked_this_call'] = False
                    elif case == 1: report['room_constraints_checked_this_call'] = False
                    elif case == 2: report['room_handoff_digest'] = '0' * 64
                    elif case == 3: report.pop('room_terrain')
                    elif case == 4: report['room_terrain']['status'] = 'unknown'
                    elif case == 5: report['room_terrain']['all_required_shapes_at_sample'] = 1
                    else: report['execution_frontier_status'] = 'construction_pending'
                port.after_review = corrupt
                with self.assertRaises(ValueError): runner.review_run(batch, auth, limits)
                self.assertEqual(port.advances, [])

    def test_existing_route_and_construction_blockers_keep_priority(self):
        for status in runner.UNREADY_FRONTIERS - runner.ROOM_FRONTIERS:
            with self.subTest(status=status), environment() as (port, batch, auth, limits, statuses):
                token = consent(batch, auth, limits)
                statuses['s01'] = status
                value = runner.run(batch, auth, limits, token)
                self.assertEqual(value['run']['stop_reason'], status)
                self.assertFalse(value['run']['faulted'])
                self.assertEqual(port.advances, ['s00'])

    def test_unknown_or_contradictory_room_states_refuse(self):
        for case in ('room_repaired', 'room_blocked', 'room_unestablished'):
            with self.subTest(case=case), environment() as (port, batch, auth, limits, statuses):
                statuses['s00'] = case
                original = port.after_review
                def corrupt(batch, state):
                    original(batch, state)
                    state['route_frontier']['room_terrain']['status'] = 'matched'
                port.after_review = corrupt
                with self.assertRaises(ValueError): runner.review_run(batch, auth, limits)
                self.assertEqual(port.advances, [])

    def test_lost_commit_reply_stops_with_original_parent_not_cached_readiness(self):
        with environment() as (port, batch, auth, limits, _):
            token = consent(batch, auth, limits)
            def fail(batch, name):
                batch.records[name] = 'indeterminate'
                raise OSError('lost reply')
            port.after_advance = fail
            value = runner.run(batch, auth, limits, token)
            self.assertTrue(value['run']['faulted'])
            self.assertEqual(value['pending_step'], 's00')
            self.assertEqual(value['room_handoff_digest'], batch.room_handoff.digest)
            self.assertFalse(value['room_constraints_checked_this_call'])
            self.assertEqual(port.advances, ['s00'])

    def test_failed_second_acquisition_does_not_republish_previous_room_result(self):
        with environment() as (port, batch, auth, limits, _):
            token = consent(batch, auth, limits)
            original = port.after_review
            def fail(batch, state):
                if state['next_step'] == 's01': raise OSError('read failed')
                original(batch, state)
            port.after_review = fail
            value = runner.run(batch, auth, limits, token)
            self.assertEqual(value['run']['stop_reason'], 'review_failed')
            self.assertNotIn('route_frontier', value)
            self.assertFalse(value['room_constraints_checked_this_call'])
            self.assertEqual(port.advances, ['s00'])

    def test_legacy_batch_cannot_adopt_room_diagnostics(self):
        from test_route_furniture_run import environment as legacy
        for change in ('parent', 'status'):
            with self.subTest(change=change), legacy() as (port, batch, auth, limits):
                def corrupt(batch, state):
                    if change == 'parent': state['room_handoff_digest'] = 'd' * 64
                    else:
                        state['route_frontier']['status'] = 'room_blocked'
                        state.update(expected_plan=None, confirm_review=None)
                port.after_review = corrupt
                with self.assertRaises(ValueError): runner.review_run(batch, auth, limits)
                self.assertEqual(port.advances, [])

    def test_publication_rechecks_authority_after_rendering_room_stop(self):
        with environment() as (port, batch, auth, limits, statuses):
            token = consent(batch, auth, limits)
            statuses['s00'] = 'room_unestablished'
            original = port.encoded
            def render(operation, value, error=False):
                raw = original(operation, value, error)
                if operation == 'run' and 'run' in value: auth.revoked = True
                return raw
            port.encoded = render
            with self.assertRaises(ValueError): runner.run(batch, auth, limits, token)
            self.assertEqual(port.advances, [])


if __name__ == '__main__':
    unittest.main()
