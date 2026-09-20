### Witnessed work-detail membership engine

Add bounded selected-citizen work-detail assignment/removal with complete paused
configuration witnesses, immutable idempotency, historical-ID checks, finite
preparation lifetime, pre-write uncertainty and verified membership/labor readback.
Only changed citizens may have their labor masks recomputed; adding must enable
the selected detail's allowed labors. Removing membership does not imply exclusive
labor disablement. Partial effects are never retried or silently rolled back.

GCC and Clang each execute 8,497 assertions under warning denial/UBSan; three
independent vectors agree and three compiled mutants are rejected per compiler.
This engine foundation is not a live SDK/MCP or production-admission claim.
