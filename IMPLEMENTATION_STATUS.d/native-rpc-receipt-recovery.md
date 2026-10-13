# Native furniture RPC receipt recovery

The actual furniture/1.19 QueryPlacement handler now invokes the existing
Engine::reconcile with only capture and insertion-verification callbacks.
Original generation, intervention sequence, full expected state including tick,
exact native relationships and final capture must match. Partial/absent effects
remain unknown; query never obtains a writer or a fresh placement permit.

Executed GCC 14.2 and Clang 17 component suites: six scenarios / 2,487 assertions
and three rejected compiled mutants per compiler, with warnings denied and UBSan.
The original RPC handler fails the positive-recovery regression. These runs use
actual handler source and explicit SDK/message doubles, not a real DFHack SDK,
protobuf runtime, live fortress, Rust workspace or production admission.
See docs/BUILD_PLACEMENT_RECOVERY.md. Durable client integration remains separate.
