"""Existing labor through the actual combined stdio server and native wire doubles.

The workforce starts enabled without an assignable detail. No fake assignment
receipt or invented Applied frontier is supplied to the real executor.
"""
from __future__ import annotations

import json


class ExistingLaborTests:
    """Mixed into CombinedProduction to reuse its real subprocess and custody."""

    def existing_labor_fixture(self, ids=(42,)):
        self.peer.citizen_ids = list(ids)
        self.peer.existing_labor_ready = True
        self.existing_source = {'eligible': True, 'historical_delta': 0}

        def capture(raw):
            # Already enabled labor need not come from a uniquely assignable
            # detail or automatic professions. The ordinary setter cannot
            # establish a new assignment from this capture.
            raw['automatic'] = False
            raw['details'] = []
            raw['labor_keys'] = ['MINE', 'CARPENTRY']
            for unit in raw['units']:
                unit['eligible'] = self.existing_source['eligible']
                unit['historical_id'] += self.existing_source['historical_delta']
                unit['labors'] = [int(key == 'MINE' and self.peer.workforce_labor is not False)
                                  for key in raw['labor_keys']]

        self.peer.workforce_capture_hook = capture

    def existing_labor_observe(self, max_game_ticks=0):
        opened = self.client.open(max_wall_millis=60000, max_game_ticks=max_game_ticks)
        self.assertTrue(opened['ok'], opened)
        sid = opened['agent_turn']['session_id']
        observed = self.client.tool('observe', session_id=sid)
        self.assertTrue(observed['ok'], observed)
        self.existing_workforce_custody = {
            path: info for path, info in self.inspection_custody().items()
            if path.startswith('workforce/')
        }
        self.assertTrue(self.existing_workforce_custody)
        return sid, observed

    def existing_labor_request(self, observed, *, stable_samples=1, inventory=False):
        request = {'kind': 'existing_labor', 'expected_observation': observed['observation']['seal'],
                   'units': ['43'], 'labor': 'MINE', 'recipe': 'wooden_bed', 'amount': 1,
                   'deadline_tick': 300, 'poll_interval_ticks': 5, 'stable_samples': stable_samples}
        if inventory:
            request['objective'] = {'kind': 'observed_inventory_minimum', 'item_type': 'BED',
                                    'minimum_stack_units': 1}
        return request

    def existing_labor_compile(self, sid, observed, **options):
        planned = self.client.tool('plan', session_id=sid,
                                   request=json.dumps(self.existing_labor_request(observed, **options)))
        self.assertTrue(planned['ok'], planned)
        original = planned['original_plan']
        self.assertEqual(len(original['steps']), 1)
        step = original['steps'][0]
        self.assertEqual(step['step_id'], 0)
        self.assertEqual(step['action']['kind'], 'create_work_order')
        self.assertEqual(step['depends_on'], [])
        self.assertFalse(planned['original_goal_proven'])
        self.assertIsNone(planned['review'])
        self.assert_no_labor_assignment()
        return original, planned

    def existing_labor_commit(self, sid, planned):
        original = planned['original_plan']
        prepared = self.client.execute_next(planned)
        self.assertTrue(prepared['ok'], prepared)
        review = prepared['review']
        self.assertEqual(review['step_id'], 0)
        self.assertEqual(review['family'], 'work-orders/1.10')
        self.assertEqual(review['original_plan_digest'], original['plan_digest'])
        self.assertEqual(review['idempotency_key'], original['steps'][0]['key'])
        self.assertTrue(review['confirmation_required'])
        self.assertFalse(self.peer.order_created)
        committed = self.client.execute_next(prepared)
        self.assertTrue(committed['ok'], committed)
        self.assertTrue(committed['frontier']['all_applied'])
        self.assertFalse(committed['original_goal_proven'])
        self.assert_existing_order(committed, original)
        return committed

    def assert_no_labor_assignment(self):
        self.assertFalse(self.peer.workforce_applied)
        self.assertIsNone(self.peer.workforce_plan)
        self.assertFalse(any(op in ('PrepareAssignment', 'CommitAssignment', 'QueryAssignment')
                             for _, op in self.peer.calls), self.peer.calls)
        if hasattr(self, 'existing_workforce_custody'):
            current = {path: info for path, info in self.inspection_custody().items()
                       if path.startswith('workforce/')}
            self.assertEqual(current, self.existing_workforce_custody,
                             'read-only prerequisites cannot manufacture assignment custody')

    def assert_existing_order(self, view, original):
        self.assertEqual(view['original_plan'], original)
        entries = view['frontier']['entries']
        self.assertEqual(len(entries), 1)
        self.assertEqual(entries[0]['step_id'], 0)
        self.assertEqual(entries[0]['idempotency_key'], original['steps'][0]['key'])
        self.assertEqual(entries[0]['phase'], 'applied')
        self.assertTrue(entries[0]['dispatch_attempted'])
        self.assertTrue(entries[0]['receipt_digest'])
        self.assertEqual(sum(op == 'CommitOrder' for _, op in self.peer.calls), 1)
        self.assert_no_labor_assignment()

    def existing_labor_sample(self, sid, tick, original, *, inventory=False):
        self.peer.goal_tick(tick)
        current = self.client.tool('wait', session_id=sid, max_wall_millis=60000)
        self.assertTrue(current['ok'], current)
        selected = current['selected_inventory_goal' if inventory else 'selected_order_goal']
        self.assertEqual(selected['step_id'], 0)
        self.assertEqual(selected['deadline_tick'], 300)
        parent = current['current_parent_goal']
        self.assertEqual(parent['anchor'], current['anchor'])
        self.assertEqual(parent['selected_anchor'], selected['anchor'])
        self.assertFalse(parent['historical_labor_receipt_is_current_evidence'])
        self.assertFalse(parent['restored_from_history'])
        self.assertFalse(current['current_state_proven'])
        self.assertFalse(current['automatic_redispatch'])
        self.assertEqual(parent['labor'], [{'unit_id': '43', 'labor': 'MINE',
            'requested_enabled': True, 'observed_enabled': self.peer.workforce_labor is not False,
            'current_truth': 'false' if self.peer.workforce_labor is False else 'true'}])
        self.assert_existing_order(current, original)
        return current

    def test_existing_labor_discovery_subset_order_and_reviewed_clock(self):
        self.enable_clock()
        self.peer.complete_order_on_clock = True
        self.existing_labor_fixture(ids=(42, 43))
        sid, observed = self.existing_labor_observe(max_game_ticks=5)
        choices = observed['observation']['workforce_choices']
        self.assertEqual(choices['options'], [])
        self.assertFalse(choices['source']['automatic'])
        self.assertEqual(choices['existing_labor_options'], [{
            'labor': 'MINE', 'eligible_enabled_ids': ['43', '44'],
            'request_kind': 'existing_labor', 'assignment_required': False,
            'candidates_epistemic_state': 'certified_derived', 'productivity_proven': False}])
        self.assertFalse(choices['mutation_authority'])
        self.assertFalse(choices['coverage']['existing_labor']['detail_shape_required'])
        original, planned = self.existing_labor_compile(sid, observed)
        current = self.existing_labor_commit(sid, planned)
        self.assertEqual(self.peer.workforce_selections[0], [42, 43])
        self.assertTrue(all(ids == [42] for ids in self.peer.workforce_selections[1:]))
        prepared = self.prepare_clock(sid, current)
        self.assertTrue(prepared['ok'], prepared)
        run = prepared['clock']['active_run']
        advanced = self.client.tool('commit', session_id=sid, scope='run',
                                    paired_seal=run['review_seal'], confirmed=True)
        self.assertTrue(advanced['ok'], advanced)
        self.assertEqual(len(self.peer.clock_commits), 1)
        self.assertEqual(self.peer.order_tick, 125)
        achieved = self.existing_labor_sample(sid, 125, original)
        self.assertTrue(achieved['original_goal_proven'])
        self.assertTrue(achieved['selected_order_goal']['proven_at_anchor'])
        self.assertFalse(achieved['runtime_admitted'])

    def test_existing_labor_original_stability_and_current_reversal(self):
        self.existing_labor_fixture()
        sid, observed = self.existing_labor_observe()
        original, planned = self.existing_labor_compile(sid, observed, stable_samples=2, inventory=True)
        self.existing_labor_commit(sid, planned)
        for tick in (125, 125):
            pending = self.existing_labor_sample(sid, tick, original, inventory=True)
            self.assertEqual(pending['selected_inventory_goal']['current_truth'], 'true')
            self.assertEqual(pending['current_parent_goal']['current_truth'], 'true')
            self.assertFalse(pending['original_goal_proven'])
        self.peer.order_retired = True
        achieved = self.existing_labor_sample(sid, 130, original, inventory=True)
        self.assertTrue(achieved['original_goal_proven'])
        first = achieved['selected_inventory_goal']['first_satisfied_anchor']
        self.assertIsNotNone(first)
        self.peer.workforce_labor = False
        self.peer.workforce_sequence = 4
        reversed_labor = self.existing_labor_sample(sid, 135, original, inventory=True)
        self.assertEqual(reversed_labor['current_parent_goal']['current_truth'], 'false')
        self.assertFalse(reversed_labor['original_goal_proven'])
        self.assertTrue(reversed_labor['selected_inventory_goal']['proven_at_anchor'])
        self.assertEqual(reversed_labor['selected_inventory_goal']['first_satisfied_anchor'], first)

    def test_existing_labor_changed_before_prepare_blocks_native_reservation(self):
        self.existing_labor_fixture()
        sid, observed = self.existing_labor_observe()
        original, planned = self.existing_labor_compile(sid, observed)
        self.peer.workforce_labor = False
        self.peer.workforce_sequence = 4
        refused = self.client.execute_next(planned)
        self.assertFalse(refused['ok'], refused)
        self.assertFalse(any(op in ('PrepareOrder', 'CommitOrder') for _, op in self.peer.calls))
        self.assertFalse(self.peer.order_created)
        current = self.client.tool('query', session_id=sid)
        self.assertTrue(current['ok'], current)
        self.assertEqual(current['original_plan'], original)
        self.assertFalse(current['frontier']['all_applied'])
        self.assert_no_labor_assignment()

    def test_existing_labor_changed_after_review_blocks_native_insertion(self):
        self.existing_labor_fixture()
        sid, observed = self.existing_labor_observe()
        _, planned = self.existing_labor_compile(sid, observed)
        reviewed = self.client.execute_next(planned)
        self.assertTrue(reviewed['ok'], reviewed)
        self.peer.workforce_labor = False
        self.peer.workforce_sequence = 4
        refused = self.client.execute_next(reviewed)
        self.assertFalse(refused['ok'], refused)
        self.assertEqual(sum(op == 'PrepareOrder' for _, op in self.peer.calls), 1)
        self.assertNotIn(('dfmcp_work_orders_v1_10', 'CommitOrder'), self.peer.calls)
        self.assertFalse(self.peer.order_created)
        self.assert_no_labor_assignment()

    def test_existing_labor_reacquires_prerequisite_after_order_queue_capture(self):
        self.existing_labor_fixture()
        sid, observed = self.existing_labor_observe()
        _, planned = self.existing_labor_compile(sid, observed)
        reviewed = self.client.execute_next(planned)
        self.assertTrue(reviewed['ok'], reviewed)
        prior_reads = sum(op == 'ObserveWorkforce' for _, op in self.peer.calls)

        def change_after_queue_read():
            self.peer.workforce_labor = False
            self.peer.workforce_sequence = 4

        # Change labor after the commit's fresh queue reply is captured. The
        # post-marker evidence refresh must still acquire the current labor
        # prerequisite before allowing the native order setter.
        self.peer.after_order_read = change_after_queue_read
        refused = self.client.execute_next(reviewed)
        self.assertFalse(refused['ok'], refused)
        self.assertIsNone(self.peer.after_order_read)
        self.assertGreaterEqual(sum(op == 'ObserveWorkforce' for _, op in self.peer.calls) - prior_reads, 1)
        self.assertNotIn(('dfmcp_work_orders_v1_10', 'CommitOrder'), self.peer.calls)
        self.assertFalse(self.peer.order_created)
        self.assert_no_labor_assignment()

    def test_existing_labor_unavailable_ineligible_and_replaced_identity_refuse(self):
        self.existing_labor_fixture()
        sid, observed = self.existing_labor_observe()
        for labor, eligible in (('BREW', True), ('MINE', False)):
            with self.subTest(labor=labor, eligible=eligible):
                self.existing_source['eligible'] = eligible
                self.peer.workforce_sequence = (self.peer.workforce_sequence or 3) + 1
                observed = self.client.tool('observe', session_id=sid)
                self.assertTrue(observed['ok'], observed)
                choices = observed['observation']['workforce_choices']['existing_labor_options']
                self.assertNotIn(labor, [choice['labor'] for choice in choices])
                request = self.existing_labor_request(observed)
                request['labor'] = labor
                refused = self.client.tool('plan', session_id=sid,
                    request=json.dumps(request))
                self.assertFalse(refused['ok'], refused)
                self.assertFalse(any(op.startswith(('Prepare', 'Commit')) for _, op in self.peer.calls))
        self.existing_source['eligible'] = True
        self.peer.workforce_sequence += 1
        observed = self.client.tool('observe', session_id=sid)
        self.assertTrue(observed['ok'], observed)
        _, planned = self.existing_labor_compile(sid, observed)
        self.existing_source['historical_delta'] = 100
        self.peer.workforce_sequence += 1
        refused = self.client.execute_next(planned)
        self.assertFalse(refused['ok'], refused)
        self.assertFalse(any(op in ('PrepareOrder', 'CommitOrder') for _, op in self.peer.calls))
        self.assertFalse(self.peer.order_created)
        self.assert_no_labor_assignment()

    def test_existing_labor_exact_offline_and_reconcile_recovery_without_assignment(self):
        self.existing_labor_fixture()
        sid, observed = self.existing_labor_observe()
        original, planned = self.existing_labor_compile(sid, observed, inventory=True)
        self.existing_labor_commit(sid, planned)
        achieved = self.existing_labor_sample(sid, 125, original, inventory=True)
        self.assertTrue(achieved['original_goal_proven'])
        first = achieved['selected_inventory_goal']['first_satisfied_anchor']
        closed = self.client.tool('cancel', session_id=sid, scope='session')
        self.assertTrue(closed['ok'], closed)
        calls, before = list(self.peer.calls), self.inspection_custody()
        offline = self.existing_labor_reopen(achieved, 'offline')
        self.assertTrue(offline['ok'], offline)
        self.assertEqual(self.peer.calls, calls)
        self.assertEqual(self.inspection_custody(), before)
        self.assert_existing_order(offline, original)
        self.assertFalse(offline['original_goal_proven'])
        self.assertIsNone(offline['current_parent_goal'])
        self.assertFalse(offline['selected_inventory_goal']['proven_at_anchor'])
        self.assertEqual(offline['selected_inventory_goal']['first_satisfied_anchor'], first)
        self.assertIsNone(offline['review'])
        reconciled = self.existing_labor_reopen(offline, 'reconcile')
        self.assertTrue(reconciled['ok'], reconciled)
        self.assert_existing_order(reconciled, original)
        self.assertFalse(reconciled['original_goal_proven'])
        current = self.existing_labor_sample(reconciled['agent_turn']['session_id'], 130,
                                             original, inventory=True)
        self.assertTrue(current['original_goal_proven'])
        self.assertEqual(current['selected_inventory_goal']['first_satisfied_anchor'], first)
