"""Read-only original-production inspection through the real stdio facade.

The coherent operations peer is an authentic wire-format double. No result in
this component probe establishes native qualification or order-attributed output.
"""
from __future__ import annotations

import copy
import json
import struct


ITEM_NAMESPACE = 2 << 40
BUILDING_NAMESPACE = 1 << 40


def text(value):
    return struct.pack('>H', len(value)) + value


def reference(value):
    return b'\x00' if value is None else b'\x01' + struct.pack('>I', value)


def operations_capture(tick, stack=1):
    """Two jobs share rotten wood inside a forbidden bin; seven free logs remain."""
    year, annual = divmod(tick, 403200)
    jobs = (b'DFMJ1200' + struct.pack('>IIBiI', year, annual, 1, 4, 100)
            + text(b'region1') + struct.pack('>I', 2))
    for ident, suspended, worker, attachments, filters in (
            (3, True, None, 2, 2), (4, False, 42, 1, 1)):
        jobs += (struct.pack('>Ii', ident, 69) + text(b'ConstructBed') + text(b'')
                 + bytes((suspended, False)) + struct.pack('>iii', 1, 1, 1)
                 + reference(worker) + reference(7)
                 + struct.pack('>iII', -1, attachments, filters))
    raw = (b'DFMO1300' + struct.pack('>I', len(jobs)) + jobs
           + struct.pack('>III', 100, 100, 1)
           + struct.pack('>Ii', 7, 4) + text(b'Workshop')
           + struct.pack('>iiiiiii', 1, 1, 3, 3, 1, 1, 3))
    items = [
        (2, 0, b'WOOD', 1, 16, 3),
        (3, 6, b'BIN', 1, 65, None),
        (4, 0, b'WOOD', 7, 64, None),
        (5, 0, b'WOOD', 99, 72, None),
    ]
    if stack > 0:
        items.append((6, 5, b'BED', stack, 64, None))
    raw += struct.pack('>I', len(items))
    for ident, kind, key, units, flags, container in items:
        raw += (struct.pack('>Ii', ident, kind) + text(key)
                + struct.pack('>iiiIiiiI', -1, 420, 0, units, 1, 1, 1, flags)
                + reference(container) + reference(None))
    attachments = ((3, 2, 0, 0), (3, 2, 1, 0), (4, 2, 0, 0))
    raw += struct.pack('>I', len(attachments))
    return raw + b''.join(struct.pack('>IIii', *item) for item in attachments)


def envelope(query, anchor=None):
    request = {'schema': 'dfmcp.query/1', 'query': query}
    if anchor is not None:
        request['expected_anchor'] = anchor
    return request


class InspectionTests:
    """Mixed into CombinedProduction so its existing subprocess fixture is reused."""

    def inspection_request(self, sid, request, *, order_step_id=None):
        before, calls = self.inspection_custody(), list(self.peer.calls)
        arguments = {'session_id': sid,
                     'request': request if isinstance(request, str) else json.dumps(request)}
        if order_step_id is not None:
            arguments['order_step_id'] = order_step_id
        result = self.client.tool('query', **arguments)
        self.assertEqual(self.peer.calls, calls, 'inspection must not read or mutate a native peer')
        self.assertEqual(self.inspection_custody(), before, 'inspection must not rewrite custody')
        return result

    def assert_inspection(self, result, observed, *, inventory=False):
        self.assertTrue(result['ok'], result)
        selected = observed['selected_inventory_goal' if inventory else 'selected_order_goal']
        source = result['production_inspection_source']
        self.assertEqual(source['scope'], 'captured_operations')
        self.assertEqual(source['attribution'], 'not_order_attributed')
        self.assertEqual(source['order_step_id'], 1)
        self.assertEqual(source['anchor'], selected['anchor'])
        self.assertEqual(source['source_digest'], selected['source_digest'])
        self.assertEqual(source['parent_anchor'], observed['anchor'])
        self.assertNotEqual(source['anchor']['state_hash'], source['parent_anchor']['state_hash'])
        self.assertNotEqual(source['source_digest'], observed['current_parent_goal']['source_digest'])
        report = result['production_inspection']
        self.assertEqual(report['schema'], 'dfmcp.query.result/1')
        self.assertEqual(report['anchor'], source['anchor'])
        self.assertEqual(report['source_digest'], source['source_digest'])
        self.assertFalse(report['mutation_authority'])
        self.assertFalse(report['commit_compatible'])
        self.assertEqual(result['original_plan'], observed['original_plan'])
        return report

    def assert_inspection_refused(self, result):
        self.assertFalse(result['ok'], result)
        self.assertIs(result['game_mutation_dispatched'], False)
        self.assertIsNone(result.get('production_inspection'))
        retained = result.get('retained_reference', {})
        self.assertIsNone(retained.get('production_inspection_source'))

    def assert_observed_stock_aggregate(self, report):
        self.assertEqual(report['kind'], 'aggregate')
        self.assertEqual(report['epistemic_state'], 'inferred')
        self.assertEqual(report['stock_semantics'], 'observed_stock_not_usable_supply')
        self.assertEqual(report['eligibility'], 'not_assessed')
        self.assertFalse(report['reservation_created'])
        self.assertFalse(report['original_goal_evidence'])
        self.assertFalse(report['mutation_authority'])
        self.assertFalse(report['commit_compatible'])
        self.assertFalse(report['coverage']['absence_proven'])
        self.assertFalse(report['truncated'])
        self.assertIsNone(report['continuation'])
        self.assertEqual(len(report['query_digest']), 64)

    def test_production_inspection_diagnoses_pages_and_follows_exact_source_links(self):
        self.peer.operations_capture = operations_capture
        sid, original, _ = self.order_plan()
        request = envelope({'kind': 'production_diagnosis', 'limit': 1})
        self.assert_inspection_refused(self.inspection_request(sid, request))
        current = self.goal_sample(sid, 125, inventory=False)
        first = self.inspection_request(sid, request)
        report = self.assert_inspection(first, current)
        self.assertEqual(report['kind'], 'production_diagnosis')
        self.assertFalse(report['reservation_created'])
        self.assertFalse(report['coverage']['causal_blockers_proven'])
        self.assertEqual(report['coverage']['game_feasibility'], 'unknown')
        self.assertEqual(report['summary']['jobs_considered'], 2)
        self.assertEqual(report['summary']['jobs_with_findings'], 2)
        self.assertEqual(report['total_rows'], 2)
        self.assertEqual(report['returned'], 1)
        self.assertTrue(report['truncated'])
        self.assertIsInstance(report['continuation'], str)
        row = report['rows'][0]
        self.assertEqual(row['native_job_id'], 3)
        self.assertEqual(row['assessment'], 'observed_conditions_not_proven_causes')
        self.assertFalse(row['blocker_proven'])
        self.assertFalse(row['job_ready_proven'])
        self.assertTrue(row['suspended'])
        self.assertFalse(row['worker_assigned'])
        self.assertEqual(row['holder_stage'], {'stage': 1, 'maximum': 3})
        self.assertEqual(row['attachment_records'], 2)
        self.assertEqual(row['distinct_attached_items'], 1)
        self.assertEqual(row['filters_without_indexed_attachments'], 1)
        self.assertEqual(row['direct_item_flags'], {'rotten': 1})
        self.assertEqual(row['container_item_flags'], {'forbidden': 1})
        self.assertEqual(row['shared_attached_items'], 1)
        self.assertEqual(row['affected_item_count'], 1)
        for finding in ('job_suspended', 'no_worker_assigned', 'holder_under_construction',
                        'filters_without_indexed_attachments', 'attached_item_flags',
                        'container_item_flags', 'shared_attachment'):
            self.assertIn(finding, row['findings'])

        next_request = copy.deepcopy(request)
        next_request['query']['continuation'] = report['continuation']
        page = self.assert_inspection(self.inspection_request(sid, next_request), current)
        self.assertEqual([entry['native_job_id'] for entry in page['rows']], [4])
        self.assertEqual(page['total_rows'], 2)
        self.assertFalse(page['truncated'])
        self.assertIsNone(page['continuation'])
        self.assertEqual(page['analysis_digest'], report['analysis_digest'])

        assignment = self.assert_inspection(
            self.inspection_request(sid, row['inspect_assignment']), current)
        self.assertEqual(assignment['kind'], 'inspect')
        self.assertEqual(assignment['row']['entity_id'], row['job']['entity_id'])
        self.assertEqual(assignment['row']['generation'], row['job']['generation'])
        self.assertIs(assignment['row']['fields']['worker_assigned']['value']['value'], False)
        relationships = self.assert_inspection(
            self.inspection_request(sid, row['inspect_relationships']), current)
        self.assertEqual(relationships['kind'], 'traverse')
        visits = {entry['entity_id'] for entry in relationships['visits']}
        self.assertTrue({row['job']['entity_id'], str(ITEM_NAMESPACE + 2),
                         str(ITEM_NAMESPACE + 3), str(BUILDING_NAMESPACE + 7)} <= visits)
        self.assertEqual(relationships['witness']['anchor'], report['anchor'])
        self.assert_single_original_effects(first, original)

    def test_production_inspection_joint_inventory_and_graph_queries_use_one_capture(self):
        self.peer.operations_capture = operations_capture
        sid, _, _ = self.inventory_plan()
        current = self.goal_sample(sid, 125)
        demands = [{'key': key, 'units': 4, 'item_types': ['WOOD']}
                   for key in ('first', 'second')]
        request = envelope({'kind': 'inventory_plan', 'quantity_unit': 'stack_units',
                            'demands': demands, 'limit': 8})
        report = self.assert_inspection(
            self.inspection_request(sid, request), current, inventory=True)
        self.assertEqual(report['kind'], 'inventory_plan')
        self.assertFalse(report['model_feasible'])
        self.assertFalse(report['reservation_created'])
        self.assertEqual(report['summary']['requested_units'], 8)
        self.assertEqual(report['summary']['allocated_units'], 7)
        self.assertEqual(report['summary']['candidate_stack_units'], 7)
        self.assertEqual(sum(row['units'] for row in report['rows']), 7)
        self.assertEqual({row['item']['entity_id'] for row in report['rows']},
                         {str(ITEM_NAMESPACE + 4)})
        self.assertEqual({row['demand_key'] for row in report['rows']}, {'first', 'second'})
        self.assertEqual(report['certificate']['flow_units'], 7)
        self.assertEqual(report['certificate']['cut_capacity'], 7)
        self.assertEqual(report['certificate']['shortage']['deficit'], 1)
        self.assertEqual(report['certificate']['shortage']['demand_keys'], ['first', 'second'])
        self.assertFalse(report['truncated'])
        jobs = self.assert_inspection(self.inspection_request(sid, envelope(
            {'kind': 'entities', 'kinds': ['job'], 'fields': ['worker_assigned'],
             'limit': 8}, report['anchor'])), current, inventory=True)
        self.assertEqual(jobs['matched'], 2)
        self.assertEqual({row['entity_id'] for row in jobs['rows']}, {'5', '6'})
        dependencies = self.assert_inspection(self.inspection_request(sid, envelope(
            {'kind': 'dependencies', 'edge_kinds': ['uses', 'contained_in']},
            report['anchor'])), current, inventory=True)
        self.assertEqual(dependencies['kind'], 'dependencies')
        self.assertEqual(dependencies['blocked_by_cycles'], [])
        self.assertEqual(dependencies['witness']['anchor'], report['anchor'])

    def test_production_inspection_rejects_bad_requests_and_requires_a_new_wait(self):
        self.peer.operations_capture = operations_capture
        sid, _, _ = self.order_plan()
        first = self.goal_sample(sid, 125, inventory=False)
        page_request = envelope({'kind': 'production_diagnosis', 'limit': 1})
        page = self.assert_inspection(self.inspection_request(sid, page_request), first)
        stale_page = copy.deepcopy(page_request)
        stale_page['query']['continuation'] = page['continuation']
        wrong_generation = copy.deepcopy(page['rows'][0]['inspect_assignment'])
        wrong_generation.pop('expected_anchor')
        wrong_generation['query']['generation'] += 1
        invalid = [
            '{',
            {**envelope({'kind': 'production_diagnosis'}), 'native_id': 3},
            envelope({'kind': 'production_diagnosis', 'repair': True}),
            envelope({'kind': 'production_diagnosis', 'limit': 0}),
            envelope({'kind': 'production_diagnosis', 'max_work': 1}),
            envelope({'kind': 'production_diagnosis',
                      'job': {'entity_id': '05', 'generation': 1}}),
            envelope({'kind': 'production_diagnosis'}, first['anchor']),
            stale_page,
            wrong_generation,
            envelope({'kind': 'arbitrary_unknown_query'}),
        ]
        for index, request in enumerate(invalid):
            with self.subTest(request=request):
                self.goal_sample(sid, 130 + 5 * index, inventory=False)
                self.assert_inspection_refused(self.inspection_request(sid, request))
                # A refusal withdraws the published inspection source. Retrying a
                # valid request cannot revive an older source or acquire a new one.
                self.assert_inspection_refused(self.inspection_request(sid, page_request))
                retained = self.client.tool('query', session_id=sid)
                self.assertTrue(retained['ok'], retained)
                self.assertIsNone(retained.get('production_inspection_source'))
        fresh = self.goal_sample(sid, 185, inventory=False)
        self.assert_inspection(self.inspection_request(sid, page_request), fresh)
        self.assert_inspection_refused(
            self.inspection_request(sid, page_request, order_step_id=999))
        self.assert_inspection_refused(self.inspection_request(sid, page_request))

    def test_production_stock_aggregate_preserves_the_original_goal_and_selected_source(self):
        self.peer.operations_capture = operations_capture
        sid, original, _ = self.inventory_plan(stable_samples=2)
        current = self.goal_sample(sid, 125)
        self.assertFalse(current['original_goal_proven'])
        source = current['production_inspection_source']
        self.assertEqual(source['inventory_totals_semantics'], 'observed_stock_not_usable_supply')
        self.assertIn('aggregate', source['query_kinds'])
        result = self.inspection_request(sid, source['inventory_totals_request'])
        report = self.assert_inspection(result, current, inventory=True)
        self.assert_observed_stock_aggregate(report)
        self.assertEqual(report['inspection_scope'], 'captured_operations')
        self.assertEqual(report['order_step_id'], source['order_step_id'])
        self.assertEqual(result['production_inspection_source'], source)
        self.assertEqual(result['selected_inventory_goal'], current['selected_inventory_goal'])
        self.assertEqual(result['current_parent_goal'], current['current_parent_goal'])
        self.assertFalse(result['original_goal_proven'])
        groups = {group['key']['value']['value']: group for group in report['groups']}
        self.assertEqual(groups['WOOD']['metrics']['stack_units']['sum'], '107')
        self.assertEqual(groups['BED']['metrics']['stack_units']['sum'], '1')
        self.assert_single_original_effects(result, original)

    def test_production_inspection_withdraws_after_a_failed_parent_bracket(self):
        self.peer.operations_capture = operations_capture
        sid, original, _ = self.inventory_plan(stable_samples=3)
        first = self.goal_sample(sid, 125)
        request = envelope({'kind': 'production_diagnosis', 'limit': 1})
        self.assert_inspection(self.inspection_request(sid, request), first, inventory=True)

        def reverse_during_operations():
            self.peer.workforce_labor = False
            self.peer.workforce_sequence = 5
        self.peer.after_operations_read = reverse_during_operations
        self.peer.goal_tick(130)
        rejected = self.client.tool('wait', session_id=sid, max_wall_millis=60000)
        self.assertFalse(rejected['ok'], rejected)
        self.assertIsNone(self.peer.after_operations_read)
        self.assertIsNone(rejected['retained_reference']['current_parent_goal'])
        self.assertIsNone(rejected['retained_reference'].get('production_inspection_source'))
        self.assert_inspection_refused(self.inspection_request(sid, request))
        retained = self.client.tool('query', session_id=sid)
        self.assertTrue(retained['ok'], retained)
        self.assert_single_original_effects(retained, original)
        self.peer.workforce_labor = True
        self.peer.workforce_sequence = 6
        fresh = self.goal_sample(sid, 135)
        report = self.assert_inspection(
            self.inspection_request(sid, request), fresh, inventory=True)
        self.assertNotEqual(report['anchor'], first['selected_inventory_goal']['anchor'])

    def test_production_inspection_clock_withdraws_source_until_fresh_wait(self):
        self.enable_clock()
        self.peer.operations_capture = operations_capture
        self.peer.complete_order_on_clock = True
        sid, original, _ = self.order_plan(stable_samples=2, max_game_ticks=10)
        current = self.client.tool('wait', session_id=sid, max_wall_millis=60000)
        self.assertTrue(current['ok'], current)
        request = envelope({'kind': 'production_diagnosis', 'limit': 1})
        first = self.assert_inspection(self.inspection_request(sid, request), current)
        prepared = self.prepare_clock(sid, current)
        self.assertTrue(prepared['ok'], prepared)
        committed = self.client.tool('commit', session_id=sid, scope='run',
            paired_seal=prepared['clock']['active_run']['review_seal'], confirmed=True)
        self.assertTrue(committed['ok'], committed)
        self.assertEqual(len(self.peer.clock_commits), 1)
        self.assertIsNone(committed['current_parent_goal'])
        self.assertIsNone(committed.get('production_inspection_source'))
        self.assert_inspection_refused(self.inspection_request(sid, request))
        fresh = self.client.tool('wait', session_id=sid, max_wall_millis=60000)
        self.assertTrue(fresh['ok'], fresh)
        report = self.assert_inspection(self.inspection_request(sid, request), fresh)
        self.assertNotEqual(report['anchor'], first['anchor'])
        self.assertNotEqual(report['source_digest'], first['source_digest'])
        self.assert_inspection_refused(
            self.inspection_request(sid, first['rows'][0]['inspect_assignment']))
        self.assertEqual(len(self.peer.clock_commits), 1)
        self.assert_single_original_effects(fresh, original)

    def test_production_inspection_offline_reopen_retains_history_without_source(self):
        self.peer.operations_capture = operations_capture
        sid, original, _ = self.inventory_plan()
        achieved = self.goal_sample(sid, 125)
        self.assertTrue(achieved['original_goal_proven'])
        request = envelope({'kind': 'production_diagnosis', 'limit': 1})
        self.assert_inspection(self.inspection_request(sid, request), achieved, inventory=True)
        historical = achieved['selected_inventory_goal']['first_satisfied_anchor']
        cancelled = self.client.tool('cancel', session_id=sid, scope='session')
        self.assertTrue(cancelled['ok'], cancelled)
        before, calls = self.inspection_custody(), list(self.peer.calls)
        offline = self.inspection_reopen_offline(achieved)
        self.assertTrue(offline['ok'], offline)
        self.assertFalse(offline['original_goal_proven'])
        self.assertIsNone(offline['current_parent_goal'])
        self.assertIsNone(offline.get('production_inspection_source'))
        self.assertEqual(offline['selected_inventory_goal']['first_satisfied_anchor'], historical)
        self.assertEqual(self.peer.calls, calls)
        self.assertEqual(self.inspection_custody(), before)
        offline_sid = offline['agent_turn']['session_id']
        self.assert_inspection_refused(self.inspection_request(offline_sid, request))
        retained = self.client.tool('query', session_id=offline_sid)
        self.assertTrue(retained['ok'], retained)
        self.assert_single_original_effects(retained, original)
        self.assertEqual(self.peer.calls, calls)
        self.assertEqual(self.inspection_custody(), before)
