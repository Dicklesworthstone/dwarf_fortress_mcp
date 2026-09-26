#!/usr/bin/env python3
"""Start, sample, inspect or cancel one receipt-linked furniture monitor.

A separate private journal retains the original goal and complete read evidence.
Each sample is foreground, read-only and one-shot. No polling worker, game-time
advancement, placement retry or construction cancellation is performed.
"""
from __future__ import annotations

import argparse
import hashlib
import json

from build_placement_wire import Rejected, Record, canonical, exact_hex, require
from construction_receipt import Goal, Progress
from construction_monitor_rpc import Authority, Budget, acquire
from construction_monitor_store import Journal, State, read_private

MAX_OUTPUT = 16384


def unique_pairs(pairs: list[tuple[str, object]]) -> dict:
    result = {}
    for key, value in pairs:
        require(key not in result, 'duplicate receipt JSON key')
        result[key] = value
    return result


def load_receipt(path: str, budget: Budget) -> bytes:
    raw = read_private(path, 16384, budget)
    if not raw.startswith(b'DFMBR019'):
        document = json.loads(raw, object_pairs_hook=unique_pairs)
        require(type(document) is dict and set(document) == {'canonical_record_hex'},
                'receipt file must contain one exact canonical_record_hex field')
        raw = exact_hex(document['canonical_record_hex'], maximum=6144)
    record = Record.decode(raw)
    require(record.phase == 'placed', 'construction monitor requires an original Placed record')
    budget.remaining()
    return record.raw


def packet(operation: str, state: State | None, *, native_contacted: bool = False,
           storage_acknowledged: bool = False, verified: bool = True, error: bool = False) -> dict:
    goal, progress = (None, None) if state is None else (state.goal, state.progress)
    record = None if goal is None else goal.record
    pending = progress is not None and not progress.terminal
    identity = None if record is None else {
        'goal_digest': goal.digest, 'placement_key': record.plan.key,
        'plan_digest': record.plan.digest.hex(),
        'placement_receipt_sha256': hashlib.sha256(record.raw).hexdigest(),
        'native_receipt_digest': record.raw[-32:].hex(),
        'building_id': record.insertion.building, 'item_id': record.insertion.item,
        'original_placement_tick': record.plan.before.tick,
    }
    result = {'identity': identity, 'progress': progress.view() if verified and progress else None,
              'native_contacted': None if error and native_contacted else native_contacted,
              'native_connection_attempted': native_contacted, 'storage_acknowledged_this_call': storage_acknowledged,
              'custody_verified': verified, 'retry_placement_permitted': False,
              'placement_effect_discharged': False, 'current_usability_proven': False}
    if goal is not None:
        result['goal'] = {'deadline_tick': goal.deadline, 'poll_interval_ticks': goal.interval,
                          'stable_samples': goal.stable_samples, 'stable_span_ticks': goal.stable_span,
                          'max_sample_gap_ticks': goal.max_gap, 'max_observations': goal.max_observations}
    active = []
    if identity is not None and (pending or not verified):
        active = [{**identity, 'kind': 'construction_monitor',
                   'state': progress.phase if verified else 'unverified',
                   'read_outcome_unknown': progress.reading if verified else True}]
    references = [] if identity is None else [identity]
    if progress is not None and verified and progress.last_capture is not None:
        references.append({'kind': 'operations_capture', 'profile': 'operations/1.4',
                           'sha256': progress.last_capture, 'game_tick': progress.last_tick,
                           'historical': True, 'canonical_world_anchor': False})
    value = {'ok': not error, 'schema': 'dfmcp.construction-monitor-result/1', 'result': result,
             'agent_turn': {'schema': 'dfmcp.agent_turn/1', 'operation': 'construction.' + operation,
                 'phase': 'reconcile' if error else 'inspect', 'profile': 'forensic',
                 'session_id': None, 'turn_id': None, 'request_id': None, 'anchor': None,
                 'continuity': {'status': 'indeterminate' if error or (progress and progress.reading) else 'stale',
                                'basis': None, 'gap': None, 'reset_reason': None},
                 'briefing': {'runtime_admitted': False, 'mutation_admissible': False,
                              'construction_scope': 'original_receipt_linked_sampled_condition_only'},
                 'changes': [], 'attention': [], 'active_work': active, 'affordances': [], 'recommendations': [],
                 'uncertainty': ['Samples establish historical endpoint conditions, not continuous monitoring or present usability.',
                                 'Original placement custody is unchanged; this monitor never discharges or retries a game effect.'],
                 'coverage': {'history': 'this_monitor_only', 'custody_verified': verified,
                              'downtime_coverage_proven': False, 'original_effect_obligations_loaded': False},
                 'budget': {'output_bytes_limit': MAX_OUTPUT, 'token_count_measured': False},
                 'references': references}}
    if error:
        value['error'] = {'code': 'CONSTRUCTION_MONITOR_REFUSED',
            'detail': 'Request, source, authority, budget or evidence custody refused. Preserve the journal and original placement; do not retry construction.'}
    return value


def output(value: dict) -> bytes:
    raw = canonical(value) + b'\n'
    require(len(raw) <= MAX_OUTPUT, 'complete construction response exceeds output allowance')
    return raw


class Parser(argparse.ArgumentParser):
    def error(self, message: str) -> None:
        # Do not reflect arbitrary arguments, paths or credentials into output.
        raise Rejected('invalid construction command arguments')


def main(argv: list[str] | None = None) -> int:
    operation, owner, last_state = 'unknown', None, None
    native_contacted = False
    try:
        parser = Parser(description=__doc__)
        parser.add_argument('operation', choices=('start', 'sample', 'inspect', 'cancel'))
        parser.add_argument('--journal', required=True)
        parser.add_argument('--receipt-file')
        parser.add_argument('--deadline-tick', type=int)
        parser.add_argument('--interval-ticks', type=int)
        parser.add_argument('--stable-samples', type=int)
        parser.add_argument('--stable-span-ticks', type=int)
        parser.add_argument('--max-gap-ticks', type=int)
        parser.add_argument('--max-observations', type=int)
        parser.add_argument('--timeout-ms', type=int, default=10000)
        args = parser.parse_args(argv)
        operation = args.operation
        budget = Budget(args.timeout_ms)
        choices = (args.interval_ticks, args.stable_samples, args.stable_span_ticks,
                   args.max_gap_ticks, args.max_observations)
        require((args.receipt_file is not None) == (operation == 'start')
                and (args.deadline_tick is not None) == (operation == 'start'),
                'start requires an original receipt and fixed deadline')
        require(operation == 'start' or all(v is None for v in choices), 'recovery cannot retarget or renew a goal')
        if operation == 'start':
            receipt = load_receipt(args.receipt_file, budget)
            goal = Goal(receipt, args.deadline_tick,
                        **{name: value for name, value in zip(
                            ('interval', 'stable_samples', 'stable_span', 'max_gap', 'max_observations'), choices)
                           if value is not None})
            authority = Authority.load()
            output(packet(operation, State(goal, authority.address, Progress(goal.digest))))
            owner = Journal(args.journal, budget, writable=True, create=(goal, authority.address))
        else:
            owner = Journal(args.journal, budget, writable=operation in ('sample', 'cancel'))
        last_state = owner.state
        stored = operation == 'start'
        if operation in ('start', 'sample') and not owner.state.progress.terminal:
            if operation != 'start':
                authority = Authority.load()
            require(authority.address == owner.state.address, 'operator endpoint differs from original monitor')
            # Reserve complete compact result before read intent and native I/O.
            output(packet(operation, owner.state, native_contacted=True, storage_acknowledged=True))
            owner.start_read()
            last_state = owner.state
            authority.guard()
            budget.remaining()
            native_contacted = True
            sample = acquire(authority, owner.state.goal, budget)
            owner.accept(sample, lambda candidate: output(packet(operation, candidate,
                         native_contacted=True, storage_acknowledged=True)))
            stored = True
        elif operation == 'cancel':
            stored = owner.cancel(lambda candidate: output(packet(operation, candidate, storage_acknowledged=True)))
        owner.check()
        last_state = owner.state
        value = packet(operation, last_state, native_contacted=native_contacted, storage_acknowledged=stored)
        value['result']['journal'] = {'frames': last_state.frames, 'bytes': owner.length, 'head': last_state.tail.hex()}
        raw = output(value)
        budget.remaining()
        owner.close()
        owner = None
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


if __name__ == '__main__':
    raise SystemExit(main())
