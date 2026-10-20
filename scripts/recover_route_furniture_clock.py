#!/usr/bin/env python3
"""Recover an original construction-clock effect without reopening its batch.

Only inspect/query are exposed. Retained Scope/FinalScope and run intent must
agree byte-for-byte; their parent is held through terminal receipt publication.
No Batch object, furniture/map source, preparation, commit or readiness query.
Beads: df-dfhack-bridge-plane-c-pic.4/.5.
"""
from __future__ import annotations

import argparse
from dataclasses import dataclass, field
import os
from pathlib import Path
import sys
import time

import bounded_run_client as clock
import bounded_run_outcomes as storage
from route_furniture_clock_context import Client, SCOPE_SUFFIX, decode_scope
from route_furniture_review import digest_text

SCHEMA = 'dfmcp.scoped-clock-recovery/1'
MAX_OUTPUT = 65536
ENVIRONMENT = ('DFMCP_ALLOW_UNADMITTED_RUN_V1_13', 'DFMCP_RUN_ENDPOINT', 'DFMCP_RUN_TOKEN')


@dataclass(frozen=True)
class Authority:
    address: tuple[str, int]
    run_token: bytes = field(repr=False)

    @classmethod
    def load(cls):
        clock.require(all(not key.startswith('DFMCP_') or key in ENVIRONMENT for key in os.environ),
                      'clock-only recovery refuses unrelated authority')
        clock.require(os.environ.get(ENVIRONMENT[0]) == '1', 'clock recovery opt-in required')
        raw = os.environ.get(ENVIRONMENT[1], '127.0.0.1:5000')
        address = clock.endpoint(raw)
        clock.require(raw == f'{address[0]}:{address[1]}', 'noncanonical clock endpoint')
        token = os.environ.get(ENVIRONMENT[2], '').encode('utf-8')
        clock.require(32 <= len(token) <= 256 and b'\0' not in token, 'invalid clock credential')
        return cls(address, token)

    def clock_guard(self):
        clock.require(Authority.load() == self, 'clock recovery authority changed')


class Budget:
    """One deadline and actual wire counters; conservative bounded disk reservations."""
    def __init__(self, timeout_ms=10000):
        self.deadline = time.monotonic() + clock.integer(timeout_ms, 1, 60000) / 1000
        self._deadline = self.deadline
        self.calls = 32
        self.network_bytes = 3 * 1024 * 1024
        self.disk_bytes = 64 * 1024 * 1024
        self.work_steps = 100_000

    def remaining(self):
        clock.require(self.deadline == self._deadline, 'clock recovery deadline changed')
        left = self.deadline - time.monotonic()
        clock.require(left > 0, 'clock recovery deadline exhausted')
        return left

    def reserve(self, kind, count):
        self.remaining()
        clock.require(kind in ('calls', 'network_bytes', 'disk_bytes'), 'unknown recovery budget')
        clock.integer(count, 0, 64 * 1024 * 1024)
        available = getattr(self, kind)
        clock.require(type(available) is int and count <= available, 'clock recovery allowance exhausted')
        setattr(self, kind, available - count)

    def work(self):
        self.remaining()
        clock.require(self.work_steps > 0, 'clock recovery work exhausted')
        self.work_steps -= 1


def record_path(directory, confirmation):
    digest_text(confirmation)
    clock.require(type(directory) is str and '\0' not in directory and len(directory) <= 4096,
                  'invalid clock directory')
    path = Path(directory)
    clock.require(path.is_absolute() and str(path) == directory and '..' not in path.parts,
                  'clock directory must be absolute and canonical')
    return path / ('progress-' + confirmation + '.run')


class Recovery:
    """Selected clock custody only, not a substitute for the execution Batch.

    Do not call Scope.bind with a surrogate batch: this reader authenticates the
    independently selected scope and intent, not missing original effect files.
    Start-owner ScopeStore, source checks and execution permissions are unchanged.
    """
    def __init__(self, directory, batch_id, confirmation, budget):
        digest_text(batch_id)
        self.path = record_path(directory, confirmation)
        self.batch_id, self.confirmation, self.budget = batch_id, confirmation, budget
        self.store, self.fd, self.scope, self.data, self.stamp = None, None, None, None, None
        self.authority, self.fenced = None, False

    def __enter__(self):
        try:
            # OutcomeStore checks/syncs at most a bounded set of small files.
            self.budget.reserve('disk_bytes', 8 * storage.MAX_FILE)
            self.store = storage.OutcomeStore(self.path).__enter__()
            self.name = self.path.name + SCOPE_SUFFIX
            self.fd = os.open(self.name, storage.OPEN_READ, dir_fd=self.store.parent)
            self.data = storage._read(self.fd, self.store.parent, self.name)
            clock.require(self.data.endswith(b'\n'), 'incomplete original clock scope')
            self.scope = decode_scope(self.data[:-1])
            self.stamp = storage._fingerprint(os.fstat(self.fd))
            self._bind()
            self.check()
            return self
        except BaseException:
            self.close()
            raise

    def _bind(self):
        value = self.scope.value()
        clock.require(self.scope.digest == self.confirmation and value['batch_id'] == self.batch_id,
                      'clock scope differs from the independently selected batch/confirmation')
        clock.require(self.store.intent == self.scope.intent(),
                      'retained clock intent differs from its original scope')

    def check(self):
        clock.require(not self.fenced and self.fd is not None, 'clock recovery fenced or closed')
        try:
            self.budget.work()
            self.budget.reserve('disk_bytes', 3 * (storage.MAX_FILE + 1))
            self.store.check()
            clock.require(storage._fingerprint(os.fstat(self.fd)) == self.stamp
                          and storage._read(self.fd, self.store.parent, self.name) == self.data,
                          'retained clock scope changed')
            self._bind()
            if self.authority is not None:
                self.authority.clock_guard()
            self.budget.remaining()
        except BaseException:
            self.fenced = True
            raise

    def result(self, retained):
        phase = retained.get('record', {}).get('phase')
        value = {'schema': SCHEMA, 'batch_id': self.batch_id,
                 'confirmation': self.confirmation, 'scope': self.scope.value(),
                 'clock_key': self.scope.clock_key, 'clock_result': retained,
                 'selected_clock_files_verified': True, 'original_batch_custody_verified': False,
                 'complete_clock_inventory_verified': False,
                 'scope_is_historical_association_only': True,
                 'recovery_required': not retained.get('terminal_record_retained', False)
                     or phase == 'source_lost',
                 'current_pause_unproved': True, 'readiness_checked': False,
                 'placement_dispatched': False, 'clock_start_dispatched': False,
                 'clock_cancel_dispatched': False, 'retry_permitted': False,
                 'construction_completion_proven': False, 'production_admitted': False}
        encoded('inspect', value)
        return value

    def inspect(self):
        self.check()
        out = self.result(self.store.view())
        self.check()
        return out

    def query(self, wait_ms=0):
        clock.integer(wait_ms, 0, 60000)
        self.check()
        if self.store.load() is not None:
            return self.inspect()  # Exact terminal history needs no native credential.
        # Scope <=8 KiB and a complete clock result <=32 KiB remain intact.
        base = self.result({})
        clock.require(len(encoded('query', base)) + storage.MAX_OUTPUT + 1024 <= MAX_OUTPUT,
                      'complete recovery outcome cannot fit before native contact')
        self.authority = Authority.load()
        clock.require(self.authority.address == clock.endpoint(self.store.intent['endpoint']),
                      'recovery endpoint differs from original clock intent')
        self.check()
        # Reserve OutcomeStore replay/append/readback; actual wire use shares
        # this operation's counters through the unchanged guarded client.
        self.budget.reserve('disk_bytes', 16 * storage.MAX_FILE)
        with Client(self.authority, self.budget, self.check,
                    intent=self.store.intent, query=True) as client:
            original = self.scope.value()['clock_manifest']['generation']
            clock.require(client.manifest['generation'] >= original,
                          'recovery handshake predates original clock generation')
            result = clock.recover(client, self.store.intent, False, wait_ms)
        self.check()
        retained = self.store.retain(result)
        out = self.result(retained)
        self.check()
        return out

    def close(self):
        if self.fd is not None:
            os.close(self.fd)
            self.fd = None
        if self.store is not None:
            self.store.close()
            self.store = None

    def __exit__(self, error_type, *_):
        try:
            if error_type is None:
                self.check()
        finally:
            self.close()


def encoded(operation, result=None):
    value = {'schema': SCHEMA, 'operation': operation, 'ok': result is not None,
             'runtime_admitted': False, 'result': result,
             'original_batch_custody_verified': False, 'retry_permitted': False}
    if result is None:
        value['error'] = {'code': 'SCOPED_CLOCK_RECOVERY_REFUSED',
            'detail': 'Original clock files, authority, source, budget or evidence refused. Preserve the original key and files; do not restart the effect.'}
    raw = clock.canonical(value) + b'\n'
    clock.require(len(raw) <= MAX_OUTPUT, 'complete scoped clock recovery output exceeds bound')
    return raw


class Parser(argparse.ArgumentParser):
    def error(self, message):
        raise ValueError('invalid scoped clock recovery arguments')


def publish(raw):
    try:
        if sys.stdout.buffer.write(raw) != len(raw):
            return 2
        sys.stdout.buffer.flush()
        return 0
    except (OSError, ValueError, KeyboardInterrupt):
        return 2


def main(argv=None):
    operation, attempted = 'unknown', False
    try:
        parser = Parser(description=__doc__)
        parser.add_argument('operation', choices=('inspect', 'query'))
        parser.add_argument('--clock-directory', required=True)
        parser.add_argument('--batch-id', required=True)
        parser.add_argument('--confirm-progress', required=True)
        parser.add_argument('--wait-ms', type=int)
        parser.add_argument('--timeout-ms', type=int, default=10000)
        args = parser.parse_args(argv)
        operation = args.operation
        clock.require(operation == 'query' or args.wait_ms is None, 'wait belongs only to query')
        wait = 0 if args.wait_ms is None else clock.integer(args.wait_ms, 0, 60000)
        budget = Budget(args.timeout_ms)
        with Recovery(args.clock_directory, args.batch_id, args.confirm_progress, budget) as owner:
            out = owner.inspect() if operation == 'inspect' else owner.query(wait)
            raw = encoded(operation, out)
            owner.check()
            attempted = True
            return publish(raw)  # Hold selected original custody through the one output attempt.
    except (OSError, ValueError, TypeError, KeyError, RecursionError, KeyboardInterrupt):
        if not attempted:
            publish(encoded(operation))
        return 2


if __name__ == '__main__':
    raise SystemExit(main())
