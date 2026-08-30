# MCP Surface

The public surface is intentionally smaller than the internal action and query registries. The
wire is **MCP 2026-07-28, modern-only**, carried by the owned `fastmcp_rust` sibling at an exact
pinned revision (ADR-013, `docs/FASTMCP_INTEGRATION.md`); the `legacy-2024-11-05` graph is never
compiled, and `dfmcp/0` semantic negotiation remains authoritative above MCP negotiation.

## Versioning

MCP protocol negotiation and `dfmcp` semantic negotiation are separate. Every session records:

- MCP protocol version;
- `dfmcp` protocol version;
- JSON Schema catalog digest;
- bridge protocol version;
- canonical schema version;
- DF/DFHack/bridge manifests;
- compatibility level.

No tool is callable before `fortress.open_session` completes.

## Tools

### `fortress.open_session`

Negotiates fortress, versions, compatibility, requested capability scopes, observation profile,
and hard budgets. Returns a concrete initial anchor and grants.

### `fortress.observe`

Accepts an exact cursor or no cursor, interest set, projection, freshness, and limits. Returns a
snapshot, delta, heartbeat, or reset. Partial results carry an opaque continuation tied to the
session and target anchor.

### `fortress.query`

Executes bounded structured DfQL at a concrete anchor. Query plans are statically costed.
Deterministic result ordering is mandatory.

### `fortress.plan`

Compiles an intent without effects. Returns an immutable plan digest, action DAG, affected scopes,
preconditions, postconditions, obligations, risks, capabilities, checkpoint policy, predicted
diff, and explanation.

### `fortress.commit`

Requires exact plan ID/digest, expected anchor, and confirmation seal when policy requires.
Revalidates before effects. Returns per-step action states, checkpoint receipt, obligations, and
evidence. Timeout may produce `indeterminate`.

### `fortress.wait`

Polls or follows plans/actions/obligations under bounded wall/game time. Returns meaningful
progress only and may provide continuation.

### `fortress.cancel`

Starts or advances request/drain/compensate/finalize. It never means “delete the record.”

### `fortress.checkpoint`

Creates and verifies a content-addressed recovery point.

### `fortress.restore`

Guarded global operation. Drains work, restores a sealed checkpoint, creates a new observation
epoch, and invalidates stale plans.

### `fortress.explain`

Returns evidence-backed rationale for a fact, score, plan, transition, compatibility decision,
error, or doctor finding.

### `fortress.doctor`

Checks bridge, compatibility, canonical state, ledger, active work, leases, checkpoints, indexes,
and replay. May propose a sealed repair plan but does not silently apply it.

## Common mutation state

```text
prepared
committing
applied_awaiting_verification
verified
compensation_pending
compensated
cancel_requested
cancelled
failed
indeterminate
```

Only `verified`, `compensated`, `cancelled`, and `failed` are ordinary terminal states.
`indeterminate` blocks blind retry and requires reconciliation.

## Resources

```text
df://session/{id}/summary
df://session/{id}/capabilities
df://fortress/{id}/anchor
df://fortress/{id}/entity/{entity}
df://fortress/{id}/map/chunk/{x}/{y}/{z}
df://fortress/{id}/events
df://fortress/{id}/plans/{plan}
df://fortress/{id}/actions/{action}
df://fortress/{id}/obligations/{obligation}
df://fortress/{id}/checkpoints/{checkpoint}
df://fortress/{id}/compatibility
df://knowledge/{document}/{span}
df://doctor/{bundle}
```

Resources are capability-checked; URI knowledge does not confer authority.

## Error behavior

Every error includes:

- stable code;
- human message;
- retry class;
- current anchor when known;
- affected scope;
- evidence/findings;
- recommended protocol next step.

Retry classes:

```text
never_unchanged
safe_read_retry
refresh_and_retry
rebase_required
backoff
reconciliation_required
operator_action_required
```

## Output shaping

Safety, continuity, action state, and uncertainty have priority over optional detail. The server
never truncate mid-object or omit a warning to meet a token budget. It returns continuations or
drill-down resources.

## Schemas

The schemas in `schemas/` freeze the naming and bounds of the logical surface. They are transport-
independent: the MCP wire layer renders the dotted tool names with underscores, and the modern-era
request routing is provided by the pinned `fastmcp-rust` facade rather than by this repository.
