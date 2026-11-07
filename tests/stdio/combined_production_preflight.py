"""Pre-plan production inspection through the actual modern stdio facade.

The native peers supply authentic Operations/1.3 wire frames. They are doubles,
and these scenarios claim neither live-game evidence nor production admission.
"""
from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import struct


_fixture_spec = importlib.util.spec_from_file_location(
    'combined_preflight_operations_fixture',
    Path(__file__).with_name('combined_production_inspection.py'))
_fixture = importlib.util.module_from_spec(_fixture_spec)
_fixture_spec.loader.exec_module(_fixture)
ITEM_NAMESPACE = 2 << 40


def envelope(query, anchor=None):
    value = {'schema': 'dfmcp.query/1', 'query': query}
    if anchor is not None:
        value['expected_anchor'] = anchor
    return value


class PreflightTests:
    """Use CombinedProduction's real process, private stores and TCP peers."""

    def preflight_open(self):
        self.peer.operations_capture = _fixture.operations_capture
        opened = self.client.open(max_wall_millis=60000)
        self.assertTrue(opened['ok'], opened)
        sid = opened['agent_turn']['session_id']
        self.assertIsNone(opened.get('original_plan'))
        return sid, self.preflight_observe(sid)

    def preflight_observe(self, sid):
        observed = self.client.tool('observe', session_id=sid,
                                    request=json.dumps({'include_operations': True}))
        self.assertTrue(observed['ok'], observed)
        source = observed['production_inspection_source']
        self.assertEqual(source['scope'], 'preplan_captured_operations')
        self.assertEqual(source['observation_phase'], 'before_plan')
        self.assertEqual(source['attribution'], 'not_order_attributed')
        self.assertIsNone(source['order_step_id'])
        self.assertEqual(source['parent_anchor'], observed['anchor'])
        for name in ('fortress_id', 'tick'):
            self.assertEqual(source['anchor'][name], observed['anchor'][name])
        self.assertNotEqual(source['anchor']['state_hash'], observed['anchor']['state_hash'])
        self.assertEqual(len(source['source_digest']), 64)
        self.assertFalse(source['mutation_authority'])
        self.assertFalse(source['restored_from_history'])
        self.assertIsNone(observed.get('original_plan'))
        self.assertFalse(observed['original_goal_proven'])
        self.assert_no_preflight_effects()
        return observed

    def preflight_report(self, sid, query, observed, *, complete=False):
        request = query if complete else envelope(query, observed['production_inspection_source']['anchor'])
        result = self.inspection_request(sid, request)
        self.assertTrue(result['ok'], result)
        source = result['production_inspection_source']
        self.assertEqual(source, observed['production_inspection_source'])
        report = result['production_inspection']
        self.assertEqual(report['schema'], 'dfmcp.query.result/1')
        self.assertEqual(report['anchor'], source['anchor'])
        self.assertEqual(report['source_digest'], source['source_digest'])
        self.assertFalse(report['mutation_authority'])
        self.assertFalse(report['commit_compatible'])
        self.assertIsNone(result.get('original_plan'))
        self.assert_no_preflight_effects()
        return report

    def preflight_operations_reads(self):
        return sum(plugin == 'dfmcp_operations_v1_3' and method == 'ReadObservation'
                   for plugin, method in self.peer.calls)

    def assert_no_preflight_effects(self):
        self.assertFalse(self.peer.workforce_applied)
        self.assertFalse(self.peer.order_created)
        self.assertIsNone(self.peer.workforce_plan)
        self.assertIsNone(self.peer.order_plan)
        self.assertFalse(any(method.startswith(('Prepare', 'Commit', 'Cancel'))
                             for _, method in self.peer.calls), self.peer.calls)
        self.assertEqual(self.peer.clock_commits, [])
        self.assertEqual(self.peer.inventory_clock_commits, [])

    def test_preflight_diagnosis_and_joint_inventory_need_no_original_or_effect(self):
        sid, observed = self.preflight_open()
        self.assertGreater(self.preflight_operations_reads(), 0)
        source = observed['production_inspection_source']
        diagnosis = self.preflight_report(sid, source['diagnosis_request'], observed, complete=True)
        self.assertEqual(diagnosis['summary']['jobs_considered'], 2)
        row = next(row for row in diagnosis['rows'] if row['native_job_id'] == 3)
        self.assertFalse(row['blocker_proven'])
        self.assertFalse(row['job_ready_proven'])
        self.assertTrue(row['suspended'])
        self.assertFalse(row['worker_assigned'])
        self.assertEqual(row['holder_stage'], {'stage': 1, 'maximum': 3})
        self.assertEqual(row['direct_item_flags'], {'rotten': 1})
        self.assertEqual(row['container_item_flags'], {'forbidden': 1})
        self.assertEqual(row['shared_attached_items'], 1)
        assignment = self.preflight_report(sid, row['inspect_assignment'], observed, complete=True)
        self.assertEqual(assignment['row']['entity_id'], row['job']['entity_id'])
        relationships = self.preflight_report(sid, row['inspect_relationships'], observed, complete=True)
        self.assertEqual(relationships['kind'], 'traverse')
        self.assertIn(str(ITEM_NAMESPACE + 2), {entry['entity_id'] for entry in relationships['visits']})
        jobs = self.preflight_report(sid, {'kind': 'entities', 'kinds': ['job'],
                                          'fields': ['worker_assigned'], 'limit': 8}, observed)
        self.assertEqual(jobs['matched'], 2)
        dependencies = self.preflight_report(sid, {'kind': 'dependencies',
            'edge_kinds': ['uses', 'contained_in']}, observed)
        self.assertEqual(dependencies['blocked_by_cycles'], [])

        inventory = self.preflight_report(sid, {'kind': 'inventory_plan',
            'quantity_unit': 'stack_units', 'limit': 8,
            'demands': [{'key': key, 'units': 4, 'item_types': ['WOOD']}
                        for key in ('first', 'second')]}, observed)
        self.assertEqual(inventory['summary']['requested_units'], 8)
        self.assertEqual(inventory['summary']['allocated_units'], 7)
        self.assertEqual(inventory['certificate']['shortage']['deficit'], 1)
        self.assertEqual({row['item']['entity_id'] for row in inventory['rows']},
                         {str(ITEM_NAMESPACE + 4)})
        self.assertFalse(inventory['reservation_created'])
        self.assertFalse(inventory['model_feasible'])
        self.assert_no_preflight_effects()

    def test_preflight_rich_structured_queries_share_one_capture_without_native_calls(self):
        sid, observed = self.preflight_open()
        aggregate = self.preflight_report(sid, {'kind': 'aggregate', 'kinds': ['item'],
            'group_by': {'kind': 'field', 'field': 'type_key'},
            'metrics': [{'name': 'units', 'field': 'stack_size', 'numeric_type': 'u64'}]}, observed)
        self.assertEqual(aggregate['matched'], 5)
        self.assertEqual(len(aggregate['groups']), 3)
        self.assertFalse(aggregate['coverage']['absence_proven'])
        search = self.preflight_report(sid, {'kind': 'search', 'text': 'WOOD',
            'kinds': ['item'], 'text_fields': ['type_key'], 'include_labels': False, 'limit': 8}, observed)
        self.assertEqual({row['entity_id'] for row in search['rows']},
                         {str(ITEM_NAMESPACE + ident) for ident in (2, 4, 5)})
        spatial = self.preflight_report(sid, {'kind': 'spatial_entities', 'kinds': ['job'],
            'region': {'min': [1, 1, 1], 'max': [1, 1, 1]},
            'geometry': {'kind': 'point', 'field': 'position'}, 'limit': 8}, observed)
        self.assertEqual(spatial['matched'], 2)
        related = self.preflight_report(sid, {'kind': 'related_entities', 'kinds': ['job'],
            'relation': {'edge_kinds': ['uses'], 'target_kinds': ['item'],
                         'target_ids': [str(ITEM_NAMESPACE + 2)]}, 'limit': 8}, observed)
        self.assertEqual({row['entity_id'] for row in related['rows']}, {'5', '6'})
        ranked = self.preflight_report(sid, {'kind': 'ranked_entities', 'kinds': ['item'],
            'fields': ['type_key', 'stack_size'],
            'order_by': [{'field': 'stack_size', 'numeric_type': 'u64', 'direction': 'descending'}],
            'limit': 3}, observed)
        self.assertEqual([row['entity_id'] for row in ranked['rows']],
                         [str(ITEM_NAMESPACE + ident) for ident in (5, 4, 2)])
        events = self.preflight_report(sid, {'kind': 'events', 'limit': 8}, observed)
        self.assertEqual(events['rows'], [])
        self.assertFalse(events['coverage']['history_complete'])
        self.assertFalse(events['coverage']['absence_proven'])
        self.assert_no_preflight_effects()

    def test_preflight_refresh_rejects_old_anchor_and_default_observe_withdraws(self):
        sid, first = self.preflight_open()
        request = first['production_inspection_source']['diagnosis_request']
        self.peer.inventory_stack += 1
        second = self.preflight_observe(sid)
        self.assertNotEqual(second['production_inspection_source']['anchor'],
                            first['production_inspection_source']['anchor'])
        self.assertNotEqual(second['production_inspection_source']['source_digest'],
                            first['production_inspection_source']['source_digest'])
        self.assert_inspection_refused(self.inspection_request(sid, request))
        # Failure cannot expose the newer source under the old expected anchor.
        self.assert_inspection_refused(self.inspection_request(sid, envelope({'kind': 'production_diagnosis'})))
        fresh = self.preflight_observe(sid)
        reads = self.preflight_operations_reads()
        ordinary = self.client.tool('observe', session_id=sid)
        self.assertTrue(ordinary['ok'], ordinary)
        self.assertIsNone(ordinary.get('production_inspection_source'))
        self.assertEqual(self.preflight_operations_reads(), reads, 'default Observe must not request operations')
        self.assert_inspection_refused(self.inspection_request(
            sid, fresh['production_inspection_source']['diagnosis_request']))
        self.assert_no_preflight_effects()

    def test_preflight_source_pin_drift_withdraws_inspection_without_effects(self):
        sid, first = self.preflight_open()
        self.peer.operations_generation += 1
        refused = self.client.tool('observe', session_id=sid, request='{"include_operations":true}')
        self.assert_inspection_refused(refused)
        self.assert_inspection_refused(self.inspection_request(
            sid, first['production_inspection_source']['diagnosis_request']))
        self.assert_no_preflight_effects()

    def test_preflight_allocator_floor_survives_default_observation_withdrawal(self):
        sid, first = self.preflight_open()
        ordinary = self.client.tool('observe', session_id=sid)
        self.assertTrue(ordinary['ok'], ordinary)
        self.assertIsNone(ordinary.get('production_inspection_source'))

        def rewind_item_allocator(tick, stack):
            payload = bytearray(_fixture.operations_capture(tick, stack))
            jobs_length = struct.unpack_from('>I', payload, 8)[0]
            # The first two fields after the embedded jobs are next-building
            # and next-item IDs. Existing item IDs still fit below 99.
            struct.pack_into('>I', payload, 12 + jobs_length + 4, 99)
            return bytes(payload)

        self.peer.operations_capture = rewind_item_allocator
        refused = self.client.tool('observe', session_id=sid, request='{"include_operations":true}')
        self.assert_inspection_refused(refused)
        self.assert_inspection_refused(self.inspection_request(
            sid, first['production_inspection_source']['diagnosis_request']))
        self.assert_no_preflight_effects()

    def test_preflight_failed_newer_tick_cannot_revive_the_old_operations_source(self):
        sid, first = self.preflight_open()
        old_tick = self.peer.order_tick
        self.peer.order_tick = old_tick + 1
        refused = self.client.tool('observe', session_id=sid, request='{"include_operations":true}')
        self.assert_inspection_refused(refused)
        self.assert_inspection_refused(self.inspection_request(
            sid, first['production_inspection_source']['diagnosis_request']))
        # The consumed newer native tick is retained even when the next double
        # response regresses. The old citizen tick cannot reauthorize that read.
        self.peer.order_tick = old_tick
        regressed = self.client.tool('observe', session_id=sid, request='{"include_operations":true}')
        self.assert_inspection_refused(regressed)
        self.assert_inspection_refused(self.inspection_request(sid, envelope({'kind': 'entities'})))
        self.assert_no_preflight_effects()

    def test_preflight_source_is_withdrawn_at_original_planning_without_baseline_reuse(self):
        sid, observed = self.preflight_open()
        reads = self.preflight_operations_reads()
        request = {'expected_observation': observed['observation']['seal'], 'units': ['43'],
                   'labor': 'MINE', 'enabled': True, 'recipe': 'wooden_bed', 'amount': 1,
                   'deadline_tick': 300, 'poll_interval_ticks': 5, 'stable_samples': 1,
                   'objective': {'kind': 'observed_inventory_minimum', 'item_type': 'BED',
                                 'minimum_stack_units': 1}}
        planned = self.client.tool('plan', session_id=sid, request=json.dumps(request))
        self.assertTrue(planned['ok'], planned)
        self.assertIsNotNone(planned['original_plan'])
        self.assertIsNone(planned.get('production_inspection_source'))
        self.assertIsNone(planned.get('review'))
        self.assertFalse(planned['original_goal_proven'])
        self.assertEqual(self.preflight_operations_reads(), reads,
                         'an inventory minimum neither reacquires nor seals the preflight census')
        self.assert_inspection_refused(self.inspection_request(
            sid, observed['production_inspection_source']['diagnosis_request']))
        self.assert_no_preflight_effects()
