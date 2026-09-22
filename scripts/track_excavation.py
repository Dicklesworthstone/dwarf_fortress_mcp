#!/usr/bin/env python3
"""Persist and sample read-only excavation floor goals; never dispatch mining.

Start creates a new private journal. Sample performs at most one bounded map
read. Inspect and cancel never connect; cancellation stops this monitor only.
"""
from __future__ import annotations

import argparse
from contextlib import contextmanager
from dataclasses import dataclass, replace
import hashlib
import json
import os
from pathlib import Path
import secrets
import stat
import struct
import sys
import time
from typing import Callable, Iterator

import excavation_observer as e

MAX_FRAME = 16384
MAX_JOURNAL = 2 * 1024 * 1024
MAX_READS = 128
MAX_EVENTS = 260
MAX_OUTPUT = 32768
ENVIRONMENT = {'DFMCP_ALLOW_UNADMITTED_EXCAVATION_V1_5', 'DFMCP_MAP_TOKEN', 'DFMCP_MAP_ENDPOINT'}
DOMAIN = b'dfmcp-excavation-journal/1\0'


def sha(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def sample_value(capture: e.Capture) -> dict:
    return {'manifest': capture.manifest.json(), 'capture_hex': capture.raw.hex()}


def sample_decode(value: object, region: e.Region) -> e.Capture:
    e.require(isinstance(value, dict) and set(value) == {'manifest', 'capture_hex'}, 'invalid sample fields')
    raw = value['capture_hex']
    e.require(isinstance(raw, str) and 2 <= len(raw) <= 4096 and len(raw) % 2 == 0,
              'sample extent exceeds floor-goal bound')
    data = bytes.fromhex(raw)
    e.require(data.hex() == raw, 'noncanonical sample hexadecimal')
    return e.decode_capture(data, e.Manifest.from_json(value['manifest']), region)


@dataclass(frozen=True)
class History:
    goal: e.Goal
    endpoint: str
    progress: e.Progress
    pending_read: bool
    attempts: int
    events: int
    identity: str
    head: str


def frame(event: dict, sequence: int, previous: str) -> bytes:
    record = {'event': event, 'sequence': sequence, 'previous': previous}
    return e.canonical({**record, 'sha256': sha(DOMAIN + e.canonical(record))}) + b'\n'


def replay(raw: bytes, checkpoint: Callable[[], object] = lambda: None) -> History:
    e.require(1 <= len(raw) <= MAX_JOURNAL and raw.endswith(b'\n'), 'empty, oversized or torn journal')
    lines = raw.splitlines(keepends=True)
    e.require(len(lines) <= MAX_EVENTS, 'journal event bound exceeded')
    history = None
    previous = '0' * 64
    for sequence, line in enumerate(lines):
        checkpoint()
        e.require(1 <= len(line) <= MAX_FRAME, 'journal frame bound exceeded')
        record = json.loads(line, object_pairs_hook=e.unique_object)
        e.require(isinstance(record, dict) and set(record) == {'event', 'sequence', 'previous', 'sha256'},
                  'invalid journal envelope')
        e.integer(record['sequence'], sequence, sequence)
        e.require(record['previous'] == previous and line == frame(record['event'], sequence, previous),
                  'journal chain, encoding or checksum mismatch')
        event = record['event']
        e.require(isinstance(event, dict) and isinstance(event.get('kind'), str), 'invalid event')
        kind = event['kind']
        head = record['sha256']
        if history is None:
            e.require(kind == 'begin' and set(event) == {'kind', 'format', 'nonce', 'endpoint', 'goal', 'sample'}
                      and event['format'] == 'dfmcp.excavation-goal/1', 'journal must begin with a complete goal')
            nonce = event['nonce']
            e.require(isinstance(nonce, str) and len(nonce) == 64 and bytes.fromhex(nonce).hex() == nonce
                      and nonce != '0' * 64, 'invalid journal incarnation')
            e.endpoint(event['endpoint'])
            goal = e.Goal.from_json(event['goal'])
            progress = e.advance(goal, None, sample_decode(event['sample'], goal.region))
            history = History(goal, event['endpoint'], progress, False, 0, 1, head, head)
        else:
            e.require(history.progress.status not in e.TERMINAL, 'terminal goal history cannot change')
            progress, pending, attempts = history.progress, history.pending_read, history.attempts
            if kind == 'read_started':
                e.require(set(event) == {'kind'} and attempts < MAX_READS, 'read attempt bound exceeded')
                if pending:
                    progress = progress.interrupted('unfinished_read')
                pending, attempts = True, attempts + 1
            elif kind == 'sample':
                e.require(pending and set(event) == {'kind', 'sample'}, 'sample lacks durable read intent')
                progress = e.advance(history.goal, progress, sample_decode(event['sample'], history.goal.region))
                pending = False
            elif kind == 'read_failed':
                e.require(pending and set(event) == {'kind'}, 'failure lacks durable read intent')
                progress, pending = progress.interrupted('read_failed'), False
            elif kind == 'cancel':
                e.require(set(event) == {'kind'}, 'invalid monitor cancellation')
                progress = replace(progress, status='cancelled', streak=0, since_tick=None,
                                   interruption='monitor_cancelled_not_game_action')
                pending = False
            else:
                raise e.Rejected('unknown journal event')
            history = replace(history, progress=progress, pending_read=pending, attempts=attempts,
                              events=sequence + 1, head=head)
        previous = head
    e.require(history is not None, 'missing journal header')
    checkpoint()
    return history


class Budget:
    def __init__(self, timeout_ms: int):
        e.integer(timeout_ms, 1, 60000)
        self.deadline = time.monotonic() + timeout_ms / 1000

    def remaining_ms(self) -> int:
        left = int((self.deadline - time.monotonic()) * 1000)
        e.require(left >= 1, 'shared operation deadline exhausted')
        return left


class Journal:
    def __init__(self, path: Path, fd: int, parent: int, writable: bool, budget: Budget):
        self.path, self.fd, self.parent, self.writable, self.budget = path, fd, parent, writable, budget
        self.identity = self.file_id(os.fstat(fd))
        self.parent_identity = self.file_id(os.fstat(parent))
        self.raw = b''
        self.history = None
        self.fenced = False

    @staticmethod
    def file_id(info) -> tuple[int, int]:
        return info.st_dev, info.st_ino

    def custody(self) -> None:
        self.budget.remaining_ms()
        e.require(not self.fenced, 'journal fenced; reopen without repair')
        parent = os.stat(self.path.parent, follow_symlinks=False)
        opened_parent = os.fstat(self.parent)
        named = os.stat(self.path.name, dir_fd=self.parent, follow_symlinks=False)
        opened = os.fstat(self.fd)
        e.require(self.path.parent.resolve(strict=True) == self.path.parent
                  and self.file_id(parent) == self.file_id(opened_parent) == self.parent_identity
                  and stat.S_ISDIR(parent.st_mode) and stat.S_IMODE(parent.st_mode) == 0o700
                  and parent.st_uid in (0, os.geteuid()), 'journal parent custody changed')
        e.require(stat.S_ISREG(named.st_mode) and stat.S_ISREG(opened.st_mode)
                  and stat.S_IMODE(named.st_mode) == stat.S_IMODE(opened.st_mode) == 0o600
                  and named.st_uid == opened.st_uid == parent.st_uid
                  and named.st_nlink == opened.st_nlink == 1
                  and self.file_id(named) == self.file_id(opened) == self.identity,
                  'journal must remain an owned private single-link regular file')

    def contents(self) -> bytes:
        self.custody()
        before = os.fstat(self.fd)
        e.require(0 <= before.st_size <= MAX_JOURNAL, 'journal byte bound exceeded')
        os.lseek(self.fd, 0, os.SEEK_SET)
        raw = bytearray()
        while len(raw) <= before.st_size:
            self.budget.remaining_ms()
            part = os.read(self.fd, min(32768, before.st_size + 1 - len(raw)))
            if not part:
                break
            raw += part
        after = os.fstat(self.fd)
        e.require(len(raw) == before.st_size and (before.st_size, before.st_mtime_ns, before.st_ctime_ns)
                  == (after.st_size, after.st_mtime_ns, after.st_ctime_ns), 'journal changed during read')
        self.custody()
        return bytes(raw)

    def verify(self, expected: bytes | None = None) -> None:
        try:
            e.require(self.contents() == (self.raw if expected is None else expected), 'journal bytes changed')
        except BaseException:
            self.fenced = True
            raise

    def sync(self) -> None:
        e.require(self.writable, 'offline journal cannot synchronize')
        try:
            self.verify()
            os.fsync(self.fd)
            os.fsync(self.parent)
            self.verify()
        except BaseException:
            self.fenced = True
            raise

    def append(self, event: dict) -> None:
        e.require(self.writable, 'offline journal cannot append')
        self.verify()
        sequence = self.history.events if self.history else 0
        previous = self.history.head if self.history else '0' * 64
        addition = frame(event, sequence, previous)
        e.require(len(addition) <= MAX_FRAME, 'frame exceeds bound')
        candidate = self.raw + addition
        # Derive/validate the whole result BEFORE writing and reserve its response.
        history = replay(candidate, self.budget.remaining_ms)
        encode_result(report(history, False, 0))
        try:
            os.lseek(self.fd, 0, os.SEEK_END)
            view = memoryview(addition)
            while view:
                self.budget.remaining_ms()
                count = os.write(self.fd, view)
                e.require(count > 0, 'short journal append')
                view = view[count:]
            self.verify(candidate)
            os.fsync(self.fd)
            os.fsync(self.parent)
            self.verify(candidate)
        except BaseException:
            self.fenced = True
            raise
        self.raw, self.history = candidate, history


@contextmanager
def open_journal(path: Path, budget: Budget, *, writable=False, create=False) -> Iterator[Journal]:
    e.require(os.name == 'posix' and hasattr(os, 'O_NOFOLLOW'), 'private journal requires POSIX custody')
    import fcntl
    e.require(path.is_absolute() and 1 <= len(str(path)) <= 4096 and '..' not in path.parts
              and path.name not in ('', '.', '..') and (not create or writable), 'invalid journal path/mode')
    parent = os.open('/', os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
    fd = None
    try:
        for part in path.parts[1:-1]:
            budget.remaining_ms()
            child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=parent)
            os.close(parent)
            parent = child
        directory = os.fstat(parent)
        e.require(stat.S_IMODE(directory.st_mode) == 0o700 and directory.st_uid in (0, os.geteuid()),
                  'journal parent must be owned and exact 0700')
        flags = os.O_RDWR | os.O_APPEND if writable else os.O_RDONLY
        if create:
            flags |= os.O_CREAT | os.O_EXCL
        fd = os.open(path.name, flags | os.O_NOFOLLOW | os.O_CLOEXEC | os.O_NONBLOCK, 0o600, dir_fd=parent)
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        journal = Journal(path, fd, parent, writable, budget)
        journal.raw = journal.contents()
        if create:
            e.require(journal.raw == b'', 'new journal is not empty')
        else:
            journal.history = replay(journal.raw, budget.remaining_ms)
        journal.verify()
        yield journal
    finally:
        if fd is not None:
            os.close(fd)
        os.close(parent)


def environment(saved_endpoint: str | None = None) -> tuple[str, bytes]:
    e.require(os.environ.get('DFMCP_ALLOW_UNADMITTED_EXCAVATION_V1_5') == '1'
              and all(not name.startswith('DFMCP_') or name in ENVIRONMENT for name in os.environ),
              'exact read-only development environment required')
    configured = os.environ.get('DFMCP_MAP_ENDPOINT')
    e.require(saved_endpoint is None or configured is None or configured == saved_endpoint,
              'configured endpoint differs from retained observation source')
    address = saved_endpoint or configured or '127.0.0.1:5000'
    e.endpoint(address)
    token = os.environ.get('DFMCP_MAP_TOKEN', '').encode('utf-8')
    e.require(32 <= len(token) <= 256, 'missing or invalid map token')
    return address, token


def report(history: History, sampled_this_call: bool, native_reads_attempted: int) -> dict:
    p = history.progress.interrupted('unfinished_read') if history.pending_read else history.progress
    return {'ok': True, 'schema': 'dfmcp.excavation-progress/1', 'goal': history.goal.json(),
            'journal_id': history.identity, 'journal_head': history.head, 'journal_events': history.events,
            'goal_status': p.status, 'terminal': p.status in e.TERMINAL,
            'floor_goal_satisfied_at_sample': p.status == 'satisfied',
            'source': p.first.binding(), 'endpoint': history.endpoint,
            'last_observed_tick': p.latest.tick, 'last_observation_witness': p.latest.witness,
            'counts_at_last_observation': e.classify(p.latest), 'matching_samples': p.streak,
            'matching_since_tick': p.since_tick, 'observations_retained': p.observations,
            'read_attempts': history.attempts, 'pending_read': history.pending_read,
            'interruption': p.interruption, 'sampled_this_call': sampled_this_call,
            'native_reads_attempted': native_reads_attempted, 'evidence_is_historical': True,
            'current_conditions_proven': False, 'continuous_stability_proven': False,
            'mining_action_completed_proven': False, 'safety_proven': False,
            'native_effect_obligations_changed': False, 'game_mutations_dispatched': False,
            'retry_designation_permitted': False,
            'next_step': 'inspect_retained_evidence' if p.status in e.TERMINAL else 'explicit_sample_or_cancel_monitor'}


def encode_result(value: dict, operation: str = 'inspect') -> bytes:
    # Add the common orientation spine without fabricating a canonical world
    # anchor, semantic request ID, authority or inventory of native effects.
    value = dict(value)
    known = 'journal_head' in value
    pending = known and not value.get('terminal', False)
    goal = value.get('goal')
    status = value.get('goal_status', 'unknown')
    reference = {'journal_id': value.get('journal_id'), 'head': value.get('journal_head')}
    value['agent_turn'] = {
        'schema': 'dfmcp.agent_turn/1', 'operation': 'excavation.' + operation,
        'phase': {'start': 'bootstrap', 'sample': 'verify', 'cancel': 'reconcile'}.get(operation, 'inspect'),
        'session_id': None, 'turn_id': None, 'request_id': None, 'anchor': None,
        'continuity': {'status': 'indeterminate' if status in ('unknown', 'invalidated') else 'stale',
                       'basis': None, 'gap': 'sampled_endpoints_only', 'reset_reason': value.get('interruption')},
        'profile': 'tactical',
        'briefing': {'goal_status': status, 'runtime_admitted': False,
                     'mutation_admissible': False, 'current_terrain_proven': False},
        'changes': [], 'attention': [],
        'active_work': {'scope': 'this_read_only_goal_only', 'inventory_verified': known,
                        'obligations': [dict(reference, status=status, deadline_tick=goal['deadline_tick'])] if pending else [],
                        'native_effect_inventory_verified': False},
        'affordances': [],
        'recommendations': [{'operation': 'sample' if pending else 'inspect',
                             'journal_id': value.get('journal_id'), 'authority_granted': False}],
        'uncertainty': [{'epistemic': 'unknown', 'message':
            'Sampled dry-floor evidence does not prove continuous stability, mining causality, safety or native effect completion.'}],
        'coverage': {'status': 'partial', 'scope': goal.get('region') if goal else None,
                     'complete_domains': ['retained_goal_history'] if known else [],
                     'omitted_domains': ['continuous_game_history', 'current_world', 'native_effect_inventory']},
        'budget': {'maximum_output_bytes': MAX_OUTPUT, 'token_count_measured': False},
        'references': [dict(reference, observation_witness=value.get('last_observation_witness'),
                            observed_game_tick=value.get('last_observed_tick'))] if known else [],
    }
    raw = e.canonical(value)
    e.require(len(raw) <= MAX_OUTPUT, 'complete progress result exceeds 32 KiB')
    return raw


def start(path: Path, region: e.Region, folder: str, site: int, max_game_ticks: int,
          stable_ticks=10, required_samples=2, max_gap_ticks=1200, timeout_ms=10000) -> dict:
    budget = Budget(timeout_ms)
    e.integer(max_game_ticks, 1, 403200)
    template = e.Goal(region, folder, site, e.MAX_TICK, stable_ticks, required_samples, max_gap_ticks)
    e.require(stable_ticks <= max_game_ticks, 'stability span exceeds goal horizon')
    address, token = environment()
    with open_journal(path, budget, writable=True, create=True) as journal:
        with e.MapClient(address, token, region, budget.remaining_ms()) as client:
            initial = client.observe()
        goal = replace(template, deadline_tick=initial.tick + max_game_ticks)
        event = {'kind': 'begin', 'format': 'dfmcp.excavation-goal/1', 'nonce': secrets.token_hex(32),
                 'endpoint': address, 'goal': goal.json(), 'sample': sample_value(initial)}
        journal.append(event)
        return report(journal.history, True, 1)


def sample(path: Path, timeout_ms=10000) -> dict:
    budget = Budget(timeout_ms)
    with open_journal(path, budget, writable=True) as journal:
        h = journal.history
        if h.progress.status in e.TERMINAL:
            return report(h, False, 0)  # No token, opt-in or native access for terminal history.
        e.require(h.attempts < MAX_READS and h.events + 3 <= MAX_EVENTS
                  and len(journal.raw) + 3 * MAX_FRAME <= MAX_JOURNAL,
                  'goal retention exhausted; cancel monitor or inspect without eviction')
        address, token = environment(h.endpoint)
        journal.sync()
        journal.append({'kind': 'read_started'})  # A crash cannot silently preserve a stable streak.
        try:
            with e.MapClient(address, token, h.goal.region, budget.remaining_ms()) as client:
                observed = client.observe()
            journal.append({'kind': 'sample', 'sample': sample_value(observed)})
        except (e.Rejected, OSError, ValueError, KeyError, TypeError, struct.error):
            if journal.fenced:
                raise
            journal.append({'kind': 'read_failed'})
            result = report(journal.history, False, 1)
            result.update(ok=False, error='map_read_failed; stable streak reset; no mutation retried')
            return result
        return report(journal.history, True, 1)


def inspect(path: Path, timeout_ms=10000) -> dict:
    with open_journal(path, Budget(timeout_ms)) as journal:
        result = report(journal.history, False, 0)
        journal.verify()
        return result


def cancel(path: Path, timeout_ms=10000) -> dict:
    with open_journal(path, Budget(timeout_ms), writable=True) as journal:
        if journal.history.progress.status not in e.TERMINAL:
            journal.append({'kind': 'cancel'})
        return report(journal.history, False, 0)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command', required=True)
    for name in ('start', 'sample', 'inspect', 'cancel'):
        command = sub.add_parser(name)
        command.add_argument('--journal', type=Path, required=True)
        command.add_argument('--timeout-ms', type=int, default=10000)
        if name == 'start':
            for field in ('x', 'y', 'z', 'width', 'height'):
                command.add_argument('--' + field, type=int, required=True)
            command.add_argument('--world-folder', required=True)
            command.add_argument('--site', type=int, required=True)
            command.add_argument('--max-game-ticks', type=int, required=True)
            command.add_argument('--stable-ticks', type=int, default=10)
            command.add_argument('--required-samples', type=int, default=2)
            command.add_argument('--max-gap-ticks', type=int, default=1200)
    args = parser.parse_args(argv)
    try:
        if args.command == 'start':
            region = e.Region((args.x, args.y, args.z), (args.width, args.height, 1))
            result = start(args.journal, region, args.world_folder, args.site, args.max_game_ticks,
                           args.stable_ticks, args.required_samples, args.max_gap_ticks, args.timeout_ms)
        else:
            result = {'sample': sample, 'inspect': inspect, 'cancel': cancel}[args.command](args.journal, args.timeout_ms)
        print(encode_result(result, args.command).decode('ascii'))
        return 0 if result['ok'] else 2
    except (e.Rejected, OSError, ValueError, KeyError, TypeError, struct.error, RecursionError):
        print(encode_result({'ok': False, 'schema': 'dfmcp.excavation-progress/1', 'goal_status': 'unknown',
            'error': 'Read, deadline, goal or journal verification failed. Preserve the original journal; no repair performed.',
            'current_conditions_proven': False, 'mining_action_completed_proven': False,
            'native_effect_obligations_changed': False, 'game_mutations_dispatched': False,
            'retry_designation_permitted': False}, args.command).decode('ascii'))
        return 2


if __name__ == '__main__':
    sys.exit(main())
