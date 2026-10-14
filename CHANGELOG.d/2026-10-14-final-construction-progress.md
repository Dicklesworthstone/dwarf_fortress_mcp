## Final-construction bounded clock progress

- Add an explicitly confirmed run/1.13 clock workflow for construction that
  remains pending after every original furniture placement is registered.
- Preserve existing v2 next-step confirmations; use a distinct final scope/key.
- Share the same complete clock-history fence, private directory lock, budgets,
  effect guards and query-only recovery. Never auto-loop or dispatch furniture.
- Reobserve all original furnishings after a verified stop without claiming
  stability-qualified construction completion or production admission.
