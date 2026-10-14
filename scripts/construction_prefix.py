"""Current receipt-linked construction quiescence for a bounded selected prefix.

This is a stateless read predicate, not a goal monitor or a receipt-custody owner.
The execution owner must supply and authenticate the COMPLETE original prefix,
bind the exact native source, and keep current authority through publication.
No deadline, stability streak, dispatch, cancellation or retry is created here.
Beads: df-dfhack-bridge-plane-c-pic.4/.5.
"""
from __future__ import annotations

from dataclasses import asdict
import hashlib
from typing import Callable

from build_placement_wire import MAX_RECORD_BYTES, Record, canonical, integer, require
from construction_receipt import MAX_CAPTURE, assess_placed_receipt, decode_operations

POLICY = 'original-installed-furniture-and-no-held-or-item-linked-jobs/1'
MAX_PREFIX = 32
MAX_WORK = 20_000_000
MAX_OUTPUT = 64 * 1024
Guard = Callable[[], None]


class Work:
    def __init__(self, maximum: int, guard: Guard):
        self.maximum = integer(maximum, 1, MAX_WORK)
        self.used, self.guard = 0, guard
        guard()

    def __call__(self) -> None:
        require(self.used < self.maximum, 'construction prefix work allowance exhausted')
        self.used += 1
        if self.used == 1 or self.used % 128 == 0:
            self.guard()

    def check(self) -> None:
        self.guard()


def assess_prefix(placed: tuple[bytes, ...], operations_raw: bytes, guard: Guard,
                  *, maximum_work: int = MAX_WORK) -> dict:
    """Decode one complete roster, then check ALL selected Placed receipts.

    Job registration, disappearing jobs, an installed-item flag or a matching
    numeric building ID alone is insufficient. Even after the shared historical
    construction condition passes, the original item must occupy its target and
    its building must have NO held job (including unknown job kinds).

    Empty input has no predecessor gate, not proof that a fortress has no work.
    The result covers only the supplied selection; its completeness as an
    execution prefix is established separately by the original batch owner.
    """
    work = Work(maximum_work, guard)
    require(type(placed) is tuple and len(placed) <= MAX_PREFIX, 'invalid construction prefix')
    require(type(operations_raw) is bytes and 0 < len(operations_raw) <= MAX_CAPTURE,
            'invalid complete construction inventory')
    records, identities, prior = [], [set() for _ in range(5)], None
    for raw in placed:
        work()
        require(type(raw) is bytes and 0 < len(raw) <= MAX_RECORD_BYTES, 'invalid original receipt')
        record = Record.decode(raw)
        require(record.phase == 'placed', 'only exact Placed receipts select predecessor construction')
        before, insertion = record.plan.before, record.insertion
        values = (record.plan.key, insertion.building, insertion.job, insertion.item, insertion.pos)
        for seen, value in zip(identities, values):
            work()
            require(value not in seen, 'duplicate original construction identity')
            seen.add(value)
        if prior is not None:
            require(before.identity == prior.identity and before.tick >= prior.tick
                    and before.sequence >= prior.sequence and before.next_job >= prior.next_job
                    and before.next_building >= prior.next_building,
                    'construction prefix source or chronology changed')
        prior = record.after
        records.append(record)
    observed = decode_operations(operations_raw, work)
    require(observed.paused, 'construction prefix requires one paused capture')
    held = {}
    for job in observed.jobs.values():
        work()
        if job.holder is not None:
            held[job.holder] = held.get(job.holder, 0) + 1
    rows = []
    for raw, record in zip(placed, records):
        work()
        finding = assess_placed_receipt(raw, observed, work)
        p = record.insertion
        status, reason = 'unestablished', finding.status
        if finding.status == 'condition_met':
            if observed.items[p.item].position != p.pos:
                reason = 'installed_item_position_mismatch'
            elif held.get(p.building, 0):
                status, reason = 'pending', 'other_held_job'
            else:
                status, reason = 'quiescent', 'original_furniture_installed_without_work'
        elif finding.status in ('pending', 'suspended', 'no_construction_job'):
            status = 'pending'
        elif finding.status == 'removal_pending':
            status = 'blocked'
        rows.append({'key': record.plan.key, 'receipt_digest': raw[-32:].hex(),
                     'native_plan_digest': record.plan.digest.hex(),
                     'building': p.building, 'item': p.item, 'original_job': p.job,
                     'target': list(p.pos), 'status': status, 'reason': reason,
                     'held_jobs': held.get(p.building, 0), 'assessment': asdict(finding)})
    counts = {name: sum(row['status'] == name for row in rows)
              for name in ('quiescent', 'pending', 'blocked', 'unestablished')}
    status = ('no_predecessors' if not rows else 'quiescent' if counts['quiescent'] == len(rows)
              else 'blocked' if counts['blocked'] else 'unestablished' if counts['unestablished']
              else 'pending')
    work()
    out = {'schema': 'dfmcp.construction-prefix/1', 'policy': POLICY, 'status': status,
           'capture_sha256': observed.digest, 'capture_bytes': len(operations_raw),
           'world_folder': observed.folder, 'site': observed.site, 'game_tick': observed.tick,
           'paused': observed.paused, 'horizons': list(observed.horizons),
           'placement_generation': None if not records else records[0].plan.before.generation,
           'selected_count': len(rows), 'counts': counts, 'rows': rows,
           'predecessor_gate_met': status in ('no_predecessors', 'quiescent'),
           'complete_roster_decoded': True, 'original_prefix_completeness_authenticated': False,
           'native_acquisition_proven': False, 'continuous_stability_proven': False,
           'goal_completion_proven': False, 'placement_effect_discharged': False,
           'global_quiescence_proven': False, 'placement_authorized': False,
           'retry_permitted': False, 'production_admitted': False}
    out['evidence_digest'] = hashlib.sha256(
        b'dfmcp-construction-prefix/1\0' + canonical(out)).hexdigest()
    out.update(work_units=work.used, maximum_work=maximum_work)
    require(len(canonical(out)) <= MAX_OUTPUT, 'complete construction prefix output exceeds bound')
    work.check()
    return out
