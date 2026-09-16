# Coherent foreground watch batches

The live spatial/1.8 server implements `poll_watches` and `await_watches` through the existing
`fortress.query` tool. These operations evaluate a selected watch set together rather than issuing
one observation and one publication for each watch. They remain unadmitted development source.

## Why a shared observation matters

Calling `await_watch` separately for several watches can advance the observation cursor between
samples of each individual watch. A watch that skips observations conservatively loses its stability
streak. The batch path preserves that safety rule while allowing every selected nonterminal watch
to see the same capture. It does not weaken cadence, failure guards, unknown-value handling,
entity-generation checks, deadlines, restart gaps or terminal-evidence immutability.

## Use the existing query tool

Evaluate every retained watch after at most one new coherent capture:

```json
{
  "session_id": "<live spatial session>",
  "query": {
    "schema": "dfmcp.query/1",
    "query": {"kind": "await_watches"}
  }
}
```

Use `poll_watches` instead to evaluate the latest already-published capture without acquiring one.
For example, an agent can call `fortress.observe` once, inspect the situation, then poll all watches.
Neither `fortress.observe` nor `fortress.wait` silently changes its existing sampling behavior.

An optional `watches` array selects one to eight distinct handles returned by registration or
`watches` discovery. Omission or JSON null selects the complete retained session set, including
terminal records. An explicit empty array is rejected. Handles are validated and sorted canonically;
unknown, duplicate, malformed and other-session handles fail before acquisition. `expected_anchor`,
when supplied on the envelope, must match the complete pre-acquisition anchor.

There is no page limit, continuation, caller-selected path, background switch, arbitrary condition
body or maximum-capture override on these requests. The live spatial schema advertises both query
variants. Archive-only and historical-query envelopes refuse them. Other development profiles are
not newly advertised as batch-capable by this increment.

## What a batch returns

The complete result contains one compact record per selected watch, with its stable handle/key,
status, stability counters, sample count, evaluation anchor, condition/failure truth when available,
and evidence digest. Per-record flags distinguish state advancement, an added sample, a new terminal
transition and replay of an already terminal result. Predicate witness trees and definitions remain
available through the existing single-watch `poll_watch` detail path.

Top-level counts include selected, advanced, sampled, terminal and remaining watches. `all_terminal`
means that no selected watch remains nonterminal; an empty session set is terminal in that sense.
`all_satisfied` is false for an empty selection and otherwise means every selected retained status is
Satisfied. It is **not** proof that all their conditions hold simultaneously now: terminal watches
are not resampled, may have different historical evaluation anchors, and retain their old evidence.
Neither field proves game-effect success or continuous truth between observations.

`native_captures` is zero or one. Awaiting a completely terminal or empty set performs no capture and
needs no Observe grant. An unfinished await requires both Query and Observe; polling requires Query.
Current grants are checked again at the new observation's tick. No authority is imported from a
watch definition, retained outcome or earlier observation.

A structured `next_step` repeats the selected set through `await_watches` when work remains. It omits
the old expected anchor. Calling it is another foreground pass, not a scheduled task. Selecting all
retained watches is the simplest way to avoid inter-watch sampling gaps; intentionally unselected
watches remain unchanged and may correctly lose stability if observations are skipped.

## Publication and recovery

The operation follows this order:

1. Validate current authority, source/custody, the complete selection and pre-acquisition anchor.
2. Bind a private single-use preparation to the session watch registry and selected handles; render
   the current result shape as an output-budget preflight without advancing any watch.
3. For an unfinished await only, acquire and publish one coherent observation through the existing
   spatial source and optional observation archive. Recheck authority and remaining wall budget.
4. Verify that the session watch registry has not changed across acquisition. Compute the entire
   candidate set using the existing watch transition function.
5. Render the complete Agent Turn, including current active work, persistence metadata and next step.
6. When configured, append/sync one existing-format watch checkpoint for the changed candidate set.
   Then replace the in-memory watch root and return the already-rendered result.

A registry conflict, predicate/counter failure, refused response or failed checkpoint publication
cannot publish only the first few watches. Unselected watches are carried forward unchanged.
Heartbeat/no-change and terminal-replay batches do not append a new checkpoint. Durable batches use
the existing paired journals and file custody; no new storage format, journal or configuration is
introduced. Complete but uncertain checkpoints recover as whole sets under existing recovery rules.

The observation archive and watch checkpoint are **not** one cross-file transaction. An observation
may already have been durably published when later evaluation, authorization, rendering or watch
storage fails. The watch set then remains unchanged; the observed world is not rolled back. When
appropriate, `poll_watches` can evaluate that retained capture without another read. Corrupt or fenced
storage must follow the existing reopen/recovery path, not automatic repair.

The preflight validates the current output shape, not every possible future outcome. A changed
capture or larger result can still exceed the final budget after acquisition. Final rendering is
always checked before watch publication; no partial response or empty-progress cursor is returned.
Large selections, long labels in active work, and recovered evidence may require a larger negotiated
output budget. The default 8,192-token budget uses the project's existing byte/4 accounting, not an
exact tokenizer. Do not confuse it with an 8,192-byte budget.

Preflight, capture, evaluation and rendering share a cooperative wall-time allowance. Synchronous
filesystem calls are not hard-preemptible. No fallible deadline check is performed after checkpoint
sync in a way that would hide the committed watch root. No detached thread, timer or background
monitor is started, and no simulation-clock control or game mutation is dispatched.

## Evidence for this increment

Seventeen logical Rust regression scenarios are registered: nine batch-engine tests and eight
actual spatial-handler/private-journal tests. They cover shared stability, heartbeat/cadence,
selection isolation, malformed/stale/foreign handles, no-Observe terminal batches, late evaluation
and rendering refusal, registry races, epoch/deadline/unknown outcomes, authority expiry, one-capture
and one-checkpoint behavior, bridge/storage failure, restart, archive refusal and eight-watch capacity.

These Rust scenarios have **not been compiled or executed in this editing environment**. Rust,
Cargo and rustfmt are unavailable, and no live DFHack instance is connected. Focused commands are:

```bash
cargo test --locked -p dfmcp-mcp query_watch::batch -- --test-threads=1
cargo test --locked -p dfmcp-mcp watch_batch::tests -- --test-threads=1
python3 scripts/test_watch_batch_contract.py
```

The Python request-schema checker passed 96 cases: 32 accepted and 64 rejected. The executed schema
and script Git blob identities match their committed bytes. Executed script SHA-256:
`5076e826fa9e9fdbb4440a828adb730a3d1315cb076893035dc61319c844da75`.
That result validates request shape only; it does not execute Rust, watch transitions, authority,
custody, capture, checkpoint durability, response rendering or MCP transport. No Rust qualification,
native/live qualification, production admission, dependency or game-effect change is claimed.
