#!/usr/bin/env python3
"""Execute the sparse normal-mining compiler, not a parallel reference model."""
import itertools
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

import dig_blueprint as b
import dig_designation_client as d


def specification(parts):
    return {'schema': b.SCHEMA, 'parts': [{'region': {'origin': list(origin), 'size': list(size)},
                                         'shape': 'floor'} for origin, size in parts]}


class LayoutTests(unittest.TestCase):
    def assert_exact(self, layout, expected):
        seen = set()
        for region in layout.regions():
            covered = b.cells(region)
            self.assertFalse(seen & covered)
            seen |= covered
        self.assertEqual(seen, set(expected))
        self.assertEqual(seen, set(layout.targets))
        self.assertLessEqual(len(layout.rectangles), b.MAX_STEPS)

    def test_every_four_by_three_sparse_mask_is_exact(self):
        positions = [(x + 15, y + 15, 2) for y in range(3) for x in range(4)]
        for mask in range(1, 1 << len(positions)):
            chosen = [p for bit, p in enumerate(positions) if mask & (1 << bit)]
            layout = b.Layout.from_json(specification([(p, (1, 1, 1)) for p in chosen]))
            self.assert_exact(layout, chosen)

    def test_large_rooms_split_at_native_width_height_without_extra_cells(self):
        for width, height in itertools.product(range(1, 25), range(1, 17)):
            if width * height > b.MAX_TARGETS:
                continue
            layout = b.Layout.from_json(specification([((10, 10, 2), (width, height, 1))]))
            self.assert_exact(layout, [(x, y, 2) for y in range(10, 10 + height)
                                      for x in range(10, 10 + width)])

    def test_levels_and_doorways_keep_intervening_walls_unselected(self):
        parts = [((5, 5, 2), (3, 3, 2)), ((9, 5, 2), (3, 3, 2)),
                 ((6, 8, 2), (1, 1, 2)), ((10, 8, 2), (1, 1, 2)), ((6, 9, 2), (5, 1, 2))]
        layout = b.Layout.from_json(specification(parts))
        expected = {(x, y, z) for (a, c, e), (w, h, levels) in parts
                    for z in range(e, e + levels) for y in range(c, c + h) for x in range(a, a + w)}
        self.assert_exact(layout, expected)
        self.assertNotIn((8, 6, 2), layout.targets)
        self.assertEqual({p[2] for p in layout.rectangles}, {2, 3})

    def test_identity_ignores_partition_order_and_input_splitting(self):
        one = b.Layout.from_json(specification([((5, 5, 2), (9, 4, 1))]))
        split = [((5, 5, 2), (4, 4, 1)), ((9, 5, 2), (5, 4, 1))]
        for values in (split, split[::-1]):
            other = b.Layout.from_json(specification(values))
            self.assertEqual(one.blueprint_digest, other.blueprint_digest)
            self.assertEqual(one.digest, other.digest)
            self.assertEqual(one.rectangles, other.rectangles)
        moved = b.Layout.from_json(specification([((6, 5, 2), (9, 4, 1))]))
        self.assertNotEqual(one.digest, moved.digest)

    def test_unsupported_goals_and_overlap_never_become_normal_mining(self):
        for shape in ('wall', 'empty', 'ramp', 'ramp_top', 'stair_up', 'stair_down', 'stair_up_down', 'other'):
            value = specification([((5, 5, 2), (1, 1, 1))])
            value['parts'][0]['shape'] = shape
            with self.assertRaises(ValueError):
                b.Layout.from_json(value)
        with self.assertRaises(ValueError):
            b.Layout.from_json(specification([((5, 5, 2), (2, 2, 1)), ((6, 6, 2), (1, 1, 1))]))

    def test_native_halo_boundaries_are_not_clamped(self):
        for origin, size in [((0, 5, 2), (1, 1, 1)), ((1, 0, 2), (1, 1, 1)),
                             ((1, 1, 0), (1, 1, 1)), ((32766, 1, 2), (2, 1, 1)),
                             ((1, 1, 32766), (1, 1, 2))]:
            with self.assertRaises(ValueError):
                b.Layout.from_json(specification([(origin, size)]))
        self.assert_exact(b.Layout.from_json(specification([((32766, 32766, 32766), (1, 1, 1))])),
                          [(32766, 32766, 32766)])

    def test_closed_json_integer_and_extent_bounds(self):
        good = specification([((5, 5, 2), (1, 1, 1))])
        for bad in ({**good, 'command': 'x'}, {**good, 'schema': 'other'}, {**good, 'parts': []},
                    specification([((5, 5, 2), (0, 1, 1))]), specification([((5, 5, 2), (True, 1, 1))]),
                    specification([((5, 5, 2), (1.0, 1, 1))]), specification([((5, 5, 2), (32, 32, 1))]),
                    specification([((1, 1, 1), (1, 1, 1)), ((100, 100, 1), (1, 1, 1))])):
            with self.assertRaises(ValueError):
                b.Layout.decode(d.canonical(bad))
        for raw in (b'{"schema":1,"schema":2}', b'[' * 9 + b']' * 9, b' ' * (b.MAX_INPUT + 1), b'\xff'):
            with self.assertRaises(ValueError):
                b.Layout.decode(raw)

    def test_capacity_is_checked_before_a_plan_is_returned(self):
        # Seventeen alternating one-tile columns across 16 rows require 136
        # 1x8 native rectangles, despite fitting the 512-cell/capture bounds.
        parts = [((2 + x * 2, 2, 2), (1, 16, 1)) for x in range(17)]
        # A contiguous column compacts vertically, so use alternating levels of
        # horizontal strips: 17 columns x 8 z-levels = 136 separate rectangles.
        parts = [((2 + x * 2, 2, 2), (1, 1, 8)) for x in range(17)]
        with self.assertRaisesRegex(ValueError, '128 native'):
            b.Layout.from_json(specification(parts))

    def test_cli_executes_compiler_and_reports_no_effect(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'layout.json'
            path.write_bytes(d.canonical(specification([((5, 5, 2), (16, 3, 1))])))
            result = subprocess.run([sys.executable, str(Path(b.__file__)), str(path)],
                                    capture_output=True, text=True, timeout=10, check=True)
            value = json.loads(result.stdout)
            self.assertEqual(value['step_count'], 2)
            self.assertFalse(value['mutation_authority_granted'])
            self.assertFalse(value['excavation_completion_proven'])


if __name__ == '__main__':
    unittest.main()
