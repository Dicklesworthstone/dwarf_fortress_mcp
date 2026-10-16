"""Actual room compiler/router/allocator integration, without producer doubles."""
from __future__ import annotations

import copy
import itertools
import json
import os
from pathlib import Path
import subprocess
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
from furniture_allocation import Candidate, allocate
from furniture_plan import canonical
from room_provisioning import RoomPlan
import room_layout as layout


def dining(name='eat', count=1, columns=1, **extra):
    return {'name': name, 'template': {'kind': 'dining_hall', 'table_count': count, 'columns': columns}, **extra}


def bedroom(name='sleep', count=2, **extra):
    return {'name': name, 'template': {'kind': 'bedroom_cluster', 'rooms_count': count, 'room_size': [3, 3]}, **extra}


def request(*areas, size=(18, 16, 1), origin=(10, 10, 2), **extra):
    return {'schema': layout.REQUEST_SCHEMA, 'world_folder': 'region1', 'site': 2,
            'region': {'origin': list(origin), 'size': list(size)},
            'areas': list(areas) or [bedroom(), dining(count=3, columns=3)], **extra}


def solve(value):
    return layout.solve(canonical(value))


def cells(plan):
    floors = set()
    for part in plan.json()['excavation_blueprint']['parts']:
        x, y, z = part['region']['origin']
        width, height, levels = part['region']['size']
        new = set(itertools.product(range(x, x + width), range(y, y + height), range(z, z + levels)))
        if floors & new:
            raise AssertionError('overlapping output rectangles')
        floors.update(new)
    return floors


def closure(admitted, start):
    found, pending = {start}, [start]
    while pending:
        x, y, z = pending.pop()
        for p in ((x-1, y, z), (x+1, y, z), (x, y-1, z), (x, y+1, z)):
            if p in admitted and p not in found:
                found.add(p)
                pending.append(p)
    return found


class LayoutTests(unittest.TestCase):
    def check_plan(self, result):
        body = result.json()
        self.assertEqual(body['status'], 'layout_found')
        plan = result.room_plan()
        self.assertEqual(RoomPlan.decode(plan.encode()), plan)
        floors = cells(plan)
        origin, size = body['request']['region']['origin'], body['request']['region']['size']
        self.assertTrue(all(all(a <= p < a + n for p, a, n in zip(point, origin, size)) for point in floors))
        self.assertEqual(len(floors), plan.json()['target_tiles'])
        for excluded in body['request']['excluded_regions']:
            self.assertFalse(any(all(a <= p < a + n for p, a, n in zip(point, excluded['origin'], excluded['size']))
                                 for point in floors))
        targets = {s.target for s in plan.request().slots}
        walkable = floors - targets
        entries = [tuple(a['entry_point']) for a in plan.json()['areas']]
        connected = closure(walkable, entries[0])
        self.assertEqual(connected, walkable)
        self.assertTrue(set(entries) <= connected)
        for target in targets:
            self.assertTrue(any(sum(abs(a-b) for a,b in zip(target, p)) == 1 for p in connected))
        self.assertEqual({a['name'] for a in plan.json()['intent']['areas']},
                         {a['name'] for a in body['request']['areas']})
        self.assertEqual(layout.Layout.decode(result.encode()), result)
        return plan

    def test_complete_mixed_layout_lowers_to_existing_connected_room_plan(self):
        plan = self.check_plan(solve(request()))
        self.assertEqual(len(plan.request().slots), 12)
        self.assertTrue(plan.json()['project_connections']['all_areas_connected'])

    def test_global_layout_backtracks_instead_of_keeping_a_greedy_prefix(self):
        excluded = [[10,14,2], [11,19,2], [12,15,2], [17,10,2], [13,17,2]]
        result = solve(request(dining('0',1,2), dining('1',2,1), dining('2',3,2), size=(13,10,1),
                               excluded_regions=[{'origin': p, 'size': [1,1,1]} for p in excluded]))
        self.check_plan(result)
        self.assertGreater(result.json()['search']['backtracks'], 0)
        self.assertEqual(result.json()['search']['nodes_visited'], 7)

    def test_fixed_origins_and_per_area_domains_are_never_relaxed(self):
        fixed = [12,11,2]
        result = solve(request(bedroom(origin=fixed), dining(origin_region={'origin':[10,17,2], 'size':[6,2,1]})))
        self.check_plan(result)
        areas = {a['name']: a for a in result.room_plan().json()['intent']['areas']}
        self.assertEqual(areas['sleep']['origin'], fixed)
        x,y,z = areas['eat']['origin']
        self.assertTrue(10 <= x < 16 and 17 <= y < 19 and z == 2)
        blocked = request(dining('a', origin=[10,10,2]), dining('b', origin=[10,10,2]))
        out = solve(blocked).json()
        self.assertEqual(out['status'], 'no_layout_in_domain')
        self.assertIsNone(out['room_plan'])

    def test_all_required_bedroom_walls_are_preserved_inside_the_domain(self):
        result = solve(request(bedroom(count=4), dining(count=2), size=(24,12,1)))
        plan = self.check_plan(result)
        floor = cells(plan)
        for area in plan.json()['areas']:
            if area['name'] != 'sleep':
                continue
            for unit in area['units']:
                x,y,z = unit['origin']; w,h,_ = unit['size']; door = tuple(unit['doorway'])
                walls = {(xx,yy,z) for xx in range(x-1,x+w+1) for yy in range(y-1,y+h+1)
                         if xx in (x-1,x+w) or yy in (y-1,y+h)} - {door}
                self.assertFalse(walls & floor)
                self.assertTrue(all(10 <= p[0] < 34 and 10 <= p[1] < 22 for p in walls))

    def test_exhaustive_small_obstacle_domains_match_independent_cell_oracle(self):
        # 256 masks; three possible translations of the actual 4x3 dining area.
        blockers = [(x, y, 2) for x in (10,13) for y in (10,11,12,14)]
        placements = [{(x,y,2) for x in range(10,14) for y in range(start,start+3)}
                      for start in range(10,13)]
        accepted = refused = 0
        for bits in range(1 << len(blockers)):
            blocked = {p for i,p in enumerate(blockers) if bits & (1<<i)}
            doc = request(dining(), size=(4,5,1), excluded_regions=[
                {'origin':list(p), 'size':[1,1,1]} for p in sorted(blocked)])
            out = solve(doc).json()
            expected = any(not p & blocked for p in placements)
            self.assertEqual(out['status'] == 'layout_found', expected, bits)
            accepted += int(expected)
            refused += int(not expected)
            if not expected:
                self.assertTrue(out['search']['domain_exhausted'])
                self.assertIsNone(out['room_plan'])
        self.assertGreater(accepted, 0)
        self.assertGreater(refused, 0)

    def test_two_area_feasibility_matches_exhaustive_origin_pairs(self):
        # The complete native-model corridor compiler must validate each pair;
        # expected placements are enumerated directly, without the solver masks.
        for width, height in itertools.product(range(4,10), range(3,7)):
            possible = False
            origins = list(itertools.product(range(10,10+width-3), range(10,10+height-2)))
            for a,b in itertools.combinations(origins,2):
                fa={(x,y,2) for x in range(a[0],a[0]+4) for y in range(a[1],a[1]+3)}
                fb={(x,y,2) for x in range(b[0],b[0]+4) for y in range(b[1],b[1]+3)}
                if fa & fb:
                    continue
                try:
                    RoomPlan.compile({'schema':'dfmcp.room-provisioning-request/1','world_folder':'region1',
                        'site':2,'connect_areas':True,'areas':[dining('a',origin=[*a,2]),dining('b',origin=[*b,2])]})
                    possible=True
                    break
                except ValueError:
                    pass
            out=solve(request(dining('a'),dining('b'),size=(width,height,1))).json()
            self.assertEqual(out['status']=='layout_found',possible,(width,height,out['status']))

    def test_complete_compiler_refusal_continues_search(self):
        # A complete exclusion stripe disconnects fixed goals. The placement
        # masks themselves fit, but the EXISTING corridor compiler must refuse.
        r=request(dining('a',origin=[10,10,2]),dining('b',origin=[18,10,2]),size=(12,5,1),
                  excluded_regions=[{'origin':[15,10,2],'size':[1,5,1]}])
        out=solve(r).json()
        self.assertEqual(out['status'],'no_layout_in_domain')
        self.assertEqual(out['search']['complete_layouts_tested'],1)
        self.assertEqual(out['search']['compiler_refusals'],1)
        self.assertIsNone(out['room_plan'])

    def test_node_budget_is_inconclusive_and_never_returns_partial_rooms(self):
        for maximum in (1,2):
            out=solve(request(search={'max_nodes':maximum})).json()
            self.assertEqual(out['status'],'search_limit_reached')
            self.assertFalse(out['search']['domain_exhausted'])
            self.assertEqual(out['search']['nodes_visited'],maximum)
            self.assertIsNone(out['room_plan'])
            self.assertFalse(out['all_areas_placed'])
        self.check_plan(solve(request(search={'max_nodes':3})))

    def test_work_allowance_is_shared_with_template_and_final_compilation(self):
        r=request(dining(),size=(4,3,1))
        used=solve(r).json()['search']['work_units']
        self.assertGreater(used,100)
        for maximum in (1,10,used-1):
            with self.assertRaises(layout.WorkLimit):
                solve({**r,'search':{'max_work':maximum}})
        out=solve({**r,'search':{'max_work':used}}).json()
        self.assertEqual(out['search']['work_units'],used)
        self.assertEqual(out['status'],'layout_found')

    def test_guard_cancellation_at_every_checkpoint_is_never_swallowed(self):
        raw=canonical(request(dining(),size=(4,3,1)))
        count=[]
        layout.solve(raw,lambda:count.append(1))
        for limit in range(len(count)):
            seen=0
            def guard():
                nonlocal seen
                if seen==limit:
                    raise ValueError('external cancellation')
                seen+=1
            with self.subTest(limit=limit),self.assertRaisesRegex(ValueError,'external cancellation'):
                layout.solve(raw,guard)

    def test_allocation_retains_every_constraint_and_shortages_remain_whole(self):
        r=request(bedroom(count=1,item_constraints={'bed':{'material':[7,-1],'subtype':2,'max_distance':40}}),
                  dining(),excluded_items=[1000,999])
        plan=self.check_plan(solve(r))
        candidates=tuple(Candidate(i,s.kind,s.target,s.material or (0,2),s.subtype if s.subtype is not None else -1)
                         for i,s in enumerate(plan.request().slots))
        out=allocate(plan.request(),candidates)
        self.assertEqual(out['status'],'allocated')
        self.assertEqual(len(out['plan']['steps']),5)
        bed=next(s for s in plan.request().slots if s.kind=='bed')
        self.assertEqual((bed.material,bed.subtype,bed.max_distance),((7,-1),2,40))
        shortage=allocate(plan.request(),candidates[:-1])
        self.assertEqual(shortage['status'],'shortage')
        self.assertIsNone(shortage['plan'])
        self.assertEqual(shortage['assignments'],[])

    def test_maximum_area_and_furniture_counts_use_complete_group(self):
        r=request(*[dining(str(i),2,2) for i in range(8)],size=(32,32,1),
                  search={'max_work':layout.MAX_WORK})
        out=solve(r)
        plan=self.check_plan(out)
        self.assertEqual(len(plan.json()['areas']),8)
        self.assertEqual(len(plan.request().slots),32)
        self.assertLessEqual(len(out.encode()),layout.MAX_OUTPUT)
        self.assertLessEqual(sum(r['count'] for r in out.json()['search']['candidate_counts']),8192)

    def test_request_order_defaults_and_process_hash_seed_do_not_change_result(self):
        r=request(excluded_items=[999,1000],excluded_regions=[{'origin':[25,23,2],'size':[1,1,1]},
                    {'origin':[27,24,2],'size':[1,1,1]}])
        original=solve(r).encode()
        r['areas'].reverse();r['excluded_items'].reverse();r['excluded_regions'].reverse()
        self.assertEqual(original,solve(r).encode())
        env={**os.environ,'PYTHONDONTWRITEBYTECODE':'1',
             'PYTHONPATH':str(Path(__file__).resolve().parents[1]/'scripts')}
        code='import sys,room_layout;sys.stdout.buffer.write(room_layout.solve(sys.stdin.buffer.read()).encode())'
        for seed in ('1','17','random'):
            process=subprocess.run([sys.executable,'-c',code],input=canonical(r),capture_output=True,
                                   env={**env,'PYTHONHASHSEED':seed},check=True)
            self.assertEqual(process.stdout,original)

    def test_canonical_import_rejects_altered_selection_constraints_and_search(self):
        result=solve(request())
        mutations=[]
        for key,val in [('layout_digest','0'*64),('status','no_layout_in_domain'),('terrain_observed',True)]:
            v=result.json();v[key]=val;mutations.append(v)
        v=result.json();v['request']['areas'].pop();mutations.append(v)
        v=result.json();v['room_plan']['intent']['areas'][0]['origin'][0]+=1;mutations.append(v)
        v=result.json();v['search']['nodes_visited']+=1;mutations.append(v)
        v=result.json();v['search']['trace_digest']='0'*64;mutations.append(v)
        for v in mutations:
            with self.assertRaises(ValueError):layout.Layout.decode(canonical(v))
        with self.assertRaises(ValueError):layout.Layout.decode(result.encode()+b'\n')
        # Returned dictionaries do not alias the immutable retained artifact.
        v=result.json();v['request']['areas'].clear()
        self.assertEqual(layout.Layout.decode(result.encode()),result)

    def test_exact_types_closed_shapes_and_shared_capacity_refuse(self):
        cases=[]
        for key,val in [('schema','bad'),('unknown',True),('areas',[]),('site',True),('world_folder','\0')]:
            r=request();r[key]=val;cases.append(r)
        for size in ([0,10,1],[10,10,2],[33,32,1],[129,1,1],[True,10,1]):
            cases.append(request(size=size))
        cases += [request(origin=(0,10,2)),request(origin=(32766,10,2)),
                  request(focus=[1,1,1]),request(search={'max_nodes':0}),request(search={'max_work':True}),
                  request(search={'beam':1}),request(*[dining(str(i)) for i in range(9)]),
                  request(bedroom(count=10),dining(count=2))]
        for extra in ({'origin':[10,10,True]}, {'origin':[10,10,2],'origin_region':{'origin':[10,10,2],'size':[1,1,1]}},
                      {'origin_region':{'origin':[1,1,2],'size':[2,2,1]}}, {'rotation':90}):
            cases.append(request(dining(**extra)))
        for r in cases:
            with self.subTest(r=r),self.assertRaises((ValueError,TypeError)):solve(r)
        for raw in (b'',b' '* (layout.MAX_INPUT+1),b'['*11+b']'*11,
                    canonical(request()).replace(b'"site":2',b'"site":2,"site":2')):
            with self.assertRaises(ValueError):layout.solve(raw)

    def test_result_never_claims_native_facts_or_global_optimization(self):
        out=solve(request()).json()
        for name in ('global_optimum_proven','terrain_observed','native_pathfinding_proven',
                     'existing_fort_access_proven','items_reserved','prepared_mutation_plan_created',
                     'game_effect_performed','production_admitted','partial_plan_returned'):
            self.assertIs(out[name],False)
        self.assertFalse(out['search']['all_layouts_ranked'])
        self.assertTrue(out['complete_original_request_retained'])


if __name__=='__main__':
    unittest.main()
