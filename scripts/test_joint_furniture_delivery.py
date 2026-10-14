"""Execute the actual joint solver against independent assignment/order oracles.

All maps and items are explicit data, not native acquisitions or live evidence.
"""
from dataclasses import replace
import hashlib
import itertools
import random
import unittest

from furniture_allocation import Candidate, Request, Slot
from furniture_plan import FurniturePlan, canonical
import route_furniture_allocation as routes
import route_furniture_joint as joint
import route_furniture_sequence as sequence
import route_furniture_review as reviews
from test_cross_level_furniture import terrain, item, oracle_distances
from test_cross_level_furniture_review import source


def blocked_choice():
    raw, cells = terrain(size=(7, 1, 1), stairs=False)
    request = Request('fort', 1, (Slot('a', 'chair', (3, 1, 0)),
        Slot('b', 'bed', (4, 1, 0), after=('a',))))
    candidates = (item(1, (1, 1, 0), kind='chair'), item(2, (2, 1, 0)), item(3, (7, 1, 0)))
    return request, candidates, raw, cells


def compatible(slot, candidate, excluded):
    # Do not call the production selector while validating search completeness.
    return (candidate.id not in excluded and candidate.kind == slot.kind
        and (slot.material is None or candidate.material == slot.material)
        and (slot.subtype is None or candidate.subtype == slot.subtype)
        and abs(candidate.position[2] - slot.target[2]) <= slot.max_vertical_distance
        and sum(abs(a-b) for a, b in zip(candidate.position, slot.target)) <= slot.max_distance)


def feasible(request, assignment, cells, order, limit):
    slots = {slot.name: slot for slot in request.slots}
    if len(set(candidate.id for candidate in assignment.values())) != len(slots):
        return False
    current, done = dict(cells), set()
    for name in order:
        slot, candidate = slots[name], assignment[name]
        if not set(slot.after) <= done or not compatible(slot, candidate, request.excluded_items):
            return False
        lengths = oracle_distances(current, slot.target)
        if candidate.position not in lengths or lengths[candidate.position] > limit:
            return False
        current[slot.target] = [2, 3, 0, 1, 0, 1]
        done.add(name)
    return done == set(slots)


def oracle(request, candidates, cells, limit):
    names = tuple(slot.name for slot in request.slots)
    for chosen in itertools.permutations(candidates, len(names)):
        if not all(compatible(slot, candidate, request.excluded_items)
                   for slot, candidate in zip(request.slots, chosen)):
            continue
        assignment = dict(zip(names, chosen))
        for order in itertools.permutations(names):
            if feasible(request, assignment, cells, order, limit):
                return True
    return False


class JointDeliveryTests(unittest.TestCase):
    def verify_plan(self, request, candidates, cells, effective, result, limit):
        plan = FurniturePlan.from_json(result['plan'])
        by_id = {candidate.id: candidate for candidate in candidates}
        chosen = {step.name: by_id[step.item] for step in plan.steps}
        order = tuple(step.name for step in plan.ordered)
        self.assertTrue(feasible(request, chosen, cells, order, limit))
        self.assertEqual(result['delivery_sequence']['order'], list(order))
        self.assertEqual(result['request_digest'], effective.digest)
        self.assertEqual(result['plan_digest'], plan.digest)
        for before, after in zip(request.slots, effective.slots):
            self.assertEqual(replace(after, after=before.after), before)
            self.assertTrue(set(before.after) <= set(after.after))
        unsigned = {k: v for k, v in result.items() if k != 'decision_digest'}
        self.assertEqual(result['decision_digest'], hashlib.sha256(
            b'dfmcp-route-furniture-decision/1\0' + canonical(unsigned)).hexdigest())
        self.assertFalse(result['future_routes_preserved'])
        self.assertFalse(result['pathfinding_proven'])
        self.assertFalse(result['delivery_search']['global_cost_optimality_proven'])

    def test_alternative_item_rescues_mandatory_blocking_dependency(self):
        r, candidates, raw, cells = blocked_choice()
        original = canonical(r.json())
        self.assertEqual(sequence.allocate_sequenced_routes(r, candidates, raw)[1]['status'], 'delivery_order_blocked')
        effective, out = joint.allocate_joint_routes(r, candidates, raw)
        self.assertEqual(out['status'], 'allocated')
        self.assertEqual(out['delivery_search']['status'], 'alternative_found')
        self.assertEqual(out['delivery_search']['item_replacements'],
                         [{'slot': 'b', 'previous_item': 2, 'selected_item': 3}])
        self.assertEqual(out['delivery_search']['visited_nodes'], 3)
        self.assertFalse(out['delivery_sequence']['selected_items_preserved'])
        self.assertEqual(canonical(r.json()), original)
        self.verify_plan(r, candidates, cells, effective, out, routes.MAX_ROUTE_STEPS)

    def test_backtracking_does_not_steal_a_material_critical_item(self):
        raw, cells = terrain(size=(8, 1, 1), stairs=False)
        r = Request('fort', 1, (Slot('a', 'bed', (3, 1, 0), material=(419, -1)),
            Slot('b', 'bed', (4, 1, 0), after=('a',))))
        candidates = (item(1, (7, 1, 0)), item(2, (8, 1, 0), material=(420, -1)),
                      item(3, (2, 1, 0), material=(420, -1)))
        effective, out = joint.allocate_joint_routes(r, candidates, raw)
        self.assertEqual(out['status'], 'allocated')
        self.assertEqual(out['delivery_search']['visited_nodes'], 4)
        self.assertEqual([row['item'] for row in out['assignments']], [1, 2])
        self.verify_plan(r, candidates, cells, effective, out, 20)

    def test_node_exhaustion_never_claims_infeasibility_or_exports_partial_plan(self):
        r, candidates, raw, _ = blocked_choice()
        for maximum in (1, 2):
            effective, out = joint.allocate_joint_routes(r, candidates, raw, max_search_nodes=maximum)
            self.assertEqual(effective, r)
            self.assertEqual(out['status'], 'delivery_search_exhausted')
            self.assertEqual(out['delivery_search']['visited_nodes'], maximum)
            self.assertFalse(out['delivery_search']['search_complete'])
            self.assertFalse(out['delivery_search']['infeasibility_proven'])
            self.assertIsNone(out['plan'])
            self.assertIsNone(out['plan_digest'])
            self.assertIsNone(out['delivery_sequence']['blockage'])
            self.assertEqual(out['delivery_sequence']['routes'], [])
        self.assertEqual(joint.allocate_joint_routes(r, candidates, raw, max_search_nodes=3)[1]['status'], 'allocated')

    def test_exhaustive_model_blockage_is_not_global_inventory_absence(self):
        r, candidates, raw, _ = blocked_choice()
        _, out = joint.allocate_joint_routes(r, candidates[:2], raw)
        self.assertEqual(out['status'], 'delivery_assignment_blocked')
        self.assertTrue(out['delivery_search']['search_complete'])
        self.assertTrue(out['delivery_search']['infeasibility_proven'])
        self.assertEqual(out['delivery_search']['scope'], joint.SCOPE)
        self.assertFalse(out['global_absence_proven'])
        self.assertIsNone(out['shortage'])
        self.assertIsNone(out['plan'])

    def test_independent_assignment_and_order_oracle_300_seeded_cases(self):
        rng = random.Random(482731)
        positions = list(itertools.product(range(1, 5), range(1, 3), (0,)))
        fallback = 0
        for case in range(300):
            n, m = rng.randint(2, 3), rng.randint(3, 5)
            targets = rng.sample(positions, n)
            names = 'abc'[:n]
            hidden_order = rng.sample(list(names), n)
            slots = tuple(Slot(name, 'bed', target,
                after=tuple(k for k in hidden_order[:hidden_order.index(name)] if rng.random() < .6),
                material=None if rng.random() < .7 else (419, -1), max_distance=rng.randint(2, 7))
                for name, target in zip(names, targets))
            candidates = tuple(item(i, rng.choice(positions), material=(419+rng.randrange(2), -1)) for i in range(m))
            excluded = (0,) if rng.random() < .1 else ()
            r = Request('fort', 1, slots, excluded)
            changes = {p: (2, 2, 0, 0, 0, 1) for p in positions if rng.random() < .13}
            raw, cells = terrain(size=(4, 2, 1), stairs=False, changes=changes)
            limit = rng.randint(2, 8)
            effective, out = joint.allocate_joint_routes(r, candidates, raw, max_route_steps=limit)
            self.assertNotEqual(out['status'], 'delivery_search_exhausted', case)
            self.assertEqual(out['status'] == 'allocated', oracle(r, candidates, cells, limit), case)
            fallback += out['delivery_search']['visited_nodes'] > 0
            if out['status'] == 'allocated':
                self.verify_plan(r, candidates, cells, effective, out, limit)
            else:
                self.assertIsNone(out['plan'])
        self.assertGreaterEqual(fallback, 10)

    def test_every_512_obstacle_mask_against_unpruned_joint_oracle(self):
        positions = list(itertools.product(range(1, 4), range(1, 4), (0,)))
        r = Request('fort', 1, (Slot('a', 'bed', (2, 2, 0)), Slot('b', 'bed', (3, 2, 0), after=('a',))))
        candidates = (item(1, (1, 2, 0)), item(2, (2, 1, 0)), item(3, (3, 3, 0)))
        for mask in range(512):
            raw, cells = terrain(size=(3, 3, 1), stairs=False,
                changes={p: (2, 2, 0, 0, 0, 1) for i, p in enumerate(positions) if mask & (1 << i)})
            effective, out = joint.allocate_joint_routes(r, candidates, raw, max_route_steps=8)
            self.assertEqual(out['status'] == 'allocated', oracle(r, candidates, cells, 8), mask)
            if out['status'] == 'allocated':
                self.verify_plan(r, candidates, cells, effective, out, 8)

    def test_full_inventory_scanned_after_blocked_routing_not_initial_best_n(self):
        r, candidates, raw, cells = blocked_choice()
        # All 40 cheap bed items are on the wrong side of the mandatory first target.
        candidates = (candidates[0], *(item(i, (2, 1, 0)) for i in range(2, 42)), item(90, (7, 1, 0)))
        effective, out = joint.allocate_joint_routes(r, candidates, raw)
        self.assertEqual(out['assignments'][1]['item'], 90)
        self.verify_plan(r, candidates, cells, effective, out, 20)

    def test_constraints_stairs_and_observability_stay_independent(self):
        r, candidates, raw, _ = blocked_choice()
        for restriction in ('exclude', 'material', 'subtype', 'distance', 'route'):
            changed, pool, limit = r, candidates, 20
            if restriction == 'exclude':
                changed = replace(r, excluded_items=(3,))
            elif restriction == 'material':
                changed = replace(r, slots=(r.slots[0], replace(r.slots[1], material=(419, -1))))
                pool = (*candidates[:2], replace(candidates[2], material=(0, 0)))
            elif restriction == 'subtype':
                changed = replace(r, slots=(r.slots[0], replace(r.slots[1], subtype=-1)))
                pool = (*candidates[:2], replace(candidates[2], subtype=1))
            elif restriction == 'distance':
                changed = replace(r, slots=(r.slots[0], replace(r.slots[1], max_distance=2)))
            else:
                limit = 2
            _, out = joint.allocate_joint_routes(changed, pool, raw, max_route_steps=limit)
            self.assertEqual(out['status'], 'delivery_assignment_blocked', restriction)
        # Vertical fallback through the stair at x=5, not an assumed floor transition.
        r = replace(r, slots=(r.slots[0], replace(r.slots[1], max_vertical_distance=1)))
        pool = (*candidates[:2], replace(candidates[2], position=(7, 1, 1)))
        for lower, upper in ((6, 7), (8, 8), (7, 6), (3, 3)):
            raw, cells = terrain(size=(7, 1, 2), stairs=False, changes={
                (5, 1, 0): (2, lower, 0, 0, 0, 1), (5, 1, 1): (2, upper, 0, 0, 0, 1)})
            effective, out = joint.allocate_joint_routes(r, pool, raw)
            self.assertEqual(out['status'] == 'allocated', lower in (6, 8) and upper in (7, 8))
            if out['status'] == 'allocated':
                self.verify_plan(r, pool, cells, effective, out, 20)

    def test_32_slots_require_complete_search_and_no_partial_prefix(self):
        r, candidates, _, _ = blocked_choice()
        slots, pool = list(r.slots), list(candidates)
        previous = 'b'
        for i in range(30):
            name = f'c{i:02}'
            p = (i+8, 1, 0)
            slots.append(Slot(name, 'bed', p, after=(previous,), material=(500+i, -1)))
            pool.append(item(100+i, p, material=(500+i, -1)))
            previous = name
        r, candidates = Request('fort', 1, tuple(slots)), tuple(pool)
        raw, cells = terrain(size=(40, 1, 1), stairs=False)
        effective, out = joint.allocate_joint_routes(r, candidates, raw, max_search_nodes=33)
        self.assertEqual(out['status'], 'allocated')
        self.assertEqual(out['delivery_search']['visited_nodes'], 33)
        self.assertEqual(len(out['delivery_sequence']['routes']), 32)
        self.verify_plan(r, candidates, cells, effective, out, routes.MAX_ROUTE_STEPS)
        self.assertEqual(joint.allocate_joint_routes(r, candidates, raw, max_search_nodes=32)[1]['status'], 'delivery_search_exhausted')

    def test_deterministic_input_permutations_and_bound_identity(self):
        r, candidates, raw, _ = blocked_choice()
        expected = joint.allocate_joint_routes(r, candidates, raw)
        for permuted in itertools.permutations(candidates):
            self.assertEqual(joint.allocate_joint_routes(replace(r, slots=tuple(reversed(r.slots))), permuted, raw), expected)
        changed = joint.allocate_joint_routes(r, candidates, raw, max_search_nodes=3)[1]
        self.assertEqual(changed['plan'], expected[1]['plan'])
        self.assertNotEqual(changed['decision_digest'], expected[1]['decision_digest'])

    def test_shared_work_boundary_cancellation_and_invalid_search_limits(self):
        r, candidates, raw, _ = blocked_choice()
        effective, out = joint.allocate_joint_routes(r, candidates, raw)
        used = out['work_units']
        self.assertEqual(joint.allocate_joint_routes(r, candidates, raw, maximum_work=used)[1]['plan'], out['plan'])
        with self.assertRaisesRegex(ValueError, 'work allowance exhausted'):
            joint.allocate_joint_routes(r, candidates, raw, maximum_work=used-1)
        for value in (0, -1, 4097, True, 1.0, None):
            with self.assertRaises(ValueError):
                joint.allocate_joint_routes(r, candidates, raw, max_search_nodes=value)
        calls = []
        joint.allocate_joint_routes(r, candidates, raw, lambda: calls.append(1))
        for stop in range(1, len(calls)+1):
            count = 0
            def guard():
                nonlocal count
                count += 1
                if count == stop:
                    raise ValueError('cancelled')
            with self.assertRaisesRegex(ValueError, 'cancelled'):
                joint.allocate_joint_routes(r, candidates, raw, guard)

    def test_preserves_feasible_initial_assignment_and_shortage_semantics(self):
        r, candidates, raw, _ = blocked_choice()
        for pool in ((candidates[0], candidates[2]), candidates[:1]):
            before_request, before = sequence.allocate_sequenced_routes(r, pool, raw)
            effective, out = joint.allocate_joint_routes(r, pool, raw)
            self.assertEqual(effective, before_request)
            self.assertEqual(out['plan'], before['plan'])
            self.assertEqual(out['assignments'], before['assignments'])
            self.assertEqual(out['work_units'], before['work_units'])
            self.assertEqual(out['delivery_search']['visited_nodes'], 0)
            self.assertEqual(out['shortage'], before['shortage'])

    def test_new_review_retains_changed_item_and_later_checks_cannot_substitute(self):
        r, candidates, raw, _ = blocked_choice()
        effective, out = joint.allocate_joint_routes(r, candidates, raw)
        native_types = {row['item']: {'bed': 1, 'chair': 2}[row['kind']] for row in out['assignments']}
        review = reviews.create(effective, out, source(raw), native_types)
        before = review.raw
        checked = reviews.revalidate(review, candidates, {1: 2, 2: 1, 3: 1}, raw, source(raw))
        self.assertEqual(checked['status'], 'constraints_met')
        checked = reviews.revalidate(review, candidates[:2], {1: 2, 2: 1}, raw, source(raw))
        self.assertEqual(checked['status'], 'unestablished')
        self.assertIsNone(checked['plan'])
        self.assertEqual(review.raw, before)
        self.assertEqual(review.unpack()[3].steps[1].item, 3)


if __name__ == '__main__':
    unittest.main()
