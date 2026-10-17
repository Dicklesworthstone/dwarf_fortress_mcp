"""Exact original-prefix confirmation and non-retry transitions; no game effects."""
from copy import deepcopy
import hashlib
import json
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
import dig_blueprint_prefix as p


def regions(count=3):
    return [dict(x=1 + index * 3, y=4, z=2, width=2, height=2) for index in range(count)]


def state(count=3):
    value = dict(batch_id='1' * 64, layout_digest='2' * 64, blueprint_digest='3' * 64,
        phase='ready', stopped=False, total_steps=count, target_tiles=count * 4,
        designated_steps=0, next_step=0, unresolved_step=None, minimum_tick=100,
        minimum_sequence=0, records=[], excavation_completion_proven=False,
        current_terrain_proven=False, global_controller_fence=False,
        checkpoint_verified=False, retry_commit_permitted=False)
    return seal(value)


def seal(value):
    value['inventory_digest'] = p.inventory_identity(value)
    return value


def add(previous, outcome='designated'):
    value = deepcopy(previous)
    index = len(value['records'])
    value['records'].append(dict(step=index, key=f"bp-{value['batch_id']}-{index:03d}",
        region=regions(value['total_steps'])[index], state=outcome, witness='4' * 64,
        plan_digest='5' * 64, intent_sha256='6' * 64,
        terminal_receipt_sha256=None if outcome == 'unknown' else '7' * 64))
    value['minimum_tick'] += 1
    value['minimum_sequence'] += int(outcome == 'designated')
    value['designated_steps'] += int(outcome == 'designated')
    value['phase'] = ('unresolved' if outcome == 'unknown' else 'refused') if outcome != 'designated' else (
        'designations_verified' if index + 1 == value['total_steps'] else 'ready')
    value['next_step'] = index + 1 if value['phase'] == 'ready' else None
    value['unresolved_step'] = index if outcome == 'unknown' else None
    return seal(value)


def review(value=None, count=2, **changes):
    value = state() if value is None else value
    return p.Prefix.create(value, regions(value['total_steps']), count,
        **dict(allow_hidden_neighbors=False, checkpoint_policy=p.CHECKPOINT_POLICY, **changes))


class PrefixTests(unittest.TestCase):
    def test_review_covers_exact_original_prefix_and_complete_batch_identity(self):
        value = review().json()
        self.assertEqual(value['start_step'], 0)
        self.assertEqual(value['step_count'], 2)
        self.assertEqual(value['original_steps'], 3)
        self.assertEqual([s['region'] for s in value['steps']], regions()[:2])
        self.assertEqual([s['key'] for s in value['steps']], ['bp-' + '1' * 64 + '-000', 'bp-' + '1' * 64 + '-001'])
        for key in ('batch_id', 'inventory_digest', 'layout_digest', 'blueprint_digest'):
            self.assertEqual(value[key], state()[key])
        self.assertEqual(value['checkpoint_policy'], p.CHECKPOINT_POLICY)

    def test_review_is_immutable_and_returned_data_never_aliases_inputs(self):
        before, source = state(), regions()
        prefix = p.Prefix.create(before, source, 2, allow_hidden_neighbors=False, checkpoint_policy=p.CHECKPOINT_POLICY)
        original = p.canonical(prefix.json())
        source[0]['x'] = 20
        before['records'].append({})
        value = prefix.json()
        value['steps'].clear()
        self.assertEqual(p.canonical(prefix.json()), original)

    def test_confirmation_matches_only_the_whole_prefix_seal(self):
        selected = review()
        selected.confirm('1' * 64, selected.seal)
        for identity in ('2' * 64, '0' * 64, 'bad', True):
            with self.assertRaises(ValueError):
                selected.confirm(identity, selected.seal)
            with self.assertRaises(ValueError):
                selected.confirm('1' * 64, identity)

    def test_per_step_review_domain_cannot_authorize_a_run(self):
        selected = review()
        wrong = hashlib.sha256(b'dfmcp-dig-blueprint-review/1\0' + selected._body).hexdigest()
        with self.assertRaises(ValueError):
            selected.confirm('1' * 64, wrong)

    def test_changed_frontier_requires_a_new_confirmation(self):
        initial = review(count=1)
        changed = review(add(state()), count=1)
        self.assertNotEqual(initial.seal, changed.seal)
        with self.assertRaises(ValueError):
            changed.confirm('1' * 64, initial.seal)
        self.assertEqual(changed.json()['start_step'], 1)

    def test_count_is_exact_not_silently_clipped_or_widened(self):
        for count in (True, False, None, '2', 2.0, 0, -1, 9, 4):
            with self.subTest(count=count), self.assertRaises(ValueError):
                review(count=count)
        with self.assertRaises(ValueError):
            review(add(add(state())), count=2)
        self.assertNotEqual(review(count=1).seal, review(count=2).seal)

    def test_all_eight_selected_steps_and_128_original_steps_are_retained(self):
        value = state(128)
        prefix = review(value, 8).json()
        self.assertEqual(prefix['original_steps'], 128)
        self.assertEqual(prefix['original_target_tiles'], 512)
        self.assertEqual(len(prefix['steps']), 8)
        self.assertLessEqual(len(p.canonical(prefix)), p.MAX_REVIEW)

    def test_refused_unknown_stopped_and_finished_batches_cannot_restart(self):
        values = [add(state(), 'unknown'), add(state(), 'refused'), add(state(1))]
        stopped = state(); stopped.update(stopped=True, next_step=None); seal(stopped); values.append(stopped)
        for value in values:
            with self.subTest(phase=value['phase']), self.assertRaises(ValueError):
                review(value, 1)

    def test_manifest_policy_cannot_be_silently_changed(self):
        for policy in (None, '', 'checkpointed', True):
            with self.assertRaises(ValueError):
                p.Prefix.create(state(), regions(), 1, allow_hidden_neighbors=False, checkpoint_policy=policy)
        for hidden in (1, 'false', None):
            with self.assertRaises(ValueError):
                p.Prefix.create(state(), regions(), 1, allow_hidden_neighbors=hidden, checkpoint_policy=p.CHECKPOINT_POLICY)
        allowed = p.Prefix.create(state(), regions(), 1, allow_hidden_neighbors=True, checkpoint_policy=p.CHECKPOINT_POLICY)
        self.assertNotEqual(allowed.seal, review(count=1).seal)

    def test_inventory_checksum_is_recomputed_not_trusted(self):
        for field, value in (('minimum_tick', 101), ('inventory_digest', '0' * 64), ('target_tiles', 11)):
            changed = state(); changed[field] = value
            with self.assertRaises(ValueError):
                review(changed)

    def test_rehashed_inventory_cannot_lie_about_counts_frontier_or_status(self):
        cases = [('phase', 'designations_verified'), ('next_step', True), ('next_step', 2),
                 ('unresolved_step', 0), ('designated_steps', 1), ('minimum_sequence', -1),
                 ('minimum_tick', True), ('retry_commit_permitted', True), ('stopped', 0),
                 ('excavation_completion_proven', True), ('extra', 1)]
        for field, value in cases:
            changed = state(); changed[field] = value; seal(changed)
            with self.subTest(field=field), self.assertRaises(ValueError):
                review(changed)

    def test_original_mask_and_halo_are_admitted_before_review(self):
        cases = []
        for field, value in (('x', 0), ('z', 32767), ('width', 9), ('height', True), ('extra', 1)):
            changed = regions(); changed[0][field] = value; cases.append(changed)
        cases += [regions()[:-1], regions() + regions()[:1], tuple(regions())]
        changed = regions(); changed[1] = changed[0]; cases.append(changed)
        for value in cases:
            with self.assertRaises(ValueError):
                p.Prefix.create(state(), value, 2, allow_hidden_neighbors=False, checkpoint_policy=p.CHECKPOINT_POLICY)

    def test_existing_record_must_use_original_region_and_key(self):
        initial = add(state())
        for field, value in (('step', True), ('key', 'other'), ('region', regions()[2]),
                             ('state', 'prepared'), ('terminal_receipt_sha256', None)):
            changed = deepcopy(initial); changed['records'][0][field] = value; seal(changed)
            with self.subTest(field=field), self.assertRaises(ValueError):
                review(changed, 1)

    def test_successor_requires_exactly_one_additional_verified_record(self):
        before = state()
        selected = review(before)
        first = add(before)
        self.assertEqual(selected.successor(before, first), 'designated')
        second = add(first)
        self.assertEqual(selected.successor(first, second), 'designated')
        for next_value in (before, second):
            with self.assertRaises(ValueError):
                selected.successor(before, next_value)
        with self.assertRaises(ValueError):
            selected.successor(second, add(second))

    def test_unknown_and_refused_successors_are_retained_but_never_unlocked(self):
        for outcome in ('unknown', 'refused'):
            selected = review()
            stopped = add(state(), outcome)
            self.assertEqual(selected.successor(state(), stopped), outcome)
            with self.assertRaises(ValueError):
                selected.successor(stopped, add(stopped))

    def test_successor_cannot_rewrite_an_earlier_receipt(self):
        before = add(state())
        selected = review(before, 1)
        changed = add(before)
        changed['records'][0]['witness'] = 'a' * 64
        seal(changed)
        with self.assertRaises(ValueError):
            selected.successor(before, changed)

    def test_successor_cannot_substitute_source_plan_or_targets(self):
        before = state(); selected = review(before)
        for key in ('batch_id', 'layout_digest', 'blueprint_digest'):
            changed = add(before); changed[key] = 'a' * 64; seal(changed)
            with self.assertRaises(ValueError):
                selected.successor(before, changed)
        changed = add(before); changed['records'][0]['region'] = regions()[1]; seal(changed)
        with self.assertRaises(ValueError):
            selected.successor(before, changed)

    def test_native_floor_regression_never_unlocks_next_step(self):
        before = add(state()); selected = review(before, 1)
        for field in ('minimum_tick', 'minimum_sequence'):
            changed = add(before); changed[field] = before[field] - 1; seal(changed)
            with self.assertRaises(ValueError):
                selected.successor(before, changed)

    def test_complete_room_identity_is_preserved_across_progress(self):
        before = state(); before.update(room_excavation={'room_plan': {'original': 'fixture'}}, batch_format='dfmcp.dig-blueprint-batch/2'); seal(before)
        selected = review(before)
        changed = add(before)
        self.assertEqual(selected.successor(before, changed), 'designated')
        changed['room_excavation']['room_plan']['original'] = 'replacement'; seal(changed)
        with self.assertRaises(ValueError):
            selected.successor(before, changed)

    def test_cancellation_at_each_review_checkpoint_propagates(self):
        calls = []
        review(guard=lambda: calls.append(1))
        for limit in range(1, len(calls) + 1):
            count = 0
            def guard():
                nonlocal count
                count += 1
                if count == limit:
                    raise InterruptedError('stop original run')
            with self.assertRaises(InterruptedError):
                review(guard=guard)

    def test_first_transition_cannot_start_on_another_inventory(self):
        selected = review()
        changed = state(); changed['minimum_tick'] += 1; seal(changed)
        with self.assertRaises(ValueError):
            selected.successor(changed, add(changed))

    def test_output_order_and_review_identity_are_deterministic(self):
        value = state()
        alternate = dict(reversed(list(value.items())))
        self.assertEqual(review(value)._body, review(alternate)._body)
        self.assertEqual(review().seal, review().seal)
        result = review().json()
        for key in ('atomic', 'automatic_recovery_or_retry', 'mutation_authority_granted',
                    'native_terrain_observed', 'excavation_completion_proven'):
            self.assertIs(result[key], False)


if __name__ == '__main__':
    unittest.main()
