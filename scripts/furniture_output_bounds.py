"""Serialization-only upper bounds for complete original furnishing results.

These deliberately impossible envelopes are private size probes, NEVER evidence,
plans, authorization or public results. Existing owners still produce and verify
all real records. Beads: df-dfhack-bridge-plane-c-pic.4/.5.
"""
from __future__ import annotations

from furniture_plan import MAX_STEPS, Step, canonical, require
from build_placement_wire import MAX_I32, MIN_I32, PHASES

MAX_BLOCKER_BYTES = 1024
PRESENTATION_MARGIN = 128


def inventory_bound(state: dict, selected: tuple[Step, ...]) -> dict:
    """Bound every next-child outcome while retaining exact immutable metadata.

    The caller already holds and authenticates the original inventory. All prior
    rows, original plan and optional room/terrain data remain byte-identical.
    Only the selected unstarted rows and finite progress fields can grow. False
    Boolean literals, signed 32-bit extrema, longest phases and 128-byte native
    keys intentionally overestimate actual exact-source Placed evidence.
    """
    require(type(state) is dict and type(selected) is tuple and 1 <= len(selected) <= 8
            and all(type(step) is Step for step in selected), 'invalid output sizing selection')
    rows = state['steps']
    require(type(rows) is list and 1 <= len(rows) <= MAX_STEPS, 'invalid sizing inventory')
    by_name = {step.name: step for step in selected}
    require(len(by_name) == len(selected), 'duplicate sizing selection')
    found, bounded = set(), []
    for row in rows:
        if row['name'] not in by_name:
            bounded.append(row)
            continue
        step = by_name[row['name']]
        require(row['phase'] == 'not_started', 'sizing cannot replace recorded evidence')
        found.add(step.name)
        bounded.append({'name': step.name, 'phase': max(PHASES, key=len),
            'key': 'k' * 128, 'plan_digest': '0' * 64, 'registered': False,
            'receipt_digest': '0' * 64, 'dispatch_recorded': False,
            'insertion': {'building_id': MAX_I32 - 1, 'job_id': MAX_I32 - 1,
                'item_id': step.item, 'kind': step.kind, 'pos': list(step.target),
                'material': MIN_I32, 'material_index': MIN_I32,
                'stage': 0, 'max_stage': 32, 'linked': False, 'construct_job': False,
                'exact_item_link': False, 'suspended': False}})
    require(found == set(by_name), 'sizing selection is outside original inventory')
    widest_name = max((row['name'] for row in rows), key=lambda name: len(canonical(name)))
    # A simultaneous pending/next frontier with maximum placed count is not a
    # valid result. It bounds widths only. Never run it through proof admission.
    return {**state, 'steps': bounded, 'status': 'pending_recovery',
            'next_step': widest_name, 'pending_step': widest_name, 'placed': MAX_STEPS,
            'advance_allowed': False, 'native_contacted': False}


def prefix_outcomes_bound(selected: list[dict]) -> list[dict]:
    require(type(selected) is list and 1 <= len(selected) <= 8, 'invalid prefix sizing selection')
    rows = [{'name': step['name'], 'key': step['key'], 'item': step['item'],
             'phase': max(PHASES, key=len), 'dispatch_recorded': False, 'blockers': []}
            for step in selected]
    # A blocker stops the whole prefix, so at most one outcome has a diagnostic
    # list. The runner checks the encoded list bound, including JSON escaping.
    rows[-1]['blockers'] = ['b' * (MAX_BLOCKER_BYTES - 4)]
    require(len(canonical(rows[-1]['blockers'])) == MAX_BLOCKER_BYTES, 'invalid blocker sizing bound')
    return rows
