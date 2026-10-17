## Exact original-prefix execution contract

Add a bounded immutable review for 1..8 consecutive original dig-batch steps and
strict successor validation. Confirmations bind the full original inventory,
keys, geometry and risk policy. Stale/altered reviews, skipped/repeated effects,
rewritten receipts, source regression and unresolved work are refused.

22 focused pure Python tests pass. Native effect-shell/CLI integration follows
separately; native protocols, journal formats and production admission are not
changed. Beads: df-dfhack-bridge-plane-c-pic.4/.5.
