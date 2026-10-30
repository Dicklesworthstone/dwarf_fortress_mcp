# Recover an original blueprint clock without mining custody

The `clock-offline` and `clock-recovery` modes of
`dfmcp-dig-control-dev-server` reopen an exact retained whole-blueprint monitor
and its independent run journal. They do not open the native designation journal
or blueprint parent, acquire terrain, take a paused bootstrap observation or
issue a mining lease. A lost running-clock response therefore does not require
the fortress to become paused before the original safety cancellation can be
requested.

These modes complete recovery of the
[reviewed finite clock workflow](DIG_BLUEPRINT_CLOCK_MCP.md). They preserve the
same original monitor and run histories. They cannot register a replacement
goal, change a deadline, prepare another interval or restore a previous review.
The original eleven tool names and native run/1.13 protocol remain unchanged.

## Keep both exact histories

Retain the original operator folder, site, scope and endpoint settings. The
`DFMCP_DIG_JOURNAL` path remains a required configuration value, but the file is
not opened or recreated. You may omit `DFMCP_DIG_BLUEPRINT_CONFIG`; no blueprint
parent file is used by these modes.

The monitor and clock files must both exist and have their original externally
retained recovery references. Set `initialize:false` for both:

```bash
export DFMCP_DIG_BLUEPRINT_MONITOR_CONFIG='{"path":"/private/mining/project/excavation.history","initialize":false,"source":{"endpoint":"127.0.0.1:5000","generation":9,"df_version":"<exact DF version>","dfhack_version":"<exact DFHack version>"},"recovery":{"monitor_id":"<original monitor id>","head":"<acknowledged monitor head>"}}'
export DFMCP_DIG_BLUEPRINT_CLOCK_CONFIG='{"path":"/private/mining/project/clock.journal","initialize":false,"source":{"endpoint":"127.0.0.1:5000","generation":41,"df_version":"<exact DF version>","dfhack_version":"<exact DFHack version>"},"recovery":{"journal_id":"<original clock journal id>","head":"<acknowledged clock head>"}}'
```

Use the actual independent map and run generations. Their endpoint and exact
software pair must agree with the original configuration; their generations
need not equal one another or the native dig generation. Each acknowledged head
must occur in its complete retained history. A valid later tail may recover a
lost acknowledgement. Rollback, another monitor or clock journal, a torn tail
and changed original bindings are refused without repair.

The two files retain their private custody requirements: a real exact-mode
`0700` parent directory, distinct exact-mode `0600` regular files and exclusive
open-descriptor ownership. Opening these modes never initializes, truncates,
replaces or repairs either file. The monitor is opened read-only even in writable
clock recovery. Its sealed specification supplies the complete original sparse
goal and receipt-set association; it does not substitute for current verification
of the missing original designation inventory.

## Open the appropriate recovery mode

Omit `region`. The game-time allowance is zero; positive values are refused.

```text
fortress.open_session(mode="clock-offline", max_wall_millis=60000)
```

Offline opening needs no game connection or native credentials. For explicit
original-outcome reconciliation, select writable `clock-recovery`. Its Query
authority requires no clock mutation permission. If safety cancellation will
also be needed, enable the independent clock permission **before opening**:

```bash
export DFMCP_DIG_ALLOW_CLOCK=1
export DFMCP_DIG_BLUEPRINT_CLOCK_TOKEN='<matching native run token>'
```

```text
fortress.open_session(mode="clock-recovery", max_wall_millis=60000,
  max_game_ticks=0)
```

Opening still makes no native call. The token is consumed only if a later
reconciliation or cancellation actually needs the original native source.
Native outcome queries need that token even when the clock opt-in is absent;
the credential does not grant cancellation authority.
The designation token, map token, designation opt-in and disposable-fortress
checkpoint exception are unnecessary for this recovery workflow. The usual
explicit unadmitted dig-control profile opt-in remains required.

| Mode | Retained authority and available work |
|---|---|
| `clock-offline` | Scoped canonical Query and separately issued native Query; local verified clock and monitor history, restricted schema discovery, history doctor and close. No journal mutation or native connection. |
| `clock-recovery` | The same inspection plus one original native reconciliation per explicit request. Exact clock opt-in at opening additionally issues independent canonical and native ControlClock authority for original safety cancellation. |

Without the opening clock opt-in, writable recovery retains only Query grants
and a fixed native Recover owner. To add safety cancellation, release and reopen
the same exact histories with the explicit opt-in; changing a variable during
the session cannot upgrade either retained authority or that private mode.

Neither mode has Plan, Observe, Designate or future-tick permission. The native
NIL-source grants are issued independently of the named-fortress grants. Enabling
a variable later cannot add authority missing at opening; expired retained grants
cannot be renewed by supplying replacement request grants. Current runtime,
operator, authority and deadline checks still apply at each native boundary.

## Inspect the retained original work

Query clock history with its original monitor identity:

```text
fortress.query(session_id,
  query='{"mode":"blueprint_clock","monitor_id":"<original monitor id>","offset":0,"limit":8}')
```

At most eight run records appear in one page. The returned `next_query` pins the
exact clock head, preventing pages from different histories from being combined.
The original active run remains visible even when its row is outside that page.
No query creates a fresh review, contacts the source or changes the journal.

The corresponding monitor history remains available:

```text
fortress.query(session_id,
  query='{"mode":"blueprint_monitor","monitor_id":"<original monitor id>","offset":0,"limit":16}')
```

`query(mode="schema")` advertises only `blueprint_clock`, `blueprint_monitor`
and `schema`. `fortress.doctor` verifies these two retained histories; it does
not inspect live-game health. Every response identifies this restricted recovery
profile and withholds current original-designation, terrain and pause claims.
An old satisfied monitor remains historical evidence. Its targets cannot be
sampled or its history cancelled from this owner.

## Reconcile or cancel the original run

In `clock-recovery`, issue one explicit native outcome query:

```text
fortress.wait(session_id, scope="clock", monitor_id="<original monitor id>",
  max_wall_millis=5000)
```

The owner contacts the original source only when the retained interval needs
reconciliation. It does not poll in a loop. Prepared history never regains a
commit permit; a lost or missing native result never proves that unpause did
not happen. A returned terminal receipt is historical evidence and remains
separate from current pause or excavation completion.

Request the original safety action with:

```text
fortress.cancel(session_id, scope="clock", monitor_id="<original monitor id>")
```

Before local dispatch, cancellation retires the retained intent without
contacting the game. After dispatch may have occurred, it requests the original
native safety pause. Only that safety action can repeat. Cancellation requires
current canonical and independently scoped native Query/ControlClock authority
and the exact clock opt-in. It requires no Plan/Observe permission, future-tick
allowance, unexpired goal deadline or game checkpoint.

The complete original monitor must be intact at startup so its sealed target can
be reconstructed. Once that target is retained by this owner, later damage to
the monitor file does not prevent query or safety cancellation of the intact
original clock. Clock operations use the clock journal's own exact current
custody and the retained original authority. They report monitor evidence as
unverified rather than treating damaged monitor bytes as an effect precondition.
Damage to the clock journal itself continues to refuse native operations.

If the monitor cannot be reconstructed at startup, this profile cannot recover
the whole-blueprint association. The separate
[bounded-run recovery runtime](BOUNDED_SIMULATION_MCP.md) can inspect or settle
an intact original run journal under its own exact source and authority rules;
it does not reconstruct the blueprint proof.

After any native response, the highest consumed game tick remains an authority
floor, including overshoot or a tick that makes grants expire. Output publication
rechecks exact clock custody after the final runtime checkpoint. If a safety
cancel may have been attempted but its response cannot be acknowledged, the
failure retains that uncertainty and the exact original recovery identity.

SourceLost remains unresolved and provides no verified pause. A native source
restart can also lose records; absence does not authorize another unpause or
invent a stop. Keep the original journal and its recovery references.

## Close without hiding an unresolved effect

Normal session close verifies settled clock custody:

```text
fortress.cancel(session_id, scope="session")
```

If the original run remains pending, source-lost or uncertain, or a retained file
has become damaged, explicitly release the local owner for later recovery:

```text
fortress.cancel(session_id, scope="session", release_for_recovery=true)
```

This releases file locks and preserves an unverified handoff. It does not pause
the game, cancel the monitor, settle a native effect, erase history or certify
quiescence. The request's runtime cancellation and wall deadline still apply.
Keep all updated monitor and clock references from responses and failure handoffs
before restarting.

## Evidence and limitations

This owner supplies a bounded recovery path through the existing native protocol
and private journals. It does not prove shared dig/map/run process identity,
named-fortress lineage of run/1.13, mining causality, structural safety, current
pause or the absence of other controllers. It grants no new run capability and
does not change compatibility admission or the production runner map.

See [implementation status](../IMPLEMENTATION_STATUS.md) for exact executed
component evidence and outstanding full-runtime, native and live qualification.
