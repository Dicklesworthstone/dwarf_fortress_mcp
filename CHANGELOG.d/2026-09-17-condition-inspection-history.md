# Stateless condition inspection and historical condition timelines

## Implemented source

- Add condition_evaluation through the existing stateless query dispatcher, with
  spatial/1.8 live, exact-record historical and offline archive execution. Reuse
  the existing watch Condition parser, joint definition validator and bounded
  Probe evaluator; do not implement a second predicate language or state machine.
- Inspect compound field, population-count, item-quantity, pause and tick
  predicates together with an optional failure guard. Retain per-leaf evidence,
  a normalized predicate digest and exact-anchor evaluation evidence digest.
- Preserve watch precedence: any explicit reference generation mismatch overrides
  boolean shortcuts, known failure overrides success, and unknown success or
  failure evidence blocks an eligible success sample. Missing entities remain
  unknown; a recycled ID is not silently treated as the original subject.
- Keep predicate inspection separate from temporal monitoring. No Watch object,
  handle, registry lookup, sample count, stability streak, deadline transition,
  terminal result or durable checkpoint is created or changed. Repeated reads
  do not advance retained watches. Omitted/null guards normalize identically.
- Extend historical_series to condition_evaluation in addition to item_quantity.
  Each sample matches individual exact-record inspection and retains original
  entity generations, source/record/anchor witnesses and evaluation evidence.
- Preserve adjacent classification changes across complete-row pages. Show
  success truth, failure truth and generation status separately. Identical
  unknown or true classifications do not prove unchanged facts, continuous
  satisfaction, deadline compliance or stable goal completion. Resets break
  comparisons; condition timelines never manufacture numeric rates or backfill
  a live watch's history.
- Reuse one verified journal prefix per page, current authority/custody checks,
  mode-specific Agent Turn and active-work reservation, and remaining wall time.
  Retain 1..32 samples plus one predecessor, with each measurement bounded to
  its existing one-million-unit evaluation allowance and 16 KiB internal result.
- Bind full measurements and failure guards to existing hs1 page identities.
  Preserve quantity timeline arithmetic, output and continuation identities.
  Narrow archive schema discovery to fifteen stateless reads plus its four
  historical operations, preserving stateful-operation refusals.

Usage: docs/CONDITION_INSPECTION_AND_HISTORY.md. All historical statements name
retained captures, not current game state or continuous events. This feature
cannot prove native readiness or authorize action. Current world, observations,
watch/baseline records and journal bytes remain unchanged. No bridge capture,
background work, native protocol, serialized watch/journal format, dependency,
top-level tool, game effect, production runner or admission permission is added.
This fragment supplements implementation/evidence status; the formal unadmitted
phase remains unchanged.

## Validation

Seventeen new Rust test functions are registered: eight pure inspection tests,
three classification transition tests, and six actual MCP-handler/private-file
scenarios. One transition test enumerates all 324 pairs of success/failure/
reference classifications. Existing quantity timeline and archive schema tests
retain their prior coverage while admitting condition inspection. Cases include
watch-evaluator parity, non-short-circuit identity checks, unknown evidence,
combined predicate budgets, output refusal, unchanged watches and journal bytes,
exact individual/series evidence equality, cross-page transitions, stale guards,
reset boundaries, offline reopening and expired current authority.

NONE of these Rust tests has been compiled or executed here. Rust, Cargo and
rustfmt are unavailable. The 324 cases are registered Rust tests, not executed
evidence. Validation consists of source review and GitHub diff/commit/branch
verification; no Python mirror is offered as execution of the Rust paths. No
passing Rust build, MCP runtime, native/live-game, filesystem-crash, full-repository
qualification or production admission is claimed.
