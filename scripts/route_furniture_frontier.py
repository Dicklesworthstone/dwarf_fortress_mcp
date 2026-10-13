"""Original route constraints at a receipt-verified furniture execution frontier.

Only an exact fixed-order prefix of historical Placed receipts removes work
from fresh loose-item checks. Unknown, Prepared, refused and cancelled work
cannot be skipped. This pure checker grants no I/O or placement authority.
Beads: df-dfhack-bridge-plane-c-pic.4/.5; df-franken-storage-mvcc-54h.4.
"""
from __future__ import annotations

import hashlib

from build_placement_wire import Capture, Record, Selection, KINDS, MAX_RECORD_BYTES
from furniture_allocation import Candidate, distance, idle
from furniture_plan import canonical, label, require
from route_furniture_review import Review, Source, candidate, digest_text, revalidate
from route_furniture_allocation import Work, MAX_WORK

POLICY = 'original-route-constraints-after-verified-placement-prefix/1'
MAX_OUTPUT = 96 * 1024


def step_key(batch_id: str, name: str) -> str:
    digest_text(batch_id)
    label(name)
    return 'frt-' + batch_id + '-' + name


def _bind(original, request, source, plan, before: Capture) -> str:
    require(type(before) is Capture, 'exact native placement capture required')
    before.encode()
    step = next((s for s in plan.steps if before.selection ==
                 Selection(KINDS.index(s.kind), s.item, *s.target)), None)
    require(step is not None and before.item.presence == 2,
            'placement differs from the original route selection')
    row = next(r for r in original['selections'] if r['slot'] == step.name)
    saved, item = candidate(row['candidate']), before.item
    current = Candidate(step.item, KINDS[item.kind], item.pos,
                        (item.material, item.material_index), item.subtype)
    slot = next(s for s in request.slots if s.name == step.name)
    require((before.folder, before.site, before.dimensions) ==
            (request.folder, request.site, source.dimensions), 'placement source differs from original routes')
    require(before.tick >= source.tick and before.next_job >= source.horizons[0]
            and before.next_building >= source.horizons[1], 'placement predates original inventory')
    require((current.id, current.kind, current.material, current.subtype, item.native_type) ==
            (saved.id, saved.kind, saved.material, saved.subtype, row['native_type'])
            and distance(slot, current) is not None, 'placement violates original item constraints')
    return step.name


def bind_native_selection(review: Review, before: Capture, guard=idle) -> str:
    """Check original identity/material/geometry for a retained or fresh BEFORE.

    This is deliberately not a route check: the execution owner must separately
    acquire the complete current map/inventory and evaluate the full frontier.
    """
    require(type(review) is Review, 'exact original route review required')
    original, request, source, plan = review.unpack(guard)
    out = _bind(original, request, source, plan, before)
    guard()
    return out


def assess(review: Review, batch_id: str, placed: tuple[bytes, ...], before: Capture,
           candidates: tuple[Candidate, ...], native_types: dict[int, int],
           terrain_raw: bytes, source: Source, guard=idle, *, maximum_work: int = MAX_WORK) -> dict:
    """Check every remaining original selection against one fresh read bracket.

    The trusted owner supplies its complete audited prefix, never client-picked
    completion names. Receipt digests are evidence identities, not signatures.
    Native preparation/commit still require the actual coordinator and grants.
    """
    work = Work(maximum_work, guard)
    require(type(review) is Review and type(source) is Source, 'invalid route frontier inputs')
    digest_text(batch_id)
    original, request, baseline, plan = review.unpack(work)
    baseline.permits(source)
    require(type(placed) is tuple and len(placed) < len(plan.ordered),
            'route frontier needs an unfinished fixed-order prefix')
    require(type(before) is Capture and before.eligible, 'ineligible native placement frontier')
    next_step = plan.ordered[len(placed)]
    require(_bind(original, request, baseline, plan, before) == next_step.name,
            'native placement is not the next original step')
    require((before.tick, before.next_job, before.next_building, before.dimensions) ==
            (source.tick, source.horizons[0], source.horizons[1], source.dimensions),
            'native placement and route inventory do not share the same frontier')
    retained, previous = [], None
    for step, raw in zip(plan.ordered, placed):
        work()
        require(type(raw) is bytes and 0 < len(raw) <= MAX_RECORD_BYTES, 'invalid prefix receipt bytes')
        record = Record.decode(raw)
        old = record.plan.before
        require(record.phase == 'placed' and record.plan.key == step_key(batch_id, step.name)
                and _bind(original, request, baseline, plan, old) == step.name,
                'only exact original Placed receipts can advance route work')
        require(old.generation == before.generation, 'prefix native incarnation differs')
        if previous is not None:
            require(old.tick >= previous.tick and old.sequence >= previous.sequence
                    and old.next_job >= previous.next_job and old.next_building >= previous.next_building,
                    'prefix native frontier regressed')
        previous = record.after
        require(before.tick >= previous.tick and before.sequence >= previous.sequence
                and before.next_job >= previous.next_job and before.next_building >= previous.next_building,
                'fresh route frontier predates a placed predecessor')
        retained.append({'slot': step.name, 'key': record.plan.key,
                         'receipt_digest': raw[-32:].hex(), 'native_plan_digest': record.plan.digest.hex()})
    # The original checker still validates the COMPLETE supplied inventory and
    # all original source constraints. Its separate work counter consumes only
    # the explicitly reserved remainder, never a replenished allowance.
    finishing = len(plan.steps) + 2
    allowance = maximum_work - work.used - finishing
    require(allowance > 0, 'route frontier has no remaining analysis allowance')
    result = revalidate(review, candidates, native_types, terrain_raw, source, work.check,
                        maximum_work=allowance)
    work.used += result['work_units']
    history = {row['slot']: row for row in retained}
    rows = []
    for row in result['rows']:
        work()
        if row['slot'] in history:
            # A historical insertion is not a current loose-item fact or proof
            # of physical construction completion. Do not carry either claim.
            row = {'slot': row['slot'], 'item': row['item'], 'target': row['target'],
                   'status': 'historical_placed', 'reason': 'exact_original_placed_receipt',
                   'receipt_digest': history[row['slot']]['receipt_digest'],
                   'current_condition_proven': False, 'construction_completion_proven': False}
        rows.append(row)
    next_row = next(row for row in rows if row['slot'] == next_step.name)
    if next_row['status'] == 'constraints_met':
        require(next_row['current_position'] == list(before.item.pos),
                'selected native item moved between placement and route captures')
    active = [row for row in rows if row['status'] != 'historical_placed']
    status = ('ready' if all(row['status'] == 'constraints_met' for row in active) else
              'route_blocked' if any(row['status'] == 'constraints_not_met' for row in active)
              else 'route_unestablished')
    work()
    out = {'schema': 'dfmcp.route-furniture-frontier/1', 'policy': POLICY, 'status': status,
           'batch_id': batch_id, 'review_digest': review.digest, 'original_plan_digest': plan.digest,
           'original_plan': plan.json(), 'next_step': next_step.name,
           'native_before_digest': before.witness.hex(), 'source': source.json(),
           'placed_prefix': retained, 'rows': rows, 'remaining_steps': len(active),
           'max_route_steps': original['max_route_steps'],
           'all_remaining_selections_checked': True, 'reallocation_performed': False,
           'placement_authorized': False, 'native_pathfinding_proven': False,
           'atomic_route_placement_proven': False, 'future_routes_preserved': False,
           'construction_completion_proven': False, 'retry_permitted': False,
           'work_units': work.used + 1, 'maximum_work': maximum_work}
    # The review seal may bind the evidence without depending on the shrinking
    # computational allowance of a later execution-phase check.
    evidence = {k: v for k, v in out.items() if k not in ('work_units', 'maximum_work')}
    out['evidence_digest'] = hashlib.sha256(b'dfmcp-route-furniture-frontier/1\0' + canonical(evidence)).hexdigest()
    require(len(canonical(out)) <= MAX_OUTPUT, 'complete route frontier exceeds output bound')
    work()
    work.check()
    return out
