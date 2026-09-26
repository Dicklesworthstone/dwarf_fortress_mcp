#!/usr/bin/env python3
"""Link verified dig/1.16 batches to foreground map/1.5 blueprint monitoring.

No designation, unpause, retry, background work or cross-plugin identity claim.
The two evidence families stay independent: selectors associate them, not causes.
"""
from __future__ import annotations

import argparse
from contextlib import ExitStack, contextmanager
from dataclasses import replace
import json
import os
from pathlib import Path
import secrets
import struct

import dig_blueprint_client as c
import dig_designation_client as d
import dig_designation_store as s
import excavation_blueprint as b
import excavation_observer as e
import track_excavation as t

LINK = 'designation-link.json'
JOURNAL = 'progress.jsonl'
FORMAT = 'dfmcp.dig-blueprint-monitor-link/1'
MAX_LINK = 32768
MAX_OUTPUT = t.MAX_OUTPUT
LINK_FIELDS = {'format', 'batch_path', 'batch_id', 'batch_directory_identity',
               'batch_manifest_sha256', 'receipt_set_digest', 'blueprint_digest',
               'journal_id', 'journal_identity'}


class Budget(c.Budget):
    """One absolute allowance crosses both existing file/codec implementations."""
    def remaining_ms(self):
        return self.milliseconds()


def receipt_digest(state):
    d.require(state['phase'] == 'designations_verified'
              and state['designated_steps'] == state['total_steps'],
              'every original designation needs a verified retained terminal receipt')
    # A later local stop may change workflow state, never the immutable proof set.
    return d.digest(b'dfmcp-dig-blueprint-receipt-set/1', d.canonical({
        'batch_id': state['batch_id'], 'layout_digest': state['layout_digest'],
        'blueprint_digest': state['blueprint_digest'], 'records': state['records']})).hex()


def validate_link(value):
    d.require(type(value) is dict and set(value) == LINK_FIELDS and value['format'] == FORMAT,
              'invalid designation-to-monitor binding')
    for field in ('batch_id', 'batch_manifest_sha256', 'receipt_set_digest',
                  'blueprint_digest', 'journal_id'):
        d.exact_hex(value[field], 32)
    raw = value['batch_path']
    d.require(type(raw) is str and 1 <= len(raw) <= 4096 and '\0' not in raw
              and Path(raw).is_absolute() and '..' not in Path(raw).parts,
              'invalid retained batch path')
    for field in ('batch_directory_identity', 'journal_identity'):
        d.require(type(value[field]) is list and len(value[field]) == 2, 'invalid retained file identity')
        for number in value[field]:
            d.integer(number, 0, 2**64 - 1)
    return value


def first_capture_matches(batch, state, capture):
    """Scope association only. Plugin generations deliberately are NOT equated."""
    expected = batch.bootstrap
    versions = batch.value['manifest']
    d.require(capture.folder == expected['folder'] and capture.site == expected['site']
              and list(capture.dimensions) == expected['dimensions']
              and capture.manifest.df_version == versions['df_version']
              and capture.manifest.dfhack_version == versions['dfhack_version']
              and capture.tick >= state['minimum_tick'],
              'initial map scope/software/clock differs from retained designation evidence')


def binding_for(batch, state, journal):
    h = journal.history
    d.require(h.profile is t.BLUEPRINT_PROFILE
              and h.goal.blueprint.digest == batch.layout.blueprint_digest
              and h.endpoint == batch.value['endpoint'], 'monitor changed its exact blueprint or endpoint')
    first_capture_matches(batch, state, h.progress.first)
    return {'format': FORMAT, 'batch_path': str(batch.directory.root), 'batch_id': batch.id,
            'batch_directory_identity': list(batch.directory.identity),
            'batch_manifest_sha256': c.sha(batch.pin.raw), 'receipt_set_digest': receipt_digest(state),
            'blueprint_digest': batch.layout.blueprint_digest, 'journal_id': h.identity,
            'journal_identity': list(journal.identity)}


class Session:
    def __init__(self, directory, link, pin, journal, batch=None, store=None):
        self.directory, self.link, self.pin, self.journal = directory, link, pin, journal
        self.batch, self.store = batch, store
        self.state = None

    def local(self):
        self.directory.verify(); self.pin.verify(); self.journal.verify()
        d.require(set(self.directory.names()) == {LINK, JOURNAL}, 'incomplete or changed monitor directory')
        h = self.journal.history
        d.require(self.journal.parent_identity == self.directory.identity
                  and list(self.journal.identity) == self.link['journal_identity']
                  and h.identity == self.link['journal_id'] and h.profile is t.BLUEPRINT_PROFILE
                  and h.goal.blueprint.digest == self.link['blueprint_digest'],
                  'monitor journal was substituted or rebound')

    def current(self):
        self.local()
        if self.batch is not None:
            state = self.batch.inventory(self.store)
            d.require(binding_for(self.batch, state, self.journal) == self.link,
                      'original batch identity or designation evidence changed')
            self.state = state
        self.local()

    def packet(self, operation, sampled=False, attempted=0, failure=None):
        self.current()
        verified = self.state is not None
        result = t.report(self.journal.history, sampled, attempted)
        result.update(schema='dfmcp.dig-blueprint-progress/1',
            designation_evidence={'batch_id': self.link['batch_id'],
                'receipt_set_digest': self.link['receipt_set_digest'],
                'verified_this_call': verified,
                'designated_steps': self.state['designated_steps'] if verified else None,
                'total_steps': self.state['total_steps'] if verified else None},
            association={'basis': 'same_endpoint_fortress_selectors_dimensions_and_software',
                'dig_generation': self.batch.value['manifest']['generation'] if verified else None,
                'map_generation': self.journal.history.progress.first.manifest.generation,
                'shared_process_identity_proven': False, 'mining_causality_proven': False},
            historical_designations_verified=verified,
            designation_and_sampled_goal_evidence_verified=verified and result['blueprint_goal_satisfied_at_sample'])
        if failure is not None:
            result.update(ok=False, error=failure)
        packet = json.loads(t.encode_result(result, operation))
        turn = packet['agent_turn']
        turn['operation'] = 'dig_blueprint_progress.' + operation
        turn['active_work']['scope'] = 'linked_blueprint_goal_and_original_designation_batch'
        turn['active_work']['native_effect_inventory_verified'] = verified
        turn['briefing']['historical_designations_verified'] = verified
        d.require(len(d.canonical(packet)) <= MAX_OUTPUT, 'complete linked progress response exceeds 32 KiB')
        self.local()
        return packet


@contextmanager
def open_monitor(root, budget, *, writable=False, source=True):
    with ExitStack() as stack:
        directory = stack.enter_context(s.open_store(d, root, budget=budget.check))
        d.require(set(directory.names()) == {LINK, JOURNAL}, 'monitor initialization incomplete; no repair')
        link, pin = stack.enter_context(c.private_file(directory, LINK, MAX_LINK))
        validate_link(link)
        journal = stack.enter_context(t.open_journal(root / JOURNAL, budget, writable=writable))
        session = Session(directory, link, pin, journal)
        session.local()
        if source:
            session.batch = stack.enter_context(c.open_batch(Path(link['batch_path']), budget))
            session.store = stack.enter_context(s.open_store(d, session.batch.effects, budget=budget.check))
        session.current()
        yield session
        session.current()


def start(root, batch_path, batch_id, max_game_ticks, stable_ticks=10, required_samples=2,
          max_gap_ticks=1200, timeout_ms=10000, *, connect=e.MapClient):
    budget = Budget(timeout_ms)
    d.exact_hex(batch_id, 32)
    e.integer(max_game_ticks, 1, 403200)
    with s.open_store(d, root, budget=budget.check) as directory:
        d.require(not directory.names(), 'linked monitor requires an existing empty private directory')
        with c.open_batch(batch_path, budget) as batch:
            d.require(batch.id == batch_id, 'wrong original blueprint batch')
            with s.open_store(d, batch.effects, budget=budget.check) as store:
                state = batch.inventory(store)
                proof = receipt_digest(state)  # Before credentials, native calls or journal creation.
                blueprint = b.Blueprint.from_json(batch.layout.blueprint())
                template = b.BlueprintGoal(blueprint, batch.bootstrap['folder'], batch.bootstrap['site'],
                    e.MAX_TICK, stable_ticks, required_samples, max_gap_ticks)
                d.require(stable_ticks <= min(max_game_ticks, max_gap_ticks * t.MAX_READS)
                          and required_samples - 1 <= min(max_game_ticks, t.MAX_READS),
                          'sample stability cannot fit the fixed horizon and retention allowance')
                credentials = t.environment(batch.value['endpoint'])
                with connect(*credentials, blueprint.region, budget.remaining_ms()) as client:
                    first = client.observe()
                first = e.decode_capture(first.raw, first.manifest, blueprint.region)
                first_capture_matches(batch, state, first)
                d.require(t.environment(batch.value['endpoint']) == credentials, 'map operator selection changed')
                d.require(receipt_digest(batch.inventory(store)) == proof, 'designation history changed during map read')
                directory.verify(); budget.check()
                goal = replace(template, deadline_tick=first.tick + max_game_ticks)
                event = {'kind': 'begin', 'format': t.BLUEPRINT_PROFILE.format, 'nonce': secrets.token_hex(32),
                         'endpoint': credentials[0], 'goal': goal.json(), 'sample': t.sample_value(first)}
                with t.open_journal(root / JOURNAL, budget, writable=True, create=True) as journal:
                    journal.append(event)
                    link = binding_for(batch, state, journal)
                    with c.private_file(directory, LINK, MAX_LINK, link) as (_, pin):
                        return Session(directory, link, pin, journal, batch, store).packet('start', True, 1)


def sample(root, timeout_ms=10000, *, connect=e.MapClient):
    budget = Budget(timeout_ms)
    with open_monitor(root, budget, writable=True) as session:
        journal = session.journal
        h = journal.history
        if h.progress.status in e.TERMINAL:
            return session.packet('sample')  # No credentials/native access for terminal history.
        e.require(h.attempts < t.MAX_READS and h.events + 3 <= t.MAX_EVENTS
                  and len(journal.raw) + 3 * h.profile.max_frame <= h.profile.max_journal,
                  'goal retention exhausted; inspect or cancel without eviction')
        credentials = t.environment(h.endpoint)
        journal.sync()
        session.pin.verify(); os.fsync(session.pin.fd); os.fsync(session.directory.parent)
        session.current()
        journal.append({'kind': 'read_started'})
        try:
            d.require(t.environment(h.endpoint) == credentials, 'map operator selection changed')
            with connect(*credentials, h.goal.region, budget.remaining_ms()) as client:
                observed = client.observe()
            # A valid source-change sample is recorded, then invalidates the goal
            # through the existing evaluator; it is not discarded as a transient error.
            event = {'kind': 'sample', 'sample': t.sample_value(observed)}
        except (e.Rejected, OSError, ValueError, KeyError, TypeError, struct.error):
            session.current()  # Local custody failure must never become a native-read outcome.
            journal.append({'kind': 'read_failed'})
            return session.packet('sample', attempted=1, failure='map_read_failed; stable streak reset; no effect retried')
        session.current()
        d.require(t.environment(h.endpoint) == credentials, 'map operator selection changed during read')
        journal.append(event)
        return session.packet('sample', True, 1)


def inspect(root, timeout_ms=10000):
    with open_monitor(root, Budget(timeout_ms)) as session:
        return session.packet('inspect')


def cancel(root, timeout_ms=10000):
    # Local cancellation remains available when the source batch is unavailable.
    # It neither reads nor clears any designation obligation or native receipt.
    with open_monitor(root, Budget(timeout_ms), writable=True, source=False) as session:
        if session.journal.history.progress.status not in e.TERMINAL:
            session.journal.append({'kind': 'cancel'})
        return session.packet('cancel')


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command', required=True)
    for name in ('start', 'sample', 'inspect', 'cancel'):
        command = sub.add_parser(name)
        command.add_argument('--directory', type=Path, required=True)
        command.add_argument('--timeout-ms', type=int, default=10000)
        if name == 'start':
            command.add_argument('--batch', type=Path, required=True)
            command.add_argument('--batch-id', required=True)
            command.add_argument('--max-game-ticks', type=int, required=True)
            command.add_argument('--stable-ticks', type=int, default=10)
            command.add_argument('--required-samples', type=int, default=2)
            command.add_argument('--max-gap-ticks', type=int, default=1200)
    args = parser.parse_args(argv)
    try:
        if args.command == 'start':
            result = start(args.directory, args.batch, args.batch_id, args.max_game_ticks,
                           args.stable_ticks, args.required_samples, args.max_gap_ticks, args.timeout_ms)
        else:
            result = {'sample': sample, 'inspect': inspect, 'cancel': cancel}[args.command](
                args.directory, args.timeout_ms)
        raw = d.canonical(result)
        d.require(len(raw) <= MAX_OUTPUT, 'linked progress output exceeds bound')
        print(raw.decode('ascii'))
        return 0 if result['ok'] else 2
    except (ValueError, OSError, TypeError, KeyError, struct.error, RecursionError):
        print(t.encode_result({'ok': False, 'schema': 'dfmcp.dig-blueprint-progress/1',
            'goal_status': 'unknown', 'historical_designations_verified': False,
            'designation_and_sampled_goal_evidence_verified': False,
            'error': 'Binding, source, deadline or journal validation failed. Preserve both original stores; no repair performed.',
            'game_mutations_dispatched': False, 'mining_action_completed_proven': False,
            'retry_designation_permitted': False}, args.command).decode('ascii'))
        return 2


if __name__ == '__main__':
    raise SystemExit(main())
