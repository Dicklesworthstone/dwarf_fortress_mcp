"""Actual Python implementation vs independent unpruned shortest-path/permutation oracles."""
import hashlib
import itertools
import random
import struct
import unittest

from furniture_allocation import Candidate, Request, Slot, allocate
from furniture_plan import FurniturePlan, canonical
from route_furniture_allocation import (
    MAX_CAPTURE_BYTES, MAX_ROUTE_STEPS, MAX_WORK, Work,
    allocate_routes, decode_terrain,
)


def capture(size=(5, 5, 1), *, origin=(0, 0, 5), changes=None, folder='region1',
            site=2, tick=3, paused=True, dimensions=(128, 128, 128)):
    raw = bytearray(b'DFMM1500')
    raw += struct.pack('>IIBIH', tick // 403200, tick % 403200, paused, site, len(folder.encode()))
    raw += folder.encode()
    raw += struct.pack('>10I', *dimensions, *origin, *size, size[0] * size[1] * size[2])
    for i in range(size[0] * size[1] * size[2]):
        value = (changes or {}).get(i, (1, 3, 0, 0, 0, 0, 0, 0, 1, 10015, 10015))
        if type(value) is int:
            raw += bytes([value])
        else:
            raw += b'\x02' + struct.pack('>IBBBBBBBIHH', *value)
    return bytes(raw)


def item(identity, position, kind='bed', material=(419, -1), subtype=-1):
    return Candidate(identity, kind, position, material, subtype)


def request(*slots, excluded=()):
    return Request('region1', 2, slots, excluded)


def oracle(req, candidates, raw, maximum=MAX_ROUTE_STEPS):
    """Independent Floyd-Warshall distances + exhaustive item permutations.

    Tiny fixtures only. Does NOT call production graph construction, BFS, matching,
    distance filtering, trimming or the minimum-cost solver.
    """
    terrain = decode_terrain(raw)
    n = len(terrain.shapes)
    dist = [[99999] * n for _ in range(n)]
    for a in range(n):
        for b in range(n):
            if not terrain.shapes[a] or not terrain.shapes[b]:
                continue
            pa, pb = terrain.position(a), terrain.position(b)
            if a == b:
                dist[a][b] = 0
            elif sum(abs(x-y) for x, y in zip(pa, pb)) == 1:
                if pa[2] == pb[2]:
                    dist[a][b] = 1
                else:
                    low, high = (a, b) if pa[2] < pb[2] else (b, a)
                    if terrain.shapes[low] in (6, 8) and terrain.shapes[high] in (7, 8):
                        dist[a][b] = 1
    for k in range(n):
        for a in range(n):
            for b in range(n):
                dist[a][b] = min(dist[a][b], dist[a][k] + dist[k][b])
    edges = []
    for slot in req.slots:
        row = {}
        for candidate in candidates:
            source, target = terrain.index(candidate.position), terrain.index(slot.target)
            if candidate.id in req.excluded_items or slot.kind != candidate.kind:
                continue
            if (slot.material is not None and slot.material != candidate.material
                or slot.subtype is not None and slot.subtype != candidate.subtype
                or slot.target[2] != candidate.position[2]
                or abs(slot.target[0]-candidate.position[0])+abs(slot.target[1]-candidate.position[1]) > slot.max_distance):
                continue
            if source is not None and target is not None and dist[source][target] <= maximum:
                row[candidate.id] = dist[source][target]
        edges.append(row)
    best = None
    for permutation in itertools.permutations(candidates, len(req.slots)):
        ids = tuple(candidate.id for candidate in permutation)
        if all(identity in edges[row] for row, identity in enumerate(ids)):
            value = (sum(edges[row][identity] for row, identity in enumerate(ids)), ids)
            best = value if best is None else min(best, value)
    maximum_matching = 0
    for count in range(len(req.slots)+1):
        for selected in itertools.combinations(range(len(req.slots)), count):
            for ids in itertools.permutations([candidate.id for candidate in candidates], count):
                if all(identity in edges[row] for row, identity in zip(selected, ids)):
                    maximum_matching = count
    return best, maximum_matching, edges


class AllocationTests(unittest.TestCase):
    def test_wall_rejects_nearer_item_and_chooses_accessible_supply(self):
        raw = capture(changes={2+5*y: 1 for y in range(5)})
        req = request(Slot('bed', 'bed', (3, 2, 5)))
        candidates = (item(1, (1, 2, 5)), item(2, (4, 4, 5)))
        self.assertEqual(allocate(req, candidates)['assignments'][0]['item'], 1)
        result = allocate_routes(req, candidates, raw)
        self.assertEqual(result['assignments'][0]['item'], 2)
        self.assertEqual(result['total_route_steps'], 3)
        self.assertFalse(result['pathfinding_proven'])
        self.assertFalse(result['future_routes_preserved'])
        self.assertEqual(FurniturePlan.from_json(result['plan']).digest, result['plan_digest'])

    def test_detour_cost_not_manhattan_is_globally_minimized(self):
        raw = capture(changes={2+5*y: 1 for y in range(1, 5)})
        req = request(Slot('bed', 'bed', (3, 3, 5)))
        candidates = (item(1, (1, 3, 5)), item(2, (4, 0, 5)))
        result = allocate_routes(req, candidates, raw)
        self.assertEqual(result['assignments'][0]['item'], 2)
        self.assertEqual(result['total_route_steps'], 4)

    def test_material_starvation_requires_whole_plan_assignment(self):
        raw = capture()
        req = request(Slot('generic', 'bed', (1, 1, 5)),
                      Slot('specific', 'bed', (2, 1, 5), ('generic',), (419, -1)))
        candidates = (item(1, (1, 1, 5)), item(2, (4, 4, 5), material=(420, -1)))
        result = allocate_routes(req, candidates, raw)
        self.assertEqual([row['item'] for row in result['assignments']], [2, 1])
        self.assertEqual(result['plan']['steps'][1]['after'], ['generic'])

    def test_hall_witness_never_emits_executable_subset(self):
        req = request(Slot('a', 'bed', (1, 1, 5)), Slot('b', 'bed', (2, 1, 5)))
        candidates = (item(1, (1, 2, 5)), item(2, (4, 2, 5)))
        raw = capture(changes={3+5*y: 1 for y in range(5)})
        result = allocate_routes(req, candidates, raw)
        self.assertEqual(result['status'], 'shortage')
        self.assertEqual(result['maximum_assignable'], 1)
        self.assertEqual(result['shortage']['slots'], ['a', 'b'])
        self.assertEqual(result['shortage']['candidate_items'], [1])
        self.assertEqual(result['shortage']['missing'], 1)
        self.assertIsNone(result['plan'])
        self.assertEqual(result['assignments'], [])
        self.assertFalse(result['global_absence_proven'])

    def test_excluded_and_outside_targets_remain_unestablished(self):
        for position, changes in [((1,1,5),{6:1}), ((1,1,5),{6:0}), ((6,1,5),{})]:
            result = allocate_routes(request(Slot('bed','bed',position)),
                                     (item(1,(1,2,5)),), capture(changes=changes))
            self.assertEqual(result['status'], 'target_unestablished')
            self.assertIsNone(result['maximum_assignable'])
            self.assertIsNone(result['shortage'])
            self.assertIsNone(result['plan'])

    def test_all_original_constraints_and_route_limit_are_separate(self):
        raw = capture()
        candidates = (item(1,(3,1,5)),)
        for slot, maximum in [(Slot('a','bed',(1,1,5),max_distance=1), 10),
                              (Slot('a','bed',(1,1,5),max_distance=10), 1),
                              (Slot('a','bed',(1,1,5),subtype=5), 10),
                              (Slot('a','chair',(1,1,5)), 10)]:
            self.assertEqual(allocate_routes(request(slot), candidates, raw,
                                            max_route_steps=maximum)['status'], 'shortage')
        self.assertEqual(allocate_routes(request(Slot('a','bed',(1,1,5)),excluded=(1,)),
                                         candidates, raw)['status'], 'shortage')
        result = allocate_routes(request(Slot('a','bed',(1,1,5))), (item(1,(1,1,5)),), raw,
                                 max_route_steps=0)
        self.assertEqual(result['total_route_steps'], 0)

    def test_stairs_support_detours_but_never_cross_level_item_selection(self):
        size=(3,1,2)
        up=(1,6,0,0,0,0,0,0,1,10015,10015)
        down=(1,7,0,0,0,0,0,0,1,10015,10015)
        raw=capture(size,origin=(1,1,5),changes={0:up,1:1,2:up,3:down,5:down})
        req=request(Slot('a','bed',(3,1,5)))
        candidates=(item(1,(1,1,5)),item(2,(3,1,6)))
        result=allocate_routes(req,candidates,raw)
        self.assertEqual(result['assignments'][0]['item'],1)
        self.assertEqual(result['total_route_steps'],4)
        broken=capture(size,origin=(1,1,5),changes={0:up,1:1,2:up,3:down})
        self.assertEqual(allocate_routes(req,candidates,broken)['status'],'shortage')

    def test_exclusions_match_existing_dry_floor_stair_policy(self):
        floor=[1,3,0,0,0,0,0,0,1,10015,10015]
        for field,value in [(1,4),(1,2),(2,1),(6,1),(7,2),(8,0)]:
            tile=floor.copy(); tile[field]=value
            result=allocate_routes(request(Slot('a','bed',(3,1,5))), (item(1,(1,1,5)),),
                                   capture((3,1,1),origin=(1,1,5),changes={1:tuple(tile)}))
            self.assertEqual(result['status'],'shortage')

    def test_all_obstacle_maps_match_unpruned_oracle(self):
        candidates=(item(8,(1,1,5)),item(3,(3,1,5)),item(5,(1,3,5)))
        req=request(Slot('a','bed',(2,2,5)),Slot('b','bed',(3,3,5)))
        for mask in range(512):
            raw=capture((3,3,1),origin=(1,1,5),changes={i:1 for i in range(9) if mask&(1<<i)})
            result=allocate_routes(req,candidates,raw)
            if mask & ((1<<4)|(1<<8)):
                self.assertEqual(result['status'],'target_unestablished')
                continue
            best, maximum, edges=oracle(req,candidates,raw)
            self.assertEqual(result['maximum_assignable'],maximum)
            if best is None:
                witness=result['shortage']
                selected=[i for i,s in enumerate(req.slots) if s.name in witness['slots']]
                neighbor_ids=sorted({identity for i in selected for identity in edges[i]})
                self.assertEqual(witness['candidate_items'],neighbor_ids)
                self.assertEqual(witness['missing'],len(selected)-len(neighbor_ids))
            else:
                self.assertEqual((result['total_route_steps'],tuple(a['item'] for a in result['assignments'])),best)

    def test_random_inventories_match_exact_permutation_objective(self):
        rng=random.Random(98411)
        for _ in range(120):
            cells=rng.sample(range(9),2)
            raw=capture((3,3,1),origin=(1,1,5),changes={i:1 for i in cells})
            positions=[(i%3+1,i//3+1,5) for i in range(9) if i not in cells]
            targets=rng.sample(positions,3)
            req=request(*(Slot(chr(97+i),'bed',p,material=None if i<2 else (419,-1)) for i,p in enumerate(targets)))
            candidates=tuple(item(i,p,material=(419+i%2,-1)) for i,p in enumerate(rng.choices(positions,k=4)))
            result=allocate_routes(req,candidates,raw)
            best, maximum, _=oracle(req,candidates,raw)
            self.assertEqual(result['maximum_assignable'],maximum)
            if best is not None:
                self.assertEqual((result['total_route_steps'],tuple(a['item'] for a in result['assignments'])),best)
            else:
                self.assertIsNone(result['plan'])

    def test_input_order_determinism_and_digest_invalidators(self):
        raw=capture()
        req=request(Slot('b','bed',(3,3,5)),Slot('a','bed',(1,1,5)))
        candidates=(item(2,(1,3,5)),item(1,(3,1,5)),item(4,(2,2,5)))
        first=allocate_routes(req,candidates,raw)
        self.assertEqual(first,allocate_routes(req,tuple(reversed(candidates)),raw))
        for changed in [capture(tick=4), capture(changes={0:1})]:
            self.assertNotEqual(first['decision_digest'],allocate_routes(req,candidates,changed)['decision_digest'])
        self.assertNotEqual(first['decision_digest'],allocate_routes(req,candidates,raw,max_route_steps=10)['decision_digest'])
        # Even an unselected candidate's changed source facts invalidate identity.
        different=candidates+(item(99,(4,4,5)),)
        self.assertNotEqual(first['candidate_digest'],allocate_routes(req,different,raw)['candidate_digest'])

    def test_exact_work_boundary_and_every_cooperative_abort(self):
        raw=capture()
        req=request(Slot('a','bed',(1,1,5)))
        candidates=(item(1,(4,4,5)),)
        result=allocate_routes(req,candidates,raw)
        exact=allocate_routes(req,candidates,raw,maximum_work=result['work_units'])
        self.assertEqual(exact['plan'],result['plan'])
        self.assertEqual(exact['work_units'],result['work_units'])
        with self.assertRaises(ValueError):
            allocate_routes(req,candidates,raw,maximum_work=result['work_units']-1)
        calls=[]
        allocate_routes(req,candidates,raw,lambda:calls.append(1))
        for stop in range(1,len(calls)+1):
            current=0
            def abort():
                nonlocal current
                current+=1
                if current==stop:
                    raise ValueError('cancelled')
            with self.assertRaises(ValueError):
                allocate_routes(req,candidates,raw,abort)

    def test_capture_rejects_every_truncation_trailing_bytes_wrong_profile_and_bad_tiles(self):
        raw=capture((2,1,1),origin=(1,1,5))
        for length in range(len(raw)):
            with self.assertRaises(ValueError):
                decode_terrain(raw[:length])
        for value in [raw+b'x', b'X'+raw[1:], b'x'*(MAX_CAPTURE_BYTES+1),
                      capture((2,1,1),origin=(1,1,5),changes={1:(2**31,3,0,0,0,0,0,0,1,0,0)}),
                      capture((2,1,1),origin=(1,1,5),changes={1:(1,9,0,0,0,0,0,0,1,0,0)})]:
            with self.assertRaises(ValueError):
                decode_terrain(value)

    def test_bad_candidates_limits_and_fortress_are_rejected_before_result(self):
        req=request(Slot('a','bed',(1,1,5)))
        candidates=(item(1,(1,1,5)),)
        for kwargs in [dict(max_route_steps=-1),dict(max_route_steps=True),dict(maximum_work=0),dict(maximum_work=MAX_WORK+1)]:
            with self.assertRaises(ValueError): allocate_routes(req,candidates,capture(),**kwargs)
        with self.assertRaises(ValueError): allocate_routes(req,candidates+ candidates,capture())
        with self.assertRaises(ValueError): allocate_routes(req,candidates,capture(folder='other'))
        with self.assertRaises(ValueError): allocate_routes(req,candidates,capture(site=3))

    def test_maximum_inventory_all_slots_and_output_are_bounded(self):
        req=request(*(Slot(f's{i:02}','bed',(i+1,1,5)) for i in range(32)))
        candidates=tuple(item(i,(1,1,5)) for i in range(65536))
        result=allocate_routes(req,candidates,capture((34,1,1),origin=(0,1,5)))
        self.assertEqual(result['status'],'allocated')
        self.assertEqual([a['item'] for a in result['assignments']],list(range(32)))
        self.assertTrue(all(c['count']==65536 for c in result['compatible_counts']))
        self.assertLessEqual(len(canonical(result)),49152)
        self.assertLessEqual(result['work_units'],MAX_WORK)

    def test_maximum_terrain_and_long_corridor_do_not_recurse(self):
        changes={i:1 for i in range(128*128)}
        floor=(1,3,0,0,0,0,0,0,1,10015,10015)
        for y in range(0,128,2):
            for x in range(128): changes[y*128+x]=floor
            if y<126: changes[(y+1)*128+(127 if (y//2)%2==0 else 0)]=floor
        raw=capture((128,128,1),origin=(1,1,5),changes=changes,dimensions=(256,256,128))
        result=allocate_routes(request(Slot('a','bed',(1,127,5))), (item(1,(1,1,5)),),raw)
        self.assertEqual(result['total_route_steps'],8254)
        self.assertLess(len(canonical(result)),5000)
        self.assertEqual(len(result['assignments'][0]['route_sha256']),64)


if __name__=='__main__': unittest.main()
