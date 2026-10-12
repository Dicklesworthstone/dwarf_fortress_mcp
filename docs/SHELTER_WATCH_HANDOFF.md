# Inspect a complete shelter group, then register its watch explicitly

Owning bead: `dfmcp-wp-pln03-labor-civilian-alert-fsm-zdm` (WP-PLN-03).

The existing native spatial/1.8 `shelter_status` query accepts an optional closed
`watch` specification. It returns a complete request for the existing foreground
watch engine, using every original selected citizen incarnation and shelter
region. Inspection does not register a watch, capture the game again, change
another watch, dispatch an effect or advance game time.

## Inspect and obtain the request

Pass `schemas/examples/shelter_watch_handoff_v1.json` as the object-valued `query`
argument to `fortress.query` in the existing spatial development session:

```json
{
  "schema": "dfmcp.query/1",
  "query": {
    "kind": "shelter_status",
    "citizens": [
      {"entity_id": "3298534883338", "generation": 1},
      {"entity_id": "3298534883339", "generation": 1}
    ],
    "shelters": [{"min": [0, 0, 5], "max": [1, 1, 5]}],
    "limit": 1,
    "watch": {
      "key": "joint-shelter-arrival",
      "deadline_tick": 1000,
      "poll_interval_ticks": 10,
      "stable_observations": 2
    }
  }
}
```

The example identities are synthetic. Use observed canonical spatial citizen
IDs and generations from the same session, and replace the example deadline
with a future absolute game tick within the negotiated game-tick horizon.
Shelter geometry is a desired location, not a native burrow or safety claim.

The usual positional rows and whole-group counts remain. The additional
`watch_proposal.arguments_json` is one JSON **string containing the full tool
arguments**, including `session_id` and the complete `dfmcp.query/1` envelope.
Parse that string and submit it as the arguments of `fortress.query` to register
its `watch`. This is not `fortress.plan` or a commit request. The comparison
`request_digest` is not a watch handle, proof, signature or authority token.

Registration binds the same session and exact `expected_anchor`. When the
observation has changed, the generated request refuses instead of being silently
retargeted. Reinspect and review a current request. The ordinary watch key binds
one immutable definition, so an existing key cannot be reused to change the
selection, deadline or sampling requirement. Idempotent registration at the
same anchor returns the existing handle and supplies no extra sample.

## The page is not the goal

Every page repeats the identical complete proposal. Its `shelter_set` contains
all original targets, including outside, missing and changed-generation citizens;
it never uses only the returned page or only currently arrived citizens.
Canonical ordering is derived from the original normalized `ShelterSelection`.
The native condition validator still checks the complete goal when registered.

The optional handoff also enters the page model identity. Changing its key,
label, deadline, cadence or required samples invalidates the continuation, as do
changes to the existing selection/source/work identity. Page size can change
without changing the assessment. Omitting `watch`, or setting it to null, keeps
the original status response shape, model digest and continuation behavior.

The full proposal is included in the summary before pagination. Complete summary
and at least one whole row must fit the request's output allowance; otherwise
inspection refuses. A scalar request is never shortened into a different roster.
No client should reconstruct a smaller registration from visible page rows.

## Await prospective joint samples

Retain `record.watch` from explicit registration. Use the existing `await_watch`
query to acquire one fresh coherent observation for a nonterminal watch, or
`poll_watch` to evaluate the already published observation without native contact.
These are foreground calls only. Existing watch batching can share one capture.
Prior shelter-status reads never become retrospective stability samples.

All original named living incarnations must be inside together in each eligible
sample. Repeating an identical capture cannot advance the streak. Completion is
historical sampled presence, not proof of continuous occupancy or safe shelter.
A completed watch does not keep sampling: later current positional truth must be
read with `shelter_status` or the same `condition_evaluation`. A later departure
can make current truth false while the original satisfied receipt remains intact.
See `SHELTER_ARRIVAL_WATCHES.md` for the exact predicate, cadence, gap, deadline,
identity, durable-custody and cancellation semantics.

Query/Observe grants and source fencing are still checked by the original server.
Producing or submitting a handoff grants neither of them. A failed or cancelled
watch does not release civilians, undo a burrow assignment or authorize an effect.

## Bounds and evidence

`watch` requires a nonempty key of at most 64 UTF-8 bytes and an absolute deadline.
The optional label defaults to the key and is bounded to 256 UTF-8 bytes; neither
may contain NUL. Cadence defaults to one game tick (1..1,000,000); required samples
default to two (1..64). An impossible earliest sampling span or a deadline outside
the current session horizon refuses before analysis. Other watch fields refuse.

Generated arguments are capped at 16 KiB. The same original 128-target/16-region
selection ceiling and native query work/wall limits apply. The generated watch
fits the existing watch input bounds, without enlarging them. Whole response
publication and native source/authority checks remain in the existing server.

Eight Rust regressions were added: full-target pagination, legacy identity,
continuation binding, malformed policies/output refusal, actual native handler
registration/await/heartbeat/completion/current inspection, stale registration,
source/Observe denial and cancellation, and maximum-selection schema discovery.
They are source-reviewed but not compiled or executed here. The preceding nine
condition tests are also unexecuted; there is no Rust/Cargo toolchain available.

Executed Python evidence: two schema metaschemas and the example passed; 41
malformed handoff shapes were rejected, two legacy forms accepted, and 16 full
handoff fixtures passed the condition schema and unchanged watch shape limits.
The maximum fixture uses 128 citizens, 16 regions, maximal generations, escaped
labels and wide identity/tick values: 547 nodes, 22,284 accounted request bytes
and 10,198 serialized argument bytes. The fixture generator is an independent
structural model, not execution of the Rust handoff builder. The preceding
44-malformed-condition schema suite also passed again.

Reversing only the new optional handoff hunks recovers the exact original
`shelter_queries.rs` Git blob. Removing the optional schema property recovers
the exact original status schema. No new top-level MCP tool, native protocol,
production runner or compatibility admission is introduced.
