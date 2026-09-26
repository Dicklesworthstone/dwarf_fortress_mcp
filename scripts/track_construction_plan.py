#!/usr/bin/env python3
"""Start, sample, inspect or cancel a receipt-linked furnishing-plan monitor.

Each foreground sample verifies every original receipt around one complete
operations capture. A separate private journal retains the entire selection,
fixed policy and shared progress; it grants no game-effect authority.
Beads: df-dfhack-bridge-plane-c-pic.4 / df-dfhack-bridge-plane-c-pic.5.
"""
from __future__ import annotations

import argparse
import hashlib
import json

from build_placement_wire import Rejected, Record, canonical, exact_hex, require
from construction_plan import Goal, Progress, MAX_TARGETS
from construction_plan_rpc import Authority, Budget, acquire
from construction_plan_store import Journal, State, MAX_FILE, MAX_FRAMES
from construction_monitor_store import read_private

MAX_OUTPUT = 65536
MAX_RECEIPT_INPUT = 512 * 1024
RECEIPT_SCHEMA = 'dfmcp.construction-plan-receipts/1'


def unique_pairs(pairs: list[tuple[str, object]]) -> dict:
    result = {}
    for key, value in pairs:
        require(key not in result, 'duplicate receipt bundle JSON key')
        result[key] = value
    return result


def load_receipts(path: str, budget: Budget) -> tuple[bytes, ...]:
    """Import the entire explicit selection, without changing placement custody."""
    raw = read_private(path, MAX_RECEIPT_INPUT, budget)
    document = json.loads(raw, object_pairs_hook=unique_pairs)
    require(type(document) is dict and set(document) == {'schema', 'receipts'}
            and document['schema'] == RECEIPT_SCHEMA, 'invalid receipt bundle schema')
    rows = document['receipts']
    require(type(rows) is list and 1 <= len(rows) <= MAX_TARGETS,
            'receipt bundle must contain one complete bounded selection')
    receipts = []
    for row in rows:
        budget.work()
        require(type(row) is dict and set(row) == {'canonical_record_hex'},
                'each receipt must contain one exact canonical_record_hex field')
        record = Record.decode(exact_hex(row['canonical_record_hex'], maximum=6144))
        require(record.phase == 'placed', 'only original Placed records can select construction work')
        receipts.append(record.raw)
    budget.remaining()
    return tuple(receipts)


def packet(operation: str, state: State | None, *, native_contacted: bool = False,
           storage_acknowledged: bool = False, verified: bool = True, error: bool = False) -> dict:
    goal, progress = (None, None) if state is None else (state.goal, state.progress)
    identity = None if goal is None else {
        'goal_digest': goal.digest,
        'target_count': len(goal.receipts),
        'selection': 'complete_original_placement_receipt_set',
    }
    targets = []
    if goal is not None:
        for child in goal.goals:
            record = child.record
            insertion = record.insertion
            targets.append({
                'placement_key': record.plan.key,
                'plan_digest': record.plan.digest.hex(),
                'placement_receipt_sha256': hashlib.sha256(record.raw).hexdigest(),
                'native_receipt_digest': record.raw[-32:].hex(),
                'building_id': insertion.building,
                'construction_job_id': insertion.job,
                'item_id': insertion.item,
                'kind': ('', 'bed', 'chair', 'table')[insertion.kind],
                'position': list(insertion.pos),
                'original_placement_tick': record.plan.before.tick,
            })
    pending = progress is not None and not progress.terminal
    result = {
        'identity': identity, 'targets': targets,
        'progress': progress.view() if verified and progress is not None else None,
        'native_contacted': None if error and native_contacted else native_contacted,
        'native_connection_attempted': native_contacted,
        'storage_acknowledged_this_call': storage_acknowledged,
        'custody_verified': verified,
        'retry_placement_permitted': False,
        'placement_effects_discharged': False,
        'current_usability_proven': False,
    }
    if goal is not None:
        result['goal'] = {
            'deadline_tick': goal.deadline, 'poll_interval_ticks': goal.interval,
            'stable_samples': goal.stable_samples, 'stable_span_ticks': goal.stable_span,
            'max_sample_gap_ticks': goal.max_gap, 'max_observations': goal.max_observations,
        }
    active = []
    if identity is not None and (pending or not verified):
        active = [{**identity, 'kind': 'construction_plan_monitor',
                   'state': progress.phase if verified else 'unverified',
                   'read_outcome_unknown': progress.reading if verified else True}]
    references = [] if identity is None else [identity]
    if progress is not None and verified and progress.last_capture is not None:
        references.append({'kind': 'operations_capture', 'profile': 'operations/1.4',
                           'sha256': progress.last_capture, 'game_tick': progress.last_tick,
                           'historical': True, 'canonical_world_anchor': False})
    value = {'ok': not error, 'schema': 'dfmcp.construction-plan-monitor-result/1', 'result': result,
             'agent_turn': {
                 'schema': 'dfmcp.agent_turn/1', 'operation': 'construction_plan.' + operation,
                 'phase': 'reconcile' if error else 'inspect', 'profile': 'forensic',
                 'session_id': None, 'turn_id': None, 'request_id': None, 'anchor': None,
                 'continuity': {'status': 'indeterminate' if error or (progress and progress.reading) else 'stale',
                                'basis': None, 'gap': None, 'reset_reason': None},
                 'briefing': {'runtime_admitted': False, 'mutation_admissible': False,
                              'construction_scope': 'all_original_receipts_same_sample_condition_only'},
                 'changes': [], 'attention': [], 'active_work': active, 'affordances': [],
                 'recommendations': [],
                 'uncertainty': [
                     'All selected conditions must hold in the same complete sampled capture; prior individual successes are not retained as current facts.',
                     'Samples establish historical endpoint conditions, not continuous monitoring or present usability.',
                     'Original placement custody is unchanged; monitoring never discharges or retries a game effect.',
                 ],
                 'coverage': {'history': 'this_monitor_only', 'custody_verified': verified,
                              'selected_receipt_count': len(targets), 'selection_complete': goal is not None,
                              'downtime_coverage_proven': False, 'original_effect_obligations_loaded': False},
                 'budget': {'output_bytes_limit': MAX_OUTPUT, 'token_count_measured': False},
                 'references': references,
             }}
    if error:
        value['error'] = {
            'code': 'CONSTRUCTION_PLAN_MONITOR_REFUSED',
            'detail': 'Request, source, authority, budget or evidence custody refused. Preserve the monitor and every original placement; do not retry construction.',
        }
    return value


def output(value: dict) -> bytes:
    raw = canonical(value) + b'\n'
    require(len(raw) <= MAX_OUTPUT, 'complete construction-plan response exceeds output allowance')
    return raw


def reserve(operation: str, state: State, *, native_contacted: bool = False,
            storage_acknowledged: bool = False) -> bytes:
    """Reserve the final journal envelope as well as every target's result.

    Decimal bounds are at least as wide as any admitted journal's actual values;
    a real SHA-256 head has exactly the placeholder's length. The final response
    therefore cannot grow beyond this fully rendered reservation.
    """
    value = packet(operation, state, native_contacted=native_contacted,
                   storage_acknowledged=storage_acknowledged)
    value['result']['journal'] = {'frames': MAX_FRAMES, 'bytes': MAX_FILE, 'head': 'f' * 64}
    return output(value)


class Parser(argparse.ArgumentParser):
    def error(self, message: str) -> None:
        # Never echo arbitrary arguments, paths or credentials into a refusal.
        raise Rejected('invalid construction-plan command arguments')


def main(argv: list[str] | None = None) -> int:
    operation, owner, last_state = 'unknown', None, None
    native_contacted = False
    try:
        parser = Parser(description=__doc__)
        parser.add_argument('operation', choices=('start', 'sample', 'inspect', 'cancel'))
        parser.add_argument('--journal', required=True)
        parser.add_argument('--receipts-file')
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
        require((args.receipts_file is not None) == (operation == 'start')
                and (args.deadline_tick is not None) == (operation == 'start'),
                'start requires the original receipt set and a fixed deadline')
        require(operation == 'start' or all(v is None for v in choices),
                'recovery cannot change the selection or renew its policy')
        if operation == 'start':
            receipts = load_receipts(args.receipts_file, budget)
            goal = Goal(receipts, args.deadline_tick,
                        **{name: value for name, value in zip(
                            ('interval', 'stable_samples', 'stable_span', 'max_gap', 'max_observations'), choices)
                           if value is not None})
            authority = Authority.load()
            reserve(operation, State(goal, authority.address, Progress(goal.digest)))
            owner = Journal(args.journal, budget, writable=True, create=(goal, authority.address))
        else:
            owner = Journal(args.journal, budget, writable=operation in ('sample', 'cancel'))
        last_state = owner.state
        stored = operation == 'start'
        if operation in ('start', 'sample') and not owner.state.progress.terminal:
            if operation != 'start':
                authority = Authority.load()
            require(authority.address == owner.state.address, 'operator endpoint differs from original monitor')
            reserve(operation, owner.state, native_contacted=True, storage_acknowledged=True)
            owner.start_read()
            last_state = owner.state
            authority.guard()
            budget.remaining()
            native_contacted = True
            sample = acquire(authority, owner.state.goal, budget)
            owner.accept(sample, lambda candidate: reserve(operation, candidate,
                         native_contacted=True, storage_acknowledged=True))
            stored = True
        elif operation == 'cancel':
            stored = owner.cancel(lambda candidate: reserve(operation, candidate, storage_acknowledged=True))
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
