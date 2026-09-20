# Typed workforce/1.17 adapter

`dfmcp_adapter::workforce_control` now decodes the existing native workforce
capture and reconstructs assignment readback in safe Rust. It does not change
native protocol 1.17, call the Python developer client, or add dependencies.
This first increment contains codec, authority and fixed RPC source; durable
coordination and MCP integration are separate increments.

A sealed plan binds its key, existing detail index, assign/remove operation and
complete before capture. Detail positions are capture-local, not persistent IDs.
The before witness covers source folder/site/generation, native clock/sequence,
all bounded detail memberships and labor definitions, and selected citizen plus
historical identities and masks. Plan creation rejects no-ops, ineligible citizens,
non-selected-only/empty details, disabled automatic professions and capacity overflow.

Applied receipts must reconstruct the full expected membership configuration.
Only actually changed citizens may have different recomputed labor masks; adding
must enable the selected detail's permissions. Removing membership may leave
permissions enabled by another detail. Identity, eligibility, unchanged citizen
masks and all other captured fields remain fixed. Receipt checksums alone cannot
prove those postconditions. Native Unknown is immutable and cannot later become
Applied. Receipts are historical configuration evidence, never job-completion or
present-state proof.

The RPC client binds six fixed methods under `dfmcp_workforce_v1_17`. It checks
nonce, protocol, software and exact generation, canonical protobuf fields, method
ID uniqueness, response shape and dimensions. Unit IDs are the explicit unpacked
repeated protobuf field. Query absence stays unknown. Every failed wire call
fences the stream; there is no reconnect, method selection or hidden retry.
Numeric loopback sockets share one absolute connect/handshake/call deadline.
Later contexts may narrow it but cannot renew it. Notifications and frame bytes
are independently bounded. A conservative 400 KiB is charged per call and seven
calls plus 24 bytes per bootstrap; canonical capture/effect ceilings remain
64 KiB/8 KiB. Observe checks the selected/detail/membership entity allowance.

Current named-fortress Query and guarded ConfigureLabor grants are checked at
native effect boundaries; preparation additionally requires guarded Plan.
Limited-use or entity-only grants fail rather than inventing canonical identities
or replayable consumption. Exact folder/site must additionally be selected and
checked by the coordinator. No admission or cross-controller lease is implied.

## Evidence

Eleven Rust regression groups are registered. They cover real C++ fixtures,
full poststate reconstruction, all 512 eight-citizen assignment/removal cases,
corruption/truncation, permanent Unknown, removal with overlapping permissions,
authority, fragmented scripted RPC, absent records and failed-commit fencing.
**They are uncompiled and unexecuted: Rust/Cargo/rustfmt are unavailable here.**

`python3 scripts/check_workforce_rust_reference.py` executes seven independent
Python reference/static groups: actual retained C++ fixture agreement, 218
single-byte corruptions, 366 incomplete capture/effect prefixes, 512 membership
cases, rehashed false evidence, source mismatches and source wiring. These are
not Rust execution, real DFHack, actual protobuf integration, MCP or qualification.
Existing native/developer-client evidence retains its original scope.
