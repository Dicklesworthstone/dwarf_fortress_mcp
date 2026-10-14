#!/usr/bin/env python3
"""Explicitly advance the game between original furniture placements.

This development-only owner composes the existing route-batch read frontier with
the bounded run/1.13 clock effect. One reviewed confirmation authorizes exactly
one bounded clock burst. It never places furniture, retries placement, loops the
clock automatically, or treats construction progress as effect reconciliation.
Beads: df-dfhack-bridge-plane-c-pic.4/.5; df-franken-storage-mvcc-54h.4.
"""
from __future__ import annotations

import argparse
from dataclasses import dataclass, field
import hashlib
import os
from pathlib import Path
import sys

import bounded_run_client as clock
import bounded_run_outcomes as outcomes
from furniture_plan import canonical, integer, require
import route_furniture_batch as route
from route_furniture_review import digest_text
from route_furniture_batch_store import Batch

POLICY = 'explicit-one-bounded-clock-burst-for-construction-progress/1'
SCHEMA = 'dfmcp.route-furniture-progress/1'
MAX_OUTPUT = 256 * 1024
CLOCK_CALL_RESERVE = 32
CLOCK_NETWORK_RESERVE = 1024 * 1024
CLOCK_DISK_RESERVE = 64 * 1024
RUN_ENV = ('DFMCP_ALLOW_UNADMITTED_RUN_V1_13', 'DFMCP_RUN_ENDPOINT', 'DFMCP_RUN_TOKEN')
ENVIRONMENT = tuple(route.ENVIRONMENT) + RUN_ENV


@dataclass(frozen=True)
class ProgressAuthority(route.Authority):
    run_token: bytes = field(repr=False, default=b'')

    @classmethod
    def load(cls):
        require(all(not key.startswith('DFMCP_') or key in ENVIRONMENT for key in os.environ),
                'progress requires its isolated route-plus-run environment')
        require(os.environ.get(route.OPT_IN) == '1', 'route-batch development opt-in required')
        require(os.environ.get('DFMCP_ALLOW_UNADMITTED_RUN_V1_13') == '1',
                'bounded-run development opt-in required')
        # This command never places furniture. A broad placement opt-in in the
        # same process would make the authority boundary ambiguous.
        require(os.environ.get(route.PLACE) in (None, '0'),
                'construction progress refuses furniture placement permission')
        route_tokens = tuple(os.environ.get(key, '').encode('utf-8')
                             for key in (route.native.TOKEN, route.OPS_TOKEN, route.MAP_TOKEN))
        run_token = os.environ.get('DFMCP_RUN_TOKEN', '').encode('utf-8')
        require(all(32 <= len(value) <= 256 and b'\0' not in value
                    for value in (*route_tokens, run_token)), 'invalid progress credentials')
        address = route.native.endpoint(os.environ.get(route.ENDPOINT, '127.0.0.1:5000'))
        run_address = clock.endpoint(os.environ.get('DFMCP_RUN_ENDPOINT', '127.0.0.1:5000'))
        require(address == run_address, 'route and bounded-run endpoints must be identical')
        return cls(address, *route_tokens, False, run_token)

    def guard(self, operation: str = 'QueryPlacement') -> None:
        require(operation in route.native.METHODS, 'unknown route-batch operation')
        require(operation not in ('PreparePlacement', 'CommitPlacement'),
                'progress owner cannot acquire furniture placement authority')
        require(ProgressAuthority.load() == self, 'progress endpoint or credentials changed')

    def clock_guard(self) -> None:
        require(ProgressAuthority.load() == self, 'bounded-run authority changed')


def _frontier(batch, authority):
    state = route.review_step(batch, authority)
    report = state['route_frontier']
    require(report['batch_id'] == batch.id, 'progress review belongs to another batch')
    return state, report


def _scope(batch, state, report, ticks, wall_ms):
    integer(ticks, 1, 1200)
    integer(wall_ms, 1, 60000)
    status = report['status']
    eligible = status == 'construction_pending'
    data = {'schema': SCHEMA, 'policy': POLICY, 'batch_id': batch.id,
            'review_digest': batch.review.digest, 'inventory_digest': state['inventory_digest'],
            'next_step': state['next_step'], 'frontier_evidence': report['evidence_digest'],
            'construction_status': report['construction_prefix']['status'],
            'game_ticks': ticks, 'wall_ms': wall_ms}
    confirmation = hashlib.sha256(
        b'dfmcp-route-furniture-progress-confirmation/1\0' + canonical(data)).hexdigest() if eligible else None
    return data, confirmation


def review_progress(batch, authority, ticks: int, wall_ms: int):
    """Read-only review. A confirmation exists only for construction_pending."""
    state, report = _frontier(batch, authority)
    scope, confirmation = _scope(batch, state, report, ticks, wall_ms)
    state['progress_review'] = {'policy': POLICY, 'scope': scope,
        'confirm_progress': confirmation, 'clock_mutation_performed': False,
        'automatic_loop_permitted': False, 'placement_permitted': False}
    _reserve(state)
    authority.guard()
    batch.check()
    return state


def _record_path(directory: str, confirmation: str) -> Path:
    digest_text(confirmation)
    base = Path(directory)
    require(base.is_absolute() and '..' not in base.parts and len(str(base)) <= 4096,
            'clock record directory must be absolute without parent traversal')
    return base / ('progress-' + confirmation + '.run')


def _reserve(state):
    # The bounded-run intent/outcome is <=16 KiB in practice; retain 32 KiB plus
    # the existing complete route-batch packet before the first clock effect.
    require(len(route.encoded('review', state)) + 32 * 1024 <= MAX_OUTPUT,
            'complete progress result cannot fit before clock dispatch')


def _clock_key(confirmation: str) -> str:
    digest_text(confirmation)
    return 'frp-' + confirmation


def progress(batch, authority: ProgressAuthority, ticks: int, wall_ms: int,
             confirmation: str, clock_directory: str):
    """Execute at most one run/1.13 effect, then re-review if it stopped safely."""
    digest_text(confirmation)
    require(isinstance(authority, ProgressAuthority), 'closed progress authority required')
    require(not batch.writable, 'progress opens route custody read-only')
    authority.clock_guard()
    initial = review_progress(batch, authority, ticks, wall_ms)
    scope = initial['progress_review']['scope']
    actual = initial['progress_review']['confirm_progress']
    require(actual is not None and actual == confirmation,
            'progress confirmation differs from the current construction frontier')
    _reserve(initial)
    batch.budget.reserve('calls', CLOCK_CALL_RESERVE)
    batch.budget.reserve('network_bytes', CLOCK_NETWORK_RESERVE)
    batch.budget.reserve('disk_bytes', CLOCK_DISK_RESERVE)
    remaining_ms = int(batch.budget.remaining() * 1000)
    require(remaining_ms >= wall_ms and remaining_ms >= 1,
            'shared foreground deadline cannot contain the bounded run')
    timeout_ms = min(60000, remaining_ms)
    record = _record_path(clock_directory, confirmation)
    key = _clock_key(confirmation)
    # Fixed path + key means repeating the same reviewed clock intent cannot
    # silently create another effect under another local identity.
    with outcomes.new_run(record):
        authority.clock_guard()
        with clock.Client(authority.address, authority.run_token, timeout_ms) as client:
            result = clock.start(client, authority.address, record, key, ticks, wall_ms)
    retained = outcomes.retain(record, result)
    authority.clock_guard()
    batch.check()
    phase = result.get('record', {}).get('phase')
    verified_stop = phase == 'stopped' and result['record'].get('pause_verified') is True
    after = None
    if verified_stop:
        # The watchdog has re-established pause. Reacquire the complete route /
        # construction frontier instead of interpreting elapsed ticks as success.
        after, _ = _frontier(batch, authority)
    output = batch.audit()
    output['construction_progress'] = {'schema': SCHEMA, 'policy': POLICY, 'scope': scope,
        'clock_key': key, 'clock_record_id': confirmation, 'clock_result': retained,
        'clock_terminal_pause_verified': verified_stop,
        'post_progress_review': None if after is None else {
            'status': after['route_frontier']['status'],
            'frontier_evidence': after['route_frontier']['evidence_digest'],
            'construction_status': after['route_frontier']['construction_prefix']['status'],
            'next_step': after['next_step'],
            'confirm_review': after['confirm_review']},
        'automatic_second_clock_burst': False, 'placement_dispatched': False,
        'placement_retry_permitted': False, 'construction_completion_proven':
            bool(after and after['route_frontier']['status'] == 'ready')}
    require(len(route.encoded('progress', output)) <= MAX_OUTPUT,
            'complete progress result exceeds output budget')
    authority.clock_guard()
    batch.check()
    return output


def query_progress(batch, authority: ProgressAuthority, confirmation: str,
                   clock_directory: str, wait_ms: int = 0):
    """Recover only the original bounded-run key; never prepare or commit again."""
    digest_text(confirmation)
    integer(wait_ms, 0, 60000)
    require(isinstance(authority, ProgressAuthority), 'closed progress authority required')
    path = _record_path(clock_directory, confirmation)
    key = _clock_key(confirmation)
    authority.clock_guard()
    with outcomes.OutcomeStore(path) as store:
        intent = store.intent
        require(intent['idempotency_key'] == key
                and intent['endpoint'] == f'{authority.address[0]}:{authority.address[1]}',
                'clock recovery record belongs to another reviewed progress effect')
        retained = store.load()
        contacted = False
        if retained is None:
            remaining_ms = int(batch.budget.remaining() * 1000)
            require(remaining_ms >= 1, 'shared foreground deadline exhausted')
            batch.budget.reserve('calls', CLOCK_CALL_RESERVE)
            batch.budget.reserve('network_bytes', CLOCK_NETWORK_RESERVE)
            batch.budget.reserve('disk_bytes', CLOCK_DISK_RESERVE)
            with clock.Client(authority.address, authority.run_token,
                              min(60000, remaining_ms)) as client:
                store.check()
                result = clock.recover(client, intent, False, wait_ms)
            retained = store.retain(result)
            contacted = True
    authority.clock_guard()
    batch.check()
    phase = retained.get('record', {}).get('phase')
    verified_stop = phase == 'stopped' and retained.get('record', {}).get('pause_verified') is True
    after = None
    if verified_stop:
        after, _ = _frontier(batch, authority)
    output = batch.audit()
    output['construction_progress'] = {'schema': SCHEMA, 'policy': POLICY,
        'clock_key': key, 'clock_record_id': confirmation, 'clock_result': retained,
        'clock_terminal_pause_verified': verified_stop, 'native_query_contacted': contacted,
        'post_progress_review': None if after is None else {
            'status': after['route_frontier']['status'],
            'frontier_evidence': after['route_frontier']['evidence_digest'],
            'construction_status': after['route_frontier']['construction_prefix']['status'],
            'next_step': after['next_step'], 'confirm_review': after['confirm_review']},
        'automatic_second_clock_burst': False, 'placement_dispatched': False,
        'placement_retry_permitted': False,
        'construction_completion_proven': bool(after and after['route_frontier']['status'] == 'ready')}
    require(len(route.encoded('progress-query', output)) <= MAX_OUTPUT,
            'complete progress recovery result exceeds output budget')
    authority.clock_guard()
    batch.check()
    return output


def packet(operation, state=None, error=False):
    value = route.packet('progress-' + operation, state, error)
    value['schema'] = 'dfmcp.route-furniture-progress-result/1'
    if state is not None and 'construction_progress' in state:
        value['agent_turn']['briefing']['clock_progress_performed'] = True
        value['agent_turn']['briefing']['post_progress_status'] = (
            state['construction_progress']['post_progress_review'] or {}).get('status')
    return value


def encoded(operation, state=None, error=False):
    raw = canonical(packet(operation, state, error)) + b'\n'
    require(len(raw) <= MAX_OUTPUT, 'complete progress packet exceeds output bound')
    return raw


def publish(raw):
    try:
        if sys.stdout.buffer.write(raw) != len(raw):
            return 2
        sys.stdout.buffer.flush()
        return 0
    except (OSError, ValueError, KeyboardInterrupt):
        return 2


class Parser(argparse.ArgumentParser):
    def error(self, message):
        raise ValueError('invalid construction-progress arguments')


def main(argv=None):
    batch = None
    operation = 'unknown'
    output_attempted = False
    try:
        parser = Parser(description=__doc__)
        parser.add_argument('operation', choices=('review', 'progress', 'query'))
        parser.add_argument('--directory', required=True)
        parser.add_argument('--batch-id', required=True)
        parser.add_argument('--ticks', type=int)
        parser.add_argument('--wall-ms', type=int)
        parser.add_argument('--confirm-progress')
        parser.add_argument('--clock-directory')
        parser.add_argument('--wait-ms', type=int, default=0)
        parser.add_argument('--timeout-ms', type=int, default=10000)
        args = parser.parse_args(argv)
        operation = args.operation
        require((args.ticks is not None and args.wall_ms is not None) == (operation in ('review', 'progress')),
                'review/progress require ticks and wall-ms; query recovers the retained intent')
        require((args.confirm_progress is not None) == (operation in ('progress', 'query'))
                and (args.clock_directory is not None) == (operation in ('progress', 'query')),
                'progress/query require confirmation and private clock directory')
        require(operation == 'query' or args.wait_ms == 0,
                'wait-ms belongs only to query')
        budget = route.Budget(args.timeout_ms)
        if operation == 'review':
            authority = route.Authority.load()
        else:
            authority = ProgressAuthority.load()
        batch = Batch(args.directory, args.batch_id, budget, writable=False)
        state = (review_progress(batch, authority, args.ticks, args.wall_ms)
                 if operation == 'review' else
                 progress(batch, authority, args.ticks, args.wall_ms,
                          args.confirm_progress, args.clock_directory)
                 if operation == 'progress' else
                 query_progress(batch, authority, args.confirm_progress,
                                args.clock_directory, args.wait_ms))
        raw = encoded(operation, state)
        authority.guard()
        batch.check()
        budget.remaining()
        output_attempted = True
        return publish(raw)
    except (OSError, ValueError, TypeError, KeyError, RecursionError, KeyboardInterrupt):
        if not output_attempted:
            publish(encoded(operation, None, True))
        return 2
    finally:
        if batch is not None:
            batch.close()


if __name__ == '__main__':
    raise SystemExit(main())
