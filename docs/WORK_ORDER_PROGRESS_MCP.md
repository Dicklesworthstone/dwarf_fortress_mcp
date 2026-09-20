# Work-order progress through the read-only MCP loop

This batched profile is separate from the single-order progress/1.11 native
reader added on upstream main in `33e9abf0f875948e0ec79472c0625661651f3c09`.
It leaves `dfmcp_order_progress_v1_11`, `DFMOP011`, its two methods and its evidence
unchanged. Batched `DFMWP012` has its own 1.12 package, method binding and session
namespace; the codecs are not interchangeable and admission does not transfer.

`dfmcp-live-work-order-progress-dev-server` connects the separate 1.12 native
reader to bounded Rust evidence decoding, endpoint comparisons and the existing
eleven-tool waist. This is unadmitted development source, not a production
runner. Rust compilation, test execution, MCP transport execution and live DFHack
qualification have NOT been performed in the implementation environment.

## What this adds after creation

The creation/1.10 runtime proves immediate insertion and retains its own durable
receipts. The new progress/1.12 runtime observes current order presence, native
validation/activity flags, finite-template recognition and remaining-work counters.
A caller can use a native ID returned by the creation runtime to select current
progress, but this does not cryptographically or semantically link that current
object to the old creation receipt across a restore or plugin restart. Responses
always leave `historical_creation_identity_proven` and
`production_completion_proven` false. Progress reads never resolve or rewrite an
indeterminate creation journal record.

Recognition is intentionally conservative. An unrecognized row may reflect a
custom order, a changed setting or native fields outside this supported slice;
it does not prove unauthorized editing. Raw selected fields remain visible.
Neither an inactive order nor its condition counts prove a causal blocker or a
material shortage. A disappeared order is not silently labeled complete.

## Operator configuration

The server and native plugin independently require the exact opt-in
`DFMCP_ALLOW_UNADMITTED_WORK_ORDER_PROGRESS_V1_12=1` and the separate
`DFMCP_WORK_ORDER_PROGRESS_TOKEN` (32..256 bytes). The server additionally uses
`DFMCP_WORK_ORDER_PROGRESS_FORTRESS_ID`, a canonical nonzero decimal folder/site
lineage ID, and optional `DFMCP_WORK_ORDER_PROGRESS_ENDPOINT`, numeric loopback
with a nonzero port (default `127.0.0.1:5000`). Only these four DFMCP names are
accepted by the server. Production/admission state and other profile variables
are refused. No request selects a credential, endpoint, fortress, native method,
file path or protocol. This profile grants only Query and Observe.

The explicitly selected binary is:

```sh
cargo run --locked --offline -p dfmcp-mcp --bin dfmcp-live-work-order-progress-dev-server
```

This is the entry point after applying the source changes in a complete checkout
with its pinned dependencies; it is not evidence that this binary was compiled.

## Tool flow

`fortress.open_session` accepts `native_order_ids` (1..32 distinct IDs), plus
optional session wall/byte/output allowances. It sorts the selection and rejects
duplicates. It negotiates the fixed two-method native protocol and obtains one
complete capture before publishing the session. A failed bootstrap publishes no
session. The response provides a session ID, current observation witness and a
ready-to-use wait request.

`fortress.observe` obtains one new capture. Optional `native_order_ids` replace
the selection; comparison resets instead of joining different selections.
`fortress.query` inspects the cached capture with no native call, optionally
requiring `expected_witness`. Cached data never claims current freshness.

`fortress.wait` requires the exact current `expected_witness` and optionally a
narrower wall allowance. It performs ONE foreground capture and comparison, not
background polling or clock advancement. Repeating a wait after a lost response
with an old witness fails before another read; query the retained capture to
resume. Comparisons are against the server's previous sample, not an asserted
client-acknowledged cursor or continuous game history.

The selected row phase is one of:

- `absent`: no matching ID in the complete validated queue scan;
- `unrecognized_or_modified`: outside the recognized finite-wood template;
- `awaiting_validation`: recognized template, validation flag not set;
- `validated_inactive`: validated, nonzero remaining, active flag not set;
- `active`: validated, active, nonzero remaining;
- `reported_zero_remaining`: validated recognized template reports zero.

None of these phases is a discharged production obligation or inventory proof.
Changes distinguish appearance, disappearance, configuration/recognition changes,
status changes, counter decreases and counter increases. Decreases are exposed
only while the recognized configuration and finite total agree at both endpoints.
Increases are separate reset/edit evidence, not negative production. Same-tick
changes can be observed but do not invent elapsed game time or distinct stable
completion samples. Replayed/reordered capture sequences are rejected even when
an older frame has a regressed clock. Incarnation/selection changes and monotonic
clock/allocation-horizon regressions never yield cross-boundary progress deltas.

`fortress.explain` returns the same retained evidence without a native call.
`fortress.doctor` reports local capture availability, not bridge-health proof.
`fortress.cancel` closes the read-only session, including after source failure or
operator revocation; it never cancels orders. Plan, commit, checkpoint and restore
remain registered but explicitly refused. There is no mutation-capable interface
under the progress reader or session.

## Context, lifecycle and bounds

Every semantic operation requires current session/fortress-scoped authority.
Cached reads evaluate grants at no earlier than the last observed tick. New
captures recheck Query/Observe against the newly observed tick before retention.
Failed native refresh clears the old capture and fences the connection. There is
no implicit reconnect; close and reopen explicitly. A new session starts without
a baseline, and no downtime continuity or durable monitoring is claimed.

The native queue scan is bounded to 4,096 entries and the selection to 32 whole
rows. This permits complete selected-presence evidence without returning the
entire queue. Unknown/unselected order configurations remain omitted. The binary
observation is at most 16 KiB; RPC frames at most 32 KiB; notifications at most
eight, 64 KiB each and 256 KiB total per call. Numeric loopback TCP reapplies the
remaining absolute deadline before every blocking operation. Connection and all
bootstrap requests share one deadline. Injected streams must honor that contract.

Session defaults are 5 seconds, 2 MiB work and 65,536 output-token proxy units;
maxima are 60 seconds, 2 MiB and 131,072 proxy units. The four-byte output proxy
is an accounting convention, not a tokenizer count. Response capacity of 16 KiB
plus 4 KiB per complete selected row is reserved before native work. Bootstrap
also reserves 1 MiB for negotiation and 384 KiB for its first read. No output is
truncated mid-object, and an inadequate request is refused before capture. An
unadmitted tiny request does not imply that a complete error fits a one-token
budget. Final rendering and acknowledgment deadlines are checked separately.

One session occupies a nonblocking, explicitly acquired process-local slot.
Inherited Asupersync I/O restrictions and cancellation are preserved at entry.
No detached worker, timer, watcher or task is created. Close drops the connection
while holding the slot; a concurrent operation receives a bounded busy refusal.
The common Agent Turn declares scoped active work, partial coverage, uncertainty
and the noncanonical selected-native anchor. Ambient production admission metadata
is removed even from error packets.

## Evidence

Twenty-four Rust test groups are registered: twelve codec/comparison/session,
six fragmented-wire/refusal and six MCP projection/policy/context groups. They
are uncompiled and unexecuted: Rust, Cargo and rustfmt were unavailable. No Rust
formatting, type checking, Clippy, workspace or MCP execution is claimed.

The independent Python checker validates the actual 129-byte native fixture,
rejects every truncated prefix, trailing data, six invalid selections and 25
invalid identity/template cases, and checks 4,800 recipe/amount/counter/status
combinations. Its conservative escaped-row plus change model is 2,343 bytes
(4,096 reserved); a 32-row page with envelope allowance is 90,376 bytes
(147,456 reserved). These are Python models, not Rust serialization or execution.

```sh
python scripts/check_work_order_progress_reference.py
python scripts/test_work_order_progress_native.py --ubsan --mutations
python scripts/test_work_order_progress_native.py --compiler clang++ --ubsan --mutations
```

Both C++ compilers pass 1,358 actual-handler assertions with explicit SDK/protobuf
doubles. Three separately compiled mutants removing complete-queue uniqueness,
static material recognition or unknown-field rejection fail executed assertions
under each compiler. This does not qualify real DFHack, generated protobuf,
CoreSuspender/events, a live fortress or production admission. Exact source hashes
are retained with the scoped reports under `docs/evidence/`.
