# Explicit future production reserves

- Add optional absolute `reserve_until_tick` to laboratory production intake,
  selecting the separately sealed `consumption_reserve_v1` generation. Require
  a future integer tick within one reference game year; refuse expired,
  excessive and overflowing requests.
- Budget joint FOOD/DRINK consumption through the later of work completion and
  the requested tick, including when the original stock minima already hold.
  Retain finite PLANT/WOOD, queued-work, setup and shared-workforce checks.
  Keep the original goal predicates and work-based action deadlines.
- Add a separate discarded-fork `reserve_forecast` for the exact requested
  frontier, including endpoints before work completion. Keep ordinary action
  completion separate from future stock prediction; neither proves continuous
  maintenance or real future availability.
- Retain the exact absolute horizon in versioned `/3` continuation sources,
  canonical source digests, saved-plan reconstruction and stale-source replay.
  Preserve historical generation bytes and seals. Expiry cannot silently
  extend a retained horizon, and current True minima still require a fresh
  production request when additional reserve work is wanted.

Usage and limits: [reserve planning](../docs/PRODUCTION_RESERVE_PLANNING.md) and
[goal continuation](../docs/PRODUCTION_GOAL_CONTINUATION.md). This increment
describes laboratory functionality and does not admit native mutations or a
live compatibility tuple.
