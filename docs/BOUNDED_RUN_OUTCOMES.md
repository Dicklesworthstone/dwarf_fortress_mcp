# Restart-safe bounded-run terminal evidence

The existing `scripts/bounded_run_client.py` commands now retain verified native
terminal records in `<record>.outcome.json`, beside the unchanged immutable intent.
This supersedes the intent-only CLI recovery limitation described in
`BOUNDED_SIMULATION_CLIENT.md`. Native run/1.13, its wire codec, Rust/MCP and the
production runner map are unchanged. The low-level `start` and `recover` Python
functions still return native results; the CLI supplies the new custody layer.

## Use

Start and query a disposable-fort development run using the existing flags.
Successful terminal start/query/cancel responses are synced before acknowledgement.
A running or absent native record is not persisted as terminal evidence.

```sh
python3 scripts/bounded_run_client.py query --record /private/run/run-001.json
python3 scripts/bounded_run_client.py inspect --record /private/run/run-001.json
```

`inspect` needs no environment, credential, bridge or game process and writes
nothing. With no outcome it still reports intent-only unknown state. With an
outcome it reconstructs and verifies the exact native record, including its
original receipt, against the original intent and captured software manifest.
Query/cancel return existing terminal history without opening another native
connection. They retain their existing explicit development opt-in and endpoint
checks. Cancellation cannot replay an unpause; a repeated terminal cancellation
cannot pause a replacement fortress.

`stopped` means historical pause evidence, not current pause or goal completion.
`refused` means the retained native record did not attempt this unpause.
`source_lost` remains indeterminate and requires operator attention even though
the native record is terminal. All views have `retry_permitted=false`,
`current_pause_unproved=true`, and `goal_completion_proved=false`.

## Storage and recovery

The sidecar is a bounded canonical JSON envelope containing format
`dfmcp.bounded-run-outcome/1`, the exact canonical intent SHA-256, source manifest
and native record hex. Its checksum uses domain `dfmcp-bounded-run-outcome/1`
followed by NUL and canonical payload bytes. Native record hashes and every
semantic field are checked again on replay, not trusted from stored summaries.
The outcome bound is 8 KiB; complete CLI output is bounded to 32 KiB.

Files are exact-mode 0600, regular and single-link under an owned exact-mode
0700 directory. Descriptor-relative no-follow traversal, bounded reads,
nonblocking opens and locks, repeated full-byte/inode/custody checks and an
exclusive directory lock protect cooperating recovery invocations. A new start
refuses either an existing intent or an orphan sidecar before native connection.
The intent is never rewritten. Terminal records never overwrite prior evidence;
identical repeats do not write, while conflicting native records fail closed.

Complete output is reserved before publication. Exclusive creation is followed by
complete write, file sync, directory sync and custody revalidation. Failed writes
or syncs report unknown without erasing either file. An incomplete or corrupt
sidecar prevents further native work through this CLI and is preserved for
investigation; no truncation, repair, eviction or automatic retry exists. A
complete frame surviving an uncertain sync can be inspected after restart, but
`storage_acknowledged_this_call=false` does not retroactively certify that sync.

This is host-local advisory custody, not a global controller fence, an external
anti-rollback root, authenticated receipt signature or game checkpoint. Filesystem
calls have no hard interruptibility guarantee. The sidecar does not make a run
whose native record disappeared before capture knowable. No daemon or detached
work is added.

## Executed tests

```sh
PYTHONPATH=scripts python3 -m unittest test_bounded_run_client test_bounded_run_outcomes
```

All 38 tests pass: the 20 existing client tests plus 18 new outcome scenarios.
Actual Python, private POSIX files, subprocess locking/offline inspection and
joined fragmented loopback TCP doubles are exercised. Cases include a lost commit
reply followed by exactly one QueryRun and no second commit, terminal cancellation
replay, source-loss uncertainty, full native-record reconstruction, 611 one-byte
corruptions and 611 incomplete prefixes, conflicting or substituted evidence,
partial writes and independent file/directory sync failures. Published source and
test blob identities match the locally executed bytes.

This is not execution of a real DFHack SDK/plugin or live fortress, Rust/MCP,
physical power-loss durability or full repository qualification. No admission is
claimed. Work advances df-dfhack-bridge-plane-c-pic.5 and
 df-action-coordinator-exec-ero.4 without closing their broader requirements.
