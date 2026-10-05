# Complete Python furniture recovery handoffs

Owning bead: `df-dfhack-bridge-plane-c-pic`.

The fixed-plan progress reducer now returns the complete original normalized
plan and its existing digest in every valid lifecycle state. Because the Python
batch audit carries this result into its existing CLI responses, a newly arrived
agent can recover unstarted item IDs, furniture kinds, exact targets and the full
dependency graph without reconstructing earlier output or reading custody files.
The dependency graph survives the batch audit's removal of duplicate per-row
presentation fields. Completed, halted and indeterminate batches retain the same
original intent; this does not authorize retry, reallocation or construction
completion claims.

Progress records now require one of the five exact recorded phase strings.
An explicit null record previously passed validation and was treated as an
absent/not-started step. It now fails closed along with malformed values and
non-prefix records.

Ten focused Python unittest methods passed, including every placed prefix,
indeterminate/refused/cancelled handoffs, deterministic ordering, immutable-plan
isolation and a dense 32-step DAG near the 16 KiB plan limit. The regression
suite reproduced the missing-plan and null-record failures on the prior source.
The reconstructed baseline file was checked against its Git blob SHA before
editing. Python syntax checks passed for the changed module and new tests.

Run the focused suite with:

```sh
python3 -m unittest discover -s tests -p 'test_furniture_plan_handoff.py' -v
```

This is focused Python development evidence only. The full batch-process suite,
Rust workspace, native DFHack and live-game campaigns were not run for this
increment. Native wire formats, durable plan bytes/digests, placement authority,
compatibility registry and production runner map are unchanged.
