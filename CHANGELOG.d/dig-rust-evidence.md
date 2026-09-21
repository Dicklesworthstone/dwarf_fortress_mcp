### Typed native mining evidence in the Rust adapter

Add bounded dig/1.16 capture decoding, explicit hidden/missing cells, sealed
native-compatible mining plans and strict complete-plan effect verification.
Expose full-halo read scope and whole-map-block scheduling-write scope. Preserve
all native bytes and historical-only designation semantics; no new wire or MCP
route, mutation authority, dependency or production admission.

Ten Rust test groups are registered but uncompiled/unexecuted. Independent Python
reconstruction matches all four existing native fixture Git blobs; that is not
Rust execution or live/SDK qualification. See docs/DIG_RUST.md.
