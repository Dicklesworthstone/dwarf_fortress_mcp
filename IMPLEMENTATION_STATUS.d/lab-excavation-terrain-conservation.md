# Laboratory excavation conserves observed terrain

Owners: `df-lab-connected-room-access-7c2` and
`df-predicate-evidence-authority-r7m` (broader acceptance remains in progress).

A complete hazard halo previously allowed a room blueprint over known void, and
the physical engine could turn OPEN_SPACE or CHASM into FLOOR. A closed shared
transition rule now rejects impossible source material at designation dispatch,
when retained work is admitted, and immediately before every tile replacement.
The last check also catches overlapping channel/mine jobs inside one event; the
existing transaction shadow prevents publication of partial physical progress.
Upward stairs and ramps require ordinary rock, while already-correct targets
remain no-ops. Unsupported codes, trees and magma walls cannot become ground.

Room, furnished-room and workshop blueprint compilation checks exact selected
cells with the same rule. Moats retain perimeter-only selection, valid channel
no-ops and unselected interiors. Source scans remain bounded, and guarded tile writes reuse one chunk decode
and the original per-tile work allowance. Cancellation remains possible after terrain
changes. No action, predicate, original-plan or archive encoding changes.

See [the transformation matrix and execution contract](../docs/LAB_EXCAVATION_TERRAIN.md).

## Verification

Before the fix, actual-source execution reproduced four failing physical-engine
regression groups and three failing blueprint groups, with their valid controls
passing. The initial terrain fix passed all five new engine groups. The broader
intent run passed 305 tests and exposed one large-excavation work-budget failure.
A checked world writer now validates the actual decoded source before mutation,
reusing the original single decode and 256-unit charge; the existing 500,000-unit
regression cap and assertions are unchanged. Two new world tests cover guard
atomicity, no-ops, revisions, and malformed or unknown source rejection.

Independent source reviews found no remaining blocker. Final checked-writer,
intent and blueprint test runs are pending at this publication checkpoint.
Repository verification passed its initial static gates, then stopped because
`br` is unavailable. The local qualification entry refused the in-progress dirty
worktree. Neither attempt establishes full qualification.

The installed toolchain is nightly-2026-08-31 (rustc 1.100.0-nightly), not a claim
of current-latest-nightly qualification. This is deterministic laboratory
behavior; native excavation, structural support, material origin, directional
stairs, full workspace qualification and live compatibility remain separate.
