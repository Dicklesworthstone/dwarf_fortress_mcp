"""One read-only connection brackets a whole construction plan's sample.

Every original receipt is checked before AND after the same complete immutable
operations capture. No member uses another socket, fresh snapshot or deadline.
The existing single-receipt codec, authority checks and native methods are reused.
Beads: df-dfhack-bridge-plane-c-pic.4 and df-dfhack-bridge-plane-c-pic.5.
"""
from __future__ import annotations

import construction_monitor_rpc as single
from build_placement_wire import require
from construction_plan import Goal, LinkedSample, MAX_TARGETS
from construction_receipt import Goal as ReceiptGoal

Authority = single.Authority

# Four read-method bindings, two handshakes, one fresh capture of at most
# 256 pages, its release, and two exact-record queries for each of 32 targets.
MAX_RPC_CALLS = len(single.BINDINGS) + 2 + single.MAX_CAPTURE // single.PAGE + 1 + 2 * MAX_TARGETS


class Budget(single.Budget):
    """The set profile adds bounded receipt queries, never per-target budgets."""

    def __init__(self, timeout_ms: int):
        super().__init__(timeout_ms)
        self.calls = MAX_RPC_CALLS


class Client(single.Client):
    def __init__(self, authority: Authority, goal: Goal, budget: Budget):
        budget.work()
        require(type(goal) is Goal, 'whole-plan construction goal required')
        # Full set validation precedes opening the first and only socket. It
        # establishes that the first child's furniture generation binds all.
        goal.encode()
        self.plan_goal = goal
        self.member_goals = goal.goals
        super().__init__(authority, self.member_goals[0], budget)

    def _member_receipt(self, goal: ReceiptGoal) -> bytes:
        self.budget.work()
        record = goal.record
        reply = self._call('build', 'QueryPlacement',
                           {10: record.plan.key.encode('ascii'), 12: record.plan.digest},
                           set(range(1, 9)) | {10, 12, 13})
        require(reply[13] >= len(self.member_goals) and reply[10] == goal.receipt,
                'whole-plan original placed record not retained byte-for-byte')
        return reply[10]

    def capture_once(self) -> LinkedSample:
        require(not self.used and not self.closed, 'whole-plan acquisition cannot be retried on this connection')
        self.used = True
        try:
            before_records = tuple(self._member_receipt(goal) for goal in self.member_goals)
            before = self.manifests['build']
            raw = self._capture_operations()
            after_records = tuple(self._member_receipt(goal) for goal in self.member_goals)
            after = self.manifests['build']
            sample = LinkedSample(before, before_records, self.manifests['operations'], raw,
                                  after, after_records)
            sample.validate(self.plan_goal, self.budget.work)
            self.authority.guard()
            self.budget.remaining()
            return sample
        except BaseException:
            self.close()
            raise


def acquire(authority: Authority, goal: Goal, budget: Budget) -> LinkedSample:
    with Client(authority, goal, budget) as client:
        return client.capture_once()
