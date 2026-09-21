# Typed Rust mining evidence — dig/1.16

`dfmcp_adapter::dig_designation` adds a typed adapter boundary for the existing
isolated native mining protocol. It is not a Python wrapper, a new native wire
generation, an admitted runtime, or an MCP designation tool.

`DigObservation::decode` validates the complete native capture before publication:
source incarnation, intervention sequence, clock, world folder/site, map bounds,
exact target rectangle and full one-cell 3D halo. Missing and hidden cells are
distinct enum cases with no attribute payload. Visible raw fields remain exact;
unknown tags, illegal flags, incomplete cells, trailing bytes and oversized input
are rejected. The 16 KiB/300-cell bounds agree with the existing native format.

`DigPlan::new` rejects every observed target/halo blocker. The hidden-neighbor
policy must be explicit and never permits hidden targets, missing context or
known hazards. Plan digests and prepare tokens use the existing native domains
and exact encoding. The new private-field retained-plan encoding is versioned
DFMDGP16, bounded to 16527 bytes, and reconstructs and validates the whole plan.
No plan or serialized record grants authority.

`DigEffect::decode` binds the complete native record to the exact retained plan.
A designated result requires the predicted target count and full post-state
witness, including priority 4000, affected blocks' designation flags, zeroed
scheduling cooldowns and precisely one intervention-sequence advance. Rehashing
an inconsistent result does not make it valid. Prepared/Unknown have no terminal
receipt; Refused accepts only the native stale/cancelled-before-dispatch reasons.
There is no inferred NotApplied or excavation-complete state.

`DigRegion::halo` exposes the complete observation scope. `write_area` covers
whole affected 16x16 map blocks, not just requested tiles, because block scheduling
writes are shared. It is deliberately conservative even at a map edge.

## Evidence and remaining integration

Ten Rust test groups are registered against the four existing native fixtures,
covering every designated-effect bit, malformed/truncated records, false rehashed
proof, hidden noninterference, hazard/target guards, geometry, shared-block scope,
retained-plan reconstruction and exhausted sequence/clock bounds. **These Rust
tests are UNCOMPILED AND UNEXECUTED.** Rust, Cargo and rustfmt were unavailable.

An independent Python struct/hashlib reconstruction was executed. All four
resulting fixture files match the exact previously checked-in Git blob identities;
this verifies fixture reconstruction, not Rust decoding or runtime behavior.
No real DFHack SDK, live fortress, workspace qualification or power-loss durability
was exercised. This increment adds no dependency, production runner, capability
grant, coordinator journal or MCP route. Those integrations require their own
source, tests and admission evidence.

The native contract remains `docs/DIG_DESIGNATION.md`. Existing CLI recovery is
covered separately by `docs/DIG_TERMINAL_RECOVERY.md` and `docs/DIG_STORE_RECOVERY.md`.
