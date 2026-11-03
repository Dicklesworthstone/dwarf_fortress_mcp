# Job-control MCP integration status

## Current evidence and preparation lifetime

The composed eight-class policy and exact `4ed1091` Rust source are described
in `IMPLEMENTATION_STATUS.md`; the four-class live10 result below is historical.
AG now proves one real current-source `ConstructBed` suspend/resume pair through
modern MCP, using actual GPT-6.1 Terra medium under NTM. Root independently
replayed 18 exchanges, six canonical journal frames and both Applied receipts;
native captures show flags `0 → 2 → 0`, all other captured fields unchanged
and zero ticks. Offline bytes and complete host cleanup were also checked.
Bead `.8.1` closes only this one-class live scope; seven policy classes remain
live unproven, and neither fortress competence nor production admission follows.

AD's resume remains historically Prepared and uncommitted. AE separately
refused an expired preparation: its retained timing proves at least 80.129
seconds between the plan response and commit dispatch, beyond native 60 seconds.
Their partial/negative evidence remains separate from the AG pair.

The dotted names below are logical names. This profile's actual MCP wire names
use underscores (`fortress_plan`, `fortress_commit`, and so on); use the exact
names returned by `tools/list`.

## 2026-10-31 positive native adapter repair

The public control loop already existed, but the real-SDK adapter deliberately
reported every job unsupported after fictional SDK calls were removed. The new
closed policy in `JOB_SUSPENSION_CONTROL.md` enables four ordinary furniture-job
types at completed workshops and restores the sole Boolean setter. The actual
SDK build succeeds; handler tests cover positive effects as well as refusal and
replay. Exact R23 source now passes full qualification and verifier; the live10
public MCP trial proves suspend/resume of one real eligible job under separate
reviews. Independent native capture, receipt and raw exchange replay are recorded
in `JOB_SUSPENSION_CONTROL.md`. None of these results grants admission or proves
later production completion.

The implementation-era unexecuted statements below are historical. R21's full
workspace qualification covers its exact prior Rust source, not the later native
adapter or this updated worktree.

## Session-bound control loop

`dfmcp_mcp::job_control_session::JobControlSession` now connects selected native
job observations to the existing sealed `SuspensionPlan` and durable
`JobControlJournal`. The separately gated
`dfmcp-live-job-control-dev-server` exposes this loop through the frozen eleven
MCP tools. It is not a production runner or a live-game qualification.

Every operation takes current `OperationContext` authority and checks session,
fortress and journal custody. The constructor accepts already opened custody and
an optional injected native source; it never creates grants. Offline inspection
can therefore retain no credentials or connection.

`observe` acquires one selected job, validates its complete evidence and exact
source manifest, checks authority against the new tick, and invalidates the old
selection on a failed refresh. The observation is not a canonical world snapshot.
`plan` constructs its digest and native token server-side from that retained
observation, a stable ASCII key and the desired suspension bit. Callers cannot
supply arbitrary native plan bytes. Preparation is synced before acknowledgement;
exact key replay does not call the bridge or renew its native lifetime.

`commit` requires the retained plan digest, exact expected witness, current
selection and ConfigureProduction authority. It delegates the only setter edge
to the coordinator, which syncs DispatchStarted first. A successful or uncertain
attempt invalidates selection. Terminal replay performs no native call. Unknown
outcomes for any retained key block preparation and dispatch of other keys until
reconciliation. A failed acknowledgement with uncertain custody or dispatch
state is reported conservatively as EffectIndeterminate, not as proof of no effect.

`reconcile` requires only Query and never reaches prepare or commit. Prepared and
terminal records are returned without a native query; polling a preparation does
not retire it as unknown. Existing writable Query-only recovery can query and
retain evidence after restart. Offline read-only recovery reports uncertainty
without contacting DFHack. Cancellation durably retires a still-prepared key; it
is neither a native cancellation nor an undo operation.

Twenty-one Rust regression groups are registered: eight orchestration scenarios,
six presentation/cursor scenarios and seven runtime policy/wiring/context scenarios.
They cover sealed replay, failed refresh, authority and budgets, cancellation,
ambiguous dispatch, Query-only restart recovery, absent receipts, cursor rebinding,
output reservation, development gates and inherited runtime restrictions.
At implementation time these groups were unexecuted because Rust, Cargo and
rustfmt were unavailable there. The later exact `4ed1091` full qualification
executes the workspace. The original Python-only checks separately covered
lexical/source wiring, native-fixture identity, 189 pagination reference cases
and modeled worst-case JSON sizing: 7,747 bytes below the 12 KiB reservation.
Those Python results remain distinct from Rust, MCP, custody and live evidence.

## Recovery discovery foundation

The isolated job-control/1.9 library provides `JobControlJournal::summary`
and `records_page`. Both require current Query authority and valid file custody.
A page holds at most 64 complete records, obeys caller row/byte/time bounds, and
includes terminal records as well as unfinished work. Ordering is ASCII key order.
`expected_head` binds discovery to one journal generation; `next_after` is the last
returned key only when more records remain. The MCP layer must additionally bind
its opaque continuation to the session and journal identity.

Summaries distinguish Prepared, reconciliation-required, and terminal counts.
They describe retained coordination evidence, not the current game or completed
production goals. Identity checks are not a full reread or protection against a
malicious same-user rewrite of already-cached bytes.

`open_private_job_reconciliation` opens an EXISTING private, exclusively locked
journal with Query authority and a writable descriptor. It neither creates a new
file nor repairs existing bytes. Its journal can append reconciliation evidence;
prepare/commit/cancel still require independently supplied ConfigureProduction on
every call. This opener grants no capability. `open_private_job_recovery` remains
read-only and cannot append even with a later mutation grant.

## Development executable and operator authority

Run `cargo run --locked --offline -p dfmcp-mcp --bin dfmcp-live-job-control-dev-server`
only in a checkout with the pinned owned dependencies and required nightly toolchain.
The runtime requires `DFMCP_ALLOW_UNADMITTED_JOB_CONTROL_V1_9=1`, refuses admitted
provenance, and rejects every other `DFMCP_*` variable except its five configuration
variables below. It does not alter or enter the production dispatcher.

The operator supplies `DFMCP_JOB_CONTROL_JOURNAL` as an absolute normalized path
under an existing exact-mode 0700 directory. Existing journals must remain
single-link exact-mode 0600 regular files under exclusive custody. The operator
also supplies `DFMCP_JOB_CONTROL_FORTRESS_ID`, the canonical nonzero decimal lineage
ID from the existing read profile (`job_fortress_id` uses the same world-folder and
site identity). No tool argument can select a path, fortress, endpoint, credential,
protocol, raw native command or Lua program.

`fortress.open_session` accepts `mode` and bounded wall/byte/output allowances:

- `offline` is the default. It opens an EXISTING journal read-only, grants only
  Query and reads no endpoint or credential. DFHack need not be running.
- `reconcile` opens an EXISTING writable journal under Query only and establishes
  one fixed job-control/1.9 connection. It can acquire selected observations and
  query/retain receipts, never prepare, commit or cancel an effect.
- `control` additionally requires `DFMCP_JOB_CONTROL_ALLOW_PRODUCTION=1`. Only this
  mode receives fortress-scoped reversible ConfigureProduction authority and can
  create a new journal. Removing that enablement removes production authority
  from subsequent calls; enabling it later cannot promote a Query-only session.

Connected modes require `DFMCP_JOB_CONTROL_TOKEN` (the native 32..256-byte secret)
and optionally `DFMCP_JOB_CONTROL_ENDPOINT` (numeric loopback, default
`127.0.0.1:5000`). The session incarnation provides a fresh nonce. There is no
implicit reconnect or retry. After a fenced connection, close and reopen explicitly
in reconciliation mode; the native plugin incarnation must still match retained
plans for a receipt to establish their outcome.

Only one session owns the runtime slot. `fortress.cancel` with `scope="session"`
and neither key nor digest drops custody and the connection without changing,
cancelling or forgetting journaled effects. It remains available when the journal
is fenced. The slot is not made available until owned resources have been dropped.
Session handles are process-scoped and never alias a reopened session.

## Eleven-tool loop

Use `fortress.query` first to discover retained work. Its `state` is `all`,
`pending` (prepared plus unresolved) or `reconciliation_required`. It returns
complete records including their key, plan digest, original observation, desired
suspension, native outcome and receipt digest. `fortress.explain` selects an exact
key/digest without a native call. `fortress.doctor` reports custody-checked journal
health, retained counts and connection presence, not connection health or admission.

In control mode, call `fortress.observe` with `native_job_id`. Then call
`fortress.plan` with that ID, `suspended`, a stable `idempotency_key` and the returned
`expected_witness`. The plan response contains the server-produced `plan_digest`.
`fortress.commit` requires the key, that digest and the expected witness. Recovery
of an uncommitted preparation after reopening requires selecting the same exact
job evidence before commit; changed evidence requires a different plan, not a
rewritten digest. Native preparation expiry remains authoritative.

Review the complete `effect` returned by Plan: its exact key, digest, expected
witness, desired suspension and original native observation. Plan and Explain
render that same record; Explain supplies no additional commit authority.
After that review, commit promptly within the native **60-second** preparation
lifetime. Replaying Plan does not renew it. An additional model reasoning turn
for Explain can exhaust the lifetime, as AE demonstrated. Explain can instead
inspect the retained receipt after Commit or recover a known preparation when
its original reply was lost.

Clients that automate this short critical section must validate the actual
Plan response and original intent before sending the exact Commit request.
They must stop on mismatched identity, witness, desired value, ineligible or
unpaused observations, unexpected errors or uncertain responses. They must
never extend a preparation, invent success, or retry an indeterminate effect
under another key. Native expiry, revalidation and authority remain decisive.

`fortress.wait` performs ONE bounded foreground receipt query for an exact key and
digest. It never sleeps, polls in a loop, dispatches, or treats absence as proof of
non-application. `fortress.cancel` with `scope="effect"` requires an exact key/digest
and production authority, and can only retire a still-prepared effect. Checkpoint
and restore remain registered but explicitly unavailable. No top-level tool is added.

## Budgets, pagination and evidence scope

Sessions default to 5 seconds, 65 MiB of bounded work and 32,768 output-token proxy
units; maxima are 60 seconds, 65 MiB and 65,536 proxy units. Output uses the existing
four-byte-per-token accounting convention, not a tokenizer-derived token count.
Query accepts further byte/output narrowing and wait accepts wall-time narrowing.
A 12 KiB envelope plus 12 KiB per whole output record is reserved before native
work. Insufficient response budget cannot dispatch an effect. Uncertain acknowledgements
retain explicit recovery warnings instead of truncated JSON. Synchronous filesystem
calls have cooperative accounting, not a claimed hard cancellation bound.

A query page has 1..16 whole output records (default 8) and scans at most 64 journal
records, further limited by remaining byte/entity allowance. A filtered page may
have no matches and still have a continuation; that is not absence. Exact summary
counts and explicit complete-set flags disambiguate this case. Continuations bind
the session, journal incarnation, exact head, state filter and requested page size.
Up to 64 issued cursors are retained to support retries after lost page responses;
expired cursors require restarting discovery. A head change always requires a
restart. No client-supplied token becomes an arbitrary journal offset.

Every response uses the common Agent Turn builder with explicit unadmitted status,
retained active-work counts, omissions and a discovery path. Where available, the
anchor is explicitly a selected native job identity, never a canonical world anchor.
Otherwise the anchor is absent and uncertainty is explicit. Native terminal receipts
prove their bounded historical outcome, not current live state, work completion,
production goals, or permission for another operation.

The original integration increment left native wire generations, the production
runner map, dependencies and registry unchanged. It did not itself establish
native build, live campaign, full repository qualification or admission.
The exact later qualification and narrow live evidence are recorded above;
production admission remains absent.
