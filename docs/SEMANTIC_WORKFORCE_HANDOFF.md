# Semantic SetLabor handoff

`dfmcp_adapter::semantic_workforce` connects one original `PreparedPlan` containing
`SetLabor` to the existing `WorkforceSession` coordinator. It retains the original
semantic plan and its proof association across preparation, native execution,
and restart. It uses the existing workforce wire protocol and native journal;
it adds no bridge method, generic executor, MCP route, evidence issuer, source
field grant, or admitted live compatibility profile.

## Supported semantics

The original plan must contain exactly one normalized `SetLabor` step, at most
32 selected canonical units, and one bounded labor key. The existing
[evidence-bound routing resolver](LIVE_SEMANTIC_ROUTING.md) resolves those units
through an eligible V1 or spatial identity projection and requires an exact
single-labor work-detail mapping. The original semantic key becomes the native
assignment key; the wrapper does not invent a second idempotency domain.

Only preconditions protected by the exact native capture are accepted: `True`,
`Paused(true)`, selected-unit existence and kind, equality of its exact native
identity field, and bounded conjunctions of those predicates. Other facts may
remain original postconditions or obligation predicates, but cannot authorize
this mutation through a workforce-only capture. Mandatory checkpoints,
compensations, step dependencies, multiple steps, and oversized plans are
refused. The wrapper supplies no cross-family lease or checkpoint workflow.

Original postconditions, the plan terminal condition, and the optional
`ObligationSpec` are retained unchanged. Their deadlines and stability windows
are not reset. `original_goal_proven()` always returns false: this owner does not
run the original goal monitor. `pending_obligation()` exposes the original
obligation for a separately authorized monitor.

## Acquisition and review

1. The operator opens the existing native workforce journal and session. Its
   verified journal ID is used when opening a separate `AssociationStore`.
   `open_private_association_store` supplies an operator-configured durable file
   through the existing locked private-file implementation. Initialization is
   exclusive; reopening never creates a missing file.
2. Construct `SemanticWorkforceSession::new(native, associations, context)`.
   Every existing native key must already have its exact semantic association.
   An empty, missing, replaced, or differently bound semantic store cannot adopt
   pre-existing native work, even if an assignment digest happens to match.
3. Supply a `WorkforceEvidenceOwner`. Its observing shell owns canonical
   acquisition and independently issued `PredicateEvidence`. It must refresh
   **after** the native capture and return a borrowed, eligible routing scope.
   This API neither manufactures grants nor accepts caller assertions as proof.
4. Call `observe(original_plan, evidence_owner, context, native_factory)`.
   The existing native session owns the selected workforce capture. The wrapper
   then refreshes canonical evidence, resolves the exact original step, and
   returns an immutable `SemanticWorkforceReview`.
5. Review `original_plan()`, `native_plan()`, and `seal()`. The seal binds both
   plan digests, the original anchor and source digest, native capture witness,
   step and key, and the exact native journal incarnation. A native plan digest
   alone is not this review seal.

## Preparation and execution

`prepare(seal, evidence_owner, context, native_factory)` first durably appends
and syncs the immutable semantic association, then calls the existing native
coordinator. It re-observes the original native capture and independently
refreshes semantic evidence. A second semantic guard runs immediately before
native preparation, after the native journal has synced its Intent record.

`commit(seal, confirmed, evidence_owner, context, native_factory)` requires the
exact attached seal and explicit confirmation. The existing coordinator again
checks the complete original native capture and syncs DispatchStarted. The
wrapper then refreshes independent canonical evidence and rechecks the original
preconditions, source binding, current canonical-unit scope, and deadlines at
the final native effect boundary. Only that guarded path calls the existing
assignment setter. An evidence failure after DispatchStarted preserves native
uncertainty; it never grants retry permission.

Native preparation/control and association writes currently require unexpired,
non-limited whole-fortress `Query`, `Plan`, and `ConfigureLabor` grants, with no
entity or map restriction. Their internal empty-entity authorization cannot
consume a selected-unit-only write grant. A review read can succeed with
correctly selected write scope, but its prepare/commit calls still refuse.
This wrapper does not synthesize wider grants. It
additionally authorizes `Plan` and `ConfigureLabor` at Guarded risk over the
original canonical units at every control boundary. No authority or confirmation
is restored from an on-disk review seal. Query-only Recover reopening remains
available for historical readback.

`SingleLaborResult::Verified` means a historical native Applied receipt proves
the requested value for that exact labor while preserving all other captured
labor columns. Applied receipts with additional labor changes are exposed as
`AppliedOutsideSemantics`. Pending, refused, cancelled and Unknown records stay
`Unverified`; native status and receipts remain available separately. None of
these results proves the original goal or current game state.

## Restart and bounded recovery

After reopening, `inventory` exposes historical native records without granting
semantic control. `reattach(exact_original_plan, context)` requires the original
prepared-plan digest, anchor and step to match the immutable association and
requires the matching native journal record. Changing preconditions or an
obligation under the same native key is refused. If a crash left the association
durable before native Intent, the exact original plan must be observed again;
reattachment cannot synthesize the missing native capture selection.

`reconcile` queries the original native key through the existing coordinator.
A Query-only native Recover session can reattach and interpret historical
readback without becoming a Control session. Settled duplicate operations and
permanent Unknown use the native coordinator's existing no-reconnect rules.
No dispatch retry is created by a lost acknowledgement or process restart.

`cancel` checks current original-unit scope at entry and at native cancellation.
It permits cleanup after original plan or obligation expiry, without rerunning
original preparation or requiring fresh semantic predicates. Existing native
rules still decide whether cancellation is local, requires a native retirement
request, or must preserve permanent Unknown. Read-only association stores deny
prepare, commit and cancel even if paired with a native Control session. They
permit history and Query reconciliation only, including when a failed sync left
a complete readable frame.

Association replay is strict: at most 64 immutable keys, canonical hash-chained
frames, native journal and fortress binding, and no tail repair. Append or
custody failure fences the open handle. Mutable reopening syncs complete
surviving bytes; read-only reopening does not claim they became durable.

## Work bounds and validation scope

Each operation has one absolute wall deadline. Every factory, observation,
preparation, commit, query and cancellation receives its shrinking remainder;
already reserved native byte budgets are preserved rather than replenished.
The outer allowance reserves native connection and two RPC costs plus bounded
journal work before entering the coordinator. At most two evidence refreshes
receive separate 16 MiB reservations. A 1 MiB local reservation covers the
bounded original plan, retained reviews and association work: a full 64-key
store is at most 26,832 bytes, a new retain costs at most `6 * length + 4096`,
and no call performs more than twelve additional full store verification reads.
Native journal views and final readback consume the same outer allowance.
The conservative default 4 MiB byte allowance is insufficient for a native
handoff call; callers must explicitly budget the connection, RPC and evidence
work. The development fixtures supply 128 MiB, with all nested reservations
charged against that one allowance.

The module's regressions use the actual Rust workforce coordinator and strict
native codecs, injected canonical evidence, memory crash boundaries, and the
existing real private-file backing. They verify adapter handoff semantics;
they do not qualify a real Dwarf Fortress/DFHack build or admit a live profile.

The final production adapter source passes
`cargo check -p dfmcp-adapter --lib --offline --locked`. The twenty new regressions
remain unexecuted: the focused unit build was killed by shared-memory exhaustion
before tests ran. Scoped formatting checks pass. These checks do not establish
full workspace, native or live qualification.
