# Host mining-control policy

`dfmcp_adapter::dig_control_policy` composes the existing mandatory runtime guard
with a live core `LeaseManager`, operator-bound protected regions and an exact
review seal. This is development control policy, not production admission.

The new `LeaseManager::verify_exclusive_spatial` checks its actual retained record,
not a client-supplied LeaseRecord: exact holder, acquired <= current < expiry,
exclusive spatial kind and containment of the complete requested write area.
Released, expired, shared, entity and unknown leases fail. A token is local to its
manager; callers must separately bind the fortress, session and journal.

The mining policy binds the exact journal identity, fortress/source/software,
operator scope, session, lease token and canonical set of up to 32 protected
cuboids. It checks whole affected 16x16 map blocks, since native scheduling changes
are shared even outside designated tiles. Protected areas are sorted and deduplicated;
malformed or out-of-bounds cuboids are refused. Current Query, Observe, Plan and
Guarded Designate capabilities remain mandatory. Historical ticks do not revive
expired authority or leases.

Checkpoint policy defaults to Required. No game-checkpoint verifier exists in
this profile, so Required refuses new preparation/commit with CheckpointRequired.
Only an explicit trusted DisposableFortress policy permits development designation
without a checkpoint. This is an acknowledged absence of checkpoint/restore
protection, not a fabricated checkpoint certificate. No tool request may switch
that operator policy. Neither policy certifies excavation safety or completion.

A review seal hashes the full native plan (including its key and exact observation)
and complete policy identity. A non-cloneable local review must be consumed by an
exact confirmation before the guard permits Commit. Seals are integrity commitments,
not signatures or proof of human attention; actual authority still comes from the
current capabilities, retained native connection, coordinator and lease book.

The PolicyDigGuard calls the mandatory runtime guard first at every edge. It then
rechecks policy and the live lease on both Prepare and Commit, including the
coordinator's checks after dispatch-state sync. Query and native retirement are
not blocked by a now-unavailable checkpoint or expired lease: they cannot start a
new designation and still undergo runtime/coordinator capability checks.

Three new core lease tests and seven policy tests are registered. They cover lease
revocation/expiry/kind/scope, protected shared blocks, canonical policy identities,
review substitution, changed session/journal, current authority and runtime
revocation, and recovery under strict checkpoint policy. These Rust tests are
UNCOMPILED AND UNEXECUTED in this editing environment: Rust/Cargo/rustfmt are absent.
No native SDK, live fortress, filesystem power-loss or full qualification claim is
made. This policy alone adds no MCP route and no new native wire generation.
