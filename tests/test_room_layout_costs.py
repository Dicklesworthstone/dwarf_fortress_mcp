"""Full compiler integration and independent finite-domain cost enumeration."""
from copy import deepcopy
import itertools
from pathlib import Path
import random
import sys
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'scripts'))
from furniture_allocation import Candidate, allocate
from furniture_plan import canonical
import room_layout as layout
from room_layout_costs import TileCosts
from room_provisioning import RoomPlan, INTENT_SCHEMA


def hall(name='hall', **extra):
    return {'name': name, 'template': {'kind': 'dining_hall', 'table_count': 1, 'columns': 1}, **extra}


def request(*areas, size=(8, 3, 1), cells=None, **extra):
    return {'schema': layout.REQUEST_SCHEMA, 'world_folder': 'fixture', 'site': 1,
            'region': {'origin': [10, 10, 2], 'size': list(size)}, 'areas': list(areas) or [hall()],
            'objective': layout.COST_OBJECTIVE, 'excavation_costs': cells or '11110000' * 3, **extra}


def solve(value):
    return layout.solve(canonical(value))


def tiles(body):
    found = set()
    for part in body['excavation_blueprint']['parts']:
        x, y, z = part['region']['origin']
        w, h, levels = part['region']['size']
        points = set(itertools.product(range(x, x + w), range(y, y + h), range(z, z + levels)))
        if found & points:
            raise AssertionError('overlapping original target rectangles')
        found |= points
    return found


def independent_score(body, value):
    low, size = value['region']['origin'], value['region']['size']
    points = tiles(body)
    cost = sum(int(value['excavation_costs'][(y-low[1])*size[0]+x-low[0]]) for x, y, _ in points)
    focus = value.get('focus', low)
    distance = sum(sum(abs(a-b) for a,b in zip(area['entry_point'], focus)) for area in body['areas'])
    placements = tuple((a['origin'][1], a['origin'][0], a['origin'][2], a.get('rotation',0))
                       for a in sorted(body['intent']['areas'], key=lambda a:a['name']))
    return cost, len(points), distance, placements


def independent_best(value):
    # Enumerate actual origins and allowed angles, not candidate bitsets, MRV,
    # ranking helpers or pruning. Only original compiler feasibility is shared.
    low, size = value['region']['origin'], value['region']['size']
    variants = []
    for area in value['areas']:
        starts = [area['origin']] if 'origin' in area else (
            [x,y,low[2]] for y in range(low[1],low[1]+size[1]) for x in range(low[0],low[0]+size[0]))
        possible = []
        for start in starts:
            for angle in area.get('rotations', [area.get('rotation',0)]):
                actual = {k:v for k,v in area.items() if k not in ('rotations','origin_region')}
                actual.update(origin=start,rotation=angle)
                try:
                    p = RoomPlan.compile({'schema':INTENT_SCHEMA,'world_folder':'fixture','site':1,'areas':[actual]})
                    if all(all(a<=c<a+n for a,c,n in zip(low,point,size)) for point in tiles(p.json())):
                        possible.append(actual)
                except ValueError:
                    pass
        variants.append(possible)
    best = None
    for combination in itertools.product(*variants):
        intent = {'schema':INTENT_SCHEMA,'world_folder':'fixture','site':1,'areas':list(combination),'connect_areas':True}
        for field in ('site_mask','excluded_regions'):
            if field in value: intent[field] = value[field]
        if 'access_point' in value:
            intent['access'] = {'point':value['access_point'],'region':value['region']}
        try:
            p = RoomPlan.compile(intent, _connection_fallback='connection_strategy' in value)
            score = independent_score(p.json(), value)
            best = score if best is None else min(best,score)
        except ValueError:
            pass
    return best


def stored_score(result):
    s=result.json()['optimization']['score']
    return None if s is None else (s['excavation_cost'],s['target_tiles'],s['total_entry_distance'],
                                   tuple(tuple(p) for p in s['placements_yxz_rotation']))


class CostLayoutTests(unittest.TestCase):
    def test_prefers_reusable_targets_without_omitting_any_furniture(self):
        result=solve(request()); body=result.json(); plan=result.room_plan()
        self.assertEqual(stored_score(result)[:3],(0,12,4))
        self.assertEqual(body['optimization']['first_feasible_score']['excavation_cost'],12)
        self.assertEqual(plan.json()['intent']['areas'][0]['origin'],[14,10,2])
        self.assertEqual(layout.Layout.decode(result.encode()),result)
        self.assertEqual(RoomPlan.decode(plan.encode()),plan)
        candidates=tuple(Candidate(i,s.kind,s.target,(0,1),-1) for i,s in enumerate(plan.request().slots))
        self.assertEqual(allocate(plan.request(),candidates)['status'],'allocated')
        self.assertIsNone(allocate(plan.request(),candidates[:-1])['plan'])
        self.assertEqual(body['optimization']['cost_evidence'],'caller_declared_not_observed')
        self.assertFalse(body['terrain_observed'])

    def test_128_binary_maps_match_full_origin_and_rotation_enumeration(self):
        rng=random.Random(1208)
        for case in range(128):
            costs=''.join(rng.choice('01') for _ in range(20))
            value=request(hall(rotations=[0,90,180,270]),size=(5,4,1),cells=costs)
            with self.subTest(case=case):
                out=solve(value)
                self.assertEqual(stored_score(out),independent_best(value))
                self.assertTrue(out.json()['optimization']['minimum_in_compiler_domain_proven'])

    def test_multiarea_cost_bound_matches_independent_global_enumeration(self):
        rng=random.Random(440)
        for size in ((8,3,1),(8,4,1),(7,6,1)):
            for case in range(6):
                value=request(hall('a'),hall('b'),size=size,cells=''.join(rng.choice('01') for _ in range(size[0]*size[1])))
                with self.subTest(size=size,case=case):
                    out=solve(value)
                    self.assertEqual(stored_score(out),independent_best(value))
                    self.assertEqual(out.json()['room_plan']['furniture_count'],4)

    def test_cost_includes_added_corridors_and_original_access_point(self):
        value=request(hall(origin=[15,10,2]),size=(9,3,1),cells='1'*27,access_point=[10,10,2])
        out=solve(value)
        self.assertEqual(stored_score(out),independent_best(value))
        self.assertGreater(stored_score(out)[0],12)
        self.assertEqual(stored_score(out)[0],len(tiles(out.room_plan().json())))

    def test_costs_do_not_relax_site_mask_exclusions_or_fixed_origins(self):
        base=request()
        base['site_mask']={'region':deepcopy(base['region']),'cells':'....XXXX'*3}
        self.assertEqual(stored_score(solve(base))[0],12)
        base.pop('site_mask')
        base['excluded_regions']=[{'origin':[14,10,2],'size':[4,3,1]}]
        self.assertEqual(stored_score(solve(base))[0],12)
        base.pop('excluded_regions')
        base['areas'][0]['origin']=[10,10,2]
        self.assertEqual(stored_score(solve(base))[0],12)

    def test_all_zero_and_all_one_maps_preserve_footprint_optimum_choice(self):
        for costs in ('0'*24,'1'*24):
            value=request(cells=costs)
            legacy={k:v for k,v in value.items() if k!='excavation_costs'}
            legacy['objective']=layout.MINIMUM_TARGETS_OBJECTIVE
            self.assertEqual(solve(value).room_plan(),solve(legacy).room_plan())

    def test_maps_are_exact_complete_and_only_legal_for_the_cost_objective(self):
        for cells in ('','0'*23,'0'*25,'0'*23+'2','0'*23+'０',[],{},None,True,0):
            value=request();value['excavation_costs']=cells
            with self.subTest(cells=cells),self.assertRaises(ValueError):solve(value)
        for objective in (None,layout.MINIMUM_TARGETS_OBJECTIVE,'unknown'):
            value=request()
            if objective is None: value.pop('objective')
            else: value['objective']=objective
            with self.assertRaises(ValueError):solve(value)
        value=request();value.pop('excavation_costs')
        with self.assertRaises(ValueError):solve(value)
        with self.assertRaises(ValueError):TileCosts.parse({'origin':[True,10,2],'size':[8,3,1]},'0'*24,lambda:None)

    def test_cost_map_does_not_implicitly_enable_the_corridor_fallback(self):
        value=request()
        out=solve(value)
        self.assertNotIn('connection_strategy',out.json()['request'])
        self.assertFalse(out.json()['optimization']['corridors_rerouted_by_cost'])
        value['connection_strategy']=layout.FALLBACK_STRATEGY
        result=solve(value)
        self.assertEqual(result.room_plan(),out.room_plan())
        self.assertNotEqual(result.digest,out.digest)

    def test_every_original_selector_and_cross_level_limit_survives_ranking(self):
        value=request(hall(item_constraints={'chair':{'material':[7,-1],'subtype':3,'max_distance':5,'max_vertical_distance':2}}),excluded_items=[11,17])
        plan=solve(value).room_plan(); wanted=plan.request()
        chair=next(s for s in wanted.slots if s.kind=='chair')
        self.assertEqual((chair.material,chair.subtype,chair.max_distance,chair.max_vertical_distance),((7,-1),3,5,2))
        self.assertEqual(wanted.excluded_items,(11,17))
        with self.assertRaises(ValueError): allocate(wanted,())

    def test_retains_all_32_furnishings_and_eight_areas(self):
        areas=[]
        for i in range(8):
            area=hall(str(i),origin=[10+(i%4)*7,10+(i//4)*3,2])
            area['template'].update(table_count=2,columns=2)
            areas.append(area)
        value=request(*areas,size=(28,6,1),cells='0'*168)
        out=solve(value)
        self.assertEqual(out.room_plan().json()['furniture_count'],32)
        self.assertEqual(len(out.room_plan().json()['areas']),8)
        self.assertEqual(stored_score(out)[0],0)

    def test_node_limit_retains_only_complete_incumbent_with_no_proof(self):
        value=request(search={'max_nodes':2})
        result=solve(value)
        self.assertEqual(result.json()['status'],'layout_found')
        self.assertEqual(stored_score(result)[0],12)
        self.assertFalse(result.json()['optimization']['minimum_in_compiler_domain_proven'])
        self.assertEqual(layout.Layout.decode(result.encode()),result)
        value['search']['max_nodes']=1
        result=solve(value)
        self.assertEqual(result.json()['status'],'search_limit_reached')
        self.assertIsNone(result.json()['room_plan'])

    def test_shared_work_limit_never_publishes_incumbent(self):
        value=request(); used=solve(value).json()['search']['work_units']
        for bound in (1,used//2,used-1):
            with self.subTest(bound=bound),self.assertRaises(layout.WorkLimit):solve({**value,'search':{'max_work':bound}})
        self.assertEqual(solve({**value,'search':{'max_work':used}}).json()['search']['work_units'],used)

    def test_cancellation_at_every_checkpoint_returns_no_plan(self):
        value=request(size=(4,3,1),cells='01'*6)
        raw=canonical(value); total=[]
        layout.solve(raw,lambda:total.append(1))
        for stop in range(1,len(total)+1):
            seen=0
            def guard():
                nonlocal seen
                seen+=1
                if seen==stop: raise ValueError('external cancellation')
            with self.subTest(stop=stop),self.assertRaisesRegex(ValueError,'external cancellation'):
                layout.solve(raw,guard)

    def test_saved_costs_score_and_proof_cannot_be_rewritten_and_rehashed(self):
        result=solve(request())
        for field in ('costs','score','proof','region'):
            body=result.json(); body.pop('layout_digest')
            if field=='costs': body['request']['excavation_costs']='0'*24
            elif field=='score': body['optimization']['score']['excavation_cost']=1
            elif field=='proof': body['optimization']['minimum_in_compiler_domain_proven']=False
            else: body['request']['region']['origin'][0]+=1
            forged=layout.Layout(canonical(body))
            with self.subTest(field=field),self.assertRaises(ValueError):layout.Layout.decode(forged.encode())

    def test_reordered_areas_and_inputs_do_not_mutate_or_change_results(self):
        value=request(hall('b',origin=[14,10,2]),hall('a',origin=[10,10,2]),cells='01'*12)
        old=deepcopy(value); first=solve(value)
        self.assertEqual(old,value)
        value['areas'].reverse()
        self.assertEqual(first,solve(value))


if __name__=='__main__':unittest.main()
