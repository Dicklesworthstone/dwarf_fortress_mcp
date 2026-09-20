### Durable fortress-bound conditional-run coordination

Add a source/software/endpoint-bound Rust effect journal and private Linux
custody. Synchronize sealed intent before prepare, dispatch before unpause and
native evidence before acknowledgement. Recover lost preparation replies by
query; never restore dispatch eligibility after a dispatch marker. Preserve
historical predicate/stop evidence, source-loss uncertainty, fixed recovery modes
and reserved cancellation/terminal capacity. No native wire or dependency changes.

Fourteen Rust journal/custody groups are registered, bringing this adapter to 27;
all remain uncompiled/unexecuted because Rust tooling is unavailable. Six
independent Python framing/state groups pass, including a binary fixture,
corruption/torn-prefix rejection and no-redispatch paths. These are not Rust,
filesystem durability, MCP, native/live-game or admission qualification.
