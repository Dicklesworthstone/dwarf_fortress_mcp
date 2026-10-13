#!/usr/bin/env python3
"""Durably start/sample/wait/inspect/cancel completion of ALL original furnishings.

Read-only game workflow, after every original route-batch placement is resolved.
The original batch and separate monitor remain locked through one final output.
No selected-receipt subset, game-time control, effect retry or route-safety claim.
Beads: df-dfhack-bridge-plane-c-pic.4/.5; WP-10.
"""
from __future__ import annotations

import argparse
from pathlib import Path
import sys

from build_placement_wire import canonical, exact_hex, require
import construction_plan_rpc as native
import construction_wait as foreground
from route_furniture_completion import Goal, POLICY, from_batch, origin_path, verify_origin
from route_furniture_completion_store import Journal, State, MAX_FILE, MAX_FRAMES

MAX_OUTPUT = 96 * 1024
TIMING = ('interval', 'stable_samples', 'stable_span', 'max_gap', 'max_observations')


def open_batch(path, expected_id, budget):
    # Reuse the actual original private parent and complete child inventory.
    from route_furniture_batch_store import Batch
    return Batch(path, expected_id, budget)


def guard(owner, batch, authority=None):
    if authority is not None:
        authority.guard()
        require(authority.address == owner.state.address, 'original query endpoint differs')
    require(owner.state.address == batch.address, 'monitor endpoint differs from original batch')
    verify_origin(owner.state.goal, batch)
    owner.check()
    owner.budget.remaining()


def acquire(authority, goal: Goal, budget):
    goal.validate(budget.work)
    authority.guard()
    require(budget.calls >= 8 + 2 * len(goal.receipts) and budget.work_steps > 0,
            'complete construction read cannot fit remaining allowance')
    with native.Client(authority, goal.condition, budget) as client:
        require(client.manifests['build'] == goal.build
                and client.manifests['operations'].generation == goal.operations_generation
                and client.manifests['operations'].software == goal.build.software,
                'original construction source changed at handshake')
        sample = client.capture_once()
    require(client.closed, 'construction source must close before publication')
    authority.guard()
    budget.remaining()
    return sample


def packet(operation, state=None, *, stored=False, contacted=False, wait=None, failed=False):
    goal = None if state is None else state.goal
    progress = None if state is None else state.progress
    result = None if state is None else {
        'goal_digest': goal.digest, 'batch_id': goal.batch_id, 'review_digest': goal.review_digest,
        'original_plan': goal.plan.json(), 'original_plan_digest': goal.plan.digest,
        'target_count': len(goal.plan.steps), 'progress': progress.view(),
        'goal': {name: getattr(goal.condition, name) for name in ('deadline', *TIMING)},
        'journal': {'frames': state.frames, 'head': state.tail.hex()},
        'native_contacted_this_call': contacted, 'storage_acknowledged_this_call': stored,
        'original_custody_verified': True, 'placement_effects_discharged': False,
        'current_usability_proven': False, 'current_routes_proven': False, 'wait': wait}
    return {'schema': 'dfmcp.route-batch-completion-result/1', 'ok': not failed,
        'policy': POLICY, 'result': result, 'runtime_admitted': False,
        **({'error': {'code': 'ROUTE_COMPLETION_REFUSED', 'detail':
             'Original batch, monitor, source, authority or budget refused. Preserve all original journals; do not repeat placement.'}}
           if failed else {}),
        'agent_turn': {'schema': 'dfmcp.agent_turn/1', 'operation': 'route_completion.' + operation,
            'phase': 'reconcile' if failed else 'inspect', 'profile': 'forensic',
            'session_id': None, 'turn_id': None, 'request_id': None, 'anchor': None,
            'continuity': {'status': 'indeterminate' if failed else 'stale', 'basis': None, 'gap': None},
            'briefing': {'runtime_admitted': False, 'mutation_admissible': False,
                'sampled_whole_batch_condition': bool(progress and progress.phase == 'satisfied')},
            'active_work': [] if progress is None or progress.terminal else [{
                'kind': 'original_batch_construction', 'goal_digest': goal.digest,
                'state': progress.phase, 'read_outcome_unknown': progress.reading}],
            'changes': [], 'attention': [], 'affordances': [], 'recommendations': [],
            'coverage': {'all_original_steps': goal is not None, 'original_batch_custody': goal is not None,
                'downtime_coverage_proven': False, 'continuous_stability_proven': False},
            'uncertainty': ['Completion is historical sampled construction, not current room usability or routing.',
                'All original conditions must hold together; earlier per-building success is not latched.',
                'This monitor neither discharges original placement effects nor grants a retry.'],
            'budget': {'maximum_output_bytes': MAX_OUTPUT, 'token_count_measured': False},
            'references': [] if goal is None else [goal.batch_id, goal.review_digest, goal.digest]}}


def output(value):
    raw = canonical(value) + b'\n'
    require(len(raw) <= MAX_OUTPUT, 'complete original completion output exceeds bound')
    return raw


def reserve(operation, state, limits=None):
    value = packet(operation, state, stored=True, contacted=True,
                   wait=None if limits is None else foreground.reserve_view(limits))
    value['result']['journal'] = {'frames': MAX_FRAMES, 'head': 'f' * 64, 'bytes': MAX_FILE}
    require(len(output(value)) + 32768 <= MAX_OUTPUT, 'all original assessment rows cannot fit')


def sample_once(owner, batch, authority, render):
    guard(owner, batch, authority)
    require(owner.writable and not owner.state.progress.terminal, 'monitor is terminal or read-only')
    require(owner.budget.calls >= 8 + 2 * len(owner.state.goal.receipts),
            'complete construction read cannot fit remaining calls')
    render(owner.state)
    guard(owner, batch, authority)
    owner.start_read()
    guard(owner, batch, authority)
    sample = acquire(authority, owner.state.goal, owner.budget)
    guard(owner, batch, authority)
    def before_publish(candidate):
        render(candidate)
        guard(owner, batch, authority)
    owner.accept(sample, before_publish)
    guard(owner, batch, authority)


class Parser(argparse.ArgumentParser):
    def error(self, message):
        raise ValueError('invalid original completion arguments')


def write_output(raw):
    try:
        if sys.stdout.buffer.write(raw) != len(raw):
            return False
        sys.stdout.buffer.flush()
        return True
    except (OSError, ValueError, KeyboardInterrupt):
        return False


def main(argv=None):
    operation, owner, batch, authority = 'unknown', None, None, None
    attempted, result_code = False, 2
    try:
        parser = Parser(description=__doc__)
        parser.add_argument('operation', choices=('start', 'sample', 'wait', 'inspect', 'cancel'))
        parser.add_argument('--journal', required=True)
        parser.add_argument('--batch-directory', required=True)
        parser.add_argument('--batch-id', required=True)
        parser.add_argument('--expected-goal')
        parser.add_argument('--deadline-tick', type=int)
        for flag, name in (('--interval-ticks', 'interval'), ('--stable-samples', 'stable_samples'),
                           ('--stable-span-ticks', 'stable_span'), ('--max-gap-ticks', 'max_gap'),
                           ('--max-observations', 'max_observations')):
            parser.add_argument(flag, dest=name, type=int)
        parser.add_argument('--timeout-ms', type=int, default=10000)
        foreground.add_arguments(parser)
        args = parser.parse_args(argv)
        operation = args.operation
        limits = foreground.parse_limits(args)
        origin_path(args.batch_directory)
        exact_hex(args.batch_id, 32)
        require((args.deadline_tick is not None) == (operation == 'start')
                and (args.expected_goal is None) == (operation == 'start'),
                'start fixes a deadline; reopening requires the original expected goal')
        require(operation == 'start' or all(getattr(args, name) is None for name in TIMING),
                'reopening cannot renew original construction policy')
        require(not Path(args.journal).is_relative_to(Path(args.batch_directory)),
                'monitor must be outside the closed original batch directory')
        budget = native.Budget(args.timeout_ms)
        if args.expected_goal is not None:
            exact_hex(args.expected_goal, 32)
        # Pin the operator-selected original before creating any monitor file.
        batch = open_batch(args.batch_directory, args.batch_id, budget)
        if operation == 'start':
            goal = from_batch(batch, args.deadline_tick, **{name: getattr(args, name)
                              for name in TIMING if getattr(args, name) is not None})
            authority = native.Authority.load()
            require(authority.address == batch.address, 'construction read endpoint changed')
            from construction_plan import Progress
            reserve(operation, State(goal, batch.address, Progress(goal.digest)))
            authority.guard()
            verify_origin(goal, batch)
            owner = Journal(args.journal, budget, writable=True, create=(goal, batch.address))
        else:
            owner = Journal(args.journal, budget, writable=operation in ('sample', 'wait', 'cancel'))
            require(owner.state.goal.digest == args.expected_goal, 'selected original goal differs')
        guard(owner, batch)
        stored, contacted, wait = operation == 'start', False, None
        if operation in ('start', 'sample') and not owner.state.progress.terminal:
            authority = authority or native.Authority.load()
            sample_once(owner, batch, authority, lambda state: reserve(operation, state))
            stored, contacted = True, True
        elif operation == 'wait':
            authority = None if owner.state.progress.terminal else native.Authority.load()
            wait_result = foreground.run(owner, authority, acquire,
                lambda state: reserve(operation, state, limits), limits,
                source_guard=lambda: guard(owner, batch))
            wait = wait_result.view()
            stored = contacted = wait_result.samples > 0
        elif operation == 'cancel':
            stored = owner.cancel(lambda state: reserve(operation, state))
        value = packet(operation, owner.state, stored=stored, contacted=contacted, wait=wait)
        value['result']['journal']['bytes'] = owner.length
        raw = output(value)
        guard(owner, batch, authority)
        attempted = True
        result_code = 0 if write_output(raw) else 2
    except (OSError, ValueError, TypeError, KeyError, RecursionError, KeyboardInterrupt):
        if not attempted:
            attempted = True
            write_output(output(packet(operation, failed=True)))
        result_code = 2
    finally:
        # Late close failure never prints another object or repeats acquisition.
        for resource in (owner, batch):
            if resource is not None:
                try:
                    resource.close()
                except (OSError, ValueError):
                    result_code = 2
    return result_code


if __name__ == '__main__':
    raise SystemExit(main())
