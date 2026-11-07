"""Dependency-free original portfolios through the actual combined stdio host.

The shared peers speak the existing native wire formats. Enabled labor is an
observed prerequisite; no assignment record or frontier entry is fabricated.
"""
from __future__ import annotations

import copy
import json


class ExistingLaborPortfolioTests:
    def existing_portfolio(self, count=3, *, inventory=False, mixed=False, stable=1,
                           ticks=0, deadline=300, dispatch=True, lose_step=None):
        self.existing_labor_fixture(ids=(42, 43))
        sid, observed = self.existing_labor_observe(max_game_ticks=ticks)
        choices = observed['observation']['workforce_choices']
        self.assertEqual(choices['options'], [])
        self.assertEqual(choices['existing_labor_options'][0]['eligible_enabled_ids'], ['43', '44'])
        recipes = [('wooden_bed', 'BED'), ('wooden_chair', 'CHAIR'),
                   ('wooden_door', 'DOOR'), ('wooden_table', 'TABLE')][:count]
        orders = [{'recipe': recipe, 'amount': 1,
                   **({'objective': {'kind': 'observed_inventory_minimum', 'item_type': kind,
                                    'minimum_stack_units': 1}} if inventory else {})}
                  for recipe, kind in recipes]
        if mixed:
            orders[0].pop('objective', None)
        request = {'kind': 'existing_labor_portfolio', 'expected_observation': observed['observation']['seal'],
                   'units': ['43'], 'labor': 'MINE', 'orders': list(reversed(orders)),
                   'deadline_tick': deadline, 'poll_interval_ticks': 5, 'stable_samples': stable}
        current = self.client.tool('plan', session_id=sid, request=json.dumps(request))
        self.assertTrue(current['ok'], current)
        original = current['original_plan']
        self.assertEqual(len(original['steps']), count)
        self.assertEqual([row['step_id'] for row in current['recovery']['portfolio']], list(range(count)))
        for index, step in enumerate(original['steps']):
            self.assertEqual(step['step_id'], index)
            self.assertEqual(step['action']['kind'], 'create_work_order')
            self.assertEqual(step['depends_on'], [])
            self.assertEqual(step['obligation']['deadline_tick'], deadline)
        self.assertEqual(current['portfolio']['selected_order_step_id'], 0)
        self.assert_no_labor_assignment()
        if dispatch:
            for index in range(count):
                reviewed = self.client.execute_next(current)
                self.assertTrue(reviewed['ok'], reviewed)
                self.assertEqual(reviewed['review']['step_id'], index)
                self.assertEqual(reviewed['review']['idempotency_key'], original['steps'][index]['key'])
                self.assertEqual(reviewed['review']['family'], 'work-orders/1.10')
                if index == lose_step:
                    self.peer.drop_order_reply = True
                current = self.client.execute_next(reviewed)
                if index == lose_step:
                    self.assertFalse(current['ok'], current)
                    return sid, original, current
                self.assertTrue(current['ok'], current)
            self.assertTrue(current['frontier']['all_applied'])
            self.assertEqual(len(self.peer.created_keys), count)
        self.assertFalse(current['original_goal_proven'])
        self.assert_no_labor_assignment()
        return sid, original, current

    def test_existing_portfolio_four_mixed_goals_require_current_workers_and_every_leaf(self):
        sid, original, _ = self.existing_portfolio(count=4, inventory=True, mixed=True)
        self.peer.orders[100].update(remaining=0, status=1)
        self.peer.stocks.update(CHAIR=1, DOOR=1)
        self.peer.goal_tick(self.base.TICK + 5)
        partial = self.cohort(sid)
        self.assertFalse(partial['original_goal_proven'])
        self.assertEqual(partial['current_parent_goal']['current_truth'], 'false')
        self.peer.stocks['TABLE'] = 1
        self.peer.goal_tick(self.base.TICK + 10)
        complete = self.cohort(sid)
        self.assertTrue(complete['original_goal_proven'], complete)
        self.assertTrue(complete['agent_turn']['briefing']['original_goal_proven'])
        self.assertEqual(complete['original_plan'], original)
        self.assertEqual([r['step_id'] for r in complete['current_parent_goal']['selected_anchors']], [0, 1, 2, 3])
        self.assertEqual(self.peer.workforce_selections[0], [42, 43])
        self.assertTrue(all(ids == [42] for ids in self.peer.workforce_selections[1:]))
        first = [r['goal']['first_satisfied_anchor'] for r in complete['portfolio']['orders']]
        self.peer.workforce_labor = False
        self.peer.workforce_sequence = 4
        self.peer.goal_tick(self.base.TICK + 15)
        reversed_labor = self.cohort(sid)
        self.assertFalse(reversed_labor['original_goal_proven'])
        self.assertEqual(reversed_labor['current_parent_goal']['labor'][0]['current_truth'], 'false')
        self.assertEqual([r['goal']['first_satisfied_anchor'] for r in reversed_labor['portfolio']['orders']], first)
        self.assert_no_labor_assignment()

    def test_existing_portfolio_second_order_rechecks_labor_before_native_preparation(self):
        sid, original, current = self.existing_portfolio(count=2, dispatch=False)
        current = self.client.execute_next(self.client.execute_next(current))
        self.assertTrue(current['ok'], current)
        self.assertEqual(len(self.peer.created_keys), 1)
        self.peer.workforce_labor = False
        self.peer.workforce_sequence = 4
        before = len(self.peer.plans)
        refused = self.client.execute_next(current)
        self.assertFalse(refused['ok'], refused)
        self.assertEqual(len(self.peer.plans), before)
        self.assertEqual(len(self.peer.created_keys), 1)
        retained = self.client.tool('query', session_id=sid)
        self.assertTrue(retained['ok'], retained)
        self.assertEqual(retained['original_plan'], original)
        self.assertFalse(retained['frontier']['all_applied'])
        self.assert_no_labor_assignment()

    def test_existing_portfolio_post_review_change_blocks_second_original_dispatch(self):
        sid, _, current = self.existing_portfolio(count=2, dispatch=False)
        current = self.client.execute_next(self.client.execute_next(current))
        self.assertTrue(current['ok'], current)
        reviewed = self.client.execute_next(current)
        self.assertTrue(reviewed['ok'], reviewed)
        self.assertEqual(reviewed['review']['step_id'], 1)
        def change():
            self.peer.workforce_labor = False
            self.peer.workforce_sequence = 4
        self.peer.after_order_read = change
        refused = self.client.execute_next(reviewed)
        self.assertFalse(refused['ok'], refused)
        self.assertIsNone(self.peer.after_order_read)
        self.assertEqual(len(self.peer.created_keys), 1)
        self.assertEqual(sum(op == 'CommitOrder' for _, op in self.peer.calls), 1)
        self.assert_no_labor_assignment()

    def test_existing_portfolio_lost_middle_insert_reopens_exact_zero_based_custody(self):
        self.enable_clock()
        self.enable_inventory_clock()
        sid, original, lost = self.existing_portfolio(count=4, inventory=True, ticks=5, lose_step=1)
        recovery = lost['retained_reference']['recovery']
        self.assertEqual([row['step_id'] for row in recovery['portfolio']], [0, 1, 2, 3])
        self.assertEqual(len({r['clock_journal_id'] for r in recovery['portfolio']}), 4)
        self.assertEqual(len({r['inventory_clock_journal_id'] for r in recovery['portfolio']}), 4)
        queried = self.client.tool('query', session_id=sid)
        self.assertTrue(queried['ok'], queried)
        workforce = queried['native_custody']['workforce_journal_id']
        self.client.close(crash=True)
        env = self.base.reopen_environment(self.env, recovery, workforce)
        files, calls = self.base.custody_files(self.directory), list(self.peer.calls)
        self.client = self.base.stdio.Client(self.directory, {}, suffix='existing-portfolio-offline', env=env)
        offline = self.client.open(mode='offline', max_wall_millis=60000)
        self.assertTrue(offline['ok'], offline)
        self.assertEqual(offline['original_plan'], original)
        self.assertEqual(offline['recovery']['portfolio'], recovery['portfolio'])
        self.assertEqual(files, self.base.custody_files(self.directory))
        self.assertEqual(calls, self.peer.calls)
        self.assertFalse(offline['original_goal_proven'])
        self.client.close()
        self.client = self.base.stdio.Client(self.directory, {}, suffix='existing-portfolio-resume', env=env)
        opened = self.client.open(max_wall_millis=60000, max_game_ticks=5)
        self.assertTrue(opened['ok'], opened)
        sid = opened['agent_turn']['session_id']
        current = self.client.tool('wait', session_id=sid, max_wall_millis=60000)
        self.assertTrue(current['ok'], current)
        self.assertEqual(len(self.peer.created_keys), 2)
        for index in (2, 3):
            reviewed = self.client.execute_next(current)
            self.assertTrue(reviewed['ok'], reviewed)
            self.assertEqual(reviewed['review']['step_id'], index)
            current = self.client.execute_next(reviewed)
            self.assertTrue(current['ok'], current)
        self.assertEqual(current['original_plan'], original)
        self.assertEqual(len(set(self.peer.created_keys)), 4)
        self.assert_no_labor_assignment()

    def test_existing_portfolio_both_clocks_keep_zero_order_target_and_original_deadline(self):
        self.enable_clock()
        self.enable_inventory_clock()
        sid, original, _ = self.existing_portfolio(count=2, inventory=True, stable=3, ticks=5,
                                                  deadline=self.base.TICK + 15)
        self.peer.stocks.update(BED=1, CHAIR=1)
        self.peer.goal_tick(self.base.TICK + 5)
        first = self.cohort(sid)
        self.assertFalse(first['original_goal_proven'])
        self.peer.orders[100]['retired'] = True
        selected = self.client.tool('query', session_id=sid, order_step_id=0)
        self.assertTrue(selected['ok'], selected)
        request = {'kind': 'inventory_run', 'order_step_id': 0,
                   'original_paired_seal': selected['inventory_clock']['original_paired_seal'],
                   'game_ticks': 5, 'wall_millis': 1000}
        reviewed = self.client.tool('plan', session_id=sid, request=json.dumps(request))
        self.assertTrue(reviewed['ok'], reviewed)
        advanced = self.client.tool('commit', session_id=sid, scope='inventory_run', order_step_id=0,
            paired_seal=reviewed['inventory_clock']['active_run']['review_seal'], confirmed=True)
        self.assertTrue(advanced['ok'], advanced)
        second = self.cohort(sid)
        self.assertFalse(second['original_goal_proven'])
        reviewed = self.prepare_run(sid, 1)
        advanced = self.client.tool('commit', session_id=sid, scope='run', order_step_id=1,
            paired_seal=reviewed['clock']['active_run']['review_seal'], confirmed=True)
        self.assertTrue(advanced['ok'], advanced)
        complete = self.cohort(sid)
        self.assertTrue(complete['original_goal_proven'], complete)
        self.assertEqual(complete['original_plan'], original)
        self.assertEqual(self.peer.order_tick, self.base.TICK + 15)
        self.assertEqual(len(self.peer.inventory_clock_commits), 1)
        self.assertEqual(len(self.peer.clock_commits), 1)
        self.assertEqual(self.base.run_wire.plan(self.peer.clock_plans[self.peer.clock_commits[0]])['before']['order_id'], 101)
        self.assertEqual([r['goal']['deadline_tick'] for r in complete['portfolio']['orders']], [self.base.TICK + 15] * 2)
        self.assert_no_labor_assignment()

    def test_existing_portfolio_shifted_recovery_ids_cannot_select_legacy_storage_slots(self):
        _, _, created = self.existing_portfolio(count=2)
        recovery = copy.deepcopy(created['recovery'])
        for row in recovery['portfolio']:
            row['step_id'] += 1
        self.client.close()
        env = self.base.reopen_environment(self.env, recovery, created['native_custody']['workforce_journal_id'])
        before, calls = self.base.custody_files(self.directory), list(self.peer.calls)
        self.client = self.base.stdio.Client(self.directory, {}, suffix='shifted-layout', env=env)
        refused = self.client.open(mode='offline', max_wall_millis=60000)
        self.assertFalse(refused['ok'], refused)
        self.assertEqual(self.base.custody_files(self.directory), before)
        self.assertEqual(self.peer.calls, calls)
        self.assert_no_labor_assignment()
