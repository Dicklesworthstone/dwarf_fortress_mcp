"""Executor/CLI contract tests; original-owner and native operations are doubles.

No native receipt, private-file or live-fort qualification is inferred here.
The actual new runner and existing furniture DAG policy execute unchanged.
"""
from copy import deepcopy
from dataclasses import dataclass
import hashlib
import io
import json
from pathlib import Path
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
from furniture_plan import FurniturePlan, canonical
import furniture_prefix as policy
import run_furniture_batch as run
from test_furniture_prefix import fixture, advance, resign


class OwnerAPI:
    """Deterministic double for the reviewed furniture_batch public interface."""
    def __init__(self, count=4, placed=0, terrain=False):
        self.state, self.keys = fixture(count, placed, terrain=terrain)
        self.events, self.budgets, self.hooks = [], [], {}
        self.phase = 'placed'
        self.blockers = []
        self.authorized, self.terrain_valid = True, True
        self.credential = 'original-test-credential'
        self.active = 0
        self.maximum = 192*1024 if terrain else 65536
        api = self

        @dataclass(frozen=True)
        class Authority:
            address: tuple = ('127.0.0.1', 5000)
            credential: str = 'original-test-credential'

            @classmethod
            def load(cls, placement=False):
                api.events.append(('authority', placement))
                if not api.authorized:
                    raise ValueError('authority revoked')
                return cls(credential=api.credential)

            def guard(self, operation):
                if not api.authorized or self.credential != api.credential:
                    raise ValueError('authority revoked or replaced')

        self.Authority = Authority

        class Batch:
            def __init__(self, path, budget):
                if api.active:
                    raise ValueError('recursive original-owner lock')
                self.id, self.budget = api.state['batch_id'], budget
                self.address = ('127.0.0.1', 5000)
                self.plan = FurniturePlan.from_json(api.state['plan'])
                api.budgets.append(budget)

            def __enter__(self):
                api.active += 1
                try:
                    self.check()
                except BaseException:
                    api.active -= 1
                    raise
                return self

            def __exit__(self, *_args):
                api.active -= 1

            def require_terrain(self):
                api.events.append(('terrain',))
                if not api.terrain_valid:
                    raise ValueError('original terrain history missing')
                self.budget.remaining()

            def audit(self):
                self.check()
                return deepcopy(api.state)

            def key(self, step):
                return api.keys[step.name]

            def check(self):
                api.hook('owner_check')
                self.budget.remaining()
                if not api.terrain_valid:
                    raise ValueError('original custody lost')

        self.Batch = Batch

    def hook(self, name):
        callback = self.hooks.get(name)
        if callback:
            callback()

    def output_limit(self, state):
        return self.maximum

    def encoded(self, operation, state, ok=True):
        self.hook('encode')
        return canonical({'schema': 'dfmcp.furniture-batch-result/1', 'ok': ok, 'result': state}) + b'\n'

    def review(self, path, batch_id, *, _budget):
        self.budgets.append(_budget)
        if self.active:
            raise ValueError('review while original owner still held')
        _budget.remaining()
        for _ in range(8):
            _budget.reserve_call()
            _budget.reserve_bytes(100)
        self.events.append(('review', self.state['next_step']))
        self.hook('review')
        current = deepcopy(self.state)
        current['native_contacted'] = True
        self.native_plan = f"{100+self.state['placed']:064x}"
        self.native_seal = hashlib.sha256(canonical(self.state)).hexdigest()
        current.update(before={'synthetic': True}, blockers=list(self.blockers),
                       expected_plan=None if self.blockers else self.native_plan,
                       confirm_review=None if self.blockers else self.native_seal)
        self.hook('after_review')
        return current

    def advance(self, path, batch_id, expected_plan, confirmation, *, _budget):
        self.budgets.append(_budget)
        _budget.remaining()
        if self.active:
            raise ValueError('advance while original owner still held')
        if expected_plan != self.native_plan or confirmation != hashlib.sha256(canonical(self.state)).hexdigest():
            raise ValueError('existing native review became stale')
        for _ in range(11):
            _budget.reserve_call()
            _budget.reserve_bytes(100)
        name = self.state['next_step']
        self.events.append(('advance', name))
        self.state = advance(self.state, self.keys, self.phase)
        self.hook('advance')
        out = deepcopy(self.state)
        out['advanced_step'] = name
        return out


class Buffer:
    def __init__(self, callback=None, short=False):
        self.data, self.calls, self.callback, self.short = b'', 0, callback, short
        self.buffer = self

    def write(self, raw):
        self.calls += 1
        self.data += raw
        if self.callback:
            self.callback()
        return len(raw) - int(self.short)

    def flush(self):
        pass


class RunFurnitureTests(unittest.TestCase):
    def review(self, api, steps):
        return run.review('/original', api.state['batch_id'], run.Budget(60000, steps), _api=api)

    def execute(self, api, steps, *, budget=None, reviewed=None):
        reviewed = reviewed or self.review(api, steps)
        budget = budget or run.Budget(60000, steps)
        return run.run('/original', api.state['batch_id'], reviewed['prefix_review']['confirm_run'], budget, _api=api)

    def test_offline_review_does_not_load_authority_or_contact_native(self):
        api = OwnerAPI()
        api.authorized = False
        result = self.review(api, 3)
        self.assertFalse(any(e[0] in ('authority', 'review', 'advance') for e in api.events))
        self.assertEqual(result['prefix_run']['rpc_calls_used'], 0)
        self.assertEqual(result['prefix_run']['stop_reason'], 'reviewed')

    def test_complete_eight_item_prefix_retains_all_32_original_items(self):
        api = OwnerAPI(32, 17, terrain=True)
        original = deepcopy(api.state['plan'])
        budget = run.Budget(60000, 8)
        result = self.execute(api, 8, budget=budget)
        self.assertEqual(result['plan'], original)
        self.assertEqual(result['placed'], 25)
        self.assertEqual(result['total'], 32)
        self.assertEqual(result['prefix_run']['steps_recorded'], 8)
        self.assertEqual(result['prefix_run']['rpc_calls_used'], 8 * 19)
        self.assertEqual([e for e in api.events if e[0] == 'advance'], [('advance', f's{i:02d}') for i in range(17, 25)])
        self.assertTrue(all(b is budget for b in api.budgets[-18:]))
        self.assertEqual(api.active, 0)

    def test_all_placed_is_historical_not_construction_completion(self):
        api = OwnerAPI(3)
        result = self.execute(api, 3)
        self.assertEqual(result['prefix_run']['stop_reason'], 'all_placed')
        self.assertFalse(result['construction_completion_proven'])
        self.assertFalse(result['atomic'])
        self.assertFalse(result['prefix_run']['production_admitted'])

    def test_preflight_blocker_does_not_dispatch_or_skip(self):
        api = OwnerAPI(4)
        def after_first():
            if api.state['placed'] == 1:
                api.blockers = ['item_busy']
        api.hooks['advance'] = after_first
        result = self.execute(api, 4)
        self.assertEqual(result['placed'], 1)
        self.assertEqual(result['prefix_run']['stop_reason'], 'preflight_blocked')
        self.assertEqual(result['prefix_run']['steps_considered'], 2)
        self.assertEqual(result['prefix_run']['steps_recorded'], 1)
        self.assertEqual(len([e for e in api.events if e[0] == 'advance']), 1)

    def test_unknown_refused_cancelled_and_indeterminate_never_continue(self):
        for phase in ('unknown', 'refused', 'cancelled', 'indeterminate'):
            api = OwnerAPI()
            api.phase = phase
            result = self.execute(api, 4)
            self.assertEqual(result['prefix_run']['steps_recorded'], 1)
            self.assertFalse(result['prefix_run']['reviewed_prefix_placed'])
            self.assertEqual(len([e for e in api.events if e[0] == 'review']), 1)
            with self.assertRaises(ValueError):
                self.review(api, 1)

    def test_lost_second_acknowledgment_never_retries_and_fences_restart(self):
        api = OwnerAPI()
        reviewed = self.review(api, 4)
        def lost():
            if len([e for e in api.events if e[0] == 'advance']) == 1:
                api.phase = 'unknown'
            else:
                raise OSError('lost response after original intent')
        api.hooks['advance'] = lost
        with self.assertRaises(OSError):
            self.execute(api, 4, reviewed=reviewed)
        self.assertEqual([e for e in api.events if e[0] == 'advance'], [('advance', 's00'), ('advance', 's01')])
        before = list(api.events)
        with self.assertRaises(ValueError):
            self.execute(api, 4, reviewed=reviewed)
        self.assertEqual([e for e in api.events[len(before):] if e[0] in ('review', 'advance')], [])

    def test_stop_between_steps_refuses_before_another_effect(self):
        api = OwnerAPI()
        def stop():
            api.state.update(stopped=True, advance_allowed=False)
            resign(api.state)
        api.hooks['advance'] = stop
        with self.assertRaises(ValueError):
            self.execute(api, 4)
        self.assertEqual(len([e for e in api.events if e[0] == 'advance']), 1)

    def test_changed_inventory_during_review_refuses_before_dispatch(self):
        api = OwnerAPI()
        api.hooks['review'] = lambda: api.state.update(inventory_digest='f'*64)
        with self.assertRaises(ValueError):
            self.execute(api, 4)
        self.assertFalse(any(e[0] == 'advance' for e in api.events))

    def test_concurrent_step_after_review_hits_existing_compare_and_swap(self):
        api = OwnerAPI()
        def other_owner():
            api.state = advance(api.state, api.keys)
        api.hooks['after_review'] = other_owner
        with self.assertRaisesRegex(ValueError, 'became stale'):
            self.execute(api, 4)
        self.assertFalse(any(e[0] == 'advance' for e in api.events))

    def test_old_confirmations_never_accept_changed_count_or_progress(self):
        api = OwnerAPI()
        reviewed = self.review(api, 3)
        for count in (1, 2, 4):
            with self.assertRaises(ValueError):
                self.execute(api, count, reviewed=reviewed)
        self.execute(api, 3, reviewed=reviewed)
        with self.assertRaises(ValueError):
            self.execute(api, 3, reviewed=reviewed)
        self.assertEqual(len([e for e in api.events if e[0] == 'advance']), 3)

    def test_fresh_explicit_review_continues_only_original_remaining_items(self):
        api = OwnerAPI(6)
        first = self.execute(api, 3)
        previous = deepcopy(first['steps'][:3])
        final = self.execute(api, 3)
        self.assertEqual(final['placed'], 6)
        self.assertEqual(final['steps'][:3], previous)
        self.assertEqual([e[1] for e in api.events if e[0] == 'advance'], [f's{i:02d}' for i in range(6)])

    def test_authority_revocation_and_replacement_stop_same_run(self):
        for replacement in (False, True):
            api = OwnerAPI()
            def revoke():
                if replacement:
                    api.credential = 'other-credential'
                else:
                    api.authorized = False
            api.hooks['advance'] = revoke
            with self.subTest(replacement=replacement), self.assertRaises(ValueError):
                self.execute(api, 4)
            self.assertEqual(len([e for e in api.events if e[0] == 'advance']), 1)

    def test_original_terrain_custody_loss_cannot_be_replaced_with_current_items(self):
        api = OwnerAPI(4, terrain=True)
        api.hooks['advance'] = lambda: setattr(api, 'terrain_valid', False)
        with self.assertRaises(ValueError):
            self.execute(api, 1)
        self.assertEqual(api.state['placed'], 1)
        self.assertEqual(api.active, 0)

    def test_result_reservation_refuses_before_credentials_or_native(self):
        api = OwnerAPI()
        api.maximum = 1024
        with self.assertRaises(ValueError):
            self.review(api, 4)
        self.assertFalse(any(e[0] in ('authority', 'review', 'advance') for e in api.events))

    def test_call_byte_and_step_admission_is_shared_not_renewed(self):
        for field, value in (('calls_left', 18), ('bytes_left', run.WIRE_PER_STEP - 1), ('steps_admitted', 2)):
            api = OwnerAPI()
            reviewed = self.review(api, 2)
            budget = run.Budget(60000, 2)
            setattr(budget, field, value)
            with self.subTest(field=field), self.assertRaises(ValueError):
                self.execute(api, 2, budget=budget, reviewed=reviewed)
            self.assertFalse(any(e[0] in ('review', 'advance') for e in api.events))

    def test_midrun_allowance_loss_does_not_start_a_new_step(self):
        api = OwnerAPI()
        budget = run.Budget(60000, 4)
        api.hooks['advance'] = lambda: setattr(budget, 'calls_left', 18)
        with self.assertRaises(ValueError):
            self.execute(api, 4, budget=budget)
        self.assertEqual(len([e for e in api.events if e[0] == 'review']), 1)

    def test_deadline_and_work_exhaustion_propagate(self):
        for what in ('deadline', 'work'):
            api = OwnerAPI()
            now = [0.0]
            budget = run.Budget(100, 4, clock=lambda: now[0])
            def expire():
                if what == 'deadline':
                    now[0] = 1.0
                else:
                    budget.work_left = 0
            api.hooks['after_review'] = expire
            with self.assertRaises(ValueError):
                self.execute(api, 4, budget=budget)
            self.assertFalse(any(e[0] == 'advance' for e in api.events))

    def test_independent_limits_are_exact_and_cannot_go_negative(self):
        for args in ((True, 1), (1, True), (0, 1), (60001, 1), (1, 0), (1, 9)):
            with self.assertRaises(ValueError):
                run.Budget(*args)
        budget = run.Budget(10000, 1)
        budget.calls_left = 0
        with self.assertRaises(ValueError):
            budget.reserve_call()
        budget.bytes_left = 1
        for amount in (2, -1, True, 524289):
            with self.assertRaises(ValueError):
                budget.reserve_bytes(amount)
        self.assertEqual(budget.bytes_left, 1)

    def test_publication_rechecks_original_owner_and_does_not_forge_partial_success(self):
        api = OwnerAPI()
        reviewed = self.review(api, 1)
        def corrupt():
            api.state['source']['generation'] += 1
            resign(api.state)
        api.hooks['advance'] = corrupt
        output = Buffer()
        with patch.object(sys, 'stdout', output):
            code = run.main(['run', '--directory', '/original', '--batch-id', api.state['batch_id'],
                             '--steps', '1', '--confirm-run', reviewed['prefix_review']['confirm_run']], _api=api)
        self.assertEqual(code, 2)
        self.assertEqual(output.calls, 1)
        self.assertIsNone(json.loads(output.data)['result'])

    def test_cli_success_and_blocker_statuses(self):
        for blocked in (False, True):
            api = OwnerAPI(2)
            reviewed = self.review(api, 2)
            if blocked:
                api.blockers = ['unsafe_target']
            output = Buffer()
            with patch.object(sys, 'stdout', output):
                code = run.main(['run', '--directory', '/original', '--batch-id', api.state['batch_id'],
                    '--steps', '2', '--confirm-run', reviewed['prefix_review']['confirm_run']], _api=api)
            self.assertEqual(code, 2 if blocked else 0)
            self.assertEqual(json.loads(output.data)['ok'], not blocked)
            self.assertEqual(output.calls, 1)
            self.assertNotIn(api.credential.encode(), output.data)

    def test_cli_final_output_holds_original_custody(self):
        api = OwnerAPI(2, terrain=True)
        reviewed = self.review(api, 2)
        held = []
        output = Buffer(lambda: held.append(api.active))
        with patch.object(sys, 'stdout', output):
            code = run.main(['run', '--directory', '/original', '--batch-id', api.state['batch_id'],
                '--steps', '2', '--confirm-run', reviewed['prefix_review']['confirm_run']], _api=api)
        self.assertEqual(code, 0)
        self.assertEqual(held, [1])
        self.assertEqual(api.active, 0)

    def test_short_write_or_postwrite_revocation_never_outputs_twice(self):
        for short in (False, True):
            api = OwnerAPI(1)
            reviewed = self.review(api, 1)
            output = Buffer(lambda: setattr(api, 'authorized', False), short=short)
            with patch.object(sys, 'stdout', output):
                code = run.main(['run', '--directory', '/original', '--batch-id', api.state['batch_id'],
                    '--steps', '1', '--confirm-run', reviewed['prefix_review']['confirm_run']], _api=api)
            self.assertEqual(code, 2)
            self.assertEqual(output.calls, 1)
            self.assertEqual(len([e for e in api.events if e[0] == 'advance']), 1)

    def test_final_serialization_revocation_prevents_success_output(self):
        api = OwnerAPI(1)
        reviewed = self.review(api, 1)
        def render():
            if api.state['placed']:
                api.authorized = False
        api.hooks['encode'] = render
        output = Buffer()
        with patch.object(sys, 'stdout', output):
            code = run.main(['run', '--directory', '/original', '--batch-id', api.state['batch_id'],
                '--steps', '1', '--confirm-run', reviewed['prefix_review']['confirm_run']], _api=api)
        self.assertEqual(code, 2)
        self.assertIsNone(json.loads(output.data)['result'])
        self.assertEqual(output.calls, 1)

    def test_cli_invalid_arguments_do_not_import_backend_or_emit_two_objects(self):
        with patch.object(run, 'backend', side_effect=AssertionError('backend should not load')):
            for arguments in ([], ['run'], ['review', '--directory', '/original', '--batch-id', 'a'*64,
                               '--steps', '0'], ['review', '--directory', '/original', '--batch-id', 'a'*64,
                               '--steps', '1', '--confirm-run', 'b'*64]):
                output = Buffer()
                with patch.object(sys, 'stdout', output):
                    self.assertEqual(run.main(arguments), 2)
                self.assertEqual(output.calls, 1)
                self.assertEqual(json.loads(output.data)['error']['code'], 'FURNITURE_PREFIX_REFUSED')


if __name__ == '__main__':
    unittest.main()
