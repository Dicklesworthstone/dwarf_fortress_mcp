# Typed Rust mining evidence — dig/1.16

`dfmcp_adapter::dig_designation` adds a typed adapter boundary for the existing
isolated native mining protocol. It is not a Python wrapper, a new native wire
generation, an admitted runtime, or an MCP designation tool.

`DigObservation::decode` validates the complete native capture before publication:
source incarnation, intervention sequence, clock, world folder/site, map bounds,
exact target rectangle and full one-cell 3D halo. Missing and hidden cells are
distinct enum cases with no attribute payload. Visible raw fields remain exact;
unknown tags, illegal flags, incomplete cells, trailing bytes and oversized input
are rejected. The 16 KiB/300-cell bounds agree with the existing native format.

`DigPlan::new` rejects every observed target/halo blocker. The hidden-neighbor
policy must be explicit and never permits hidden targets, missing context or
known hazards. Plan digests and prepare tokens use the existing native domains
and exact encoding. The new private-field retained-plan encoding is versioned
DFMDGP16, bounded to 16527 bytes, and reconstructs and validates the whole plan.
No plan or serialized record grants authority.

`DigEffect::decode` binds the complete native record to the exact retained plan.
A designated result requires the predicted target count and full post-state
witness, including priority 4000, affected blocks' designation flags, zeroed
scheduling cooldowns and precisely one intervention-sequence advance. Rehashing
an inconsistent result does not make it valid. Prepared/Unknown have no terminal
receipt; Refused accepts only the native stale/cancelled-before-dispatch reasons.
There is no inferred NotApplied or excavation-complete state.

`DigRegion::halo` exposes the complete observation scope. `write_area` covers
whole affected 16x16 map blocks, not just requested tiles, because block scheduling
writes are shared. It is deliberately conservative even at a map edge.

## Evidence and remaining integration

Ten Rust test groups are registered against the four existing native fixtures,
covering every designated-effect bit, malformed/truncated records, false rehashed
proof, hidden noninterference, hazard/target guards, geometry, shared-block scope,
retained-plan reconstruction and exhausted sequence/clock bounds. **These Rust
tests are UNCOMPILED AND UNEXECUTED.** Rust, Cargo and rustfmt were unavailable.

An independent Python struct/hashlib reconstruction was executed. All four
resulting fixture files match the exact previously checked-in Git blob identities;
this verifies fixture reconstruction, not Rust decoding or runtime behavior.
No real DFHack SDK, live fortress, workspace qualification or power-loss durability
was exercised. The initial codec/RPC increment added no coordinator. The later
journal and private-file source are now described in `docs/DIG_RUST_COORDINATOR.md`;
those additions do not grant runtime, MCP or production admission.

The native contract remains `docs/DIG_DESIGNATION.md`. Existing CLI recovery is
covered separately by `docs/DIG_TERMINAL_RECOVERY.md` and `docs/DIG_STORE_RECOVERY.md`.

## Fixed, capability-scoped native RPC

`dig_designation::rpc` now supplies `DigSource`, `DigRpcClient`, typed preparation
replay metadata and a loopback TCP implementation. It binds only the existing six
1.16 methods: Handshake, ReadDesignation, PrepareDesignation, CommitDesignation,
QueryDesignation and CancelDesignation. Each connection is pinned to one region
and one native software/incarnation manifest; an observation additionally proves
its folder/site against the current OperationContext fortress identity.

Query authorization covers the whole halo, including during negotiation, so a
properly scoped grant need not be broadened to global Query. Reads additionally
require Observe over the halo. Prepare requires Plan and Designate at Guarded
risk over all shared blocks touched by native scheduling, and Observe over the
halo. Commit/cancel require the same Query/Observe/Designate scopes. Querying a
retained effect requires only Query. Current cancellation, risk, expiry and
limited-use grant checks use the existing core authority implementation. Returned
observations recheck authority against their actual fortress and game tick.

Only a fresh Prepared response issued on that exact connection can make its
sealed plan dispatchable. Imported, queried or replayed Prepared evidence cannot
manufacture this permit. Commit consumes the permit before any dispatch and is
attempted at most once per connection, even after failure. Cancel consumes a
matching permit and never sends CommitDesignation. Malformed, lost, contradictory
or source-shifted replies fence the connection. Query absence is not proof of
nonapplication. There is no automatic reconnect or commit replay.

Requests are at most 2 KiB, replies 32 KiB, and notification traffic is bounded to
eight frames/256 KiB per call. Unknown/duplicate fields, wrong wire types,
nonminimal/overflowing integers, aliased method IDs and noncanonical refusals
fail closed. A complete native effect is validated by the sealed codec above.
A 300 KiB worst-case byte allowance is consumed before each attempted native call;
negotiation reserves seven such calls plus 24 greeting bytes. That connection
budget cannot be renewed by passing another context. The absolute TCP deadline,
at most 60 seconds from connect, includes negotiation and can only narrow; partial
reads/writes never renew it. Entity budgets cover the full halo before dispatch.

These are synchronous adapter calls: the eventual runtime must own them inside
its supervised blocking region and repeat runtime/operator/custody checks at its
actual effect boundary. The low-level RPC client is not itself a coordinator.
`dig_designation::journal` now supplies durable intent/dispatch/terminal state,
restart fencing and exact-digest confirmation; `journal::private_file` supplies
Linux private-file custody. A mandatory caller guard still needs the real runtime
lease/checkpoint policy before MCP integration. Do not use a fresh connection/key
or a different journal to bypass unresolved work. The existing Python directory
registry is not silently adopted as this Rust journal.

Twelve additional Rust RPC groups are registered (22 total), using fragmented
in-memory streams and real core authorization types. They cover lifecycle/replay,
ambiguous commit, scoped grants, post-capture expiry, cancellation, malformed
frames, binding aliases, missing records and nonrenewable work allowances. **All
22 Rust groups remain uncompiled and unexecuted.** No TCP/native/Rust/MCP execution,
full qualification or production admission is claimed by this RPC increment.

With the pinned Rust toolchain and locked dependencies available, the targeted
command is `cargo test --locked --offline -p dfmcp-adapter dig_designation`.

The later coordinator and Linux storage increments register 32 additional Rust
tests, for 54 across the dig module, including one child-process lock probe. All
remain uncompiled and unexecuted in this environment. The independent journal
framing reference is not execution of those tests or proof of filesystem durability.
