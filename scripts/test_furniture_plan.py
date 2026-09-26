"""Execute exact furniture DAG compilation, not a model of native placement."""
import itertools
import json
import unittest
from dataclasses import replace

import furniture_plan as p


def step(name='bed', item=1, target=(10, 10, 2), after=()):
    return p.Step(name, 'bed', item, target, after)


class FurniturePlanTests(unittest.TestCase):
    def test_all_four_node_dependency_graphs_against_permutation_oracle(self):
        names = ('a', 'b', 'c', 'd')
        edges = [(a, b) for a in names for b in names if a != b]
        accepted = 0
        for mask in range(1 << len(edges)):
            selected = [edge for i, edge in enumerate(edges) if mask & (1 << i)]
            valid = [order for order in itertools.permutations(names)
                     if all(order.index(a) < order.index(b) for a, b in selected)]
            steps = tuple(step(name, i, (10+i, 10, 2), tuple(a for a,b in selected if b == name))
                          for i, name in enumerate(names))
            if not valid:
                with self.assertRaises(ValueError): p.FurniturePlan(steps)
            else:
                plan = p.FurniturePlan(steps)
                self.assertEqual(tuple(s.name for s in plan.ordered), min(valid))
                self.assertEqual(p.FurniturePlan(tuple(reversed(steps))), plan)
                accepted += 1
        self.assertEqual(accepted, 543)

    def test_semantic_identity_order_dependency_order_and_roundtrip(self):
        steps = (step('c',3,(10,12,2),('b','a')), step('a'),step('b',2,(10,11,2)))
        expected = p.FurniturePlan(steps)
        for order in itertools.permutations(steps):
            plan = p.FurniturePlan(order)
            self.assertEqual(plan.digest, expected.digest)
            self.assertEqual(p.FurniturePlan.decode(p.canonical(plan.json())), expected)
        self.assertEqual(p.FurniturePlan((replace(steps[0],after=('a','b')), *steps[1:])), expected)
        for change in (replace(steps[0],kind='chair'),replace(steps[0],item=4),
                       replace(steps[0],target=(11,12,2)),replace(steps[0],after=('a',))):
            self.assertNotEqual(p.FurniturePlan((change,*steps[1:])).digest, expected.digest)

    def test_invalid_steps_and_conflicting_resources(self):
        for kwargs in ({'name':''},{'name':'a'*49},{'name':'bad/slot'},{'kind':'throne'},
                       {'item':True},{'item':-1},{'item':2147483647}, {'target':(0,1,2)},
                       {'target':(1,32767,2)},{'target':(1,1,-1)},{'target':(1,1,32768)},
                       {'after':('bed',)},{'after':('x','x')},{'after':['x']}):
            with self.assertRaises(ValueError): replace(step(), **kwargs)
        for steps in ((),(step(),step()),(step(),step('b')),
                      (step(),step('b',2)),(step(after=('missing',)),),
                      tuple(step(str(i),i,(10+i,10,2)) for i in range(33))):
            with self.assertRaises(ValueError): p.FurniturePlan(steps)

    def test_map_context_and_vertical_limits(self):
        plan = p.FurniturePlan((step(target=(1,1,0)),step('b',2,(32766,32766,32767))))
        plan.check_dimensions([32768]*3)
        for dims in ([32767,32768,32768],[32768,32767,32768],[32768,32768,32767],
                     [32768,32768,True], [0,10,10], [10,10], [32769]*3):
            with self.assertRaises(ValueError): plan.check_dimensions(dims)

    def test_closed_bounded_json(self):
        valid = p.FurniturePlan((step(),)).json()
        minimal = json.loads(json.dumps(valid)); del minimal['steps'][0]['after']
        self.assertEqual(p.FurniturePlan.from_json(minimal),p.FurniturePlan.from_json(valid))
        invalid = [b'',b' '* (p.MAX_BYTES+1),b'['*9+b'0'+b']'*9,
                   b'{"schema":"x","schema":"y","steps":[]}',b'\xff',b'null']
        for raw in invalid:
            with self.assertRaises((ValueError,UnicodeError)): p.FurniturePlan.decode(raw)
        for target in ('extra','steps','schema'):
            changed = dict(valid); changed[target] = None
            with self.assertRaises(ValueError): p.FurniturePlan.from_json(changed)
        changed = json.loads(json.dumps(valid)); changed['steps'][0]['commit'] = True
        with self.assertRaises(ValueError): p.FurniturePlan.from_json(changed)

    def test_full_plan_preserves_exact_item_and_target_without_translation(self):
        plan = p.FurniturePlan(tuple(p.Step(f'room{i:02}',p.KINDS[i%3],100+i,
                     (2+i*3,3, i%16),tuple(f'room{j:02}' for j in range(i))) for i in range(32)))
        plan.check_dimensions([128,128,16])
        self.assertEqual(len(plan.ordered),32)
        self.assertEqual({s.item for s in plan.steps},set(range(100,132)))
        self.assertEqual(len({s.target for s in plan.steps}),32)
        self.assertLess(len(p.canonical(plan.json())),p.MAX_BYTES)

    def test_only_verified_placed_prefix_can_advance(self):
        plan = p.FurniturePlan((step('a'),step('b',2,(11,10,2),('a',)),step('c',3,(12,10,2))))
        for count in range(4):
            saved = {s.name:'placed' for s in plan.ordered[:count]}
            result = p.progress(plan,saved)
            self.assertEqual(result['placed'],count)
            self.assertEqual(result['status'],'all_placed' if count == 3 else 'ready')
            self.assertEqual(result['next_step'],None if count == 3 else plan.ordered[count].name)
            if count < 3:
                for phase in ('unknown','indeterminate','refused','cancelled'):
                    changed = dict(saved); changed[plan.ordered[count].name] = phase
                    result = p.progress(plan,changed)
                    self.assertIsNone(result['next_step'])
                    self.assertFalse(result['retry_permitted'])
                    self.assertFalse(result['construction_completion_proven'])
        for bad in ({'b':'placed'},{'a':'unknown','b':'placed'},{'a':'prepared'}, {'absent':'placed'}, {'a':True}):
            with self.assertRaises(ValueError): p.progress(plan,bad)


if __name__ == '__main__': unittest.main()
