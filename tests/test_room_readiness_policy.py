"""Durable versions, exact closed flag algebra and original point binding."""
from copy import deepcopy
from dataclasses import replace
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
from room_readiness_policy import ReadinessPolicy, INNER_MAGICS, OUTER_MAGICS


def body():
    return {'intent': {'connect_areas': True, 'access': {
        'point': [10, 10, 2], 'region': {'origin': [10, 10, 2], 'size': [10, 6, 1]}}}}


class ReadinessPolicyTests(unittest.TestCase):
    def test_all_six_envelopes_roundtrip_exactly(self):
        for original, magics in ((False, INNER_MAGICS), (True, OUTER_MAGICS)):
            for version, magic in enumerate(magics, 1):
                policy = ReadinessPolicy.decode(magic, original=original)
                self.assertEqual(policy.version, version)
                self.assertEqual(policy.magic(original=original), magic)
                self.assertEqual(policy.circulation, version >= 2)
                self.assertEqual(policy.project_access, version == 3)

    def test_legacy_policy_does_not_inherit_point_from_new_plan(self):
        for policy in (ReadinessPolicy(), ReadinessPolicy(True)):
            self.assertIsNone(policy.access_point(body()))
        self.assertEqual(ReadinessPolicy(True, True).access_point(body()), (10, 10, 2))

    def test_stronger_scope_has_distinct_sealed_bytes_and_label(self):
        policies = (ReadinessPolicy(), ReadinessPolicy(True), ReadinessPolicy(True, True))
        self.assertEqual(len({p.magic() for p in policies}), 3)
        self.assertEqual(len({p.magic(original=True) for p in policies}), 3)
        self.assertEqual(len({p.name for p in policies}), 3)

    def test_wrong_envelope_family_and_unknown_versions_fail(self):
        for magic in INNER_MAGICS:
            with self.assertRaises(ValueError): ReadinessPolicy.decode(magic, original=True)
        for magic in OUTER_MAGICS:
            with self.assertRaises(ValueError): ReadinessPolicy.decode(magic)
        for magic in (b'', b'DFMRDG00', b'DFMRDG04', b'DFMRDG03x', b'DFMRDG0', 'DFMRDG03', bytearray(b'DFMRDG03')):
            with self.subTest(magic=magic), self.assertRaises(ValueError): ReadinessPolicy.decode(magic)

    def test_project_access_never_enables_circulation_implicitly_in_codec(self):
        with self.assertRaises(ValueError): ReadinessPolicy(False, True)
        for value in (0, 1, 'true', [], None):
            with self.assertRaises(ValueError): ReadinessPolicy(value)
            with self.assertRaises(ValueError): ReadinessPolicy(True, value)
        with self.assertRaises(ValueError): ReadinessPolicy().magic(original=1)
        with self.assertRaises(ValueError): ReadinessPolicy.decode(b'DFMRDG01', original=1)

    def test_access_is_original_exact_immutable_selection(self):
        policy = ReadinessPolicy(True, True)
        value = body(); before = deepcopy(value)
        point = policy.access_point(value)
        self.assertEqual(value, before)
        value['intent']['access']['point'][0] = 20
        self.assertEqual(point, (10, 10, 2))
        self.assertEqual(policy.access_point(value), (20, 10, 2))

    def test_missing_or_nonconnected_original_point_is_not_invented(self):
        cases = ({}, {'intent': {}}, {'intent': {'connect_areas': True}},
                 {'intent': {'connect_areas': False, 'access': body()['intent']['access']}},
                 {'intent': {'connect_areas': 1, 'access': body()['intent']['access']}})
        for value in cases:
            with self.subTest(value=value), self.assertRaises(ValueError):
                ReadinessPolicy(True, True).access_point(value)

    def test_untrusted_point_shapes_and_integer_coercions_fail(self):
        for point in ([True, 10, 2], [10.0, 10, 2], ['10', 10, 2], [10, 10],
                      [0, 10, 2], [32767, 10, 2], (10, 10, 2), None):
            value = body(); value['intent']['access']['point'] = point
            with self.subTest(point=point), self.assertRaises(ValueError):
                ReadinessPolicy(True, True).access_point(value)

    def test_replaced_frozen_policy_rechecks_flag_invariant(self):
        with self.assertRaises(ValueError): replace(ReadinessPolicy(True, True), circulation=False)


if __name__ == '__main__':
    unittest.main()
