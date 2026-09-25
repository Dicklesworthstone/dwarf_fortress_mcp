#!/usr/bin/env python3
"""Plan, start once, inspect, query or cancel an excavation-conditioned native run.

Unadmitted POSIX developer workflow, not Rust/MCP or a production controller.
Existing journals are recovery-only. No automatic unpause retry exists.
"""
from __future__ import annotations

import argparse
import json
import time

from excavation_run_rpc import Authority, Budget, Client, Reply
from excavation_run_store import RunDirectory, Journal, software_equal
from excavation_run_wire import Plan, Region, Spec, Rejected, canonical, exact_hex, integer, key_text, require

MAX_OUTPUT = 65536


def bounded_output(value: dict) -> bytes:
    out = canonical(value) + b'\n'
    require(len(out) <= MAX_OUTPUT, 'complete output exceeds 64 KiB')
    return out


def plan_view(value: Plan) -> dict:
    return {'plan_digest': value.digest.hex(), 'spec': list(value.spec.values()),
            'before': value.before.view(), 'game_effect_performed': False,
            'requires_exact_confirmation': True, 'retry_permitted': False}


def bind(journal: Journal, reply: Reply) -> None:
    require(software_equal(journal.state.manifest, reply.manifest)
            and reply.manifest.generation >= journal.state.manifest.generation,
            'recovery source/software does not match recorded intent')


def result(journal: Journal, reply: Reply | None, stored: bool, queries: int = 0, stopped_by: str | None = None) -> dict:
    journal.owner.check()
    out = journal.state.view()
    out.update(native_contacted=reply is not None, storage_acknowledged_this_call=stored,
               queries=queries, foreground_stop=stopped_by)
    if reply is not None:
        bind(journal, reply)
        out['current_manifest'] = reply.manifest.view()
        out['source_changed_since_intent'] = reply.manifest.generation != journal.state.manifest.generation
        out['effect'] = reply.record.view() if reply.record is not None else None
        out['effect_status'] = reply.record.phase if reply.record else 'unknown_absent_native_record'
    bounded_output({'ok': True, 'profile': 'excavation-run/1.18', 'runtime_admitted': False, 'result': out})
    return out


def start(owner: RunDirectory, authority: Authority, region: Region, spec: Spec, key: str,
          expected_plan: str) -> dict:
    expected = exact_hex(expected_plan, 32)
    owner.ready(key)  # Full pending-work/custody check BEFORE a native connection.
    authority.guard('CommitRun')
    with Client(authority, owner.budget, region) as client:
        observed = client.observe()
        require(not observed.owner_active, 'native clock already owned')
        value = Plan(key, spec, observed.capture)
        require(value.digest == expected, 'confirmed plan differs from fresh capture or selected limits')
        # Reserve enough whole-packet space for the maximum bounded receipt view.
        require(len(canonical(plan_view(value))) + 32768 < MAX_OUTPUT, 'output reserve exhausted')
        owner.check()
        # Refuse a currently retained identity, including Prepared, rather than
        # treating native preparation replay as new dispatch permission.
        existing = client.query(value)
        require(existing.record is None and not existing.owner_active, 'native key is already retained or clock owned')
        journal = owner.create(value, observed.manifest, authority.address)
        owner.check()
        prepared = client.prepare(value)
        if prepared.record.phase != 'prepared':
            require(prepared.record.terminal, 'unexpected nonprepared native state')
            stored = journal.retain(prepared)
            return result(journal, prepared, stored)
        journal.retain(prepared, prepared=True)
        journal.append('dispatch', {'plan_digest': value.digest.hex()})
        owner.check()
        # These checks run after both durable publications and immediately before
        # the transport's own current-authority/one-shot/deadline checks.
        authority.guard('CommitRun')
        owner.budget.remaining()
        reply = client.commit(value)
        stored = journal.retain(reply) if reply.record.terminal else False
        return result(journal, reply, stored)


def recover(journal: Journal, authority: Authority | None, cancel: bool = False,
            wait_ms: int = 0, max_queries: int = 16) -> dict:
    integer(wait_ms, 0, 60000)
    integer(max_queries, 1, 32)
    require(not cancel or wait_ms == 0, 'cancellation is one request, not a polling loop')
    journal.owner.check()
    if journal.state.terminal is not None:
        return result(journal, None, False)
    require(authority is not None and authority.address == journal.state.address,
            'operator endpoint differs from durable intent')
    end = min(journal.owner.budget.deadline, time.monotonic() + wait_ms / 1000)
    with Client(authority, journal.owner.budget, journal.state.plan.before.region) as client:
        require(software_equal(client.manifest, journal.state.manifest)
                and client.manifest.generation >= journal.state.manifest.generation,
                'recovery handshake source/software mismatch')
        queries = 0
        while True:
            journal.owner.check()
            reply = client.cancel(journal.state.plan) if cancel else client.query(journal.state.plan)
            queries += int(not cancel)
            bind(journal, reply)
            if reply.record is not None and reply.record.terminal:
                stored = journal.retain(reply)
                return result(journal, reply, stored, queries, 'terminal')
            if cancel or not wait_ms or reply.record is None or reply.record.phase == 'prepared':
                return result(journal, reply, False, queries, 'single_pass')
            if queries >= max_queries:
                return result(journal, reply, False, queries, 'query_limit')
            # Leave time for a complete final custody check and packet. This is
            # cooperative, not a hard real-time bound on filesystem syscalls.
            if end - time.monotonic() <= 0.15:
                return result(journal, reply, False, queries, 'wait_limit')
            time.sleep(0.1)  # Only QueryRun repeats. Prepare/Commit/Cancel never do.


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', choices=('plan', 'start', 'inventory', 'inspect', 'query', 'cancel'))
    parser.add_argument('--directory')
    parser.add_argument('--key')
    parser.add_argument('--region', type=int, nargs=5, metavar=('X', 'Y', 'Z', 'WIDTH', 'HEIGHT'))
    parser.add_argument('--spec', type=int, nargs=6, metavar=('TICKS', 'WALL_MS', 'SAMPLES', 'STABLE_TICKS', 'INTERVAL', 'MAX_GAP'))
    parser.add_argument('--expected-plan')
    parser.add_argument('--timeout-ms', type=int, default=10000)
    parser.add_argument('--wait-ms', type=int, default=0)
    parser.add_argument('--max-queries', type=int, default=16)
    parser.add_argument('--limit', type=int, default=32)
    parser.add_argument('--continuation')
    args = parser.parse_args(argv)
    try:
        planning = args.operation in ('plan', 'start')
        require((args.region is not None) == planning and (args.spec is not None) == planning,
                'plan/start require --region and --spec; recovery cannot change them')
        require((args.expected_plan is not None) == (args.operation == 'start'), 'start requires --expected-plan only')
        require((args.directory is not None) == (args.operation != 'plan'), 'stateful commands require --directory')
        require((args.key is not None) == (args.operation in ('start', 'inspect', 'query', 'cancel')),
                '--key belongs to start/inspect/query/cancel')
        require(args.operation == 'query' or (args.wait_ms == 0 and args.max_queries == 16), 'query-only wait options')
        require(args.operation == 'inventory' or (args.limit == 32 and args.continuation is None), 'inventory-only page options')
        integer(args.wait_ms, 0, 60000)
        integer(args.max_queries, 1, 32)
        budget = Budget(args.timeout_ms)
        if args.key is not None:
            key_text(args.key)
        if planning:
            region, spec = Region(*args.region), Spec(*args.spec)
        if args.operation == 'plan':
            authority = Authority.load()
            with Client(authority, budget, region) as client:
                reply = client.observe()
                out = plan_view(Plan('preview', spec, reply.capture))
                out['manifest'] = reply.manifest.view()
                out['native_contacted'] = True
        else:
            with RunDirectory(args.directory, budget, writable=args.operation in ('start', 'query', 'cancel')) as owner:
                if args.operation == 'inventory':
                    out = owner.inventory(args.limit, args.continuation)
                    out['native_contacted'] = False
                elif args.operation == 'start':
                    out = start(owner, Authority.load(True), region, spec, args.key, args.expected_plan)
                else:
                    journal = owner.get(args.key)
                    if args.operation == 'inspect':
                        out = result(journal, None, False)
                    else:
                        authority = None if journal.state.terminal is not None else Authority.load()
                        out = recover(journal, authority, args.operation == 'cancel', args.wait_ms, args.max_queries)
        packet = {'ok': True, 'profile': 'excavation-run/1.18', 'runtime_admitted': False, 'result': out}
        print(bounded_output(packet).decode('ascii'), end='')
        return 0
    except (OSError, ValueError, TypeError, KeyError, RecursionError, KeyboardInterrupt) as error:
        # Raw replies, credentials, arbitrary server text and filesystem contents
        # never enter error packets. A failed response does not license retry.
        print(canonical({'ok': False, 'profile': 'excavation-run/1.18', 'runtime_admitted': False,
                         'error_class': type(error).__name__, 'effect_status': 'unknown', 'retry_permitted': False,
                         'detail': str(error) if isinstance(error, Rejected) else 'I/O, interruption or decoding failed; retained work must be recovered'}).decode('ascii'))
        return 2


if __name__ == '__main__':
    raise SystemExit(main())
