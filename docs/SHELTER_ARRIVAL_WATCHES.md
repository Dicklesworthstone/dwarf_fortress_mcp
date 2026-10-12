# One joint shelter-arrival goal through existing watches

Owning bead: `dfmcp-wp-pln03-labor-civilian-alert-fsm-zdm` (WP-PLN-03).

The unadmitted spatial/1.8 development query surface now accepts a `shelter_set`
condition in the existing `watch` and `condition_evaluation` requests. It checks
all original named living citizen incarnations against a fixed union of inclusive
3D regions. The same foreground engine owns registration, polling, batches,
deadlines, cancellation, active-work presentation and configured watch journals.
There is no new monitor registry, timer, game effect, top-level tool or admission.

## Register the whole goal

Use `schemas/examples/shelter_watch_v1.json` as the object-valued `query` argument
of `fortress.query`, alongside the existing spatial session ID. Replace the
synthetic IDs/generations with the current session's citizen identities, and
replace the example deadline with a future absolute game tick within the
session's negotiated horizon. Laboratory IDs and raw native unit IDs are not
spatial/1.8 canonical IDs.

```json
{
  "schema": "dfmcp.query/1",
  "query": {
    "kind": "watch",
    "key": "original-shelter-arrival",
    "condition": {
      "op": "shelter_set",
      "citizens": [
        {"entity_id": "3298534883338", "generation": 1},
        {"entity_id": "3298534883339", "generation": 1}
      ],
      "shelters": [{"min": [0, 0, 5], "max": [1, 1, 5]}]
    },
    "deadline_tick": 1000,
    "poll_interval_ticks": 10,
    "stable_observations": 2
  }
}
```

Retain the returned `record.watch`. Use `poll_watch` to evaluate the currently
published observation, or the existing `await_watch` to explicitly acquire one
new coherent capture when the watch requires it. `await_watches` retains its
shared-capture batch behavior. These are foreground read operations, not
background scheduling or game-time control. Identical captures cannot provide
new stability credit; another authorized controller must advance game time.

`condition_evaluation` accepts the same condition for one current or retained
observation without registering a watch. `fortress.query(mode="schema")`
discovers the new condition. The existing `shelter_status` remains the paginated
single-capture positional inspection path.

## What counts toward completion

Every eligible sample checks the entire named selection together. One citizen
arriving earlier and leaving before another arrives cannot accumulate a joint
success. Inside yields true; an observed outside or dead citizen yields false;
missing citizens, unavailable positions and unqualified fields yield unknown.
A changed named generation invalidates the watch, even inside a Boolean group
whose earlier child already decided its truth. Babies, children and busy citizens
remain in the goal. Missing targets are not dropped or replaced.

The unchanged watch engine requires the original number of successful samples
separated by the original game-tick cadence. Observed false/unknown conditions
reset the streak, including observations before the next sample is due. Cursor
gaps reset it; epoch/clock/identity regressions invalidate it. The original
absolute deadline never extends when a citizen is late. A dead target keeps this
arrival condition false; it is not silently removed or automatically redefined.
An independently requested failure condition retains the ordinary watch rules.

These are sampled observations, not continuous occupancy. The existing watch
policy does not impose an additional maximum elapsed time between consecutive
cursor observations. Cadence sets a minimum sampling interval, not continuous
coverage or a physical dwell-time guarantee.

A satisfied watch retains its historical evidence. It does not keep sampling or
claim that citizens remain inside after later departures. Check
`evaluation_current`, and use a fresh `condition_evaluation` or `shelter_status`
for present truth. Cancellation stops only local observation work; it neither
releases civilians nor cancels an evacuation or other game effect.

## Source, custody and bounded representation

The condition is evaluated only inside the existing authorized query path over
the observing shell's published snapshot. Exact spatial/1.8 profile, nonzero
shared source digest, map dimensions, native identity, citizen/resident flags,
liveness, active/visible flags and positions are checked at the capture tick.
Other profiles, unavailable fields, replay/assertion labels, changed producer
labels and cross-capture values cannot establish arrival. These consistency
checks issue no independent source policy or compatibility admission.

Selection validation reuses `ShelterSelection`: 1..128 citizens, 1..16 regions,
nonzero generations, bounded canonical IDs and at most 262,144 summed region
tiles. Citizen identities must be numeric ascending; cuboids must use the same
lexicographic `(min, max)` order as the canonical selection. Duplicates and
reversed/out-of-range regions refuse. Invalid observed map dimensions or shelters
outside them block the current condition as unknown.

Shared success/failure validation permits at most 256 shelter target references
and 32 regions across the complete Boolean expression. All field/region visits
consume the existing shared evaluation allowance; the input remains bounded to
1,024 JSON nodes and 32 KiB of aggregate accounting. Full diagnostics, including
every target, must fit before registration or sample publication. No prefix of a
group can be published as the full goal.

Configured spatial watch journals retain the new serialized condition through
the existing checkpoint path. No journal format or old condition encoding is
changed. Existing recovery requires the paired observation archive, preserves
original deadlines, resets unfinished stability and withholds current proof
until a fresh observation. An older binary that does not understand
`shelter_set` cannot decode a checkpoint using it; it must not repair or reinterpret
that record. Source gaps and release remain owned by the existing watch lifecycle.

## Validation boundary

Nine new Rust tests use the production native projection fixture, the actual
condition evaluator and the existing watch store. They cover differential
single-capture results, joint arrivals, repeated ticks, departures, gaps,
incarnation reuse, source mutations, publication refusal, authority, cancellation,
deadlines, legacy serialization and maximum group size. They are source-reviewed
but not compiled or run: this editing environment has no Rust/Cargo toolchain.

`python3 scripts/test_shelter_watch_schema.py` executed successfully: one schema,
the example, 44 malformed conditions rejected and the 128-citizen/16-region
request fitting the unchanged shape bounds (540 nodes, 21,621 accounted bytes).
The original watch source is byte-identical after reversing only the module,
condition, validation, evaluation and schema-extension additions. These static
checks do not establish Rust, DFHack, live-game or production qualification.
