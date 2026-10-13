"""Closed original-attempt evidence transition; no native I/O or dispatch.

Bead df-dfhack-bridge-plane-c-pic.4. The original uncertain receipt remains
historical evidence; only full exact Placed readback may resolve the attempt.
"""
from build_placement_wire import Record, require


def verify_reconciliation(previous: Record, current: Record) -> None:
    """Only full original-attempt Placed evidence can resolve Indeterminate.

    This validates typed evidence, not acquisition or the native reader's custody.
    No observation, absence, cancellation, changed plan or new key is a resolution.
    Callers must independently check the exact original native manifest.
    """
    require(type(previous) is Record and type(current) is Record,
            'invalid reconciliation records')
    require(previous.phase == 'indeterminate' and current.phase == 'placed'
            and previous.plan == current.plan, 'not an original-attempt reconciliation')
    # Reuse the full codec checks, including expected-after byte equality and
    # exact stage-zero building/job/item links. A checksum alone is insufficient.
    Record.decode(previous.raw, previous.plan)
    Record.decode(current.raw, previous.plan)
