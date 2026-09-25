"""Execute the blueprint evaluator on native-layout map/1.5 bytes, not a mirror."""
from dataclasses import replace
from itertools import product
import json
import struct
import unittest

import excavation_blueprint as b
import excavation_observer as e


def capture(region, tick=100, cells=None, *, folder='region1', site=1,
            manifest=None, dimensions=(32768, 32768, 32768)):
    """Explicit native-layout test double, x-fast then y then z; no live DF claim."""
    cells = cells or {}
    encoded_folder = folder.encode('utf-8')
    raw = bytearray(b'DFMM1500')
    raw += struct.pack('>IIBIH', tick // 403200, tick % 403200, 1, site, len(encoded_folder))
    raw += encoded_folder
    raw += struct.pack('>10I', *dimensions, *region.origin, *region.size, region.volume)
    for index in range(region.volume):
        # (presence, shape, liquid depth, dig designation)
        presence, shape, depth, dig = cells.get(index, (2, 3, 0, 0))
        raw += bytes((presence,))
        if presence == 2:
            raw += struct.pack('>IBBBBBBBIHH', 42, shape, depth, 0, 0, dig, 0, 0, 1, 10015, 10015)
    return e.decode_capture(bytes(raw), manifest or e.Manifest(7, 'test-df', 'test-dfhack'), region)


def part(origin=(10, 20, 30), size=(1, 1, 1), shape='floor'):
    return b.Part(e.Region(origin, size), shape)


def goal(blueprint=None, **options):
    return b.BlueprintGoal(blueprint or b.Blueprint((part(),)), 'region1', 1,
                           options.pop('deadline_tick', 1000), **options)


class BlueprintTests(unittest.TestCase):
    def test_canonical_mask_identity_and_split_equivalence(self):
        whole = b.Blueprint((part(size=(2, 1, 1)),))
        halves = (part(origin=(11, 20, 30)), part())
        split = b.Blueprint(halves)
        self.assertEqual(whole.digest, split.digest)
        self.assertEqual(split, b.Blueprint(tuple(reversed(halves))))
        self.assertEqual(split, b.Blueprint.decode(e.canonical(split.json())))
        self.assertNotEqual(whole.digest, b.Blueprint((part(size=(2, 1, 1), shape='wall'),)).digest)
        self.assertNotEqual(whole.digest, b.Blueprint((part(origin=(11, 20, 30), size=(2, 1, 1)),)).digest)
        self.assertEqual(goal(split), b.BlueprintGoal.from_json(goal(split).json()))

    def test_overlap_is_not_silently_deduplicated(self):
        for shape in ('floor', 'wall'):
            with self.assertRaises(e.Rejected):
                b.Blueprint((part(size=(2, 2, 2)), part(origin=(11, 21, 31), shape=shape)))

    def test_extent_and_count_bounds_before_expansion(self):
        valid = b.Blueprint((part(size=(8, 8, 8)),))
        self.assertEqual(len(valid.targets), 512)
        sparse = b.Blueprint((part(size=(8, 8, 7)), part(origin=(17, 27, 45))))
        self.assertEqual(sparse.region.volume, 1024)
        for parts in ((), (part(),) * 33,
                      (part(size=(8, 8, 8)), part(origin=(18, 20, 30))),
                      (part(), part(origin=(42, 52, 30))),
                      (part(), part(origin=(138, 20, 30)))):
            with self.assertRaises(e.Rejected):
                b.Blueprint(parts)
        with self.assertRaises(e.Rejected):
            part(size=(8, 8, 9))
        with self.assertRaises(e.Rejected):
            part(origin=(32767, 0, 0), size=(2, 1, 1))
        accepted = b.Blueprint(tuple(part(origin=(i, 0, 0)) for i in range(32)))
        self.assertEqual(len(accepted.targets), 32)
        self.assertEqual(b.Blueprint((part(origin=(32767, 32767, 32767)),)).region.volume, 1)

    def test_strict_closed_input_schema_and_byte_nesting_limits(self):
        value = b.Blueprint((part(),)).json()
        variants = [[], None, dict(value, commit=True), dict(value, schema='dfmcp.excavation-blueprint/2'),
                    dict(value, parts=[]), dict(value, parts=[dict(value['parts'][0], lua='return true')])]
        for shape in ('other', 'FLOOR', 'stair', 3, True, [], None):
            variants.append(dict(value, parts=[dict(value['parts'][0], shape=shape)]))
        for bad in (True, 1.5, -1, '1'):
            variants.append(dict(value, parts=[{'region': {'origin': [bad, 0, 0], 'size': [1, 1, 1]}, 'shape': 'floor'}]))
        for variant in variants:
            with self.subTest(variant=variant), self.assertRaises(e.Rejected):
                b.Blueprint.decode(json.dumps(variant).encode())
        for raw in (b'', b' ' * (b.MAX_SPEC_BYTES + 1), b'[' * 1000 + b']' * 1000,
                    b'{"schema": "x", "schema": "y", "parts": []}', b'\xff', b'NaN'):
            with self.assertRaises(e.Rejected):
                b.Blueprint.decode(raw)
        for field, value in (('required_samples', 0), ('stable_ticks', -1), ('max_gap_ticks', 0),
                             ('deadline_tick', e.MAX_TICK + 1), ('site', True), ('folder', '')):
            options = goal().json()
            options[field] = value
            with self.assertRaises(e.Rejected):
                b.BlueprintGoal.from_json(options)

    def test_all_normalized_shapes_liquids_designations(self):
        cases = 0
        for expected in range(1, 9):
            g = goal(b.Blueprint((part(shape=b.SHAPES[expected]),)), stable_ticks=0, required_samples=1)
            for shape, depth, dig in product(range(9), range(8), range(8)):
                c = capture(g.region, cells={0: (2, shape, depth, dig)})
                result = b.advance(g, None, c)
                self.assertEqual(result.status == 'satisfied', (shape, depth, dig) == (expected, 0, 0))
                cases += 1
        self.assertEqual(cases, 4608)

    def test_sparse_multilevel_capture_and_native_index_order(self):
        blueprint = b.Blueprint((part(origin=(11, 22, 31), shape='stair_down'),
                                 part(shape='stair_up')))
        self.assertEqual(blueprint.targets, ((0, 6), (11, 7)))
        self.assertEqual(blueprint.position(11), (11, 22, 31))
        cells = {i: (1, 0, 0, 0) for i in range(12)}
        cells.update({0: (2, 6, 0, 0), 11: (2, 7, 0, 0)})
        c = capture(blueprint.region, cells=cells)
        result = b.diagnose(blueprint, c)
        self.assertEqual(result['counts']['matched'], 2)
        self.assertEqual(result['counts']['hidden'], 0)
        self.assertEqual(result['unselected_tiles'], 10)
        self.assertEqual(b.advance(goal(blueprint, stable_ticks=0, required_samples=1), None, c).status, 'satisfied')
        cells[11] = (1, 7, 0, 0)
        unknown = b.diagnose(blueprint, capture(blueprint.region, cells=cells))
        self.assertEqual(unknown['remaining'][0]['observed'], None)
        self.assertEqual(unknown['remaining'][0]['mismatches'], [])
        self.assertEqual(unknown['counts']['hidden'], 1)
        cells[11] = (0, 7, 0, 0)
        self.assertEqual(b.advance(goal(blueprint), None, capture(blueprint.region, cells=cells)).status, 'unknown')

    def test_whole_rows_counts_omissions_and_no_hidden_attributes(self):
        blueprint = b.Blueprint((part(size=(4, 1, 1)),))
        c = capture(blueprint.region, cells={0: (2, 2, 7, 1), 1: (1, 3, 0, 0), 2: (0, 3, 0, 0)})
        result = b.diagnose(blueprint, c, 1)
        self.assertEqual(result['counts'], {'matched': 1, 'mismatched': 1, 'hidden': 1, 'missing': 1,
                                           'wrong_shape': 1, 'wet': 1, 'designated': 1})
        self.assertEqual(result['remaining_omitted'], 2)
        self.assertEqual(result['remaining'][0]['mismatches'], ['wrong_shape', 'wet', 'designated'])
        self.assertEqual(b.diagnose(blueprint, c, 0)['remaining_omitted'], 3)
        self.assertEqual(result['observation_witness'], c.witness)
        self.assertFalse(result['safety_proven'])
        self.assertFalse(result['mining_action_completed_proven'])
        for bad in (-1, 65, True):
            with self.assertRaises(e.Rejected):
                b.diagnose(blueprint, c, bad)

    def test_caller_cannot_forge_derived_capture_facts(self):
        g = goal(stable_ticks=0, required_samples=1)
        wall = capture(g.region, cells={0: (2, 2, 0, 0)})
        floor = capture(g.region)
        forged = replace(wall, tiles=floor.tiles, tick=999)
        self.assertEqual(b.advance(g, None, forged).status, 'pending')
        self.assertEqual(b.diagnose(g.blueprint, forged)['observed_tick'], 100)
        with self.assertRaises(e.Rejected):
            b.advance(g, None, capture(e.Region((9, 20, 30), (1, 1, 1))))

    def test_targets_must_match_simultaneously_not_accumulate_across_parts(self):
        g = goal(b.Blueprint((part(size=(2, 1, 1)),)))
        prior = None
        for tick in range(100, 120):
            prior = b.advance(g, prior, capture(g.region, tick, {tick % 2: (2, 2, 0, 0)}))
            self.assertEqual((prior.status, prior.streak), ('pending', 0))
        prior = b.advance(g, prior, capture(g.region, 120))
        self.assertEqual(prior.status, 'stabilizing')
        self.assertEqual(b.advance(g, prior, capture(g.region, 130)).status, 'satisfied')

    def test_tick_stability_interruptions_gaps_and_deadline(self):
        g = goal(stable_ticks=10, max_gap_ticks=10)
        p = b.advance(g, None, capture(g.region))
        for _ in range(4):
            p = b.advance(g, p, capture(g.region))
            self.assertEqual(p.streak, 1)
        p = b.advance(g, p, capture(g.region, 111))
        self.assertEqual((p.streak, p.since_tick, p.interruption), (1, 111, 'sample_gap_reset'))
        p = b.advance(g, p.interrupted('unfinished_read'), capture(g.region, 120))
        self.assertEqual((p.streak, p.since_tick), (1, 120))
        p = b.advance(g, p, capture(g.region, 125, {0: (1, 0, 0, 0)}))
        self.assertEqual((p.status, p.streak), ('unknown', 0))
        p = b.advance(g, p, capture(g.region, 130))
        p = b.advance(g, p, capture(g.region, 140))
        self.assertEqual(p.status, 'satisfied')
        with self.assertRaises(e.Rejected):
            b.advance(g, p, capture(g.region, 150))
        exact = goal(deadline_tick=110)
        first = b.advance(exact, None, capture(exact.region))
        self.assertEqual(b.advance(exact, first, capture(exact.region, 110)).status, 'satisfied')
        self.assertEqual(b.advance(exact, first, capture(exact.region, 111)).status, 'expired')
        with self.assertRaises(e.Rejected):
            b.advance(exact, None, capture(exact.region, 111))

    def test_source_identity_clock_and_dimension_changes_invalidate(self):
        g = goal()
        c = capture(g.region)
        p = b.advance(g, None, c)
        variants = [capture(g.region, 99), capture(g.region, 110, folder='other'),
                    capture(g.region, 110, site=2), capture(g.region, 110, dimensions=(32767, 32768, 32768)),
                    capture(g.region, 110, manifest=e.Manifest(8, 'test-df', 'test-dfhack')),
                    capture(g.region, 110, manifest=e.Manifest(7, 'new-df', 'test-dfhack'))]
        for changed in variants:
            result = b.advance(g, p, changed)
            self.assertEqual(result.status, 'invalidated')
            self.assertEqual(result.latest.witness, c.witness)
            with self.assertRaises(e.Rejected):
                b.advance(g, result, c)
        with self.assertRaises(e.Rejected):
            b.advance(g, None, capture(g.region, site=2))

    def test_floor_transition_differential_against_unchanged_legacy_evaluator(self):
        region = e.Region((10, 20, 30), (2, 1, 1))
        legacy = e.Goal(region, 'region1', 1, 115, 5, 2, 8)
        new = goal(b.Blueprint((part(size=region.size),)), deadline_tick=115,
                   stable_ticks=5, required_samples=2, max_gap_ticks=8)
        kinds = [(2, 3, 0, 0), (2, 2, 0, 0), (2, 3, 1, 0), (1, 0, 0, 0), (0, 0, 0, 0)]
        for sequence in product(kinds, repeat=3):
            for ticks in ((100, 100, 105), (100, 105, 115), (100, 109, 116), (100, 99, 105)):
                left = right = None
                for tick, state in zip(ticks, sequence):
                    c = capture(region, tick, {0: state})
                    left = e.advance(legacy, left, c)
                    right = b.advance(new, right, c)
                    self.assertEqual(left, right)
                    if left.status in e.TERMINAL:
                        break


if __name__ == '__main__':
    unittest.main()
