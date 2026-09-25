# Durable excavation-run workflow

Connect native excavation-run/1.18 to a foreground plan/start/inspect/query/wait/
cancel/inventory CLI. Require exact fresh-capture confirmation and reject retained
native keys. Publish intent, preparation and dispatch durably before one commit;
recover only by original-key query/cancel. Preserve terminal native records for
offline inspection. Fence new keys while any journal remains unresolved, including
source loss; expose complete pending counts and inventory-bound pagination.

Execute 48 Python/POSIX/subprocess/TCP-double tests, including a killed start
process and all six predispatch sync-failure positions. Four weakened variants
fail focused regressions. No Rust/MCP, live/native qualification or admission claim.
