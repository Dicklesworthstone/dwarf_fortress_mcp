# Native resource project MCP

## Scope and evidence posture

`dfmcp-native-resource-project-dev-server` owns one original Carpenter
Workshop construction followed by one fixed wooden-furniture stock target.
It composes the actual Workshop and combined-production runtimes inside one
modern MCP session. The public surface retains the eleven existing tool names.

This is explicitly **unadmitted development source**. It adds no bridge method,
production runner, registry entry, canonical cross-family world adapter, or
automatic native effect. Every construction, labor assignment, finite order and
clock interval still requires its child's own current review and confirmation.

An already completed Tree harvest can supply material through the existing
Workshop Tree-material importer. Tree designation and Manager appointment are
not automatic project stages. An existing configured Manager prerequisite and
input-supply policy remain the combined child's independent guards.

Owning work: `df-dfhack-bridge-plane-c-pic.4` and
`dfmcp-reality-wiring-proof-4bf.14`. Both remain in progress. This increment
implements a concrete two-stage consumer; it does not complete generic
mixed-family planning or the full live acceptance campaign.

## What the parent owns

The parent fixes the original goal, host policy identities, custody directories,
stage order and absolute game-tick deadline before opening a child. Its journal
and independent acknowledgement retain:

`WorkshopOpening → Workshop → Transition → ProductionOpening → Production`

`Cancelled` records its prior stage and retains all existing child references.
It cannot transition forward again.

The parent writes and acknowledges an opening marker before initializing either
child. It writes and acknowledges the Workshop-to-production transition before
releasing the Workshop writer. Production then opens the exact Workshop donor
Offline using its existing importer. The parent does not bypass either child's
journal, independently acknowledged origin, native permissions, source checks,
lease, current review or effect confirmation.

A marker left by an incomplete open is **not initialization permission**. Reopen
existing custody only. The controller does not recreate missing files, choose a
replacement key, silently repair a journal, or dispatch an effect from replay.

## Start the profile

Select the fixed profile with the standard executable:

```text
dwarf-fortress-mcp serve-gameplay-dev native-resource-project
```

The dedicated `dfmcp-native-resource-project-dev-server` binary selects the
same owner. Supply the operator configuration below before startup. The
separate `workshop-production` profile retains its broader staged production
requests and externally held parent checkpoint; this profile adds a fixed
root goal, absolute deadline and independently acknowledged parent journal.

## Operator configuration

Configuration, source endpoints, credentials, private paths, footprints and
native authority are host inputs. MCP requests cannot select them.

The dedicated profile requires:

| Setting | Purpose |
| --- | --- |
| `DFMCP_ALLOW_UNADMITTED_NATIVE_RESOURCE_PROJECT=1` | Explicit parent development opt-in |
| `DFMCP_NATIVE_RESOURCE_PROJECT_CONFIG` | Closed parent configuration described below |
| `DFMCP_WORKSHOP_MCP_CONFIG` | Existing Workshop source, footprint, material, authority and custody configuration |
| `DFMCP_COMBINED_PRODUCTION_CONFIG` | Existing production configuration without a caller-selected `workshop_prerequisite` |
| `DFMCP_SEMANTIC_WORKFORCE_CONFIG` | Existing workforce source and custody configuration |
| `DFMCP_SEMANTIC_WORK_ORDER_CONFIG` | Existing finite-order source and custody configuration |

All existing independent family opt-ins and permission flags remain required.
Configured Tree material, Workshop clock, production clock, inventory clock and
Manager credentials are accepted only for the corresponding configured feature.
Production launch/admission variables and unrelated `DFMCP_` profiles refuse.

The parent and each child retain exact byte hashes of their live configuration
strings. A changed configuration during a session refuses further operation.
On restart, only the explicitly recognized initialization/recovery selectors
are omitted from the stable policy identities; their actual journal identities,
acknowledgements, original archive references and floors are bound separately
to the retained parent checkpoint.

The parent, Workshop, production, workforce and order directories must be
distinct canonical absolute paths, without ancestor/descendant overlap.
Existing private-file ownership, exact modes, no-follow checks, exclusive locks,
file synchronization and directory synchronization remain in force.

A fresh parent configuration has this shape:

```json
{
  "schema": "dfmcp.native-resource-project-development/1",
  "directory": "/private/fortress/resource-project",
  "initialize": true,
  "goal": {
    "recipe": "wooden_bed",
    "minimum_stack_units": 5,
    "max_order_amount": 20,
    "deadline_tick": 13000,
    "poll_interval_ticks": 10,
    "stable_samples": 2,
    "assign_labor": false
  }
}
```

The tick above is illustrative. Choose an absolute future deadline within the
original Workshop host authority and the production child's existing goal
horizon. If a Workshop clock is configured, its fixed deadline must equal the
parent deadline. Future review, restart, handoff or source progress cannot
extend it. Query, original reconciliation and separately authorized clock safety
stops retain the original host authority expiry, so the parent goal deadline
does not itself revoke those safety paths.

Supported recipes are `wooden_bed`, `wooden_door`, `wooden_table` and
`wooden_chair`. The target is 1–1,000,000 observed type stack units; a single
finite order is bounded by `max_order_amount` in 1–100. Positive deficits beyond
that bound refuse instead of being silently truncated.

`assign_labor: false` uses already-enabled eligible Carpenter workers.
`assign_labor: true` adds an independently reviewed labor-enable step before the
dependent finite order. This choice is fixed in the parent original. Worker IDs
come from the later production observation; labor remains `CARPENTER`.

The production template must not contain `workshop_prerequisite`: the parent
supplies the typed, independently verified original donor at the transition.
All three production configuration strings are validated before the first
Workshop operation. Fresh initialization requires both existing native family
authorities even when the selected later plan will not assign labor.

## One-session workflow

### 1. Open and inspect the Workshop offer

Open with `mode: "control"` and the intended per-request budget. The default
mode is Offline. Opening creates custody and can acquire bounded native
observations; it never constructs, inserts an order or advances game time.

Call `fortress_observe`. For operator-pinned material, the existing closed
Workshop request is:

```json
{
  "kind": "carpenters",
  "expected_observation": "<published Workshop witness>",
  "material": "<published material_handle>"
}
```

For a configured completed Tree donor, use the existing
`carpenters_from_tree_supply` request and its exact published donor witness and
material handle. The importer freshly qualifies supply and the Workshop owner
independently verifies the selected log and footprint.

Pass the JSON as the `request` string to `fortress_plan`. Read the complete
child review. Confirm that exact seal with `fortress_commit`; then reconcile
and sample the original with `fortress_wait`. A placement receipt is not
completed construction.

### 2. Use only independently reviewed time

Time allowance defaults to zero. If the host configured a Workshop clock,
`kind: "run_workshop"` uses the existing original target ID, bounded game ticks
and wall milliseconds. Review and confirm that interval, then call
`fortress_wait(scope: "clock")` to reconcile it.

The same project deadline remains in force. A clock receipt establishes
historical clock-control custody, not Workshop completion. Pending, unresolved,
prepared or advancement-fenced Workshop clocks prevent the handoff.

### 3. Transfer the completed original

Use the latest parent head returned under
`retained_reference.parent_acknowledged.head`:

```json
{
  "kind": "advance_to_production",
  "expected_head": "<exact current parent head>"
}
```

Send it through `fortress_plan`. The transition:

1. Verifies the original Workshop journal and independent ACK.
2. Requires settled original clock custody without a live clock preparation.
3. Reads fresh progress for the exact original building and material.
4. Consumes source generation, tick and building allocator floors even on a
   later refusal, and checks the final native reply manifest.
5. Rechecks complete custody and original completion.
6. Retains updated Workshop recovery floors, then acknowledges `Transition`.
7. Releases the exclusive Workshop owner.
8. Acknowledges `ProductionOpening` before opening the actual production owner.
9. Independently imports the exact donor Offline and retains production custody.

This transition dispatches no construction, labor, order or clock effect. The
fresh handoff sample is evidence at its export anchor; production still freshly
qualifies the original donor before positive planning and protected native
order boundaries.

### 4. Plan the fixed stock target

Call `fortress_observe` for current production worker choices. The existing
closed `stock_target` request names the published observation seal, selected
eligible worker IDs and all original goal terms:

```json
{
  "kind": "stock_target",
  "expected_observation": "<published production observation seal>",
  "units": ["<published eligible worker ID>"],
  "labor": "CARPENTER",
  "recipe": "wooden_bed",
  "minimum_stack_units": 5,
  "max_order_amount": 20,
  "deadline_tick": 13000,
  "poll_interval_ticks": 10,
  "stable_samples": 2,
  "assign_labor": false
}
```

Recipe, minimum, order bound, deadline, cadence, stability and assignment choice
must equal the original parent goal. A different target, portfolio, labor or
deadline refuses.

The child obtains its real qualified complete stock census and computes the
finite deficit. A met initial target returns `already_satisfied` with its exact
anchor, source identity, counters and assessment witness, without creating an
order. That result explicitly retains `fresh_for_current_call: false`,
`current_inventory_proven: false` and `stability_proven: false`. It is not a
current temporal completion proof or a causal production claim.

For a positive deficit, the compiler fixes the complete original before any
child publication. The parent then writes and independently acknowledges a
`dfmcp.project-production-intent/1` record containing the exact goal, plan and
archive digests, citizen capsule identity, initial execution-journal identity,
Workshop/Manager associations, input binding and native custody identities.
Only after that acknowledgement succeeds can the child create its original
archive, capsule, associations and execution journal. This publication creates
no native review, preparation, order or labor effect. The no-work census path
creates neither this intent nor an order.

For a positive deficit, plan each dependency-ready original step with:

```json
{
  "kind": "step",
  "original_plan_digest": "<the retained original production digest>"
}
```

Read and confirm each exact current review. The existing
`review_prepared` request can obtain an eligible fresh review of the same
undispatched original step; it never repeats native preparation or restores an
old confirmation. An acknowledged labor step and dependent order remain
separate native effects.

### 5. Verify the original outcome

`fortress_wait` performs one foreground reconciliation or goal sample. It
does not run a background loop. The existing full-parent labor/inventory
monitor, original deadline, cadence and stability rules determine
`original_goal_proven`. Settled clocks are required. An order insertion
receipt or a retired order cannot alone produce this flag.

Independently configured production clocks retain their existing requests:

| Plan kind | Commit/cancel scope | Purpose |
| --- | --- | --- |
| `run` | `clock` | Advance the exact retained original order/goal |
| `inventory_run` | `inventory_clock` | Advance an already observed inventory stability window |

Each interval needs a separate current review and explicit confirmation.
Neither changes the parent deadline or grants another native mutation.

## Stop, close and recover

Default `fortress_cancel` uses `scope: "project"`; `goal` is an alias.
It first acknowledges the terminal parent stop, then retires the child's
undispatched preparation. It cannot undo construction or remove an already
inserted order. Explicit `clock`/`inventory_clock` cancellation delegates to
the existing independently authorized safety-stop owner. Continue original
reconciliation until the relevant child state is known.

`scope: "session"` releases local sockets and locks while retaining files. It
does not claim that an external native effect stopped or that the game paused.

Retain the complete recovery reference after every successful or failed call:

- Parent `resource-project.journal`, `resource-project.ack` and the external
  `parent_acknowledged` pin.
- Exact child custody, ACKs, original archive/frontier identities and clock pins.
- The immutable handoff from the parent checkpoint.
- Any immutable `production_intent` from the parent checkpoint, including when
  the child original was not yet acknowledged in the next parent checkpoint.
- Consumed authority/source floors and sticky discontinuity fences, including
  newer in-memory rejection evidence returned after a failed append.

Reopen the parent with `initialize: false`, the same original goal/policies,
and `acknowledged` equal to the externally retained parent pin. Reopen an
active child with its existing custody selectors. Production recovery also
repeats the exact retained handoff in the parent configuration's `handoff`
field so configuration can be preflighted before custody I/O. The runtime then
compares that value against the actual parent journal.

Recover mode cannot start a new stage. Explicit parent Control can continue
from an acknowledged transition under current host permissions; recovering a
Workshop child still grants no fresh construction authority. Child checkpoint
selectors must match the parent's retained originals before opening, and
subsequent child validation accepts only authenticated descendants. A
cancelled opening remains a local terminal marker and never activates a child.

If the exact production intent was acknowledged but the child reply or next
parent checkpoint was lost, retain the intent and reopen the existing files.
The parent can derive the original recovery selectors from that intent, so a
missing child reply does not require guessing a plan digest or journal head.
Leave the combined configuration's `recovery` absent when those selectors were
lost; supplied selectors must match the parent-held original. Keep the native
workforce/order custody identities and configured clock journal identities.
Their files still undergo the ordinary child validation.

This pending-publication recovery requires the exact original archive, citizen
capsule, prerequisite associations and initial execution journal. Native effect
journals must remain empty; goal history, execution progress or clock runs
cannot be adopted through the pending intent. If the child checkpoint was
already acknowledged, ordinary authenticated descendant recovery applies.
Both paths retain newer rejection floors and sticky source fences. The intent
is immutable after publication and remains present through later checkpoints
and cancellation; it never supplies a replacement key or resets a deadline.

The Agent Turn exposes a pending publication under `active_work.publications`
and withholds plan/commit recommendations until the original has been verified.
An intent alone does not prove that the child publication finished. Missing,
partial, foreign or changed files refuse without repair or initialization.
Legacy parent journals remain readable, but an original created without any
prior acknowledged intent cannot be automatically adopted. Automatic Tree and
Manager stages and full multi-family orchestration remain open.

A parent append/readback/synchronization failure fences the owner. Partial or
rolled-back journal/ACK data is not repaired in place. A failed response fit or
expired final publication budget withdraws live reviews before the session lock
is released. Recovery data never restores a native permit or current goal proof.

## Shared request and output bounds

| Bound | Value |
| --- | --- |
| Wall time | Default and maximum 60,000 ms per foreground call |
| Parent logical byte-work | Default 16 GiB, selectable 4–32 GiB |
| Output | Exactly 65,536 admitted tokens under the existing four-byte convention; complete encoded response capped at 256 KiB |
| Entities | 4,096 ceiling, narrowed by the child |
| Actions | Five ceiling; Workshop child narrowed to one |
| Game-time allowance | Default zero, maximum 1,200 per explicitly reviewed interval |
| Parent journal | 8 MiB, at most 4,096 frames |
| Immutable original/handoff | 64 KiB each |
| Child checkpoint | 32 KiB each |
| Exact production publication intent | 32 KiB |

Byte-work is a conservative bound on repeated reads, replay, hashing and
verification; it is not resident memory or output size. Ledger operations
prepay 128 MiB each. Child work is prepaid at a narrowed ceiling, and unused
reservations are not recycled into another stage. Positive stock-target
planning prepays separate intent and closing-checkpoint reservations before
child source reads. A closing checkpoint and
complete output reserve remain available before child I/O. Final publication
rechecks the same original wall interval.

The first MCP text block contains the canonical Agent Turn. Large evidence
fields or complete recovery metadata may use
`dfmcp.native-resource-evidence/1` descriptors. Each descriptor names the exact
JSON field path, encoding, total byte length, SHA-256 and ordered content blocks
with byte offsets and chunk hashes. Hex evidence reconstructs the original
bytes; `utf-8-json` blocks reconstruct the complete serialized JSON subtree.
Descriptors can refer to evidence descriptors inside that subtree. Reassemble
all referenced data before reviewing a native confirmation or retaining restart
configuration. No review is published as a truncated prefix.

There is no synthesized cross-family canonical anchor. Native child anchors,
historical custody and current production goal anchors remain explicit and
separate.

## Validation record

The current recovery increment passes exact-source component library and
test-target metadata checking with locked dependencies on
`rustc 1.102.0-nightly (bdf712ee8 2026-11-21)`. The component retains the complete
actual core, world, intent, adapter and laboratory crates and selected MCP
owner/test bodies. Its eleven external dependency identities match the original
lockfile. FastMCP transport/codec, unrelated module surfaces and real Asupersync
scheduling are excluded; explicit synchronous I/O-capable context glue is used.
All 82 selected tests pass: 41 parent, eight child recovery, five project
configuration, nineteen combined configuration, four fixed-goal, four ordinary
stock-target runtime campaigns and one new publication campaign. The new actual
Runtime/native-TCP/private-file campaign checks no original writes before the
parent ACK, zero writes on no-work or refused ACK, exact lost-selector recovery,
missing/corrupt/foreign custody, incomplete publication and advanced native
Prepare refusal. Recovery performs no native calls or custody writes. The
component's 2,282 source hashes and seventy selected recovery paths match;
proof SHA-256 is
`f195c238253e6dc8acbfb7bbdf6b81aa88ef0ae96ebb91a031f911d5c966354e`.

The complete actual adapter library also passes metadata checking. Full MCP
checking was killed in unchanged Asupersync 0.5.0 before repository diagnostics,
despite serial frontend/backend compilation. Current agent/dependency/status
and Workshop source gates, eleven Workshop inventory negatives and scoped
formatting pass. Mandatory verification reaches missing `br`; release
qualification refuses the changing worktree. These are development checks,
not a full MCP transport, live-game or production-admission qualification.

### Earlier parent implementation evidence

Before the execution host disconnected, a focused ledger harness passed
17/17 tests, including two cases using the actual Linux private-file storage.
It used the actual core crate, complete new ledger/test logic, exact journal
trait/mode excerpts and the real private-file/locking implementation; unrelated
adapter decoding factories were excluded.

That host became unavailable before the integrated controller could compile.
Source was reconstructed through GitHub against the then-current main,
preserving the concurrent completed-Workshop export. The reconstructed
formatted bytes had not been replayed at that checkpoint. Parent facade/output/
budget tests, Workshop handoff/recovery tests and production goal/custody tests
were authored then. The component checks above supersede the earlier
uncompiled-source status; execution, full verification, modern stdio and live
DFHack qualification remain separately recorded.

The independent source reviews and GitHub blob readbacks are not Rust,
native, live-campaign or admission evidence. Existing registry and production
protocol mappings remain unchanged.
