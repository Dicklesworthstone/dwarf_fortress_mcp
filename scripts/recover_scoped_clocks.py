#!/usr/bin/env python3
"""Reconcile a reviewed inventory of original construction clocks, query-only.

Use route_furniture_progress.py inventory first and retain its inventory digest.
This command needs only the existing run/1.13 query credentials, not route read
or furniture placement credentials. It never starts, repeats or cancels effects.
Beads: df-dfhack-bridge-plane-c-pic.4/.5.
"""
from __future__ import annotations

import argparse
from pathlib import Path
import sys

from furniture_plan import canonical, integer, require
from route_furniture_review import digest_text
from scoped_clock_recovery import MAX_QUERIES, QueryAuthority, Recovery

MAX_OUTPUT = 256 * 1024


def make_budget(timeout_ms):
    # Reuse one original aggregate budget, before opening any original custody.
    from route_furniture_batch import Budget
    return Budget(timeout_ms)


def open_batch(directory, expected, budget):
    from route_furniture_batch_store import Batch
    return Batch(directory, expected, budget, writable=False)


def state_for(batch, report):
    state = batch.audit()
    require(state['inventory_verified'] is True and report['batch_id'] == batch.id,
            'clock recovery report belongs to another original batch')
    state.update(advance_allowed=False, expected_plan=None, confirm_review=None,
                 execution_review_required=True, clock_recovery=report)
    return state


def packet(state=None, *, failed=False):
    report = None if state is None else state['clock_recovery']
    return {'schema': 'dfmcp.scoped-clock-recovery-result/1', 'ok': not failed,
        'result': state, 'runtime_admitted': False,
        **({'error': {'code': 'SCOPED_CLOCK_RECOVERY_REFUSED', 'detail':
            'Original inventory, custody, query authority, native response or budget refused. '
            'Preserve every original record; never repeat a clock effect.'}} if failed else {}),
        'agent_turn': {
            'schema': 'dfmcp.agent_turn/1', 'operation': 'construction_clock.reconcile',
            'phase': 'reconcile', 'profile': 'forensic', 'session_id': None,
            'turn_id': None, 'request_id': None, 'anchor': None,
            'continuity': {'status': 'indeterminate' if failed else 'stale', 'basis': None, 'gap': None},
            'briefing': {'runtime_admitted': False, 'clock_start_authorized': False,
                'unresolved_clocks': None if report is None else report['counts']['unresolved'],
                'remaining_queryable_originals': None if report is None else report['counts']['query_supported'],
                'attempted_original_queries': 0 if report is None else report['attempted_count'],
                'stop_reason': None if report is None else report['stop_reason']},
            'active_work': [] if report is None else [{
                'kind': 'construction_clock_history', 'unresolved': report['counts']['unresolved'],
                'query_supported': report['counts']['query_supported'], 'retry_permitted': False}],
            'changes': [] if report is None else [{
                'kind': 'retained_clock_evidence', 'clock_record_id': row['clock_record_id'],
                'native_phase': row['native_phase'], 'receipt_digest': row['receipt_digest'],
                'resolved': row['resolved']} for row in report['outcomes'] if row['outcome_retained']],
            'attention': [], 'affordances': [], 'recommendations': [],
            'coverage': {'complete_local_directory': report is not None,
                         'other_batch_details_omitted': True, 'global_clock_history': False},
            'uncertainty': [
                'A retained clock outcome is historical, not current pause, route readiness or construction completion.',
                'Missing, prepared, running, source-lost and unassociated work never grants effect retry.',
                'Counts cover the selected local directory; another directory or controller is outside this guarantee.'],
            'budget': {'maximum_output_bytes': MAX_OUTPUT, 'token_count_measured': False},
            'references': [] if report is None else [report['batch_id'],
                report['initial_inventory_digest'], report['inventory_digest']]}}


def encoded(state=None, *, failed=False):
    raw = canonical(packet(state, failed=failed)) + b'\n'
    require(len(raw) <= MAX_OUTPUT, 'complete original clock recovery output exceeds bound')
    return raw


def publish(raw):
    try:
        if sys.stdout.buffer.write(raw) != len(raw):
            return False
        sys.stdout.buffer.flush()
        return True
    except (OSError, ValueError, KeyboardInterrupt):
        return False


class Parser(argparse.ArgumentParser):
    def error(self, message):
        raise ValueError('invalid scoped clock recovery arguments')


def directory_path(value):
    path = Path(value)
    require(path.is_absolute() and '..' not in path.parts and len(str(path)) <= 4096,
            'absolute original private directory required')
    return path


def main(argv=None):
    batch, history, authority = None, None, None
    output_attempted, result_code = False, 2
    try:
        parser = Parser(description=__doc__)
        parser.add_argument('--directory', required=True)
        parser.add_argument('--batch-id', required=True)
        parser.add_argument('--clock-directory', required=True)
        parser.add_argument('--expected-inventory', required=True)
        parser.add_argument('--max-queries', type=int, default=8)
        parser.add_argument('--timeout-ms', type=int, default=10000)
        args = parser.parse_args(argv)
        directory_path(args.directory)
        clock_directory = directory_path(args.clock_directory)
        digest_text(args.batch_id)
        digest_text(args.expected_inventory)
        integer(args.max_queries, 1, MAX_QUERIES)
        integer(args.timeout_ms, 1, 60000)
        budget = make_budget(args.timeout_ms)
        batch = open_batch(args.directory, args.batch_id, budget)
        history = Recovery(clock_directory, batch).__enter__()
        # Refuse stale independent selection before even loading credentials.
        require(history.digest == args.expected_inventory, 'clock inventory changed; inspect again')
        if history.counts()['query_supported']:
            authority = QueryAuthority.load()
        report = history.reconcile(args.expected_inventory, authority, maximum=args.max_queries,
            render=lambda value: encoded(state_for(batch, value), failed=value['faulted']))
        raw = encoded(state_for(batch, report), failed=report['faulted'])
        history.check()
        if authority is not None:
            authority.clock_guard()
        batch.check()
        budget.remaining()
        output_attempted = True
        result_code = 0 if publish(raw) and not report['faulted'] else 2
    except (OSError, ValueError, TypeError, KeyError, RecursionError, KeyboardInterrupt):
        if not output_attempted:
            output_attempted = True
            publish(encoded(failed=True))
        result_code = 2
    finally:
        # Both original owners remain held until the one output attempt ends.
        # Late close failures cannot emit another packet or repeat any query.
        for resource in (history, batch):
            if resource is not None:
                try:
                    resource.close()
                except (OSError, ValueError):
                    result_code = 2
    return result_code


if __name__ == '__main__':
    raise SystemExit(main())
