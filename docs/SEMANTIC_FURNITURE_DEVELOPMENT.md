# Original furniture construction development runtime

This isolated runtime connects one original canonical `Build` plan to exact-item
native placement, durable recovery and receipt-linked proof of construction. It
supports a single-tile **bed, chair or table**, one original goal and one private
custody bundle per process session. It retains the frozen eleven-tool MCP surface.

This is an **unadmitted development profile**. Component tests are not a native
DFHack build, a live fortress campaign, a qualified stdio executable or
compatibility admission. See [implementation status](../IMPLEMENTATION_STATUS.md)
for the exact evidence available.

## What the result means

These claims are separate throughout the response and recovery workflow:

| Claim | Evidence and meaning |
| --- | --- |
| Historical placement | The original native key returned a linked building/item/job receipt. Registration alone does not prove construction. |
| Current construction | A fresh, complete, paused operations capture is bracketed by queries for that same receipt. The original building and installed item must agree, construction must reach its final stage, and construction/removal jobs and item-job links must be absent. |
| Original goal achievement | All original postconditions and terminal predicates satisfy the original deadline, cadence and required stable samples under independently issued evidence policy. |
| Historical first achievement | The original goal was satisfied at a retained qualified anchor. A later contradiction or restart does not make that anchor current. |

Unknown completion is reported as unknown. A failed publication, ambiguous native
effect or fenced source cannot become a new current proof. The goal's canonical
building identity is derived from the original action key; no replacement goal,
building identity or renewed deadline is substituted.

## Operator configuration

Build/run the dedicated source entry point in an environment containing the
repository's exact pinned dependencies:

```bash
cargo run --locked -p dfmcp-mcp --bin dfmcp-live-semantic-furniture-dev-server
```

This command identifies the entry point; it is not a claim that the complete
FastMCP executable or its transport has been qualified.

The process accepts only these `DFMCP_*` environment names:

| Variable | Value |
| --- | --- |
| `DFMCP_ALLOW_UNADMITTED_SEMANTIC_FURNITURE` | Exactly `1`. |
| `DFMCP_SEMANTIC_FURNITURE_CONFIG` | The bounded JSON object below, as the environment value itself, not a filename. |
| `DFMCP_SEMANTIC_FURNITURE_ALLOW_PLACE` | Exactly `1` when the operator enables initial placement. Omit for recovery-only operation. |
| `DFMCP_SEMANTIC_FURNITURE_SPATIAL_TOKEN` | Credential for the independently pinned spatial/1.8 source. |
| `DFMCP_SEMANTIC_FURNITURE_BUILD_TOKEN` | Credential for the furniture/1.19 source. |
| `DFMCP_SEMANTIC_FURNITURE_OPERATIONS_TOKEN` | Credential for the operations/1.4 construction source. |

Tokens must contain 32–256 UTF-8 bytes and no NUL. Supply them through the
operator's secret handling, not tool arguments. Offline reopening does not need
native credentials. Production admission environment and other `DFMCP_*` names
are rejected. Configuration and credential continuity are checked again at
bounded native I/O boundaries.

The following values are **illustrative**, not discovered source identities.
Replace the world, software versions, generation counters, map dimensions and
scope with independently obtained values for the exact running sources:

```json
{
  "schema": "dfmcp.semantic-furniture-development/1",
  "directory": "/absolute/private/furniture-goal",
  "endpoint": "127.0.0.1:5000",
  "initialize": true,
  "sources": {
    "kind": "operator-development-source-acceptance/1",
    "world_folder": "region1",
    "site_id": 1,
    "df_version": "OPERATOR_OBSERVED_DF_VERSION",
    "dfhack_version": "OPERATOR_OBSERVED_DFHACK_VERSION",
    "spatial_generation": 7,
    "furniture_generation": 3,
    "operations_generation": 11,
    "map_dimensions": [96, 96, 32]
  },
  "scope": [8, 8, 5, 23, 23, 5],
  "protected": [[18, 18, 5, 20, 20, 5]],
  "checkpoint": "disposable-fortress-no-checkpoint"
}
```

The generation counters belong to three independent source namespaces. Their
values need not match. The endpoint must be canonical numeric IPv4 loopback with
a nonzero port. The scope and every protected cuboid must be ordered and inside
the pinned map; at most 32 protected cuboids are accepted. The target's full
3-by-3 placement halo and the original ground item must fit the operator's
retained scope and construction lease without intersecting protected areas.

The custody directory must already be a real, owner-private, exact-mode `0700`
directory. The runtime owns these fixed, regular, single-link, exact-mode
`0600` files; symbolic links are rejected:

- `furniture.journal`: native effect identity and single-attempt coordination.
- `semantic.associations`: immutable original/native association.
- `original.plan`: complete original plan and archive identity.
- `goal.history`: bounded original construction samples and proof history.

Fresh initialization requires control mode, explicit placement enablement, the
disposable-fortress checkpoint exception and a paused fortress. It performs a read-only native bootstrap before creating the
journal. Creation and reopening are separate operations; existing custody is
never silently reinitialized.

The default checkpoint setting is `required`. This profile has no checkpoint
provider, so it refuses fresh journal initialization and new planning under that
setting. The explicit
`disposable-fortress-no-checkpoint` option is a trusted operator decision for a
disposable development fortress, made before the original plan is sealed. MCP
input cannot supply that exception or remove a sealed checkpoint requirement.

## Complete tool workflow

1. **Open.** Call `fortress.open_session` with `mode="control"` for a fresh
   operator-enabled goal. The default mode is `offline`. Preserve the returned
   session ID and journal identity.
2. **Observe.** Call `fortress.observe` with that session ID while the fortress is
   paused. This publishes one complete canonical spatial observation and its seal.
3. **Choose an item.** Use `fortress.query` with a JSON string such as
   `{"mode":"items","expected_observation":"<exact seal>","offset":0,"limit":32}`.
   Select the canonical decimal item identity from that exact observation.
   A caller cannot submit an arbitrary native item ID.
4. **Plan.** Pass the closed JSON request below to `fortress.plan`. Save the
   returned complete recovery object and inspect the paired original/native
   review. Planning prepares the native action but does not place it.
5. **Commit.** Call `fortress.commit` with the exact returned `paired_seal` and
   `confirmed=true`. Current operator authority, original custody and fresh
   spatial evidence are rechecked at the dispatch boundary. A placement receipt
   is historical registration evidence.
6. **Wait explicitly.** Call `fortress.wait` to reconcile an unsettled placement
   or initialize/replay construction history. Those are separate bounded calls;
   a later wait acquires one fresh construction sample.
7. **Continue construction.** Advance game time externally, pause again and call
   `fortress.wait` at or after the original cadence. Repeat only while the
   original goal and deadline allow it. Inspect current and historical results
   with `fortress.query` or `fortress.explain`.

The plan request below is illustrative. Replace the seal and item with the exact
returned values; this deadline example assumes an observed tick of 500000:

```json
{
  "expected_observation": "<exact observation seal>",
  "kind": "bed",
  "item": "12345",
  "target": [12, 12, 5],
  "deadline_tick": 501200,
  "poll_interval_ticks": 10,
  "stable_samples": 2
}
```

The accepted kinds are exactly `bed`, `chair` and `table`. The deadline is an
absolute future game tick, at most one game year from the original anchor.
Cadence defaults to 10 ticks and stability to one sample; stability accepts
1–64 samples. The requested cadence and sample count must fit the deadline.
Unknown fields, selectors, native identities, paths, credentials, source pins
and policy are not accepted as plan input.

The canonical plan retains its default material selector and original
construction predicates. Exact item choice is additionally sealed into the
request identity and paired native review. Both must still agree when committing.

`wait` does not unpause the fortress, advance a clock, repeat placement, start a
background worker or continuously poll. Repeated reads at one game tick cannot
manufacture stable samples. An initial sample before the next cadence is due
cannot complete a stability requirement. Calling wait also retires the current
opening's permission to commit; finish the intended paired review/commit sequence
before switching to recovery.

The runtime intentionally retains one original goal. A new observation or plan
cannot replace it after publication. `fortress.checkpoint` and
`fortress.restore` return explicit unsupported-operation denials.

## Exact restart and lost replies

Keep the **complete returned recovery object** outside the custody directory.
It includes the journal ID, original semantic digest, original archive object
digest, full original anchor and whether goal history is present. Copy partial
publication diagnostics too when returned. Cached packets retain identities
during errors and response overflow; they do not become current proof.

Before restarting, use
`fortress.cancel(session_id, scope="session")` when the old process is available.
This releases its local file locks and sockets. It does not undo a native effect,
remove a building, cancel a construction job, erase history or delete custody.

Keep the same exact operator source configuration and fixed files. Set
`initialize=false`, add the exact top-level `journal_id`, and copy the complete
returned object into `recovery`. A journal with no original plan can reopen
using its exact journal ID alone. A retained original requires its original
recovery object; guessing a seal or anchor is not supported.

Choose the reopening mode according to the work required:

| Mode | Behavior |
| --- | --- |
| `offline` | Opens exact retained custody without native contact. Use query, explain or doctor. Wait is denied. |
| `reconcile` | Can query the original native key and acquire fresh construction evidence under current source authority. Does not acquire placement permission. |
| `control` after restart | Still cannot restore the lost original commit permit. Recovery remains read/reconcile/retire only. |

Online recovery requires the exact native incarnation and independent source
pins. Native reloads or changed incarnations are not silently accepted. A lost
commit reply is reconciled by its original key; neither Unknown nor an
indeterminate effect permits redispatch.

A recovered native Prepared record may be queried and then explicitly retired
with `fortress.cancel(..., scope="goal")`. Retirement does not remove an already
placed building. A permanently fenced history must remain fenced: preserve the
handoff and release the session instead of repeatedly waiting or replacing the
original goal. An ambiguous archive publication similarly preserves its exact
intended identity for inspection; a diagnostic flag cannot excuse missing or
mismatched bytes.

Recovery clears current construction proof and breaks an unfinished stability
streak. It retains the original cadence, deadline, consumed source floors and
historical first achievement. History is reauthorized independently under the
current authority tick; reading an older original archive cannot rewind an
expired Query grant. The explicit read-only archive authority-floor API retains
the original anchor unchanged while allowing that current authorization check.

## Bounds and implementation

Each call owns its work and has no detached task. Session opening defaults to
5000 ms and 2 GiB of logical byte-work; explicit allowances are capped at 60000 ms
and 32 GiB for longer history replay. The byte-work allowance counts bounded work
and repeated custody checks, not a promise to allocate that much resident memory.
Fresh runtime opening reserves 512 MiB of logical work before custody creation.
Together with opening inspection and the complete response, the minimum
negotiable opening allowance is 768 MiB plus 128 KiB; the 2 GiB default exceeds
that minimum. All of these reservations precede I/O.
The output allowance is 32768 proxy units. Game-tick allowance is fixed at zero.

Goal history is limited to 64 MiB and 512 events. A poll prepays its possible
records before contacting a source: read intent precedes acquisition, complete
captured evidence precedes policy, and publication precedes acknowledgement.
An interrupted read or failed append cannot be silently replayed as fresh proof.
Exhausted budgets refuse the operation while retaining recoverable identities.

Source entry points:

- [Original furniture coordinator](../crates/dfmcp-adapter/src/semantic_furniture.rs)
- [Construction proof and history](../crates/dfmcp-adapter/src/semantic_furniture/goal_history.rs)
- [Runtime and recovery](../crates/dfmcp-mcp/src/semantic_furniture_runtime.rs)
- [Operator/native composition](../crates/dfmcp-mcp/src/semantic_furniture_runtime/native.rs)
- [MCP tool facade](../crates/dfmcp-mcp/src/live_semantic_furniture_server.rs)

Automatic construction clock runs, a multi-step furnishing dispatcher, a
checkpoint provider, native/live qualification and production admission remain
outside this implemented slice.
