# Sparse normal-mining blueprint batches

`scripts/dig_blueprint.py` compiles the **floor-only** subset of the existing
`dfmcp.excavation-blueprint/1` format to disjoint dig/1.16 rectangles. It preserves
unselected gaps, walls, room separation and levels; it never mines the enclosing
bounding box. Its geometry digest uses a separate execution domain. The original
blueprint semantic-mask digest remains compatible with the read-only monitor.

Run `python3 scripts/dig_blueprint.py blueprint.json` to inspect the complete
partition. Parts use `{"region":{"origin":[x,y,z],"size":[w,h,levels]},"shape":"floor"}`.
The top-level object is `{"schema":"dfmcp.excavation-blueprint/1","parts":[...]}`.
A shape goal is not native mutation permission: each resulting rectangle still
requires a fresh paused, eligible natural-wall observation and exact confirmation.
Already-excavated floor cells are NOT silently skipped or treated as native work.

The deterministic partition visits z/y/x order, takes up to eight consecutive
x-coordinates and then extends through up to eight complete selected rows. It
is an exact cover, not a claim of minimum rectangle count. Reordering or splitting
input parts without changing the selected mask preserves the compiled identity.
Overlapping parts, channels, ramps, stairs and wall-preservation goals are refused,
not silently interpreted as normal mining. Coordinates must leave the complete
one-tile three-dimensional native halo inside the supported coordinate range.

The existing monitor's limits remain: 32 parts, 512 targets, a bounding capture
of at most 1,024 cells with per-axis extent at most 128. The compiled batch must
also fit the existing non-evicting directory registry's 128 native intents.
All extent checks precede target expansion; no partial oversized plan is returned.

Validation: `python3 scripts/test_dig_blueprint.py`. Nine executed groups cover
all 4,095 nonempty 4x3 masks, 384 large rectangle sizes, multiple levels and room
gaps, semantic identity, unsupported modes, overlap, malformed JSON, native halo
edges, retention capacity and the actual compiler subprocess. These tests execute
Python; they do not establish Rust, DFHack SDK, live-game or production admission.

This compiler alone performs no native I/O. Beads
`df-dfhack-bridge-plane-c-pic.4` and `df-action-coordinator-exec-ero.4` remain open.

## Execute through the existing native client

`scripts/dig_blueprint_client.py` now connects that exact compiler to the existing
`dig_designation_client` and its directory registry. It does not reimplement the
wire, call a subprocess, unpause the game, add a native protocol, or add an MCP
tool. This is the isolated **development** workflow, not the Rust production
coordinator. Use an explicitly selected disposable fortress only. It creates no
global clock/spatial lease and offers no verified checkpoint or rollback.

The target dig designations exclude every unselected cell. As in the underlying
1.16 native handler, block scheduling metadata can still change across affected
16x16 blocks. Excluding a wall from the target mask is not proof of structural
safety or continuous preservation while the game runs.

Create an existing, empty, exact-mode 0700 directory. Use the unchanged native
1.16 plugin and environment; unrelated `DFMCP_*` names remain refused:

```sh
export DFMCP_ALLOW_UNADMITTED_DIG_V1_16=1
export DFMCP_DIG_TOKEN='<matching native token, 32..256 bytes>'
export DFMCP_DIG_ENDPOINT=127.0.0.1:5000
export DFMCP_DIG_ALLOW_DESIGNATE=1

python3 scripts/dig_blueprint_client.py init \
  --directory /private/bedroom-batch --blueprint bedrooms.json \
  --world-folder region1 --site 2 \
  --checkpoint-policy disposable-fortress-no-checkpoint
```

`init` acquires one native observation to bind folder, site, dimensions,
incarnation and software. Every future rectangle's complete halo is checked
against those map dimensions before publishing anything. This does not inspect
all future terrain for eligibility. It persists the complete blueprint and pins
the effects directory identity. The input blueprint file is no longer needed
for later commands. It never prepares or commits a designation. Failed or
interrupted initialization never repairs an existing directory or file.

The required literal checkpoint policy is an explicit development exception,
not a claim that a game checkpoint exists. Hidden neighbors remain refused unless
`--allow-hidden-neighbors` was explicitly selected during initialization. That
immutable policy never permits hidden targets or overrides known native blockers.

```sh
python3 scripts/dig_blueprint_client.py observe --directory /private/bedroom-batch
```

This finds the next unattempted step from retained native evidence, then obtains
one fresh bounded observation. The result includes its complete visible/redacted
halo, blockers, `batch_id`, `step`, observation `witness`, native
`plan_digest_for_confirmation`, and a `review_seal` binding the exact inventory.
The seal is a content confirmation, not proof of human review or a new capability.
All unknown/refused work and a local stop prevent further observations for start.

Advance only the exact reviewed step:

```sh
python3 scripts/dig_blueprint_client.py advance \
  --directory /private/bedroom-batch --batch-id '<batch_id>' --step 0 \
  --expected-witness '<observation.witness>' \
  --confirm-plan '<plan_digest_for_confirmation>' --review-seal '<review_seal>'
```

One invocation makes **at most one** native commit attempt. Under both batch and
existing effect-store custody it verifies the original inventory, reacquires the
complete observation, persists/registers the native intent, and delegates fresh
prepare/commit plus terminal evidence retention to the existing client. An optional
narrowing guard on `start_designation` revalidates the enclosing batch while the
existing store lock remains held. Ordinary single-capsule callers are unchanged.
Operator/source/custody/deadline checks occur again at the effect boundaries.

After a verified designation, call `observe` again for the next region. Its
capture incorporates scheduling changes from earlier regions even when their
halos or native blocks overlap. No stale precomputed observation is reused.
This is a **sequence of separate effects, not an atomic multi-room transaction**.
Later terrain can fail eligibility after earlier designations succeeded.

## Restart, unknown outcomes and stopping

```sh
# Offline inventory: original input file, endpoint and token are not needed.
python3 scripts/dig_blueprint_client.py inspect --directory /private/bedroom-batch

# One native receipt query for an already retained step; never commit again.
python3 scripts/dig_blueprint_client.py query \
  --directory /private/bedroom-batch --batch-id '<batch_id>' --step 0

# Retire an existing native preparation. This cannot undo a designation.
python3 scripts/dig_blueprint_client.py cancel \
  --directory /private/bedroom-batch --batch-id '<batch_id>' --step 0

# Persistently stop future batch steps, with no native call or clock change.
python3 scripts/dig_blueprint_client.py stop \
  --directory /private/bedroom-batch --batch-id '<batch_id>'
```

`inspect` returns the complete bounded inventory and retained blueprint. A batch
becomes `designations_verified` only when every step has a verified retained
historical designation receipt. This never means excavation completion, current
terrain, current safety, or current pause. The retained blueprint and its semantic
mask digest can be used separately with `track_excavation.py start-blueprint`;
that read-only map/1.5 source is not joined to the dig/1.16 incarnation or given
causal attribution merely because the geometry matches.

A lost reply, missing native record, unfinished preparation or failed intent
publication blocks all later steps. A replayed native preparation is never
committed. Querying an absent native key preserves uncertainty. Source changes
never rebind the batch. A refused child stops the sequence rather than silently
skipping it. No command replaces a key, retries a commit, deletes an intent,
truncates a torn tail, or forgets an earlier unknown because a later receipt exists.

`stop` persists an immutable batch-bound marker. It only disables future steps;
it does not stop miners, pause simulation, cancel an in-flight command in another
process, or undo existing designations. Lock contention is a refusal, not an
acknowledged stop. Original-key query/cancellation remains available afterward.
Stored terminal query results return locally before credentials/native access.

## Custody, bounds and executed evidence

The root has fixed `blueprint.json`, `effects/`, and optional `stopped.json` names.
Directory traversal is no-follow and locked; files are owned, exact-mode 0600,
single-link regular files under exact-mode 0700 custody. The manifest pins the
child directory's device/inode identity. The existing registry detects missing
registered intents; complete native receipts are validated against their exact
capsules, not trusted as serialized status. Publication syncs file and parent.
Offline receipt acknowledgement retains the existing client's re-sync behavior;
“offline” means no native access, not a claim of zero filesystem synchronization.

The manifest is at most 64 KiB, stop marker at most 1 KiB, complete output at most
128 KiB, and native intents/registry inherit their existing limits. The 1..60,000 ms
command deadline includes bootstrap, storage, reads and the native operation.
It is cooperative around filesystem calls, not a hard realtime syscall deadline.
Output failures after a dispatch never grant retry permission. This custody is
local, not an external anti-rollback root or exclusion of arbitrary other clients.

Run both actual Python suites:

```sh
python3 -m unittest discover -s scripts -p 'test_dig_blueprint*.py' -v
```

Thirty-three groups execute the compiler, batch coordinator, unchanged native
Python codec/client, POSIX files/locks/sync, joined fragmented TCP peers, and CLI
subprocesses. Tests include multi-level restart, exact masks, lost reply recovery
without duplicate commit, all six intent/registry/terminal sync failure points,
torn writes, stale confirmation, same-size manifest corruption after prepare,
operator revocation, source/clock changes, missing/substituted storage, intact stop
markers, hidden/hazard refusal, the original single-designation path, all 128
retained child receipts, and maximum-field actual JSON serialization.

TCP peers and installed byte fixtures are test doubles, **not a real DFHack SDK
or live fortress**. Rust/Cargo/rustfmt and the full workspace were unavailable;
no Rust, MCP, SDK, power-loss, full-repository or production qualification is
claimed. `docs/evidence/dig-blueprint-batch.json` binds the executed tests to exact
source hashes. The bridge/coordinator beads remain open for broader integration.
