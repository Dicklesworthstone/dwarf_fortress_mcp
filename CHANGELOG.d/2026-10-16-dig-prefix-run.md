## Exact original-prefix execution contract

Add a bounded immutable review for 1..8 consecutive original dig-batch steps and
strict successor validation. Confirmations bind the full original inventory,
keys, geometry and risk policy. Stale/altered reviews, skipped/repeated effects,
rewritten receipts, source regression and unresolved work are refused.

22 focused pure Python tests pass. Native effect-shell/CLI integration follows
separately; native protocols, journal formats and production admission are not
changed. Beads: df-dfhack-bridge-plane-c-pic.4/.5.

Add `run_dig_blueprint.py review|run` through the existing original batch owner,
write-ahead dispatcher and one-shot native client. One confirmed foreground call
can advance 1..8 exact NEW original steps. Each step gets fresh observations and
a distinct one-shot connection; any uncertainty/refusal stops the call without
retry, automatic query, skipped steps, or changed keys. The original journal,
room handoff and source policies remain authoritative. Response capacity is
reserved before contact, with one shrinking run budget and final authority check.

52 tests pass in total: 22 policy and 30 legacy native-client/private-file/TCP/CLI
cases, including 512 tiles in eight steps and a real child SIGKILL after a
synthetic native effect. Component validation uses a local adapter at the eager
room import boundary; room-backed end-to-end execution is not qualified. No
adapter is shipped. Rust/native/live/full-repository admission is not claimed.
