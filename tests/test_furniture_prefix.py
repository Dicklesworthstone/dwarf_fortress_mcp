"""Pure prefix contract tests using the existing actual furniture DAG reducer."""
from copy import deepcopy
import hashlib
import itertools
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
from furniture_plan import FurniturePlan, Step, canonical, progress
from furniture_prefix import Prefix, inventory, same_inventory, MAX_STEPS


def fixture(count=4, placed=0, room=False, terrain=False):
    plan = FurniturePlan(tuple(Step(f's{i:02d}', ('bed', 'chair', 'table')[i % 3], i,
                                  (10 + i, 20, 2), (f's{i-1:02d}',) if i else ())
                               for i in range(count)))
    keys = {s.name: 'fr-' + 'a' * 64 + '-' + s.name for s in plan.steps}
    state = progress(plan, {s.name: 'placed' for s in plan.ordered[:placed]})
    for i, row in enumerate(state['steps']):
        row.pop('dependencies')
        if i < placed:
            row.update(key=keys[row['name']], plan_digest=f'{100+i:064x}', registered=True,
                       receipt_digest=f'{200+i:064x}', insertion={'building_id': 300+i, 'job_id': 400+i},
                       dispatch_recorded=True)
    state.update(batch_id='a' * 64, stopped=False, inventory_verified=True,
                 advance_allowed=placed < count, source={'generation': 1, 'df_version': 'fixture',
                     'dfhack_version': 'fixture'}, endpoint='127.0.0.1:5000', native_contacted=False)
    if room or terrain:
        state.update(room_origin={'digest': 'b' * 64}, room_plan={'original': 'complete'},
                     allocation={'digest': 'c' * 64})
    if terrain:
        state.update(terrain_origin={'digest': 'd' * 64}, terrain_history_verified_this_call=True,
                     terrain_history_required_for_new_work=True)
    resign(state)
    return state, keys


def resign(state):
    state['inventory_digest'] = hashlib.sha256(canonical({k: v for k, v in state.items()
        if k not in ('inventory_digest', 'native_contacted')})).hexdigest()
    return state


def advance(state, keys, phase='placed'):
    result = deepcopy(state)
    index = result['placed']
    row = result['steps'][index]
    row.update(phase=phase, key=keys[row['name']], plan_digest=f'{100+index:064x}', registered=True,
               receipt_digest=f'{200+index:064x}' if phase != 'unknown' else None,
               insertion={'building_id': 300+index, 'job_id': 400+index} if phase == 'placed' else None,
               dispatch_recorded=True)
    plan = FurniturePlan.from_json(result['plan'])
    expected = progress(plan, {r['name']: r['phase'] for r in result['steps'] if r['phase'] != 'not_started'})
    for field in ('status', 'placed', 'next_step', 'pending_step'):
        result[field] = expected[field]
    result['advance_allowed'] = result['status'] == 'ready' and not result['stopped']
    result['native_contacted'] = True
    return resign(result)


class FurniturePrefixTests(unittest.TestCase):
    def test_entire_selected_dag_and_exact_items_are_retained(self):
        state, keys = fixture()
        prefix = Prefix.create(state, keys, 3)
        self.assertEqual(prefix.json()['steps'], [{**s, 'key': keys[s['name']]}
                         for s in state['plan']['steps'][:3]])
        self.assertEqual(prefix.json()['original_step_count'], 4)
        prefix.confirm(state['batch_id'], prefix.seal)
        self.assertFalse(prefix.json()['mutation_authority_granted'])

    def test_nonlexical_dependency_order_uses_actual_kahn_reducer(self):
        state, _ = fixture(3)
        plan = FurniturePlan((Step('a', 'table', 1, (20, 20, 2), ('z',)),
                              Step('z', 'chair', 2, (21, 20, 2)),
                              Step('b', 'bed', 3, (22, 20, 2), ('a',))))
        state.update(progress(plan, {}))
        for row in state['steps']:
            row.pop('dependencies')
        keys = {s.name: 'key-' + s.name for s in plan.steps}
        resign(state)
        self.assertEqual([s['name'] for s in Prefix.create(state, keys, 3).json()['steps']], ['z', 'a', 'b'])

    def test_all_original_prefix_lengths_and_offsets(self):
        for count in range(1, 33):
            for placed in range(count):
                for take in range(1, min(MAX_STEPS, count - placed) + 1):
                    state, keys = fixture(count, placed)
                    prefix = Prefix.create(state, keys, take)
                    self.assertEqual(prefix.json()['start_index'], placed)
                    self.assertEqual(len(prefix.json()['steps']), take)
                    after = advance(state, keys)
                    self.assertEqual(prefix.successor(state, after), 'placed')

    def test_eight_steps_in_complete_32_item_project(self):
        state, keys = fixture(32, 17, terrain=True)
        prefix = Prefix.create(state, keys, 8)
        for _ in range(8):
            after = advance(state, keys)
            self.assertEqual(prefix.successor(state, after), 'placed')
            state = after
        with self.assertRaises(ValueError):
            prefix.successor(state, advance(state, keys))
        self.assertEqual(state['placed'], 25)
        self.assertEqual(state['total'], 32)

    def test_every_nonplaced_phase_stops_the_prefix(self):
        for phase in ('unknown', 'indeterminate', 'refused', 'cancelled'):
            state, keys = fixture()
            prefix = Prefix.create(state, keys, 4)
            after = advance(state, keys, phase)
            self.assertEqual(prefix.successor(state, after), phase)
            with self.assertRaises(ValueError):
                Prefix.create(after, keys, 1)
            with self.assertRaises(ValueError):
                prefix.successor(after, after)

    def test_repeated_or_skipped_effects_refuse(self):
        state, keys = fixture()
        prefix = Prefix.create(state, keys, 4)
        once = advance(state, keys)
        for invalid in (state, advance(once, keys)):
            with self.assertRaises(ValueError):
                prefix.successor(state, invalid)
        with self.assertRaises(ValueError):
            prefix.successor(once, once)

    def test_changed_receipt_or_dispatch_history_refuses(self):
        state, keys = fixture(4, 1)
        prefix = Prefix.create(state, keys, 2)
        for field, value in (('receipt_digest', 'f'*64), ('plan_digest', 'e'*64),
                             ('dispatch_recorded', False), ('insertion', {'building_id': 999})):
            after = advance(state, keys)
            after['steps'][0][field] = value
            resign(after)
            with self.subTest(field=field), self.assertRaises(ValueError):
                prefix.successor(state, after)

    def test_immutable_source_room_and_terrain_changes_refuse(self):
        state, keys = fixture(4, 0, terrain=True)
        prefix = Prefix.create(state, keys, 2)
        for field in ('source', 'endpoint', 'allocation', 'room_origin', 'room_plan', 'terrain_origin'):
            after = advance(state, keys)
            after[field] = 'changed'
            resign(after)
            with self.subTest(field=field), self.assertRaises(ValueError):
                prefix.successor(state, after)

    def test_original_terrain_history_is_required_without_renewal(self):
        state, keys = fixture(terrain=True)
        for field in ('terrain_history_verified_this_call', 'terrain_history_required_for_new_work'):
            value = deepcopy(state)
            value[field] = False
            value['advance_allowed'] = False
            with self.assertRaises(ValueError):
                Prefix.create(value, keys, 1)

    def test_review_seals_change_for_count_selection_and_original_inventory(self):
        state, keys = fixture()
        original = Prefix.create(state, keys, 1)
        changed = deepcopy(state)
        changed['source']['generation'] += 1
        choices = [Prefix.create(state, keys, 2), Prefix.create(changed, keys, 1)]
        changed_keys = dict(keys, s00='another-original-key')
        choices.append(Prefix.create(state, changed_keys, 1))
        for prefix in choices:
            self.assertNotEqual(prefix.seal, original.seal)
            with self.assertRaises(ValueError):
                prefix.confirm(state['batch_id'], original.seal)

    def test_native_contact_disclosure_does_not_reseal_durable_inventory(self):
        state, keys = fixture()
        changed = deepcopy(state)
        changed['native_contacted'] = True
        same_inventory(state, changed)
        self.assertEqual(Prefix.create(state, keys, 1), Prefix.create(changed, keys, 1))

    def test_shape_count_and_progress_types_are_exact(self):
        state, keys = fixture()
        for count in (True, False, 1.0, '1', 0, -1, 9, None):
            with self.subTest(count=count), self.assertRaises(ValueError):
                Prefix.create(state, keys, count)
        for field, value in (('placed', False), ('total', 4.0), ('stopped', 0),
                             ('advance_allowed', 1), ('inventory_verified', 1), ('unknown', 1)):
            changed = deepcopy(state)
            changed[field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                Prefix.create(changed, keys, 1)

    def test_insufficient_remaining_count_is_not_clamped(self):
        state, keys = fixture(4, 3)
        with self.assertRaises(ValueError):
            Prefix.create(state, keys, 2)

    def test_stopped_complete_and_unregistered_inventories_refuse(self):
        state, keys = fixture()
        state.update(stopped=True, advance_allowed=False)
        with self.assertRaises(ValueError):
            Prefix.create(state, keys, 1)
        state, keys = fixture(4, 4)
        with self.assertRaises(ValueError):
            Prefix.create(state, keys, 1)
        state, keys = fixture(4, 1)
        state['steps'][0]['registered'] = False
        with self.assertRaises(ValueError):
            Prefix.create(state, keys, 1)

    def test_incomplete_keys_duplicate_keys_and_original_key_replacement_refuse(self):
        state, keys = fixture(4, 1)
        for invalid in ({}, dict(keys, extra='extra'), dict.fromkeys(keys, 'same'),
                        dict(keys, s00='replacement'), dict(keys, s01='\0')):
            with self.assertRaises(ValueError):
                Prefix.create(state, invalid, 1)

    def test_future_effect_or_partial_plan_cannot_hide_behind_rehashed_inventory(self):
        state, keys = fixture()
        changed = advance(state, keys)
        changed['steps'][0], changed['steps'][1] = changed['steps'][1], changed['steps'][0]
        resign(changed)
        with self.assertRaises(ValueError):
            inventory(changed)
        changed = deepcopy(state)
        changed['steps'].pop()
        resign(changed)
        with self.assertRaises(ValueError):
            inventory(changed)

    def test_first_advance_binds_full_reviewed_inventory(self):
        state, keys = fixture()
        prefix = Prefix.create(state, keys, 2)
        changed = deepcopy(state)
        changed['inventory_digest'] = 'e'*64
        with self.assertRaises(ValueError):
            prefix.successor(changed, advance(changed, keys))

    def test_inputs_and_decoded_results_do_not_alias(self):
        state, keys = fixture()
        before = deepcopy(state)
        prefix = Prefix.create(state, keys, 2)
        prefix.json()['steps'].clear()
        self.assertEqual(len(prefix.json()['steps']), 2)
        self.assertEqual(state, before)

    def test_cancellation_at_every_policy_checkpoint(self):
        state, keys = fixture(4, 1)
        calls = []
        Prefix.create(state, keys, 2, lambda: calls.append(1))
        for stop in range(1, len(calls) + 1):
            seen = 0
            def guard():
                nonlocal seen
                seen += 1
                if seen == stop:
                    raise InterruptedError('cancel policy')
            with self.assertRaises(InterruptedError):
                Prefix.create(state, keys, 2, guard)
        prefix = Prefix.create(state, keys, 2)
        after = advance(state, keys)
        with self.assertRaisesRegex(ValueError, 'caller interruption'):
            prefix.successor(state, after, lambda: (_ for _ in ()).throw(ValueError('caller interruption')))

    def test_topological_input_permutations_have_one_plan_and_prefix(self):
        state, keys = fixture(4)
        original = Prefix.create(state, keys, 4)
        for order in itertools.permutations(state['plan']['steps']):
            changed = deepcopy(state)
            changed['plan'] = FurniturePlan.from_json({'schema': 'dfmcp.furniture-plan/1', 'steps': list(order)}).json()
            self.assertEqual(Prefix.create(changed, dict(reversed(list(keys.items()))), 4), original)


if __name__ == '__main__':
    unittest.main()
