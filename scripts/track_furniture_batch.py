#!/usr/bin/env python3
"""Monitor completion of every step in one original, fully placed furnishing batch.

The complete requested DAG and its original placement journals are retained in a
separate fixed-goal journal. Every fresh sample uses the existing query-only
whole-plan connection; no placement authority or effect retry is introduced.
Beads: df-dfhack-bridge-plane-c-pic.4 / df-dfhack-bridge-plane-c-pic.5.
"""
from __future__ import annotations

import argparse
from pathlib import Path

from build_placement_wire import Rejected, Record, require
from construction_plan import Goal as Condition, Progress
from construction_plan_rpc import Authority, Budget, acquire
from furniture_batch import Batch
from furniture_completion import Goal, Origin
from furniture_completion_store import Journal, State, MAX_FILE, MAX_FRAMES
import track_construction_plan as selected
import construction_wait as foreground

MAX_OUTPUT = selected.MAX_OUTPUT


def separate_journal(path: str, batch_path: str) -> None:
    """Never add monitoring files to a closed original effect inventory."""
    value, source = Path(path), Path(batch_path)
    require(value.is_absolute() and str(value) == path and '..' not in value.parts,
            'absolute canonical completion journal required')
    require(not value.is_relative_to(source), 'completion journal must be outside the original batch')


def packet(operation: str, state: State | None, *, source_verified: bool = False,
           native_contacted: bool = False, storage_acknowledged: bool = False,
           verified: bool = True, error: bool = False) -> dict:
    """Expose the requested plan and exact step association beside shared progress."""
    value = selected.packet(operation, state, native_contacted=native_contacted,
                            storage_acknowledged=storage_acknowledged, verified=verified, error=error)
    value['schema'] = 'dfmcp.furniture-batch-monitor-result/1'
    result, turn = value['result'], value['agent_turn']
    origin = None if state is None else state.goal.origin
    source_verified = source_verified and verified and not error and origin is not None
    result['origin'] = None if origin is None else {
        'batch_id': origin.batch_id,
        'plan_digest': origin.plan.digest,
        'origin_digest': origin.digest,
        'requested_step_count': len(origin.plan.steps),
        'source_custody_verified': source_verified,
    }
    result['requested_plan'] = None if origin is None else origin.plan.json()
    result['original_placement_history_verified'] = source_verified
    result['complete_original_plan_sampled_condition'] = bool(
        source_verified and state.progress.phase == 'satisfied')
    if origin is not None:
        by_key = {Record.decode(raw).plan.key: step.name
                  for step, raw in zip(origin.plan.ordered, origin.receipts)}
        for target in result['targets']:
            target['plan_step'] = by_key[target['placement_key']]
        result['identity']['selection'] = 'every_step_of_original_furnishing_batch'
        result['identity']['batch_id'] = origin.batch_id
        result['identity']['furniture_plan_digest'] = origin.plan.digest
    turn['operation'] = 'furniture_batch_completion.' + operation
    turn['briefing']['construction_scope'] = 'complete_original_furnishing_plan_same_sample_condition_only'
    turn['coverage'].update({
        'original_effect_obligations_loaded': source_verified,
        'original_batch_custody_verified': source_verified,
        'original_plan_complete': source_verified,
        'selection_complete': origin is not None,
    })
    for work in turn['active_work']:
        work['kind'] = 'furniture_batch_completion_monitor'
    turn['uncertainty'].append(
        'The retained original plan identifies every requested step. Combined completion requires '
        'verified original batch custody in this call; cancellation verifies only the monitor journal.')
    if origin is not None:
        turn['references'].append({'kind': 'original_furnishing_batch', 'batch_id': origin.batch_id,
                                   'plan_digest': origin.plan.digest, 'origin_digest': origin.digest,
                                   'custody_verified_this_call': source_verified})
    if error:
        value['error'] = {
            'code': 'FURNITURE_BATCH_MONITOR_REFUSED',
            'detail': 'Original batch, complete plan, source, authority, budget or monitor custody refused. '
                      'Preserve both original stores; do not retry placement.',
        }
    return value


def output(value: dict) -> bytes:
    return selected.output(value)


def reserve(operation: str, state: State, *, source_verified: bool = False,
            native_contacted: bool = False, storage_acknowledged: bool = False,
            wait_limits: foreground.Limits | None = None) -> bytes:
    value = packet(operation, state, source_verified=source_verified, native_contacted=native_contacted,
                   storage_acknowledged=storage_acknowledged)
    value['result']['journal'] = {'frames': MAX_FRAMES, 'bytes': MAX_FILE, 'head': 'f' * 64}
    if wait_limits is not None:
        value['result']['wait'] = foreground.reserve_view(wait_limits)
    return output(value)


class Parser(argparse.ArgumentParser):
    def error(self, message: str) -> None:
        raise Rejected('invalid furnishing-completion arguments')


def main(argv: list[str] | None = None) -> int:
    operation, owner, batch, last_state = 'unknown', None, None, None
    native_contacted = False
    wait_result, authority = None, None
    try:
        parser = Parser(description=__doc__)
        parser.add_argument('operation', choices=('start', 'sample', 'wait', 'inspect', 'cancel'))
        parser.add_argument('--journal', required=True)
        parser.add_argument('--batch')
        parser.add_argument('--batch-id')
        parser.add_argument('--deadline-tick', type=int)
        parser.add_argument('--interval-ticks', type=int)
        parser.add_argument('--stable-samples', type=int)
        parser.add_argument('--stable-span-ticks', type=int)
        parser.add_argument('--max-gap-ticks', type=int)
        parser.add_argument('--max-observations', type=int)
        parser.add_argument('--timeout-ms', type=int, default=10000)
        foreground.add_arguments(parser)
        args = parser.parse_args(argv)
        operation = args.operation
        wait_limits = foreground.parse_limits(args)
        budget = Budget(args.timeout_ms)
        initial = (args.batch, args.batch_id, args.deadline_tick)
        choices = (args.interval_ticks, args.stable_samples, args.stable_span_ticks,
                   args.max_gap_ticks, args.max_observations)
        require(all(value is not None for value in initial) if operation == 'start'
                else all(value is None for value in initial + choices),
                'start requires original batch and fixed policy; reopening cannot replace them')
        if operation == 'start':
            batch = Batch(args.batch, budget)
            origin = Origin.from_batch(batch, args.batch_id)
            separate_journal(args.journal, origin.batch_path)
            condition = Condition(origin.receipts, args.deadline_tick,
                                  **{name: value for name, value in zip(
                                      ('interval', 'stable_samples', 'stable_span', 'max_gap', 'max_observations'),
                                      choices) if value is not None})
            goal = Goal(origin, condition)
            authority = Authority.load()
            require(authority.address == origin.address, 'query endpoint differs from original furnishing batch')
            reserve(operation, State(goal, origin.address, Progress(goal.digest)), source_verified=True)
            origin.verify_batch(batch)
            owner = Journal(args.journal, budget, writable=True, create=(goal, origin.address))
        else:
            owner = Journal(args.journal, budget, writable=operation in ('sample', 'wait', 'cancel'))
            last_state = owner.state
            if operation != 'cancel':
                separate_journal(args.journal, owner.state.goal.origin.batch_path)
                batch = Batch(owner.state.goal.origin.batch_path, budget)

        if batch is not None:
            owner.bind_batch(batch)

        def source_guard() -> None:
            require(batch is not None, 'original batch custody is unavailable')
            owner.state.goal.origin.verify_batch(batch)
            budget.remaining()

        last_state = owner.state
        stored = operation == 'start'
        if operation != 'cancel':
            source_guard()
        if operation in ('start', 'sample') and not owner.state.progress.terminal:
            if operation != 'start':
                authority = Authority.load()
            require(authority.address == owner.state.address, 'query endpoint differs from original completion goal')
            reserve(operation, owner.state, source_verified=True, native_contacted=True, storage_acknowledged=True)
            source_guard()
            owner.start_read()
            last_state = owner.state
            authority.guard()
            budget.remaining()
            native_contacted = True
            sample = acquire(authority, owner.state.goal.condition, budget)

            def render(candidate: State) -> None:
                reserve(operation, candidate, source_verified=True, native_contacted=True, storage_acknowledged=True)
                authority.guard()
                source_guard()

            owner.accept(sample, render)
            stored = True
        elif operation == 'wait':
            authority = None if owner.state.progress.terminal else Authority.load()

            def acquire_wait(current_authority, original_goal, current_budget):
                nonlocal native_contacted
                native_contacted = True
                # The journal retains the full original batch goal. Only its
                # exact immutable receipt condition enters the shared transport.
                return acquire(current_authority, original_goal.condition, current_budget)

            wait_result = foreground.run(owner, authority, acquire_wait,
                lambda candidate: reserve(operation, candidate, source_verified=True,
                    native_contacted=False, storage_acknowledged=False, wait_limits=wait_limits),
                wait_limits, source_guard=source_guard)
            stored = wait_result.samples > 0
        elif operation == 'cancel':
            stored = owner.cancel(lambda candidate: reserve(operation, candidate, storage_acknowledged=True))
        owner.check()
        if operation != 'cancel':
            source_guard()
        last_state = owner.state
        value = packet(operation, last_state, source_verified=operation != 'cancel',
                       native_contacted=native_contacted, storage_acknowledged=stored)
        value['result']['journal'] = {'frames': last_state.frames, 'bytes': owner.length, 'head': last_state.tail.hex()}
        if wait_result is not None:
            value['result']['wait'] = wait_result.view()
        raw = output(value)
        if operation == 'wait' and authority is not None:
            authority.guard()
        if operation != 'cancel':
            source_guard()
        owner.check()
        budget.remaining()
        owner.close()
        owner = None
        if batch is not None:
            batch.close()
            batch = None
        print(raw.decode('ascii'), end='')
        return 0
    except (OSError, ValueError, TypeError, KeyError, RecursionError, KeyboardInterrupt):
        if owner is not None:
            last_state = owner.state
        value = packet(operation, last_state, native_contacted=native_contacted, verified=False, error=True)
        print(output(value).decode('ascii'), end='')
        return 2
    finally:
        if owner is not None:
            owner.close()
        if batch is not None:
            batch.close()


if __name__ == '__main__':
    raise SystemExit(main())
