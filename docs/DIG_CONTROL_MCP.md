# Policy-guarded development mining through MCP

`dfmcp-dig-control-dev-server` connects the existing Rust dig/1.16 session,
coordinator, private journal and native client to the eleven-tool MCP interface.
It supplies observation, preparation, explicitly confirmed one-attempt commit,
query-only reconciliation and native preparation retirement. This is separate
from `dfmcp-dig-recovery-dev-server`, whose Query-only restrictions are unchanged.

**This is source-present development functionality, not a qualified live runtime.**
The Rust implementation and its tests have not been compiled or executed here.
There is no production admission, real game-checkpoint creation, restore,
structural-safety proof, global controller fence or excavation-completion claim.

## Operator configuration and checkpoint policy

The runtime accepts exactly these ten DFMCP environment names:

| Name | Meaning |
|---|---|
| `DFMCP_ALLOW_UNADMITTED_DIG_CONTROL_V1_16` | Required exact value `1`; this selects the development profile, not production admission. |
| `DFMCP_DIG_WORLD_FOLDER` | Exact world folder, 1..512 UTF-8 bytes. |
| `DFMCP_DIG_SITE_ID` | Canonical nonnegative decimal, at most i32::MAX. |
| `DFMCP_DIG_SCOPE` | `[min_x,min_y,min_z,max_x,max_y,max_z]`, ordered coordinates 0..32767. |
| `DFMCP_DIG_JOURNAL` | Operator-owned normalized absolute path to the Rust journal. |
| `DFMCP_DIG_ENDPOINT` | Numeric loopback socket; default `127.0.0.1:5000`. |
| `DFMCP_DIG_TOKEN` | Matching native token, 32..256 UTF-8 bytes; read only when connecting. |
| `DFMCP_DIG_ALLOW_DESIGNATE` | Absent disables new designation/native retirement; exact `1` enables consideration under all other policy checks. |
| `DFMCP_DIG_CHECKPOINT_POLICY` | Absent or `required` requires an unavailable verified game checkpoint. Only `disposable-fortress-no-checkpoint` selects the explicit development exception. |
| `DFMCP_DIG_PROTECTED` | Optional array of up to 32 cuboids in the same six-coordinate format; default `[]`. |

Other DFMCP profile variables, recovery-profile switches, production admission
state, noncanonical enablement and malformed configuration fail closed. Paths,
credentials, protected areas and checkpoint policy are not MCP arguments.

The default checkpoint policy returns `checkpoint_required` before native
preparation and before creating an intent. It does not silently downgrade to an
uncheckpointed operation. There is currently no game-checkpoint verifier to
satisfy Required. Only an operator deliberately working on a disposable fortress
may select `disposable-fortress-no-checkpoint`. That value acknowledges **no
checkpoint or rollback protection**; it is not a checkpoint certificate.

Example development configuration for a disposable fortress only:

```sh
export DFMCP_ALLOW_UNADMITTED_DIG_CONTROL_V1_16=1
export DFMCP_DIG_WORLD_FOLDER=region1
export DFMCP_DIG_SITE_ID=1
export DFMCP_DIG_SCOPE='[0,0,0,63,63,7]'
export DFMCP_DIG_JOURNAL=/private/mining/journal
export DFMCP_DIG_TOKEN='<matching native token>'
export DFMCP_DIG_ALLOW_DESIGNATE=1
export DFMCP_DIG_CHECKPOINT_POLICY=disposable-fortress-no-checkpoint
export DFMCP_DIG_PROTECTED='[]'
cargo run --locked --offline -p dfmcp-mcp --bin dfmcp-dig-control-dev-server
```

The native plugin separately requires its existing opt-in and designation
permission in the DFHack process. This profile does not alter those native
checks. Transport remains authenticated numeric-loopback TCP, not encryption.

## Agent workflow

Tool names below use the existing logical fortress.* spelling; the owned modern
MCP transport supplies its usual wire spelling and metadata conventions.

1. `fortress.open_session(region="[15,15,2,2,2]", max_wall_millis=60000)` performs
   a bounded source observation solely to establish the exact operator-selected
   fortress and journal binding. It creates a missing journal exclusively or
   verifies an existing one without repair. It does not prepare or commit, and
   the bootstrap capture is not retained as permission to plan.
2. `fortress.observe(session_id, region="[15,15,2,2,2]")` captures the complete
   target and one-cell 3D halo, retains that exact observation and its native
   connection, and returns a witness, blockers and a typed tile query. Width and
   height are 1..8. A new observation abandons any old local commit permission,
   never the outstanding durable obligation.
3. `fortress.plan(session_id, idempotency_key, observation_witness,
   allow_hidden_neighbors)` seals that exact observation, checks current policy,
   reobserves it, synchronizes intent, and obtains/synchronizes native preparation.
   A fresh preparation returns its native plan digest and policy-bound review
   seal. The explicit hidden-neighbor choice never permits hidden targets, missing
   context or known hazards. The source connection remains owned by the session.
4. `fortress.commit(session_id, idempotency_key, plan_digest, review_seal)` requires
   both exact digests and the non-restored local review. It prechecks the current
   lease/policy, consumes the review, reobserves the exact plan, and delegates the
   sole commit to the original session/connection. Dispatch state is synchronized
   before the native call. Runtime, source, capabilities, lease and policy are
   checked again after that sync. Terminal native proof is synchronized before a
   known outcome is acknowledged. The source and ephemeral permission are dropped
   after the attempt. There is no commit connection factory or retry loop.
5. `fortress.wait(session_id, idempotency_key, plan_digest, max_wall_millis)` queries
   the exact original operation at most once. Missing retention remains unknown;
   terminal and permanent native Unknown return locally without connecting.
   Queried Prepared evidence cannot revive commit permission.
6. `fortress.cancel(..., scope="effect", idempotency_key=..., plan_digest=...)`
   retires only the exact native preparation under current native-retirement
   authorization. It cannot undo a designation. Checkpoint/lease failures do not
   prevent recovery or retirement, but runtime, operator and core capability
   checks still apply.

Plan and commit errors abandon ephemeral review/connection permission after
request work admission; they do not delete the native or durable obligation.
An identical successful local plan replay returns the same review without native
repreparation. A terminal commit replay returns verified history locally. Neither
behavior permits retrying an uncertain commit. After restart, even Prepared
history has no review or connection permit; use query/retirement, not commit.

The original connection has an absolute deadline, at most 60 seconds from its
creation, including the interval between tool calls. It is not renewed by plan,
commit or a new request budget. The default request ceiling is 10 seconds; the
60-second open ceiling in the example provides more time for the whole reviewed
connection lifecycle. Expiry requires recovery, not a replacement commit client.

## Policy, leases and protected regions

`DigControlPolicy` binds the exact journal, fortress/source/software, operator
scope, session, live lease token, canonical protected cuboids and checkpoint
policy. The review seal additionally binds the complete native plan, including
its operation key. Seals are integrity commitments, not signatures or proof that
a human read the review. They do not substitute for current authority.

One server session holds a real core exclusive spatial lease over its configured
scope for 1200 game ticks from its highest observed tick. The host checks the
manager's actual lease record, not a serialized client claim. There is no automatic
renewal or live lease import on restart. Every affected shared 16x16 block must
fit the lease and avoid protected regions, even when a protected tile is outside
the target rectangle. Expired/released/shared/entity leases cannot authorize mining.

This is deliberately conservative host-local coordination. The mandatory private
journal lock excludes another owner of that same file, and any nonterminal record
blocks new keys throughout it. Other journals, directories, servers, plugins and
UI input are not globally fenced. Operator replacement/deletion/restoration of
all evidence is not prevented by hashes. Never switch journals to bypass uncertainty.

Changing immutable operator configuration requires explicit session release and
reopen. Removing designation enablement takes effect at the next native boundary.
Capabilities absent at session opening are not added by enabling a variable later.
Query-only historical inspection and outcome recovery do not need Designate.

## Discovery, output and close

`fortress.query` takes a closed JSON string with one of four modes:

```json
{"mode":"records","limit":8}
{"mode":"selection_tiles","witness":"<exact 64-character lowercase digest>","offset":0,"limit":16}
{"mode":"plan_tiles","idempotency_key":"dig-001","plan_digest":"<exact digest>","offset":0,"limit":16}
{"mode":"schema"}
```

Records use 1..8 whole rows and bounded opaque continuations bound to the exact
session, journal head and page width. Tile pages use 1..16 cells and return a
source-bound next query. Hidden/missing cells have no attribute payload. Omit
optional fields rather than supplying null. `fortress.explain` returns exact
retained plan/receipt evidence, and `fortress.doctor` verifies local coordination
inventory, not live-game health.

Every success/error uses the existing Agent Turn builder. Pending work is exposed
independently of history-page position. Failed final custody verification withdraws
inventory certainty while preserving prior pending identity as historical evidence.
Requests reserve 32 KiB for complete output and two final/baseline inventory reads
before action work. Token ceilings use an explicit four-byte accounting proxy;
8,192..65,536 proxy tokens and at most 1 GiB work bytes are accepted. The byte
ceiling is conservative accounting, not a 1 GiB allocation. Deadlines include
blocking-pool queue time, and nested operation allowances shrink rather than renew.

Synchronous I/O runs in inherited `Cx::spawn_blocking` tasks with joined results,
parent/worker cancellation and I/O checks. No fallback or detached thread is used.
Cancellation during a kernel filesystem operation cannot forcibly interrupt it;
no hard filesystem cancellation bound is claimed. Journal state survives a lost
response. Cursor publication waits for final verification and complete rendering.

`fortress.cancel(scope="session")` closes only settled custody. An explicit
`release_for_recovery=true` permits releasing unresolved/fenced local ownership
without erasing history or claiming native quiescence. Checkpoint and restore
remain explicit refusals: a coordinator journal is not a game save.

## Evidence and remaining qualification

The policy increment adds three core lease and seven adapter-policy tests.
This MCP increment adds seventeen actual-handler tests and five runtime/configuration
tests, for 32 newly registered Rust tests across both increments. They cover the
owned lifecycle, one-attempt dispatch, lost replies, restart, exact review,
current grant/lease/runtime revocation, protected shared blocks, terminal-sync
failure, absorbing Unknown, source-free history and complete response bounds.
**All 32 Rust tests are uncompiled and unexecuted in this editing environment.**

The executed Python query-schema checker passes 28 valid and 102 invalid cases.
It validates the published JSON Schema only, not Rust parsing, stateful cursors,
MCP routing, runtime scheduling, receipt handling or native I/O. Mathematical JSON
Schema integer semantics do not establish Rust lexical-number or duplicate-field
behavior. The source-bound report is `docs/evidence/dig-control-query-schema.json`.

```sh
cargo test --locked --offline -p dfmcp-core spatial_verification_tests
cargo test --locked --offline -p dfmcp-adapter dig_control_policy
cargo test --locked --offline -p dfmcp-mcp dig_control_server
python3 scripts/test_dig_control_query_schema.py
```

The real DFHack SDK/plugin, generated protobuf runtime, disposable-fort lifecycle,
MCP stdio, power-loss durability, Rust/Clippy/rustfmt and full repository qualification
remain unverified. No dependency, native-wire, production runner or recovery-only
profile is changed. Actual verified game checkpoints/restore, globally coordinated
controller ownership, and observed excavation-completion obligations remain missing.
