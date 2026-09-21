# Workforce assignment through the eleven-tool MCP interface

`dfmcp-live-workforce-dev-server` connects the existing workforce/1.17 native
profile, typed adapter, durable journal and `WorkforceSession` to the existing
modern FastMCP/Asupersync runtime. No Python/shell wrapper, new native protocol,
new dependency, production runner, or compatibility admission is added.

**Evidence level: source plus independent Python model/lexical checks only.**
The 18 new Rust tests, generated handlers and actual MCP process have not been
compiled or executed in the authoring environment. The source depends on
`WorkforceSession` introduced in `e41ba26c180c65bda0c5b3eba4a9402313ce3c98`.
It is not a qualified or production-admitted executable.

## Operator configuration

Only these DFMCP environment names are accepted:

```text
DFMCP_ALLOW_UNADMITTED_WORKFORCE_V1_17=1
DFMCP_WORKFORCE_WORLD_FOLDER=region1
DFMCP_WORKFORCE_SITE_ID=7
DFMCP_WORKFORCE_JOURNAL=/absolute/private-directory/workforce.bin
DFMCP_WORKFORCE_ENDPOINT=127.0.0.1:5000
DFMCP_WORKFORCE_TOKEN=<operator-managed secret, 32..256 bytes>
DFMCP_WORKFORCE_ALLOW_LABOR=1
```

The endpoint defaults to numeric loopback port 5000. No DNS or remote plaintext
credential delivery is introduced. The game process needs its native workforce
opt-in, token and labor enablement as described in `WORKFORCE_NATIVE.md`.
The final journal directory/file require exact modes 0700/0600 and the existing
Linux private-file ownership rules. Paths, credentials, fortress selection and
labor enablement cannot be supplied through MCP arguments. Store secrets outside
command history. Other DFMCP state, including production admission, is refused.

The binary uses the repository-pinned runtime and toolchain:

```bash
cargo run --locked -p dfmcp-mcp --bin dfmcp-live-workforce-dev-server
```

This command is an entry point, not an assertion that this environment built it.

`fortress.open_session` defaults to `mode="offline"`. Offline opens an existing
read-only journal without reading endpoint/token or constructing a native source.
Recover opens existing writable custody with fresh Query-only authority; receipt
reconciliation is allowed, assignment and cancellation are not. Control requires
separate operator labor enablement and grants Query, Plan and guarded ConfigureLabor.
Modes never promote themselves when broader grants are later injected.

One process retains one session and one journal lock. Connections are explicit,
per-operation and dropped on return. Opening selects no citizens and makes no
workforce observation. The adapter owns native selection, current authority,
clock-floor enforcement and all commit/recovery decisions.

## Observe, inspect, review, commit

After opening a control session, discover unsettled records first. To observe:

```json
{"tool":"fortress.observe","arguments":{
  "session_id":"<session>","selection":"{\"unit_ids\":[2,5]}"
}}
```

Selection is a closed JSON string of at most 2 KiB with 1..32 sorted unique
nonnegative native citizen IDs. IDs above signed-32-bit range, duplicates,
reordering, boolean/float IDs, unknown and duplicate fields are refused.
The compact result reports selected identities, eligibility and the capture
witness, not thousands of membership rows.

Inspect the exact cached capture without another native call:

```json
{"tool":"fortress.query","arguments":{
  "session_id":"<session>",
  "query":"{\"mode\":\"details\",\"witness\":\"<capture witness>\",\"offset\":0,\"limit\":4}"
}}
```

Each details page returns whole work-detail records and one shared labor-column
catalog. Masks use this exact catalog order; raw labor enum inputs are not
accepted. `next_query` remains bound to the same capture witness. Detail indices
are capture-local positions, not stable IDs. Failed refresh invalidates both the
presentation copy and adapter-owned planning selection, including budget refusal.
The adapter also refuses a clock observation below its retained tick floor.

Prepare an assignment from the retained capture:

```json
{"tool":"fortress.plan","arguments":{
  "session_id":"<session>","idempotency_key":"miners-001",
  "expected_witness":"<capture witness>","detail_index":0,"assigned":true
}}
```

Preparation delegates to `WorkforceSession` and the existing journal: exact
source/capture checks, intent sync before native preparation, prepared-receipt
sync before acknowledgement. Only membership in an existing selected-only
work detail changes. No detail creation, mode change or job interruption occurs.
Use `assigned=false` for a separately reviewed removal plan; overlapping work
details can still grant labor permissions after removal.

Review `result.effect.review`, especially the selected detail, citizen identities,
changed IDs and labor columns. Then confirm `result.effect.record.plan_digest`:

```json
{"tool":"fortress.commit","arguments":{
  "session_id":"<session>","idempotency_key":"miners-001",
  "plan_digest":"<reviewed digest>","confirm":true
}}
```

The adapter revalidates the paused workforce and syncs dispatch before the only
assignment attempt. Every native edge rechecks inherited runtime I/O permission,
cancellation, current operator fortress selection and labor enablement. Revocation
after dispatch sync can therefore leave an unresolved marker without sending the
setter; it cannot make the operation eligible for retry.

## Recover, cancel and hand off

`fortress.wait` takes the exact key/digest and optional narrower wall limit. It
makes at most one receipt query. Prepared, terminal and permanent Unknown evidence
returns locally. Missing native records remain uncertain; Unknown never becomes
Applied through a later query. There is no polling, reconnection loop, preparation
renewal or assignment replay. `fortress.explain` exposes the retained full review
and historical receipt, including offline.

```json
{"tool":"fortress.query","arguments":{
  "session_id":"<session>",
  "query":"{\"mode\":\"records\",\"state\":\"unresolved\",\"limit\":4}"
}}
```

Record discovery uses compact whole-record references, not repeated full workforce
captures. Filters are all/pending/unresolved/terminal, limit 1..8. Continuations
bind session, journal ID, exact head, filter and limit. A 64-entry cursor cache can
expire older handles. Head changes or reopen require a new first page. Durable
keys and plan digests survive restart; presentation continuations do not.

`fortress.cancel(scope="effect", idempotency_key=..., plan_digest=...)` retires
undispatched local intent without a native call. After a dispatch marker it can
retire a surviving native preparation, but does not undo membership or repair
Unknown. `membership_undone=false` remains explicit.

Session close normally refuses unsettled work. Explicit
`fortress.cancel(scope="session", release_for_recovery=true)` can release even
fenced/revoked custody, but does not cancel effects, erase evidence, restore
membership or prove quiescence. Checkpoint and restore explicitly refuse.

## Bounds, projection and tests

The default work allowance is 1 GiB, an accounting ceiling rather than an
allocation. It covers repeated verification of a journal whose own cap remains
64 MiB. Pre/post view reservations each cover the complete journal and up to 64
maximum-size plan/effect copies. Opening has a separate allowance; connection
bootstrap and the adapter operation share a narrowing wall deadline.

Session maxima are 60,000 ms, 1 GiB total byte-work, 65,536 output token-proxy
units, one action and zero game-clock advancement. One proxy unit is four output
UTF-8 bytes, not measured tokenization. Complete output is reserved before any
native work: 32 KiB for compact discovery and 192 KiB for detailed reviews.
Requests below required reservations are refused, not silently truncated.
Storage latency is cooperative, not hard real-time.

Agent Turns expose a coordination-journal anchor with explicit tick=0 sentinel,
not a canonical world snapshot. Current-workforce and job-completion claims stay
false. Active-work counts/references are local to this verified journal, with
explicit omissions and no fabricated absence from unverified empty arrays.
Production provenance is removed from this isolated projection.

```bash
python3 scripts/check_workforce_mcp_reference.py
cargo test --locked -p dfmcp-mcp live_workforce_server
```

Only the first command executed in the authoring environment: ten independent
model/lexical groups passed. They cover 259 accepted/16 rejected selections,
45 accepted/20 rejected queries, 2,080 pagination models, byte reservations and
26 conservative output models. The largest modeled packet is 141,099 bytes,
below the 196,608-byte detailed reservation. ASCII escaping deliberately
upper-bounds UTF-8 output; this is not actual Rust serialization.

The 18 registered Rust groups exercise the real dispatcher/session/journal APIs
with injected edges, including reviewed confirmation, lost preparation/commit,
no redispatch after reopen, permanent Unknown, local cancellation, mode isolation,
failed refresh, clock regression, post-marker revocation, uncertain sync, cursor
binding, authority, release semantics and generated dotted tool definitions.
They are **uncompiled and unexecuted** here. Native SDK/ABI, live fortresses,
power-loss recovery, cross-controller exclusion and full qualification remain
outstanding. Neither these models nor this source confers production admission.
