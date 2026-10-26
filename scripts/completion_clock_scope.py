"""Distinct immutable consent for the next original construction stability sample.

This is an evidence association, not a monitor, acquisition proof or authority.
The execution owner must keep the original monitor and batch locked and recheck
this exact journal head before either native effect. Existing scopes do not gain
permission to advance quiescent construction merely by changing a field.
"""
from __future__ import annotations

from dataclasses import dataclass

import bounded_run_client as clock
import bounded_run_outcomes as storage
from completion_clock_plan import Window, next_sample
from furniture_plan import canonical, decode_json, require
from route_furniture_clock_context import FinalScope, FINAL_SCOPE_KEYS, FINAL_SCHEMA, FINAL_POLICY

SCHEMA = 'dfmcp.route-furniture-completion-clock/1'
POLICY = 'original-goal-and-journal-bound-next-stability-sample/1'
KEYS = FINAL_SCOPE_KEYS | {'completion', 'max_ticks'}


@dataclass(frozen=True)
class CompletionScope(FinalScope):
    @property
    def digest(self):
        return clock.digest(b'dfmcp-completion-clock-confirmation/1', self.raw).hex()

    @property
    def clock_key(self):
        return 'frv-' + self.digest

    def value(self):
        require(type(self.raw) is bytes and 0 < len(self.raw) < storage.MAX_FILE,
                'completion clock scope exceeds bound')
        value = decode_json(self.raw)
        require(type(value) is dict and set(value) == KEYS and canonical(value) == self.raw
                and value['schema'] == SCHEMA and value['policy'] == POLICY
                and value['phase'] == 'completion_verification',
                'invalid closed completion clock scope')
        # Validate the unchanged clock/source/batch fields with the existing
        # validator. Only this temporary validation value is adapted; original
        # bytes, identities and saved confirmations are never rewritten.
        base = {key: value[key] for key in FINAL_SCOPE_KEYS}
        base.update(schema=FINAL_SCHEMA, policy=FINAL_POLICY, phase='final_construction')
        FinalScope.create(base).value()
        window = Window.from_json(value['completion'])
        decision = next_sample(window, value['source']['tick'], max_ticks=value['max_ticks'])
        require(decision['status'] == 'advance_for_sample'
                and decision['game_ticks'] == value['game_ticks']
                and value['source']['inventory_sha256'] == window.last_capture,
                'clock interval differs from the original eligible completion sample')
        return value
