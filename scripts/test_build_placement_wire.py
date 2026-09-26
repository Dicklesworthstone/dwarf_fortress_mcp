#!/usr/bin/env python3
"""Execute furniture/1.19 codec against independent native vectors and hostile receipts."""
from __future__ import annotations

from dataclasses import FrozenInstanceError, replace
import hashlib
import itertools
import json
from pathlib import Path
import struct
import unittest

import build_placement_wire as w

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = json.loads((ROOT / 'bridge/common/tests/fixtures/build_placement_v1_19.json').read_text(encoding='utf-8'))
CAPTURE = bytes.fromhex(FIXTURES['capture'])
PLACED = bytes.fromhex(FIXTURES['placed'])
# Independently specified field positions for the retained golden capture.
SELECTION = 72
TILES = 89
ITEM = 215
# The placed golden has its complete 286-byte after-capture, then a 53-byte proof.
AFTER = PLACED[350:636]
INSERTION = PLACED[638:691]


def sha(domain: bytes, raw: bytes) -> bytes:
    return hashlib.sha256(domain + b'\0' + raw).digest()


def frame(raw: bytes) -> bytes:
    return struct.pack('>H', len(raw)) + raw


def patch(raw: bytes, offset: int, new: bytes) -> bytes:
    return raw[:offset] + new + raw[offset + len(new):]


def n32(value: int) -> bytes:
    return struct.pack('>I', value)


def rehash(raw: bytes) -> bytes:
    return raw[:-32] + sha(b'dfmcp-build-receipt/1', raw[:-32])


def record(*, before=CAPTURE, key=b'golden', phase=0, reason=0, attempted=None,
           after=None, insertion=None, plan=None, token=None, trailing=b'') -> bytes:
    """Independent raw envelope writer permits corrupt claims the codec forbids."""
    selection_offset = 65 + int.from_bytes(before[60:62], 'big')
    selection = before[selection_offset:selection_offset + 17]
    if plan is None:
        plan = sha(b'dfmcp-build-plan/1', selection + hashlib.sha256(before).digest())
    if token is None:
        token = sha(b'dfmcp-build-token/1', frame(key) + plan)[:16]
    attempted = phase in (1, 2) if attempted is None else attempted
    raw = (b'DFMBR019' + frame(key) + frame(before) + plan + token
           + bytes([phase, reason, attempted, after is not None]))
    if after is not None:
        raw += frame(after) + frame(INSERTION if insertion is None else insertion)
    raw += trailing
    return raw + sha(b'dfmcp-build-receipt/1', raw)


class FurnitureWireTests(unittest.TestCase):
    def setUp(self):
        self.before = w.Capture.decode(CAPTURE)
        self.plan = w.Plan('golden', self.before)

    def assert_rejected(self, call, *args, **kwargs):
        with self.assertRaises(w.Rejected):
            call(*args, **kwargs)

    def test_all_eight_independent_native_fixtures(self):
        self.assertEqual(set(FIXTURES), {'capture', 'plan', 'token', 'prepared', 'placed',
                                         'indeterminate', 'expired', 'cancelled'})
        self.assertEqual(self.before.raw, CAPTURE)
        self.assertEqual(self.before.witness, hashlib.sha256(CAPTURE).digest())
        self.assertEqual(self.plan.digest.hex(), FIXTURES['plan'])
        self.assertEqual(self.plan.token.hex(), FIXTURES['token'])
        self.assertEqual(w.plan_for(self.before.selection, self.before.witness), self.plan.digest)
        self.assertEqual(w.token_for('golden', self.plan.digest), self.plan.token)
        for name in ('prepared', 'placed', 'indeterminate', 'expired', 'cancelled'):
            with self.subTest(name=name):
                raw = bytes.fromhex(FIXTURES[name])
                parsed = w.verify_record(raw, self.plan)
                self.assertEqual(parsed.raw, raw)
                self.assertEqual(parsed.plan, self.plan)
                self.assertEqual(parsed.phase, 'refused' if name == 'expired' else name)
        self.assertEqual(record(), bytes.fromhex(FIXTURES['prepared']))
        self.assertEqual(record(phase=2, after=AFTER), PLACED)
        self.assertEqual(self.before.expected_after().raw, AFTER)
        proof = w.Insertion.decode(INSERTION)
        self.assertEqual(proof.raw, INSERTION)
        self.assertTrue(proof.matches(self.before))

    def test_every_truncated_prefix_and_trailing_data(self):
        values = [(w.Capture.decode, CAPTURE), (w.Selection.decode, CAPTURE[SELECTION:SELECTION + 17]),
                  (w.Insertion.decode, INSERTION)]
        values.extend((w.Record.decode, bytes.fromhex(FIXTURES[name]))
                      for name in ('prepared', 'placed', 'indeterminate', 'expired', 'cancelled'))
        for decode, raw in values:
            for end in range(len(raw)):
                with self.subTest(decoder=decode.__qualname__, end=end):
                    self.assert_rejected(decode, raw[:end])
            self.assert_rejected(decode, raw + b'\0')

    def test_every_corrupt_record_byte_fails_integrity(self):
        for name in ('prepared', 'placed', 'indeterminate', 'expired', 'cancelled'):
            raw = bytes.fromhex(FIXTURES[name])
            for offset in range(len(raw)):
                with self.subTest(name=name, offset=offset):
                    self.assert_rejected(w.Record.decode, patch(raw, offset, bytes([raw[offset] ^ 128])))

    def test_capture_profile_counters_and_map_bounds(self):
        bad = (
            (0, b'DFMBC018'), (8, struct.pack('>Q', 0)), (8, struct.pack('>Q', w.MAX_U64)),
            (16, struct.pack('>Q', w.MAX_U64)), (24, struct.pack('>Q', w.MAX_TICK + 1)),
            (32, n32(w.MAX_I32 + 1)), (36, n32(0)), (40, n32(32769)),
            (44, n32(0)), (48, n32(w.MAX_I32 + 1)), (52, n32(w.MAX_I32 + 1)),
            (56, n32(w.MAX_BUILDINGS + 1)),
            (SELECTION + 5, n32(0)), (SELECTION + 9, n32(63)),
            (SELECTION + 13, n32(8)), (ITEM + 1, n32(64)),
            (ITEM + 5, n32(64)), (ITEM + 9, n32(8)),
        )
        for offset, data in bad:
            with self.subTest(offset=offset, data=data):
                self.assert_rejected(w.Capture.decode, patch(CAPTURE, offset, data))
        for size in (0, 513, 65535):
            self.assert_rejected(w.Capture.decode, patch(CAPTURE, 60, struct.pack('>H', size)))
        self.assert_rejected(w.Capture.decode, b'x' * (w.MAX_CAPTURE_BYTES + 1))
        self.assert_rejected(w.Record.decode, b'x' * (w.MAX_RECORD_BYTES + 1))

    def test_utf8_and_nonbyte_input_rejected_without_normalizing(self):
        for folder in (b'', b'a\0b', b'\xff', b'\xed\xa0\x80', b'\xc0\xaf', b'\xf4\x90\x80\x80', b'x' * 513):
            raw = CAPTURE[:60] + frame(folder) + CAPTURE[69:]
            self.assert_rejected(w.Capture.decode, raw)
        for folder in ('fort-é-🏰', 'e\u0301', 'é', 'x' * 512):
            value = replace(self.before, folder=folder)
            self.assertEqual(w.Capture.decode(value.raw), value)
        self.assertNotEqual(replace(self.before, folder='e\u0301').witness,
                            replace(self.before, folder='é').witness)
        for raw in (bytearray(CAPTURE), memoryview(CAPTURE), 'DFMBC019', None):
            self.assert_rejected(w.Capture.decode, raw)
        self.assert_rejected(replace, self.before, folder='\ud800')

    def test_strict_boolean_presence_and_enum_bytes(self):
        for offset in (69, 70, 71, TILES + 12, TILES + 13, ITEM + 46, ITEM + 47):
            for value in (2, 255):
                with self.subTest(offset=offset, value=value):
                    self.assert_rejected(w.Capture.decode, patch(CAPTURE, offset, bytes([value])))
        for offset in (*[TILES + i * 14 for i in range(9)], ITEM):
            self.assert_rejected(w.Capture.decode, patch(CAPTURE, offset, b'\3'))
        for value in (0, 4, 255):
            self.assert_rejected(w.Capture.decode, patch(CAPTURE, SELECTION, bytes([value])))
        for value in (4, 255):
            self.assert_rejected(w.Capture.decode, patch(CAPTURE, ITEM + 13, bytes([value])))
        self.assert_rejected(w.Capture.decode, patch(CAPTURE, SELECTION + 1, n32(w.MAX_I32)))

    def test_hidden_and_missing_have_no_attribute_backing(self):
        for presence in (0, 1):
            tile = w.Tile(presence)
            item = w.Item(presence)
            self.assertEqual(tile.encode(), bytes([presence]))
            self.assertEqual(item.encode(), bytes([presence]))
            self.assertEqual(set(tile.view()), {'presence'})
            self.assertEqual(set(item.view()), {'presence'})
            capture = replace(self.before, item=item, tiles=(tile,) * 9)
            self.assertEqual(w.Capture.decode(capture.raw), capture)
            self.assertFalse(capture.eligible)
            self.assert_rejected(w.Plan, 'key', capture)
            self.assert_rejected(w.Tile.decode, bytes([presence, 0]))
            self.assert_rejected(w.Item.decode, bytes([presence, 0]))
            self.assert_rejected(replace, item, quality=1)
            self.assert_rejected(replace, item, ground=w.Tile(1))
            self.assert_rejected(replace, tile, building=0)
            # A visible payload cannot be relabelled hidden while retaining bytes.
            self.assert_rejected(w.Capture.decode, patch(CAPTURE, ITEM, bytes([presence])))
            self.assert_rejected(w.Capture.decode, patch(CAPTURE, TILES, bytes([presence])))

    def test_visible_numeric_limits_and_job_order(self):
        for offset, data in ((TILES + 1, n32(w.MAX_I32 + 1)), (TILES + 5, b'\x09'),
                             (TILES + 6, b'\x08'), (TILES + 7, b'\x08'),
                             (ITEM + 14, n32(w.MAX_I32 + 1)), (ITEM + 30, n32(w.MAX_I32 + 1)),
                             (ITEM + 34, n32(w.MAX_I32 + 1)), (ITEM + 42, n32(4097)),
                             (ITEM + 48, b'\x09')):
            self.assert_rejected(w.Capture.decode, patch(CAPTURE, offset, data))
        for jobs in ((2, 1), (2, 2), (w.MAX_I32,), tuple(range(9))):
            raw = CAPTURE[:ITEM + 48] + bytes([len(jobs)]) + b''.join(n32(j) for j in jobs) + CAPTURE[ITEM + 49:]
            self.assert_rejected(w.Capture.decode, raw)
        item = replace(self.before.item, jobs=tuple(range(8)), subtype=w.MIN_I32,
                       material=w.MAX_I32, material_index=w.MIN_I32,
                       quality=w.MAX_I32, other_flags=w.MAX_U32, other_refs=4096,
                       ground=replace(self.before.item.ground, building=w.MAX_I32 - 1))
        self.assertEqual(w.Item.decode(item.encode()), item)
        self.assertEqual(w.Capture.decode(replace(self.before, item=item).raw).item, item)
        self.assert_rejected(replace, self.before.item, jobs=[1])
        self.assert_rejected(replace, self.before.item, ground=w.Tile(0))

    def test_public_value_construction_is_strict_and_immutable(self):
        for value in (True, 1.0, '1', None):
            self.assert_rejected(w.Selection, value, 42, 15, 15, 2)
            self.assert_rejected(replace, self.before, generation=value)
        for value in (0, 1, 'true', None):
            self.assert_rejected(replace, self.before, paused=value)
            self.assert_rejected(replace, self.before.item, on_ground=value)
        self.assert_rejected(replace, self.before, tiles=list(self.before.tiles))
        self.assert_rejected(replace, self.before, dimensions=[64, 64, 8])
        for count in (0, 8, 10):
            self.assert_rejected(replace, self.before, tiles=(self.before.tiles[0],) * count)
        with self.assertRaises(FrozenInstanceError):
            self.before.tick = 0
        changed = replace(self.before, tick=self.before.tick + 1)
        self.assertNotEqual(changed.raw, CAPTURE)
        self.assertNotEqual(w.Plan('golden', changed).digest, self.plan.digest)

    def test_all_native_preconditions_produce_blockers(self):
        floor = self.before.tiles[4]
        cases = [replace(self.before, **change) for change in (
            {'paused': False}, {'free_tile': False}, {'supported': False},
            {'sequence': w.MAX_U64 - 1}, {'next_building': w.MAX_I32},
            {'next_job': w.MAX_I32}, {'building_count': w.MAX_BUILDINGS})]
        for change in ({'shape': 2}, {'dig': 1}, {'liquid': 1}, {'occupied': True},
                       {'building': 0}, {'occupancy_other': 1}):
            tiles = list(self.before.tiles)
            tiles[4] = replace(floor, **change)
            cases.append(replace(self.before, tiles=tuple(tiles)))
        for n in range(9):
            for tile in (w.Tile(0), w.Tile(1), replace(floor, liquid=1)):
                tiles = list(self.before.tiles)
                tiles[n] = tile
                cases.append(replace(self.before, tiles=tuple(tiles)))
        tiles = tuple(replace(t, shape=2) if n in (1, 3, 5, 7) else t for n, t in enumerate(self.before.tiles))
        cases.append(replace(self.before, tiles=tiles))
        for change in ({'kind': 2}, {'on_ground': False}, {'in_job': True}, {'other_flags': 1},
                       {'other_refs': 1}, {'jobs': (9,)}, {'wear': 1}, {'material': -1},
                       {'ground': replace(floor, liquid=1)}, {'ground': replace(floor, shape=2)},
                       {'ground': replace(floor, occupied=True)}):
            cases.append(replace(self.before, item=replace(self.before.item, **change)))
        for capture in cases:
            with self.subTest(blockers=capture.blockers):
                self.assertFalse(capture.eligible)
                self.assertTrue(w.blockers(capture))
                self.assert_rejected(w.expected_after, capture)
                self.assert_rejected(w.Plan, 'key', capture)
                self.assert_rejected(w.Record.decode, record(before=capture.raw))

    def test_exact_kind_and_permitted_nonblocking_values(self):
        for selected, actual in itertools.product(range(1, 4), range(4)):
            c = replace(self.before, selection=replace(self.before.selection, kind=selected),
                        item=replace(self.before.item, kind=actual))
            self.assertEqual(c.eligible, selected == actual)
        # The engine does not claim pathfinding and permits ground occupancy bits
        # other than buildings, along with nonzero item quality and any subtype.
        c = replace(self.before, item=replace(self.before.item, quality=5, subtype=123,
                    ground=replace(self.before.item.ground, dig=1, building=5, occupancy_other=123)))
        self.assertTrue(c.eligible)
        self.assertFalse(c.view()['pathfinding_proved'])
        for index in (1, 3, 5, 7):
            tiles = tuple(t if n in (4, index) else replace(t, shape=2) for n, t in enumerate(self.before.tiles))
            self.assertTrue(replace(self.before, tiles=tiles).eligible)

    def test_last_ids_and_sequence_use_exact_checked_after_state(self):
        c = replace(self.before, next_building=w.MAX_I32 - 1, next_job=w.MAX_I32 - 1,
                    building_count=w.MAX_BUILDINGS - 1, sequence=w.MAX_U64 - 2)
        after = c.expected_after()
        self.assertEqual((after.next_building, after.next_job, after.building_count, after.sequence),
                         (w.MAX_I32, w.MAX_I32, w.MAX_BUILDINGS, w.MAX_U64 - 1))
        self.assertFalse(after.eligible)
        self.assertEqual(after.tiles[4].building, w.MAX_I32 - 1)
        self.assertEqual(after.item.jobs, (w.MAX_I32 - 1,))
        self.assertEqual(w.Capture.decode(after.raw), after)

    def test_every_phase_reason_attempt_combination(self):
        allowed = {(0, 0), (1, 5), (2, 0), (3, 1), (3, 2), (3, 3), (4, 4)}
        for phase, reason, attempted in itertools.product(range(6), range(7), (0, 1, 2)):
            raw = record(phase=phase, reason=reason, attempted=attempted,
                         after=AFTER if phase == 2 else None)
            if (phase, reason) in allowed and attempted == (phase in (1, 2)):
                self.assertEqual(w.Record.decode(raw).phase, w.PHASES[phase])
            else:
                with self.subTest(phase=phase, reason=reason, attempted=attempted):
                    self.assert_rejected(w.Record.decode, raw)

    def test_rehashed_record_key_plan_token_and_payload_corruption(self):
        for key in (b'', b'../key', b'with space', b'x' * 129, b'\xff'):
            self.assert_rejected(w.Record.decode, record(key=key))
        for changes in ({'plan': b'x' * 32}, {'token': b'x' * 16},
                        {'phase': 0, 'after': AFTER}, {'phase': 2}, {'trailing': b'\0'}):
            self.assert_rejected(w.Record.decode, record(**changes))
        for offset in (346, 347):
            self.assert_rejected(w.Record.decode, rehash(patch(PLACED, offset, b'\2')))
        for expected in (w.Plan('different', self.before),
                         w.Plan('golden', replace(self.before, tick=self.before.tick + 1))):
            self.assert_rejected(w.Record.decode, PLACED, expected)

    def test_rehashed_after_capture_cannot_omit_or_invent_any_change(self):
        after = w.Capture.decode(AFTER)
        changes = [replace(after, **change) for change in (
            {'generation': 42}, {'sequence': 2}, {'tick': after.tick + 1}, {'site': 3},
            {'folder': 'other'}, {'dimensions': (65, 64, 8)}, {'next_building': 72},
            {'next_job': 92}, {'building_count': 6}, {'paused': False}, {'supported': False},
            {'free_tile': True}, {'selection': replace(after.selection, item=43)})]
        for n in range(9):
            tiles = list(after.tiles)
            tiles[n] = replace(tiles[n], tiletype=38)
            changes.append(replace(after, tiles=tuple(tiles)))
        for item_change in ({'jobs': ()}, {'in_job': False}, {'jobs': (91,)}, {'quality': 3},
                            {'native_type': 102}, {'material': 420}, {'pos': (11, 11, 2)},
                            {'ground': replace(after.item.ground, occupancy_other=16)}):
            changes.append(replace(after, item=replace(after.item, **item_change)))
        for changed in [self.before, *changes]:
            self.assert_rejected(w.Record.decode, record(phase=2, after=changed.raw))

    def test_rehashed_insertion_requires_exact_job_item_and_stage(self):
        proof = w.Insertion.decode(INSERTION)
        changes = ({'building': 71}, {'job': 91}, {'item': 43}, {'kind': 2}, {'pos': (16, 15, 2)},
                   {'material': 420}, {'material_index': 0}, {'stage': 1}, {'linked': False},
                   {'construct_job': False}, {'exact_item_link': False}, {'suspended': True})
        for change in changes:
            bad = replace(proof, **change)
            self.assertFalse(bad.matches(self.before))
            self.assert_rejected(w.Record.decode, record(phase=2, after=AFTER, insertion=bad.raw))
        for offset, data in ((0, b'DFMBI018'), (8, n32(w.MAX_I32)), (20, b'\0'),
                             (21, n32(32768)), (41, n32(2)), (45, n32(0)), (45, n32(33)),
                             (49, b'\2'), (50, b'\2'), (51, b'\2'), (52, b'\2')):
            self.assert_rejected(w.Record.decode, record(phase=2, after=AFTER,
                                                        insertion=patch(INSERTION, offset, data)))
        self.assertEqual(w.Record.decode(record(phase=2, after=AFTER,
                             insertion=replace(proof, max_stage=32).raw)).insertion.max_stage, 32)

    def test_replay_is_immutable_after_any_attempt_or_refusal(self):
        values = [w.Record.decode(bytes.fromhex(FIXTURES[name]))
                  for name in ('prepared', 'placed', 'indeterminate', 'expired', 'cancelled')]
        for value in values:
            w.successor(values[0], value)
            w.successor(value, value)
        for previous, current in itertools.product(values[1:], values):
            if previous != current:
                self.assert_rejected(w.successor, previous, current)
        another = w.Record.decode(record(key=b'another'))
        self.assert_rejected(w.successor, values[0], another)

    def test_views_preserve_uncertainty_and_historical_scope(self):
        for name in ('prepared', 'placed', 'indeterminate', 'expired', 'cancelled'):
            parsed = w.Record.decode(bytes.fromhex(FIXTURES[name]))
            view = parsed.view()
            self.assertEqual(view['historical_job_registration_verified'], name == 'placed')
            self.assertEqual(view['operator_attention_required'], name == 'indeterminate')
            self.assertEqual(parsed.attempted, name in ('placed', 'indeterminate'))
            self.assertEqual(parsed.resolved, name in ('placed', 'expired', 'cancelled'))
            for field_name in ('building_completed_proved', 'building_usable_proved',
                               'current_pause_proved', 'checkpoint_proved', 'retry_permitted',
                               'production_admitted'):
                self.assertFalse(view[field_name])
            self.assertLess(len(w.canonical(view)), 16384)

    def test_hex_key_domain_and_direct_record_construction(self):
        for value in ('', 'AA', '0', ' 00', '00 ', 'xx', 1):
            self.assert_rejected(w.exact_hex, value)
        self.assert_rejected(w.exact_hex, '00', 2)
        self.assert_rejected(w.exact_hex, '00' * (w.MAX_RECORD_BYTES + 1))
        self.assertEqual(w.exact_hex('00ff', 2), b'\0\xff')
        for key in ('', '../key', 'two words', 'x' * 129, 'é'):
            self.assert_rejected(w.Plan, key, self.before)
        for key in ('a', 'x' * 128, 'ABC_123.-'):
            self.assertEqual(w.token_for(key, self.plan.digest),
                             sha(b'dfmcp-build-token/1', frame(key.encode()) + self.plan.digest)[:16])
        self.assert_rejected(w.plan_for, self.before.selection, b'x' * 31)
        self.assert_rejected(w.token_for, 'key', b'x' * 33)
        self.assert_rejected(w.Record, self.plan, 'placed', 'none')
        self.assert_rejected(w.Record, self.plan, 'prepared', 'expired')
        self.assert_rejected(w.Record, self.plan, 'indeterminate', 'native_failure',
                             self.before.expected_after(), w.Insertion.decode(INSERTION))


if __name__ == '__main__':
    unittest.main(verbosity=2)
