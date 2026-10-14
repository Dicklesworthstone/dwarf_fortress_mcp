"""Offline discovery and original-history gates for scoped construction clocks.

The caller retains original batch custody. One private clock-directory lock
covers complete bounded discovery, pagination, and optional admission checks
around a NEW clock effect. No read issues a native call, repairs a file, adopts
an unassociated intent, retries a clock, or authorizes furniture placement.
Beads: df-dfhack-bridge-plane-c-pic.4/.5.
"""
from __future__ import annotations

from dataclasses import dataclass
import hashlib
import os
from pathlib import Path
import re

import bounded_run_outcomes as storage
from furniture_plan import canonical, decode_json, integer, require
from route_furniture_clock_context import Scope, SCOPE_SUFFIX
from route_furniture_review import digest_text

POLICY = 'complete-private-scoped-clock-history-before-new-effect/1'
SCHEMA = 'dfmcp.route-furniture-clock-inventory/1'
MAX_RECORDS = 256
MAX_ENTRIES = 3 * MAX_RECORDS
MAX_PAGE = 64
MAX_OUTPUT = 65536
NAME = re.compile(r'progress-([0-9a-f]{64})\.run(\.scope\.json|\.outcome\.json)?\Z')
# The unchanged OutcomeStore checks exact source/outcome bytes several times.
# Reserve a conservative whole-store bound before entering it, never refund it.
STORE_READ_RESERVATION = 12 * (storage.MAX_FILE + 1)


@dataclass(frozen=True)
class FileWitness:
    stamp: tuple
    sha256: str


class Inventory:
    """Complete local history with an optional borrowed, already-held lock.

    File identities AND exact content commitments remain checked at subsequent
    effect/publication boundaries. A new effect may add only its own three fixed
    names; old files cannot disappear or change. Its ScopeStore/OutcomeStore
    still own validation of the new files themselves. This is cooperative local
    custody, not malicious-owner rollback protection or a cross-directory fence.
    """
    def __init__(self, directory: Path, batch, *, parent_fd: int | None = None):
        require(isinstance(directory, Path) and directory.is_absolute()
                and '..' not in directory.parts, 'absolute private clock directory required')
        self.directory, self.batch, self.budget = directory, batch, batch.budget
        self.borrowed_parent, self.parent = parent_fd, None
        self._files, self._rows, self._allowed_new = {}, (), frozenset()
        self._groups = 0
        self._bound = None
        self._entered = False

    def _guard(self):
        require(self.batch.budget is self.budget, 'clock inventory budget owner changed')
        self.budget.work()
        self.batch.check()
        self.budget.remaining()

    def _binding(self):
        state = self.batch.audit()
        require(state['inventory_verified'] is True
                and state['batch_id'] == self.batch.id
                and state['review_digest'] == self.batch.review.digest,
                'clock inventory lacks original batch custody')
        for value in (self.batch.id, self.batch.review.digest, self.batch.plan.digest,
                      state['inventory_digest']):
            digest_text(value)
        return (self.batch.id, self.batch.review.digest, self.batch.plan.digest,
                state['inventory_digest'])

    def _parent_check(self):
        fd = storage._open_parent(self.directory / 'inventory')
        try:
            left, right = os.fstat(fd), os.fstat(self.parent)
            require((left.st_dev, left.st_ino) == (right.st_dev, right.st_ino),
                    'clock inventory directory replaced')
        finally:
            os.close(fd)

    def _names(self):
        self._guard()
        names = []
        with os.scandir(self.parent) as entries:
            for entry in entries:
                self.budget.work()
                require(len(names) < MAX_ENTRIES and NAME.fullmatch(entry.name) is not None,
                        'clock directory has unsupported names or exceeds its entry bound')
                names.append(entry.name)
        return tuple(sorted(names))

    def _read(self, name):
        self._guard()
        self.budget.reserve('disk_bytes', storage.MAX_FILE + 1)
        fd = os.open(name, storage.OPEN_READ, dir_fd=self.parent)
        try:
            before = os.fstat(fd)
            storage._private(before)
            require(0 < before.st_size <= storage.MAX_FILE, 'invalid bounded clock artifact size')
            raw = bytearray()
            while len(raw) <= before.st_size:
                self.budget.work()
                part = os.pread(fd, min(4096, before.st_size + 1 - len(raw)), len(raw))
                if not part:
                    break
                raw.extend(part)
            after = os.fstat(fd)
            named = os.stat(name, dir_fd=self.parent, follow_symlinks=False)
            stamp = storage._fingerprint(before)
            require(stamp == storage._fingerprint(after) == storage._fingerprint(named)
                    and len(raw) == before.st_size, 'clock artifact changed during discovery')
            return bytes(raw), FileWitness(stamp, hashlib.sha256(raw).hexdigest())
        finally:
            os.close(fd)

    def __enter__(self):
        import fcntl
        require(not self._entered, 'clock inventory owner cannot be reused')
        self._entered = True
        try:
            self._guard()
            self.parent = (storage._open_parent(self.directory / 'inventory')
                           if self.borrowed_parent is None else os.dup(self.borrowed_parent))
            fcntl.flock(self.parent, fcntl.LOCK_EX | fcntl.LOCK_NB)
            self._parent_check()
            self._bound = self._binding()
            names = self._names()
            groups = {}
            for name in names:
                self.budget.work()
                match = NAME.fullmatch(name)
                identity, suffix = match.group(1), match.group(2) or ''
                groups.setdefault(identity, {})[suffix] = name
                require(len(groups) <= MAX_RECORDS, 'clock directory exceeds record bound')
            rows = []
            for identity, files in sorted(groups.items()):
                self._guard()
                raw = {}
                for suffix, name in sorted(files.items()):
                    raw[suffix], self._files[name] = self._read(name)
                require('' in files or SCOPE_SUFFIX in files, 'orphan clock outcome')
                require(storage.SUFFIX not in files or '' in files,
                        'clock outcome has no original native intent')
                scope, association, next_step = None, 'unbound', None
                if SCOPE_SUFFIX in files:
                    require(raw[SCOPE_SUFFIX].endswith(b'\n'), 'incomplete clock scope')
                    scope = Scope(raw[SCOPE_SUFFIX][:-1])
                    value = scope.value()
                    require(scope.digest == identity, 'clock scope filename differs from its identity')
                    association = 'original_batch' if value['batch_id'] == self.batch.id else 'other_batch'
                    if association == 'original_batch':
                        scope.bind(self.batch)
                        next_step = value['next_step']
                saved = None
                if '' in files:
                    self.budget.reserve('disk_bytes', STORE_READ_RESERVATION)
                    with storage.OutcomeStore(self.directory / files[''], parent_fd=self.parent) as store:
                        require(store.source_stat == self._files[files['']].stamp
                                and store.source_data == raw[''], 'intent changed between reads')
                        require(store.intent['idempotency_key'] == 'frp-' + identity,
                                'clock intent key differs from its record name')
                        if scope is not None:
                            require(store.intent == scope.intent(), 'native intent differs from original scope')
                        saved = store.load()
                        require((saved is not None) == (storage.SUFFIX in files),
                                'clock outcome appeared during discovery')
                        if saved is not None:
                            require(store.outcome_stat == self._files[files[storage.SUFFIX]].stamp
                                    and store.outcome_data == raw[storage.SUFFIX],
                                    'outcome changed between reads')
                phase = None if saved is None else saved['record']['phase']
                # SourceLost is terminal native history but NOT a resolved effect.
                resolved = scope is not None and phase in ('stopped', 'refused')
                queryable = scope is not None and '' in files and saved is None
                reason = ('unassociated_intent' if scope is None else
                          'scope_without_intent' if '' not in files else
                          'native_outcome_unknown' if saved is None else
                          'source_lost' if phase == 'source_lost' else 'terminal_record_verified')
                rows.append({'clock_record_id': identity, 'clock_key': 'frp-' + identity,
                             'association': association, 'next_step': next_step,
                             'native_phase': phase, 'status': reason, 'resolved': resolved,
                             'query_supported': queryable and association == 'original_batch',
                             'receipt_digest': None if saved is None else saved['record']['receipt_digest_hex'],
                             'retry_permitted': False})
            self._rows, self._groups = tuple(rows), len(groups)
            require(self._names() == names, 'clock directory membership changed during discovery')
            self.check()
            return self
        except BaseException:
            self.close()
            raise

    def check(self):
        require(self.parent is not None and self._bound is not None, 'clock inventory is closed')
        self._guard()
        self._parent_check()
        require(self._binding() == self._bound, 'original batch changed during clock discovery')
        names = frozenset(self._names())
        require(frozenset(self._files) <= names <= frozenset(self._files) | self._allowed_new,
                'clock inventory membership changed')
        for name, expected in sorted(self._files.items()):
            _, current = self._read(name)
            require(current == expected, 'original clock history changed')
        require(frozenset(self._names()) == names
                and self._binding() == self._bound, 'clock history changed before publication')
        self._guard()

    @property
    def digest(self):
        info = os.fstat(self.parent)
        value = {'policy': POLICY, 'batch': self._bound, 'directory': str(self.directory),
                 'directory_identity': [info.st_dev, info.st_ino],
                 'files': [{'name': name, 'stamp': witness.stamp, 'sha256': witness.sha256}
                           for name, witness in sorted(self._files.items())]}
        return hashlib.sha256(b'dfmcp-scoped-clock-inventory/1\0' + canonical(value)).hexdigest()

    def counts(self):
        return {'records': self._groups,
                'original_batch': sum(r['association'] == 'original_batch' for r in self._rows),
                'other_batch': sum(r['association'] == 'other_batch' for r in self._rows),
                'unassociated': sum(r['association'] == 'unbound' for r in self._rows),
                'unresolved': sum(not r['resolved'] for r in self._rows),
                'original_batch_unresolved': sum(not r['resolved'] and r['association'] == 'original_batch'
                                                 for r in self._rows),
                'query_supported': sum(r['query_supported'] for r in self._rows)}

    def admit_new(self, scope: Scope):
        """Narrow local admission only; no native permission or file creation.

        ALL prior scoped records in this selected directory must be resolved.
        Unknown/unassociated/foreign-source-loss history cannot be hidden by a
        caller's page filter. A different directory is outside this local claim.
        """
        require(type(scope) is Scope and not self._allowed_new, 'new clock scope already selected')
        self.check()
        scope.bind(self.batch, current=True)
        require(self._groups < MAX_RECORDS and all(r['resolved'] for r in self._rows),
                'recover original clock history before starting another progress effect')
        name = 'progress-' + scope.digest + '.run'
        allowed = frozenset((name, name + SCOPE_SUFFIX, name + storage.SUFFIX))
        require(not allowed.intersection(self._files), 'new clock scope already has history')
        self._allowed_new = allowed
        self.check()

    def page(self, *, selection='all', limit=16, continuation=None):
        require(not self._allowed_new, 'discovery pages cannot omit a newly started clock effect')
        require(selection in ('all', 'pending', 'terminal'), 'invalid clock-history selection')
        integer(limit, 1, MAX_PAGE)
        self.check()
        root = self.digest
        rows = [r for r in self._rows if r['association'] != 'other_batch'
                and (selection == 'all' or (not r['resolved'] if selection == 'pending' else r['resolved']))]
        offset = 0
        if continuation is not None:
            require(type(continuation) is str and 0 < len(continuation) <= 2048
                    and continuation.count('.') == 1, 'invalid clock-history continuation')
            text, checksum = continuation.split('.')
            require(re.fullmatch(r'(?:[0-9a-f]{2})+', text) is not None,
                    'noncanonical clock-history continuation')
            raw = bytes.fromhex(text)
            value = decode_json(raw)
            require(type(value) is dict and set(value) == {'root', 'selection', 'limit', 'offset'}
                    and canonical(value) == raw and checksum == hashlib.sha256(
                        b'dfmcp-scoped-clock-page/1\0' + raw).hexdigest(), 'invalid continuation identity')
            integer(value['limit'], 1, MAX_PAGE)
            require(value['root'] == root and value['selection'] == selection and value['limit'] == limit,
                    'clock-history continuation is stale or for another selection')
            offset = integer(value['offset'], 1, MAX_RECORDS)
            require(offset < len(rows), 'clock-history continuation exceeds result horizon')
        chosen = rows[offset:offset + limit]
        following = None
        if offset + len(chosen) < len(rows):
            raw = canonical({'root': root, 'selection': selection, 'limit': limit,
                             'offset': offset + len(chosen)})
            following = raw.hex() + '.' + hashlib.sha256(b'dfmcp-scoped-clock-page/1\0' + raw).hexdigest()
        result = {'schema': SCHEMA, 'policy': POLICY, 'inventory_digest': root,
                  'batch_id': self.batch.id, 'counts': self.counts(), 'rows': [dict(r) for r in chosen],
                  'selection': selection, 'matched': len(rows), 'continuation': following,
                  'complete_directory_validated': True, 'other_batch_details_omitted': True,
                  'directory_history_reconciled': all(r['resolved'] for r in self._rows),
                  'native_contacted': False, 'files_modified': False, 'retry_permitted': False,
                  'clock_start_authorized': False, 'current_pause_unproved': True,
                  'construction_completion_proven': False, 'production_admitted': False}
        require(len(canonical(result)) <= MAX_OUTPUT, 'complete clock-history page exceeds output bound')
        self.check()
        return result

    def close(self):
        if self.parent is not None:
            os.close(self.parent)
            self.parent = None

    def __exit__(self, error_type, *_):
        try:
            if error_type is None:
                self.check()
        finally:
            self.close()
