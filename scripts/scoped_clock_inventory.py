"""Discover complete retained construction-clock custody without a transcript.

One private directory lock, bounded sequential readers, no batch reconstruction.
Local file evidence is not current game truth, original batch custody or a global
clock census. Existing scope, intent and native receipt codecs are unchanged.
Beads: df-dfhack-bridge-plane-c-pic.4/.5; GOAL-005.
"""
from __future__ import annotations

from dataclasses import dataclass
import hashlib
import json
import os
import re

import bounded_run_client as clock
import bounded_run_outcomes as storage
from route_furniture_clock_context import SCOPE_SUFFIX, decode_scope, FinalScope
from route_furniture_review import digest_text
import recover_route_furniture_clock as recovery

SCHEMA = 'dfmcp.scoped-clock-inventory/1'
MAX_RECORDS = 256
MAX_PAGE = 32
MAX_OUTPUT = 65536
NAME = re.compile(r'progress-([0-9a-f]{64})\.run(\.scope\.json|\.outcome\.json)?\Z')
SELECTIONS = ('all', 'pending', 'terminal', 'attention')


class Budget(recovery.Budget):
    """Directory-scale allowance; the deadline is never renewed per record."""
    def __init__(self, timeout_ms=10000):
        super().__init__(timeout_ms)
        self.disk_bytes = 256 * 1024 * 1024


@dataclass(frozen=True)
class Entry:
    confirmation: str
    batch_id: str
    summary: bytes
    files: tuple

    def row(self):
        # Return independent presentation data, never the pinned internal record.
        return json.loads(self.summary)


def _files(owner):
    store = owner.store
    return tuple((name, stamp, hashlib.sha256(data).hexdigest()) for name, stamp, data in (
        (owner.path.name, store.source_stat, store.source_data),
        (owner.name, owner.stamp, owner.data),
        (owner.path.name + storage.SUFFIX, store.outcome_stat, store.outcome_data))
        if data is not None)


def _entry(owner):
    result = owner.store.view()
    record = result.get('record', {})
    phase = record.get('phase')
    scope = owner.scope.value()
    row = {'confirmation': owner.confirmation, 'batch_id': owner.batch_id,
           'clock_key': owner.scope.clock_key,
           'phase': 'final_construction' if type(owner.scope) is FinalScope else 'between_placements',
           'endpoint': scope['source']['endpoint'],
           'generation': scope['clock_manifest']['generation'],
           'prepared_tick': clock.snapshot(bytes.fromhex(owner.store.intent['observation_hex']))['tick'],
           'game_ticks': scope['game_ticks'], 'wall_ms': scope['wall_ms'],
           'native_phase': phase, 'reason': record.get('reason'),
           'receipt_digest': record.get('receipt_digest_hex'),
           'query_required': phase is None, 'operator_attention': phase == 'source_lost',
           'unresolved': phase is None or phase == 'source_lost'}
    return Entry(owner.confirmation, owner.batch_id, clock.canonical(row), _files(owner))


class Inventory:
    """Complete selected-directory evidence, not all effects in a fortress.

    Readers close after each bounded record. Persistent fingerprints and byte
    digests are rechecked under the same directory lock before publication. This
    keeps descriptor use independent of record count; no clock files are created,
    repaired, removed or adopted. Any malformed/orphan entry refuses the census.
    """
    def __init__(self, directory, budget):
        self.probe = recovery.record_path(directory, '0' * 64)
        self.directory, self.budget = directory, budget
        self.parent, self.entries, self.fenced = None, (), False

    def _parent(self):
        self.budget.work()
        fd = storage._open_parent(self.probe)
        try:
            actual, held = os.fstat(fd), os.fstat(self.parent)
            clock.require((actual.st_dev, actual.st_ino) == (held.st_dev, held.st_ino),
                          'clock inventory directory replaced')
        finally:
            os.close(fd)

    def _names(self):
        names = []
        with os.scandir(self.parent) as found:
            for item in found:
                self.budget.work()
                clock.require(len(names) < 3 * MAX_RECORDS and NAME.fullmatch(item.name) is not None,
                              'unsupported or excessive clock inventory entries')
                names.append(item.name)
        return tuple(sorted(names))

    def _scan(self):
        self._parent()
        names = self._names()
        groups = {}
        for name in names:
            match = NAME.fullmatch(name)
            groups.setdefault(match[1], set()).add(match[2] or '')
        clock.require(len(groups) <= MAX_RECORDS
                      and all({'', SCOPE_SUFFIX} <= suffixes for suffixes in groups.values()),
                      'clock inventory contains too many records or orphan evidence')
        entries = []
        for confirmation in sorted(groups):
            self.budget.work()
            name = 'progress-' + confirmation + '.run' + SCOPE_SUFFIX
            self.budget.reserve('disk_bytes', storage.MAX_FILE + 1)
            fd = os.open(name, storage.OPEN_READ, dir_fd=self.parent)
            try:
                raw = storage._read(fd, self.parent, name)
                stamp = storage._fingerprint(os.fstat(fd))
            finally:
                os.close(fd)
            clock.require(raw.endswith(b'\n'), 'incomplete clock inventory scope')
            scope = decode_scope(raw[:-1])
            batch_id = scope.value()['batch_id']
            # This batch ID is discovered historical association, not user
            # confirmation or permission to reopen/execute an original batch.
            with recovery.Recovery(self.directory, batch_id, confirmation, self.budget,
                                   parent_fd=self.parent) as owner:
                clock.require(owner.data == raw and owner.stamp == stamp,
                              'clock inventory scope changed during selection')
                entries.append(_entry(owner))
        clock.require(self._names() == names, 'clock inventory membership changed during scan')
        self._parent()
        return tuple(entries)

    def __enter__(self):
        import fcntl
        try:
            self.budget.work()
            self.parent = storage._open_parent(self.probe)
            fcntl.flock(self.parent, fcntl.LOCK_EX | fcntl.LOCK_NB)
            self.entries = self._scan()
            return self
        except BaseException:
            self.close()
            raise

    def check(self):
        clock.require(self.parent is not None and not self.fenced, 'clock inventory fenced or closed')
        try:
            clock.require(self._scan() == self.entries, 'retained clock inventory changed')
        except BaseException:
            self.fenced = True
            raise

    @property
    def digest(self):
        info = os.fstat(self.parent)
        value = {'directory': self.directory, 'identity': [info.st_dev, info.st_ino],
                 'records': [[e.confirmation, e.batch_id, e.files] for e in self.entries]}
        return clock.digest(b'dfmcp-scoped-clock-inventory/1', clock.canonical(value)).hex()

    def page(self, *, selection='all', batch_id=None, limit=16, continuation=None):
        clock.require(selection in SELECTIONS, 'invalid clock inventory selection')
        clock.integer(limit, 1, MAX_PAGE)
        if batch_id is not None:
            digest_text(batch_id)
        self.check()
        root = self.digest
        rows = [e.row() for e in self.entries if batch_id is None or e.batch_id == batch_id]
        counts = {'total': len(rows), 'query_required': sum(r['query_required'] for r in rows),
                  'operator_attention': sum(r['operator_attention'] for r in rows),
                  'unresolved': sum(r['unresolved'] for r in rows),
                  'terminal_resolved': sum(not r['unresolved'] for r in rows)}
        selected = [r for r in rows if selection == 'all'
                    or (selection == 'pending' and r['query_required'])
                    or (selection == 'terminal' and r['native_phase'] is not None)
                    or (selection == 'attention' and r['operator_attention'])]
        query = clock.digest(b'dfmcp-scoped-clock-page/1', clock.canonical(
            [root, selection, batch_id, limit])).hex()
        offset = 0
        if continuation is not None:
            clock.require(type(continuation) is str and len(continuation) <= 68,
                          'invalid clock inventory continuation')
            match = re.fullmatch(r'([0-9a-f]{64}):([1-9][0-9]{0,2})', continuation)
            clock.require(match is not None and match[1] == query,
                          'clock inventory continuation is stale or belongs to another query')
            offset = int(match[2])
            clock.require(offset < len(selected) and offset % limit == 0,
                          'clock inventory continuation does not identify a page boundary')
        end = min(len(selected), offset + limit)
        out = {'inventory_digest': root, 'directory_records': len(self.entries),
               'batch_filter': batch_id, 'selection': selection, 'counts': counts,
               'selected_total': len(selected), 'offset': offset, 'rows': selected[offset:end],
               'truncated': end < len(selected),
               'continuation': query + ':' + str(end) if end < len(selected) else None,
               'complete_clock_inventory_verified': True,
               'inventory_scope': 'selected_private_directory_only',
               'original_batch_custody_verified': False,
               'discovered_batch_ids_are_historical_associations': True,
               'current_pause_unproved': True, 'readiness_checked': False,
               'construction_completion_proven': False, 'retry_permitted': False,
               'native_contacted': False}
        encoded('inventory', out)
        return out

    def close(self):
        if self.parent is not None:
            os.close(self.parent)
            self.parent = None

    def __exit__(self, *_):
        self.close()


def encoded(operation, result=None):
    packet = {'schema': SCHEMA, 'operation': operation, 'ok': result is not None,
              'result': result, 'runtime_admitted': False, 'retry_permitted': False,
              'clock_start_dispatched': False, 'clock_cancel_dispatched': False,
              'original_batch_custody_verified': False}
    if result is None:
        packet['error'] = {'code': 'SCOPED_CLOCK_INVENTORY_REFUSED',
                           'detail': 'Clock directory, evidence, continuation or budget refused. Preserve all original files; no effect was restarted.'}
    raw = clock.canonical(packet) + b'\n'
    clock.require(len(raw) <= MAX_OUTPUT, 'complete clock inventory output exceeds bound')
    return raw


def main(argv=None):
    attempted = False
    try:
        parser = recovery.Parser(description=__doc__)
        parser.add_argument('operation', choices=('inventory',))
        parser.add_argument('--clock-directory', required=True)
        parser.add_argument('--batch-id')
        parser.add_argument('--selection', choices=SELECTIONS, default='all')
        parser.add_argument('--limit', type=int, default=16)
        parser.add_argument('--continuation')
        parser.add_argument('--timeout-ms', type=int, default=10000)
        args = parser.parse_args(argv)
        budget = Budget(args.timeout_ms)
        with Inventory(args.clock_directory, budget) as owner:
            result = owner.page(selection=args.selection, batch_id=args.batch_id,
                                limit=args.limit, continuation=args.continuation)
            raw = encoded('inventory', result)
            owner.check()
            budget.remaining()
            attempted = True
            return recovery.publish(raw)
    except (OSError, ValueError, TypeError, KeyError, RecursionError, KeyboardInterrupt):
        if not attempted:
            recovery.publish(encoded('inventory'))
        return 2
