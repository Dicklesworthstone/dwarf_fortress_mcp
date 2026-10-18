#!/usr/bin/env python3
"""Review/run 1..8 original furnishings through the existing one-step owner.

No native setter, journal format, item allocator or failure retry is added.
Fresh per-step review and advance retain their existing source, custody and
one-shot dispatch checks. Beads: df-dfhack-bridge-plane-c-pic.4/.5.
"""
from __future__ import annotations

import argparse
from contextlib import contextmanager
import sys
import time

import furniture_prefix as policy

RPC_PER_STEP = 19  # Review: 6 bindings + handshake + read; advance: those + query/prepare/commit.
WIRE_PER_STEP = 6 * 1024 * 1024  # Includes maximum bounded notifications in both connections.
MAX_WORK = 8_000_000
REVIEW_FIELDS = frozenset(('before', 'blockers', 'expected_plan', 'confirm_review'))


class Budget:
    """Existing owner budget interface, with one explicitly bounded run allowance."""
    def __init__(self, timeout_ms: int, steps: int, *, clock=time.monotonic):
        policy.integer(timeout_ms, 1, 60000)
        policy.integer(steps, 1, policy.MAX_STEPS)
        self.clock, self.deadline = clock, clock() + timeout_ms / 1000
        self.steps, self.steps_admitted = steps, 0
        self.calls_left, self.bytes_left = steps * RPC_PER_STEP, steps * WIRE_PER_STEP
        self.work_left = MAX_WORK
        self._authority = None

    def remaining(self) -> float:
        left = self.deadline - self.clock()
        policy.require(left > 0 and self.work_left > 0, 'whole furnishing run deadline or work exhausted')
        self.work_left -= 1
        if self._authority is not None:
            # A run requires the SAME currently enabled operator, including at
            # read and output boundaries; a replacement credential cannot inherit it.
            self._authority.guard('CommitPlacement')
        return left

    def bind(self, authority) -> None:
        policy.require(self._authority is None or self._authority == authority, 'run authority cannot be rebound')
        self._authority = authority
        self.remaining()

    def reserve_call(self) -> None:
        self.remaining()
        policy.require(self.calls_left > 0, 'whole furnishing run RPC allowance exhausted')
        self.calls_left -= 1

    def reserve_bytes(self, count: int) -> None:
        self.remaining()
        policy.integer(count, 0, 524288)  # Preserve the native client's per-transfer bound.
        policy.require(count <= self.bytes_left, 'whole furnishing run byte allowance exhausted')
        self.bytes_left -= count

    def admit_step(self) -> None:
        self.remaining()
        policy.require(self.steps_admitted < self.steps and self.calls_left >= RPC_PER_STEP
                       and self.bytes_left >= WIRE_PER_STEP, 'cannot reserve another complete original furnishing')
        self.steps_admitted += 1


def backend():
    # Fixed repository module only; no user-selected import, method or subprocess.
    import furniture_batch
    return furniture_batch


@contextmanager
def checked(api, path: str, batch_id: str, budget: Budget):
    policy.identity(batch_id)
    with api.Batch(path, budget) as batch:
        policy.require(batch.id == batch_id, 'wrong original furnishing batch')
        batch.require_terrain()
        state = batch.audit()
        policy.inventory(state, budget.remaining)
        keys = {step.name: batch.key(step) for step in batch.plan.steps}
        yield batch, state, keys
        batch.check()
        budget.remaining()
    budget.remaining()


def result(state: dict, selected: policy.Prefix, outcomes: list[dict], reason: str, budget: Budget) -> dict:
    return {**state, 'prefix_review': selected.json(),
            'prefix_run': {'schema': 'dfmcp.furniture-prefix-run/1', 'stop_reason': reason,
                'steps_considered': len(outcomes), 'steps_recorded': sum(r['phase'] != 'not_started' for r in outcomes),
                'outcomes': outcomes, 'steps_admitted': budget.steps_admitted,
                'rpc_calls_used': budget.steps * RPC_PER_STEP - budget.calls_left,
                'native_bytes_used': budget.steps * WIRE_PER_STEP - budget.bytes_left,
                'work_checks_used': MAX_WORK - budget.work_left,
                'reviewed_prefix_placed': reason in ('prefix_completed', 'all_placed'),
                'automatic_recovery_or_retry': False, 'production_admitted': False}}


def reserve(api, state: dict, selected: policy.Prefix, budget: Budget) -> None:
    from furniture_output_bounds import inventory_bound, prefix_outcomes_bound, PRESENTATION_MARGIN
    from furniture_plan import Step
    steps = selected.json()['steps']
    original = tuple(Step.from_json({k: v for k, v in step.items() if k != 'key'}) for step in steps)
    bounded = inventory_bound(state, original)
    probe = result(bounded, selected, prefix_outcomes_bound(steps), 'preflight_blocked', budget)
    probe['prefix_run'].update(steps_admitted=budget.steps,
        rpc_calls_used=budget.steps * RPC_PER_STEP, native_bytes_used=budget.steps * WIRE_PER_STEP,
        work_checks_used=MAX_WORK, reviewed_prefix_placed=False)
    # This envelope bounds sizes only and is never returned or retained. Include
    # the real packet renderer and exact original plan/room/terrain metadata.
    raw = api.encoded('run_prefix', probe, False)
    policy.require(len(raw) + PRESENTATION_MARGIN <= api.output_limit(state),
                   'complete original furnishing output cannot fit before native contact')
    budget.remaining()


def review(path: str, batch_id: str, budget: Budget, *, _api=None) -> dict:
    api = backend() if _api is None else _api
    with checked(api, path, batch_id, budget) as (_, state, keys):
        selected = policy.Prefix.create(state, keys, budget.steps, budget.remaining)
        reserve(api, state, selected, budget)
        out = result(state, selected, [], 'reviewed', budget)
        api.encoded('review_prefix', out)
        budget.remaining()
        return out


def run(path: str, batch_id: str, confirm_run: str, budget: Budget, *, _api=None) -> dict:
    api = backend() if _api is None else _api
    policy.identity(confirm_run)
    with checked(api, path, batch_id, budget) as (batch, state, keys):
        selected = policy.Prefix.create(state, keys, budget.steps, budget.remaining)
        selected.confirm(batch_id, confirm_run)
        reserve(api, state, selected, budget)
        authority = api.Authority.load(True)
        policy.require(authority.address == batch.address, 'operator endpoint differs from original batch')
        budget.bind(authority)
    outcomes, reason = [], 'prefix_completed'
    for step in selected.json()['steps']:
        budget.admit_step()
        # These APIs close their own connections and original locks before return.
        # Reopening is for a NEW reviewed step, never recovery of a failed step.
        proposed = api.review(path, batch_id, _budget=budget)
        policy.require(type(proposed) is dict and REVIEW_FIELDS <= set(proposed), 'incomplete native furniture review')
        current = {k: v for k, v in proposed.items() if k not in REVIEW_FIELDS}
        policy.same_inventory(state, current, budget.remaining)
        policy.require(current['next_step'] == step['name'], 'review escaped the confirmed prefix')
        from furniture_output_bounds import MAX_BLOCKER_BYTES
        blockers = proposed['blockers']
        policy.require(type(blockers) is list and len(blockers) <= 32
                       and all(type(b) is str and 1 <= len(b) <= 128 for b in blockers)
                       and len(policy.canonical(blockers)) <= MAX_BLOCKER_BYTES, 'invalid native blocker disclosure')
        if blockers:
            policy.require(proposed['expected_plan'] is None and proposed['confirm_review'] is None,
                           'blocked native review carries execution confirmation')
            outcomes.append({'name': step['name'], 'key': step['key'], 'item': step['item'],
                             'phase': 'not_started', 'dispatch_recorded': False, 'blockers': blockers})
            state, reason = current, 'preflight_blocked'
            break
        policy.identity(proposed['expected_plan'])
        policy.identity(proposed['confirm_review'])
        budget.remaining()
        advanced = api.advance(path, batch_id, proposed['expected_plan'], proposed['confirm_review'], _budget=budget)
        policy.require(type(advanced) is dict and advanced.get('advanced_step') == step['name'],
                       'existing dispatcher advanced another original furnishing')
        after = {k: v for k, v in advanced.items() if k != 'advanced_step'}
        phase = selected.successor(state, after, budget.remaining)
        row = next(r for r in after['steps'] if r['name'] == step['name'])
        policy.require(row['plan_digest'] == proposed['expected_plan'], 'recorded native plan differs from fresh review')
        outcomes.append({'name': step['name'], 'key': step['key'], 'item': step['item'],
                         'phase': phase, 'dispatch_recorded': row['dispatch_recorded'], 'blockers': []})
        state = after
        if phase != 'placed':
            reason = 'indeterminate' if phase in ('unknown', 'indeterminate') else 'native_' + phase
            break
    if reason == 'prefix_completed' and state['status'] == 'all_placed':
        reason = 'all_placed'
    with checked(api, path, batch_id, budget) as (_, final, final_keys):
        policy.same_inventory(state, final, budget.remaining)
        policy.require(final_keys == keys, 'original furnishing key inventory changed')
        out = result(state, selected, outcomes, reason, budget)
        api.encoded('run_prefix', out, reason in ('prefix_completed', 'all_placed'))
        budget.remaining()
        return out


def write_output(raw: bytes) -> int:
    try:
        if sys.stdout.buffer.write(raw) != len(raw):
            return 2
        sys.stdout.buffer.flush()
    except (OSError, ValueError, KeyboardInterrupt):
        return 2
    return 0


class Parser(argparse.ArgumentParser):
    def error(self, message):
        raise ValueError('invalid furniture prefix arguments')


def refusal() -> bytes:
    return policy.canonical({'schema': 'dfmcp.furniture-prefix-error/1', 'ok': False, 'result': None,
        'error': {'code': 'FURNITURE_PREFIX_REFUSED', 'detail':
            'Original custody, confirmation, authority, budget or evidence refused. '
            'Earlier placements may exist. Inspect/query original keys; never repeat uncertain work.'},
        'effect_status': 'unknown', 'retry_permitted': False, 'construction_completion_proven': False,
        'agent_turn': {'schema': 'dfmcp.agent_turn/1', 'operation': 'furniture_batch.prefix',
            'phase': 'reconcile', 'session_id': None, 'request_id': None, 'anchor': None,
            'continuity': {'status': 'indeterminate', 'basis': None, 'gap': None},
            'briefing': {'runtime_admitted': False, 'mutation_admissible': False},
            'changes': [], 'attention': [], 'active_work': [], 'affordances': [], 'recommendations': [],
            'uncertainty': ['No current outcome or complete inventory is established by this refusal.'],
            'coverage': {'inventory_verified': False},
            'budget': {'maximum_output_bytes': 65536, 'token_count_measured': False}, 'references': []}}) + b'\n'


def main(argv=None, *, _api=None) -> int:
    attempted = False
    try:
        parser = Parser(description=__doc__, allow_abbrev=False)
        parser.add_argument('operation', choices=('review', 'run'))
        parser.add_argument('--directory', required=True)
        parser.add_argument('--batch-id', required=True)
        parser.add_argument('--steps', type=int, required=True)
        parser.add_argument('--confirm-run')
        parser.add_argument('--timeout-ms', type=int, default=10000)
        args = parser.parse_args(argv)
        policy.require((args.confirm_run is not None) == (args.operation == 'run'),
                       'only run accepts and requires exact prefix confirmation')
        budget = Budget(args.timeout_ms, args.steps)
        api = backend() if _api is None else _api
        out = (review(args.directory, args.batch_id, budget, _api=api) if args.operation == 'review'
               else run(args.directory, args.batch_id, args.confirm_run, budget, _api=api))
        reason = out['prefix_run']['stop_reason']
        ok = reason in ('reviewed', 'prefix_completed', 'all_placed')
        # Reopen the SAME complete owner through final serialization and the sole
        # output write, including terrain custody. A stale result is not published.
        with checked(api, args.directory, args.batch_id, budget) as (_, final, _):
            baseline = {k: v for k, v in out.items() if k not in ('prefix_review', 'prefix_run')}
            policy.same_inventory(baseline, final, budget.remaining)
            raw = api.encoded(args.operation + '_prefix', out, ok)
            budget.remaining()
            attempted = True
            code = write_output(raw)
        return (0 if ok else 2) if code == 0 else 2
    except (OSError, ValueError, TypeError, KeyError, RecursionError, KeyboardInterrupt):
        if not attempted:
            write_output(refusal())
        return 2  # No second object, retry, query, repair or replacement batch.


if __name__ == '__main__':
    raise SystemExit(main())
