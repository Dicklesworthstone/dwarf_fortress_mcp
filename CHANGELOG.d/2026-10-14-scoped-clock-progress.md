## Exact-clock-bound original construction progress

- Make separately confirmed clock bursts executable through the real run/1.13
  client with original batch association, exact clock/source consent, one shared
  context and permission/custody checks before every effect.
- Sync a private immutable scope before native intent; preserve the same clock
  directory lock through outcome publication. Reject scope/intent replacement.
- Recover only the original key; terminal inspection is offline by default.
  Fresh post-stop reads cannot regress behind the stopped clock or imply whole
  batch completion. Old unassociated records retain their original recovery path.
- 64 tests pass with real clock TCP and private files; six weakened variants fail
  assertions. Route/batch acquisition, custody and outer packet remain explicit
  ports; no live DFHack, Rust/MCP or production admission qualification.
