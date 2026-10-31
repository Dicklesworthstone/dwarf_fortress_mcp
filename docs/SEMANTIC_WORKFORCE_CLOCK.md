# Reviewed clock advancement for original labor goals

Owner: `df-dfhack-bridge-plane-c-pic.4`.

The semantic workforce runtime can advance game time for an already applied
original labor goal that still needs advancing observations. Each finite run
has its own review and confirmation. The original assignment, selected citizen
identities, absolute deadline, polling cadence and stability requirement remain
unchanged. This closes the gap between assigning labor and demonstrating its
sampled stability without requiring another controller to advance the game.

This is an explicitly unadmitted development composition of workforce/1.17 and
the existing bounded-run/1.13 coordinator. It adds no native method or wire
generation. It does not admit a live configuration or change a production runner.

## Two independent authorities

Labor permission and clock permission are independent operator decisions:

```text
DFMCP_SEMANTIC_WORKFORCE_ALLOW_LABOR=1
DFMCP_SEMANTIC_WORKFORCE_ALLOW_CLOCK=1
DFMCP_SEMANTIC_WORKFORCE_CLOCK_TOKEN=<operator-managed run/1.13 credential>
```

Clock control requires the second gate and a clock configuration. The dedicated
token is subject to the existing 32–256-byte credential bound and is never an
MCP argument or part of a recovery response. Initialization still needs labor
authority for the original assignment journal. A later Control reopen can
retain only clock authority; that cannot regain assignment authority.

Run/1.13 identifies an exact endpoint, software versions and source incarnation.
**It does not encode a named fortress lineage.** The concrete shell issues a
separate source-scoped native authority under the explicit clock gate. It never
changes an existing named-fortress grant into a source grant. The original
named-fortress context must independently authorize every goal operation and
clock horizon. Neither scope can stand in for the other.

Fresh original-labor evidence must agree with the clock's starting tick. After
the durable clock marker and immediately before native preparation or unpause,
the runtime rechecks original plan bytes, original citizen capsule, historical
assignment identity, current operator/runtime permission and another exact
same-tick labor sample. False, unknown, changed-source or changed-tick evidence
refuses unpause. This bracket does not establish global exclusion of other
controllers or create a fortress identity field in run/1.13. Operate this
development profile with exclusive external control of the selected game.

## Optional private clock custody

Add this field to the existing closed operator configuration, replacing the
illustrative generation with the exact accepted run/1.13 incarnation:

```json
{
  "clock": {
    "generation": 30
  }
}
```

With `initialize: true`, the fixed `clock.journal` must be absent. The runtime
creates it in the same operator-owned `0700` directory, with exact-mode `0600`
regular-file custody. Initialization can be performed from operator source pins
without a native connection; the actual source must match those pins before
any native clock operation. Creating custody does not establish compatibility.

Every reopen uses `initialize: false` and the externally retained journal ID:

```json
{
  "clock": {
    "generation": 30,
    "journal_id": "<exact 64-character clock journal identity>"
  }
}
```

When an original goal exists, its external `recovery` object also carries
`clock_journal_id`, and both locations must agree. Missing, substituted, corrupt
or differently bound journals refuse. Reopening in Control mode cannot recreate
a missing journal. Omitting the clock configuration cannot hide an existing
clock file. Old configurations without clock custody remain supported.

An Offline open needs no native credential or connection. Reconcile can query
retained runs and acquire fresh labor observations, but cannot prepare, unpause
or cancel. A journal opened without control stays without control even if an
environment gate is later enabled; close and explicitly reopen to change mode.

## Runtime transitions

The Rust API provides these operations on the existing original-goal owner:

| Operation | Behavior |
| --- | --- |
| `prepare_run` | Obtain fresh original-goal evidence, bind a finite run to the original paired seal and return a separate clock review. |
| `commit_run` | Require the exact clock review and explicit confirmation; sync the dispatch marker before the single native unpause attempt. |
| `wait` | Reconcile an unsettled run first. Attempt a fresh paused labor sample only when retained clock uncertainty permits acquisition. |
| `cancel_run` | Retire an undispatched run locally or request the original native owner's safety pause. |
| `inspect` | Verify local custody and expose both original-goal and clock history without native calls. |

The closed run request contains only:

```json
{
  "kind": "run",
  "original_paired_seal": "<original labor review seal>",
  "game_ticks": 5,
  "wall_millis": 10000
}
```

There is no client-selected native key, citizen, work detail, stop predicate,
endpoint, path, source grant or replacement deadline. A run requires historical
`SingleLaborResult::Verified`, fresh true original labor predicates and an
unfinished original goal. Time alone cannot repair a false or unknown labor
assignment. Original semantic completion stops further run proposals.

Game-time permission defaults to zero and must be explicitly negotiated. Each
run admits 1–1,200 ticks and 1–60,000 milliseconds, within both the negotiated
allowance and the original absolute deadline. Native callback stop conditions
can overshoot; they are not exact-tick or hard real-time guarantees.

At most one unsettled run and 64 retained runs belong to one original goal.
One initial sample followed by 63 cadence-sized runs can satisfy the existing
64-sample stability requirement, subject to the unchanged history/work budgets
and deadline. Extra inspections do not add stability at the same tick and can
consume the finite goal-history allowance. There is no automatic run chaining,
background polling, renewed goal deadline or pruning of replay protection.

## Evidence and failure behavior

A native stopped receipt records a historical pause. It does not establish the
game's current pause state, current labor truth, productivity or goal completion.
The response retains the complete original goal, current sampled proof,
historical first achievement, separate clock history and both recovery IDs.
Fresh labor evidence older than the retained clock tick is rejected before goal
proof can be published.

A lost commit reply, failed guard after the dispatch marker, or process restart
never grants another unpause. Query the original run. While the clock may still
be running, `wait` returns retained clock evidence without attempting a paused
labor read. Source loss remains unresolved; it cannot be treated as a verified
stop in a replacement source.

Safety cancellation requires current Query and independent ControlClock
permission. It does not require Plan, Observe, labor-control permission, a true
current labor predicate or an unexpired original goal. Cancellation cannot undo
elapsed game time or the applied labor assignment. Cancellation/receipt failure
retains uncertainty and the original recovery identity.

The original private files, native wire formats and existing bounded-run journal
format remain unchanged. Focused tests execute actual Rust coordinators,
archive/history codecs and private-file custody with injected game/source
responses. This is component evidence, not a DFHack SDK build, live fortress,
complete FastMCP executable or production admission.


## Later bounded development witness

The R20 live9 trial used source `3f89445824ff146f617807ae9958cbba16a2f4d6`,
semantic executable `fc1585f3ccfa04a1747c1e62f170546bb88a5e9cbcfdc7b6dc2e6629ccfa0f81`,
and freshly built native run plugin
`6ca8e54fde444d30605116ed8d4d501a0284f5cc8589008fc2c5611223298066`.
Its native build checked 85 source, 3,639 SDK and three extra inputs before and
after compilation. This is a development build, not R1 or tuple admission.

Terra through NTM negotiated ten ticks and declared an original MINE labor goal
with cadence five, one stable sample and deadline 12112900. One reviewed
assignment left the same-tick goal active with zero eligible samples. A distinct
clock review bound the original labor seal to one ten-tick/20,000-ms run. One
confirmed unpause stopped at tick 12112810 for `tick_limit`, zero observed
overshoot. A fresh paused sample then proved that single-sample original goal
and semantic completion. Independent reads confirmed raw pause and inactive
clock ownership. A clock receipt itself still grants no current goal proof.

Parent independently replayed twelve main MCP exchanges, the eighteen-event
goal-history hash chain and six canonical native captures; the last raw capture
hash equals the published goal source digest. Its policy leaves whole-domain
coverage unknown and grants only the selected positive identity/labor facts.
No intermediate paused sample or two-sample/continuous stability is inferred.

The trial preserves the earlier wrong-version refusal and unused operator
configuration, correction-message ambiguity, and four screenshot copies before
subsequent host scans. Later package export was gated on exact scanned bytes,
with all nine private values absent. These are procedural limitations, so this
is bounded control evidence rather than a clean live qualification campaign.
The original save and backup each retained all 30 files, and all owned clients,
game and NTM processes were stopped. Export manifest SHA-256:
`6479141c9b6121bdbc98452e0234dfbee72c58f76caf433ceb46b68bdb5a35c1`,
under `target/reality-check-20261030/terra-live9-export`.
