# Work-orders/1.10: durable creation through MCP

`dfmcp-live-work-orders-dev-server` connects the existing finite native furniture
creation path to the creation-specific durable coordinator and the frozen eleven
MCP tools. This is explicitly unadmitted development source. It has not been
compiled or run against MCP, native DFHack or a live fortress in this environment.
No production runner, dependency, existing native generation or compatibility
registry change is part of this increment.

Read WORK_ORDER_CREATION.md for the four-recipe native template, WORK_ORDER_RUST.md
for exact native evidence/RPC validation and WORK_ORDER_CONTROL.md for journal
custody, state transitions and restart semantics. A Created receipt establishes
insertion of a finite manager order with verified template readback. It does NOT
prove manager approval, material/workshop feasibility, later order state or goods
produced. The queue observation covers membership, not existing configurations.

## Operator-selected custody and authority

The runtime accepts only these six DFMCP environment names; every other DFMCP_*
variable, including production admission tickets/protocol/floor/receipt state,
is refused. The shared Agent Turn builder's ambient admission metadata is also
removed from isolated-profile error packets, never presented as this runtime's
provenance.

| Variable | Meaning |
|---|---|
| `DFMCP_ALLOW_UNADMITTED_WORK_ORDERS_V1_10` | Must be exactly `1`. |
| `DFMCP_WORK_ORDERS_JOURNAL` | Absolute normalized operator path under an existing exact-mode 0700 directory. |
| `DFMCP_WORK_ORDERS_FORTRESS_ID` | Canonical nonzero decimal folder/site lineage ID, matching the existing read profiles. |
| `DFMCP_WORK_ORDERS_TOKEN` | Connected modes require the native 32..256-byte secret. |
| `DFMCP_WORK_ORDERS_ENDPOINT` | Optional numeric loopback endpoint; default `127.0.0.1:5000`. |
| `DFMCP_WORK_ORDERS_ALLOW_PRODUCTION` | Absent or exactly `1`; required additionally for control mode. |

No MCP argument can choose a file path, fortress, protocol, endpoint, credential,
native command, reaction or Lua program. The runtime rejects non-Unicode or
out-of-bound configuration, wrong profile session handles and unknown mode names.
The native plugin independently checks its own development and production gates;
enabling the server does not enable a separately configured DFHack process.

The executable is selected explicitly:

```sh
cargo run --locked --offline -p dfmcp-mcp --bin dfmcp-live-work-orders-dev-server
```

This requires the repository's pinned owned dependencies and toolchain. The command
is an entry-point description, not a claim that this revision has compiled.

`fortress.open_session` accepts three modes:

- `offline` (default) opens an EXISTING journal with Query, a read-only descriptor
  and no source object. It does not read the credential or endpoint configuration.
  No native connection, file creation, repair or evidence append is possible.
- `reconcile` opens EXISTING writable custody with Query only and a fixed native
  connection. It may read the queue and query/retain exact receipts. It cannot
  prepare, commit or locally cancel creations, even with a later injected grant.
- `control` requires the extra operator production gate. It receives Query and
  fortress-scoped reversible ConfigureProduction and may initialize new custody.
  Removing enablement removes production authority from subsequent calls; Query
  remains available. Enabling it later cannot promote a recovery session.

Only one session owns this runtime slot. Session IDs are process-scoped with a
separate profile namespace. Resource closure drops the native connection and
journal while still holding the slot lock; another opener cannot race release.
Busy session locks fail promptly rather than waiting beyond the request budget.
Inherited Asupersync I/O restrictions and cancellation are checked at runtime
entry. No detached task, timer or implicit reconnection is created.

## Agent control loop

The following use logical dotted tool names; the owned MCP tool macros expose
the corresponding underscore wire names, as for the other profiles.

1. Open in the appropriate mode and call `fortress.query` to discover `pending`
   or `reconciliation_required` work. Do not initialize another journal to bypass
   unresolved, missing or corrupt evidence.
2. In control mode, `fortress.observe(session_id)` reads the complete bounded
   native queue. Its witness is a selected native observation, not a canonical
   world anchor. Failed refresh clears the old selection.
3. `fortress.plan` accepts `session_id`, `idempotency_key`, `recipe_name`, `amount`
   and `expected_witness`. Recipes are exactly `wooden_bed`, `wooden_door`,
   `wooden_table` and `wooden_chair`; amount is 1..100. The server builds the
   sealed plan and native token from retained evidence and syncs preparation.
   It does not insert an order or renew native lifetime on exact-key replay.
4. `fortress.commit` requires `session_id`, that key, returned `plan_digest` and
   exact `expected_witness`. The journal rechecks current authority, source,
   witness, global unresolved work and capacity before syncing DispatchStarted.
   Only then may the single native insertion call occur. There is no direct
   transport commit in the MCP layer.
5. `fortress.wait` takes an exact key/digest and optional narrower wall allowance.
   It performs one Query-authorized reconciliation, never a polling loop or a
   new insertion. Prepared/terminal records return stored evidence without queries.

Example plan arguments, after an actual queue observation:

```json
{
  "session_id": "<returned session_id>",
  "idempotency_key": "bed-order-001",
  "recipe_name": "wooden_bed",
  "amount": 5,
  "expected_witness": "<returned observation witness>"
}
```

`fortress.explain` returns one exact durable plan and receipt without native work.
`fortress.doctor` checks custody and reports journal counts and connection presence;
it is not a native health probe or an admission decision. Checkpoint and restore
remain registered but are explicitly unavailable. A coordination journal is not
a game checkpoint.

`fortress.cancel(scope="effect")` requires exact key/digest and production
authority, and can only permanently retire a still-prepared LOCAL creation.
It does not delete a manager order, send a native cancellation or undo insertion.
`fortress.cancel(scope="session")`, with no key/digest, releases resources even
after cancellation, revocation or journal fencing. It never erases evidence or
cancels retained effects.

## Lost replies, restart and recovery

An unknown creation blocks OTHER keys as well as the original attempt at the
coordinator boundary. No later queue membership observation alone can resolve
it. Missing native retention, a new generation or a Prepared reply after dispatch
is not a negative receipt. An explicit native Unknown is immutable.

A failed connection is fenced. A wait on that connection cannot reconnect or
repeat insertion. Close explicitly and open in reconciliation mode to establish
a new connection to the same native incarnation, then query the retained exact
key/digest. An exact Created/Refused native receipt may resolve a lost reply;
absence cannot. Offline recovery remains useful while DFHack is unavailable.
After a restart, an uncommitted preparation requires reacquiring identical queue
evidence before commit; native TTL and commit-time witness checks still apply.

The scope remains one authoritative operator journal and the fixed native profile.
Another journal/controller or bypassing the coordinator is not protected by this
per-file lock. Full native SDK, crash/fault and disposable-fort campaigns remain
necessary before stronger operational claims.

## Complete responses and bounded discovery

Session defaults are 5,000 ms, 68 MiB of byte work and 65,536 output-token proxy
units. Maxima are 60,000 ms, 68 MiB and 262,144 proxy units. Token accounting is the
existing four-byte proxy, not a tokenizer count. The byte allowance includes the
64 MiB maximum journal plus bounded handshake and response work; ordinary calls
do not scan or allocate that maximum merely because it was admitted.

Every operation reserves a 16 KiB envelope plus 64 KiB per whole output record
before native work. An insufficient output budget cannot dispatch a creation.
A too-small requested narrowing returns a complete refusal within the original
session allowance, not a claim that a safety packet fits a one-token request.
Final responses are checked again; uncertain acknowledgements retain warnings
instead of truncating JSON. Synchronous filesystem calls have cooperative elapsed
checks, not a guaranteed interruptible fsync or hard cancellation latency.

`fortress.query` accepts state `all`, `pending` or `reconciliation_required`,
limit 1..8 (default 2), an optional continuation and narrower byte/token bounds.
It scans at most 64 whole journal records, additionally limited by byte/entity
allowances, then emits at most the requested number of matching complete records.
A record includes the entire original queue observation and validated native effect,
not a partial record pretending to be complete. A filtered page may be empty with
a continuation. Matching totals and complete-set flags distinguish this from absence.

Issued continuations bind session, journal incarnation, exact head, state filter
and page size. Up to 64 remain available for retries; old or changed-head tokens
require restarting discovery. Client strings never become arbitrary offsets.
Every response carries a common Agent Turn with retained work counts/discovery,
unknown-custody labeling, explicit coverage omissions and non-authoritative
creation affordances. Empty unavailable summaries never establish no pending work.

## Evidence for the MCP increment

Sixteen additional Rust regression groups are registered: eleven actual shared
handler/policy/runtime cases and five projection/cursor cases. They cover the
observe/plan/discover/commit loop, exact fixture receipts, replay, lost replies,
explicit reopen, offline discovery, immutable recovery modes, cancellation,
output reservation, current authority, complete filtered pages, lost custody,
maximal queue responses, operator/session isolation and inherited runtime restrictions.
Together with the preceding eighteen coordinator/session groups, these two
increments register thirty-four Rust groups. **None was compiled or executed**: Rust, Cargo and
rustfmt are unavailable. No MCP execution, Rust formatting, Clippy, workspace,
filesystem crash, real DFHack SDK or live-game qualification is claimed.

Executed independent checks:

```sh
python scripts/check_creation_journal_reference.py
python scripts/check_work_order_mcp_reference.py
```

The journal reference rejects 1,385 byte corruptions, 1,382 torn prefixes and
eight illegal rehashed histories. The MCP reference passes 297 pagination-model
cases and verifies lexical/source wiring of the eleven tools. Its maximal record
JSON model is 51,809 bytes; the envelope model is 3,500 bytes; an eight-record page
model is 417,979 bytes. These are independent Python models, not Rust serialization,
a Rust parser, transport execution or filesystem evidence. Reports with source
hashes are retained in `docs/evidence/creation-journal-reference.json` and
`docs/evidence/work-order-mcp-reference.json`.
