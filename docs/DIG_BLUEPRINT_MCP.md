# Sparse mining projects through MCP

`dfmcp-dig-control-dev-server` can retain and execute a complete sparse floor
blueprint through the existing eleven tools. One immutable parent binds the
original shape, native source, journal and deterministic child keys. Each child
gets a fresh observation, its own review and at most one confirmed native commit
attempt. Unselected gaps are preserved, including when separate rooms share a
native scheduling block.

This is an unadmitted development path using the existing dig/1.16 protocol.
Its native result is historical designation evidence. Optional independent
map/1.5 monitoring now evaluates sampled excavation of the entire retained
blueprint through the same tools; see [the monitoring workflow](DIG_BLUEPRINT_MONITOR.md).
Neither path automatically advances the game. The limits are 32 nonoverlapping floor parts, 512
selected tiles, a 1,024-cell bounding capture and 128 native regions, each at
most 8x8 on one level. See [the geometry and custody contract](DIG_BLUEPRINT_BATCH.md).

## Configure one original project

Start with the configuration in [DIG_CONTROL_MCP.md](DIG_CONTROL_MCP.md). Use
an operator-owned private directory with exact mode `0700`; the journal and
parent must be distinct, absent files. The implementation creates exact-mode
`0600` files exclusively. It never truncates, replaces or repairs existing files.

Add the closed operator configuration:

```sh
export DFMCP_DIG_JOURNAL=/private/mining/project/native.journal
export DFMCP_DIG_BLUEPRINT_CONFIG='{"path":"/private/mining/project/blueprint","initialize":true}'
export DFMCP_DIG_ALLOW_DESIGNATE=1
export DFMCP_DIG_CHECKPOINT_POLICY=disposable-fortress-no-checkpoint
```

The checkpoint value is the existing explicit disposable-fortress exception,
not a verified game save. New initialization preflights current Plan/Designate
authority, this policy and the parent-opening allowance before creating the
native journal. Unpredictable filesystem failures can still leave partial
custody; preserve those files and the returned identities instead of retrying
creation over them. Parent and native journal publication is not an atomic
two-file transaction.

Open with the original control profile:

```text
fortress.open_session(region="[4,4,2,5,4]", mode="control", max_wall_millis=60000)
```

The initial region is an ordinary bounded bootstrap selection. Opening does not
prepare a child or publish a blueprint. Once this optional project configuration
is enabled, raw child actions are blocked until the original blueprint is
published. Paths, source identity, credentials, protected regions and checkpoint
policy remain operator configuration, never tool arguments.

## Publish the complete shape

For example, two rooms joined by a one-tile corridor:

```json
{
  "schema": "dfmcp.excavation-blueprint/1",
  "parts": [
    {"region":{"origin":[4,4,2],"size":[5,4,1]},"shape":"floor"},
    {"region":{"origin":[14,4,2],"size":[5,4,1]},"shape":"floor"},
    {"region":{"origin":[9,5,2],"size":[5,1,1]},"shape":"floor"}
  ]
}
```

Pass this object serialized as the `blueprint` JSON string to
`fortress.plan(session_id, blueprint=...)`. Omit the individual child review
arguments. A request must contain either the blueprint or all three child
review arguments; mixed or incomplete requests are refused.

Publication checks every future halo and shared 16x16 write block against the
operator scope and protected regions. It takes one fresh paused observation of
the first compiled region, validates every future region against the observed
map dimensions, and synchronizes the immutable parent before returning. It
does not prepare or commit a native effect. Future terrain eligibility is
checked only when that region is actually selected.

The result contains:

- `blueprint.blueprint_id`, layout/mask digests, original native `journal_id`,
  target count and complete child count;
- `blueprint.recovery`, the exact `{blueprint_id, head}` object to retain outside
  the process for restart;
- `blueprint.next`, with the original next region and idempotency key;
- `observation`, the fresh first-region witness and eligibility blockers;
- bounded progress rows and structured Agent Turn next-step suggestions.

Identical publication requests return verified local history. They cannot
replace the original shape, source, native journal or keys. Reordered equivalent
parts normalize deterministically. Changed original parts are refused even when
another decomposition would select the same tiles.

## Review and designate one region at a time

Use the returned `blueprint.next.idempotency_key` and exact selected observation:

```text
fortress.plan(session_id,
  idempotency_key="<original next key>",
  observation_witness="<selected witness>",
  allow_hidden_neighbors=false)

fortress.commit(session_id,
  idempotency_key="<same key>",
  plan_digest="<returned native plan digest>",
  review_seal="<returned paired review seal>")
```

The review binds the parent identity and stop state, complete native journal
head, exact native plan and current policy review. The parent and original
prefix are checked at the native boundaries, including after the durable
dispatch marker and before the sole native commit call. The native coordinator
retains its original one-attempt semantics.

After a durably terminal Designated receipt, read the returned `blueprint.next`
and call `fortress.observe(session_id, region="[x,y,z,width,height]")` for that
exact next region. Then prepare and confirm its separate review. Each new
capture includes scheduling changes from earlier regions. There is no queued
list of stale observations and no request dispatches multiple children.

The first publication capture may be used directly while its original
connection remains valid. Native preparation connections keep their existing
absolute deadline; a later call does not renew them. Expiry after preparation
requires original-key recovery or retirement, not a fresh commit connection.

Hidden-neighbor relaxation is refused for these Rust projects. Missing context,
hidden targets, known hazards, non-natural walls and existing designations are
not silently skipped. A refused or cancelled child halts the original sequence;
later children cannot be substituted or advanced around it.

## Inspect progress without native access

`fortress.query` accepts these closed JSON strings:

```json
{"mode":"blueprint","blueprint_id":"<original id>","offset":0,"limit":8}
{"mode":"blueprint_layout","blueprint_id":"<original id>"}
```

Progress verifies the entire original native prefix before returning 1..16
rows. `next_query` pins both the native journal head and parent stop head; it
fails if either changes between pages. Pending work remains visible even when
its row is beyond the current page. The retained canonical input is returned
as `result.definition` by `blueprint_layout`.

The main statuses are:

| Status | Meaning |
|---|---|
| `ready` | A verified designated prefix permits a fresh review of the next original child under current authority. |
| `pending_recovery` | An original native operation remains unsettled. Recover its exact key and plan. |
| `halted_unknown` | Native Unknown is permanent evidence; no later child or commit retry is permitted. |
| `halted_refused` / `halted_cancelled` | An original child ended without a designation; the sequence halts. |
| `stopped` | A permanent parent stop prevents future advancement. |
| `all_designated` | Every original child has a verified historical Designated receipt. Excavation completion is unproven. |

The canonical geometry can be used separately with read-only blueprint
assessment or `track_excavation.py start-blueprint`. Matching geometry does not
join their source identities, prove causal attribution or discharge this
project's native effects.

## Restart, reconcile and stop

Retain the returned recovery object, release the current session, and change
only the operator configuration needed for exact reopening:

```sh
export DFMCP_DIG_BLUEPRINT_CONFIG='{"path":"/private/mining/project/blueprint","initialize":false,"recovery":{"blueprint_id":"<original id>","head":"<last acknowledged parent head>"}}'
```

`fortress.open_session` supports three modes for that existing project:

| Mode | Opening and subsequent authority |
|---|---|
| `offline` | Opens exact original custody with Query only and makes no native connection. Inspection and layout export are available. |
| `reconcile` | Opening is source-free. Exact original outcome queries can connect later; no new Plan/Designate authority is issued. |
| `control` | Audits the complete retained prefix, reads the exact source's current clock and sequence, discards that source, then issues a new host lease at the current floor. Current operator designation policy still applies. |

The `region` argument remains required and must fit the operator scope. A
Control reopen uses the retained first region for its independent clock read.
It rejects source, dimension, clock or sequence regression. This prevents both
immediate lease expiry after a long absence and acceptance of a later stale
capture. Offline/reconcile modes need no native token at opening.

No mode restores an old selection, preparation connection, review or dispatch
permit. Control can advance only an absent next child after a verified terminal
prefix. An old prepared child must be inspected, queried or retired under the
original key. Use `fortress.wait(session_id, idempotency_key, plan_digest)` for
one exact native outcome query; it never retries commit or advances game time.
Use `fortress.cancel(scope="effect", ...)` under current native-retirement
authority to retire the original preparation; it cannot undo designation.

To stop future project advancement permanently:

```text
fortress.cancel(session_id, scope="blueprint", blueprint_id="<original id>")
```

This publishes a parent stop in a writable mode using current scoped Query
authority. It drops local preparation permission, makes no native call, and
does not cancel native work, stop miners or undo any designation. Preserve the
new recovery head. A lost stop acknowledgement may adopt a complete newer stop;
an acknowledged stop cannot reopen an older unstopped header.

Native outcome query and retirement remain available when the parent is stopped
or its current verification fails. A successful native recovery result carries
`blueprint_error` if the parent cannot be verified. Other failures, overflow,
release and final runtime-check failure preserve bounded historical recovery
identities without implying current inventory certainty or dispatch permission.

## Verification scope

The focused Rust tests execute the actual layout, parent store, native journal,
MCP action handlers, policy, presentation and injected native interfaces. They
cover the complete sparse lifecycle, original-key recovery, 128-child inventory,
private files, stop/corruption failures, current grants and clock/sequence floors.
The query-schema script separately validates the published JSON Schema.

These component checks exclude FastMCP macro/stdio routing and ambient runtime
scheduling, and do not constitute full-workspace Rust qualification, a native
DFHack build, a live fortress campaign or compatibility admission. See the dated
results in [IMPLEMENTATION_STATUS.md](../IMPLEMENTATION_STATUS.md). Generic mixed
action DAGs, verified game checkpoint/restore, live monitoring qualification
and continuous terrain or structural-safety proof remain separate work.
