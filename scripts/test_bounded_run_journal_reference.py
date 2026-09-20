#!/usr/bin/env python3
"""Independent run-journal format/state model; DOES NOT execute Rust or filesystem custody."""
from pathlib import Path
import hashlib
import itertools
import struct
import unittest

ROOT = Path(__file__).resolve().parents[1]


def h(domain, value):
    return hashlib.sha256(domain + b'\0' + value).digest()


def record(state, plan, native=b''):
    return bytes([state]) + struct.pack('>H', len(plan)) + plan + struct.pack('>H', len(native)) + native


def frame(serial, previous, body):
    prefix = b'DFMRF001' + struct.pack('>IQ', len(body), serial) + previous + body
    return prefix + h(b'dfmcp-run-frame/1', prefix) + b'DFMREND1'


def chain(raw, head):
    result, number = [], 1
    while raw:
        if len(raw) < 92 or raw[:8] != b'DFMRF001':
            raise ValueError('frame')
        n, serial = struct.unpack('>IQ', raw[8:20])
        if n > 452 or serial != number or raw[20:52] != head or len(raw) < 92+n:
            raise ValueError('bound, gap, fork or incomplete frame')
        body = raw[52:52+n]
        digest = raw[52+n:84+n]
        if raw[84+n:92+n] != b'DFMREND1' or digest != h(b'dfmcp-run-frame/1', raw[:52+n]):
            raise ValueError('checksum/footer')
        result.append(body); head = digest; number += 1; raw = raw[92+n:]
    return result


def legal(old, new):
    if old is None:
        return new == 0
    if old in (4, 6):
        return False
    return {0: False, 1: old in (0, 1), 2: old == 1, 3: True, 4: True,
            5: old not in (0, 1), 6: old in (0, 1)}[new]


class Reference(unittest.TestCase):
    def setUp(self):
        self.native = bytes.fromhex((ROOT / 'crates/dfmcp-adapter/tests/fixtures/bounded_run_stopped_v1_13.hex').read_text())
        self.plan = self.native[8:57]
        self.assertEqual(len(self.plan), 49)
        self.head = h(b'dfmcp-run-journal/1', b'explicit reference header')

    def test_fixture_plan_token_receipt(self):
        n = struct.unpack('>H', self.native[8:10])[0]
        self.assertEqual(n, 4)
        spec = self.native[14:22]; obs = self.native[22:57]
        plan = h(b'dfmcp-bounded-run-plan/1', spec+obs)
        self.assertEqual(self.native[57:89], plan)
        self.assertEqual(self.native[89:105], h(b'dfmcp-bounded-run-token/1', self.native[8:14]+plan)[:16])
        self.assertEqual(self.native[-32:], h(b'dfmcp-bounded-run-receipt/1', self.native[:-32]))

    def test_every_byte_corruption_and_incomplete_prefix(self):
        blob = frame(1, self.head, record(4, self.plan, self.native))
        self.assertEqual(len(chain(blob, self.head)), 1)
        for index in range(len(blob)):
            with self.subTest(corrupt=index):
                bad = bytearray(blob); bad[index] ^= 1
                with self.assertRaises(ValueError): chain(bytes(bad), self.head)
        for width in range(1, len(blob)):
            with self.subTest(prefix=width), self.assertRaises(ValueError): chain(blob[:width], self.head)

    def test_multiple_frames_detect_gap_fork_reorder_and_torn_tail(self):
        first = frame(1, self.head, record(0, self.plan))
        second = frame(2, first[-40:-8], record(2, self.plan))
        third = frame(3, second[-40:-8], record(4, self.plan, self.native))
        self.assertEqual(len(chain(first+second+third,self.head)), 3)
        for bad in [first+third, second+first, first+second+second,
                    first+second+third[:-1], first+frame(2,self.head,record(4,self.plan,self.native))]:
            with self.assertRaises(ValueError): chain(bad,self.head)

    def test_terminal_states_cannot_change(self):
        for old, new in itertools.product((4,6),range(7)):
            self.assertFalse(legal(old,new))

    def test_all_state_pairs(self):
        accepted = {(None,0),(0,1),(0,3),(0,4),(0,6),(1,1),(1,2),(1,3),(1,4),(1,6),
                    (2,3),(2,4),(2,5),(3,3),(3,4),(3,5),(5,3),(5,4),(5,5)}
        for old, new in itertools.product((None,0,1,2,3,4,5,6),range(7)):
            self.assertEqual(legal(old,new),(old,new) in accepted)

    def test_all_short_paths_have_at_most_one_dispatch_marker(self):
        # Explore legal transitions only; a marker may never regain preparation.
        paths = [(None,0)]
        for _ in range(8):
            next_paths = []
            for path in paths:
                for state in range(7):
                    if legal(path[-1],state):
                        extended = (*path,state)
                        self.assertLessEqual(extended.count(2),1)
                        next_paths.append(extended)
            paths = next_paths
        self.assertTrue(paths)

    def test_size_and_cancellation_reserves(self):
        self.assertEqual(1+2+173+2+274,452)
        self.assertEqual(8+4+8+32+32+8,92)
        for other_pending in range(256):
            for state, slots in ((0,2),(1,2),(2,3),(3,3),(5,1),(4,0),(6,0)):
                reserve = other_pending*2+slots
                at_boundary = 4096-1-reserve
                self.assertLessEqual(at_boundary+1+reserve,4096)
                self.assertGreater(at_boundary+2+reserve,4096)
                if state in (2,3):
                    # Once monitoring is refused, cancellation and terminal can fit.
                    self.assertLessEqual(at_boundary+2+other_pending*2+1,4096)
                    self.assertLessEqual(at_boundary+3+other_pending*2,4096)


if __name__ == '__main__':
    unittest.main(verbosity=2)
