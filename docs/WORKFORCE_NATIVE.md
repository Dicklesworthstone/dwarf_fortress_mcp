# Native workforce/1.17

`bridge/dfhack-workforce-v1_17` connects the witnessed membership engine to six
fixed authenticated DFHack RPCs: Handshake, ObserveWorkforce, PrepareAssignment,
CommitAssignment, QueryAssignment, CancelAssignment. This is a new isolated
development profile, not an extension of read, clock, mining or work-order wire.
It is not a production-admitted profile or Rust/MCP runtime.

## Native action and limits

Observe selects 1..32 sorted unique citizen IDs and captures the complete bounded
work-detail configuration, all detail membership lists, labor column keys and
selected labor masks in one suspension. It scans at most 4,096 active units,
rejects null/duplicate IDs, and refuses missing/non-citizen targets rather than
exposing guessed visitor/invader state. Selected children/insane citizens can be
observed but fail mutation eligibility. Native/historical-figure ID pairs and
source generation prevent silently treating a replaced selected unit as the old
one. This is not a global citizen-generation registry.

Only membership in an existing nonempty OnlySelectedDoesThis detail changes.
The native writer rechecks source and the complete paused capture, preallocates
the replacement vector, resolves all selected pointers, swaps membership once,
and invokes supported `Units::setAutomaticProfessions` for changed citizens only.
Automatic professions must be enabled; the endpoint refuses to fight autolabor
or another controller that disables DF's work-detail system.

Adding must enable all of the detail's allowed labor bits. Removal may leave
bits enabled by other details. All selected post-recompute masks and the full
post-capture digest are retained; other captured configuration must be identical.
Automatic-profession recomputation can affect labor/equipment-related native
state beyond these masks. This endpoint does not certify a complete native-memory
write set, exclusivity, job assignment/completion, tools, skills or path access.

A failed/partial recomputation is Unknown. No new key can bypass that state in
the loaded engine. A lost response after verified readback retains the Applied
record for QueryAssignment. Query performs no live capture, recomputation or
write. Cancel only retires a still-prepared record, never undoes membership.
After plugin/map/world restart, absent native records remain unknown history.
Native idempotency does not survive process/plugin restart.

## Operator configuration

Compile with the matching DFHack source/SDK using the supplied CMake definition,
keeping `../common` headers. In the game process set:

```text
DFMCP_ALLOW_UNADMITTED_WORKFORCE_V1_17=1
DFMCP_WORKFORCE_TOKEN=<32..256-byte secret>
DFMCP_WORKFORCE_ALLOW_LABOR=1
```

The last variable is required by prepare, commit and cancel, including directly
inside the writer; reads need no labor opt-in. Any production admitted-protocol
variable causes refusal. Bearer credentials are not compatibility evidence.
Only trusted loopback transport is appropriate: DFHack's native TCP envelope
is not encrypted. Use disposable forts pending actual native/live qualification.

## Wire and evidence

The protobuf package is `dfmcp.workforce.v1_17`. Required fields are token, nonce
(16..64 bytes), major=1 and minor=17. Observe adds unit_ids. Prepare adds sorted
unit_ids, key, detail_index, assigned, expected_witness and plan_digest. Commit
and cancel add key, plan_digest and prepare_token. Query adds key and plan_digest.
Handshake has no optional fields. Wrong field-presence sets and unknown fields
are refused. Native request size is bounded to 2 KiB after protobuf decoding;
pre-decode limits and duplicate-scalar handling remain upstream responsibilities.

All canonical application integers are big-endian; strings use u16 byte lengths.
Capture magic is DFMWF017. Capture order is generation/sequence/tick (u64), site
(u32), folder, paused/automatic (u8), labor count (u16), labor key strings, detail
count (u16), detail records, unit count (u16), unit records. Detail records contain
name, raw flags (u32), selected_only (u8), exactly labor-count booleans, member
count (u16) and sorted IDs (u32). Unit records contain ID/historical ID (u32),
eligible (u8) and exactly labor-count booleans. Maximum capture size is 65,536.

Spec is detail_index (u32) and assigned (u8). Plan is SHA-256 of domain
`dfmcp-workforce-plan/1` + NUL + spec + SHA-256(capture). Token is first 16 bytes
of SHA-256(`dfmcp-workforce-token/1` + NUL + field(key) + plan).

Effect magic is DFMWE017, followed by field(key), plan (32), token (16), spec,
before witness (32), before generation/sequence/tick (u64), phase (u8), after
witness (32), labor count (u16), post-unit count (u16), post-unit records, receipt
(32). Receipt hashes all preceding effect bytes with domain
`dfmcp-workforce-receipt/1` + NUL. Phases: Prepared=0, Unknown=1, Applied=2,
Refused=3, Cancelled=4. Only Applied has a post-witness/units; other backing is
zero/empty. Checksums are integrity commitments, not signatures or authority.
The maximum effect is 8 KiB and the protobuf reply ceiling is 128 KiB.

Applied proves immediate membership/recompute readback at its source/tick,
not current state at receipt lookup. A failed Commit reply never proves no write.
The independent client must bind the complete before capture and reconstruct
expected post-configuration with observed changed-unit masks, rather than trusting
an unbound self-hashed after witness.

## Executed native tests

`python3 scripts/test_workforce_bridge.py --mutations` compiles the complete
actual plugin translation unit with explicit SDK/protobuf API doubles. GCC and
Clang each pass 3,208 assertions under warning denial and nonrecovering UBSan.
The corpus covers all 128 optional-field shapes across six operations, 32-citizen
batches, overlapping-detail removal, ownership/identity drift, automatic-profession
revocation, partial recomputation, wrong readback, lost reply and source reset.
Each compiler rejects three independently compiled mutants: missing recomputation,
wrong membership write, and weakened complete-witness checks.

The separate actual engine suite passes 8,497 assertions/compiler and agrees with
three independent Python vectors. Neither suite executes a real SDK/ABI, protobuf
serialization, GUI cache, DFHack lifecycle manager or live fortress. Rust/MCP,
power-loss, full repository qualification and production admission remain absent.
