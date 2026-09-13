# Foreground condition watches

The protocol-1.1 development `fortress.query` tool can retain a bounded condition,
refresh one observation, and report whether the condition has become stably true,
failed, expired, or become unknowable. It does not mutate the game or add a tool.
These are process-local foreground watches, not autonomous background workers,
durable action obligations, or a claim that the protocol-1.1 runtime is admitted.

## Operations

Pass a `dfmcp.query/1` envelope in the existing `query` argument. Do not mix it
with top-level `mode`, `limit`, or `continuation` arguments.

| Query kind | Behavior |
|---|---|
| `watch` | Register a session-owned definition and evaluate the published snapshot once. |
| `poll_watch` | Evaluate the current published snapshot without bridge I/O. |
| `await_watch` | Validate the handle, refresh at most one adapter observation, then evaluate. |
| `watches` | List retained active and terminal watches without evaluating them. |
| `cancel_watch` | Stop local watch bookkeeping without a game effect. |
| `release_watch` | Release a terminal record; cancel active work first. |

`await_watch` is a bounded single observation, not a sleep or a loop until success.
Its `read_calls` count refers to `GameAdapter::observe`, not individual native RPC
pages: the existing adapter may assemble a complete observation from bounded pages.
It never unpauses the fortress or advances controlled game time. An unchanged
paused fortress can therefore remain waiting indefinitely in wall-clock time;
the watch's deadline is in game ticks. A terminal watch skips refresh entirely.
The separate `fortress.wait` mutation-work tool is unchanged by this slice.

## Example

Suppose the current anchor's game tick is 100, canonical citizen ID `23` has
generation 1, and the session permits at least 100 further game ticks. The
following value for the `query` argument watches the citizen's observed `sane`
field for two successful observations at least five ticks apart:

```json
{
  "schema": "dfmcp.query/1",
  "query": {
    "kind": "watch",
    "key": "citizen-23-sanity",
    "label": "Verify citizen 23 remains sane",
    "condition": {
      "op": "field",
      "entity_id": "23",
      "generation": 1,
      "field": "sane",
      "comparison": "eq",
      "value": {"type": "bool", "value": true}
    },
    "deadline_tick": 200,
    "poll_interval_ticks": 5,
    "stable_observations": 2
  }
}
```

Use the exact returned `record.watch` as the handle for `await_watch` or
`poll_watch`. Retrying registration with the same key and definition returns the
original record without incrementing its sample count. A different definition
under that retained key is a conflict. Release ends that idempotency lifetime;
a later registration receives a new handle and cannot resurrect the old one.

To recover handles after losing conversational context:

```json
{"schema":"dfmcp.query/1","query":{"kind":"watches"}}
```

To monitor the game clock instead of a field, the condition can be:

```json
{"op":"tick_at_least","value":150}
```

The schema is available through `mode="schema"` and in
`schemas/mcp_query_v1.json`. Use entity inspection to obtain canonical IDs,
generations, actual field names, and coverage; arbitrary native objects cannot
be used to widen the observed projection.

## Conditions and evidence

Conditions support `field`, `paused`, `tick_at_least`, nonempty `all`/`any`, and
`not`. Field comparisons use exact scalar types: null, boolean, signed/unsigned
integer, text, or fixed-point with the same scale. No scripting, expressions,
regular expressions, raw bridge methods, or callbacks are accepted.

A field can establish truth only when its known representations agree, its
source is `DfhackField`, its source digest is nonzero, and it was observed at the
current snapshot's game tick. Missing, absent, omitted, unsupported, redacted,
stale, conflicting, inferred, replayed, and agent-asserted fields are unknown.
Mismatched types or fixed-point scales are also unknown, including under `not`.
Known decisive boolean branches retain ordinary strong three-valued semantics.

A missing entity is unknown, not proof of death. A changed entity generation
invalidates the entire watch, even inside an otherwise decisive boolean branch.
This prevents an old handle from silently monitoring a replacement citizen.

The optional `failure_condition` is evaluated alongside the success condition.
Known failure takes precedence. Unknown failure blocks success. Failure, false,
and unknown results are noticed on every new observed anchor, even before the
next positive sample is due.

## Stability, time, and terminal states

Registration performs the first sample. Repeated reads of the identical anchor
cannot increase stability or rewrite evidence. Multiple anchors at the same game
tick cannot increase the positive sample count. Only distinct due samples count.
False/unknown evaluations and skipped published observation sequences reset the
streak. Fortress/epoch switches, regressed ticks or sequences, and same-cursor
forks invalidate nonterminal watches.

The statuses are `waiting`, `candidate`, `blocked_unknown`, `satisfied`, `failed`,
`expired`, `invalidated`, and `cancelled`. The last five are terminal. Proof at the
exact deadline may complete a watch; a later observation cannot establish late
success. Terminal records never resume or change their evidence. Their evaluated
anchor remains visible, and `evaluation_current` is false at a different anchor.

Each evaluation records bounded fact evidence, condition/failure truth, sample
count, streak, and a rolling evidence digest. This is not authentication, a
complete history archive, or proof that the condition held between samples.
The result and Agent Turn explicitly state sampled temporal coverage. Satisfying
a watch does not establish mutation success, causation, or completion of a game
objective beyond the exact observed predicates.

## Ownership, budgets, and publication

Every request requires the session's Query authority. A nonterminal `await_watch`
also requires Observe authority, validates the handle and optional pre-refresh
`expected_anchor` before I/O, and rechecks Query authority at the returned anchor.
The caller's original expected anchor is the refresh basis, not a demand that the
result remain at the old anchor. A complete snapshot or exact heartbeat is
required. Bridge errors cannot publish a successful sample.

Active watches appear in successful protocol-1.1 query Agent Turns under
`active_work.obligations`, explicitly tagged `foreground_observation_watch` with
`game_effect="none"`. Pure queries reserve this metadata before filling their
result pages. Authorized query errors add retained work when the complete error
packet fits. Other tools and runtime variants are not implicitly upgraded.

Listing, cancellation, and terminal release remain available with a poisoned
bridge and mark source continuity stale. They never interpret cached game facts
as fresh. There is no detached drain to await: cancellation changes only the
retained local record. Active records cannot simply be released and forgotten.

The engine permits eight retained watches per session and 128 per process. It
bounds input bytes/nodes, shared success/failure condition nodes (64), condition
depth (8), strings, positive sampling cadence, stability (1..64), and output.
No unbounded timer or polling task is started. Explicit terminal release reclaims
capacity; process restart discards these non-durable records.

Registration, evaluation, cancellation, and release build a bounded candidate
state. The complete response must render within the admitted budget before that
state is published. A rendering failure leaves the prior watch state intact.
A bridge refresh may already have published a newer observation independently;
retry `poll_watch` against that current observation instead of assuming the read
was rolled back. This distinction avoids losing evidence or inventing samples.

## Validation status

This tranche adds 25 registered Rust scenarios: sixteen condition-engine cases,
four one-observation adapter-boundary cases, and five dispatcher/Agent Turn cases,
including an 8192-byte full-response path. They cover positive completion,
unknown/type/provenance handling, failure priority, cadence, retries, missed
observations, identity, deadlines, authority, cancellation, retention, and failed
response publication. These Rust tests have not been compiled or executed here.

The edited self-contained JSON Schema passed 73 local cases (34 accepted and 39
rejected), while preserving all prior definitions and ten existing variants.
Source bytes were compared with their Git blob identities. Neither check is Rust,
stdio, native DFHack, live-game, admission, or release qualification evidence.
The editing environment has no Rust compiler, Cargo, or rustfmt.
