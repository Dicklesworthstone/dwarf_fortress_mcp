"""Whole original route-batch construction goals, never placement authority.

Reuse the receipt-linked whole-plan condition and its single stability window.
A goal preserves ALL original exact-item steps and their original placed receipts;
imported bytes alone do not attest private batch custody or native acquisition.
Beads: df-dfhack-bridge-plane-c-pic.4/.5; WP-10.
"""
from __future__ import annotations

from dataclasses import dataclass, replace
import hashlib
from pathlib import Path

from build_placement_wire import KINDS, exact_hex, field, integer, require, text_bytes
from construction_receipt import Cursor, Guard, Manifest
import construction_plan as condition
from furniture_plan import FurniturePlan, MAX_BYTES as MAX_PLAN_BYTES, canonical

MAGIC = b'DFMRGC01'
MAX_GOAL = condition.MAX_GOAL + MAX_PLAN_BYTES + 8192
POLICY = 'all-original-route-batch-furnishings-sampled-complete/1'


def idle() -> None:
    """Pure decoding retains structural limits without an external deadline."""


def origin_path(value: str) -> str:
    text_bytes(value, 4096)
    path = Path(value)
    require(path.is_absolute() and str(path) == value and '..' not in path.parts
            and len(path.parts) > 1, 'original batch path must be absolute and canonical')
    return value


@dataclass(frozen=True)
class Goal:
    origin: str
    batch_id: str
    review_digest: str
    plan: FurniturePlan
    build: Manifest
    operations_generation: int
    condition: condition.Goal

    def validate(self, guard: Guard = idle) -> None:
        guard()
        origin_path(self.origin)
        exact_hex(self.batch_id, 32)
        exact_hex(self.review_digest, 32)
        require(type(self.plan) is FurniturePlan and type(self.condition) is condition.Goal
                and type(self.build) is Manifest, 'invalid original construction goal types')
        self.build.encode()
        integer(self.operations_generation, 1, 2**64 - 2)
        require(1 <= len(self.plan.steps) <= 32 and len(self.condition.receipts) == len(self.plan.steps),
                'construction goal must retain every original step')
        # Validate bounded original plan bytes before trusting its cached DAG.
        raw_plan = canonical(self.plan.json())
        require(len(raw_plan) <= MAX_PLAN_BYTES
                and FurniturePlan.decode(raw_plan) == self.plan, 'original plan is not canonical')
        recovered = condition.Goal.decode(self.condition.encode())
        records = {child.record.plan.key: child.record for child in recovered.goals}
        previous = None
        for step in self.plan.ordered:
            guard()
            record = records.get('frt-' + self.batch_id + '-' + step.name)
            require(record is not None, 'receipt is not for the original batch step')
            before = record.plan.before
            require(before.generation == self.build.generation
                    and (before.selection.kind, before.selection.item, before.selection.target)
                    == (KINDS.index(step.kind), step.item, step.target),
                    'receipt changed the original selection or incarnation')
            if previous is not None:
                require(before.tick >= previous.tick and before.sequence >= previous.sequence
                        and before.next_job >= previous.next_job
                        and before.next_building >= previous.next_building,
                        'original placement receipt chronology regressed')
            previous = record.after
        self.plan.check_dimensions(recovered.goals[0].record.plan.before.dimensions)
        guard()

    def encode(self, guard: Guard = idle) -> bytes:
        self.validate(guard)
        plan = canonical(self.plan.json())
        child = self.condition.encode()
        raw = (MAGIC + field(text_bytes(self.origin, 4096))
               + exact_hex(self.batch_id, 32) + exact_hex(self.review_digest, 32)
               + self.build.encode() + self.operations_generation.to_bytes(8, 'big')
               + len(plan).to_bytes(4, 'big') + plan + len(child).to_bytes(4, 'big') + child)
        require(len(raw) <= MAX_GOAL, 'complete original construction goal exceeds bound')
        guard()
        return raw

    @classmethod
    def decode(cls, raw: bytes, guard: Guard = idle) -> Goal:
        guard()
        reader = Cursor(raw, MAX_GOAL)
        require(reader.take(8) == MAGIC, 'wrong original batch construction goal')
        path = reader.string(4096)
        batch_id, review = reader.take(32).hex(), reader.take(32).hex()
        build, generation = Manifest.read(reader), reader.number(8)
        plan = FurniturePlan.decode(reader.take(integer(reader.u32(MAX_PLAN_BYTES), 1, MAX_PLAN_BYTES)))
        child = condition.Goal.decode(reader.take(integer(reader.u32(condition.MAX_GOAL), 1, condition.MAX_GOAL)))
        reader.finish()
        value = cls(path, batch_id, review, plan, build, generation, child)
        require(value.encode(guard) == raw, 'noncanonical original construction goal')
        return value

    @property
    def digest(self) -> str:
        return hashlib.sha256(b'dfmcp-route-batch-construction-goal/1\0' + self.encode()).hexdigest()


def from_batch(batch, deadline: int, **timing) -> Goal:
    """Called with an already locked, fully verified original Batch owner.

    This is not a receipt-selection API. The complete original plan and every
    registered child must agree with the actual parent owner at publication.
    """
    state = batch.audit()
    require(state['status'] == 'all_placed' and state['pending_step'] is None
            and len(batch.entries) == len(batch.plan.steps)
            and all(row.get('registered') is True for row in state['steps']),
            'complete registered original placements are required before monitoring')
    receipts = batch.placed_prefix()
    require(len(receipts) == len(batch.plan.steps), 'original batch has an incomplete placed prefix')
    goal = Goal(batch.path, batch.id, batch.review.digest, batch.plan,
                Manifest(batch.native.generation, batch.native.df_version, batch.native.dfhack_version),
                batch.source.inventory_generation, condition.Goal(receipts, deadline, **timing))
    goal.validate(batch.budget.work)
    verify_origin(goal, batch)
    return goal


def verify_origin(goal: Goal, batch) -> None:
    """Recheck actual original custody, including for historical terminal output."""
    goal.validate(batch.budget.work)
    state = batch.audit()
    require(batch.path == goal.origin and batch.id == goal.batch_id
            and batch.review.digest == goal.review_digest and batch.plan == goal.plan
            and (batch.native.generation, batch.native.df_version, batch.native.dfhack_version)
            == (goal.build.generation, goal.build.df_version, goal.build.dfhack_version)
            and batch.source.inventory_generation == goal.operations_generation
            and state['status'] == 'all_placed'
            and len(batch.entries) == len(goal.plan.steps)
            and all(row.get('registered') is True for row in state['steps']),
            'original route batch custody or complete selection changed')
    receipts = batch.placed_prefix()
    require(len(receipts) == len(goal.plan.steps)
            and set(receipts) == set(goal.condition.receipts), 'original placed receipts changed')
    batch.check()


def advance(state: condition.Progress, goal: Goal, sample: condition.LinkedSample,
            guard: Guard = idle) -> condition.Progress:
    goal.validate(guard)
    require(type(sample) is condition.LinkedSample and state.goal_digest == goal.digest,
            'construction sample belongs to another original goal')
    # Pin the first sample to the original inventory incarnation too; the generic
    # selected-receipt monitor otherwise learns that source from its first sample.
    require(sample.before == sample.after == goal.build
            and sample.operations.generation == goal.operations_generation
            and sample.operations.software == goal.build.software,
            'construction sample changed the original native sources')
    child_state = replace(state, goal_digest=goal.condition.digest)
    result = condition.advance(child_state, goal.condition, sample, guard)
    guard()
    return replace(result, goal_digest=goal.digest)
