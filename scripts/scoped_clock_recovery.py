"""Bounded query-only reconciliation of discovered original construction clocks.

One held original batch and clock-directory lock, one shrinking budget, at most
one QueryRun per selected original record. No preparation, commit, cancellation,
source adoption, intent repair, route refresh, or automatic retry. The unchanged
native OutcomeStore synchronizes terminal evidence before the next record.
Beads: df-dfhack-bridge-plane-c-pic.4/.5.
"""
from __future__ import annotations

from dataclasses import dataclass, field
import os

import bounded_run_client as clock
import bounded_run_outcomes as storage
from furniture_plan import canonical, integer, require
from route_furniture_clock_context import Client, ScopeStore, SCOPE_SUFFIX
from route_furniture_clock_inventory import Inventory, STORE_READ_RESERVATION
from route_furniture_review import digest_text

SCHEMA = 'dfmcp.scoped-clock-reconciliation/1'
POLICY = 'one-query-per-original-unknown-clock-in-selected-inventory/1'
MAX_QUERIES = 16
MAX_OUTPUT = 64 * 1024
QUERY_CALLS = 8  # Six fixed bindings, Handshake, QueryRun; never a write call.
QUERY_BYTES = 3 * 1024 * 1024
QUERY_DISK = 1024 * 1024
FINAL_MARGIN = 0.25
ENVIRONMENT = ('DFMCP_ALLOW_UNADMITTED_RUN_V1_13', 'DFMCP_RUN_ENDPOINT', 'DFMCP_RUN_TOKEN')


@dataclass(frozen=True)
class QueryAuthority:
    """Run-only recovery configuration; deliberately needs no route credentials."""
    address: tuple[str, int]
    run_token: bytes = field(repr=False)

    @classmethod
    def load(cls):
        require(all(not name.startswith('DFMCP_') or name in ENVIRONMENT for name in os.environ),
                'clock recovery requires its isolated run-only environment')
        require(os.environ.get(ENVIRONMENT[0]) == '1', 'explicit clock recovery opt-in required')
        raw = os.environ.get(ENVIRONMENT[1], '127.0.0.1:5000')
        address = clock.endpoint(raw)
        require(raw == f'{address[0]}:{address[1]}', 'canonical clock recovery endpoint required')
        token = os.environ.get(ENVIRONMENT[2], '').encode('utf-8')
        require(32 <= len(token) <= 256 and b'\0' not in token, 'invalid clock recovery credential')
        return cls(address, token)

    def clock_guard(self):
        require(QueryAuthority.load() == self, 'clock recovery authority changed')


class Recovery(Inventory):
    """Inventory owner allowing only verified missing terminal-outcome publication.

    Existing bytes remain pinned. A successful retain registers its one new
    outcome witness; a failed/ambiguous retain fences this owner instead of
    adopting arbitrary files or repairing history. Keep this owner open through
    final output. Base Inventory discovery and new-effect admission are unchanged.
    """
    def __init__(self, directory, batch):
        require(not batch.writable, 'clock recovery requires read-only original batch custody')
        self.fenced, self.used = False, False
        super().__init__(directory, batch)

    def _guard(self):
        require(not self.fenced, 'ambiguous clock recovery publication requires reopening')
        super()._guard()

    def admit_new(self, scope):
        raise ValueError('recovery inventory cannot admit a new clock effect')

    def _retain(self, identity, binding, store, result):
        """Accept one absent outcome only after the unchanged codec/store verifies it."""
        self.check()
        binding.check()
        require(store.load() is None and store.intent == binding.scope.intent(),
                'recovery cannot replace a retained outcome or original intent')
        name = store.path.name + storage.SUFFIX
        require(store.path.name == 'progress-' + identity + '.run'
                and name not in self._files and not self._allowed_new,
                'unexpected recovery outcome identity')
        # Reserve internal OutcomeStore traversals, write and sync-readback work.
        self.budget.reserve('disk_bytes', STORE_READ_RESERVATION)
        try:
            view = store.retain(result)
            if store.outcome is not None:
                # Permit precisely this new path while every OLD witness is checked.
                self._allowed_new = frozenset((name,))
                self.check()
                raw, witness = self._read(name)
                require(raw == store.outcome_data and witness.stamp == store.outcome_stat,
                        'retained recovery outcome changed before inventory publication')
                self._files[name] = witness
                self._allowed_new = frozenset()
                record = view['record']
                self._rows = tuple(dict(row, native_phase=record['phase'],
                    status='source_lost' if record['phase'] == 'source_lost' else 'terminal_record_verified',
                    resolved=record['phase'] in ('stopped', 'refused'), query_supported=False,
                    receipt_digest=record['receipt_digest_hex'])
                    if row['clock_record_id'] == identity else row for row in self._rows)
            binding.check()
            self.check()
            return view
        except BaseException:
            self.fenced = True
            raise

    def _report(self, initial, selected, outcomes, stop_reason, *, faulted=False):
        self.check()
        counts = self.counts()
        result = {'schema': SCHEMA, 'policy': POLICY, 'batch_id': self.batch.id,
            'initial_inventory_digest': initial, 'inventory_digest': self.digest,
            'counts': counts, 'selected_count': selected, 'attempted_count': len(outcomes),
            'terminal_records_retained': sum(row['outcome_retained'] for row in outcomes),
            'outcomes': [dict(row) for row in outcomes], 'stop_reason': stop_reason,
            'faulted': faulted, 'directory_history_reconciled': counts['unresolved'] == 0,
            'complete_directory_validated': True, 'other_batch_details_omitted': True,
            'native_contact_attempted': bool(outcomes), 'clock_start_authorized': False,
            'current_pause_unproved': True, 'construction_completion_proven': False,
            'placement_effects_discharged': False, 'retry_permitted': False,
            'automatic_recovery_retry': False, 'production_admitted': False}
        require(len(canonical(result)) <= MAX_OUTPUT, 'complete clock recovery report exceeds bound')
        return result

    def reconcile(self, expected: str, authority: QueryAuthority | None, *, maximum=8,
                  render=lambda report: None):
        """Query up to 16 discovered original records once each, never effects.

        `expected` is the independently retained complete inventory digest, not a
        page cursor or a newly generated consent value. `render` is an embedding
        output-size reservation, not a stdout writer. All old files are checked
        before/after each query and retention. A transport failure stops the pass;
        custody/authority/publication failures may refuse the entire report while
        preserving previously synchronized original terminal evidence on disk.
        """
        digest_text(expected)
        integer(maximum, 1, MAX_QUERIES)
        require(not self.used, 'recovery owner is single-use')
        self.check()
        require(self.digest == expected, 'clock inventory changed; inspect the original history again')
        self.used = True
        targets = [dict(row) for row in self._rows if row['query_supported']][:maximum]
        if targets:
            require(type(authority) is QueryAuthority and authority.address == self.batch.address,
                    'original run-only recovery authority required')
        require(authority is None or type(authority) is QueryAuthority, 'invalid recovery authority')

        def guard():
            if authority is not None:
                authority.clock_guard()
            self.check()

        guard()
        # A fixed width reserve covers every selected compact outcome plus all
        # count/digest/stop metadata before opening any native connection.
        reservation = self._report(expected, len(targets), [], 'output_reserved')
        reservation['reserved_outcomes'] = 'x' * (4096 + 1024 * len(targets))
        require(len(canonical(reservation)) <= MAX_OUTPUT, 'recovery reservation exceeds bound')
        render(reservation)
        guard()
        outcomes, faulted = [], False
        stop = 'query_limit' if self.counts()['query_supported'] > len(targets) else 'selected_queries_finished'
        if not targets:
            stop = 'no_queryable_original_records'
        for row in targets:
            guard()
            if (self.budget.calls < QUERY_CALLS or self.budget.network_bytes < QUERY_BYTES
                    or self.budget.disk_bytes < QUERY_DISK or self.budget.remaining() <= FINAL_MARGIN):
                stop = 'aggregate_budget_boundary'
                break
            identity = row['clock_record_id']
            path = self.directory / ('progress-' + identity + '.run')
            self.budget.reserve('disk_bytes', STORE_READ_RESERVATION)
            with storage.OutcomeStore(path, parent_fd=self.parent) as store:
                with ScopeStore(path, self.parent, self.batch, identity) as binding:
                    require(store.intent == binding.scope.intent() and store.load() is None,
                            'selected original recovery intent changed')
                    require(store.source_stat == self._files[path.name].stamp
                            and binding.stamp == self._files[path.name + SCOPE_SUFFIX].stamp,
                            'selected original recovery file was replaced')
                    guard()
                    entry = {'clock_record_id': identity, 'clock_key': binding.scope.clock_key,
                             'native_phase': None, 'receipt_digest': None,
                             'outcome_retained': False, 'resolved': False, 'status': 'query_failed'}
                    outcomes.append(entry)

                    def query_guard():
                        # Bound complete directory scans to operation boundaries;
                        # exact selected intent/scope and original batch stay checked
                        # on each native frame/byte/deadline boundary by Client.
                        self._guard()
                        binding.check()
                        self.budget.reserve('disk_bytes', STORE_READ_RESERVATION)
                        store.check()

                    try:
                        with Client(authority, self.budget, query_guard,
                                    intent=store.intent, query=True) as client:
                            result = client.call('QueryRun', clock.intent_fields(store.intent))
                    except (OSError, ValueError, TypeError, KeyError, RecursionError, KeyboardInterrupt):
                        guard()
                        stop, faulted = 'query_failed', True
                        break
                    guard()
                    retained = self._retain(identity, binding, store, result)
                    record = retained.get('record', {})
                    phase = record.get('phase')
                    entry.update(native_phase=phase, receipt_digest=record.get('receipt_digest_hex'),
                        outcome_retained=store.outcome is not None,
                        resolved=phase in ('stopped', 'refused'),
                        status=('source_lost' if phase == 'source_lost' else
                                'terminal_record_verified' if store.outcome is not None else
                                'native_outcome_unknown'))
                    guard()
        guard()
        result = self._report(expected, len(targets), outcomes, stop, faulted=faulted)
        render(result)
        guard()
        return result
