"""Pure stage-guard and serialization tests; NOT journal/native integration.

The lightweight objects below represent already-replayed, already-decoded
inputs. Native and private-file behavior is tested separately by
`test_room_furniture_export`; these tests deliberately claim neither.
"""
from copy import deepcopy
from types import SimpleNamespace as NS
import json
import unittest

import export_room_furniture as x


def idle():
    pass


def state():
    binding = {'generation': 7, 'world_folder': 'region1', 'site': 2,
               'dimensions': [100, 100, 10], 'df_version': 'df', 'dfhack_version': 'dfhack'}
    source = lambda tick: NS(tick=tick, binding=lambda: deepcopy(binding))
    goal = NS(required_samples=2, stable_ticks=10, max_gap_ticks=20,
              region=NS(origin=(10, 10, 2), size=(3, 1, 1)), room_plan=NS(digest='plan'))
    history = NS(goal=goal, pending_read=False,
                 progress=NS(status='satisfied', streak=2, since_tick=100,
                             first=source(100), latest=source(110)))
    floor = lambda: NS(presence=2, attributes=(42, 3, 0, 0, 0, 0, 0, 0, 1, 10015, 10015))
    observed = NS(tick=115, witness='fresh', binding=lambda: deepcopy(binding),
                  tiles=[floor(), floor(), floor()])
    diagnosis = {'all_required_shapes_at_sample': True, 'deficits': {'count': 0},
                 'observation_witness': 'fresh', 'observed_tick': 115, 'room_plan_digest': 'plan'}
    request = NS(slots=[NS(target=(10, 10, 2)), NS(target=(11, 10, 2))])
    return history, observed, diagnosis, request


class TransitionUnitTests(unittest.TestCase):
    def test_complete_stable_goal_and_fresh_whole_map_accept_exact_gap(self):
        h, o, d, r = state()
        self.assertEqual(x._verify_transition(h, o, d, r, idle), 5)
        for tick in (110, 130):
            o.tick, d['observed_tick'] = tick, tick
            self.assertEqual(x._verify_transition(h, o, d, r, idle), tick - 110)

    def test_nonterminal_cancelled_expired_invalidated_and_pending_read_refuse(self):
        for status in ('pending', 'stabilizing', 'unknown', 'cancelled', 'expired', 'invalidated'):
            with self.subTest(status=status):
                h, o, d, r = state()
                h.progress.status = status
                self.assertRaises(ValueError, x._verify_transition, h, o, d, r, idle)
        h, o, d, r = state()
        h.pending_read = True
        self.assertRaises(ValueError, x._verify_transition, h, o, d, r, idle)

    def test_stability_count_span_and_missing_origin_are_not_interchangeable(self):
        for name, value in (('streak', 1), ('since_tick', 105), ('since_tick', None)):
            h, o, d, r = state()
            setattr(h.progress, name, value)
            self.assertRaises(ValueError, x._verify_transition, h, o, d, r, idle)

    def test_source_incarnation_dimensions_software_and_both_clock_edges_refuse(self):
        for field in ('generation', 'world_folder', 'site', 'dimensions', 'df_version', 'dfhack_version'):
            h, o, d, r = state()
            changed = o.binding()
            changed[field] = 'different'
            o.binding = lambda: changed
            self.assertRaises(ValueError, x._verify_transition, h, o, d, r, idle)
        for tick in (109, 131):
            h, o, d, r = state()
            o.tick, d['observed_tick'] = tick, tick
            self.assertRaises(ValueError, x._verify_transition, h, o, d, r, idle)
        h, o, d, r = state()
        h.progress.latest.binding = lambda: {'substituted': True}
        self.assertRaises(ValueError, x._verify_transition, h, o, d, r, idle)

    def test_whole_goal_diagnosis_must_match_this_capture_and_original_plan(self):
        for key, value in (('all_required_shapes_at_sample', False),
                           ('deficits', {'count': 1}), ('observation_witness', 'old'),
                           ('observed_tick', 100), ('room_plan_digest', 'residual')):
            h, o, d, r = state()
            d[key] = value
            self.assertRaises(ValueError, x._verify_transition, h, o, d, r, idle)

    def test_each_furniture_target_requires_visible_clear_floor(self):
        for presence in (0, 1):
            h, o, d, r = state()
            o.tiles[1].presence = presence
            self.assertRaises(ValueError, x._verify_transition, h, o, d, r, idle)
        for index, value in ((1, 2), (2, 1), (3, 1), (5, 1), (6, 1), (7, 1)):
            h, o, d, r = state()
            attrs = list(o.tiles[1].attributes)
            attrs[index] = value
            o.tiles[1].attributes = tuple(attrs)
            self.assertRaises(ValueError, x._verify_transition, h, o, d, r, idle)

    def test_occupied_non_target_floor_does_not_invent_placement_blocker(self):
        h, o, d, r = state()
        attrs = list(o.tiles[2].attributes)
        attrs[7] = 1
        o.tiles[2].attributes = tuple(attrs)
        self.assertEqual(x._verify_transition(h, o, d, r, idle), 5)

    def test_no_empty_oversized_or_out_of_capture_request(self):
        for slots in ([], [NS(target=(10, 10, 2))] * 33,
                      [NS(target=(9, 10, 2))], [NS(target=(10, 10, 3))]):
            h, o, d, r = state()
            r.slots = slots
            self.assertRaises(ValueError, x._verify_transition, h, o, d, r, idle)

    def test_all_32_targets_and_every_guard_boundary_are_checked(self):
        h, o, d, r = state()
        h.goal.region.size = (32, 1, 1)
        o.tiles = [deepcopy(o.tiles[0]) for _ in range(32)]
        r.slots = [NS(target=(10 + i, 10, 2)) for i in range(32)]
        checks = []
        x._verify_transition(h, o, d, r, lambda: checks.append(1))
        self.assertEqual(len(checks), 34)
        class Stop(Exception):
            pass
        for stop in range(len(checks)):
            calls = [0]
            def guard():
                if calls[0] == stop:
                    raise Stop()
                calls[0] += 1
            self.assertRaises(Stop, x._verify_transition, h, o, d, r, guard)

    def test_narrow_export_preserves_canonical_request_without_newline(self):
        request = {'schema': 'dfmcp.furniture-request/1', 'world_folder': 'région',
                   'excluded_items': [4, 8], 'slots': [{'after': ['first'], 'material': [0, -1]}]}
        packet = {'furniture_request': request, 'extra_evidence': 'retained'}
        raw = x._select_output(packet, 'request', idle)
        self.assertEqual(raw, x._canonical(request))
        self.assertIn(b'r\\u00e9gion', raw)
        self.assertFalse(raw.endswith(b'\n'))
        self.assertEqual(json.loads(x._select_output(packet, 'report', idle)), packet)

    def test_narrow_export_still_reserves_full_report_and_rechecks_guard(self):
        packet = {'furniture_request': {}, 'oversized': 'x' * x.MAX_OUTPUT}
        self.assertRaises(ValueError, x._select_output, packet, 'request', idle)
        packet = {'furniture_request': {'oversized': 'x' * x.MAX_REQUEST}}
        self.assertRaises(ValueError, x._select_output, packet, 'request', idle)
        self.assertRaises(ValueError, x._select_output, {'furniture_request': {}}, 'partial', idle)
        class Stop(Exception):
            pass
        for stop in (0, 1):
            calls = [0]
            def guard():
                if calls[0] == stop:
                    raise Stop()
                calls[0] += 1
            self.assertRaises(Stop, x._select_output, {'furniture_request': {}}, 'request', guard)

    def test_refusal_contains_no_partial_request_or_effect_authority(self):
        value = json.loads(x.refused())
        self.assertFalse(value['ok'])
        self.assertFalse(value['game_mutations_dispatched'])
        self.assertFalse(value['journal_written'])
        self.assertFalse(value['retry_designation_permitted'])
        self.assertNotIn('furniture_request', value)
        self.assertEqual(value['agent_turn']['references'], [])
        self.assertLess(len(x.refused()), x.MAX_OUTPUT)


if __name__ == '__main__':
    unittest.main()
