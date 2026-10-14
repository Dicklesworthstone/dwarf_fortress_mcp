"""Real run owner/plan codecs with explicit original-custody and step ports.

These regressions model the mandatory construction gate's advance_allowed=False,
not the older permissive fixture. They are not native transport/game evidence.
"""
from copy import deepcopy
import unittest

from furniture_plan import canonical
import route_furniture_run as run
from test_route_furniture_run import consent, environment


STATUSES = ('construction_pending', 'construction_unestablished', 'construction_blocked',
            'route_blocked', 'route_unestablished')


def block(port, status, slot='s01'):
    def after_review(batch, state):
        if state['next_step'] == slot:
            state.update(advance_allowed=False, expected_plan=None, confirm_review=None,
                         route_constraints_checked_this_call=True,
                         route_frontier={'status': status, 'evidence_digest': 'd' * 64,
                             'construction_prefix': {'status': 'pending', 'rows': [
                                 {'slot': 's00', 'condition': 'construction_job_present'}]}})
    port.after_review = after_review


class ReadinessRunTests(unittest.TestCase):
    def test_unready_review_returns_complete_blocker_without_consent(self):
        for status in STATUSES:
            with self.subTest(status=status), environment() as (port, batch, auth, limits):
                block(port, status, 's00')
                result = run.review_run(batch, auth, limits)
                self.assertFalse(result['advance_allowed'])
                self.assertEqual(result['route_frontier']['status'], status)
                self.assertIsNone(result['run_review']['confirm_run'])
                self.assertIsNone(result['run_review']['scope'])
                self.assertEqual(len(result['plan']['steps']), 3)
                self.assertEqual(port.advances, [])

    def test_first_unready_run_is_read_only_and_does_not_validate_consent(self):
        for status in STATUSES:
            with self.subTest(status=status), environment() as (port, batch, auth, limits):
                token = consent(batch, auth, limits)
                block(port, status, 's00')
                result = run.run(batch, auth, limits, token)
                self.assertEqual(result['run']['stop_reason'], status)
                self.assertEqual(result['run']['outcomes'], [])
                self.assertFalse(result['run']['confirmation_validated'])
                self.assertIsNone(result['run']['scope'])
                self.assertFalse(result['run']['faulted'])
                self.assertFalse(result['advance_allowed'])
                self.assertEqual(port.advances, [])

    def test_pending_after_progress_retains_the_exact_blocking_observation(self):
        for status in STATUSES:
            with self.subTest(status=status), environment() as (port, batch, auth, limits):
                token = consent(batch, auth, limits)
                block(port, status)
                result = run.run(batch, auth, limits, token)
                self.assertEqual(result['run']['stop_reason'], status)
                self.assertEqual(port.advances, ['s00'])
                self.assertEqual(result['placed'], 1)
                self.assertEqual(result['next_step'], 's01')
                self.assertEqual(result['run']['confirmed_placements_this_call'], 1)
                self.assertEqual(result['route_frontier']['evidence_digest'], 'd' * 64)
                self.assertEqual(result['route_frontier']['construction_prefix']['rows'][0]['slot'], 's00')
                self.assertTrue(result['route_constraints_checked_this_call'])
                self.assertFalse(result['advance_allowed'])
                self.assertFalse(result['run']['faulted'])
                self.assertTrue(result['run']['confirmation_validated'])
                self.assertEqual(len(result['run']['scope']['approved_steps']), 3)

    def test_unfinished_constructor_can_later_finish_but_old_consent_cannot_resume(self):
        with environment() as (port, batch, auth, limits):
            original = canonical(batch.plan.json())
            token = consent(batch, auth, limits)
            block(port, 'construction_pending')
            run.run(batch, auth, limits, token)
            port.after_review = lambda batch, state: None
            batch.budget = run.make_budget(limits)
            with self.assertRaises(ValueError):
                run.run(batch, auth, limits, token)
            self.assertEqual(port.advances, ['s00'])
            fresh = consent(batch, auth, limits)
            self.assertNotEqual(fresh, token)
            result = run.run(batch, auth, limits, fresh)
            self.assertEqual(result['run']['stop_reason'], 'all_placed')
            self.assertEqual(port.advances, ['s00', 's01', 's02'])
            self.assertEqual(canonical(batch.plan.json()), original)

    def test_review_failure_after_progress_retains_outcomes_but_no_cached_readiness(self):
        with environment() as (port, batch, auth, limits):
            token = consent(batch, auth, limits)
            def fail(batch, state):
                if state['next_step'] == 's01':
                    raise OSError('private token/path must not appear in result')
            port.after_review = fail
            result = run.run(batch, auth, limits, token)
            self.assertEqual(result['run']['stop_reason'], 'review_failed')
            self.assertTrue(result['run']['faulted'])
            self.assertEqual(result['run']['advance_invocations_this_call'], 1)
            self.assertEqual(port.advances, ['s00'])
            self.assertNotIn('route_frontier', result)
            self.assertFalse(result['route_constraints_checked_this_call'])
            self.assertFalse(result['advance_allowed'])
            self.assertNotIn(b'private token', canonical(result))

    def test_first_read_failure_still_raises_and_never_creates_a_child(self):
        with environment() as (port, batch, auth, limits):
            def fail(batch, state):
                raise OSError('source unavailable')
            port.after_review = fail
            with self.assertRaises(OSError):
                run.run(batch, auth, limits, 'f' * 64)
            self.assertEqual(port.advances, [])
            self.assertEqual(batch.records, {})

    def test_changed_or_foreign_review_binding_never_creates_a_child(self):
        for key, value in (('inventory_digest', 'e' * 64), ('batch_id', 'a' * 64),
                           ('review_digest', 'f' * 64), ('next_step', 's02'),
                           ('placed', 2), ('plan_digest', '0' * 64),
                           ('inventory_verified', False)):
            with self.subTest(key=key), environment() as (port, batch, auth, limits):
                token = consent(batch, auth, limits)
                port.after_review = lambda b, state: state.update({key: value})
                with self.assertRaises(ValueError):
                    run.run(batch, auth, limits, token)
                self.assertEqual(port.advances, [])

    def test_changed_custody_during_review_is_not_a_pending_construction(self):
        with environment() as (port, batch, auth, limits):
            token = consent(batch, auth, limits)
            port.after_review = lambda b, state: b.records.update(s00='unknown')
            with self.assertRaises(ValueError):
                run.run(batch, auth, limits, token)
            self.assertEqual(port.advances, [])
            self.assertEqual(batch.records, {'s00': 'unknown'})

    def test_contradictory_status_or_confirmation_is_not_readiness(self):
        for mutation in ('unknown', 'ready_false', 'pending_seal', 'missing_seal'):
            with self.subTest(mutation=mutation), environment() as (port, batch, auth, limits):
                token = consent(batch, auth, limits)
                def change(batch, state):
                    if mutation == 'unknown': state['route_frontier']['status'] = 'future_success'
                    elif mutation == 'ready_false': state['advance_allowed'] = False
                    elif mutation == 'pending_seal': state['route_frontier']['status'] = 'construction_pending'
                    else: state['confirm_review'] = None
                port.after_review = change
                with self.assertRaises(ValueError):
                    run.run(batch, auth, limits, token)
                self.assertEqual(port.advances, [])

    def test_local_final_audit_never_manufactures_execution_permission(self):
        with environment(3, 1) as (port, batch, auth, limits):
            token = consent(batch, auth, limits)
            result = run.run(batch, auth, limits, token)
            self.assertEqual(result['status'], 'ready')  # Original journal eligibility only.
            self.assertFalse(result['advance_allowed'])
            self.assertIsNone(result['expected_plan'])
            self.assertIsNone(result['confirm_review'])
            self.assertTrue(result['execution_review_required'])
            self.assertFalse(result['route_constraints_checked_this_call'])

    def test_pending_report_still_requires_current_authority_and_custody(self):
        for change in ('revoke', 'corrupt'):
            with self.subTest(change=change), environment() as (port, batch, auth, limits):
                token = consent(batch, auth, limits)
                block(port, 'construction_pending')
                original = port.after_review
                def fail(batch, state):
                    original(batch, state)
                    if state['next_step'] == 's01':
                        if change == 'revoke': auth.revoked = True
                        else: batch.corrupt = True
                port.after_review = fail
                with self.assertRaises(ValueError):
                    run.run(batch, auth, limits, token)
                self.assertEqual(port.advances, ['s00'])

    def test_every_original_suffix_remains_visible_when_construction_is_pending(self):
        for count in range(1, 32):
            with self.subTest(prefix=count), environment(32, 32) as (port, batch, auth, limits):
                batch.records = {f's{i:02}': 'placed' for i in range(count)}
                before = deepcopy(batch.records)
                block(port, 'construction_pending', f's{count:02}')
                result = run.review_run(batch, auth, limits)
                self.assertEqual(result['placed'], count)
                self.assertEqual(result['next_step'], f's{count:02}')
                self.assertEqual(len(result['plan']['steps']), 32)
                self.assertIsNone(result['run_review']['confirm_run'])
                self.assertEqual(batch.records, before)
                self.assertEqual(port.advances, [])


if __name__ == '__main__':
    unittest.main()
