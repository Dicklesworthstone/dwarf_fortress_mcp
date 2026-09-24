#!/usr/bin/env python3
"""Bounded offline inventory and query-only recovery of run/1.13 intent directories.

No start, prepare, cancel, unpause, daemon, command evaluator or native replay.
Bead: df-dfhack-bridge-plane-c-pic.5. This is a developer workflow, not admission.
"""
from __future__ import annotations

import argparse
from contextlib import ExitStack
import hashlib
import json
import os
from pathlib import Path
import re
import time

import bounded_run_client as c
import bounded_run_outcomes as o

MAX_RECORDS = 256
MAX_QUERIES = 16
MAX_OUTPUT = 65536
NAME = re.compile(r'[A-Za-z0-9][A-Za-z0-9_.-]{0,191}\Z')
FORMAT = 'dfmcp.bounded-run-recovery/1'


def bounded(packet: dict) -> bytes:
    raw = c.canonical(packet)
    c.require(len(raw) <= MAX_OUTPUT, 'complete recovery packet exceeds output allowance')
    return raw


def settings() -> tuple[tuple[str, int], bytes]:
    allowed = {'DFMCP_ALLOW_UNADMITTED_RUN_V1_13', 'DFMCP_RUN_TOKEN', 'DFMCP_RUN_ENDPOINT'}
    c.require(not any(key.startswith('DFMCP_') and key not in allowed for key in os.environ),
              'query-only recovery rejects other development and production authority')
    c.require(os.environ.get('DFMCP_ALLOW_UNADMITTED_RUN_V1_13') == '1', 'exact run recovery opt-in required')
    token = os.environ.get('DFMCP_RUN_TOKEN', '').encode('utf-8')
    c.require(32 <= len(token) <= 256, 'invalid run recovery credential length')
    return c.endpoint(os.environ.get('DFMCP_RUN_ENDPOINT', '127.0.0.1:5000')), token


class QueryOnlyClient(c.Client):
    """Inherit bounded native framing, but reject every non-query dispatch."""
    def __init__(self, address, token, deadline):
        self.outer_deadline = deadline
        remaining = int((deadline - time.monotonic()) * 1000)
        c.integer(remaining, 1, 60000)
        super().__init__(address, token, remaining)

    def remaining(self):
        left = min(super().remaining(), self.outer_deadline - time.monotonic())
        c.require(left >= 0.001, 'whole-pass native deadline exhausted')
        return left

    def call(self, operation, fields=None):
        c.require(operation in ('Handshake', 'QueryRun'), 'recovery transport is query-only')
        return super().call(operation, fields)


class Inventory:
    """One directory lock owns a complete bounded set of pinned intent/receipt readers."""
    def __init__(self, directory: Path):
        self.directory = directory
        self.stack = ExitStack()
        self.stores = {}
        self.parent = None

    def names(self) -> list[str]:
        names = []
        with os.scandir(self.parent) as entries:
            for entry in entries:
                c.require(len(names) < 2 * MAX_RECORDS, 'recovery directory exceeds entry bound')
                base = entry.name[:-len(o.SUFFIX)] if entry.name.endswith(o.SUFFIX) else entry.name
                c.require(NAME.fullmatch(base) is not None, 'unsupported recovery filename')
                names.append(entry.name)
        return sorted(names)

    def __enter__(self):
        import fcntl
        try:
            self.parent = o._open_parent(self.directory / 'inventory')
            self.stack.callback(os.close, self.parent)
            fcntl.flock(self.parent, fcntl.LOCK_EX | fcntl.LOCK_NB)
            names = self.names()
            intents = [name for name in names if not name.endswith(o.SUFFIX)]
            c.require(len(intents) <= MAX_RECORDS, 'too many retained run intents')
            c.require(all(not name.endswith(o.SUFFIX) or name[:-len(o.SUFFIX)] in intents for name in names),
                      'orphan outcome in recovery directory')
            seen = set()
            for name in intents:
                store = self.stack.enter_context(o.OutcomeStore(self.directory / name, parent_fd=self.parent))
                intent = store.intent
                scope = (intent['endpoint'], intent['df_version'], intent['dfhack_version'],
                         c.snapshot(bytes.fromhex(intent['observation_hex']))['generation'], intent['idempotency_key'])
                c.require(scope not in seen, 'duplicate native idempotency identity in recovery directory')
                seen.add(scope)
                self.stores[name] = store
            self.check()
            return self
        except BaseException:
            self.stack.close()
            raise

    def check(self):
        fd = o._open_parent(self.directory / 'inventory')
        try:
            a, b = os.fstat(fd), os.fstat(self.parent)
            c.require((a.st_dev, a.st_ino) == (b.st_dev, b.st_ino), 'inventory directory was replaced')
        finally:
            os.close(fd)
        expected = list(self.stores)
        for name, store in self.stores.items():
            store.check()
            if store.outcome is not None:
                expected.append(name + o.SUFFIX)
        c.require(self.names() == sorted(expected), 'inventory membership changed')

    def digest(self) -> str:
        entries = [{'name': name, 'intent': hashlib.sha256(store.source_data).hexdigest(),
                    'outcome': hashlib.sha256(store.outcome_data).hexdigest() if store.outcome_data is not None else None}
                   for name, store in self.stores.items()]
        info = os.fstat(self.parent)
        payload = {'directory': str(self.directory), 'identity': [info.st_dev, info.st_ino], 'entries': entries}
        return c.digest(b'dfmcp-bounded-run-inventory/1', c.canonical(payload)).hex()

    def rows(self) -> list[dict]:
        rows = []
        for name, store in self.stores.items():
            saved = store.load()
            record = saved['record'] if saved else {}
            before = c.snapshot(bytes.fromhex(store.intent['observation_hex']))
            phase = record.get('phase')
            rows.append({'name': name, 'key': store.intent['idempotency_key'],
                         'plan_digest': store.intent['plan_digest_hex'], 'generation': before['generation'],
                         'prepared_tick': before['tick'], 'game_ticks': store.intent['game_ticks'],
                         'wall_ms': store.intent['wall_ms'], 'native_phase': phase,
                         'reason': record.get('reason'), 'observed_tick': record.get('observed_tick'),
                         'receipt_digest': record.get('receipt_digest_hex'),
                         'query_required': saved is None, 'operator_attention': phase == 'source_lost',
                         'unresolved': saved is None or phase == 'source_lost'})
        return rows

    def counts(self) -> dict:
        rows = self.rows()
        pending = [row for row in rows if row['unresolved']]
        return {'total': len(rows), 'unresolved': len(pending),
                'query_required': sum(row['query_required'] for row in rows),
                'operator_attention': sum(row['operator_attention'] for row in rows),
                'terminal_resolved': sum(not row['unresolved'] for row in rows),
                'pending_names_preview': [row['name'] for row in pending[:8]],
                'pending_names_omitted': max(0, len(pending) - 8)}

    def page(self, selection='all', limit=16, continuation=None) -> dict:
        c.require(selection in ('all', 'pending', 'terminal'), 'invalid inventory selection')
        c.integer(limit, 1, 64)
        self.check()
        root = self.digest()
        rows = [row for row in self.rows() if selection == 'all'
                or (row['unresolved'] if selection == 'pending' else not row['unresolved'])]
        offset = 0
        if continuation is not None:
            c.require(isinstance(continuation, str) and len(continuation) <= 2048 and continuation.count('.') == 1,
                      'invalid inventory continuation')
            encoded, checksum = continuation.split('.')
            c.require(len(encoded) % 2 == 0, 'invalid continuation encoding')
            raw = c.exact_hex(encoded, len(encoded) // 2)
            value = json.loads(raw)
            c.require(c.canonical(value) == raw and isinstance(value, dict)
                      and set(value) == {'root', 'selection', 'limit', 'offset'}
                      and c.digest(b'dfmcp-bounded-run-page/1', raw).hex() == checksum,
                      'continuation integrity mismatch')
            c.integer(value['limit'], 1, 64)
            c.require(value['root'] == root and value['selection'] == selection and value['limit'] == limit,
                      'inventory continuation is stale or belongs to another query')
            offset = c.integer(value['offset'], 1, MAX_RECORDS)
            c.require(offset < len(rows), 'continuation is beyond the result horizon')
        page = rows[offset:offset + limit]
        next_token = None
        if offset + len(page) < len(rows):
            value = {'root': root, 'selection': selection, 'limit': limit, 'offset': offset + len(page)}
            raw = c.canonical(value)
            next_token = raw.hex() + '.' + c.digest(b'dfmcp-bounded-run-page/1', raw).hex()
        packet = {'ok': True, 'format': FORMAT, 'operation': 'inventory', 'inventory_digest': root,
                  'counts': self.counts(), 'rows': page, 'matched': len(rows), 'continuation': next_token,
                  'native_contacted': False, 'runtime_admitted': False, 'retry_permitted': False,
                  'current_pause_unproved': True, 'goal_completion_proved': False}
        bounded(packet)
        return packet

    def __exit__(self, error_type, error, tb):
        try:
            if error_type is None:
                self.check()
        finally:
            self.stack.__exit__(error_type, error, tb)


def reconcile(directory: Path, expected: str, maximum: int = 8, timeout_ms: int = 10000,
              *, factory=QueryOnlyClient, clock=time.monotonic) -> dict:
    c.exact_hex(expected, 32)
    c.integer(maximum, 1, MAX_QUERIES); c.integer(timeout_ms, 1, 60000)
    # A small cooperative finalization reserve is not a hard filesystem deadline.
    deadline = clock() + timeout_ms / 1000 - min(0.05, timeout_ms / 5000)
    initial_settings = settings()
    with Inventory(directory) as inventory:
        c.require(inventory.digest() == expected, 'recovery inventory changed; inspect it again')
        rows = [row for row in inventory.rows() if row['query_required']]
        chosen = rows[:maximum]
        address, token = initial_settings
        for row in chosen:
            c.require(inventory.stores[row['name']].intent['endpoint'] == f'{address[0]}:{address[1]}',
                      'selected intent endpoint differs from the recovery endpoint')
        # Fixed bounded summaries only: reserve a larger-than-eligible row for
        # every attempted/deferred record and all complete inventory metadata.
        bounded({'rows': ['x' * 2048 for _ in chosen], 'metadata': 'x' * 8192})
        processed, failure, failed_name = [], None, None
        try:
            if chosen:
                c.require(clock() < deadline and settings() == initial_settings, 'recovery allowance or authority changed')
                with factory(address, token, deadline) as client:
                    for row in chosen:
                        failed_name = row['name']
                        c.require(clock() < deadline and settings() == initial_settings, 'recovery allowance or authority changed')
                        store = inventory.stores[row['name']]
                        store.check()
                        native = client.call('QueryRun', c.intent_fields(store.intent))
                        c.match_record(native, store.intent)
                        view = store.retain(native)
                        processed.append({'name': row['name'], 'key': row['key'],
                                          'effect_status': view['effect_status'],
                                          'native_record_status': view['native_record_status'],
                                          'terminal_record_retained': view['terminal_record_retained'],
                                          'storage_acknowledged_this_call': view['storage_acknowledged_this_call']})
                        failed_name = None
        except (OSError, ValueError, TypeError, KeyError, RecursionError):
            failure = 'native_or_custody_or_budget_refusal'
        inventory.check()  # Never publish a current inventory over corrupt custody.
        done = {row['name'] for row in processed}
        packet = {'ok': failure is None, 'format': FORMAT, 'operation': 'reconcile',
                  'inventory_before': expected, 'inventory_after': inventory.digest(), 'counts': inventory.counts(),
                  'processed': processed, 'failed_name': failed_name, 'failure': failure,
                  'deferred': [row['name'] for row in chosen if row['name'] not in done],
                  'not_selected_count': len(rows) - len(chosen), 'runtime_admitted': False,
                  'retry_permitted': False, 'current_pause_unproved': True, 'goal_completion_proved': False}
        bounded(packet)
        return packet


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', choices=('inventory', 'reconcile'))
    parser.add_argument('--directory', type=Path, required=True)
    parser.add_argument('--selection', choices=('all', 'pending', 'terminal'), default='all')
    parser.add_argument('--limit', type=int, default=16)
    parser.add_argument('--continuation')
    parser.add_argument('--expected-inventory')
    parser.add_argument('--max-queries', type=int, default=8)
    parser.add_argument('--timeout-ms', type=int, default=10000)
    args = parser.parse_args(argv)
    try:
        if args.operation == 'inventory':
            c.require(args.expected_inventory is None and args.max_queries == 8 and args.timeout_ms == 10000,
                      'live recovery options supplied to offline inventory')
            with Inventory(args.directory) as inventory:
                packet = inventory.page(args.selection, args.limit, args.continuation)
        else:
            c.require(args.selection == 'all' and args.limit == 16 and args.continuation is None,
                      'pagination options supplied to recovery pass')
            packet = reconcile(args.directory, args.expected_inventory, args.max_queries, args.timeout_ms)
        print(bounded(packet).decode())
        return 0 if packet['ok'] else 2
    except (OSError, ValueError, TypeError, KeyError, RecursionError):
        print(c.canonical({'ok': False, 'format': FORMAT, 'error': 'recovery_refused',
                           'effect_status': 'unknown', 'retry_permitted': False,
                           'retained_progress_may_exist': True, 'runtime_admitted': False}).decode())
        return 2


if __name__ == '__main__':
    raise SystemExit(main())
