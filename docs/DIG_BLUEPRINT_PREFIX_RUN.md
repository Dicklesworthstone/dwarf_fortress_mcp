# Confirmed original-prefix designation

Beads: `df-dfhack-bridge-plane-c-pic.4` and `.5` (partial progress).

`dig_blueprint_prefix.Prefix` binds an exact consecutive set of 1..8 NEW native
rectangles from the existing floor-blueprint batch. Its confirmation covers the
original batch, layout, complete inventory digest, keys, target geometry, hidden
context policy and the existing disposable-fortress checkpoint policy.

The confirmation explicitly selects fresh per-step native observation and
revalidation of that fixed target prefix. It is not a collection of pre-observed
native witnesses, an arbitrary future action grant, or proof that the remaining
terrain is safe. A changed frontier requires a newly reviewed confirmation.
No count is silently clipped, no room/rectangle is omitted, and no original key
is changed or reused. Original room-backed inventory remains bound as well.

The successor check allows exactly one additional original record. All earlier
receipts, original room intent, policy and source floors remain unchanged or
monotone as appropriate. Only `designated` permits another step. `unknown` and
`refused` stop progression, and stopped or completed batches cannot start again.
These checks supplement the existing private-file/receipt replay; untrusted
caller-authored inventory is not authority.

The pure policy has 22 passing Python tests. The foreground command described
below integrates it with the existing effect shell. No existing command, native
protocol, one-shot connection rule, journal format or admission gate is changed.
There is no live-game or Rust qualification claim.

```sh
python3 -m unittest discover -s tests -p 'test_dig_blueprint_prefix.py' -v
```

## Foreground execution

`scripts/run_dig_blueprint.py` now connects this policy to the existing native
coordinator. It accepts an ALREADY initialized original batch; it does not create
replacement batches, allocate keys, change the blueprint, or import a review as
authority. Both the legacy and room-backed formats use their existing original
manifest, source validation and effect custody. Existing `dig_blueprint_client.py`
commands and all native protocol/journal formats remain unchanged.

For an existing batch with at least two remaining rectangles:

```sh
python3 scripts/run_dig_blueprint.py review --directory "$BATCH" --steps 2

# Review the returned original blueprint and exact prefix. Copy batch_id and
# review.confirm_run from THAT result, not an older or different batch.
python3 scripts/run_dig_blueprint.py run --directory "$BATCH" --steps 2 \
  --batch-id "$BATCH_ID" --confirm-run "$CONFIRM_RUN"
```

`review` is offline and requires no native credentials. `run` uses only the
existing isolated dig/1.16 environment: `DFMCP_ALLOW_UNADMITTED_DIG_V1_16=1`,
`DFMCP_DIG_TOKEN`, the original `DFMCP_DIG_ENDPOINT`, and separate
`DFMCP_DIG_ALLOW_DESIGNATE=1`. Other DFMCP profile/admission settings are refused.
Credentials stay in the environment and never enter the command output.
The original `disposable-fortress-no-checkpoint` policy must already be present;
this command cannot waive a checkpoint or change hidden-context handling.

This is a different confirmation scope from the existing single-step review:
it approves a fixed original target prefix under fresh native revalidation,
NOT a previously observed collection of per-step native witnesses. Before each
new step, the command reads the current original rectangle and its native halo.
Known blockers stop before that step's intent. The existing coordinator then
rereads that same selection and requires the exact new witness before writing
intent. It synchronizes the original capsule and registry before prepare/commit,
checks original history at the native effect boundaries, and persists verified
terminal proof before another step may begin.

Each NEW step gets one distinct one-shot connection, with the previous connection
closed first. The existing native client's `commit_attempted` flag is never
reset. A failure, stale read, refusal, unknown outcome, changed authority, source
regression, or cancellation stops the whole call. There is no automatic reconnect
for the failed step, no query/cancel inside the run, and no skip to a later key.

One initial budget covers the entire call: 1..60,000 ms wall time, eight million
cooperative work checks, 11 RPC frames per selected step (maximum 88), and 4 MiB
native bytes per selected step (maximum 32 MiB). A complete next-step allowance
is checked before its connection. No allowance is renewed per iteration. The
full response is reserved before contact using the complete original inventory,
review and blueprint, plus bounded growth for every selected outcome. Final
serialization and owner release are followed by the current authority check.
These are cooperative limits, not hard realtime guarantees.

The original batch directory stays locked for the foreground call. `stop` can
prevent a future call but cannot acquire that same lock to interrupt a running
call. Interrupting the process closes its owned connections and stops subsequent
steps; already dispatched native work remains subject to original-key recovery.
No background worker, game clock advancement, rollback or native drain promise
is introduced.

## Outcomes and recovery

`run.stop_reason` distinguishes `prefix_completed`, `all_designations_verified`,
`preflight_blocked`, `native_refused` and `indeterminate`. The first two have
`ok: true`; the others have `ok: false` and exit status 2. A broader exception or
interruption returns `DIG_PREFIX_RUN_REFUSED`, with unknown effect status and no
unverified inventory presentation. A short stdout write is terminal and never
causes a second output object or another execution.

`steps_considered` includes a refused preflight; `steps_recorded` counts only
new original intents. `commit_attempts_acknowledged` counts calls with a validated
reply, not a universal count of effects after a failed call. The original batch
remains the only recovery owner. Earlier successful effects are NOT undone by a
later refusal. Even `all_designations_verified` is historical designation
readback, not completed mining, room readiness, current terrain or admission.

Inspect/recover through the unchanged original commands:

```sh
python3 scripts/dig_blueprint_client.py inspect --directory "$BATCH"
python3 scripts/dig_blueprint_client.py query --directory "$BATCH" \
  --batch-id "$BATCH_ID" --step "$ORIGINAL_STEP"
```

An uncertain original intent blocks a new review/run before any native contact.
After explicit reconciliation, only remaining NEW work can receive a new prefix
review. Any recorded step changes the inventory commitment and invalidates the
old prefix confirmation. A blocked preflight with no new intent may leave that
inventory unchanged; another call is still explicit and repeats fresh safety
checks. Never recreate a batch to evade an unresolved key.

## Executed test scope

52 focused Python tests passed: 22 pure confirmation/progression cases and 30
legacy-profile execution/custody/TCP/CLI cases. The latter use the actual existing
four dig modules, real owned private files, fsync, locks, actual native client
framing and one-shot dispatch, and joined synthetic peers. They cover a 512-tile,
eight-step run, exact sparse targets, a lost second commit, explicit query-only
recovery, replayed preparation, refusals, source/authority/budget failures,
interruption and sync failures, CLI short output, and SIGKILL after the peer's
first effect but before acknowledgment. No automatic next step or duplicate
original effect was observed in those tested schedules.

Validation used a component checkout. The four existing dig files were verified
against their Git blob identities. At their eager room-module import boundary,
a validation-only adapter supplied the unchanged bounded JSON reader and refused
room-handoff use. Therefore these runs qualify the legacy blueprint path only;
room-backed codec/transport integration is implemented through the unchanged
owner but was not executed end-to-end here. The adapter is not shipped or used
by this command. The committed tests use ordinary repository imports.

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts:tests python3 -m unittest \
  test_dig_blueprint_prefix test_run_dig_blueprint -v
```

There is no native DFHack producer, SDK/Rust build, live-fortress campaign,
full-repository qualification, or production-admission claim in this evidence.
