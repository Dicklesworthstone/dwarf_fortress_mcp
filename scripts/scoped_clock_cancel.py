"""One explicit cancellation of an original construction-clock intent.

Reuse run/1.13 framing/codecs, but never observe, query, prepare, commit, reconnect
or retry. Query-only Client and the original start owners remain unchanged.
Beads: df-dfhack-bridge-plane-c-pic.4/.5.
"""
from __future__ import annotations

import bounded_run_client as clock


class CancelClient(clock.Client):
    def __init__(self, authority, budget, guard, intent):
        self.authority, self.budget, self.guard = authority, budget, guard
        self.intent = clock.verify_intent(dict(intent))
        self.original = clock.snapshot(bytes.fromhex(self.intent['observation_hex']))
        self.operation_deadline = budget.deadline
        self.cancel_attempted = False
        self._check()
        clock.require(authority.address == clock.endpoint(self.intent['endpoint']),
                      'cancellation endpoint differs from original intent')
        timeout = min(60000, int(budget.remaining() * 1000))
        clock.integer(timeout, 1, 60000)
        super().__init__(authority.address, authority.run_token, timeout)
        self.deadline = min(self.deadline, self.operation_deadline)

    def _check(self):
        self.authority.clock_guard()
        clock.require(self.budget.deadline == self.operation_deadline, 'cancellation deadline changed')
        self.budget.remaining()
        self.guard()

    def remaining(self):
        self._check()
        return min(super().remaining(), self.budget.remaining())

    def send(self, data):
        self._check()
        self.budget.reserve('network_bytes', len(data))
        super().send(data)

    def read(self, size):
        self._check()
        clock.integer(size, 0, 65536)
        self.budget.reserve('network_bytes', size)
        return super().read(size)

    def frame(self, method, request):
        self._check()
        self.budget.reserve('calls', 1)
        return super().frame(method, request)

    def call(self, operation, fields=None):
        try:
            self._check()
            clock.require(operation in ('Handshake', 'CancelRun'), 'cancellation-only transport')
            if operation == 'CancelRun':
                clock.require(not self.cancel_attempted and fields == clock.intent_fields(self.intent, True),
                              'cancellation already attempted or original identity changed')
                # Consume before the first potentially effectful frame. A lost
                # reply cannot enable a second attempt on this connection.
                self.cancel_attempted = True
            else:
                clock.require(fields is None and self.manifest is None,
                              'only the initial cancellation handshake is allowed')
            result = super().call(operation, fields)
            clock.match_record(result, self.intent)
            clock.require(result['manifest']['generation'] == self.original['generation'],
                          'original clock incarnation changed; use query-only recovery')
            if operation == 'CancelRun':
                clock.require(result.get('record', {}).get('phase') in
                              ('stopping', 'stopped', 'refused', 'source_lost'),
                              'cancellation returned contradictory native evidence')
            self._check()
            return result
        except BaseException:
            self.close()
            raise
