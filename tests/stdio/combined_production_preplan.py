"""Preplan production discovery through the actual combined stdio server.

The Operations/1.3 source is the existing coherent inspection wire double. Its
read route is available before an order exists; no Created receipt is fabricated
to make discovery work. These probes do not establish native qualification.
"""
from __future__ import annotations

import copy
import importlib.util
import json
from pathlib import Path
import struct


_spec = importlib.util.spec_from_file_location(
    'preplan_inspection_fixture', Path(__file__).with_name('combined_production_inspection.py'))
_inspection = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_inspection)
envelope = _inspection.envelope
ITEM_NAMESPACE = _inspection.ITEM_NAMESPACE
OPERATIONS = 'dfmcp_operations_v1_3'


def _blob(tag, payload):
    """Encode the existing length-delimited protobuf capture field."""
    length, encoded = len(payload), bytearray()
    while length > 127:
        encoded.append((length & 127) | 128)
        length >>= 7
    encoded.append(length)
    return bytes([tag << 3 | 2]) + bytes(encoded) + payload


def _unpaused(raw):
    raw = bytearray(raw)
    # The Operations wrapper is twelve bytes; jobs has two u32 tick fields.
    raw[12 + 8 + 4 + 4] = 0
    return bytes(raw)


def _regressed_item_allocator(raw):
    raw = bytearray(raw)
    jobs_bytes = struct.unpack_from('>I', raw, 8)[0]
    struct.pack_into('>I', raw, 12 + jobs_bytes + 4, 99)
    return bytes(raw)


class PreplanInspectionTests:
    """Mixed into CombinedProduction to reuse its public subprocess fixture."""

    def preplan_fixture(self):
        self.peer.operations_capture = _inspection.operations_capture
        self.preplan_transform = lambda raw: raw
        original_reply = self.peer.reply

        def reply(plugin, operation, request):
            if plugin != OPERATIONS or operation != 'ReadObservation':
                return original_reply(plugin, operation, request)
            # Preserve the existing token, protocol, and bounded-request checks.
            # Only remove the fixture's post-Created prerequisite for a read.
            header = original_reply(plugin, 'Handshake', request)
            capture = self.peer.operations_capture(self.peer.order_tick, self.peer.inventory_stack)
            result = header + _blob(9, self.preplan_transform(capture))
            if self.peer.after_operations_read is not None:
                hook, self.peer.after_operations_read = self.peer.after_operations_read, None
                hook()
            return result

        self.peer.reply = reply

    def preplan_open(self):
        self.preplan_fixture()
        opened = self.client.open(max_wall_millis=60000)
        self.assertTrue(opened['ok'], opened)
        self.preplan_custody = self.inspection_custody()
        return opened['agent_turn']['session_id']

    def preplan_observe(self, sid):
        result = self.client.tool('observe', session_id=sid,
                                  request=json.dumps({'include_production': True}))
        self.assertTrue(result['ok'], result)
        source = result['production_inspection_source']
        self.assertEqual(source['scope'], 'preplan_captured_operations')
        self.assertEqual(source['attribution'], 'not_order_attributed')
        self.assertEqual(source['source_protocol'], 'operations/1.3')
        self.assertEqual(source['source_generation'], 71)
        self.assertIsNone(source['order_step_id'])
        self.assertEqual(source['canonical_anchor'], result['anchor'])
        self.assertEqual(source['parent_anchor'], result['anchor'])
        for field in ('fortress_id', 'tick'):
            self.assertEqual(source['anchor'][field], result['anchor'][field])
        self.assertEqual(len(source['source_digest']), 64)
        self.assertFalse(source['mutation_authority'])
        self.assertFalse(source['commit_compatible'])
        self.assertFalse(source['original_goal_evidence'])
        self.assertFalse(source['baseline_reused_for_planning'])
        self.assertFalse(source['restored_from_history'])
        self.assertEqual(set(source['query_kinds']), {
            'production_diagnosis', 'inventory_plan', 'entities', 'inspect',
            'traverse', 'dependencies', 'aggregate'})
        self.assertEqual(source['inventory_totals_semantics'], 'observed_stock_not_usable_supply')
        self.assert_preplan_no_effect(result)
        return result

    def assert_preplan_no_effect(self, result):
        self.assertIsNone(result.get('original_plan'))
        self.assertIsNone(result.get('selected_inventory_goal'))
        self.assertIsNone(result.get('selected_order_goal'))
        self.assertIsNone(result.get('current_parent_goal'))
        self.assertFalse(result.get('original_goal_proven', False))
        self.assertFalse(self.peer.order_created)
        self.assertFalse(self.peer.workforce_applied)
        self.assertIsNone(self.peer.order_plan)
        self.assertIsNone(self.peer.workforce_plan)
        self.assertEqual(self.peer.clock_commits, [])
        self.assertEqual(self.peer.inventory_clock_commits, [])
        self.assertFalse(any(operation.startswith(('Prepare', 'Commit'))
                             for _, operation in self.peer.calls))
        self.assertEqual(self.inspection_custody(), self.preplan_custody,
                         'preplan discovery must not create effect keys or alter custody')

    def preplan_query(self, sid, request, observed):
        result = self.inspection_request(sid, request)
        self.assertTrue(result['ok'], result)
        source = observed['production_inspection_source']
        self.assertEqual(result['production_inspection_source'], source)
        report = result['production_inspection']
        self.assertEqual(report['schema'], 'dfmcp.query.result/1')
        self.assertEqual(report['inspection_scope'], 'preplan_captured_operations')
        self.assertEqual(report['attribution'], 'not_order_attributed')
        self.assertEqual(report['anchor'], source['anchor'])
        self.assertEqual(report['canonical_anchor'], observed['anchor'])
        self.assertEqual(report['source_digest'], source['source_digest'])
        self.assertIsNone(report['order_step_id'])
        self.assertFalse(report['mutation_authority'])
        self.assertFalse(report['commit_compatible'])
        self.assertFalse(report['original_goal_evidence'])
        self.assert_preplan_no_effect(result)
        return report

    def preplan_reads(self):
        return self.peer.calls.count((OPERATIONS, 'ReadObservation'))

    @staticmethod
    def preplan_plan_request(observed):
        return {'expected_observation': observed['observation']['seal'], 'units': ['43'],
                'labor': 'MINE', 'enabled': True, 'recipe': 'wooden_bed', 'amount': 1,
                'deadline_tick': 300, 'poll_interval_ticks': 5, 'stable_samples': 1,
                'objective': {'kind': 'observed_inventory_increase', 'item_type': 'BED',
                              'increase_stack_units': 1}}

    def test_preplan_discovers_inventory_diagnosis_and_retained_queries_without_plan(self):
        sid = self.preplan_open()
        observed = self.preplan_observe(sid)
        self.assertEqual(self.preplan_reads(), 1)
        source = observed['production_inspection_source']
        inventory = self.preplan_query(sid, source['inventory_request'], observed)
        self.assertEqual(inventory['kind'], 'entities')
        self.assertEqual(inventory['matched'], 5)
        beds = [row for row in inventory['rows'] if row['entity_id'] == str(ITEM_NAMESPACE + 6)]
        self.assertEqual(len(beds), 1)
        self.assertEqual(beds[0]['fields']['type_key']['value']['value'], 'BED')
        self.assertEqual(beds[0]['fields']['stack_size']['value']['value'], 1)

        diagnosis = self.preplan_query(sid, source['diagnosis_request'], observed)
        self.assertEqual(diagnosis['summary']['jobs_considered'], 2)
        self.assertFalse(diagnosis['coverage']['causal_blockers_proven'])
        row = diagnosis['rows'][0]
        self.assertEqual(row['native_job_id'], 3)
        self.assertIn('job_suspended', row['findings'])
        self.assertIn('shared_attachment', row['findings'])
        self.assertFalse(row['blocker_proven'])
        assignment = self.preplan_query(sid, row['inspect_assignment'], observed)
        self.assertFalse(assignment['row']['fields']['worker_assigned']['value']['value'])
        traversal = self.preplan_query(sid, row['inspect_relationships'], observed)
        self.assertEqual(traversal['kind'], 'traverse')
        self.assertEqual(traversal['witness']['anchor'], source['anchor'])
        dependencies = self.preplan_query(sid, envelope(
            {'kind': 'dependencies', 'edge_kinds': ['uses', 'contained_in']}, source['anchor']), observed)
        self.assertEqual(dependencies['blocked_by_cycles'], [])
        allocation = self.preplan_query(sid, envelope(
            {'kind': 'inventory_plan', 'quantity_unit': 'stack_units', 'limit': 8,
             'demands': [{'key': name, 'units': 4, 'item_types': ['WOOD']}
                         for name in ('first', 'second')]}, source['anchor']), observed)
        self.assertFalse(allocation['model_feasible'])
        self.assertFalse(allocation['reservation_created'])
        self.assertEqual(allocation['summary']['allocated_units'], 7)
        self.assertEqual(allocation['certificate']['shortage']['deficit'], 1)
        self.assertEqual(self.preplan_reads(), 1, 'Query must use the single retained capture')

    def test_preplan_copied_stock_totals_count_observed_units_without_allocating_them(self):
        sid = self.preplan_open()
        observed = self.preplan_observe(sid)
        source = observed['production_inspection_source']
        request = source['inventory_totals_request']
        self.assertEqual(request, envelope({
            'kind': 'aggregate', 'kinds': ['item'],
            'group_by': {'kind': 'field', 'field': 'type_key'},
            'metrics': [{'name': 'stack_units', 'field': 'stack_size', 'numeric_type': 'u64'}],
            'max_groups': 32}, source['anchor']))
        report = self.preplan_query(sid, request, observed)
        self.assert_observed_stock_aggregate(report)
        self.assertEqual(report['matched'], 5)
        groups = {group['key']['value']['value']: group for group in report['groups']}
        self.assertEqual(set(groups), {'WOOD', 'BED', 'BIN'})
        wood = groups['WOOD']
        self.assertEqual(wood['key'], {
            'kind': 'field', 'presence': 'known', 'value': {'type': 'text', 'value': 'WOOD'}})
        self.assertEqual(wood['count'], 3, 'record count is distinct from stack units')
        stats = wood['metrics']['stack_units']
        self.assertEqual(stats['samples'], 3)
        self.assertEqual(stats['sum'], '107')
        self.assertEqual(stats['minimum'], '1')
        self.assertEqual(stats['maximum'], '99')
        self.assertEqual(stats['mean'], {'numerator': '107', 'denominator': 3})
        for kind in ('BED', 'BIN'):
            self.assertEqual(groups[kind]['count'], 1)
            self.assertEqual(groups[kind]['metrics']['stack_units']['sum'], '1')
        allocation = self.preplan_query(sid, envelope({
            'kind': 'inventory_plan', 'quantity_unit': 'stack_units',
            'demands': [{'key': 'logs', 'units': 108, 'item_types': ['WOOD']}], 'limit': 8},
            source['anchor']), observed)
        self.assertEqual(allocation['summary']['candidate_stack_units'], 7)
        self.assertEqual(allocation['summary']['allocated_units'], 7)
        self.assertFalse(allocation['model_feasible'])
        self.assertFalse(allocation['reservation_created'])
        self.assertEqual(self.preplan_reads(), 1)

    def test_preplan_stock_filters_preserve_false_unknown_and_empty_statistics(self):
        sid = self.preplan_open()
        observed = self.preplan_observe(sid)
        base = observed['production_inspection_source']['inventory_totals_request']
        unavailable = {'op': 'compare', 'field': 'native_recipe_eligible', 'comparison': 'eq',
                       'value': {'type': 'bool', 'value': False}}
        cases = [
            ('known false flag', {'op': 'compare', 'field': 'removed', 'comparison': 'eq',
                                  'value': {'type': 'bool', 'value': False}}, 4, 1, 0, '10'),
            ('no matching type', {'op': 'compare', 'field': 'type_key', 'comparison': 'eq',
                                 'value': {'type': 'text', 'value': 'DOOR'}}, 0, 5, 0, None),
            ('unknown field', unavailable, 0, 0, 5, None),
            ('negated unknown', {'op': 'not', 'arg': unavailable}, 0, 0, 5, None),
        ]
        for name, predicate, matched, false_rows, unknown_rows, total in cases:
            with self.subTest(selection=name):
                request = copy.deepcopy(base)
                request['query'].pop('group_by')
                request['query']['where'] = predicate
                report = self.preplan_query(sid, request, observed)
                self.assert_observed_stock_aggregate(report)
                self.assertEqual(report['matched'], matched)
                self.assertEqual(report['selection']['selected_entities'], 5)
                self.assertEqual(report['selection']['excluded_false'], false_rows)
                self.assertEqual(report['selection']['excluded_unknown'], unknown_rows)
                self.assertEqual(report['selection']['complete_within_projection'], unknown_rows == 0)
                self.assertEqual(len(report['groups']), 1)
                group = report['groups'][0]
                self.assertEqual(group['key'], {'kind': 'all'})
                self.assertEqual(group['count'], matched)
                stats = group['metrics']['stack_units']
                self.assertEqual(stats['samples'], matched)
                self.assertEqual(stats['sum'], total)
                if matched == 0:
                    for field in ('sum', 'minimum', 'maximum', 'mean'):
                        self.assertIsNone(stats[field], 'an empty statistic is not a stock value')
        self.assertEqual(self.preplan_reads(), 1)

    def test_preplan_aggregate_limits_withdraw_all_source_handles_until_new_observe(self):
        sid = self.preplan_open()
        for invalid in ('too few groups', 'invalid metric type'):
            with self.subTest(request=invalid):
                observed = self.preplan_observe(sid)
                valid = observed['production_inspection_source']['inventory_totals_request']
                request = copy.deepcopy(valid)
                if invalid == 'too few groups':
                    request['query']['max_groups'] = 1
                else:
                    request['query']['metrics'][0]['numeric_type'] = 'f64'
                self.assert_inspection_refused(self.inspection_request(sid, request))
                self.assert_inspection_refused(self.inspection_request(sid, valid))
                retained = self.client.tool('query', session_id=sid)
                self.assertTrue(retained['ok'], retained)
                self.assertIsNone(retained['production_inspection_source'])
                self.assert_preplan_no_effect(retained)
        fresh = self.preplan_observe(sid)
        report = self.preplan_query(
            sid, fresh['production_inspection_source']['inventory_totals_request'], fresh)
        self.assert_observed_stock_aggregate(report)
        self.assertEqual(self.preplan_reads(), 3)

    def test_preplan_opt_in_refresh_withdrawal_and_stale_continuation(self):
        sid = self.preplan_open()
        request = envelope({'kind': 'production_diagnosis', 'limit': 1})
        self.assert_inspection_refused(self.inspection_request(sid, request))
        ordinary = self.client.tool('observe', session_id=sid)
        self.assertTrue(ordinary['ok'], ordinary)
        self.assertIsNone(ordinary['production_inspection_source'])
        self.assertEqual(self.preplan_reads(), 0)
        first = self.preplan_observe(sid)
        page = self.preplan_query(sid, request, first)
        self.assertIsInstance(page['continuation'], str)
        next_page = copy.deepcopy(request)
        next_page['query']['continuation'] = page['continuation']
        second_page = self.preplan_query(sid, next_page, first)
        self.assertEqual([row['native_job_id'] for row in second_page['rows']], [4])

        ordinary = self.client.tool('observe', session_id=sid, request='{"include_production":false}')
        self.assertTrue(ordinary['ok'], ordinary)
        self.assertIsNone(ordinary['production_inspection_source'])
        self.assertEqual(self.preplan_reads(), 1)
        self.assert_inspection_refused(self.inspection_request(sid, request))
        self.peer.inventory_stack = 2
        fresh = self.preplan_observe(sid)
        self.assertNotEqual(fresh['production_inspection_source']['source_digest'],
                            first['production_inspection_source']['source_digest'])
        self.assert_inspection_refused(self.inspection_request(sid, next_page))
        self.assert_inspection_refused(self.inspection_request(sid, request))
        self.assertEqual(self.preplan_reads(), 2)
        restored = self.preplan_observe(sid)
        self.preplan_query(sid, request, restored)

    def test_preplan_growth_plan_captures_independent_baseline_and_proves_only_delta(self):
        sid = self.preplan_open()
        self.peer.inventory_stack = 2
        observed = self.preplan_observe(sid)
        request = self.preplan_plan_request(observed)
        # Same paused citizen anchor, changed stock: Plan must reacquire its own
        # complete baseline, not turn the advisory two-bed source into authority.
        self.peer.inventory_stack = 4
        planned = self.client.tool('plan', session_id=sid, request=json.dumps(request))
        self.assertTrue(planned['ok'], planned)
        self.assertEqual(self.preplan_reads(), 2)
        self.assertIsNone(planned['production_inspection_source'])
        selected = planned['selected_inventory_goal']
        self.assertEqual(selected['kind'], 'observed_inventory_increase')
        self.assertEqual(selected['baseline_stack_units'], 4)
        self.assertEqual(selected['increase_stack_units'], 1)
        self.assertEqual(selected['target_stack_units'], 5)
        self.assertFalse(planned['original_goal_proven'])
        self.assertFalse(self.peer.order_created)
        self.assertIsNone(self.peer.order_plan)
        self.assert_inspection_refused(self.inspection_request(
            sid, observed['production_inspection_source']['inventory_request']))

        original = planned['original_plan']
        current = self.client.tool('query', session_id=sid)
        self.assertTrue(current['ok'], current)
        for step in (0, 1):
            prepared = self.client.execute_next(current)
            self.assertTrue(prepared['ok'], prepared)
            self.assertEqual(prepared['review']['step_id'], step)
            current = self.client.execute_next(prepared)
            self.assertTrue(current['ok'], current)
        self.assertTrue(current['frontier']['all_applied'])
        unchanged = self.goal_sample(sid, 125)
        self.assertFalse(unchanged['original_goal_proven'])
        self.assertEqual(unchanged['selected_inventory_goal']['current_truth'], 'false')
        self.peer.inventory_stack = 5
        grown = self.goal_sample(sid, 130)
        self.assertTrue(grown['original_goal_proven'])
        self.assertEqual(grown['selected_inventory_goal']['baseline_stack_units'], 4)
        self.assertEqual(grown['selected_inventory_goal']['production_attribution'], 'unknown')
        self.assertEqual(grown['original_plan'], original)

    def test_preplan_rejects_foreign_unpaused_and_incomplete_sources_without_reviving_old_data(self):
        sid = self.preplan_open()
        request = envelope({'kind': 'production_diagnosis', 'limit': 1})
        for name, transform in (
                ('foreign fortress', lambda raw: raw.replace(b'region1', b'region2')),
                ('unpaused', _unpaused), ('incomplete capture', lambda raw: raw[:-1])):
            with self.subTest(source=name):
                self.preplan_transform = lambda raw: raw
                self.preplan_observe(sid)
                self.preplan_transform = transform
                refused = self.client.tool('observe', session_id=sid,
                                           request='{"include_production":true}')
                self.assert_inspection_refused(refused)
                self.assert_inspection_refused(self.inspection_request(sid, request))
                retained = self.client.tool('query', session_id=sid)
                self.assertTrue(retained['ok'], retained)
                self.assertIsNone(retained['production_inspection_source'])
                self.assert_preplan_no_effect(retained)
        self.preplan_transform = lambda raw: raw
        fresh = self.preplan_observe(sid)
        self.preplan_query(sid, request, fresh)

    def test_preplan_consumed_future_tick_survives_failed_capture(self):
        sid = self.preplan_open()
        self.preplan_observe(sid)
        self.peer.order_tick = 125
        refused = self.client.tool('observe', session_id=sid,
                                   request='{"include_production":true}')
        self.assert_inspection_refused(refused)
        self.assertEqual(refused['retained_reference']['observation_tick_floor'], 125)
        reads = self.preplan_reads()
        self.peer.order_tick = 120
        rolled_back = self.client.tool('observe', session_id=sid,
                                       request='{"include_production":true}')
        self.assert_inspection_refused(rolled_back)
        self.assertEqual(rolled_back['retained_reference']['observation_tick_floor'], 125)
        self.assertEqual(self.preplan_reads(), reads,
                         'a lower citizen tick must fail before another operations capture')
        retained = self.client.tool('query', session_id=sid)
        self.assertTrue(retained['ok'], retained)
        self.assertIsNone(retained['production_inspection_source'])
        self.assert_preplan_no_effect(retained)

    def test_preplan_retains_generations_and_permanently_fences_allocator_regression(self):
        sid = self.preplan_open()
        first = self.preplan_observe(sid)
        inventory = self.preplan_query(sid, first['production_inspection_source']['inventory_request'], first)
        bed = next(row for row in inventory['rows'] if row['entity_id'] == str(ITEM_NAMESPACE + 6))
        self.peer.inventory_stack = 0
        absent = self.preplan_observe(sid)
        rows = self.preplan_query(sid, absent['production_inspection_source']['inventory_request'], absent)['rows']
        self.assertNotIn(bed['entity_id'], [row['entity_id'] for row in rows])
        ordinary = self.client.tool('observe', session_id=sid)
        self.assertTrue(ordinary['ok'], ordinary)
        self.assertIsNone(ordinary['production_inspection_source'])
        self.peer.inventory_stack = 1
        returned = self.preplan_observe(sid)
        rows = self.preplan_query(sid, returned['production_inspection_source']['inventory_request'], returned)['rows']
        new_bed = next(row for row in rows if row['entity_id'] == bed['entity_id'])
        self.assertGreater(new_bed['generation'], bed['generation'])
        stale = envelope({'kind': 'inspect', 'entity_id': bed['entity_id'],
                          'generation': bed['generation'], 'fields': ['stack_size']})
        self.assert_inspection_refused(self.inspection_request(sid, stale))
        self.preplan_observe(sid)
        self.preplan_transform = _regressed_item_allocator
        regressed = self.client.tool('observe', session_id=sid,
                                     request='{"include_production":true}')
        self.assert_inspection_refused(regressed)
        reads = self.preplan_reads()
        self.preplan_transform = lambda raw: raw
        fenced = self.client.tool('observe', session_id=sid,
                                  request='{"include_production":true}')
        self.assert_inspection_refused(fenced)
        self.assertEqual(self.preplan_reads(), reads,
                         'restoring a counter cannot silently remove a source regression fence')

    def test_preplan_restart_withdraws_source_and_reconcile_needs_no_plan_authority(self):
        sid = self.preplan_open()
        observed = self.preplan_observe(sid)
        request = observed['production_inspection_source']['inventory_request']
        calls = list(self.peer.calls)
        offline = self.existing_labor_reopen(observed, 'offline')
        self.assertTrue(offline['ok'], offline)
        self.assertEqual(self.peer.calls, calls)
        offline_sid = offline['agent_turn']['session_id']
        self.assertIsNone(offline['production_inspection_source'])
        self.assert_inspection_refused(self.inspection_request(offline_sid, request))

        reopened = self.existing_labor_reopen(observed, 'reconcile')
        self.assertTrue(reopened['ok'], reopened)
        self.assertFalse(reopened['labor_control_enabled'])
        self.assertFalse(reopened['production_control_enabled'])
        fresh_sid = reopened['agent_turn']['session_id']
        self.assertIsNone(reopened['production_inspection_source'])
        self.assert_inspection_refused(self.inspection_request(fresh_sid, request))
        fresh = self.preplan_observe(fresh_sid)
        self.preplan_query(fresh_sid, fresh['production_inspection_source']['inventory_request'], fresh)
        reads = self.preplan_reads()
        refused = self.client.tool('plan', session_id=fresh_sid,
                                   request=json.dumps(self.preplan_plan_request(fresh)))
        self.assertFalse(refused['ok'], refused)
        self.assertEqual(self.preplan_reads(), reads)
        self.assert_preplan_no_effect(refused.get('retained_reference', {}))

    def test_preplan_closed_query_and_work_budget_refusals_withdraw_the_source(self):
        sid = self.preplan_open()
        request = envelope({'kind': 'production_diagnosis', 'limit': 1})
        invalid = [
            '{', envelope({'kind': 'production_diagnosis', 'max_work': 1}),
            envelope({'kind': 'production_diagnosis', 'repair': True}),
            envelope({'kind': 'production_diagnosis', 'limit': 0}),
            envelope({'kind': 'arbitrary_unknown_query'}),
        ]
        for bad_request in invalid:
            with self.subTest(request=bad_request):
                self.preplan_observe(sid)
                self.assert_inspection_refused(self.inspection_request(sid, bad_request))
                self.assert_inspection_refused(self.inspection_request(sid, request))
                retained = self.client.tool('query', session_id=sid)
                self.assertTrue(retained['ok'], retained)
                self.assertIsNone(retained['production_inspection_source'])
                self.assert_preplan_no_effect(retained)
        self.preplan_observe(sid)
        self.assert_inspection_refused(self.inspection_request(sid, request, order_step_id=0))
        self.assert_inspection_refused(self.inspection_request(sid, request))
