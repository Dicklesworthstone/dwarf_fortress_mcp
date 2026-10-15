"""Fresh whole-room truth, separate from the retained monitor's terminal history.

The owning shell supplies a complete acquired Sample and replays the original
Goal/Progress under file custody. This pure query never advances that progress,
changes its deadline, clears a lost read, or reconciles a placement effect.
Beads: df-dfhack-bridge-plane-c-pic.3/.4/.5; WP-05/07/10.
"""
from __future__ import annotations

import hashlib
import json
from typing import Callable

MAX_OUTPUT = 64 * 1024
POLICY = 'dfmcp.original-room-current-condition/1'
IDENTITY_FAILURES = frozenset({
    'world_identity_mismatch', 'source_regressed', 'building_missing', 'item_missing',
    'building_identity_mismatch', 'item_identity_mismatch', 'original_job_identity_mismatch',
})
KNOWN_PENDING = frozenset({'pending', 'suspended', 'no_construction_job', 'removal_pending'})


def _require(value: bool, message: str) -> None:
    if not value:
        raise ValueError(message)


def _canonical(value: object) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(',', ':'),
                      ensure_ascii=True, allow_nan=False).encode('ascii')


def _all(values) -> str:
    values = tuple(values)
    _require(bool(values) and all(v in ('true', 'false', 'unknown') for v in values),
             'incomplete or invalid current-condition truth')
    return 'false' if 'false' in values else 'unknown' if 'unknown' in values else 'true'


def _terrain_truth(diagnosis: dict) -> str:
    values = []
    for domain in ('floors', 'required_walls'):
        counts = diagnosis['counts'][domain]
        # These categories are the existing complete terrain-goal diagnosis,
        # not counts reconstructed from its bounded deficit preview.
        _require(set(counts) == {'matched', 'wrong_shape', 'liquid', 'designated',
                                'occupied_wall', 'hidden', 'missing'},
                 'unknown room terrain diagnostic generation')
        _require(all(type(n) is int and n >= 0 for n in counts.values()),
                 'invalid room terrain diagnostic count')
        values.append('false' if any(counts[k] for k in
                      ('wrong_shape', 'liquid', 'designated', 'occupied_wall')) else
                      'unknown' if counts['hidden'] or counts['missing'] else 'true')
    return _all(values)


def inspect(goal, sample, *, baseline=None, guard: Callable[[], None] = lambda: None) -> dict:
    """Evaluate one complete sample, including after deadline or cancellation.

    Imports are local to keep the pure truth utilities usable without constructing
    any native client. The real Goal/Sample codecs and single-receipt evaluator
    remain authoritative. A caller-created projection is never accepted instead.
    Baseline is an owner-replayed Progress with the INNER goal digest; it carries
    historical source floors, not fresh authority or a reusable stability streak.
    """
    import construction_plan as construction
    import room_readiness as readiness

    guard()
    _require(type(goal) is readiness.Goal and type(sample) is readiness.Sample,
             'complete native room goal and sample required')
    goal = readiness.Goal.decode(goal.encode(), guard)
    if baseline is not None:
        _require(type(baseline) is readiness.Progress and baseline.goal_digest == goal.digest,
                 'current inspection baseline belongs to another original goal')
    captured, diagnosis = sample.validate(goal, guard)
    # Match the existing joint reducer's full decoding path. Do not run its
    # temporal advance with a fabricated deadline or a one-sample policy.
    observed = sample.furnishings.validate(goal.condition, guard)
    findings = []
    invalidators = []
    for child in goal.condition.goals:
        guard()
        record = child.record
        placement = record.insertion
        finding = construction.Assessment(placement.building, placement.item, placement.job,
            record.plan.key, child.receipt[-32:].hex(), construction.assess(child, observed, guard))
        findings.append(finding)
        if finding.condition.status in IDENTITY_FAILURES:
            invalidators.append({'reason': finding.condition.status, 'building_id': placement.building})
    _require(1 <= len(findings) <= 32, 'current inspection lost the original furnishing selection')
    source = (sample.furnishings.before.generation, sample.furnishings.operations.generation,
              *sample.furnishings.operations.software)
    map_source = _canonical(captured.binding())
    if baseline is not None:
        for changed, reason in (
            (baseline.phase == 'invalidated', 'original_monitor_identity_invalidated'),
            (baseline.source is not None and baseline.source != source, 'native_source_changed'),
            (baseline.map_source is not None and baseline.map_source != map_source, 'room_map_source_changed'),
            (baseline.last_tick is not None and observed.tick < baseline.last_tick, 'game_clock_regressed'),
            (baseline.horizons is not None and any(a < b for a, b in
                zip(observed.horizons, baseline.horizons)), 'native_horizon_regressed'),
        ):
            if changed:
                invalidators.append({'reason': reason, 'building_id': None})
        previous = {row.building_id: row.condition for row in baseline.assessments}
        for row in findings:
            guard()
            old, current = previous.get(row.building_id), row.condition
            if old is not None and old.building_type is not None and current.building_type != old.building_type:
                invalidators.append({'reason': 'native_building_type_changed', 'building_id': row.building_id})
            if old is not None and old.stage is not None and current.stage is not None and current.stage < old.stage:
                invalidators.append({'reason': 'construction_stage_regressed', 'building_id': row.building_id})
    occupancy = []
    ox, oy, oz = captured.region.origin
    sx, sy, sz = captured.region.size
    for step in goal.room.allocation.plan().steps:
        guard()
        x, y, z = step.target
        _require(ox <= x < ox + sx and oy <= y < oy + sy and oz <= z < oz + sz,
                 'original furnishing lies outside complete inspection capture')
        tile = captured.tiles[((z - oz) * sy + y - oy) * sx + x - ox]
        occupancy.append('unknown' if tile.presence != 2 else 'true' if tile.attributes[6] else 'false')
    domains = {
        'terrain': _terrain_truth(diagnosis),
        'furniture_occupancy': _all(occupancy),
        'furnishings': _all('true' if row.condition.status == 'condition_met' else
                           'false' if row.condition.status in KNOWN_PENDING else 'unknown'
                           for row in findings),
    }
    if goal.require_circulation:
        domains['circulation'] = diagnosis['circulation']['truth']
    raw_truth = _all(domains.values())
    truth = 'unknown' if invalidators else raw_truth
    status = ('invalidated_reference' if invalidators else 'condition_met' if truth == 'true' else
              'condition_not_met' if truth == 'false' else 'blocked_unknown')
    result = {
        'schema': 'dfmcp.room-current-condition/1', 'policy': POLICY,
        'goal_digest': goal.digest, 'room_plan_digest': goal.room.room_plan.digest,
        'status': status, 'condition_truth': truth, 'domain_truth': domains,
        'all_original_conditions_at_sample': status == 'condition_met',
        'game_tick': captured.tick, 'joint_sample_digest': sample.digest,
        'map_observation_witness': captured.witness, 'operations_capture_sha256': observed.digest,
        'source': {'placement_generation': source[0], 'operations_generation': source[1],
                   'map_generation': sample.map_before.generation},
        'historical_basis': None if baseline is None else {
            'phase': baseline.phase, 'game_tick': baseline.last_tick,
            'joint_sample_digest': baseline.sample_digest,
            'read_outcome_unknown': baseline.reading,
            'original_deadline_tick': goal.condition.deadline,
        },
        'invalidators': invalidators, 'selected_furnishings': len(findings),
        'assessments': [row.view() for row in findings], 'assessments_complete': True,
        'terrain': diagnosis, 'circulation_required': goal.require_circulation,
        'stability_evaluated': False, 'deadline_evaluated': False,
        'monitor_updated': False, 'durable_sample_published': False,
        'unresolved_read_cleared': False, 'placement_effect_reconciled': False,
        'room_completion_proven': False, 'native_pathfinding_proven': False,
        'atomic_cross_profile_snapshot_proven': False, 'continuous_presence_proven': False,
        'mutation_dispatched': False, 'native_acquisition_independently_attested': False,
        'interpretation': 'Whole original condition at this supplied sample only. Not historical '
                          'stability, current usability, continuous access or renewed effect authority.',
    }
    guard()
    result['evidence_digest'] = hashlib.sha256(b'dfmcp-room-current-condition/1\0' + _canonical(result)).hexdigest()
    _require(len(_canonical(result)) <= MAX_OUTPUT, 'complete current room diagnosis exceeds output bound')
    guard()
    return result
