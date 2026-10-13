#!/usr/bin/env python3
"""Review and place one exact bed/chair/table, or recover its original intent.

Explicitly unadmitted POSIX developer workflow. No automatic placement retry,
deconstruction, item substitution, game checkpoint or completed-building claim.
"""
from __future__ import annotations

import argparse
import re
import sys
from typing import Callable

from build_placement_rpc import Authority, Budget, Client, Reply
from build_placement_store import PlacementDirectory, Journal, software_equal
from build_placement_wire import Plan, Selection, canonical, exact_hex, integer, key_text, require

MAX_OUTPUT = 65536
KINDS = {'bed': 1, 'chair': 2, 'table': 3}


def bounded_output(value: dict) -> bytes:
    out = canonical(value) + b'\n'
    require(len(out) <= MAX_OUTPUT, 'complete furniture output exceeds 64 KiB')
    return out


def capture_reference(plan: Plan) -> dict:
    before = plan.before
    return {'schema': 'dfmcp.build-capture-reference/1', 'profile': 'furniture/1.19',
            'folder': before.folder, 'site': before.site, 'native_generation': before.generation,
            'native_sequence': before.sequence, 'game_tick': before.tick,
            'capture_sha256': before.witness.hex()}


def packet(operation: str, out: dict, plan: Plan | None = None) -> dict:
    pending = out.get('pending', False) or bool(out.get('pending_count', 0))
    work = []
    if pending and plan is not None:
        work.append({'kind': 'placement', 'key': plan.key, 'plan_digest': plan.digest.hex(),
                     'state': out.get('effect_status', 'unknown'), 'retry_permitted': False})
    elif pending:
        work = out.get('pending_work', [])
        key = out.get('key')
        if not work and type(key) is str and re.fullmatch('[A-Za-z0-9_.-]{1,128}', key):
            work = [{'kind': 'placement', 'key': key, 'state': 'unknown', 'retry_permitted': False}]
    historical = operation != 'plan'
    return {'ok': True, 'profile': 'furniture/1.19', 'runtime_admitted': False, 'result': out,
            'agent_turn': {
                'schema': 'dfmcp.agent_turn/1', 'operation': 'build_placement.' + operation,
                'phase': 'reconcile' if pending else 'inspect' if historical else 'propose',
                'session_id': None, 'turn_id': None, 'request_id': None, 'anchor': None,
                'continuity': {'status': 'indeterminate' if pending and plan is None
                               else 'stale' if historical and plan else 'bootstrap',
                               'basis': None, 'gap': None, 'reset_reason': None},
                'profile': 'forensic' if historical else 'tactical',
                'briefing': {'runtime_admitted': False, 'mutation_admissible': False,
                             'development_workflow': True, 'construction_completion_proven': False},
                'changes': [], 'attention': [], 'active_work': work,
                'affordances': [], 'recommendations': [],
                'uncertainty': ['Placement evidence is historical construction-job registration.',
                                'Current terrain, finished buildings and game checkpoint are not established.'],
                'coverage': {'profile': 'furniture/1.19', 'history': 'retained_local_intents_only',
                             'global_controller_fence': False},
                'budget': {'output_bytes_limit': MAX_OUTPUT, 'token_count_measured': False},
                'references': [] if plan is None else [capture_reference(plan)]}}


def plan_view(plan: Plan) -> dict:
    return {'plan_digest': plan.digest.hex(), 'selection': plan.before.selection.view(),
            'before': plan.before.view(), 'game_effect_performed': False,
            'requires_exact_confirmation': True, 'retry_permitted': False,
            'construction_completion_proven': False}


def bind(journal: Journal, reply: Reply) -> None:
    require(software_equal(journal.state.manifest, reply.manifest)
            and reply.manifest.generation >= journal.state.manifest.generation,
            'recovery source or software differs from recorded intent')


def result(journal: Journal, reply: Reply | None, stored: bool) -> dict:
    journal.owner.check()
    out = journal.state.view()
    out.update(native_contacted=reply is not None, storage_acknowledged_this_call=stored,
               construction_completion_proven=False, current_building_proved=False)
    if reply is not None:
        bind(journal, reply)
        out['current_manifest'] = reply.manifest.view()
        out['source_changed_since_intent'] = reply.manifest.generation != journal.state.manifest.generation
        out['effect'] = reply.record.view() if reply.record is not None else None
        out['effect_status'] = reply.record.phase if reply.record else 'unknown_absent_native_record'
        out['native_unresolved'] = reply.unresolved
    bounded_output(packet('inspect', out, journal.state.plan))
    return out


def start(owner: PlacementDirectory, authority: Authority, selection: Selection,
          key: str, expected_plan: str,
          guard: Callable[[Plan, Reply, str], None] | None = None) -> tuple[dict, Plan]:
    # An enclosing batch may add custody/confirmation checks, never bypass any
    # native or journal check. Exceptions fence this call before its next effect.
    expected = exact_hex(expected_plan, 32)
    owner.ready(key)
    authority.guard('CommitPlacement')
    with Client(authority, owner.budget, selection) as client:
        observed = client.observe()
        require(not observed.unresolved, 'native unresolved placement blocks new work')
        plan = Plan(key, observed.capture)
        require(plan.digest == expected, 'confirmed plan differs from the current complete capture')
        # Native record <=6144 bytes; its fixed nine-tile view, exact capture,
        # two 128-byte software versions and packet metadata fit this reservation.
        require(len(bounded_output(packet('plan', plan_view(plan), plan))) + 32768 < MAX_OUTPUT,
                'complete outcome output cannot be reserved')
        owner.check()
        known = client.query(plan)
        require(known.record is None and not known.unresolved, 'native key already retained or placement unresolved')
        if guard is not None:
            guard(plan, known, 'before_intent')
        journal = owner.create(plan, observed.manifest, authority.address)
        owner.check()
        if guard is not None:
            guard(plan, known, 'before_prepare')
        prepared = client.prepare(plan)
        if prepared.record.phase != 'prepared':
            stored = journal.retain(prepared)
            return result(journal, prepared, stored), plan
        require(prepared.replayed is False, 'replayed preparation does not authorize dispatch')
        journal.retain(prepared, prepared=True)
        journal.append('dispatch', {'plan_digest': plan.digest.hex()})
        owner.check()
        if guard is not None:
            guard(plan, known, 'before_commit')
        authority.guard('CommitPlacement')
        owner.budget.remaining()
        reply = client.commit(plan)
        stored = journal.retain(reply)
        return result(journal, reply, stored), plan


def recovery_requires_native(journal: Journal, cancel: bool = False) -> bool:
    """Indeterminate is pending evidence, not a terminal offline result.

    Cancellation never turns an attempted placement into nonapplication. It
    remains an offline no-op for a retained uncertain receipt; query may recover
    exact Placed evidence. Already resolved receipts always stay offline.
    """
    require(type(cancel) is bool, 'invalid recovery operation')
    record = journal.state.terminal
    return record is None or (not cancel and record.phase == 'indeterminate')


def recover(journal: Journal, authority: Authority | None, cancel: bool = False) -> dict:
    journal.owner.check()
    if not recovery_requires_native(journal, cancel):
        return result(journal, None, False)
    require(authority is not None and authority.address == journal.state.address,
            'operator endpoint differs from durable intent')
    with Client(authority, journal.owner.budget, journal.state.plan.before.selection) as client:
        require(software_equal(client.manifest, journal.state.manifest)
                and client.manifest.generation >= journal.state.manifest.generation,
                'recovery handshake source or software mismatch')
        journal.owner.check()
        reply = client.cancel(journal.state.plan) if cancel else client.query(journal.state.plan)
        bind(journal, reply)
        previous = journal.state.terminal
        if previous is not None and previous.phase == 'indeterminate' and reply.record is not None and reply.record.phase == 'placed':
            stored = journal.retain_reconciliation(reply)
        else:
            stored = journal.retain(reply) if reply.record is not None else False
        out = result(journal, reply, stored)
        authority.guard('QueryPlacement')
        journal.owner.budget.remaining()
        journal.owner.check()
        return out


def write_output(raw: bytes) -> int:
    # A failed output attempt must never emit a second object or repeat a query.
    try:
        stream = getattr(sys.stdout, 'buffer', None)
        data = raw if stream is not None else raw.decode('ascii')
        stream = sys.stdout if stream is None else stream
        if stream.write(data) != len(data):
            return 2
        stream.flush()
    except (OSError, ValueError, KeyboardInterrupt):
        return 2
    return 0


def publish(operation: str, out: dict, plan: Plan | None, budget: Budget,
            authority: Authority | None, owner: PlacementDirectory | None = None) -> int:
    raw = bounded_output(packet(operation, out, plan))
    if authority is not None:
        authority.guard('QueryPlacement')
    if owner is not None:
        owner.check()
    budget.remaining()
    return write_output(raw)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', choices=('plan', 'start', 'inventory', 'inspect', 'query', 'cancel'))
    parser.add_argument('--directory')
    parser.add_argument('--key')
    parser.add_argument('--kind', choices=tuple(KINDS))
    parser.add_argument('--item', type=int)
    parser.add_argument('--target', type=int, nargs=3, metavar=('X', 'Y', 'Z'))
    parser.add_argument('--expected-plan')
    parser.add_argument('--timeout-ms', type=int, default=10000)
    parser.add_argument('--limit', type=int, default=32)
    parser.add_argument('--continuation')
    args = parser.parse_args(argv)
    output_attempted = False
    try:
        planning = args.operation in ('plan', 'start')
        require(all((value is not None) == planning for value in (args.kind, args.item, args.target)),
                'plan/start require exact kind, item and target; recovery cannot change them')
        require((args.expected_plan is not None) == (args.operation == 'start'), 'start requires --expected-plan only')
        require((args.directory is not None) == (args.operation != 'plan'), 'stateful commands require --directory')
        require((args.key is not None) == (args.operation in ('start', 'inspect', 'query', 'cancel')),
                '--key belongs to start/inspect/query/cancel')
        require(args.operation == 'inventory' or (args.limit == 32 and args.continuation is None),
                'inventory-only page options')
        integer(args.limit, 1, 64)
        budget, plan, authority = Budget(args.timeout_ms), None, None
        if args.key is not None:
            key_text(args.key)
        if planning:
            selected = Selection(KINDS[args.kind], args.item, *args.target)
        if args.operation == 'plan':
            authority = Authority.load()
            with Client(authority, budget, selected) as client:
                reply = client.observe()
                if reply.capture.eligible and not reply.unresolved:
                    plan = Plan('preview', reply.capture)
                    out = plan_view(plan)
                else:
                    out = {'selection': selected.view(), 'before': reply.capture.view(), 'plan_digest': None,
                           'blockers': list(reply.capture.blockers) + (['native_unresolved'] if reply.unresolved else []),
                           'game_effect_performed': False}
                out.update(manifest=reply.manifest.view(), native_contacted=True)
            return publish(args.operation, out, plan, budget, authority)
        else:
            with PlacementDirectory(args.directory, budget, writable=args.operation in ('start', 'query', 'cancel')) as owner:
                if args.operation == 'inventory':
                    out = owner.inventory(args.limit, args.continuation)
                    out['native_contacted'] = False
                    out['pending_count'] = out['pending']
                    out['pending_work'] = [row for row in out['rows'] if row['pending']]
                elif args.operation == 'start':
                    authority = Authority.load(True)
                    out, plan = start(owner, authority, selected, args.key, args.expected_plan)
                else:
                    journal = owner.get(args.key)
                    plan = journal.state.plan
                    if args.operation == 'inspect':
                        out = result(journal, None, False)
                    else:
                        authority = Authority.load() if recovery_requires_native(journal, args.operation == 'cancel') else None
                        out = recover(journal, authority, args.operation == 'cancel')
                # Keep original directory custody through final serialization and output.
                status = publish(args.operation, out, plan, budget, authority, owner)
                output_attempted = True
                return status
    except (OSError, ValueError, TypeError, KeyError, RecursionError, KeyboardInterrupt) as error:
        # No arbitrary native text, source bytes, credentials or paths in errors.
        failure = packet(args.operation, {'effect_status': 'unknown', 'pending': True,
            'key': args.key if type(args.key) is str and re.fullmatch('[A-Za-z0-9_.-]{1,128}', args.key) else None,
            'retry_permitted': False, 'construction_completion_proven': False})
        failure.update(ok=False, error_class=type(error).__name__,
            detail='Request, custody, source, budget or evidence refused. Inspect retained work; do not retry placement.')
        if not output_attempted:
            write_output(bounded_output(failure))
        return 2


if __name__ == '__main__':
    raise SystemExit(main())
