# Excavation progress in the mining recovery handoff

The existing `dfmcp-dig-recovery-dev-server` now accepts an optional operator-selected
inventory of the Python progress journals written by `scripts/track_excavation.py`.
It verifies their complete history in Rust and includes their goals and outcomes
in the same Agent Turn as the native mining-effect inventory. A fresh agent can
therefore discover both unfinished native effects and unfinished terrain goals
without reconstructing earlier terminal output.

This extends the recovery-only server, not the mutation-enabled control server.
The standalone tracker still creates, samples and cancels terrain monitors. MCP
neither invokes that script nor runs another writer, performs a new map read,
changes a monitor, nor gains any native mutation capability. The native effect
journal and its QueryDesignation reconciliation remain separate and unchanged.
The code and tests are source-present, not Rust-qualified or live-game-qualified.

## Configure exact retained goals

Keep the existing recovery configuration from `DIG_RECOVERY_MCP.md`. Add:

```sh
export DFMCP_DIG_GOAL_JOURNALS='[
  {"label":"entrance-tunnel","goal_id":"<64-character goal journal_id>","journal":"/private/progress/tunnel.jsonl"}
]'
```

The value is a closed JSON array of zero to four entries. `label` is 1..48 ASCII
letters, digits, periods, underscores or hyphens. `goal_id` is the exact lowercase
nonzero SHA-256 identity of the monitor's initial frame, printed as `journal_id`
by `track_excavation.py inspect`. `journal` is a normalized absolute UTF-8 path,
at most 4096 bytes, with no empty, dot or parent components. The full configuration
is at most 20 KiB. Duplicate labels, identities or paths and unknown fields fail.
The server sorts entries by goal identity; configuration changes require reopening.

This is the **eighth** admitted DFMCP environment name for the recovery profile,
extending the original seven-name list in `DIG_RECOVERY_MCP.md`. It is not admitted
by the control server or the standalone tracker. Run those programs with their
own existing isolated environments. Paths and journal selection are never MCP
arguments. The goal must belong to the exact configured world folder/site and fit
entirely inside the recovery session's operator scope. Merely naming a journal
cannot expand Query authority.

Without this variable the existing recovery response is unchanged. The native
Rust journal is still required for recovery bootstrap; a progress journal cannot
replace it. This feature is read-only interoperability with Python **monitor**
journals, not migration of Python mining intents or designation receipts.

## What an agent sees

Admitted opened-session requests attach the optional projection at:

```text
agent_turn.active_work.excavation_goals
```

Requests rejected before session/authority/budget admission keep the ordinary
unbound refusal packet and expose no progress-file data. A fatal cancellation or
exhausted deadline may also prevent inventory publication. Otherwise, the projection
contains the configured count, inventory verification, pending count and all
configured goal rows. Each verified row includes its label, exact
goal/journal-head identity, fixed goal parameters, selected region, last accepted
sample tick and source digest, floor/wall/hidden/missing counts, matching streak,
interruption, terminal state and unfinished-read status. It exposes no token,
filesystem path or hidden tile payload.

The native journal's `indeterminate_effects`, pending key and inventory verification
are not overwritten. There are two distinct questions: whether a native operation
has a verified receipt, and whether a sampled terrain goal held. A satisfied goal
cannot clear an indeterminate designation or authorize retry. The map reader's
incarnation is not joined to the independent dig reader's incarnation; matching
coordinates do not establish causality or a coherent cross-profile snapshot.

Missing, busy, corrupt, substituted or wrong-identity progress files remain visible
as `verification=unavailable`, `goal_status=unknown`. They never disappear or prove
absence. When an earlier verified row exists in the same session it is retained
under `historical_prior`, without inheriting its terminal status into the current
unknown row. Final file changes similarly withdraw verification. A verified
historical tick raises, never lowers, the current authorization-expiry floor.

Session release lists configured identities as unverified without reopening any
progress file. It releases only local mining custody; the independently owned
terrain goals remain in their original journals. It does not cancel monitoring,
delete history or prove native quiescence.

## Recompute, do not trust reported success

The archive decoder consumes the existing canonical JSON-line frames, not the
tracker's printed status packet. It checks exact fields, sequence, predecessor,
SHA-256 domain, canonical lowercase hexadecimal and Python-compatible ASCII JSON.
It rejects duplicate fields, alternate formatting, floating-point numbers,
unknown events, invalid native captures, torn tails and changes after termination.
Unicode source identities use the same escaped UTF-16 representation as Python's
`ensure_ascii=True`, including surrogate pairs.

Native samples pass through the existing map/1.5 Rust decoder and the new typed
`dfmcp_adapter::excavation_goal` evaluator. The predicate is exactly visible FLOOR,
zero liquid depth and no dig designation throughout one 1..8 by 1..8 rectangle.
Hidden/missing cells remain unknown. Equal-tick reads do not increase the matching
count. Contradictions, unknown captures, read failures, unfinished prior reads and
excessive sample gaps reset stability. The deadline is fixed and inclusive; source
or clock discontinuity invalidates rather than rebinding. Terminal evidence is
immutable. The Rust native decoder also retains its existing signed native-enum
bound; broader arbitrary u32 tile types are not admitted through Python input.

A trailing read intent is displayed as unknown with zero matching progress. A
completed response to the first intent may legitimately extend an old streak;
repeated read intents mark an interrupted attempt and cannot bridge its gap.
Reopening recomputes the complete retained history; no serialized success field is
accepted. The last accepted original-source observation stays distinct from an
invalidating foreign or regressed sample.

This proves only historical sampled floor conditions. It does not establish
current terrain, continuously stable intervals, structural support, mining safety,
walkability, material properties, a particular miner's work, or goal attribution
to a native operation. Constructed floors can satisfy the declared predicate.

## Read-only custody and bounded work

Linux x86_64/aarch64 use read-only, no-follow file descriptors under pinned directory
descriptors. Files must be owned, exact-mode 0600, single-link regular files under
owned exact-mode 0700 directories. An exclusive nonblocking lock spans the snapshot,
replay and final verification; the standalone writer may report contention while
that short-lived request owns the snapshot. Unsupported platforms refuse before
opening a path. No write, flush, synchronization, creation, truncation or repair
path exists in the progress reader.

The operator pins the header identity. Within a session, successful reads also pin
the inode and complete prior prefix, so only append-only growth of that same file
can advance the projection. Final verification rereads every byte before publishing
the new pin. These are not an external anti-rollback anchor: restoring an older
consistent directory before a new process starts is not detected as rollback.

Bounds are four configured files, 2 MiB per file, 16 KiB per frame, 260 frames,
128 subsequent read attempts and 2048 native bytes per sample. Each configured
file reserves 32 MiB of conservative work accounting before goal/native I/O;
this is not a buffer allocation. The original request ceiling remains enforced,
and the same deadline includes blocking-pool queue time, replay, native recovery,
final file checks and rendering. Kernel filesystem calls are not claimed to be
forcibly interruptible.

All goal rows fit an 8 KiB reservation inside the existing complete 32 KiB Agent
Turn bound. Under pressure, whole optional tool-result details are replaced by an
explicit budget error; native and goal recovery inventories are retained, never
truncated mid-object. Native pagination tokens are published only after final
combined verification and rendering succeeds. Progress discovery creates no extra
poller, thread, subprocess or native request.

## Checks and evidence limits

```sh
cargo test --locked --offline -p dfmcp-adapter excavation_goal
cargo test --locked --offline -p dfmcp-mcp dig_recovery_server
python3 scripts/check_excavation_inventory_fixtures.py
# Cross-check against the existing Python monitor:
python3 scripts/check_excavation_inventory_fixtures.py --python-monitor
```

Ten adapter and nineteen new MCP/archive/custody regression tests are registered,
including 576 terrain combinations, literal Python-format journals, all single-byte
corruptions and prefixes of the terminal fixture, Unicode identity, illegal state
transitions, same-tick and interrupted stability, pending native effects alongside
satisfied goals, output fallback, source mismatch, expired grants, file locks,
substitution and rollback detection. **All 29 new Rust tests are uncompiled and
unexecuted in this environment.** Rust, Cargo and rustfmt were unavailable.

The deterministic Python fixture checker and Python syntax check ran successfully:
18 valid examples, 12 rehashed invalid examples, 60 frame/checksum reconstructions
and four canonical-string cases. The same corpus also passed `--python-monitor`,
executing the unchanged, Git-blob-verified Python tracker's real replay/evaluator:
all eighteen states matched and all twelve invalid histories were rejected.
The default checker alone validates encoding, not semantic acceptance. A separate
Python conservative size model measured 6381 bytes for four maximum-field unknown
rows retaining historical summaries, within the 8192-byte row reservation.
These checks do not execute the new Rust parser, custody, MCP, runtime, native SDK
or live fortress and establish no power-loss or full repository qualification.
The source-bound receipt is `docs/evidence/excavation-inventory-fixtures.json`.

This supersedes only the standalone-progress statement that there is no MCP
inventory integration: the recovery server can now display verified journals.
Native sampling through MCP, control-server progress integration, automatic goal
creation after designation, and causal excavation-completion obligations remain
unfinished. No production admission or mutation capability is widened.
