"""Actual run owner with explicit step/custody ports; not native/game evidence."""
from __future__ import annotations

from contextlib import contextmanager
from copy import deepcopy
import hashlib
import sys
import time
import types
import unittest
from unittest.mock import patch

from furniture_plan import FurniturePlan, Step, canonical, progress
import route_furniture_run as run


def sha(value):
    return hashlib.sha256(canonical(value)).hexdigest()


class Budget:
    def __init__(self, timeout_ms):
        self.deadline = time.monotonic() + timeout_ms / 1000
        self.calls, self.network_bytes = 560, 40 * 1024 * 1024
        self.disk_bytes, self.work_steps = 1024 * 1024 * 1024, 20_000_000

    def remaining(self):
        if self.deadline <= time.monotonic():
            raise ValueError('deadline')
        return self.deadline - time.monotonic()


class Authority:
    revoked = False

    def guard(self, operation=None):
        if self.revoked:
            raise ValueError('revoked')


class Batch:
    def __init__(self, size, budget):
        self.plan = FurniturePlan(tuple(Step(f's{i:02}', 'bed', i, (i + 2, 2, 0)) for i in range(size)))
        self.id = 'b' * 64
        self.review = types.SimpleNamespace(digest='c' * 64)
        self.budget, self.writable, self.stopped = budget, True, False
        self.records = {}
        self.checks = 0
        self.corrupt = False
        self.generation = 1

    def check(self):
        self.checks += 1
        self.budget.remaining()
        if self.corrupt:
            raise ValueError('custody changed')

    def audit(self):
        self.check()
        out = progress(self.plan, self.records)
        for row in out['steps']:
            if row['name'] in self.records:
                row.update(receipt_digest=sha(row) if row['phase'] == 'placed' else None,
                           dispatch_recorded=True)
        out.update(batch_id=self.id, review_digest=self.review.digest, inventory_verified=True,
                   inventory_digest=sha([self.id, self.records, self.stopped]), stopped=self.stopped,
                   advance_allowed=out['status'] == 'ready' and not self.stopped)
        return out


class Port(types.ModuleType):
    """Trusted step port double. The production owner is not rewritten for tests."""
    def __init__(self):
        super().__init__('route_furniture_batch')
        self.Budget = Budget
        self.MAX_OUTPUT = 256 * 1024
        self.reviews, self.advances = [], []
        self.after_review = lambda batch, state: None
        self.before_advance = lambda batch, name: None
        self.after_advance = lambda batch, name: None
        self.blocked = set()

    def encoded(self, operation, state, error=False):
        return canonical({'operation': operation, 'ok': not error, 'result': state})

    def review_step(self, batch, authority):
        authority.guard()
        state = batch.audit()
        name = state['next_step']
        self.reviews.append(name)
        batch.budget.calls -= 15
        batch.budget.work_steps -= 6_000_000
        state['expected_plan'] = sha([name, batch.generation, state['inventory_digest']])
        state['confirm_review'] = sha([state['expected_plan'], 'paired-route'])
        state['route_frontier'] = {'status': 'ready'}
        if name in self.blocked:
            state.update(expected_plan=None, confirm_review=None, route_frontier={'status': 'route_blocked'})
        self.after_review(batch, state)
        return state

    def advance(self, batch, expected, confirmation, authority):
        authority.guard('CommitPlacement')
        state = batch.audit()
        name = state['next_step']
        assert expected == sha([name, batch.generation, state['inventory_digest']])
        assert confirmation == sha([expected, 'paired-route'])
        self.advances.append(name)
        self.before_advance(batch, name)
        batch.records[name] = 'placed'
        batch.budget.calls -= 30
        batch.budget.work_steps -= 12_000_000
        self.after_advance(batch, name)
        return dict(batch.audit(), advanced_step=name)


@contextmanager
def environment(size=3, count=3):
    port = Port()
    with patch.dict(sys.modules, route_furniture_batch=port):
        limits = run.Limits(count, 60000)
        batch = Batch(size, run.make_budget(limits))
        authority = Authority()
        yield port, batch, authority, limits


def consent(batch, authority, limits):
    value = run.review_run(batch, authority, limits)
    # Review and execution are distinct operations, each with ONE new total
    # budget. Within a run there is never a new budget per step.
    batch.budget = run.make_budget(limits)
    return value['run_review']['confirm_run']


class RunTests(unittest.TestCase):
    def test_complete_bounded_prefix_and_one_budget_for_every_step(self):
        with environment() as (port, batch, authority, limits):
            token = consent(batch, authority, limits)
            before = batch.budget
            result = run.run(batch, authority, limits, token)
            self.assertIs(batch.budget, before)
            self.assertEqual(port.advances, ['s00', 's01', 's02'])
            self.assertEqual(result['run']['stop_reason'], 'all_placed')
            self.assertEqual(result['run']['confirmed_placements_this_call'], 3)
            self.assertEqual(result['run']['advance_invocations_this_call'], 3)
            self.assertLess(batch.budget.work_steps, limits.json()['maximum_work_units'])
            self.assertFalse(result['run']['automatic_retry_permitted'])
            self.assertFalse(result['run']['construction_completion_proven'])

    def test_explicit_limit_stops_and_fresh_consent_resumes_only_original_suffix(self):
        with environment(3, 1) as (port, batch, authority, limits):
            token = consent(batch, authority, limits)
            result = run.run(batch, authority, limits, token)
            self.assertEqual(result['run']['stop_reason'], 'step_limit')
            self.assertEqual(result['next_step'], 's01')
            batch.budget = run.make_budget(limits)
            with self.assertRaises(ValueError):
                run.run(batch, authority, limits, token)
            self.assertEqual(port.advances, ['s00'])
            token = consent(batch, authority, limits)
            run.run(batch, authority, limits, token)
            self.assertEqual(port.advances, ['s00', 's01'])

    def test_32_steps_and_all_possible_limits_preserve_exact_prefix(self):
        for limit in range(1, 33):
            with self.subTest(limit=limit), environment(32, limit) as (port, batch, authority, limits):
                token = consent(batch, authority, limits)
                value = run.run(batch, authority, limits, token)
                self.assertEqual(port.advances, [s.name for s in batch.plan.ordered[:limit]])
                self.assertEqual(value['run']['scope']['approved_steps'], port.advances)
                self.assertLess(len(port.encoded('run', value)), port.MAX_OUTPUT)

    def test_confirmation_binds_count_timeout_review_inventory_and_native_frontier(self):
        for kind in ('count', 'timeout', 'review', 'batch', 'native'):
            with self.subTest(kind=kind), environment() as (port, batch, authority, limits):
                token = consent(batch, authority, limits)
                if kind == 'count':
                    limits = run.Limits(2, 60000)
                elif kind == 'timeout':
                    limits = run.Limits(3, 30000)
                elif kind == 'review':
                    batch.review.digest = 'd' * 64
                elif kind == 'batch':
                    batch.id = 'e' * 64
                else:
                    batch.generation += 1
                batch.budget = run.make_budget(limits)
                with self.assertRaises(ValueError):
                    run.run(batch, authority, limits, token)
                self.assertEqual(port.advances, [])

    def test_wrong_malformed_or_missing_confirmation_never_dispatches(self):
        for token in ('bad', None, 'f' * 64):
            with self.subTest(token=token), environment() as (port, batch, authority, limits):
                with self.assertRaises(ValueError):
                    run.run(batch, authority, limits, token)
                self.assertEqual(port.advances, [])

    def test_stopped_pending_and_resolved_batch_refuse_before_native_review(self):
        for phase in ('indeterminate', 'unknown', 'refused', 'cancelled', 'complete', 'stopped'):
            with self.subTest(phase=phase), environment() as (port, batch, authority, limits):
                token = consent(batch, authority, limits)
                before = len(port.reviews)
                if phase == 'stopped':
                    batch.stopped = True
                elif phase == 'complete':
                    batch.records = {s.name: 'placed' for s in batch.plan.steps}
                else:
                    batch.records['s00'] = phase
                with self.assertRaises(ValueError):
                    run.run(batch, authority, limits, token)
                self.assertEqual(len(port.reviews), before)
                self.assertEqual(port.advances, [])

    def test_new_route_blockage_stops_before_the_next_intent(self):
        with environment() as (port, batch, authority, limits):
            token = consent(batch, authority, limits)
            port.blocked.add('s01')
            value = run.run(batch, authority, limits, token)
            self.assertEqual(value['run']['stop_reason'], 'route_blocked')
            self.assertEqual(port.advances, ['s00'])
            self.assertEqual(value['next_step'], 's01')

    def test_unknown_result_and_lost_reply_never_retry_or_skip(self):
        for raised in (False, True):
            with self.subTest(raised=raised), environment() as (port, batch, authority, limits):
                token = consent(batch, authority, limits)
                def fail(batch, name):
                    batch.records[name] = 'indeterminate'
                    if raised:
                        raise OSError('lost reply')
                port.after_advance = fail
                value = run.run(batch, authority, limits, token)
                self.assertEqual(port.advances, ['s00'])
                self.assertEqual(value['pending_step'], 's00')
                self.assertEqual(value['run']['confirmed_placements_this_call'], 0)
                self.assertEqual(value['run']['outcomes'][0]['phase'], 'indeterminate')
                self.assertEqual(value['run']['faulted'], raised)

    def test_fully_persisted_lost_acknowledgement_stops_without_redelivery(self):
        with environment() as (port, batch, authority, limits):
            token = consent(batch, authority, limits)
            def fail(batch, name):
                raise OSError('lost after durable retain')
            port.after_advance = fail
            value = run.run(batch, authority, limits, token)
            self.assertEqual(port.advances, ['s00'])
            self.assertEqual(value['placed'], 1)
            self.assertTrue(value['run']['faulted'])
            self.assertEqual(value['run']['outcomes'][0]['phase'], 'placed')

    def test_budget_boundary_returns_original_pending_suffix_without_next_review(self):
        for field in ('calls', 'network_bytes', 'work_steps', 'disk_bytes'):
            with self.subTest(field=field), environment() as (port, batch, authority, limits):
                token = consent(batch, authority, limits)
                def drain(batch, name):
                    setattr(batch.budget, field, 100)
                port.after_advance = drain
                value = run.run(batch, authority, limits, token)
                self.assertEqual(port.advances, ['s00'])
                self.assertEqual(value['run']['stop_reason'], 'aggregate_budget_boundary')
                self.assertEqual(port.reviews, ['s00', 's00'])
                self.assertEqual(len(value['plan']['steps']), 3)

    def test_readonly_revoked_deadline_and_custody_refuse_without_next_dispatch(self):
        for case in ('readonly', 'revoked', 'expired', 'corrupt'):
            with self.subTest(case=case), environment() as (port, batch, authority, limits):
                token = consent(batch, authority, limits)
                if case == 'readonly': batch.writable = False
                elif case == 'revoked': authority.revoked = True
                elif case == 'expired': batch.budget.deadline = 0
                else: batch.corrupt = True
                with self.assertRaises(ValueError):
                    run.run(batch, authority, limits, token)
                self.assertEqual(port.advances, [])

    def test_revocation_between_steps_preserves_completed_prefix(self):
        with environment() as (port, batch, authority, limits):
            token = consent(batch, authority, limits)
            port.after_advance = lambda batch, name: setattr(authority, 'revoked', True)
            with self.assertRaises(ValueError):
                run.run(batch, authority, limits, token)
            self.assertEqual(port.advances, ['s00'])
            self.assertEqual(batch.records, {'s00': 'placed'})

    def test_limits_are_closed_and_increased_budget_is_refused(self):
        for count, timeout in ((0, 1), (33, 1), (True, 1), (1, 0), (1, 60001), (1, True)):
            with self.assertRaises(ValueError): run.Limits(count, timeout)
        with environment() as (port, batch, authority, limits):
            token = consent(batch, authority, limits)
            batch.budget.calls += 1
            with self.assertRaises(ValueError): run.run(batch, authority, limits, token)
            self.assertEqual(port.advances, [])

    def test_complete_output_is_reserved_before_any_child(self):
        with environment() as (port, batch, authority, limits):
            token = consent(batch, authority, limits)
            port.after_review = lambda batch, state: state.update(padding='x' * port.MAX_OUTPUT)
            with self.assertRaises(ValueError): run.run(batch, authority, limits, token)
            self.assertEqual(port.advances, [])
            self.assertEqual(batch.records, {})

    def test_unready_review_has_no_batch_consent(self):
        with environment() as (port, batch, authority, limits):
            port.blocked.add('s00')
            value = run.review_run(batch, authority, limits)
            self.assertIsNone(value['run_review']['confirm_run'])
            self.assertIsNone(value['run_review']['scope'])
            self.assertEqual(port.advances, [])


if __name__ == '__main__':
    unittest.main()
