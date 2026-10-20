## Dig recovery behind interrupted stop publication

Original dig inspection/query/cancel can preserve and report a bounded private
incomplete stop marker while validating all original intent and receipt custody.
New work remains fenced; no signal is repaired and no designation is repeated.
See `docs/DIG_STOP_RECOVERY.md`. Eighteen real-file/TCP component tests pass with
synthetic native sources; no native/live or full-repository qualification claim.

CLI recovery now holds original stopped-batch custody through final serialization
and one output attempt. Lost acknowledgment never repeats a query. Local final
validation passes 61 cases, including actual stop-writer process death, a complete
512-target batch, and room-backed recovery/export against synthetic native peers.
The new test sources are delivered separately after connector upload refusal.
