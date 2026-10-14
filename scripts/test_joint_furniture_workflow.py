"""Actual create/check/CLI code with explicit acquisition and packet doubles.

This is not a native TCP, private-parent, Rust or live-game qualification.
"""
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from furniture_plan import canonical
import route_furniture_joint as joint
import route_furniture_review as reviews
from test_joint_furniture_delivery import blocked_choice
from test_furniture_delivery_workflow import workflow, captured, Budget, Authority, Region


class JointWorkflowTests(unittest.TestCase):
    def setUp(self):
        self.runtime, self.cli, self.boundary = workflow()
        self.request, self.items, self.raw, _ = blocked_choice()
        self.region = Region((1, 1, 0), (7, 1, 1))

    def create(self, *, items=None, nodes=4096, authority=None, budget=None):
        supplied = self.items if items is None else items
        with patch.object(self.runtime, 'acquire', return_value=captured(self.request, supplied, self.raw)) as read:
            raw = self.runtime.create(self.request, self.region, authority or Authority(), budget or Budget(),
                sequence_deliveries=True, search_delivery_items=True, max_search_nodes=nodes)
        self.assertEqual(read.call_count, 1)
        return json.loads(raw)

    def test_explicit_search_exports_complete_new_review_and_visible_replacements(self):
        out = self.create(nodes=3)
        self.assertTrue(out['ok'])
        self.assertEqual(out['result']['status'], 'allocated')
        self.assertEqual(out['review']['selections'][1]['candidate']['item'], 3)
        self.assertEqual(out['request'], out['review']['request'])
        self.assertEqual(out['result']['request_digest'], reviews.Request.from_json(out['request']).digest)
        self.assertEqual(out['result']['delivery_search']['item_replacements'],
                         [{'slot': 'b', 'previous_item': 2, 'selected_item': 3}])
        self.assertEqual(out['agent_turn']['coverage']['delivery_item_search_policy'], joint.POLICY)
        messages = ' '.join(out['agent_turn']['uncertainty'])
        self.assertIn('Replacements are listed', messages)
        self.assertNotIn('It preserves selected items', messages)
        self.assertLessEqual(len(canonical(out)), 96 * 1024)
        review = reviews.Review.decode(canonical(out['review']), out['review_digest'])
        self.assertEqual(review.unpack()[3].json(), out['result']['plan'])

    def test_saved_review_check_never_reassigns_its_now_missing_original_item(self):
        created = self.create()
        review = reviews.Review.decode(canonical(created['review']), created['review_digest'])
        before = review.raw
        with patch.object(self.runtime, 'acquire', return_value=captured(self.request, self.items[:2], self.raw)), \
             patch.object(joint, 'allocate_joint_routes', side_effect=AssertionError('must not reassign')) as search:
            out = json.loads(self.runtime.check(review, Authority(), Budget()))
        search.assert_not_called()
        self.assertEqual(out['result']['status'], 'unestablished')
        self.assertIsNone(out['result']['plan'])
        self.assertIsNone(out['review'])
        self.assertEqual(review.raw, before)
        self.assertEqual(out['result']['original_plan']['steps'][1]['item'], 3)

    def test_incomplete_and_exhaustive_failure_never_export_a_review(self):
        for out, status, complete in ((self.create(nodes=2), 'delivery_search_exhausted', False),
                                     (self.create(items=self.items[:2]), 'delivery_assignment_blocked', True)):
            self.assertTrue(out['ok'])
            self.assertEqual(out['result']['status'], status)
            self.assertEqual(out['result']['delivery_search']['search_complete'], complete)
            self.assertIsNone(out['result']['plan'])
            self.assertIsNone(out['review'])
            self.assertIsNone(out['review_digest'])
            self.assertEqual(out['result']['delivery_sequence']['routes'], [])

    def test_invalid_modes_limits_and_small_budget_fail_before_acquisition(self):
        cases = ({'search_delivery_items': True}, {'sequence_deliveries': True, 'search_delivery_items': 1},
                 {'max_search_nodes': 2}, {'max_search_nodes': True}, {'max_search_nodes': 0},
                 {'max_search_nodes': 4097})
        with patch.object(self.runtime, 'acquire', side_effect=AssertionError('no contact')) as read:
            for options in cases:
                with self.assertRaises(ValueError):
                    self.runtime.create(self.request, self.region, Authority(), Budget(), **options)
            with self.assertRaises(ValueError):
                self.runtime.create(self.request, self.region, Authority(), Budget(work=100_000),
                    sequence_deliveries=True, search_delivery_items=True)
        read.assert_not_called()

    def test_cli_explicit_opt_in_preserves_input_and_forbids_existing_review_search(self):
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / 'request.json'
            original = canonical(self.request.json())
            path.write_bytes(original)
            args = ['create', '--request-file', str(path), '--origin', '1', '1', '0', '--size', '7', '1', '1',
                    '--sequence-deliveries', '--search-delivery-items', '--delivery-search-nodes', '3']
            outputs = []
            with patch.object(self.runtime, 'acquire', return_value=captured(self.request, self.items, self.raw)) as read, \
                 patch.object(self.cli, 'publish', side_effect=lambda raw: outputs.append(json.loads(raw)) or 0):
                code = self.cli.main(args)
            self.assertEqual(code, 0)
            self.assertEqual(len(outputs), 1)
            self.assertEqual(read.call_count, 1)
            self.assertEqual(outputs[0]['result']['delivery_search']['visited_nodes'], 3)
            self.assertEqual(path.read_bytes(), original)
            review_path = Path(d) / 'review.json'
            review_bytes = canonical(outputs[0]['review'])
            review_path.write_bytes(review_bytes)
            check = ['check', '--review-file', str(review_path), '--expected-review', outputs[0]['review_digest']]
            for flags in (['--search-delivery-items'], ['--delivery-search-nodes', '3'],
                          ['--sequence-deliveries', '--search-delivery-items']):
                with patch.object(self.cli, 'read_file', side_effect=AssertionError('no file reads')) as read, \
                     patch.object(self.cli, 'publish', return_value=0):
                    self.assertEqual(self.cli.main(check + flags), 2)
                read.assert_not_called()
            self.assertEqual(review_path.read_bytes(), review_bytes)

    def test_default_outputs_match_pinned_predecessor_fixture_bytes(self):
        expected = {False: 'c58575dc67a59ee3802cde08b287d0a8ece9a368fa3add0735bcd47d6204d089', True: '81f06687a65fd0c3f397de8702c7f752db4b4e116ccca6deeb741bd332c462e6'}
        for flag in (False, True):
            with patch.object(self.runtime, 'acquire', return_value=captured(self.request, self.items, self.raw)):
                out = self.runtime.create(self.request, self.region, Authority(), Budget(), sequence_deliveries=flag)
            self.assertEqual(hashlib.sha256(out).hexdigest(), expected[flag])
            self.assertNotIn('delivery_search', json.loads(out)['result'])

    def test_one_solver_allowance_includes_initial_search_and_forward_verification(self):
        initial = self.create()
        used = initial['result']['work_units']
        success = self.create(budget=Budget(work=100_000 + used))
        self.assertEqual(success['result']['maximum_work'], used)
        self.assertEqual(success['result']['plan'], initial['result']['plan'])
        with self.assertRaisesRegex(ValueError, 'work allowance exhausted'):
            self.create(budget=Budget(work=100_000 + used - 1))

    def test_final_authority_and_output_failure_do_not_publish_or_reacquire(self):
        for failure in ('revoked', 'overflow'):
            authority = Authority()
            encoder = self.boundary.bounded_output
            def render(value):
                if failure == 'overflow':
                    raise ValueError('output allowance')
                raw = encoder(value)
                authority.revoked = True
                return raw
            with patch.object(self.runtime, 'acquire', return_value=captured(self.request, self.items, self.raw)) as read, \
                 patch.object(self.boundary, 'bounded_output', side_effect=render):
                with self.assertRaises(ValueError):
                    self.runtime.create(self.request, self.region, authority, Budget(),
                        sequence_deliveries=True, search_delivery_items=True)
            self.assertEqual(read.call_count, 1)

    def test_cli_native_failure_is_one_sanitized_object_without_retry(self):
        outputs = []
        args = ['create', '--request-file', '/unopened', '--origin', '1', '1', '0', '--size', '7', '1', '1',
                '--sequence-deliveries', '--search-delivery-items']
        with patch.object(self.cli, 'read_file', return_value=canonical(self.request.json())), \
             patch.object(self.runtime, 'acquire', side_effect=ValueError('PRIVATE-CREDENTIAL')) as read, \
             patch.object(self.cli, 'publish', side_effect=lambda raw: outputs.append(raw) or 0):
            self.assertEqual(self.cli.main(args), 2)
        self.assertEqual(read.call_count, 1)
        self.assertEqual(len(outputs), 1)
        self.assertNotIn(b'PRIVATE-CREDENTIAL', outputs[0])
        self.assertFalse(json.loads(outputs[0])['ok'])
        self.assertIsNone(json.loads(outputs[0])['review'])


if __name__ == '__main__':
    unittest.main()
