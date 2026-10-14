"""Actual prefix/operations/receipt code with an explicit validated-route input port.

Route decisions in these tests are synthetic; they are not fresh map acquisition.
"""
from copy import deepcopy
from dataclasses import replace
import hashlib
import unittest
from unittest.mock import patch

import construction_frontier as execution
from build_placement_wire import Plan, Record, canonical
from construction_receipt import decode_operations
from test_construction_prefix import receipt, fields, operations, job, idle


def route_result(raw, placed=(), *, status='ready', work_units=100):
    """Explicit route-decision port; full production evidence envelope format."""
    observed = decode_operations(raw, idle)
    rows = []
    for r in placed:
        record = Record.decode(r)
        rows.append(dict(slot=record.plan.key, key=record.plan.key, receipt_digest=r[-32:].hex(),
                         native_plan_digest=record.plan.digest.hex()))
    value = dict(schema='dfmcp.route-furniture-frontier/1', policy=execution.ROUTE_POLICY,
                 status=status, source=dict(inventory_sha256=observed.digest, tick=observed.tick,
                                           horizons=list(observed.horizons)),
                 placed_prefix=rows, all_remaining_selections_checked=True,
                 delivery_suffix={'status': 'checked', 'candidate_model_sequence_proven': status == 'ready'},
                 placement_authorized=False, retry_permitted=False, original_plan_digest='a' * 64,
                 next_step='next', native_before_digest='b' * 64)
    value['evidence_digest'] = hashlib.sha256(execution.DOMAIN + canonical(value)).hexdigest()
    value.update(work_units=work_units, maximum_work=6_000_000)
    return value


def reseal(value):
    body = {k: v for k, v in value.items() if k not in ('work_units', 'maximum_work', 'evidence_digest')}
    value['evidence_digest'] = hashlib.sha256(execution.DOMAIN + canonical(body)).hexdigest()
    return value


class ConstructionFrontierTests(unittest.TestCase):
    def check(self, raw=None, placed=None, route=None, **kwargs):
        placed = (receipt(),) if placed is None else placed
        raw = operations() if raw is None else raw
        route = route_result(raw, placed) if route is None else route
        return execution.gate_frontier(route, placed, raw, idle, **kwargs)

    def test_registration_ready_routes_do_not_imply_ready_construction(self):
        raw = operations(jobs=[job()])
        route = route_result(raw, (receipt(),))
        self.assertEqual(route['status'], 'ready')
        out = self.check(raw)
        self.assertEqual(out['route_status'], 'ready')
        self.assertEqual(out['status'], 'construction_pending')
        self.assertFalse(out['construction_prefix']['predecessor_gate_met'])

    def test_installed_prefix_unlocks_without_rewriting_original_route_or_receipt(self):
        raw, placed = operations(), (receipt(),)
        route = route_result(raw, placed)
        before = deepcopy(route)
        out = self.check(raw, placed, route)
        self.assertEqual(out['status'], 'ready')
        self.assertEqual(route, before)
        self.assertEqual(out['route_evidence_digest'], route['evidence_digest'])
        self.assertNotEqual(out['evidence_digest'], route['evidence_digest'])
        self.assertEqual(out['original_plan_digest'], route['original_plan_digest'])
        self.assertIs(out['placement_authorized'], False)
        self.assertIs(out['retry_permitted'], False)

    def test_missing_and_removed_original_work_do_not_unlock_following_steps(self):
        raw = operations(buildings=[], items=[])
        self.assertEqual(self.check(raw)['status'], 'construction_unestablished')
        raw = operations(jobs=[job(101, kind='DestroyBuilding')])
        self.assertEqual(self.check(raw)['status'], 'construction_blocked')

    def test_completed_construction_never_overrides_blocked_or_unestablished_routes(self):
        for status in ('route_blocked', 'route_unestablished'):
            raw = operations()
            self.assertEqual(self.check(raw, route=route_result(raw, (receipt(),), status=status))['status'], status)

    def test_empty_prefix_still_binds_whole_captured_inventory(self):
        raw = operations(records=())
        out = self.check(raw, ())
        self.assertEqual(out['status'], 'ready')
        self.assertEqual(out['construction_prefix']['status'], 'no_predecessors')
        with self.assertRaises(ValueError):
            self.check(operations(records=(),tick=21), (), route_result(raw))

    def test_exact_capture_tick_and_horizons_must_agree(self):
        raw = operations()
        for key, value in [('inventory_sha256', 'c' * 64), ('tick', 19), ('horizons', [999, 2000, 3000])]:
            report = route_result(raw,(receipt(),))
            report['source'][key] = value
            reseal(report)
            with self.subTest(key=key), self.assertRaises(ValueError):
                self.check(raw, route=report)

    def test_omitted_or_substituted_original_prefix_is_refused(self):
        raw = operations()
        report = route_result(raw,(receipt(),))
        with self.assertRaises(ValueError):
            self.check(raw, (), report)
        r = Record.decode(receipt())
        other = Record(Plan('other', r.plan.before), 'placed','none',r.after,r.insertion).raw
        with self.assertRaises(ValueError):
            self.check(raw, (other,), report)
        for key in ('receipt_digest', 'native_plan_digest', 'key'):
            changed = deepcopy(report); changed['placed_prefix'][0][key] = 'd' * 64;reseal(changed)
            with self.assertRaises(ValueError):
                self.check(raw, route=changed)

    def test_old_or_corrupt_route_evidence_cannot_receive_a_new_confirmation(self):
        raw = operations()
        for changes in [dict(policy='old/1'),dict(schema='foreign/1'),dict(all_remaining_selections_checked=False),dict(status='success')]:
            report = route_result(raw,(receipt(),));report.update(changes);reseal(report)
            with self.assertRaises(ValueError):
                self.check(raw,route=report)
        report = route_result(raw,(receipt(),));report['next_step']='forged'
        with self.assertRaises(ValueError):
            self.check(raw,route=report)

    def test_final_recheck_removal_changes_the_exact_confirmation(self):
        a = self.check()
        b = self.check(operations(jobs=[job(101,kind='DestroyBuilding')]))
        self.assertNotEqual(a['evidence_digest'], b['evidence_digest'])
        self.assertEqual(a['status'],'ready')
        self.assertNotEqual(b['status'],'ready')

    def test_budget_carries_route_cost_and_keeps_semantic_digest_stable(self):
        out = self.check()
        again = self.check(maximum_work=out['work_units'])
        self.assertEqual(out['evidence_digest'],again['evidence_digest'])
        self.assertEqual(out['construction_prefix'],again['construction_prefix'])
        self.assertGreater(out['work_units'],100)
        for budget in [100,out['work_units']-1]:
            with self.assertRaises(ValueError):
                self.check(maximum_work=budget)
        report=route_result(operations(),(receipt(),),work_units=101)
        self.assertEqual(self.check(route=report)['evidence_digest'],out['evidence_digest'])
        self.assertEqual(self.check(route=report)['work_units'],out['work_units']+1)

    def test_input_is_bounded_before_hashing_including_cycles(self):
        raw=operations();base=route_result(raw,(receipt(),))
        cycle={};cycle['self']=cycle
        for extra in [cycle,['x']*32769,'x'*100000,2**100, b'bytes',1.5]:
            value=deepcopy(base);value['extra']=extra
            with self.assertRaises(ValueError):
                self.check(raw,route=value)

    def test_all_original_predecessors_checked_under_one_shared_allowance(self):
        placed=tuple(receipt(i) for i in range(31))
        buildings,items=zip(*(fields(r) for r in placed));buildings=list(buildings)
        buildings[14]['stage']=0
        raw=operations(records=placed,buildings=buildings,items=list(items))
        out=self.check(raw,placed)
        self.assertEqual(out['status'],'construction_pending')
        self.assertEqual(out['construction_prefix']['counts']['quiescent'],30)
        self.assertEqual(out['construction_prefix']['selected_count'],31)
        self.assertLess(len(canonical(out)),160*1024)

    def test_outer_evidence_seals_the_complete_new_decision(self):
        out = self.check()
        body = {k: v for k, v in out.items() if k not in ('work_units', 'maximum_work', 'evidence_digest')}
        self.assertEqual(out['evidence_digest'], hashlib.sha256(execution.DOMAIN + canonical(body)).hexdigest())
        self.assertIn('construction_prefix', body)

    def test_final_guard_and_serialization_failure_publish_nothing(self):
        raw=operations();report=route_result(raw,(receipt(),));revoked=False
        original=execution.canonical
        def encode(value):
            nonlocal revoked
            result=original(value)
            if value.get('policy')==execution.POLICY and 'maximum_work' in value:
                revoked=True
            return result
        def guard():
            if revoked:raise ValueError('revoked')
        with patch.object(execution,'canonical',side_effect=encode),self.assertRaises(ValueError):
            execution.gate_frontier(report,(receipt(),),raw,guard)
        with patch.object(execution,'MAX_OUTPUT',1),self.assertRaises(ValueError):
            self.check()


if __name__ == '__main__':
    unittest.main()
