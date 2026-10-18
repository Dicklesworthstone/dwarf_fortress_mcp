"""Independent bounded-size comparisons with actual canonical receipt values.

Size probes never satisfy evidence or prefix admission. These are pure component
cases; TCP/private-file coverage lives in test_furniture_prefix_tcp.py.
"""
from __future__ import annotations

from copy import deepcopy
from dataclasses import replace
import itertools
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
import furniture_batch as batch
import furniture_output_bounds as bounds
import furniture_prefix as prefix
import run_furniture_batch as runner
from furniture_plan import FurniturePlan, Step, canonical, progress
from build_placement_wire import Capture, Item, Plan, Record, Selection, Tile, MAX_I32, MIN_I32
from furniture_recovery_peer import placed


def capture(step, index):
    floor = Tile(2, 42, 3)
    selected = Selection(('bed', 'chair', 'table').index(step.kind) + 1, step.item, *step.target)
    item = Item(presence=2, pos=(1, 1, 2), kind=selected.kind, native_type=MAX_I32,
                subtype=-1, material=MAX_I32, material_index=MIN_I32,
                on_ground=True, ground=floor)
    return Capture(2**64 - 2, index, 1700000000000000, MAX_I32, (32768, 32768, 32768),
                   MAX_I32 - 33 + index, MAX_I32 - 33 + index, 65500 + index,
                   'size-fixture', True, True, True, selected, (floor,) * 9, item)


def fixture(count, completed=0, dense=False):
    names = [f'item{i:02}' + ('-' + 'a' * 41 if dense else '') for i in range(count)]
    plan = FurniturePlan(tuple(Step(name, ('bed', 'chair', 'table')[i % 3], MAX_I32 - 32 + i,
        (32766 - 3 * i, 32766, 32767), tuple(names[max(0, i - 8):i]) if dense else ())
        for i, name in enumerate(names)))
    states = {s.name: 'placed' for s in plan.ordered[:completed]}
    out = progress(plan, states)
    keys = {s.name: 'k' * (128 - len(s.name)) + s.name for s in plan.steps}
    for i, row in enumerate(out['steps']):
        row.pop('dependencies')
        if i < completed:
            row.update(row_for(plan.ordered[i], keys[plan.ordered[i].name], i, 'placed'))
    out.update(batch_id='a' * 64, stopped=False, inventory_verified=True,
               advance_allowed=completed < count, source={'generation': 2**64 - 2,
               'df_version': '\U0001f680' * 32, 'dfhack_version': '\U0001f680' * 32},
               endpoint='127.0.0.1:65535', native_contacted=False, inventory_digest='b' * 64)
    return plan, keys, out


def row_for(step, key, index, phase):
    before = capture(step, index)
    plan = Plan(key, before)
    record = None if phase == 'unknown' else (
        placed(plan) if phase == 'placed' else Record(plan, phase,
            {'refused': 'stale', 'cancelled': 'cancelled', 'indeterminate': 'native_failure'}[phase]))
    if record is not None:
        record = Record.decode(record.raw, plan)
    return {'name': step.name, 'phase': phase, 'key': key, 'plan_digest': plan.digest.hex(),
            'registered': True, 'receipt_digest': None if record is None else record.raw[-32:].hex(),
            'insertion': record.insertion.view() if record is not None and record.insertion is not None else None,
            'dispatch_recorded': phase in ('unknown', 'placed', 'indeterminate')}


def after(state, plan, keys, phases):
    out = deepcopy(state)
    start = state['placed']
    for i, phase in enumerate(phases, start):
        out['steps'][i] = row_for(plan.ordered[i], keys[plan.ordered[i].name], i, phase)
    actual = progress(plan, {r['name']: r['phase'] for r in out['steps'] if r['phase'] != 'not_started'})
    for field in ('status', 'next_step', 'pending_step', 'placed', 'total'):
        out[field] = actual[field]
    out.update(advance_allowed=out['status'] == 'ready', native_contacted=True, inventory_digest='c' * 64)
    return out


class OutputBoundsTests(unittest.TestCase):
    def test_real_maximum_width_receipt_rows_fit_every_single_step_phase(self):
        cases = 0
        for count, dense in itertools.product((1, 2, 8, 16, 32), (False, True)):
            for completed in sorted({0, count // 2, count - 1}):
                plan, keys, before = fixture(count, completed, dense)
                step = plan.ordered[completed]
                probe = bounds.inventory_bound(before, (step,))
                probe['advanced_step'] = step.name
                maximum = len(batch.encoded('advance', probe, False)) + bounds.PRESENTATION_MARGIN
                for phase in ('placed', 'unknown', 'indeterminate', 'refused', 'cancelled'):
                    out = after(before, plan, keys, [phase])
                    out['advanced_step'] = step.name
                    self.assertLessEqual(len(batch.encoded('advance', out)), maximum)
                    cases += 1
        self.assertEqual(cases, 120)

    def test_complete_prefix_with_each_failure_position_fits_reserved_envelope(self):
        cases = 0
        for count in (1, 8, 32):
            for completed in sorted({0, count // 2, count - 1}):
                plan, keys, before = fixture(count, completed, True)
                selected_count = min(8, count - completed)
                selected = prefix.Prefix.create(before, keys, selected_count)
                captured = []
                class Encoder:
                    encoded = staticmethod(lambda operation, value, ok=True:
                        captured.append(batch.encoded(operation, value, ok)) or captured[-1])
                    output_limit = staticmethod(batch.output_limit)
                runner.reserve(Encoder, before, selected, runner.Budget(60000, selected_count))
                maximum = len(captured[-1]) + bounds.PRESENTATION_MARGIN
                for length in range(1, selected_count + 1):
                    for terminal in ('placed', 'unknown', 'indeterminate', 'refused', 'cancelled'):
                        out = after(before, plan, keys, ['placed'] * (length - 1) + [terminal])
                        outcomes = [{'name': s.name, 'key': keys[s.name], 'item': s.item,
                                     'phase': out['steps'][completed + i]['phase'],
                                     'dispatch_recorded': True, 'blockers': []}
                                    for i, s in enumerate(plan.ordered[completed:completed + length])]
                        result = runner.result(out, selected, outcomes, 'native_cancelled',
                                               runner.Budget(60000, selected_count))
                        result['prefix_run'].update(rpc_calls_used=selected_count * 19,
                            native_bytes_used=selected_count * runner.WIRE_PER_STEP,
                            work_checks_used=runner.MAX_WORK, steps_admitted=selected_count)
                        self.assertLessEqual(len(batch.encoded('run_prefix', result, False)), maximum)
                        cases += 1
        self.assertEqual(cases, 155)

    def test_bounded_blocker_list_includes_json_escaping_and_stops_only_once(self):
        for count in range(1, 9):
            plan, keys, state = fixture(8)
            selected = prefix.Prefix.create(state, keys, count)
            outcomes = bounds.prefix_outcomes_bound(selected.json()['steps'])
            self.assertEqual(len(canonical(outcomes[-1]['blockers'])), bounds.MAX_BLOCKER_BYTES)
            self.assertTrue(all(not row['blockers'] for row in outcomes[:-1]))
        # Check actual native predicate vocabulary independently against the bound.
        step = Step('one', 'bed', 1, (20, 20, 2))
        value = capture(step, 0)
        rejected = replace(value, paused=False, free_tile=False, supported=False,
            sequence=2**64 - 2, next_building=MAX_I32, next_job=MAX_I32,
            building_count=65536, tiles=(Tile(1),) * 9, item=Item())
        blockers = list(rejected.blockers) + ['native_unresolved', 'native_retention_full']
        self.assertLessEqual(len(canonical(blockers)), bounds.MAX_BLOCKER_BYTES)

    def test_original_state_is_not_mutated_and_size_probe_is_never_valid_evidence(self):
        plan, _, state = fixture(32, 24, True)
        before = canonical(state)
        probe = bounds.inventory_bound(state, plan.ordered[24:])
        self.assertEqual(canonical(state), before)
        self.assertEqual(probe['plan'], state['plan'])
        self.assertEqual(probe['steps'][:24], state['steps'][:24])
        with self.assertRaises(ValueError):
            prefix.inventory(probe)
        with self.assertRaises(ValueError):
            bounds.inventory_bound(state, (plan.ordered[0],))

    def test_room_terrain_metadata_remains_exact_in_sizing(self):
        plan, _, state = fixture(2)
        state.update(room_origin={'digest': 'd' * 64}, room_plan={'original': 'retained'},
            terrain_origin={'journal': 'e' * 64}, terrain_history_verified_this_call=True,
            terrain_history_required_for_new_work=True)
        probe = bounds.inventory_bound(state, plan.ordered)
        for field in ('room_origin', 'room_plan', 'terrain_origin', 'terrain_history_verified_this_call'):
            self.assertEqual(probe[field], state[field])
        # This is size-only metadata preservation, not live room/terrain evidence.
        self.assertLessEqual(len(batch.encoded('advance', probe)), batch.MAX_ROOM_OUTPUT)

    def test_contract_matches_output_bounds_and_never_grants_evidence(self):
        import json
        contract = json.loads((Path(__file__).resolve().parents[1] /
            'architecture/furniture_prefix_v1.json').read_text())
        self.assertEqual(contract['maximum_blocker_json_bytes'], bounds.MAX_BLOCKER_BYTES)
        self.assertEqual(contract['output_presentation_margin_bytes'], bounds.PRESENTATION_MARGIN)
        self.assertIs(contract['size_probes_are_evidence'], False)
        self.assertEqual((batch.MAX_OUTPUT, batch.MAX_ROOM_OUTPUT), (65536, 196608))

    def test_structural_selection_bounds_refuse(self):
        plan, _, state = fixture(16)
        for selected in ((), plan.ordered[:9], list(plan.ordered[:1]), (plan.ordered[0],) * 2):
            with self.assertRaises(ValueError):
                bounds.inventory_bound(state, selected)
        for selected in ([], [{}] * 9, ()):
            with self.assertRaises(ValueError):
                bounds.prefix_outcomes_bound(selected)


if __name__ == '__main__':
    unittest.main()
